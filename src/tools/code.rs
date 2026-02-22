use super::treesitter;
use super::ToolResult;
use crate::lsp::LspManager;
use crate::code_graph::{trace, impact};
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;
use tokio::sync::Mutex;

// Global LSP manager (lazy initialized)
static LSP_MANAGER: OnceLock<Mutex<Option<LspManager>>> = OnceLock::new();

// Global CodeGraph (lazy initialized)
static CODE_GRAPH: OnceLock<Mutex<Option<crate::code_graph::CodeGraph>>> = OnceLock::new();

/// Initialize or get the LSP manager
async fn get_lsp_manager(workdir: &Path) -> Option<&'static Mutex<Option<LspManager>>> {
    let manager = LSP_MANAGER.get_or_init(|| {
        Mutex::new(Some(LspManager::new(workdir.to_path_buf())))
    });
    Some(manager)
}

/// Get or initialize the code graph
async fn get_code_graph(_workdir: &Path) -> Option<&'static Mutex<Option<crate::code_graph::CodeGraph>>> {
    let graph = CODE_GRAPH.get_or_init(|| {
        // In a real scenario, we would build the graph here using RepoMap
        // For now, we return an empty graph that will be populated during indexing
        Mutex::new(Some(crate::code_graph::CodeGraph::new()))
    });
    Some(graph)
}

/// Trace call chain upstream or downstream
pub async fn trace_call_chain(args: &Value, workdir: &Path) -> ToolResult {
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'symbol' parameter");
    };

    let direction = args.get("direction").and_then(|v| v.as_str()).unwrap_or("downstream");
    let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

    if let Some(graph_lock) = get_code_graph(workdir).await {
        let graph = graph_lock.lock().await;
        if let Some(ref g) = *graph {
            let result = trace::trace_calls(g, symbol, direction, max_depth);
            return ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default());
        }
    }

    ToolResult::err("Code graph not available")
}

/// Analyze impact of changing a symbol
pub async fn impact_analysis(args: &Value, workdir: &Path) -> ToolResult {
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'symbol' parameter");
    };

    let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

    if let Some(graph_lock) = get_code_graph(workdir).await {
        let graph = graph_lock.lock().await;
        if let Some(ref g) = *graph {
            let result = impact::analyze_impact(g, symbol, max_depth);
            return ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default());
        }
    }

    ToolResult::err("Code graph not available")
}

/// List code definitions (functions, classes, etc.) using LSP or tree-sitter
pub async fn list_definitions(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };

    let full_path = workdir.join(path);
    
    // Try LSP first
    if let Some(manager_lock) = get_lsp_manager(workdir).await {
        let manager = manager_lock.lock().await;
        if let Some(ref mgr) = *manager {
            if let Some(symbols) = mgr.get_document_symbols(&full_path).await {
                return ToolResult::ok(serde_json::to_string_pretty(&symbols).unwrap_or_default());
            }
        }
    }

    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to read {path}: {e}")),
    };

    let ext = full_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    // Use tree-sitter for supported languages
    match treesitter::parse_definitions(&content, ext) {
        Ok(symbols) if !symbols.is_empty() => {
            let output: Vec<String> = symbols
                .iter()
                .map(|s| format!("{:4}: {} {} - {}", s.start_line, s.kind, s.name, s.signature))
                .collect();
            ToolResult::ok(output.join("\n"))
        }
        Ok(_) => {
            // Fallback to regex for unsupported languages
            let definitions = extract_definitions_regex(&content, ext);
            if definitions.is_empty() {
                ToolResult::ok("No definitions found")
            } else {
                ToolResult::ok(definitions.join("\n"))
            }
        }
        Err(_) => {
            // Fallback to regex
            let definitions = extract_definitions_regex(&content, ext);
            if definitions.is_empty() {
                ToolResult::ok("No definitions found")
            } else {
                ToolResult::ok(definitions.join("\n"))
            }
        }
    }
}

/// Get symbol definition using LSP (with tree-sitter/regex fallback)
pub async fn get_definition(args: &Value, workdir: &Path) -> ToolResult {
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'symbol' parameter");
    };

    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let line = args.get("line").and_then(|v| v.as_u64()).map(|v| v as u32);
    let character = args.get("character").and_then(|v| v.as_u64()).map(|v| v as u32);
    let search_path = workdir.join(path);
    
    // Try LSP first if we have line/character position
    if let (Some(line), Some(character)) = (line, character) {
        if search_path.is_file() {
            if let Some(manager_lock) = get_lsp_manager(workdir).await {
                let manager = manager_lock.lock().await;
                if let Some(ref mgr) = *manager {
                    if let Some(locations) = mgr.go_to_definition(&search_path, line, character).await {
                        if !locations.is_empty() {
                            let mut results = Vec::new();
                            for loc in locations.iter().take(5) {
                                if let Some(file_path) = loc.file_path() {
                                    let rel_path = Path::new(&file_path)
                                        .strip_prefix(workdir)
                                        .unwrap_or(Path::new(&file_path));
                                    
                                    // Read context around the definition
                                    if let Ok(content) = std::fs::read_to_string(&file_path) {
                                        let lines: Vec<&str> = content.lines().collect();
                                        let start = loc.range.start.line as usize;
                                        let end = (loc.range.end.line as usize + 5).min(lines.len());
                                        
                                        let context: String = lines[start..end]
                                            .iter()
                                            .enumerate()
                                            .map(|(i, l)| format!("{:4}|{}", start + i + 1, l))
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        
                                        results.push(format!(
                                            "{}:{}\n{}",
                                            rel_path.display(),
                                            loc.range.start.line + 1,
                                            context
                                        ));
                                    } else {
                                        results.push(format!(
                                            "{}:{}:{}",
                                            rel_path.display(),
                                            loc.range.start.line + 1,
                                            loc.range.start.character + 1
                                        ));
                                    }
                                }
                            }
                            if !results.is_empty() {
                                return ToolResult::ok(format!("Found via LSP:\n{}", results.join("\n\n")));
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Fallback to tree-sitter/regex search by symbol name

    // Search files with tree-sitter
    for entry in walkdir::WalkDir::new(&search_path)
        .max_depth(10)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_path = entry.path();
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        
        if let Ok(content) = std::fs::read_to_string(file_path) {
            // Try tree-sitter first
            if let Some(sym) = treesitter::find_definition(&content, ext, symbol) {
                let rel_path = file_path.strip_prefix(workdir).unwrap_or(file_path);
                let lines: Vec<&str> = content.lines().collect();
                
                let start = sym.start_line.saturating_sub(2);
                let end = (sym.end_line + 5).min(lines.len());
                
                let context: String = lines[start..end]
                    .iter()
                    .enumerate()
                    .map(|(i, l)| format!("{:4}|{}", start + i + 1, l))
                    .collect::<Vec<_>>()
                    .join("\n");

                return ToolResult::ok(format!(
                    "Found {} '{}' in {}:{}\n{}",
                    sym.kind,
                    sym.name,
                    rel_path.display(),
                    sym.start_line,
                    context
                ));
            }
        }
    }

    // Fallback to regex search
    let patterns = vec![
        format!(r"(?:fn|func|def|function)\s+{}\s*\(", regex::escape(symbol)),
        format!(r"(?:class|struct|type|interface)\s+{}\s*[{{\(<:]", regex::escape(symbol)),
        format!(r"(?:const|let|var)\s+{}\s*[:=]", regex::escape(symbol)),
    ];

    for pattern in patterns {
        if let Ok(regex) = regex::Regex::new(&pattern) {
            for entry in walkdir::WalkDir::new(&search_path)
                .max_depth(10)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    for (line_num, line) in content.lines().enumerate() {
                        if regex.is_match(line) {
                            let rel_path = entry.path().strip_prefix(workdir).unwrap_or(entry.path());
                            let lines: Vec<&str> = content.lines().collect();
                            let start = line_num.saturating_sub(2);
                            let end = (line_num + 10).min(lines.len());
                            
                            let context: String = lines[start..end]
                                .iter()
                                .enumerate()
                                .map(|(i, l)| format!("{:4}|{}", start + i + 1, l))
                                .collect::<Vec<_>>()
                                .join("\n");

                            return ToolResult::ok(format!(
                                "Found in {}:{}\n{}",
                                rel_path.display(),
                                line_num + 1,
                                context
                            ));
                        }
                    }
                }
            }
        }
    }

    ToolResult::err(format!("Definition for '{}' not found", symbol))
}

/// Find symbol references using LSP (with regex fallback)
pub async fn find_references(args: &Value, workdir: &Path) -> ToolResult {
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'symbol' parameter");
    };

    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let line = args.get("line").and_then(|v| v.as_u64()).map(|v| v as u32);
    let character = args.get("character").and_then(|v| v.as_u64()).map(|v| v as u32);
    let search_path = workdir.join(path);
    
    // Try LSP first if we have line/character position
    if let (Some(line), Some(character)) = (line, character) {
        if search_path.is_file() {
            if let Some(manager_lock) = get_lsp_manager(workdir).await {
                let manager = manager_lock.lock().await;
                if let Some(ref mgr) = *manager {
                    if let Some(locations) = mgr.find_references(&search_path, line, character).await {
                        if !locations.is_empty() {
                            let mut results: Vec<String> = locations.iter()
                                .take(50)
                                .filter_map(|loc| {
                                    let file_path = loc.file_path()?;
                                    let rel_path = Path::new(&file_path)
                                        .strip_prefix(workdir)
                                        .unwrap_or(Path::new(&file_path));
                                    
                                    // Get the line content
                                    let line_content = std::fs::read_to_string(&file_path).ok()
                                        .and_then(|content| {
                                            content.lines()
                                                .nth(loc.range.start.line as usize)
                                                .map(|l| l.trim().to_string())
                                        })
                                        .unwrap_or_default();
                                    
                                    Some(format!(
                                        "{}:{}:{}",
                                        rel_path.display(),
                                        loc.range.start.line + 1,
                                        line_content
                                    ))
                                })
                                .collect();
                            
                            if !results.is_empty() {
                                if locations.len() > 50 {
                                    results.push("... (truncated)".to_string());
                                }
                                return ToolResult::ok(format!(
                                    "Found {} references via LSP:\n{}",
                                    locations.len(),
                                    results.join("\n")
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Fallback to regex search
    let pattern = format!(r"\b{}\b", regex::escape(symbol));
    let regex = match regex::Regex::new(&pattern) {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("Invalid symbol: {e}")),
    };

    let mut results = Vec::new();

    for entry in walkdir::WalkDir::new(&search_path)
        .max_depth(10)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_path = entry.path();
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        
        if file_name.starts_with('.') {
            continue;
        }

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
                    
                    if results.len() >= 50 {
                        results.push("... (truncated)".to_string());
                        return ToolResult::ok(results.join("\n"));
                    }
                }
            }
        }
    }

    if results.is_empty() {
        ToolResult::ok(format!("No references to '{}' found", symbol))
    } else {
        ToolResult::ok(format!("Found {} references:\n{}", results.len(), results.join("\n")))
    }
}

/// Extract definitions from source code (regex fallback)
fn extract_definitions_regex(content: &str, ext: &str) -> Vec<String> {
    let mut defs = Vec::new();

    let patterns: Vec<(&str, &str)> = match ext {
        "rs" => vec![
            (r"(?m)^pub\s+(?:async\s+)?fn\s+(\w+)", "fn"),
            (r"(?m)^pub\s+struct\s+(\w+)", "struct"),
            (r"(?m)^pub\s+enum\s+(\w+)", "enum"),
            (r"(?m)^pub\s+trait\s+(\w+)", "trait"),
            (r"(?m)^impl(?:<[^>]+>)?\s+(?:\w+\s+for\s+)?(\w+)", "impl"),
        ],
        "py" => vec![
            (r"(?m)^def\s+(\w+)\s*\(", "def"),
            (r"(?m)^class\s+(\w+)", "class"),
            (r"(?m)^async\s+def\s+(\w+)", "async def"),
        ],
        "ts" | "tsx" | "js" | "jsx" => vec![
            (r"(?m)^(?:export\s+)?(?:async\s+)?function\s+(\w+)", "function"),
            (r"(?m)^(?:export\s+)?class\s+(\w+)", "class"),
            (r"(?m)^(?:export\s+)?interface\s+(\w+)", "interface"),
            (r"(?m)^(?:export\s+)?type\s+(\w+)", "type"),
            (r"(?m)^(?:export\s+)?const\s+(\w+)\s*=\s*(?:async\s+)?\(", "const fn"),
        ],
        "go" => vec![
            (r"(?m)^func\s+(?:\([^)]+\)\s+)?(\w+)\s*\(", "func"),
            (r"(?m)^type\s+(\w+)\s+struct", "struct"),
            (r"(?m)^type\s+(\w+)\s+interface", "interface"),
        ],
        _ => vec![
            (r"(?m)^(?:pub\s+)?(?:fn|func|def|function)\s+(\w+)", "fn"),
            (r"(?m)^(?:pub\s+)?(?:class|struct|type)\s+(\w+)", "type"),
        ],
    };

    for (pattern, kind) in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            for (line_num, line) in content.lines().enumerate() {
                if let Some(caps) = re.captures(line) {
                    if let Some(name) = caps.get(1) {
                        defs.push(format!("{:4}: {} {}", line_num + 1, kind, name.as_str()));
                    }
                }
            }
        }
    }

    defs.sort_by_key(|d| d.split(':').next().unwrap_or("0").trim().parse::<usize>().unwrap_or(0));
    defs
}

/// Search for function/method definitions by name
pub async fn search_functions(args: &Value, workdir: &Path) -> ToolResult {
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'query' parameter");
    };
    let search_path = args.get("path").and_then(|v| v.as_str())
        .map(|p| workdir.join(p))
        .unwrap_or_else(|| workdir.to_path_buf());

    let patterns = [
        format!(r"(?:pub\s+)?(?:async\s+)?fn\s+{}", regex::escape(query)),     // Rust
        format!(r"(?:async\s+)?def\s+{}\s*\(", regex::escape(query)),          // Python
        format!(r"(?:async\s+)?function\s+{}\s*[\(<]", regex::escape(query)),  // JS/TS
        format!(r"func\s+(?:\([^)]+\)\s+)?{}\s*\(", regex::escape(query)),     // Go
    ];

    let grep_args = serde_json::json!({
        "pattern": patterns.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("|"),
        "path": search_path.to_string_lossy(),
        "context": 1
    });
    super::search::grep(&grep_args, workdir).await
}

/// Search for struct/class/interface/type definitions by name
pub async fn search_classes(args: &Value, workdir: &Path) -> ToolResult {
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'query' parameter");
    };
    let search_path = args.get("path").and_then(|v| v.as_str())
        .map(|p| workdir.join(p))
        .unwrap_or_else(|| workdir.to_path_buf());

    let escaped = regex::escape(query);
    let pattern = format!(
        r"(?:pub\s+)?(?:struct|enum|trait|class|interface|type)\s+{}",
        escaped
    );

    let grep_args = serde_json::json!({
        "pattern": pattern,
        "path": search_path.to_string_lossy(),
        "context": 1
    });
    super::search::grep(&grep_args, workdir).await
}

/// Find files by name pattern
pub async fn search_files(args: &Value, workdir: &Path) -> ToolResult {
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'query' parameter");
    };
    let base = args.get("path").and_then(|v| v.as_str())
        .map(|p| workdir.join(p))
        .unwrap_or_else(|| workdir.to_path_buf());

    // Build a glob: if query already contains * use as-is, else wrap with **/*query*
    let pattern = if query.contains('*') || query.contains('/') {
        query.to_string()
    } else {
        format!("**/*{}*", query)
    };

    let glob_args = serde_json::json!({ "pattern": pattern, "path": base.to_string_lossy() });
    super::search::glob_search(&glob_args, workdir).await
}

/// LSP go-to-definition at exact position
pub async fn lsp_go_to_definition(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let character = args.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let full_path = workdir.join(path);

    let Some(mgr_mutex) = get_lsp_manager(workdir).await else {
        return ToolResult::err("LSP manager unavailable");
    };
    let guard = mgr_mutex.lock().await;
    let Some(ref mgr) = *guard else {
        return ToolResult::err("LSP manager not initialized");
    };

    match mgr.go_to_definition(&full_path, line, character).await {
        Some(locations) if !locations.is_empty() => {
            let mut out = format!("Definition(s) found ({}):\n", locations.len());
            for loc in locations.iter().take(5) {
                let def_path = loc.uri.trim_start_matches("file://");
                let snippet = std::fs::read_to_string(def_path).ok()
                    .and_then(|c| c.lines().nth(loc.range.start.line as usize).map(|l| l.trim().to_string()))
                    .unwrap_or_default();
                out.push_str(&format!("  {}:{} — {}\n", def_path, loc.range.start.line + 1, snippet));
            }
            ToolResult::ok(out)
        }
        _ => ToolResult::err(format!("No definition found at {}:{}:{}", path, line, character)),
    }
}

/// LSP find-all-references at exact position
pub async fn lsp_find_references(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let character = args.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let full_path = workdir.join(path);

    let Some(mgr_mutex) = get_lsp_manager(workdir).await else {
        return ToolResult::err("LSP manager unavailable");
    };
    let guard = mgr_mutex.lock().await;
    let Some(ref mgr) = *guard else {
        return ToolResult::err("LSP manager not initialized");
    };

    match mgr.find_references(&full_path, line, character).await {
        Some(locations) if !locations.is_empty() => {
            let mut out = format!("Found {} reference(s):\n", locations.len());
            for loc in locations.iter().take(50) {
                let ref_path = loc.uri.trim_start_matches("file://");
                let snippet = std::fs::read_to_string(ref_path).ok()
                    .and_then(|c| c.lines().nth(loc.range.start.line as usize).map(|l| l.trim().to_string()))
                    .unwrap_or_default();
                out.push_str(&format!("  {}:{} — {}\n", ref_path, loc.range.start.line + 1, snippet));
            }
            ToolResult::ok(out)
        }
        _ => ToolResult::err(format!("No references found at {}:{}:{}", path, line, character)),
    }
}

/// LSP hover — type info and docs at position
pub async fn lsp_hover(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let character = args.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let full_path = workdir.join(path);

    let Some(mgr_mutex) = get_lsp_manager(workdir).await else {
        return ToolResult::err("LSP manager unavailable");
    };
    let guard = mgr_mutex.lock().await;
    let Some(ref mgr) = *guard else {
        return ToolResult::err("LSP manager not initialized");
    };

    match mgr.hover(&full_path, line, character).await {
        Some(info) => ToolResult::ok(info),
        None => ToolResult::err(format!("No hover info at {}:{}:{}", path, line, character)),
    }
}
