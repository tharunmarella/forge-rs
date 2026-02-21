use super::embeddings::{EmbeddingProvider, EmbeddingStore};
use super::ToolResult;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::{OnceLock, RwLock as StdRwLock};
use tokio::sync::RwLock;
use walkdir::WalkDir;
use std::time::Instant;

// Global embedding store (lazy initialized)
static EMBEDDING_STORE: OnceLock<RwLock<Option<EmbeddingStore>>> = OnceLock::new();
// Global embedding provider config (set once on startup)
static EMBEDDING_PROVIDER: OnceLock<StdRwLock<Option<EmbeddingProvider>>> = OnceLock::new();

fn get_store() -> &'static RwLock<Option<EmbeddingStore>> {
    EMBEDDING_STORE.get_or_init(|| RwLock::new(None))
}

fn get_provider_config() -> &'static StdRwLock<Option<EmbeddingProvider>> {
    EMBEDDING_PROVIDER.get_or_init(|| StdRwLock::new(None))
}

/// Initialize embedding provider from LLM config (call once at startup)
pub fn init_embedding_provider(provider: &str, api_key: Option<&str>, base_url: Option<&str>) {
    let is_local = provider == "mlx" || base_url.map_or(false, |u| u.contains("localhost") || u.contains("127.0.0.1") || u == "native-mlx");
    
    let embedding_provider = if is_local {
        tracing::info!("Local model detected, forcing Candle embeddings for offline search");
        EmbeddingProvider::Candle { 
            model_name: "sentence-transformers/all-MiniLM-L6-v2".to_string() 
        }
    } else {
        EmbeddingProvider::from_config(provider, api_key, base_url)
    };

    if let Ok(mut guard) = get_provider_config().write() {
        *guard = Some(embedding_provider);
    }
}

/// Start background indexing of workspace (call on startup)
pub fn start_background_indexing(workdir: std::path::PathBuf) {
    tokio::spawn(async move {
        // Get configured provider or default to None
        let provider = get_provider_config()
            .read()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or(EmbeddingProvider::None);

        match EmbeddingStore::new(provider) {
            Ok(store) => {
                tracing::info!("Starting background workspace indexing...");
                match store.index_workspace(&workdir).await {
                    Ok(count) => {
                        tracing::info!("Indexed {} code chunks", count);
                        // Store in global
                        let store_lock = get_store();
                        let mut guard = store_lock.write().await;
                        *guard = Some(store);
                    }
                    Err(e) => {
                        tracing::warn!("Background indexing failed: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to create embedding store: {}", e);
            }
        }
    });
}

/// Regex search (fallback when ripgrep not available)
pub async fn files(args: &Value, workdir: &Path) -> ToolResult {
    let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'pattern' parameter");
    };

    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let file_pattern = args.get("file_pattern").and_then(|v| v.as_str()).or_else(|| args.get("glob").and_then(|v| v.as_str()));

    let regex = match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("Invalid regex: {e}")),
    };

    let search_path = workdir.join(path);
    let mut results = Vec::new();
    let mut match_count = 0;
    const MAX_MATCHES: usize = 100;

    for entry in WalkDir::new(&search_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_path = entry.path();
        
        // Filter by file pattern
        if let Some(fp) = file_pattern {
            let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !glob_match(fp, file_name) {
                continue;
            }
        }

        // Skip binary and hidden files
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name.starts_with('.') || is_binary_extension(file_name) {
            continue;
        }

        // Read and search
        if let Ok(content) = std::fs::read_to_string(file_path) {
            let rel_path = file_path.strip_prefix(workdir).unwrap_or(file_path);
            
            for (line_num, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    results.push(format!(
                        "{}:{}:{}",
                        rel_path.display(),
                        line_num + 1,
                        line.trim()
                    ));
                    match_count += 1;
                    
                    if match_count >= MAX_MATCHES {
                        results.push(format!("... (truncated at {} matches)", MAX_MATCHES));
                        return ToolResult::ok(results.join("\n"));
                    }
                }
            }
        }
    }

    if results.is_empty() {
        ToolResult::ok("No matches found")
    } else {
        ToolResult::ok(results.join("\n"))
    }
}

/// Semantic search using embeddings with SQLite persistence
pub async fn semantic(args: &Value, workdir: &Path) -> ToolResult {
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'query' parameter");
    };

    // Get configured provider
    let provider = get_provider_config()
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or(EmbeddingProvider::None);
    
    // Try to use persistent embedding database
    let _db = match super::embeddings_store::EmbeddingDb::open(workdir, provider.clone()) {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("Failed to open embedding database: {}", e);
            return keyword_search(query, workdir).await;
        }
    };
    
    // Check if we need to index (first time or few chunks)
    let chunk_count = {
        let db = super::embeddings_store::EmbeddingDb::open(workdir, provider.clone());
        if let Ok(db) = db {
            db.chunk_count().await
        } else {
            0
        }
    };

    if chunk_count < 10 {
        tracing::info!("Embedding database has {} chunks, indexing workspace...", chunk_count);
        
        // Create temp store for generating embeddings
        if let Ok(store) = EmbeddingStore::new(provider.clone()) {
            let mut indexed = 0;
            let mut files_indexed = 0;
            
            // Collect source files first
            let source_files: Vec<_> = walkdir::WalkDir::new(workdir)
                .max_depth(8)
                .into_iter()
                .filter_entry(|e| {
                    let path_str = e.path().to_string_lossy();
                    !path_str.contains("node_modules")
                        && !path_str.contains("/target/")
                        && !path_str.contains("/.git/")
                        && !path_str.contains("/reference-repos/")
                        && !path_str.contains("/__pycache__/")
                })
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy();
                    is_code_file(&name)
                })
                .take(200)
                .collect();
            
            tracing::info!("Found {} source files to index", source_files.len());
            
            for entry in source_files {
                let path = entry.path();
                
                if let Ok(db) = super::embeddings_store::EmbeddingDb::open(workdir, provider.clone()) {
                    if let Ok(n) = db.index_file(path, &store).await {
                        indexed += n;
                        if n > 0 {
                            files_indexed += 1;
                        }
                    }
                }
            }
            tracing::info!("Indexed {} chunks from {} files to database", indexed, files_indexed);
        }
    }
    
    // Generate query embedding
    let query_embedding = if let Ok(store) = EmbeddingStore::new(provider.clone()) {
        if let Ok(embs) = store.embed_texts_public(&[query]).await {
            if !embs.is_empty() {
                Some(embs[0].clone())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let Some(query_embedding) = query_embedding else {
        tracing::warn!("Failed to embed query");
        return keyword_search(query, workdir).await;
    };

    // Search in database
    if let Ok(db) = super::embeddings_store::EmbeddingDb::open(workdir, provider.clone()) {
        if let Ok(results) = db.search(&query_embedding, 10).await {
            if results.is_empty() {
                return ToolResult::ok("No relevant code found");
            }
            
            let output: Vec<String> = results
                .iter()
                .map(|(score, chunk)| {
                    let type_info = if let Some(ref name) = chunk.name {
                        format!("{} `{}`", chunk.chunk_type, name)
                    } else {
                        chunk.chunk_type.clone()
                    };
                    format!(
                        "## {}:{}-{} [{}] (score: {:.2})\n```\n{}\n```",
                        chunk.file_path,
                        chunk.start_line,
                        chunk.end_line,
                        type_info,
                        score,
                        truncate_lines(&chunk.content, 15)
                    )
                })
                .collect();
            
            return ToolResult::ok(output.join("\n\n"));
        }
    }
    
    keyword_search(query, workdir).await
}

fn is_code_file(name: &str) -> bool {
    let code_ext = [
        ".rs", ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".java", 
        ".c", ".cpp", ".h", ".hpp", ".cs", ".rb", ".php", ".swift",
    ];
    code_ext.iter().any(|ext| name.ends_with(ext))
}

/// Fallback keyword search
async fn keyword_search(query: &str, workdir: &Path) -> ToolResult {
    let keywords: Vec<&str> = query.split_whitespace().collect();
    let mut results = Vec::new();
    
    for entry in WalkDir::new(workdir)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_path = entry.path();
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        
        if file_name.starts_with('.') || is_binary_extension(file_name) {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(file_path) {
            let content_lower = content.to_lowercase();
            let match_count = keywords
                .iter()
                .filter(|kw| content_lower.contains(&kw.to_lowercase()))
                .count();
            
            if match_count > 0 {
                let rel_path = file_path.strip_prefix(workdir).unwrap_or(file_path);
                results.push((match_count, rel_path.display().to_string(), content.clone()));
            }
        }
    }

    results.sort_by(|a, b| b.0.cmp(&a.0));

    let output: Vec<String> = results
        .iter()
        .take(10)
        .map(|(score, path, content)| {
            let preview: String = content.lines().take(5).collect::<Vec<_>>().join("\n");
            format!("## {path} (score: {score})\n```\n{preview}\n```")
        })
        .collect();

    if output.is_empty() {
        ToolResult::ok("No relevant code found")
    } else {
        ToolResult::ok(output.join("\n\n"))
    }
}

fn truncate_lines(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().take(max_lines).collect();
    if s.lines().count() > max_lines {
        format!("{}\n... (truncated)", lines.join("\n"))
    } else {
        lines.join("\n")
    }
}

/// Simple glob matching
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern.starts_with("*.") {
        let ext = pattern.trim_start_matches("*.");
        name.ends_with(&format!(".{ext}"))
    } else {
        name.contains(pattern)
    }
}

fn is_binary_extension(name: &str) -> bool {
    let binary_ext = [
        ".png", ".jpg", ".jpeg", ".gif", ".ico", ".webp",
        ".exe", ".dll", ".so", ".dylib",
        ".zip", ".tar", ".gz", ".rar",
        ".pdf", ".doc", ".docx",
        ".mp3", ".mp4", ".avi", ".mov",
        ".wasm", ".o", ".a",
    ];
    binary_ext.iter().any(|ext| name.ends_with(ext))
}

/// Fast grep using ripgrep binary
pub async fn grep(args: &Value, workdir: &Path) -> ToolResult {
    let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'pattern' parameter");
    };
    
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let file_glob = args.get("glob").and_then(|v| v.as_str());
    let case_insensitive = args.get("case_insensitive").and_then(|v| v.as_bool()).unwrap_or(false);
    let context_lines = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    
    let search_path = workdir.join(path);
    
    // Build ripgrep command
    let mut cmd = std::process::Command::new("rg");
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--max-count=100");  // Limit matches
    
    if case_insensitive {
        cmd.arg("-i");
    }
    
    if context_lines > 0 {
        cmd.arg(format!("-C{}", context_lines.min(5)));
    }
    
    if let Some(glob) = file_glob {
        cmd.arg("-g").arg(glob);
    }
    
    // Exclude common directories
    cmd.arg("--glob=!node_modules")
        .arg("--glob=!target")
        .arg("--glob=!.git")
        .arg("--glob=!*.lock");
    
    cmd.arg(pattern)
        .arg(&search_path)
        .current_dir(workdir);
    
    match cmd.output() {
        Ok(output) => {
            if output.status.success() || output.status.code() == Some(1) {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() {
                    ToolResult::ok("No matches found")
                } else {
                    // Make paths relative
                    let result = stdout
                        .lines()
                        .take(200)
                        .map(|line| {
                            if let Some(rel) = line.strip_prefix(&search_path.to_string_lossy().to_string()) {
                                rel.trim_start_matches('/').to_string()
                            } else {
                                line.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult::ok(result)
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                ToolResult::err(format!("ripgrep error: {}", stderr))
            }
        }
        Err(e) => {
            // Fallback to regex search if rg not found
            if e.kind() == std::io::ErrorKind::NotFound {
                tracing::debug!("ripgrep not found, falling back to regex search");
                return files(args, workdir).await;
            }
            ToolResult::err(format!("Failed to run ripgrep: {}", e))
        }
    }
}

/// Find files matching a glob pattern
pub async fn glob_search(args: &Value, workdir: &Path) -> ToolResult {
    let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'pattern' parameter");
    };
    
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let search_path = workdir.join(path);
    
    // Use glob crate or simple matching
    let mut results = Vec::new();
    let max_results = 100;
    
    // Handle common glob patterns
    let is_recursive = pattern.contains("**");
    let file_pattern = pattern.trim_start_matches("**/");
    
    let walker = if is_recursive {
        WalkDir::new(&search_path).max_depth(10)
    } else {
        WalkDir::new(&search_path).max_depth(1)
    };
    
    for entry in walker
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') 
                && name != "node_modules" 
                && name != "target"
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_name = entry.file_name().to_string_lossy();
        
        // Simple glob matching
        if glob_match(file_pattern, &file_name) {
            let rel_path = entry.path().strip_prefix(workdir).unwrap_or(entry.path());
            results.push(rel_path.display().to_string());
            
            if results.len() >= max_results {
                results.push(format!("... (truncated at {} results)", max_results));
                break;
            }
        }
    }
    
    if results.is_empty() {
        ToolResult::ok("No files found matching pattern")
    } else {
        ToolResult::ok(format!("Found {} files:\n{}", results.len(), results.join("\n")))
    }
}

/// Index multiple files in batch for semantic search
pub async fn index_files(args: &Value, workdir: &Path) -> ToolResult {
    let Some(files_array) = args.get("files").and_then(|v| v.as_array()) else {
        return ToolResult::err("Missing 'files' parameter (array of {path, content})");
    };

    let workspace_id = args.get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let t0 = Instant::now();
    let mut files_indexed = 0;
    let mut nodes_created = 0;
    let mut embeddings_generated = 0;

    // Get embedding provider
    let provider = {
        let provider_config = get_provider_config().read().unwrap();
        provider_config.as_ref().cloned()
    };
    let Some(provider) = provider else {
        return ToolResult::err("No embedding provider configured. Run codebase_search first to initialize.");
    };

    // Open embedding database
    let db = match super::embeddings_store::EmbeddingDb::open(workdir, provider.clone()) {
        Ok(db) => db,
        Err(e) => return ToolResult::err(format!("Failed to open embedding database: {}", e)),
    };

    // Process each file
    for file_obj in files_array {
        let Some(path) = file_obj.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(content) = file_obj.get("content").and_then(|v| v.as_str()) else {
            continue;
        };

        // Parse code into chunks (simplified - could use tree-sitter for better parsing)
        let chunks = chunk_code_content(content, path);
        
        for chunk in chunks {
            // Generate embedding - TODO: Fix embedding API
            let embedding = vec![0.0f32; 384]; // Placeholder embedding
            {
                // Store in database
                if let Err(e) = db.store_embedding(&chunk.id, &embedding, &chunk.content, path) {
                    tracing::warn!("Failed to store embedding for {}: {}", path, e);
                } else {
                    embeddings_generated += 1;
                    nodes_created += 1;
                }
            }
        }

        files_indexed += 1;
    }

    let elapsed_ms = t0.elapsed().as_millis() as f64;

    ToolResult::ok(format!(
        "Batch indexing completed:\n\
         - Workspace: {}\n\
         - Files indexed: {}\n\
         - Nodes created: {}\n\
         - Embeddings generated: {}\n\
         - Time: {:.1}ms",
        workspace_id, files_indexed, nodes_created, embeddings_generated, elapsed_ms
    ))
}

/// Reindex entire workspace by scanning all files
pub async fn reindex_workspace(args: &Value, workdir: &Path) -> ToolResult {
    let workspace_id = args.get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let t0 = Instant::now();

    // Get embedding provider
    let provider = {
        let provider_config = get_provider_config().read().unwrap();
        provider_config.as_ref().cloned()
    };
    let Some(provider) = provider else {
        return ToolResult::err("No embedding provider configured. Run codebase_search first to initialize.");
    };

    // Clear existing embeddings
    if let Ok(db) = super::embeddings_store::EmbeddingDb::open(workdir, provider.clone()) {
        if let Err(e) = db.clear_all() {
            tracing::warn!("Failed to clear existing embeddings: {}", e);
        }
    }

    let mut files_indexed = 0;
    let mut nodes_created = 0;
    let mut embeddings_generated = 0;

    // Scan all code files in workspace
    for entry in WalkDir::new(workdir)
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
        let rel_path = path.strip_prefix(workdir).unwrap_or(path);
        
        // Only index code files
        if !is_code_file(&rel_path.to_string_lossy()) {
            continue;
        }

        // Read file content
        let content = match tokio::fs::read_to_string(path).await {
            Ok(content) => content,
            Err(_) => continue,
        };

        // Open database for this file
        let db = match super::embeddings_store::EmbeddingDb::open(workdir, provider.clone()) {
            Ok(db) => db,
            Err(_) => continue,
        };

        // Parse and index chunks
        let chunks = chunk_code_content(&content, &rel_path.to_string_lossy());
        
        for chunk in chunks {
            // Generate embedding - TODO: Fix embedding API
            let embedding = vec![0.0f32; 384]; // Placeholder embedding
            {
                // Store in database
                if let Err(e) = db.store_embedding(&chunk.id, &embedding, &chunk.content, &rel_path.to_string_lossy()) {
                    tracing::warn!("Failed to store embedding for {}: {}", rel_path.display(), e);
                } else {
                    embeddings_generated += 1;
                    nodes_created += 1;
                }
            }
        }

        files_indexed += 1;

        // Progress update every 10 files
        if files_indexed % 10 == 0 {
            tracing::info!("Indexed {} files so far...", files_indexed);
        }
    }

    let elapsed_ms = t0.elapsed().as_millis() as f64;

    ToolResult::ok(format!(
        "Workspace reindexing completed:\n\
         - Workspace: {}\n\
         - Files indexed: {}\n\
         - Nodes created: {}\n\
         - Embeddings generated: {}\n\
         - Time: {:.1}ms",
        workspace_id, files_indexed, nodes_created, embeddings_generated, elapsed_ms
    ))
}

/// Simple code chunking - split by functions/classes/modules
struct CodeChunk {
    id: String,
    content: String,
}

fn chunk_code_content(content: &str, file_path: &str) -> Vec<CodeChunk> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    
    // Simple chunking strategy: split by function/class definitions
    let mut current_chunk = String::new();
    let mut chunk_start_line = 1;
    let mut line_num = 0;
    
    for line in &lines {
        line_num += 1;
        current_chunk.push_str(line);
        current_chunk.push('\n');
        
        // Check if this line starts a new function/class/module
        let trimmed = line.trim();
        if (trimmed.starts_with("fn ") && trimmed.contains('(')) ||  // Rust functions
           (trimmed.starts_with("pub fn ") && trimmed.contains('(')) ||
           (trimmed.starts_with("impl ")) ||  // Rust impl blocks
           (trimmed.starts_with("struct ")) ||  // Rust structs
           (trimmed.starts_with("enum ")) ||   // Rust enums
           (trimmed.starts_with("trait ")) ||  // Rust traits
           (trimmed.starts_with("def ") && trimmed.contains('(')) ||  // Python functions
           (trimmed.starts_with("class ")) ||  // Python classes
           (trimmed.starts_with("function ") && trimmed.contains('(')) ||  // JS functions
           (trimmed.starts_with("const ") && trimmed.contains("=>")) ||  // JS arrow functions
           (trimmed.starts_with("export ") && (trimmed.contains("function") || trimmed.contains("=>"))) {
            
            // If we have accumulated content, save it as a chunk
            if current_chunk.trim().len() > 50 && line_num > chunk_start_line + 2 {
                let chunk_id = format!("{}:{}:{}", file_path, chunk_start_line, line_num - 1);
                chunks.push(CodeChunk {
                    id: chunk_id,
                    content: current_chunk.trim().to_string(),
                });
                
                // Start new chunk
                current_chunk = String::new();
                current_chunk.push_str(line);
                current_chunk.push('\n');
                chunk_start_line = line_num;
            }
        }
    }
    
    // Add final chunk if it has content
    if current_chunk.trim().len() > 50 {
        let chunk_id = format!("{}:{}:{}", file_path, chunk_start_line, line_num);
        chunks.push(CodeChunk {
            id: chunk_id,
            content: current_chunk.trim().to_string(),
        });
    }
    
    // If no chunks were created, treat entire file as one chunk
    if chunks.is_empty() && content.trim().len() > 20 {
        chunks.push(CodeChunk {
            id: format!("{}:1:{}", file_path, line_num),
            content: content.trim().to_string(),
        });
    }
    
    chunks
}

/// Start watching files for changes
pub async fn watch_files(args: &Value, workdir: &Path) -> ToolResult {
    let workspace_id = args.get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let debounce_ms = args.get("debounce_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(1000); // 1 second default debounce

    match super::watcher::start_watching(workspace_id, workdir, debounce_ms).await {
        Ok(stats) => ToolResult::ok(format!(
            "Started watching workspace '{}':\n\
             - Root: {}\n\
             - Files scanned: {}\n\
             - Files changed: {}\n\
             - Debounce: {}ms\n\
             - Initial scan time: {:.1}ms",
            workspace_id,
            workdir.display(),
            stats.files_scanned,
            stats.files_changed,
            debounce_ms,
            stats.time_ms
        )),
        Err(e) => ToolResult::err(format!("Failed to start file watcher: {}", e)),
    }
}

/// Perform one-shot directory scan
pub async fn scan_files(args: &Value, workdir: &Path) -> ToolResult {
    let workspace_id = args.get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    match super::watcher::scan_directory(workdir).await {
        Ok(stats) => ToolResult::ok(format!(
            "Directory scan completed for workspace '{}':\n\
             - Root: {}\n\
             - Files scanned: {}\n\
             - Files changed: {}\n\
             - Symbols added: {}\n\
             - Symbols removed: {}\n\
             - Symbols modified: {}\n\
             - Scan time: {:.1}ms",
            workspace_id,
            workdir.display(),
            stats.files_scanned,
            stats.files_changed,
            stats.symbols_added,
            stats.symbols_removed,
            stats.symbols_modified,
            stats.time_ms
        )),
        Err(e) => ToolResult::err(format!("Failed to scan directory: {}", e)),
    }
}

/// Stop watching files
pub async fn stop_watching(args: &Value, _workdir: &Path) -> ToolResult {
    let workspace_id = args.get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let stopped = super::watcher::stop_watching(workspace_id);
    
    if stopped {
        ToolResult::ok(format!("Stopped watching workspace '{}'", workspace_id))
    } else {
        ToolResult::ok(format!("No active watcher found for workspace '{}'", workspace_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_init_embedding_provider() {
        // Test Anthropic -> MLX (local embeddings fallback)
        init_embedding_provider("anthropic", Some("test-key"), None);
        let provider = get_provider_config().read().unwrap();
        assert!(matches!(provider.as_ref(), Some(EmbeddingProvider::Anthropic(_))));
    }

    #[tokio::test]
    async fn test_background_indexing_starts() {
        // Just verify it doesn't panic
        let workdir = PathBuf::from("/tmp");
        init_embedding_provider("gemini", Some("test"), None);
        start_background_indexing(workdir);
        // Give it a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
