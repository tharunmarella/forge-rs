//! Background process execution and port utilities.
//!
//! These tools enable handling of long-running processes (dev servers, watchers)
//! without blocking the agent or hitting timeouts.

use super::ToolResult;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Max output to return from background process reads (chars).
const MAX_OUTPUT_CHARS: usize = 20_000;

/// Global registry of background processes.
/// Wrapped in Arc<Mutex> so it can be shared across tool calls.
fn background_processes() -> &'static Arc<Mutex<HashMap<u32, BackgroundProcess>>> {
    static INSTANCE: OnceLock<Arc<Mutex<HashMap<u32, BackgroundProcess>>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// A background process with output capture.
struct BackgroundProcess {
    pid: u32,
    command: String,
    started_at: Instant,
    output_buffer: Arc<Mutex<String>>,
    /// Handle to the child process (None if already reaped).
    child: Option<Child>,
}

impl BackgroundProcess {
    fn runtime_secs(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }
}

/// Execute a command in the background, returning immediately.
///
/// Args:
/// - command: The shell command to execute
/// - wait_seconds: Seconds to wait for initial output (default: 3)
pub async fn execute_background(args: &Value, workdir: &Path) -> ToolResult {
    let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'command' parameter");
    };

    let wait_seconds = args
        .get("wait_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(3);

    // Spawn the process
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to spawn: {e}")),
    };

    let pid = child.id().unwrap_or(0);
    let output_buffer = Arc::new(Mutex::new(String::new()));

    // Start background task to collect output
    let buffer_clone = output_buffer.clone();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    tokio::spawn(async move {
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let mut buf = buffer_clone.lock().await;
                if buf.len() < MAX_OUTPUT_CHARS * 2 {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
        }
    });

    let buffer_clone2 = output_buffer.clone();
    tokio::spawn(async move {
        if let Some(stderr) = stderr {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let mut buf = buffer_clone2.lock().await;
                if buf.len() < MAX_OUTPUT_CHARS * 2 {
                    buf.push_str("[stderr] ");
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
        }
    });

    // Wait briefly for initial output
    tokio::time::sleep(Duration::from_secs(wait_seconds)).await;

    // Check if still running
    let is_running = child.try_wait().map(|s| s.is_none()).unwrap_or(false);

    // Store in registry
    let bg_proc = BackgroundProcess {
        pid,
        command: command.to_string(),
        started_at: Instant::now(),
        output_buffer: output_buffer.clone(),
        child: Some(child),
    };

    background_processes().lock().await.insert(pid, bg_proc);

    // Get initial output
    let initial_output = {
        let buf = output_buffer.lock().await;
        let len = buf.len().min(MAX_OUTPUT_CHARS);
        buf[..len].to_string()
    };

    ToolResult::ok(format!(
        "Process started in background.\n\
         PID: {pid}\n\
         Running: {is_running}\n\
         --- Initial output ({wait_seconds}s) ---\n\
         {initial_output}"
    ))
}

/// Read output from a background process.
///
/// Args:
/// - pid: Process ID to read output from
/// - tail_lines: Number of lines from the end (default: 100)
/// - follow_seconds: Seconds to wait for new output (default: 0)
pub async fn read_process_output(args: &Value, _workdir: &Path) -> ToolResult {
    let Some(pid) = args.get("pid").and_then(|v| v.as_u64()).map(|p| p as u32) else {
        return ToolResult::err("Missing 'pid' parameter");
    };

    let tail_lines = args
        .get("tail_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    let follow_seconds = args
        .get("follow_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Wait if requested
    if follow_seconds > 0 {
        tokio::time::sleep(Duration::from_secs(follow_seconds)).await;
    }

    let processes = background_processes().lock().await;
    let Some(proc) = processes.get(&pid) else {
        let active_pids: Vec<_> = processes.keys().collect();
        return ToolResult::err(format!(
            "No background process with PID {pid}. Active PIDs: {active_pids:?}"
        ));
    };

    let output = {
        let buf = proc.output_buffer.lock().await;
        let lines: Vec<&str> = buf.lines().collect();
        if lines.len() > tail_lines {
            format!(
                "... (showing last {tail_lines} of {} lines) ...\n{}",
                lines.len(),
                lines[lines.len() - tail_lines..].join("\n")
            )
        } else {
            buf.clone()
        }
    };

    // Check if still running
    let status = if proc.child.is_some() {
        "running"
    } else {
        "exited"
    };

    ToolResult::ok(format!(
        "PID: {pid} | Status: {status} | Runtime: {:.1}s\n\
         --- Output ---\n{output}",
        proc.runtime_secs()
    ))
}

/// Check status of background processes.
///
/// Args:
/// - pid: Optional specific PID to check (if omitted, returns all)
pub async fn check_process_status(args: &Value, _workdir: &Path) -> ToolResult {
    let pid = args.get("pid").and_then(|v| v.as_u64()).map(|p| p as u32);

    let mut processes = background_processes().lock().await;

    if let Some(pid) = pid {
        let Some(proc) = processes.get_mut(&pid) else {
            return ToolResult::err(format!("No background process with PID {pid}"));
        };

        // Check if still running
        let (is_running, exit_code) = if let Some(ref mut child) = proc.child {
            match child.try_wait() {
                Ok(Some(status)) => (false, status.code()),
                Ok(None) => (true, None),
                Err(_) => (false, None),
            }
        } else {
            (false, None)
        };

        let exit_str = exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "N/A".to_string());

        return ToolResult::ok(format!(
            "PID: {pid}\n\
             Command: {}\n\
             Running: {is_running}\n\
             Exit code: {exit_str}\n\
             Runtime: {:.1}s",
            proc.command,
            proc.runtime_secs()
        ));
    }

    // Return all processes
    if processes.is_empty() {
        return ToolResult::ok("No background processes running.");
    }

    let mut lines = vec!["Active background processes:".to_string()];
    for (pid, proc) in processes.iter_mut() {
        let is_running = if let Some(ref mut child) = proc.child {
            child.try_wait().map(|s| s.is_none()).unwrap_or(false)
        } else {
            false
        };
        let status = if is_running { "running" } else { "exited" };
        let cmd_preview: String = proc.command.chars().take(50).collect();
        lines.push(format!(
            "  PID {pid}: {status} | {:.1}s | {cmd_preview}...",
            proc.runtime_secs()
        ));
    }

    ToolResult::ok(lines.join("\n"))
}

/// Kill a background process.
///
/// Args:
/// - pid: Process ID to kill
/// - force: Use SIGKILL instead of SIGTERM (default: false)
pub async fn kill_process(args: &Value, _workdir: &Path) -> ToolResult {
    let Some(pid) = args.get("pid").and_then(|v| v.as_u64()).map(|p| p as u32) else {
        return ToolResult::err("Missing 'pid' parameter");
    };

    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut processes = background_processes().lock().await;

    if let Some(proc) = processes.get_mut(&pid) {
        if let Some(ref mut child) = proc.child {
            let result = if force {
                child.kill().await
            } else {
                // Send SIGTERM via kill command
                let _ = Command::new("kill")
                    .arg("-15")
                    .arg(pid.to_string())
                    .output()
                    .await;
                // Wait briefly
                tokio::time::sleep(Duration::from_millis(500)).await;
                child.try_wait().map(|_| ()).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            };

            match result {
                Ok(_) => {
                    // Wait for exit
                    let _ = child.wait().await;
                    proc.child = None;
                    return ToolResult::ok(format!("Process {pid} terminated."));
                }
                Err(e) => {
                    return ToolResult::err(format!("Failed to kill process {pid}: {e}"));
                }
            }
        } else {
            return ToolResult::ok(format!("Process {pid} already exited."));
        }
    }

    // Try system kill for non-tracked processes
    let signal = if force { "-9" } else { "-15" };
    let result = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            ToolResult::ok(format!("Sent {} to PID {pid}", if force { "SIGKILL" } else { "SIGTERM" }))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            ToolResult::err(format!("Failed to kill PID {pid}: {stderr}"))
        }
        Err(e) => ToolResult::err(format!("Failed to kill PID {pid}: {e}")),
    }
}

/// Wait until a port is accepting connections (and optionally responding to HTTP).
///
/// Args:
/// - port: Port number to check
/// - host: Host to check (default: localhost)
/// - timeout: Max seconds to wait (default: 30)
/// - interval: Seconds between checks (default: 1)
/// - http_check: If true, verify HTTP GET returns 2xx/3xx (default: false)
/// - path: HTTP path to check (default: "/")
pub async fn wait_for_port(args: &Value, _workdir: &Path) -> ToolResult {
    let Some(port) = args.get("port").and_then(|v| v.as_u64()).map(|p| p as u16) else {
        return ToolResult::err("Missing 'port' parameter");
    };

    let host = args
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("localhost");

    let timeout_secs = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);

    let interval_secs = args
        .get("interval")
        .and_then(|v| v.as_u64())
        .unwrap_or(1);

    let http_check = args
        .get("http_check")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("/");

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let interval = Duration::from_secs(interval_secs);
    let mut attempts = 0;
    let mut last_error = String::new();

    let addr = format!("{host}:{port}");

    while start.elapsed() < timeout {
        attempts += 1;

        // First check TCP connection
        match tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await {
            Ok(Ok(_)) => {
                // TCP connection successful
                if !http_check {
                    return ToolResult::ok(format!(
                        "Port {port} is now accepting connections!\n\
                         Host: {host}\n\
                         Time waited: {:.1}s\n\
                         Attempts: {attempts}",
                        start.elapsed().as_secs_f64()
                    ));
                }

                // HTTP health check requested
                let url = format!("http://{host}:{port}{path}");
                match http_health_check(&url).await {
                    Ok(status) => {
                        return ToolResult::ok(format!(
                            "Server is healthy!\n\
                             URL: {url}\n\
                             Status: {status}\n\
                             Time waited: {:.1}s\n\
                             Attempts: {attempts}",
                            start.elapsed().as_secs_f64()
                        ));
                    }
                    Err(e) => {
                        last_error = format!("HTTP check failed: {e}");
                        // Continue waiting - server might still be starting
                    }
                }
            }
            Ok(Err(e)) => {
                last_error = format!("TCP connection failed: {e}");
            }
            Err(_) => {
                last_error = "TCP connection timed out".to_string();
            }
        }

        tokio::time::sleep(interval).await;
    }

    ToolResult::err(format!(
        "Port {port} on {host} did not become available within {timeout_secs}s.\n\
         Attempts: {attempts}\n\
         Last error: {last_error}"
    ))
}

/// Perform a simple HTTP GET and check for success status.
async fn http_health_check(url: &str) -> Result<u16, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status().as_u16();
    
    if status >= 200 && status < 400 {
        Ok(status)
    } else {
        Err(format!("HTTP {status}"))
    }
}

/// Check if a port is currently in use.
///
/// Args:
/// - port: Port number to check
/// - host: Host to check (default: localhost)
pub async fn check_port(args: &Value, _workdir: &Path) -> ToolResult {
    let Some(port) = args.get("port").and_then(|v| v.as_u64()).map(|p| p as u16) else {
        return ToolResult::err("Missing 'port' parameter");
    };

    let host = args
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("localhost");

    let addr = format!("{host}:{port}");

    match tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => {
            // Port is in use, try to find what's using it
            let pid_info = if let Ok(output) = Command::new("lsof")
                .args(["-i", &format!(":{port}"), "-t"])
                .output()
                .await
            {
                let pids = String::from_utf8_lossy(&output.stdout);
                let pids: Vec<&str> = pids.trim().lines().collect();
                if pids.is_empty() {
                    "PID: unknown".to_string()
                } else {
                    format!("PIDs: {}", pids.join(", "))
                }
            } else {
                "PID: could not determine".to_string()
            };

            ToolResult::ok(format!("Port {port} is IN USE on {host}. {pid_info}"))
        }
        _ => ToolResult::ok(format!("Port {port} is AVAILABLE on {host}.")),
    }
}

/// Kill the process using a specific port.
///
/// Args:
/// - port: Port number
/// - force: Use SIGKILL instead of SIGTERM (default: false)
pub async fn kill_port(args: &Value, _workdir: &Path) -> ToolResult {
    let Some(port) = args.get("port").and_then(|v| v.as_u64()).map(|p| p as u16) else {
        return ToolResult::err("Missing 'port' parameter");
    };

    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

    // Find PIDs using this port
    let output = match Command::new("lsof")
        .args(["-i", &format!(":{port}"), "-t"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return ToolResult::err(format!("Failed to find process on port {port}: {e}")),
    };

    let pids_str = String::from_utf8_lossy(&output.stdout);
    let pids: Vec<&str> = pids_str.trim().lines().filter(|s| !s.is_empty()).collect();

    if pids.is_empty() {
        return ToolResult::ok(format!("No process found using port {port}"));
    }

    let signal = if force { "-9" } else { "-15" };
    let mut killed = Vec::new();
    let mut failed = Vec::new();

    for pid in pids {
        let result = Command::new("kill")
            .arg(signal)
            .arg(pid)
            .output()
            .await;

        match result {
            Ok(o) if o.status.success() => killed.push(pid.to_string()),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                failed.push(format!("{pid}: {err}"));
            }
            Err(e) => failed.push(format!("{pid}: {e}")),
        }
    }

    let mut result_lines = Vec::new();
    if !killed.is_empty() {
        result_lines.push(format!("Killed PIDs: {}", killed.join(", ")));
    }
    if !failed.is_empty() {
        result_lines.push(format!("Failed: {}", failed.join("; ")));
    }

    if result_lines.is_empty() {
        ToolResult::ok("No action taken")
    } else {
        ToolResult::ok(result_lines.join("\n"))
    }
}
