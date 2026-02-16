use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use uuid::Uuid;

/// Global trace store
static TRACE_STORE: once_cell::sync::Lazy<Arc<Mutex<TraceStore>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(TraceStore::new())));

/// Execution trace for a single agent run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub id: String,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub user_message: String,
    pub ai_response: Option<String>,
    pub tool_calls: Vec<ToolCallTrace>,
    pub status: TraceStatus,
    pub error_message: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Status of a trace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Individual tool call within a trace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallTrace {
    pub id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub result: Option<ToolCallResult>,
    pub duration_ms: Option<u64>,
}

/// Result of a tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

/// In-memory trace store with file persistence
pub struct TraceStore {
    traces: HashMap<String, ExecutionTrace>,
    traces_dir: PathBuf,
}

impl TraceStore {
    pub fn new() -> Self {
        let traces_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".forge")
            .join("traces");
        
        Self {
            traces: HashMap::new(),
            traces_dir,
        }
    }

    pub async fn initialize(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Create traces directory
        fs::create_dir_all(&self.traces_dir).await?;
        
        // Load existing traces from disk
        self.load_traces().await?;
        
        Ok(())
    }

    async fn load_traces(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.traces_dir.exists() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&self.traces_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path).await {
                    if let Ok(trace) = serde_json::from_str::<ExecutionTrace>(&content) {
                        self.traces.insert(trace.id.clone(), trace);
                    }
                }
            }
        }

        tracing::info!("Loaded {} traces from disk", self.traces.len());
        Ok(())
    }

    pub async fn save_trace(&mut self, trace: &ExecutionTrace) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Save to memory
        self.traces.insert(trace.id.clone(), trace.clone());

        // Save to disk
        let file_path = self.traces_dir.join(format!("{}.json", trace.id));
        let content = serde_json::to_string_pretty(trace)?;
        fs::write(file_path, content).await?;

        Ok(())
    }

    pub fn get_trace(&self, id: &str) -> Option<&ExecutionTrace> {
        self.traces.get(id)
    }

    pub fn list_traces(&self, workspace_id: Option<&str>, limit: Option<usize>) -> Vec<&ExecutionTrace> {
        let mut traces: Vec<&ExecutionTrace> = self.traces.values().collect();
        
        // Filter by workspace if specified
        if let Some(ws_id) = workspace_id {
            traces.retain(|t| t.workspace_id == ws_id);
        }

        // Sort by start time (newest first)
        traces.sort_by(|a, b| b.start_time.cmp(&a.start_time));

        // Apply limit
        if let Some(limit) = limit {
            traces.truncate(limit);
        }

        traces
    }

    pub async fn cleanup_old_traces(&mut self, days: u64) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs() - (days * 24 * 60 * 60);

        let mut removed = 0;
        let mut to_remove = Vec::new();

        for (id, trace) in &self.traces {
            if trace.start_time < cutoff {
                to_remove.push(id.clone());
            }
        }

        for id in to_remove {
            self.traces.remove(&id);
            let file_path = self.traces_dir.join(format!("{}.json", id));
            if file_path.exists() {
                fs::remove_file(file_path).await?;
            }
            removed += 1;
        }

        Ok(removed)
    }
}

/// Start a new execution trace
pub async fn start_trace(
    workspace_id: &str,
    session_id: Option<&str>,
    user_message: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let trace_id = Uuid::new_v4().to_string();
    let trace = ExecutionTrace {
        id: trace_id.clone(),
        workspace_id: workspace_id.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        start_time: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        end_time: None,
        user_message: user_message.to_string(),
        ai_response: None,
        tool_calls: Vec::new(),
        status: TraceStatus::Running,
        error_message: None,
        metadata: HashMap::new(),
    };

    let mut store = TRACE_STORE.lock().unwrap();
    store.save_trace(&trace).await?;

    Ok(trace_id)
}

/// Add a tool call to a trace
pub async fn add_tool_call(
    trace_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let call_id = Uuid::new_v4().to_string();
    let tool_call = ToolCallTrace {
        id: call_id.clone(),
        tool_name: tool_name.to_string(),
        arguments,
        start_time: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        end_time: None,
        result: None,
        duration_ms: None,
    };

    let mut store = TRACE_STORE.lock().unwrap();
    if let Some(trace) = store.traces.get_mut(trace_id) {
        trace.tool_calls.push(tool_call);
        let trace_clone = trace.clone();
        drop(store); // Release the lock before async operation
        let mut store = TRACE_STORE.lock().unwrap();
        store.save_trace(&trace_clone).await?;
    }

    Ok(call_id)
}

/// Complete a tool call in a trace
pub async fn complete_tool_call(
    trace_id: &str,
    call_id: &str,
    result: ToolCallResult,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let end_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let mut store = TRACE_STORE.lock().unwrap();
    if let Some(trace) = store.traces.get_mut(trace_id) {
        if let Some(tool_call) = trace.tool_calls.iter_mut().find(|tc| tc.id == call_id) {
            tool_call.end_time = Some(end_time);
            tool_call.result = Some(result);
            if let Some(start) = tool_call.start_time.checked_sub(0) {
                tool_call.duration_ms = Some((end_time - start) * 1000);
            }
        }
        let trace_clone = trace.clone();
        drop(store); // Release the lock before async operation
        let mut store = TRACE_STORE.lock().unwrap();
        store.save_trace(&trace_clone).await?;
    }

    Ok(())
}

/// Complete an execution trace
pub async fn complete_trace(
    trace_id: &str,
    ai_response: Option<&str>,
    status: TraceStatus,
    error_message: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let end_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let mut store = TRACE_STORE.lock().unwrap();
    if let Some(trace) = store.traces.get_mut(trace_id) {
        trace.end_time = Some(end_time);
        trace.ai_response = ai_response.map(|s| s.to_string());
        trace.status = status;
        trace.error_message = error_message.map(|s| s.to_string());
        let trace_clone = trace.clone();
        drop(store); // Release the lock before async operation
        let mut store = TRACE_STORE.lock().unwrap();
        store.save_trace(&trace_clone).await?;
    }

    Ok(())
}

/// Get a trace by ID
pub async fn get_trace(trace_id: &str) -> Option<ExecutionTrace> {
    let store = TRACE_STORE.lock().unwrap();
    store.get_trace(trace_id).cloned()
}

/// List traces with optional filtering
pub async fn list_traces(
    workspace_id: Option<&str>,
    limit: Option<usize>,
) -> Vec<ExecutionTrace> {
    let store = TRACE_STORE.lock().unwrap();
    store.list_traces(workspace_id, limit)
        .into_iter()
        .cloned()
        .collect()
}

/// Initialize the trace system
pub async fn initialize() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut store = TRACE_STORE.lock().unwrap();
    store.initialize().await?;
    Ok(())
}

/// Generate a simple HTML dashboard for viewing traces
pub fn generate_dashboard_html(traces: &[ExecutionTrace]) -> String {
    let mut html = String::from(r#"
<!DOCTYPE html>
<html>
<head>
    <title>Forge Execution Traces</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 40px; }
        .trace { border: 1px solid #ddd; margin: 20px 0; padding: 20px; border-radius: 8px; }
        .trace.running { border-left: 4px solid #007acc; }
        .trace.completed { border-left: 4px solid #28a745; }
        .trace.failed { border-left: 4px solid #dc3545; }
        .trace-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 15px; }
        .trace-id { font-family: monospace; font-size: 12px; color: #666; }
        .status { padding: 4px 8px; border-radius: 4px; font-size: 12px; font-weight: bold; }
        .status.running { background: #cce7ff; color: #007acc; }
        .status.completed { background: #d4edda; color: #155724; }
        .status.failed { background: #f8d7da; color: #721c24; }
        .tool-calls { margin-top: 15px; }
        .tool-call { background: #f8f9fa; padding: 10px; margin: 5px 0; border-radius: 4px; font-size: 14px; }
        .tool-call.success { border-left: 3px solid #28a745; }
        .tool-call.error { border-left: 3px solid #dc3545; }
        pre { background: #f8f9fa; padding: 10px; border-radius: 4px; overflow-x: auto; }
    </style>
</head>
<body>
    <h1>🔍 Forge Execution Traces</h1>
    <p>Total traces: <strong>"#);
    
    html.push_str(&traces.len().to_string());
    html.push_str("</strong></p>");

    for trace in traces {
        let status_class = match trace.status {
            TraceStatus::Running => "running",
            TraceStatus::Completed => "completed",
            TraceStatus::Failed => "failed",
            TraceStatus::Cancelled => "failed",
        };

        html.push_str(&format!(r#"
    <div class="trace {}">
        <div class="trace-header">
            <div>
                <strong>{}</strong>
                <div class="trace-id">ID: {}</div>
            </div>
            <span class="status {}">{:?}</span>
        </div>
        <p><strong>Message:</strong> {}</p>
        "#, 
        status_class,
        trace.workspace_id,
        trace.id,
        status_class,
        trace.status,
        html_escape(&trace.user_message)
        ));

        if let Some(response) = &trace.ai_response {
            html.push_str(&format!("<p><strong>Response:</strong></p><pre>{}</pre>", html_escape(response)));
        }

        if !trace.tool_calls.is_empty() {
            html.push_str("<div class=\"tool-calls\"><strong>Tool Calls:</strong>");
            for tool_call in &trace.tool_calls {
                let call_class = if let Some(result) = &tool_call.result {
                    if result.success { "success" } else { "error" }
                } else {
                    ""
                };

                html.push_str(&format!(r#"
                <div class="tool-call {}">
                    <strong>{}</strong>
                    "#, call_class, tool_call.tool_name));

                if let Some(result) = &tool_call.result {
                    if result.success {
                        html.push_str(&format!(" ✅ <em>{}</em>", html_escape(&result.output[..result.output.len().min(100)])));
                    } else {
                        html.push_str(&format!(" ❌ <em>{}</em>", html_escape(&result.output[..result.output.len().min(100)])));
                    }
                }

                if let Some(duration) = tool_call.duration_ms {
                    html.push_str(&format!(" <small>({}ms)</small>", duration));
                }

                html.push_str("</div>");
            }
            html.push_str("</div>");
        }

        html.push_str("</div>");
    }

    html.push_str(r#"
</body>
</html>"#);

    html
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}