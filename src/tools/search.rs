use super::embeddings::EmbeddingStore;
use super::ToolResult;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use walkdir::WalkDir;

// Global embedding store (lazy initialized)
static EMBEDDING_STORE: OnceLock<RwLock<Option<EmbeddingStore>>> = OnceLock::new();

fn get_store() -> &'static RwLock<Option<EmbeddingStore>> {
    EMBEDDING_STORE.get_or_init(|| RwLock::new(None))
}

/// Grep search using regex
pub async fn grep(args: &Value, workdir: &Path) -> ToolResult {
    let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'pattern' parameter");
    };

    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let file_pattern = args.get("file_pattern").and_then(|v| v.as_str());

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

/// Semantic search using embeddings
pub async fn semantic(args: &Value, workdir: &Path) -> ToolResult {
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'query' parameter");
    };

    // Try to use embedding store
    let store_lock = get_store();
    let mut store_guard = store_lock.write().await;
    
    // Initialize embedding store if needed
    if store_guard.is_none() {
        match EmbeddingStore::new() {
            Ok(store) => {
                // Index workspace in background
                if let Err(e) = store.index_workspace(workdir).await {
                    tracing::warn!("Failed to index workspace: {}", e);
                }
                *store_guard = Some(store);
            }
            Err(e) => {
                tracing::warn!("Failed to create embedding store: {}", e);
                // Fall back to keyword search
                drop(store_guard);
                return keyword_search(query, workdir).await;
            }
        }
    }

    // Search with embeddings
    if let Some(store) = store_guard.as_ref() {
        match store.search(query, 10).await {
            Ok(results) => {
                if results.is_empty() {
                    return ToolResult::ok("No relevant code found");
                }
                
                let output: Vec<String> = results
                    .iter()
                    .map(|(score, chunk)| {
                        format!(
                            "## {}:{}-{} (score: {:.2})\n```\n{}\n```",
                            chunk.file_path,
                            chunk.start_line,
                            chunk.end_line,
                            score,
                            truncate_lines(&chunk.content, 15)
                        )
                    })
                    .collect();
                
                return ToolResult::ok(output.join("\n\n"));
            }
            Err(e) => {
                tracing::warn!("Embedding search failed: {}", e);
            }
        }
    }
    
    // Fallback
    drop(store_guard);
    keyword_search(query, workdir).await
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
