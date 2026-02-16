use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Global registry of active watchers
static WATCHERS: once_cell::sync::Lazy<Arc<Mutex<HashMap<String, WatcherHandle>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Handle to a running file watcher
pub struct WatcherHandle {
    pub workspace_id: String,
    pub root_path: PathBuf,
    pub cancel_tx: mpsc::UnboundedSender<()>,
    pub started_at: Instant,
}

/// Statistics from a scan/watch operation
#[derive(Debug, Default)]
pub struct WatchStats {
    pub files_scanned: usize,
    pub files_changed: usize,
    pub symbols_added: usize,
    pub symbols_removed: usize,
    pub symbols_modified: usize,
    pub cascade_reembeds: usize,
    pub time_ms: f64,
}

/// Start watching a directory for file changes
pub async fn start_watching(
    workspace_id: &str,
    root_path: &Path,
    debounce_ms: u64,
) -> Result<WatchStats, Box<dyn std::error::Error + Send + Sync>> {
    let t0 = Instant::now();

    // Stop any existing watcher for this workspace
    stop_watching(workspace_id);

    // Create channels for communication
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel();

    // Set up file system watcher
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        Config::default(),
    )?;

    watcher.watch(root_path, RecursiveMode::Recursive)?;

    // Initial scan
    let mut stats = scan_directory(root_path).await?;

    // Store watcher handle
    {
        let mut watchers = WATCHERS.lock().unwrap();
        watchers.insert(
            workspace_id.to_string(),
            WatcherHandle {
                workspace_id: workspace_id.to_string(),
                root_path: root_path.to_path_buf(),
                cancel_tx: cancel_tx.clone(),
                started_at: Instant::now(),
            },
        );
    }

    // Spawn background task to handle file events
    let workspace_id = workspace_id.to_string();
    let root_path = root_path.to_path_buf();
    tokio::spawn(async move {
        let mut pending_changes: HashSet<PathBuf> = HashSet::new();
        let mut last_event_time = Instant::now();
        let debounce_duration = Duration::from_millis(debounce_ms);

        loop {
            tokio::select! {
                // Handle file system events
                Some(event) = rx.recv() => {
                    if should_process_event(&event, &root_path) {
                        for path in event.paths {
                            if is_code_file(&path.to_string_lossy()) {
                                pending_changes.insert(path);
                                last_event_time = Instant::now();
                            }
                        }
                    }
                }

                // Process pending changes after debounce period
                _ = sleep(debounce_duration), if !pending_changes.is_empty() && last_event_time.elapsed() >= debounce_duration => {
                    let changes: Vec<PathBuf> = pending_changes.drain().collect();
                    
                    info!("Processing {} file changes for workspace {}", changes.len(), workspace_id);
                    
                    // Process the changes (re-index affected files)
                    for path in changes {
                        if let Err(e) = process_file_change(&path, &root_path).await {
                            warn!("Failed to process file change {}: {}", path.display(), e);
                        }
                    }
                }

                // Handle cancellation
                _ = cancel_rx.recv() => {
                    info!("File watcher for workspace {} stopped", workspace_id);
                    break;
                }
            }
        }
    });

    stats.time_ms = t0.elapsed().as_millis() as f64;
    Ok(stats)
}

/// Perform a one-shot scan without starting a watcher
pub async fn scan_directory(root_path: &Path) -> Result<WatchStats, Box<dyn std::error::Error + Send + Sync>> {
    let t0 = Instant::now();
    let mut stats = WatchStats::default();

    // Walk through all files and check if they need reindexing
    for entry in walkdir::WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.')
                && name != "node_modules"
                && name != "target"
                && name != "__pycache__"
                && name != ".git"
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if is_code_file(&path.to_string_lossy()) {
            stats.files_scanned += 1;
            
            // Check if file needs reindexing (simplified - always reindex for now)
            if needs_reindexing(path).await {
                stats.files_changed += 1;
                
                if let Err(e) = process_file_change(path, root_path).await {
                    warn!("Failed to process file {}: {}", path.display(), e);
                }
            }
        }
    }

    stats.time_ms = t0.elapsed().as_millis() as f64;
    Ok(stats)
}

/// Stop watching a workspace
pub fn stop_watching(workspace_id: &str) -> bool {
    let mut watchers = WATCHERS.lock().unwrap();
    if let Some(handle) = watchers.remove(workspace_id) {
        let _ = handle.cancel_tx.send(());
        info!("Stopped file watcher for workspace {}", workspace_id);
        true
    } else {
        false
    }
}

/// Get status of all active watchers
pub fn get_watcher_status() -> Vec<(String, PathBuf, Duration)> {
    let watchers = WATCHERS.lock().unwrap();
    watchers
        .values()
        .map(|handle| {
            (
                handle.workspace_id.clone(),
                handle.root_path.clone(),
                handle.started_at.elapsed(),
            )
        })
        .collect()
}

/// Check if a file system event should be processed
fn should_process_event(event: &Event, _root_path: &Path) -> bool {
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => true,
        _ => false,
    }
}

/// Check if a file needs reindexing (simplified implementation)
async fn needs_reindexing(_path: &Path) -> bool {
    // For now, always return true - in a real implementation,
    // we'd check file modification time vs last index time
    true
}

/// Process a file change by reindexing it
async fn process_file_change(
    path: &Path,
    root_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rel_path = path.strip_prefix(root_path).unwrap_or(path);
    
    // Read file content
    let _content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(e) => {
            debug!("Could not read file {}: {}", path.display(), e);
            return Ok(());
        }
    };

    // TODO: Re-index the file using the embedding system
    // This would call into the search::index_files function
    debug!("Would reindex file: {}", rel_path.display());
    
    Ok(())
}

/// Check if a file is a code file that should be indexed
fn is_code_file(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    
    // Check extension
    for ext in &[
        ".rs", ".py", ".js", ".ts", ".jsx", ".tsx", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
        ".cs", ".php", ".rb", ".swift", ".kt", ".scala", ".clj", ".hs", ".ml", ".elm", ".dart",
        ".vue", ".svelte", ".json", ".yaml", ".yml", ".toml", ".md", ".sql", ".sh", ".bash",
    ] {
        if path_lower.ends_with(ext) {
            return true;
        }
    }
    
    // Check for common executable files without extensions
    if path_lower.contains("makefile") || path_lower.contains("dockerfile") {
        return true;
    }
    
    false
}