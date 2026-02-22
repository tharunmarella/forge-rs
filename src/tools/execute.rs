use super::ToolResult;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Max output size per command (chars). Prevents a single tool result from
/// blowing up the LLM context window (e.g., `cat` on a 2MB bundle).
const MAX_OUTPUT_CHARS: usize = 30_000;

/// Default command timeout in seconds. Prevents hung builds or `sleep`
/// from blocking the agent forever. Override per-call via `timeout_secs` param.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Maximum allowed timeout (10 minutes).
const MAX_TIMEOUT_SECS: u64 = 600;

/// Execute a shell command with timeout protection.
pub async fn run(args: &Value, workdir: &Path) -> ToolResult {
    let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'command' parameter");
    };

    // ── Command sanitization ────────────────────────────────────
    // Intercept grep/find commands that should use the dedicated tools.
    let command = sanitize_command(command, workdir);

    // Allow per-call timeout override (clamped to MAX).
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .min(MAX_TIMEOUT_SECS);

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to spawn: {e}")),
    };

    // Run the output collection with a timeout.
    let timeout_duration = std::time::Duration::from_secs(timeout_secs);

    match tokio::time::timeout(timeout_duration, collect_output(&mut child)).await {
        Ok((output, truncated, exit_code)) => {
            let mut result = output;
            if truncated {
                result.push_str(&format!(
                    "\n... (output truncated at {} chars. Use head/tail/grep to get specific parts)",
                    MAX_OUTPUT_CHARS
                ));
            }

            if exit_code == 0 {
                ToolResult::ok(format!("Exit code: 0\n{result}"))
            } else {
                ToolResult::err(format!("Exit code: {exit_code}\n{result}"))
            }
        }
        Err(_) => {
            // Timeout expired — kill the entire process tree, not just the shell.
            if let Some(pid) = child.id() {
                // Kill the entire process group (negative PID kills the group)
                // First try SIGTERM to allow graceful shutdown
                let _ = Command::new("pkill")
                    .args(["-TERM", "-P", &pid.to_string()])
                    .output()
                    .await;
                
                // Give processes a moment to exit gracefully
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                
                // Then force kill any remaining children
                let _ = Command::new("pkill")
                    .args(["-KILL", "-P", &pid.to_string()])
                    .output()
                    .await;
            }
            
            // Finally kill the shell itself
            let _ = child.kill().await;
            
            ToolResult::err(format!(
                "Command timed out after {}s. The process was killed.\n\
                 If this command needs more time, add \"timeout_secs\": {} (max {}) to the arguments.\n\
                 Command: {}",
                timeout_secs,
                timeout_secs * 2,
                MAX_TIMEOUT_SECS,
                truncate_cmd(&command, 200),
            ))
        }
    }
}

/// Collect stdout + stderr from a child process concurrently.
/// Reading both streams in parallel prevents the classic deadlock where the
/// child fills its stderr pipe buffer while we are blocked draining stdout
/// (or vice-versa), causing the child to hang indefinitely.
/// Returns (output_string, was_truncated, exit_code).
async fn collect_output(
    child: &mut tokio::process::Child,
) -> (String, bool, i32) {
    use tokio::sync::mpsc;

    // Each stream sends lines to a shared channel so we can interleave them
    // in arrival order without blocking either pipe.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    let stdout_tx = tx.clone();
    let stderr_tx = tx;

    // Drain stdout in a separate task
    let stdout_task = if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                if stdout_tx.send(line).is_err() {
                    break;
                }
            }
        })
    } else {
        tokio::spawn(async {})
    };

    // Drain stderr in a separate task
    let stderr_task = if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                if stderr_tx.send(line).is_err() {
                    break;
                }
            }
        })
    } else {
        tokio::spawn(async {})
    };

    // Collect lines from both streams as they arrive, enforcing the cap
    let mut output = String::new();
    let mut truncated = false;

    // Wait for both drain tasks to finish (they close the channel when done)
    let collect_task = tokio::spawn(async move {
        let mut out = String::new();
        let mut trunc = false;
        while let Some(line) = rx.recv().await {
            if out.len() + line.len() + 1 > MAX_OUTPUT_CHARS {
                trunc = true;
                // Keep draining the channel so the tasks don't block on send
                while rx.recv().await.is_some() {}
                break;
            }
            out.push_str(&line);
            out.push('\n');
        }
        (out, trunc)
    });

    // Drive all three tasks concurrently
    let (_, _, collected) = tokio::join!(stdout_task, stderr_task, collect_task);
    if let Ok((out, trunc)) = collected {
        output = out;
        truncated = trunc;
    }

    let status = child.wait().await;
    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

    (output, truncated, exit_code)
}

/// Sanitize commands to prevent conflicts with dedicated tools.
/// Rewrites grep/find commands to use ripgrep for better performance.
fn sanitize_command(command: &str, _workdir: &Path) -> String {
    let trimmed = command.trim();
    
    // Rewrite grep commands to use ripgrep (rg) for better performance
    if trimmed.starts_with("grep ") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 2 {
            let mut rg_args = vec!["rg", "--line-number", "--no-heading", "--color=never", "--max-filesize=100K"];
            let mut pattern: Option<&str> = None;
            let mut search_path: Option<&str> = None;
            let mut i = 1; // Skip "grep"
            
            while i < parts.len() {
                let part = parts[i];
                if part.starts_with('-') {
                    // Translate grep flags to rg flags
                    if part.contains('i') && !part.contains("-include") {
                        rg_args.push("-i");
                    }
                    if part.contains('l') {
                        rg_args.push("-l");
                    }
                    if part.contains('c') && part.len() <= 4 {
                        rg_args.push("--count");
                    }
                    // Handle --include="*.py" → -g "*.py"
                    if part.starts_with("--include=") {
                        let glob = part.trim_start_matches("--include=").trim_matches('"').trim_matches('\'');
                        rg_args.push("-g");
                        rg_args.push(glob);
                    }
                } else if part.starts_with("--include") {
                    // --include "*.py" (space-separated)
                    if i + 1 < parts.len() {
                        i += 1;
                        let glob = parts[i].trim_matches('"').trim_matches('\'');
                        rg_args.push("-g");
                        rg_args.push(glob);
                    }
                } else if pattern.is_none() {
                    pattern = Some(part);
                } else {
                    search_path = Some(part);
                }
                i += 1;
            }
            
            if let Some(pat) = pattern {
                rg_args.push(pat);
            }
            if let Some(path) = search_path {
                rg_args.push(path);
            }
            
            // Handle piped commands (e.g., `grep -R foo . | head`)
            let pipe_suffix = if let Some(pipe_idx) = trimmed.find(" | ") {
                &trimmed[pipe_idx..]
            } else {
                ""
            };
            
            let rewritten = format!("{}{}", rg_args.join(" "), pipe_suffix);
            tracing::info!("Rewriting grep to rg: {} -> {}", truncate_cmd(trimmed, 100), truncate_cmd(&rewritten, 100));
            return rewritten;
        }
    }

    trimmed.to_string()
}

/// Truncate command string for logging/error messages.
fn truncate_cmd(cmd: &str, max_len: usize) -> String {
    if cmd.len() <= max_len {
        cmd.to_string()
    } else {
        format!("{}...", &cmd[..max_len])
    }
}
