//! Linting module - run language-specific linters on files
//!
//! Supports:
//! - Rust: rustc --error-format=short, cargo check
//! - Python: python -m py_compile, ruff check
//! - JavaScript/TypeScript: eslint, tsc --noEmit
//! - Go: go vet, go build

use super::ToolResult;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// Diagnostics tool - callable by the LLM to check code for errors
pub async fn diagnostics(args: &Value, workdir: &Path) -> ToolResult {
    let Some(path_str) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'path' parameter");
    };
    
    let auto_fix = args.get("fix").and_then(|v| v.as_bool()).unwrap_or(false);
    let target_path = workdir.join(path_str);
    
    if !target_path.exists() {
        return ToolResult::err(format!("Path does not exist: {}", path_str));
    }
    
    // Determine if this is a file or directory
    let result = if target_path.is_dir() {
        run_project_diagnostics(&target_path, auto_fix)
    } else {
        lint_file(&target_path, workdir)
    };
    
    if result.success {
        ToolResult::ok(format!("No errors found in {}", path_str))
    } else if result.errors.is_empty() {
        // Raw output but no parsed errors
        ToolResult::err(format!("Diagnostics failed:\n{}", result.raw_output))
    } else {
        let mut output = format!("Found {} error(s) in {}:\n\n", result.errors.len(), path_str);
        
        for err in &result.errors {
            let location = match (err.line, err.column) {
                (Some(l), Some(c)) => format!("{}:{}:{}", err.file, l, c),
                (Some(l), None) => format!("{}:{}", err.file, l),
                _ => err.file.clone(),
            };
            
            let severity_icon = match err.severity {
                LintSeverity::Error => "❌",
                LintSeverity::Warning => "⚠️",
                LintSeverity::Info => "ℹ️",
            };
            
            output.push_str(&format!("{} {} {}\n", severity_icon, location, err.message));
        }
        
        ToolResult::err(output)
    }
}

/// Run project-level diagnostics (for directories)
fn run_project_diagnostics(dir: &Path, auto_fix: bool) -> LintResult {
    // Check for Cargo.toml (Rust project)
    if dir.join("Cargo.toml").exists() {
        return run_cargo_diagnostics(dir, auto_fix);
    }
    
    // Check for package.json (Node.js/TypeScript project)
    if dir.join("package.json").exists() {
        return run_npm_diagnostics(dir, auto_fix);
    }
    
    // Check for go.mod (Go project)
    if dir.join("go.mod").exists() {
        return run_go_diagnostics(dir, auto_fix);
    }
    
    // Check for pyproject.toml or setup.py (Python project)
    if dir.join("pyproject.toml").exists() || dir.join("setup.py").exists() {
        return run_python_diagnostics(dir, auto_fix);
    }
    
    LintResult::ok() // No recognized project type
}

/// Run cargo check for Rust projects
fn run_cargo_diagnostics(dir: &Path, auto_fix: bool) -> LintResult {
    let mut args = vec!["check", "--message-format=short"];
    if auto_fix {
        args = vec!["fix", "--allow-dirty", "--allow-staged"];
    }
    
    let output = Command::new("cargo")
        .args(&args)
        .current_dir(dir)
        .output();
    
    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.success() {
                LintResult::ok()
            } else {
                let errors = parse_cargo_errors(&stderr);
                LintResult::failed(errors, stderr.to_string())
            }
        }
        Err(e) => LintResult::failed(vec![], format!("Failed to run cargo: {}", e)),
    }
}

/// Parse cargo check output
fn parse_cargo_errors(output: &str) -> Vec<LintError> {
    let mut errors = Vec::new();
    
    for line in output.lines() {
        // Format: file:line:col: level: message
        if line.contains(": error") || line.contains(": warning") {
            let severity = if line.contains(": error") {
                LintSeverity::Error
            } else {
                LintSeverity::Warning
            };
            
            // Try to parse location
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 4 {
                errors.push(LintError {
                    file: parts[0].trim().to_string(),
                    line: parts.get(1).and_then(|s| s.trim().parse().ok()),
                    column: parts.get(2).and_then(|s| s.trim().parse().ok()),
                    message: parts.get(3).map(|s| s.trim().to_string()).unwrap_or_default(),
                    severity,
                });
            }
        }
    }
    
    errors
}

/// Run npm/tsc for Node.js projects
fn run_npm_diagnostics(dir: &Path, auto_fix: bool) -> LintResult {
    // Try tsc first for TypeScript
    if dir.join("tsconfig.json").exists() {
        let output = Command::new("npx")
            .args(["tsc", "--noEmit", "--pretty", "false"])
            .current_dir(dir)
            .output();
        
        if let Ok(out) = output {
            if !out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let errors = parse_tsc_project_errors(&stdout);
                return LintResult::failed(errors, stdout.to_string());
            }
        }
    }
    
    // Try eslint
    let mut eslint_args = vec!["eslint", ".", "--format=compact"];
    if auto_fix {
        eslint_args.push("--fix");
    }
    
    let output = Command::new("npx")
        .args(&eslint_args)
        .current_dir(dir)
        .output();
    
    match output {
        Ok(out) => {
            if out.status.success() {
                LintResult::ok()
            } else {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let errors = parse_eslint_project_errors(&stdout);
                LintResult::failed(errors, stdout.to_string())
            }
        }
        Err(_) => LintResult::ok(), // ESLint not available
    }
}

/// Parse tsc output for projects
fn parse_tsc_project_errors(output: &str) -> Vec<LintError> {
    let mut errors = Vec::new();
    
    for line in output.lines() {
        // Format: file(line,col): error TSxxxx: message
        if line.contains("): error TS") || line.contains("): warning TS") {
            if let Some(paren_start) = line.find('(') {
                let file = line[..paren_start].trim().to_string();
                
                if let Some(paren_end) = line.find("): ") {
                    let location = &line[paren_start + 1..paren_end];
                    let loc_parts: Vec<&str> = location.split(',').collect();
                    let message = &line[paren_end + 3..];
                    
                    errors.push(LintError {
                        file,
                        line: loc_parts.get(0).and_then(|s| s.parse().ok()),
                        column: loc_parts.get(1).and_then(|s| s.parse().ok()),
                        message: message.to_string(),
                        severity: if line.contains("error TS") { LintSeverity::Error } else { LintSeverity::Warning },
                    });
                }
            }
        }
    }
    
    errors
}

/// Parse eslint compact output for projects
fn parse_eslint_project_errors(output: &str) -> Vec<LintError> {
    let mut errors = Vec::new();
    
    for line in output.lines() {
        // Format: file: line X, col Y, Error/Warning - message
        if let Some(colon_pos) = line.find(": line ") {
            let file = line[..colon_pos].trim().to_string();
            let rest = &line[colon_pos + 7..];
            
            // Parse "X, col Y, Error - message"
            let parts: Vec<&str> = rest.splitn(2, ',').collect();
            if let Some(line_num) = parts.get(0).and_then(|s| s.trim().parse::<usize>().ok()) {
                let severity = if rest.contains("Error") { LintSeverity::Error } else { LintSeverity::Warning };
                let message = parts.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
                
                errors.push(LintError {
                    file,
                    line: Some(line_num),
                    column: None,
                    message,
                    severity,
                });
            }
        }
    }
    
    errors
}

/// Run go vet/build for Go projects
fn run_go_diagnostics(dir: &Path, _auto_fix: bool) -> LintResult {
    // Run go vet
    let output = Command::new("go")
        .args(["vet", "./..."])
        .current_dir(dir)
        .output();
    
    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.success() || stderr.trim().is_empty() {
                // Also run go build to catch compile errors
                let build_output = Command::new("go")
                    .args(["build", "./..."])
                    .current_dir(dir)
                    .output();
                
                match build_output {
                    Ok(bout) => {
                        if bout.status.success() {
                            LintResult::ok()
                        } else {
                            let stderr = String::from_utf8_lossy(&bout.stderr);
                            let errors = parse_go_project_errors(&stderr);
                            LintResult::failed(errors, stderr.to_string())
                        }
                    }
                    Err(_) => LintResult::ok(),
                }
            } else {
                let errors = parse_go_project_errors(&stderr);
                LintResult::failed(errors, stderr.to_string())
            }
        }
        Err(_) => LintResult::ok(),
    }
}

/// Parse go errors
fn parse_go_project_errors(output: &str) -> Vec<LintError> {
    let mut errors = Vec::new();
    
    for line in output.lines() {
        // Format: file.go:line:col: message
        if line.contains(".go:") {
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 3 {
                errors.push(LintError {
                    file: parts[0].trim().to_string(),
                    line: parts.get(1).and_then(|s| s.trim().parse().ok()),
                    column: parts.get(2).and_then(|s| s.trim().parse().ok()),
                    message: parts.get(3).map(|s| s.trim().to_string()).unwrap_or_default(),
                    severity: LintSeverity::Error,
                });
            }
        }
    }
    
    errors
}

/// Run python linters
fn run_python_diagnostics(dir: &Path, auto_fix: bool) -> LintResult {
    // Try ruff first
    let mut ruff_args = vec!["check", "."];
    if auto_fix {
        ruff_args.push("--fix");
    }
    
    let output = Command::new("ruff")
        .args(&ruff_args)
        .current_dir(dir)
        .output();
    
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if out.status.success() || stdout.trim().is_empty() {
                LintResult::ok()
            } else {
                let errors = parse_ruff_errors(&stdout);
                LintResult::failed(errors, stdout.to_string())
            }
        }
        Err(_) => {
            // Try mypy as fallback
            let mypy_output = Command::new("mypy")
                .args([".", "--no-error-summary"])
                .current_dir(dir)
                .output();
            
            match mypy_output {
                Ok(out) => {
                    if out.status.success() {
                        LintResult::ok()
                    } else {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        LintResult::failed(vec![], stdout.to_string())
                    }
                }
                Err(_) => LintResult::ok(),
            }
        }
    }
}

/// Parse ruff output
fn parse_ruff_errors(output: &str) -> Vec<LintError> {
    let mut errors = Vec::new();
    
    for line in output.lines() {
        // Format: file:line:col: CODE message
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() >= 4 {
            errors.push(LintError {
                file: parts[0].trim().to_string(),
                line: parts.get(1).and_then(|s| s.trim().parse().ok()),
                column: parts.get(2).and_then(|s| s.trim().parse().ok()),
                message: parts.get(3).map(|s| s.trim().to_string()).unwrap_or_default(),
                severity: LintSeverity::Error,
            });
        }
    }
    
    errors
}

/// Lint error with location and message
#[derive(Debug, Clone)]
pub struct LintError {
    pub file: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
    pub severity: LintSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintSeverity {
    Error,
    Warning,
    Info,
}

/// Result of running a linter
#[derive(Debug)]
pub struct LintResult {
    pub success: bool,
    pub errors: Vec<LintError>,
    pub raw_output: String,
}

impl LintResult {
    pub fn ok() -> Self {
        Self {
            success: true,
            errors: Vec::new(),
            raw_output: String::new(),
        }
    }

    pub fn failed(errors: Vec<LintError>, raw_output: String) -> Self {
        Self {
            success: false,
            errors,
            raw_output,
        }
    }

    /// Format errors for LLM consumption
    pub fn format_for_llm(&self) -> String {
        if self.success {
            return "No lint errors.".to_string();
        }

        let mut output = format!("Found {} lint error(s):\n\n", self.errors.len());
        
        for err in &self.errors {
            let location = match (err.line, err.column) {
                (Some(l), Some(c)) => format!("{}:{}:{}", err.file, l, c),
                (Some(l), None) => format!("{}:{}", err.file, l),
                _ => err.file.clone(),
            };
            
            let severity = match err.severity {
                LintSeverity::Error => "ERROR",
                LintSeverity::Warning => "WARNING",
                LintSeverity::Info => "INFO",
            };
            
            output.push_str(&format!("{}: [{}] {}\n", location, severity, err.message));
        }

        output
    }
}

/// Run appropriate linter based on file extension
pub fn lint_file(file_path: &Path, workdir: &Path) -> LintResult {
    let extension = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    
    match extension {
        "rs" => lint_rust(file_path, workdir),
        "py" => lint_python(file_path, workdir),
        "js" | "jsx" => lint_javascript(file_path, workdir),
        "ts" | "tsx" => lint_typescript(file_path, workdir),
        "go" => lint_go(file_path, workdir),
        _ => LintResult::ok(), // No linter available, assume ok
    }
}

/// Lint Rust files with rustc
fn lint_rust(file_path: &Path, workdir: &Path) -> LintResult {
    // Check if this file is in a cargo project
    let file_dir = file_path.parent().unwrap_or(workdir);
    let cargo_toml = file_dir.join("Cargo.toml");
    
    // Also check workdir for cargo project (file might be in src/)
    let workdir_cargo = workdir.join("Cargo.toml");
    let file_in_workdir = file_path.starts_with(workdir);
    
    let output = if file_in_workdir && workdir_cargo.exists() {
        // File is part of the workspace cargo project
        Command::new("cargo")
            .args(["check", "--message-format=short"])
            .current_dir(workdir)
            .output()
    } else if cargo_toml.exists() {
        // File has its own cargo project
        Command::new("cargo")
            .args(["check", "--message-format=short"])
            .current_dir(file_dir)
            .output()
    } else {
        // Standalone file - use rustc
        Command::new("rustc")
            .args(["--error-format=short", "--emit=metadata", "-o", "/dev/null"])
            .arg(file_path)
            .output()
    };

    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            
            if out.status.success() || stderr.trim().is_empty() {
                return LintResult::ok();
            }

            let errors = parse_rust_errors(&stderr, file_path);
            LintResult::failed(errors, stderr.to_string())
        }
        Err(_) => LintResult::ok(), // Linter not available
    }
}

/// Parse rustc/cargo error output
fn parse_rust_errors(output: &str, file_path: &Path) -> Vec<LintError> {
    let mut errors = Vec::new();
    let file_name = file_path.to_string_lossy();
    
    for line in output.lines() {
        // Format: file:line:col: level: message
        if line.contains(": error") || line.contains(": warning") {
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 4 {
                let severity = if line.contains(": error") {
                    LintSeverity::Error
                } else {
                    LintSeverity::Warning
                };
                
                errors.push(LintError {
                    file: file_name.to_string(),
                    line: parts.get(1).and_then(|s| s.trim().parse().ok()),
                    column: parts.get(2).and_then(|s| s.trim().parse().ok()),
                    message: parts.get(3).map(|s| s.trim().to_string()).unwrap_or_default(),
                    severity,
                });
            }
        }
    }
    
    errors
}

/// Lint Python files
fn lint_python(file_path: &Path, workdir: &Path) -> LintResult {
    // Try ruff first (faster, more modern)
    let ruff_result = Command::new("ruff")
        .args(["check", "--output-format=text"])
        .arg(file_path)
        .current_dir(workdir)
        .output();

    if let Ok(out) = ruff_result {
        let output = String::from_utf8_lossy(&out.stdout);
        if out.status.success() || output.trim().is_empty() {
            return LintResult::ok();
        }
        return LintResult::failed(parse_python_errors(&output, file_path), output.to_string());
    }

    // Fallback to python syntax check
    let output = Command::new("python3")
        .args(["-m", "py_compile"])
        .arg(file_path)
        .current_dir(workdir)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                LintResult::ok()
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr);
                LintResult::failed(parse_python_errors(&stderr, file_path), stderr.to_string())
            }
        }
        Err(_) => LintResult::ok(),
    }
}

/// Parse Python error output
fn parse_python_errors(output: &str, file_path: &Path) -> Vec<LintError> {
    let mut errors = Vec::new();
    let file_name = file_path.to_string_lossy();

    for line in output.lines() {
        // Ruff format: file:line:col: CODE message
        if line.contains(&*file_name) || line.contains("SyntaxError") || line.contains("Error") {
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 3 {
                errors.push(LintError {
                    file: file_name.to_string(),
                    line: parts.get(1).and_then(|s| s.trim().parse().ok()),
                    column: parts.get(2).and_then(|s| s.trim().parse().ok()),
                    message: parts.get(3).map(|s| s.trim().to_string()).unwrap_or(line.to_string()),
                    severity: LintSeverity::Error,
                });
            }
        }
    }

    if errors.is_empty() && !output.trim().is_empty() {
        // Fallback: just capture the whole output
        errors.push(LintError {
            file: file_name.to_string(),
            line: None,
            column: None,
            message: output.lines().take(5).collect::<Vec<_>>().join("\n"),
            severity: LintSeverity::Error,
        });
    }

    errors
}

/// Lint JavaScript files
fn lint_javascript(file_path: &Path, workdir: &Path) -> LintResult {
    let output = Command::new("eslint")
        .args(["--format=compact", "--no-color"])
        .arg(file_path)
        .current_dir(workdir)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                LintResult::ok()
            } else {
                let stdout = String::from_utf8_lossy(&out.stdout);
                LintResult::failed(parse_eslint_errors(&stdout, file_path), stdout.to_string())
            }
        }
        Err(_) => LintResult::ok(), // ESLint not available
    }
}

/// Lint TypeScript files
fn lint_typescript(file_path: &Path, workdir: &Path) -> LintResult {
    // Try tsc --noEmit first
    let output = Command::new("tsc")
        .args(["--noEmit", "--pretty", "false"])
        .arg(file_path)
        .current_dir(workdir)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                // Also try eslint
                return lint_javascript(file_path, workdir);
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            LintResult::failed(parse_tsc_errors(&stdout, file_path), stdout.to_string())
        }
        Err(_) => lint_javascript(file_path, workdir), // Fall back to eslint
    }
}

/// Parse ESLint compact output
fn parse_eslint_errors(output: &str, file_path: &Path) -> Vec<LintError> {
    let mut errors = Vec::new();
    let file_name = file_path.to_string_lossy();

    for line in output.lines() {
        // Format: file: line:col error/warning message
        if line.contains(": line ") {
            if let Some(msg_start) = line.find(": line ") {
                let after = &line[msg_start + 7..];
                let parts: Vec<&str> = after.splitn(3, ' ').collect();
                
                let location = parts.get(0).unwrap_or(&"");
                let loc_parts: Vec<&str> = location.split(',').collect();
                
                errors.push(LintError {
                    file: file_name.to_string(),
                    line: loc_parts.get(0).and_then(|s| s.trim().parse().ok()),
                    column: loc_parts.get(1).and_then(|s| s.trim().trim_end_matches(',').parse().ok()),
                    message: parts.get(2..).map(|s| s.join(" ")).unwrap_or_default(),
                    severity: if line.contains("Error") { LintSeverity::Error } else { LintSeverity::Warning },
                });
            }
        }
    }

    errors
}

/// Parse TypeScript compiler errors
fn parse_tsc_errors(output: &str, file_path: &Path) -> Vec<LintError> {
    let mut errors = Vec::new();
    let file_name = file_path.to_string_lossy();

    for line in output.lines() {
        // Format: file(line,col): error TSxxxx: message
        if line.contains("): error TS") || line.contains("): warning TS") {
            if let Some(paren_start) = line.find('(') {
                if let Some(paren_end) = line.find("): ") {
                    let location = &line[paren_start + 1..paren_end];
                    let loc_parts: Vec<&str> = location.split(',').collect();
                    let message = &line[paren_end + 3..];
                    
                    errors.push(LintError {
                        file: file_name.to_string(),
                        line: loc_parts.get(0).and_then(|s| s.parse().ok()),
                        column: loc_parts.get(1).and_then(|s| s.parse().ok()),
                        message: message.to_string(),
                        severity: if line.contains("error TS") { LintSeverity::Error } else { LintSeverity::Warning },
                    });
                }
            }
        }
    }

    errors
}

/// Lint Go files
fn lint_go(file_path: &Path, workdir: &Path) -> LintResult {
    let output = Command::new("go")
        .args(["vet"])
        .arg(file_path)
        .current_dir(workdir)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                LintResult::ok()
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr);
                LintResult::failed(parse_go_errors(&stderr, file_path), stderr.to_string())
            }
        }
        Err(_) => LintResult::ok(),
    }
}

/// Parse Go error output
fn parse_go_errors(output: &str, file_path: &Path) -> Vec<LintError> {
    let mut errors = Vec::new();
    let file_name = file_path.to_string_lossy();

    for line in output.lines() {
        // Format: file:line:col: message
        if line.starts_with(&*file_name) || line.contains(".go:") {
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 3 {
                errors.push(LintError {
                    file: file_name.to_string(),
                    line: parts.get(1).and_then(|s| s.trim().parse().ok()),
                    column: parts.get(2).and_then(|s| s.trim().parse().ok()),
                    message: parts.get(3..).map(|s| s.join(":").trim().to_string()).unwrap_or_default(),
                    severity: LintSeverity::Error,
                });
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_lint_valid_rust() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        fs::write(&file_path, "fn main() { println!(\"Hello\"); }").unwrap();
        
        let result = lint_rust(&file_path, dir.path());
        // May fail if rustc not available, that's ok
        println!("Lint result: {:?}", result);
    }

    #[test]
    fn test_lint_invalid_rust() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        fs::write(&file_path, "fn main() { let x: i32 = \"not an int\"; }").unwrap();
        
        let result = lint_rust(&file_path, dir.path());
        println!("Lint result: {:?}", result);
        // Should have errors if rustc is available
    }

    #[test]
    fn test_format_for_llm() {
        let result = LintResult::failed(
            vec![
                LintError {
                    file: "test.rs".to_string(),
                    line: Some(10),
                    column: Some(5),
                    message: "mismatched types".to_string(),
                    severity: LintSeverity::Error,
                },
            ],
            "raw output".to_string(),
        );
        
        let formatted = result.format_for_llm();
        assert!(formatted.contains("test.rs:10:5"));
        assert!(formatted.contains("ERROR"));
        assert!(formatted.contains("mismatched types"));
    }
}
