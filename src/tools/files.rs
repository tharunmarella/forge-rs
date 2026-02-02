use super::ToolResult;
use super::ide;
use serde_json::Value;
use std::path::Path;
use walkdir::WalkDir;

/// Read file contents
pub async fn read(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };

    let full_path = workdir.join(path);
    
    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to read {path}: {e}")),
    };

    // Handle line range
    let start = args.get("start_line").and_then(|v| v.as_u64()).map(|n| n as usize);
    let end = args.get("end_line").and_then(|v| v.as_u64()).map(|n| n as usize);

    if start.is_some() || end.is_some() {
        let lines: Vec<&str> = content.lines().collect();
        let start = start.unwrap_or(1).saturating_sub(1);
        let end = end.unwrap_or(lines.len()).min(lines.len());
        
        let output: String = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:4}|{}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");
        
        ToolResult::ok(output)
    } else {
        // Add line numbers
        let output: String = content
            .lines()
            .enumerate()
            .map(|(i, line)| format!("{:4}|{}", i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");
        
        ToolResult::ok(output)
    }
}

/// Write new file
pub async fn write(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    let Some(content) = args.get("content").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'content' parameter");
    };

    let full_path = workdir.join(path);

    // Read old content for diff (if file exists)
    let old_content = std::fs::read_to_string(&full_path).ok();

    // Create parent directories
    if let Some(parent) = full_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ToolResult::err(format!("Failed to create directories: {e}"));
        }
    }

    match std::fs::write(&full_path, content) {
        Ok(_) => {
            // Show diff in IDE if file was modified (not new)
            if let Some(old) = old_content {
                ide::show_diff_in_ide(&full_path, &old, content);
            } else {
                // New file - just open it
                ide::open_file_in_ide(&full_path, None);
            }
            ToolResult::ok(format!("Created {path} ({} bytes)", content.len()))
        }
        Err(e) => ToolResult::err(format!("Failed to write: {e}")),
    }
}

/// Replace text in file
pub async fn replace(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    let Some(old_str) = args.get("old_str").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'old_str' parameter");
    };
    let Some(new_str) = args.get("new_str").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'new_str' parameter");
    };

    let full_path = workdir.join(path);

    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to read {path}: {e}")),
    };

    // Check for exact match
    if !content.contains(old_str) {
        return ToolResult::err(format!(
            "old_str not found in {path}. Make sure it matches exactly including whitespace."
        ));
    }

    // Count occurrences
    let count = content.matches(old_str).count();
    if count > 1 {
        return ToolResult::err(format!(
            "old_str found {count} times in {path}. It must be unique. Add more context."
        ));
    }

    let new_content = content.replacen(old_str, new_str, 1);

    match std::fs::write(&full_path, &new_content) {
        Ok(_) => {
            // Show diff in IDE if available
            ide::show_diff_in_ide(&full_path, &content, &new_content);
            ToolResult::ok(format!("Updated {path}"))
        }
        Err(e) => ToolResult::err(format!("Failed to write: {e}")),
    }
}

/// Apply unified diff patch
pub async fn apply_patch(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    let Some(patch) = args.get("patch").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'patch' parameter");
    };

    let full_path = workdir.join(path);

    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(_) => String::new(), // New file
    };

    // Parse and apply unified diff
    match apply_unified_diff(&content, patch) {
        Ok(new_content) => {
            if let Err(e) = std::fs::write(&full_path, &new_content) {
                return ToolResult::err(format!("Failed to write: {e}"));
            }
            // Show diff in IDE
            ide::show_diff_in_ide(&full_path, &content, &new_content);
            ToolResult::ok(format!("Patched {path}"))
        }
        Err(e) => ToolResult::err(format!("Failed to apply patch: {e}")),
    }
}

/// List files in directory
pub async fn list(args: &Value, workdir: &Path) -> ToolResult {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);

    let full_path = workdir.join(path);

    if !full_path.exists() {
        return ToolResult::err(format!("Path does not exist: {path}"));
    }

    let mut entries = Vec::new();

    if recursive {
        for entry in WalkDir::new(&full_path)
            .max_depth(10)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if let Ok(rel) = entry.path().strip_prefix(&full_path) {
                let rel_str = rel.to_string_lossy();
                if !rel_str.is_empty() {
                    entries.push(rel_str.to_string());
                }
            }
        }
    } else {
        if let Ok(dir) = std::fs::read_dir(&full_path) {
            for entry in dir.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                let suffix = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    "/"
                } else {
                    ""
                };
                entries.push(format!("{name}{suffix}"));
            }
        }
    }

    entries.sort();
    ToolResult::ok(entries.join("\n"))
}

/// Simple unified diff parser
fn apply_unified_diff(original: &str, patch: &str) -> Result<String, String> {
    let mut lines: Vec<String> = original.lines().map(|s| s.to_string()).collect();
    
    let patch_lines: Vec<&str> = patch.lines().collect();
    let mut i = 0;

    while i < patch_lines.len() {
        let line = patch_lines[i];
        
        // Parse hunk header: @@ -start,count +start,count @@
        if line.starts_with("@@") {
            // Extract the line numbers
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                i += 1;
                continue;
            }
            
            let old_range = parts[1].trim_start_matches('-');
            let old_start: usize = old_range
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            
            let mut current_line = old_start.saturating_sub(1);
            i += 1;

            // Process hunk lines
            while i < patch_lines.len() && !patch_lines[i].starts_with("@@") {
                let hunk_line = patch_lines[i];
                
                if hunk_line.starts_with('-') {
                    // Remove line
                    if current_line < lines.len() {
                        lines.remove(current_line);
                    }
                } else if hunk_line.starts_with('+') {
                    // Add line
                    let content = hunk_line.strip_prefix('+').unwrap_or("");
                    lines.insert(current_line, content.to_string());
                    current_line += 1;
                } else if hunk_line.starts_with(' ') || hunk_line.is_empty() {
                    // Context line
                    current_line += 1;
                }
                
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    Ok(lines.join("\n"))
}
