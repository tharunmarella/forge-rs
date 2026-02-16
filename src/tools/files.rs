use super::ToolResult;
use serde_json::Value;
use std::path::Path;
use tokio::fs;
use walkdir::WalkDir;
use regex::Regex;

// ══════════════════════════════════════════════════════════════════
//  MULTI-STRATEGY EDIT MATCHING
//  Ported from forge-ide: tries Exact -> Flexible -> Regex
// ══════════════════════════════════════════════════════════════════

/// Which replacement strategy succeeded.
#[derive(Debug, Clone, Copy)]
enum MatchStrategy {
    Exact,
    Flexible,
    Regex,
}

impl std::fmt::Display for MatchStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchStrategy::Exact => write!(f, "exact"),
            MatchStrategy::Flexible => write!(f, "flexible"),
            MatchStrategy::Regex => write!(f, "regex"),
        }
    }
}

/// Result of a successful replacement.
struct ReplacementResult {
    new_content: String,
    #[allow(dead_code)]
    occurrences: usize,
    strategy: MatchStrategy,
}

/// Read file contents
pub async fn read(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };

    let full_path = workdir.join(path);
    
    let content = match fs::read_to_string(&full_path).await {
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

    // Create parent directories
    if let Some(parent) = full_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ToolResult::err(format!("Failed to create directories: {e}"));
        }
    }

    match std::fs::write(&full_path, content) {
        Ok(_) => ToolResult::ok(format!("Created {path} ({} bytes)", content.len())),
        Err(e) => ToolResult::err(format!("Failed to write: {e}")),
    }
}

/// Try all 3 strategies in order: exact -> flexible -> regex.
/// Returns None if no strategy matched.
fn try_replace(content: &str, old_str: &str, new_str: &str) -> Option<ReplacementResult> {
    // Strategy 1: Exact match
    if let Some(r) = try_exact_replace(content, old_str, new_str) {
        return Some(r);
    }
    // Strategy 2: Flexible (whitespace-tolerant) match
    if let Some(r) = try_flexible_replace(content, old_str, new_str) {
        return Some(r);
    }
    // Strategy 3: Regex-based flexible match
    if let Some(r) = try_regex_replace(content, old_str, new_str) {
        return Some(r);
    }
    None
}

/// Strategy 1: Exact literal match (current behavior).
fn try_exact_replace(content: &str, old_str: &str, new_str: &str) -> Option<ReplacementResult> {
    let count = content.matches(old_str).count();
    if count == 1 {
        Some(ReplacementResult {
            new_content: content.replacen(old_str, new_str, 1),
            occurrences: 1,
            strategy: MatchStrategy::Exact,
        })
    } else if count > 1 {
        // Multiple matches -- can't use exact, but return None to try other strategies
        None
    } else {
        None
    }
}

/// Strategy 2: Flexible whitespace-tolerant match.
/// Strips leading whitespace from each line, matches by trimmed content,
/// then applies replacement preserving original indentation.
fn try_flexible_replace(content: &str, old_str: &str, new_str: &str) -> Option<ReplacementResult> {
    // Split source into lines preserving line endings
    let source_lines: Vec<&str> = content.lines().collect();
    let search_lines: Vec<&str> = old_str.lines().collect();
    let replace_lines: Vec<&str> = new_str.lines().collect();

    if search_lines.is_empty() {
        return None;
    }

    let search_stripped: Vec<&str> = search_lines.iter().map(|l| l.trim()).collect();

    let mut occurrences = 0;
    let mut match_positions: Vec<usize> = Vec::new();

    // Slide a window over source lines
    if source_lines.len() >= search_stripped.len() {
        for i in 0..=(source_lines.len() - search_stripped.len()) {
            let window_stripped: Vec<&str> = source_lines[i..i + search_stripped.len()]
                .iter()
                .map(|l| l.trim())
                .collect();
            if window_stripped == search_stripped {
                occurrences += 1;
                match_positions.push(i);
            }
        }
    }

    if occurrences != 1 {
        return None; // 0 or multiple matches
    }

    let pos = match_positions[0];

    // Detect indentation from the first line of the match
    let first_match_line = source_lines[pos];
    let indentation = &first_match_line[..first_match_line.len() - first_match_line.trim_start().len()];

    // Build replacement with original indentation
    let indented_replacement: Vec<String> = replace_lines
        .iter()
        .enumerate()
        .map(|(j, line)| {
            if j == 0 && line.trim().is_empty() {
                line.to_string()
            } else {
                format!("{}{}", indentation, line.trim_start())
            }
        })
        .collect();

    // Reconstruct content
    let mut new_lines: Vec<String> = Vec::with_capacity(source_lines.len());
    for line in &source_lines[..pos] {
        new_lines.push(line.to_string());
    }
    for line in &indented_replacement {
        new_lines.push(line.clone());
    }
    for line in &source_lines[pos + search_stripped.len()..] {
        new_lines.push(line.to_string());
    }

    // Preserve trailing newline
    let had_trailing_newline = content.ends_with('\n');
    let mut new_content = new_lines.join("\n");
    if had_trailing_newline && !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    Some(ReplacementResult {
        new_content,
        occurrences: 1,
        strategy: MatchStrategy::Flexible,
    })
}

/// Strategy 3: Regex-based flexible match.
/// Tokenizes old_str around delimiters, joins with \s*, matches with regex.
fn try_regex_replace(content: &str, old_str: &str, new_str: &str) -> Option<ReplacementResult> {
    let delimiters = ['(', ')', ':', '[', ']', '{', '}', '>', '<', '='];

    // Tokenize: split around delimiters by inserting spaces around them
    let mut processed = old_str.to_string();
    for delim in &delimiters {
        processed = processed.replace(*delim, &format!(" {} ", delim));
    }

    // Split by whitespace and filter empties
    let tokens: Vec<&str> = processed.split_whitespace().filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return None;
    }

    // Escape each token for regex and join with \s*
    let escaped: Vec<String> = tokens.iter().map(|t| regex::escape(t)).collect();
    let pattern = format!(r"(?m)^(\s*){}", escaped.join(r"\s*"));

    let re = match Regex::new(&pattern) {
        Ok(r) => r,
        Err(_) => return None,
    };

    let mat = re.find(content)?;

    // Extract indentation from the match
    let matched_text = mat.as_str();
    let indentation = &matched_text[..matched_text.len() - matched_text.trim_start().len()];

    // Build replacement with indentation
    let replace_lines: Vec<&str> = new_str.lines().collect();
    let indented: Vec<String> = replace_lines
        .iter()
        .map(|line| format!("{}{}", indentation, line))
        .collect();
    let replacement = indented.join("\n");

    // Replace only the first occurrence
    let new_content = format!(
        "{}{}{}",
        &content[..mat.start()],
        replacement,
        &content[mat.end()..],
    );

    Some(ReplacementResult {
        new_content,
        occurrences: 1,
        strategy: MatchStrategy::Regex,
    })
}

/// Replace text in file using multi-strategy matching
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

    // Try multi-strategy replacement
    match try_replace(&content, old_str, new_str) {
        Some(result) => {
            match std::fs::write(&full_path, &result.new_content) {
                Ok(_) => ToolResult::ok(format!(
                    "Replaced text in {path} using {} strategy", 
                    result.strategy
                )),
                Err(e) => ToolResult::err(format!("Failed to write {path}: {e}")),
            }
        }
        None => {
            // Count exact matches for better error message
            let exact_count = content.matches(old_str).count();
            if exact_count == 0 {
                ToolResult::err(format!(
                    "String not found in {path}. Tried exact, flexible, and regex matching strategies."
                ))
            } else if exact_count > 1 {
                ToolResult::err(format!(
                    "Multiple matches ({}) found in {path}. Please provide more specific context to make the match unique.",
                    exact_count
                ))
            } else {
                ToolResult::err(format!("Failed to match text in {path} with any strategy"))
            }
        }
    }
}

/// Apply unified diff patch
pub async fn apply_patch(args: &Value, workdir: &Path) -> ToolResult {
    // Check if this is V4A format (has "*** Begin Patch" or "*** Update File:")
    if let Some(input) = args.get("input").and_then(|v| v.as_str()) {
        if input.contains("*** Begin Patch") || input.contains("*** Update File:") 
            || input.contains("*** Add File:") || input.contains("*** Delete File:") {
            return apply_v4a_patch(args, workdir).await;
        }
    }
    
    // Traditional unified diff format
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
            ToolResult::ok(format!("Patched {path}"))
        }
        Err(e) => ToolResult::err(format!("Failed to apply patch: {e}")),
    }
}

/// Delete a file or directory
pub async fn delete(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    
    let full_path = workdir.join(path);
    
    // Security: prevent deletion outside workdir
    if !full_path.starts_with(workdir) {
        return ToolResult::err("Cannot delete files outside workspace");
    }
    
    // Prevent deleting critical files
    let dangerous_paths = [".git", "node_modules", "target", ".env", "Cargo.toml", "package.json"];
    for dangerous in dangerous_paths {
        if path == dangerous || path.ends_with(&format!("/{}", dangerous)) {
            return ToolResult::err(format!("Refusing to delete protected path: {}", dangerous));
        }
    }
    
    if !full_path.exists() {
        return ToolResult::err(format!("Path does not exist: {}", path));
    }
    
    if full_path.is_dir() {
        // Check if directory is empty or small
        let entry_count: usize = std::fs::read_dir(&full_path)
            .map(|entries| entries.count())
            .unwrap_or(0);
        
        if entry_count > 10 {
            return ToolResult::err(format!(
                "Directory has {} entries. Use execute_command with 'rm -rf' for large deletions.",
                entry_count
            ));
        }
        
        match std::fs::remove_dir_all(&full_path) {
            Ok(_) => ToolResult::ok(format!("Deleted directory: {}", path)),
            Err(e) => ToolResult::err(format!("Failed to delete directory: {}", e)),
        }
    } else {
        match std::fs::remove_file(&full_path) {
            Ok(_) => ToolResult::ok(format!("Deleted file: {}", path)),
            Err(e) => ToolResult::err(format!("Failed to delete file: {}", e)),
        }
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

/// Apply a V4A format patch (used by apply_patch tool)
/// Format:
/// *** Begin Patch
/// *** Update File: path/to/file
/// @@ class ClassName (optional context)
/// context line
/// - removed line
/// + added line
/// context line
/// *** End Patch
pub async fn apply_v4a_patch(args: &Value, workdir: &Path) -> ToolResult {
    let Some(input) = args.get("input").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'input' parameter");
    };
    
    // Extract patch content between *** Begin Patch and *** End Patch
    let patch_start = input.find("*** Begin Patch");
    let patch_end = input.find("*** End Patch");
    
    let patch_content = match (patch_start, patch_end) {
        (Some(start), Some(end)) => &input[start..end + "*** End Patch".len()],
        _ => input, // Try to parse the whole input
    };
    
    let mut results = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_action: Option<&str> = None;
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current_hunk = Hunk::default();
    
    for line in patch_content.lines() {
        let line = line.trim_end();
        
        // File headers
        if line.starts_with("*** Add File:") {
            // Process previous file if any
            if let Some(ref file) = current_file {
                let result = apply_hunks_to_file(file, &hunks, current_action, workdir);
                results.push(result);
            }
            current_file = Some(line.trim_start_matches("*** Add File:").trim().to_string());
            current_action = Some("add");
            hunks.clear();
            current_hunk = Hunk::default();
        } else if line.starts_with("*** Update File:") {
            if let Some(ref file) = current_file {
                let result = apply_hunks_to_file(file, &hunks, current_action, workdir);
                results.push(result);
            }
            current_file = Some(line.trim_start_matches("*** Update File:").trim().to_string());
            current_action = Some("update");
            hunks.clear();
            current_hunk = Hunk::default();
        } else if line.starts_with("*** Delete File:") {
            if let Some(ref file) = current_file {
                let result = apply_hunks_to_file(file, &hunks, current_action, workdir);
                results.push(result);
            }
            current_file = Some(line.trim_start_matches("*** Delete File:").trim().to_string());
            current_action = Some("delete");
            hunks.clear();
            current_hunk = Hunk::default();
        } else if line.starts_with("@@") {
            // Context marker - save current hunk if it has content
            if !current_hunk.is_empty() {
                hunks.push(current_hunk.clone());
                current_hunk = Hunk::default();
            }
            current_hunk.context_markers.push(line.trim_start_matches("@@").trim().to_string());
        } else if line.starts_with("*** Begin Patch") || line.starts_with("*** End Patch") {
            // Ignore markers
        } else if line.starts_with('-') && !line.starts_with("---") {
            current_hunk.removals.push(line[1..].to_string());
            current_hunk.lines.push(HunkLine::Remove(line[1..].to_string()));
        } else if line.starts_with('+') && !line.starts_with("+++") {
            current_hunk.additions.push(line[1..].to_string());
            current_hunk.lines.push(HunkLine::Add(line[1..].to_string()));
        } else if current_file.is_some() && !line.is_empty() {
            // Context line
            current_hunk.context.push(line.to_string());
            current_hunk.lines.push(HunkLine::Context(line.to_string()));
        }
    }
    
    // Process last file
    if let Some(ref file) = current_file {
        if !current_hunk.is_empty() {
            hunks.push(current_hunk);
        }
        let result = apply_hunks_to_file(file, &hunks, current_action, workdir);
        results.push(result);
    }
    
    if results.is_empty() {
        return ToolResult::err("No valid patch content found");
    }
    
    let success = results.iter().all(|r| r.0);
    let output = results.iter().map(|r| r.1.clone()).collect::<Vec<_>>().join("\n");
    
    if success {
        ToolResult::ok(output)
    } else {
        ToolResult { success: false, output }
    }
}

#[derive(Default, Clone)]
struct Hunk {
    context_markers: Vec<String>,
    context: Vec<String>,
    removals: Vec<String>,
    additions: Vec<String>,
    lines: Vec<HunkLine>,
}

#[derive(Clone)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

impl Hunk {
    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

fn apply_hunks_to_file(file_path: &str, hunks: &[Hunk], action: Option<&str>, workdir: &Path) -> (bool, String) {
    let full_path = workdir.join(file_path);
    
    match action {
        Some("delete") => {
            match std::fs::remove_file(&full_path) {
                Ok(_) => (true, format!("Deleted {}", file_path)),
                Err(e) => (false, format!("Failed to delete {}: {}", file_path, e)),
            }
        }
        Some("add") => {
            // Collect all addition lines
            let content: String = hunks.iter()
                .flat_map(|h| h.additions.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            
            // Create parent directories
            if let Some(parent) = full_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            
            match std::fs::write(&full_path, content) {
                Ok(_) => (true, format!("Created {}", file_path)),
                Err(e) => (false, format!("Failed to create {}: {}", file_path, e)),
            }
        }
        Some("update") | _ => {
            // Read existing file
            let content = match std::fs::read_to_string(&full_path) {
                Ok(c) => c,
                Err(e) => return (false, format!("Failed to read {}: {}", file_path, e)),
            };
            
            let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            
            for hunk in hunks {
                // Find the location to apply this hunk
                if let Some(pos) = find_hunk_position(&lines, hunk) {
                    // Apply the hunk
                    let mut new_lines = Vec::new();
                    let mut i = 0;
                    let mut in_hunk = false;
                    let mut hunk_line_idx = 0;
                    
                    while i < lines.len() {
                        if i == pos && !in_hunk {
                            in_hunk = true;
                        }
                        
                        if in_hunk && hunk_line_idx < hunk.lines.len() {
                            match &hunk.lines[hunk_line_idx] {
                                HunkLine::Context(_) => {
                                    new_lines.push(lines[i].clone());
                                    i += 1;
                                    hunk_line_idx += 1;
                                }
                                HunkLine::Remove(_) => {
                                    // Skip this line (remove it)
                                    i += 1;
                                    hunk_line_idx += 1;
                                }
                                HunkLine::Add(content) => {
                                    new_lines.push(content.clone());
                                    hunk_line_idx += 1;
                                    // Don't increment i - we're inserting
                                }
                            }
                        } else {
                            new_lines.push(lines[i].clone());
                            i += 1;
                            if in_hunk {
                                in_hunk = false;
                            }
                        }
                    }
                    
                    // Handle remaining additions at the end
                    while hunk_line_idx < hunk.lines.len() {
                        if let HunkLine::Add(content) = &hunk.lines[hunk_line_idx] {
                            new_lines.push(content.clone());
                        }
                        hunk_line_idx += 1;
                    }
                    
                    lines = new_lines;
                } else {
                    return (false, format!("Could not find location to apply hunk in {}", file_path));
                }
            }
            
            // Write back
            match std::fs::write(&full_path, lines.join("\n")) {
                Ok(_) => (true, format!("Updated {}", file_path)),
                Err(e) => (false, format!("Failed to write {}: {}", file_path, e)),
            }
        }
    }
}

fn find_hunk_position(lines: &[String], hunk: &Hunk) -> Option<usize> {
    // Get the context lines before the first change
    let mut context_before = Vec::new();
    for line in &hunk.lines {
        match line {
            HunkLine::Context(c) => context_before.push(c.clone()),
            HunkLine::Remove(_) | HunkLine::Add(_) => break,
        }
    }
    
    if context_before.is_empty() {
        // No context, try to match the first removal line
        if let Some(HunkLine::Remove(first_remove)) = hunk.lines.iter().find(|l| matches!(l, HunkLine::Remove(_))) {
            for (i, line) in lines.iter().enumerate() {
                if line.trim() == first_remove.trim() {
                    return Some(i);
                }
            }
        }
        return Some(0); // Default to start of file
    }
    
    // Search for matching context
    'outer: for i in 0..lines.len() {
        if i + context_before.len() > lines.len() {
            break;
        }
        
        for (j, ctx) in context_before.iter().enumerate() {
            if lines[i + j].trim() != ctx.trim() {
                continue 'outer;
            }
        }
        
        return Some(i);
    }
    
    None
}
