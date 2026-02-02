use super::ToolResult;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Execute a shell command
pub async fn run(args: &Value, workdir: &Path) -> ToolResult {
    let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'command' parameter");
    };

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

    let mut output = String::new();

    // Stream stdout
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
        }
    }

    // Stream stderr
    if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            output.push_str(&line);
            output.push('\n');
        }
    }

    let status = child.wait().await;
    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

    if exit_code == 0 {
        ToolResult::ok(format!("Exit code: 0\n{output}"))
    } else {
        ToolResult::err(format!("Exit code: {exit_code}\n{output}"))
    }
}
