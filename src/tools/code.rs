use super::treesitter;
use super::ToolResult;
use serde_json::Value;
use std::path::Path;

/// List code definitions (functions, classes, etc.) using tree-sitter
pub async fn list_definitions(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };

    let full_path = workdir.join(path);
    
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

/// Get symbol definition using tree-sitter
pub async fn get_definition(args: &Value, workdir: &Path) -> ToolResult {
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'symbol' parameter");
    };

    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let search_path = workdir.join(path);

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

/// Find symbol references
pub async fn find_references(args: &Value, workdir: &Path) -> ToolResult {
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'symbol' parameter");
    };

    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let search_path = workdir.join(path);

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
