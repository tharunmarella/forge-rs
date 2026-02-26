mod execute;
mod files;
mod search;
mod code;
mod web;
mod watcher;
pub mod trace;
pub mod embeddings;
mod embeddings_store;
mod treesitter;
mod process;
pub mod lint;


use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};


/// All available tools
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    // Essential file operations
    ExecuteCommand,
    ReadFile,
    WriteToFile,
    ReplaceInFile,
    ApplyPatch,
    ListFiles,
    DeleteFile,
    
    // Background processes & ports
    ExecuteBackground,
    ReadProcessOutput,
    CheckProcessStatus,
    KillProcess,
    WaitForPort,
    CheckPort,
    KillPort,
    
    // Search & Indexing
    Grep,
    Glob,
    CodebaseSearch,
    IndexFiles,
    ReindexWorkspace,
    WatchFiles,
    ScanFiles,
    StopWatching,
    ListTraces,
    GetTrace,
    TraceDashboard,
    GenerateRepoMap,
    
    // Code intelligence
    ListCodeDefinitions,
    GetSymbolDefinition,
    FindSymbolReferences,
    TraceCallChain,
    ImpactAnalysis,
    Diagnostics,
    GetArchitectureMap,
    SearchFunctions,
    SearchClasses,
    SearchFiles,
    LspGoToDefinition,
    LspFindReferences,
    LspHover,
    
    // Web & Documentation
    WebSearch,
    WebFetch,
    FetchDocs,
    
    // Interaction
    AttemptCompletion,
    AskFollowupQuestion,
    Think,

    // Planning
    CreatePlan,
    UpdatePlan,
    AddPlanStep,
    RemovePlanStep,
    DiscardPlan,
    Replan,
    
    // Mode control (internal)
    PlanModeRespond,
    ActModeRespond,
    FocusChain,
}

impl Tool {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ExecuteCommand => "run",
            Self::ReadFile => "read",
            Self::WriteToFile => "write",
            Self::ReplaceInFile => "replace",
            Self::ApplyPatch => "apply_patch",
            Self::ListFiles => "ls",
            Self::DeleteFile => "delete_file",
            // Background processes & ports
            Self::ExecuteBackground => "execute_background",
            Self::ReadProcessOutput => "read_process_output",
            Self::CheckProcessStatus => "check_process_status",
            Self::KillProcess => "kill_process",
            Self::WaitForPort => "wait_for_port",
            Self::CheckPort => "check_port",
            Self::KillPort => "kill_port",
            Self::Grep => "grep",
            Self::Glob => "glob",
            Self::CodebaseSearch => "search",
            Self::IndexFiles => "index_files",
            Self::ReindexWorkspace => "reindex_workspace",
            Self::WatchFiles => "watch_files",
            Self::ScanFiles => "scan_files",
            Self::StopWatching => "stop_watching",
            Self::ListTraces => "list_traces",
            Self::GetTrace => "get_trace",
            Self::TraceDashboard => "trace_dashboard",
            Self::GenerateRepoMap => "repomap",
            Self::ListCodeDefinitions => "list_code_definition_names",
            Self::GetSymbolDefinition => "get_symbol_definition",
            Self::FindSymbolReferences => "find_symbol_references",
            Self::Diagnostics => "diagnostics",
            Self::GetArchitectureMap => "repomap",
            Self::SearchFunctions => "search_functions",
            Self::SearchClasses => "search_classes",
            Self::SearchFiles => "search_files",
            Self::LspGoToDefinition => "lsp_go_to_definition",
            Self::LspFindReferences => "lsp_find_references",
            Self::LspHover => "lsp_hover",
            Self::WebSearch => "web_search",
            Self::WebFetch => "web_fetch",
            Self::FetchDocs => "docs",
            Self::AttemptCompletion => "attempt_completion",
            Self::AskFollowupQuestion => "ask_followup_question",
            Self::Think => "think",
            Self::CreatePlan => "create_plan",
            Self::UpdatePlan => "update_plan",
            Self::AddPlanStep => "add_plan_step",
            Self::RemovePlanStep => "remove_plan_step",
            Self::DiscardPlan => "discard_plan",
            Self::PlanModeRespond => "plan_mode_respond",
            Self::ActModeRespond => "act_mode_respond",
            Self::FocusChain => "focus_chain",
            Self::TraceCallChain => "trace_call_chain",
            Self::ImpactAnalysis => "impact_analysis",
            Self::Replan => "replan",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "run" => Some(Self::ExecuteCommand),
            "read" => Some(Self::ReadFile),
            "write" => Some(Self::WriteToFile),
            "replace" => Some(Self::ReplaceInFile),
            "apply_patch" => Some(Self::ApplyPatch),
            "ls" => Some(Self::ListFiles),
            "delete_file" => Some(Self::DeleteFile),
            // Background processes & ports
            "execute_background" => Some(Self::ExecuteBackground),
            "read_process_output" => Some(Self::ReadProcessOutput),
            "check_process_status" => Some(Self::CheckProcessStatus),
            "kill_process" => Some(Self::KillProcess),
            "wait_for_port" => Some(Self::WaitForPort),
            "check_port" => Some(Self::CheckPort),
            "kill_port" => Some(Self::KillPort),
            "grep" => Some(Self::Grep),
            "glob" => Some(Self::Glob),
            "search" => Some(Self::CodebaseSearch),
            "index_files" => Some(Self::IndexFiles),
            "reindex_workspace" => Some(Self::ReindexWorkspace),
            "watch_files" => Some(Self::WatchFiles),
            "scan_files" => Some(Self::ScanFiles),
            "stop_watching" => Some(Self::StopWatching),
            "list_traces" => Some(Self::ListTraces),
            "get_trace" => Some(Self::GetTrace),
            "trace_dashboard" => Some(Self::TraceDashboard),
            "repomap" => Some(Self::GenerateRepoMap),
            "list_code_definition_names" => Some(Self::ListCodeDefinitions),
            "get_symbol_definition" => Some(Self::GetSymbolDefinition),
            "find_symbol_references" => Some(Self::FindSymbolReferences),
            "trace_call_chain" => Some(Self::TraceCallChain),
            "impact_analysis" => Some(Self::ImpactAnalysis),
            "diagnostics" => Some(Self::Diagnostics),
            "search_functions" => Some(Self::SearchFunctions),
            "search_classes" => Some(Self::SearchClasses),
            "search_files" => Some(Self::SearchFiles),
            "lsp_go_to_definition" => Some(Self::LspGoToDefinition),
            "lsp_find_references" => Some(Self::LspFindReferences),
            "lsp_hover" => Some(Self::LspHover),
            "replan" => Some(Self::Replan),
            "web_search" => Some(Self::WebSearch),
            "web_fetch" => Some(Self::WebFetch),
            "docs" => Some(Self::FetchDocs),
            "attempt_completion" => Some(Self::AttemptCompletion),
            "ask_followup_question" => Some(Self::AskFollowupQuestion),
            "think" => Some(Self::Think),
            "create_plan" => Some(Self::CreatePlan),
            "update_plan" => Some(Self::UpdatePlan),
            "add_plan_step" => Some(Self::AddPlanStep),
            "remove_plan_step" => Some(Self::RemovePlanStep),
            "discard_plan" => Some(Self::DiscardPlan),
            "plan_mode_respond" => Some(Self::PlanModeRespond),
            "act_mode_respond" => Some(Self::ActModeRespond),
            "focus_chain" => Some(Self::FocusChain),
            _ => None,
        }
    }

    /// Returns true if tool modifies workspace
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::ExecuteCommand
                | Self::WriteToFile
                | Self::ReplaceInFile
                | Self::ApplyPatch
                | Self::DeleteFile
                | Self::ExecuteBackground
                | Self::KillProcess
                | Self::KillPort
                | Self::IndexFiles
                | Self::ReindexWorkspace
                | Self::WatchFiles
                | Self::StopWatching
        )
    }
}

/// Generate repository map
async fn generate_repo_map(args: &Value, workdir: &Path) -> ToolResult {
    let max_tokens = args.get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(2048) as usize;

    let chat_files: Vec<PathBuf> = args.get("chat_files")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| workdir.join(s))
                .collect()
        })
        .unwrap_or_default();

    let other_files: Vec<PathBuf> = args.get("other_files")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| workdir.join(s))
                .collect()
        })
        .unwrap_or_default();

    let mut repo_map = crate::repomap::RepoMap::new(workdir.to_path_buf(), max_tokens);
    
    let map_content = if chat_files.is_empty() && other_files.is_empty() {
        // Generate from entire directory
        repo_map.build_from_directory()
    } else {
        // Generate from specific files
        repo_map.build(&chat_files, &other_files)
    };

    // Save to file if requested
    if let Some(output_file) = args.get("output_file").and_then(|v| v.as_str()) {
        let output_path = workdir.join(output_file);
        match tokio::fs::write(&output_path, &map_content).await {
            Ok(_) => ToolResult::ok(format!(
                "Repository map generated ({} tokens) and saved to: {}\n\n{}",
                map_content.split_whitespace().count(),
                output_path.display(),
                map_content
            )),
            Err(e) => ToolResult::err(format!("Failed to write repo map: {}", e)),
        }
    } else {
        ToolResult::ok(format!(
            "Repository map generated ({} tokens):\n\n{}",
            map_content.split_whitespace().count(),
            map_content
        ))
    }
}

/// List execution traces
async fn list_traces(args: &Value, _workdir: &Path) -> ToolResult {
    let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);

    let traces = trace::list_traces(workspace_id, limit).await;
    
    if traces.is_empty() {
        ToolResult::ok("No execution traces found".to_string())
    } else {
        let mut output = format!("Found {} execution traces:\n\n", traces.len());
        
        for trace in traces.iter().take(10) { // Show first 10
            let status = match trace.status {
                trace::TraceStatus::Running => "🟡 Running",
                trace::TraceStatus::Completed => "✅ Completed",
                trace::TraceStatus::Failed => "❌ Failed",
                trace::TraceStatus::Cancelled => "⚠️ Cancelled",
            };
            
            let duration = if let Some(end_time) = trace.end_time {
                format!(" ({}s)", end_time - trace.start_time)
            } else {
                String::new()
            };

            output.push_str(&format!(
                "• {} - {} - {} tools{}\n  Message: {}\n  ID: {}\n\n",
                status,
                trace.workspace_id,
                trace.tool_calls.len(),
                duration,
                &trace.user_message[..trace.user_message.len().min(80)],
                trace.id
            ));
        }
        
        if traces.len() > 10 {
            output.push_str(&format!("... and {} more traces\n", traces.len() - 10));
        }
        
        ToolResult::ok(output)
    }
}

/// Get detailed trace information
async fn get_trace(args: &Value, _workdir: &Path) -> ToolResult {
    let Some(trace_id) = args.get("trace_id").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'trace_id' parameter".to_string());
    };

    if let Some(trace) = trace::get_trace(trace_id).await {
        let mut output = format!("Trace: {}\n", trace.id);
        output.push_str(&format!("Workspace: {}\n", trace.workspace_id));
        output.push_str(&format!("Status: {:?}\n", trace.status));
        output.push_str(&format!("Message: {}\n", trace.user_message));
        
        if let Some(response) = &trace.ai_response {
            output.push_str(&format!("Response: {}\n", response));
        }
        
        if !trace.tool_calls.is_empty() {
            output.push_str(&format!("\nTool Calls ({}):\n", trace.tool_calls.len()));
            for (i, call) in trace.tool_calls.iter().enumerate() {
                let status = if let Some(result) = &call.result {
                    if result.success { "✅" } else { "❌" }
                } else { "⏳" };
                
                let duration = if let Some(ms) = call.duration_ms {
                    format!(" ({}ms)", ms)
                } else { String::new() };
                
                output.push_str(&format!("  {}. {} {} {}{}\n", 
                    i + 1, status, call.tool_name, 
                    serde_json::to_string(&call.arguments).unwrap_or_default(),
                    duration
                ));
                
                if let Some(result) = &call.result {
                    let preview = if result.output.len() > 100 {
                        format!("{}...", &result.output[..100])
                    } else {
                        result.output.clone()
                    };
                    output.push_str(&format!("     Result: {}\n", preview));
                }
            }
        }
        
        ToolResult::ok(output)
    } else {
        ToolResult::err(format!("Trace not found: {}", trace_id))
    }
}

/// Generate trace dashboard HTML
async fn trace_dashboard(args: &Value, workdir: &Path) -> ToolResult {
    let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(50);

    let traces = trace::list_traces(workspace_id, Some(limit)).await;
    let html = trace::generate_dashboard_html(&traces);
    
    // Save to file
    let dashboard_path = workdir.join("forge-traces-dashboard.html");
    match tokio::fs::write(&dashboard_path, &html).await {
        Ok(_) => ToolResult::ok(format!(
            "📊 Trace dashboard generated: {}\n\nOpen in browser to view {} traces",
            dashboard_path.display(),
            traces.len()
        )),
        Err(e) => ToolResult::err(format!("Failed to write dashboard: {}", e)),
    }
}

/// Tool call from LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
    /// Gemini 3 thought signature (must be passed back for function calling)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

/// Result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self { success: true, output: output.into() }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self { success: false, output: output.into() }
    }
}

/// Execute a tool call
pub async fn execute(tool: &ToolCall, workdir: &Path, plan_mode: bool) -> ToolResult {
    use std::time::Instant;
    let start = Instant::now();
    
    let Some(t) = Tool::from_name(&tool.name) else {
        return ToolResult::err(format!("Unknown tool: {}", tool.name));
    };

    // Block mutating tools in plan mode
    if plan_mode && t.is_mutating() {
        return ToolResult::err("Cannot modify files in plan mode");
    }

    let result = match t {
        Tool::ExecuteCommand => execute::run(&tool.arguments, workdir).await,
        Tool::ReadFile => files::read(&tool.arguments, workdir).await,
        Tool::WriteToFile => files::write(&tool.arguments, workdir).await,
        Tool::ReplaceInFile => files::replace(&tool.arguments, workdir).await,
        Tool::ApplyPatch => files::apply_patch(&tool.arguments, workdir).await,
        Tool::ListFiles => files::list(&tool.arguments, workdir).await,
        Tool::DeleteFile => files::delete(&tool.arguments, workdir).await,
        // Background processes & ports
        Tool::ExecuteBackground => process::execute_background(&tool.arguments, workdir).await,
        Tool::ReadProcessOutput => process::read_process_output(&tool.arguments, workdir).await,
        Tool::CheckProcessStatus => process::check_process_status(&tool.arguments, workdir).await,
        Tool::KillProcess => process::kill_process(&tool.arguments, workdir).await,
        Tool::WaitForPort => process::wait_for_port(&tool.arguments, workdir).await,
        Tool::CheckPort => process::check_port(&tool.arguments, workdir).await,
        Tool::KillPort => process::kill_port(&tool.arguments, workdir).await,
        Tool::Grep => search::grep(&tool.arguments, workdir).await,
        Tool::Glob => search::glob_search(&tool.arguments, workdir).await,
        Tool::CodebaseSearch => search::semantic(&tool.arguments, workdir).await,
        Tool::IndexFiles => search::index_files(&tool.arguments, workdir).await,
        Tool::ReindexWorkspace => search::reindex_workspace(&tool.arguments, workdir).await,
        Tool::WatchFiles => search::watch_files(&tool.arguments, workdir).await,
        Tool::ScanFiles => search::scan_files(&tool.arguments, workdir).await,
        Tool::StopWatching => search::stop_watching(&tool.arguments, workdir).await,
        Tool::ListTraces => list_traces(&tool.arguments, workdir).await,
        Tool::GetTrace => get_trace(&tool.arguments, workdir).await,
        Tool::TraceDashboard => trace_dashboard(&tool.arguments, workdir).await,
        Tool::GenerateRepoMap => generate_repo_map(&tool.arguments, workdir).await,
        Tool::ListCodeDefinitions => code::list_definitions(&tool.arguments, workdir).await,
        Tool::GetSymbolDefinition => code::get_definition(&tool.arguments, workdir).await,
        Tool::FindSymbolReferences => code::find_references(&tool.arguments, workdir).await,
        Tool::TraceCallChain => code::trace_call_chain(&tool.arguments, workdir).await,
        Tool::ImpactAnalysis => code::impact_analysis(&tool.arguments, workdir).await,
        Tool::Diagnostics => lint::diagnostics(&tool.arguments, workdir).await,
        Tool::GetArchitectureMap => generate_repo_map(&tool.arguments, workdir).await,
        Tool::SearchFunctions => code::search_functions(&tool.arguments, workdir).await,
        Tool::SearchClasses => code::search_classes(&tool.arguments, workdir).await,
        Tool::SearchFiles => code::search_files(&tool.arguments, workdir).await,
        Tool::LspGoToDefinition => code::lsp_go_to_definition(&tool.arguments, workdir).await,
        Tool::LspFindReferences => code::lsp_find_references(&tool.arguments, workdir).await,
        Tool::LspHover => code::lsp_hover(&tool.arguments, workdir).await,
        Tool::WebSearch => web::search(&tool.arguments).await,
        Tool::WebFetch => web::fetch(&tool.arguments).await,
        Tool::FetchDocs => web::fetch_docs(&tool.arguments).await,
        
        // These are handled in ForgeToolAdapter before reaching execute()
        Tool::AttemptCompletion
        | Tool::AskFollowupQuestion
        | Tool::PlanModeRespond
        | Tool::ActModeRespond
        | Tool::FocusChain
        | Tool::CreatePlan
        | Tool::UpdatePlan
        | Tool::AddPlanStep
        | Tool::RemovePlanStep
        | Tool::DiscardPlan
        | Tool::Replan
        | Tool::Think => ToolResult::ok(""),
    };
    
    let elapsed = start.elapsed();
    if elapsed.as_millis() > 100 {
        tracing::info!("⏱ Tool {} completed in {:?}", tool.name, elapsed);
    } else {
        tracing::debug!("⏱ Tool {} completed in {:?}", tool.name, elapsed);
    }
    
    result
}

/// Generate tool definitions for LLM
pub fn definitions(plan_mode: bool) -> Vec<Value> {
    let mut tools = vec![
        // Essential tools
        serde_json::json!({
            "name": "run",
            "description": "Execute a shell command. Use for running builds, tests, git, etc.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The shell command to execute" }
                },
                "required": ["command"]
            }
        }),
        serde_json::json!({
            "name": "read",
            "description": "Read the contents of a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "start_line": { "type": "integer", "description": "Optional start line (1-indexed)" },
                    "end_line": { "type": "integer", "description": "Optional end line" }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "write",
            "description": "Create a new file with the given content",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path for the new file" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }
        }),
        serde_json::json!({
            "name": "replace",
            "description": "Replace text in a file. old_str must match exactly.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "old_str": { "type": "string", "description": "Exact text to find" },
                    "new_str": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old_str", "new_str"]
            }
        }),
        serde_json::json!({
            "name": "apply_patch",
            "description": "Apply a patch to one or more files. Supports two formats:\n1. V4A format (multi-file): Use 'input' parameter with *** Begin Patch / *** Update File: / *** End Patch markers\n2. Unified diff format (single file): Use 'path' and 'patch' parameters",
            "parameters": {
                "type": "object",
                "properties": {
                    "input": { "type": "string", "description": "V4A format patch with *** Begin Patch, *** Update File:, - removals, + additions" },
                    "path": { "type": "string", "description": "Path to file (for unified diff format)" },
                    "patch": { "type": "string", "description": "Unified diff patch content (for single file)" }
                }
            }
        }),
        serde_json::json!({
            "name": "ls",
            "description": "List files in a directory",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path" },
                    "recursive": { "type": "boolean", "description": "List recursively" }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "delete_file",
            "description": "Delete a file or empty directory. Protected paths like .git, node_modules, Cargo.toml cannot be deleted.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to file or directory to delete" }
                },
                "required": ["path"]
            }
        }),
        // Background processes & ports
        serde_json::json!({
            "name": "execute_background",
            "description": "Execute a command in the background (e.g., dev server, watcher). Returns immediately with a PID.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "wait_seconds": { "type": "integer", "description": "Seconds to wait for initial output (default: 3)" }
                },
                "required": ["command"]
            }
        }),
        serde_json::json!({
            "name": "read_process_output",
            "description": "Read stdout/stderr from a background process",
            "parameters": {
                "type": "object",
                "properties": {
                    "pid": { "type": "integer", "description": "Process ID" },
                    "tail_lines": { "type": "integer", "description": "Number of lines from the end (default: 100)" }
                },
                "required": ["pid"]
            }
        }),
        serde_json::json!({
            "name": "check_process_status",
            "description": "Check if background processes are still running",
            "parameters": {
                "type": "object",
                "properties": {
                    "pid": { "type": "integer", "description": "Optional specific PID to check" }
                }
            }
        }),
        serde_json::json!({
            "name": "kill_process",
            "description": "Terminate a background process",
            "parameters": {
                "type": "object",
                "properties": {
                    "pid": { "type": "integer", "description": "Process ID" },
                    "force": { "type": "boolean", "description": "Use SIGKILL (default: false)" }
                },
                "required": ["pid"]
            }
        }),
        serde_json::json!({
            "name": "wait_for_port",
            "description": "Wait until a port is accepting connections",
            "parameters": {
                "type": "object",
                "properties": {
                    "port": { "type": "integer", "description": "Port number" },
                    "host": { "type": "string", "description": "Host (default: localhost)" },
                    "timeout": { "type": "integer", "description": "Max seconds to wait (default: 30)" },
                    "http_check": { "type": "boolean", "description": "Verify HTTP GET returns 2xx/3xx" }
                },
                "required": ["port"]
            }
        }),
        serde_json::json!({
            "name": "check_port",
            "description": "Check if a port is in use",
            "parameters": {
                "type": "object",
                "properties": {
                    "port": { "type": "integer", "description": "Port number" }
                },
                "required": ["port"]
            }
        }),
        serde_json::json!({
            "name": "kill_port",
            "description": "Kill the process using a specific port",
            "parameters": {
                "type": "object",
                "properties": {
                    "port": { "type": "integer", "description": "Port number" }
                },
                "required": ["port"]
            }
        }),
        // SEARCH TOOLS - order matters for model selection
        serde_json::json!({
            "name": "search",
            "description": "SEMANTIC/CONCEPTUAL search - find code by meaning. Use for understanding ('how does X work'), finding related code ('authentication logic'), or exploring unfamiliar areas. This is the PRIMARY search tool.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language query describing what you're looking for" },
                    "path": { "type": "string", "description": "Optional: limit search to directory" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "grep",
            "description": "LITERAL text search - use ONLY when you know the exact string to find (specific function name, error message, import statement). Fast but requires exact match.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Exact text or regex pattern" },
                    "path": { "type": "string", "description": "Directory to search (default: current)" },
                    "glob": { "type": "string", "description": "File filter, e.g., '*.rs'" },
                    "case_insensitive": { "type": "boolean", "description": "Ignore case" },
                    "context": { "type": "integer", "description": "Context lines (0-5)" }
                },
                "required": ["pattern"]
            }
        }),
        serde_json::json!({
            "name": "glob",
            "description": "Find files by name/extension pattern. Returns file paths only.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Pattern like '*.rs', '**/*.test.ts'" },
                    "path": { "type": "string", "description": "Base directory" }
                },
                "required": ["pattern"]
            }
        }),
        serde_json::json!({
            "name": "list_code_definition_names",
            "description": "List function/class/type definitions in a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "get_symbol_definition",
            "description": "Go to the definition of a symbol. Uses LSP when available for accurate results, falls back to tree-sitter/regex search.",
            "parameters": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "The symbol name to find the definition of" },
                    "path": { "type": "string", "description": "File path where the symbol is used (for LSP lookup)" },
                    "line": { "type": "integer", "description": "Line number (0-indexed) where the symbol appears" },
                    "character": { "type": "integer", "description": "Character position (0-indexed) in the line" }
                },
                "required": ["symbol"]
            }
        }),
        serde_json::json!({
            "name": "find_symbol_references",
            "description": "Find all references to a symbol across the codebase. Uses LSP when available for accurate results, falls back to regex search.",
            "parameters": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "The symbol name to find references of" },
                    "path": { "type": "string", "description": "File path where the symbol is defined (for LSP lookup)" },
                    "line": { "type": "integer", "description": "Line number (0-indexed) where the symbol is defined" },
                    "character": { "type": "integer", "description": "Character position (0-indexed) in the line" }
                },
                "required": ["symbol"]
            }
        }),
        serde_json::json!({
            "name": "trace_call_chain",
            "description": "Trace the call chain of a symbol upstream (who calls it) or downstream (what it calls).",
            "parameters": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "The symbol name to trace" },
                    "direction": { "type": "string", "enum": ["upstream", "downstream"], "description": "Direction to trace" },
                    "max_depth": { "type": "integer", "description": "Maximum depth to trace (default: 3)" }
                },
                "required": ["symbol"]
            }
        }),
        serde_json::json!({
            "name": "impact_analysis",
            "description": "Analyze the impact of changing a symbol by finding all its dependents.",
            "parameters": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "The symbol name to analyze" },
                    "max_depth": { "type": "integer", "description": "Maximum depth to analyze (default: 3)" }
                },
                "required": ["symbol"]
            }
        }),
        serde_json::json!({
            "name": "diagnostics",
            "description": "Get compiler/linter errors and warnings for a file or directory. Use this to check code for errors before or after making changes.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File or directory to check. For directories, runs the appropriate build tool (cargo check, tsc, etc.)" },
                    "fix": { "type": "boolean", "description": "If true, attempt to auto-fix issues (when supported by the linter)" }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "web_search",
            "description": "Search the web for information",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "web_fetch",
            "description": "Fetch content from a URL",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" }
                },
                "required": ["url"]
            }
        }),
        serde_json::json!({
            "name": "docs",
            "description": "Fetch official library/framework documentation from Context7. PREFERRED over web_search for programming libraries. Use when you need API details, usage patterns, or aren't familiar with a library.",
            "parameters": {
                "type": "object",
                "properties": {
                    "library": { "type": "string", "description": "Library or framework name (e.g., 'react', 'tokio', 'fastapi')" },
                    "topic": { "type": "string", "description": "Optional: specific topic to focus on (e.g., 'hooks', 'async', 'middleware')" }
                },
                "required": ["library"]
            }
        }),
        serde_json::json!({
            "name": "ask_followup_question",
            "description": "Ask the user a clarifying question",
            "parameters": {
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The question to ask" }
                },
                "required": ["question"]
            }
        }),
        serde_json::json!({
            "name": "think",
            "description": "Write out your reasoning or thoughts about the current task",
            "parameters": {
                "type": "object",
                "properties": {
                    "thought": { "type": "string", "description": "Your reasoning or thoughts" }
                },
                "required": ["thought"]
            }
        }),
        // Architecture & search
        serde_json::json!({
            "name": "repomap",
            "description": "Get a hierarchical map of the codebase: key files, symbols, and their locations. Use this first to understand project structure before diving into code.",
            "parameters": {
                "type": "object",
                "properties": {
                    "max_tokens": { "type": "integer", "description": "Token budget for the map (default: 2048)" }
                }
            }
        }),
        serde_json::json!({
            "name": "search_functions",
            "description": "Find function/method definitions by name pattern across the codebase. Use when you know the name of a function and want to find where it's defined.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Function name or partial name to search for" },
                    "path": { "type": "string", "description": "Optional: limit search to this directory" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "search_classes",
            "description": "Find struct/class/interface/type definitions by name across the codebase.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Type name or partial name to search for" },
                    "path": { "type": "string", "description": "Optional: limit search to this directory" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "search_files",
            "description": "Find files by name pattern (e.g. 'auth', 'config', '*.test.ts'). Returns matching file paths.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Filename or glob pattern to search for" },
                    "path": { "type": "string", "description": "Optional: root directory to search in" }
                },
                "required": ["query"]
            }
        }),
        // LSP tools
        serde_json::json!({
            "name": "lsp_go_to_definition",
            "description": "Precise LSP-based jump to the definition of a symbol. More accurate than get_symbol_definition when you have an exact file position.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path where the symbol appears" },
                    "line": { "type": "integer", "description": "Line number (0-indexed)" },
                    "character": { "type": "integer", "description": "Character offset (0-indexed)" }
                },
                "required": ["path", "line", "character"]
            }
        }),
        serde_json::json!({
            "name": "lsp_find_references",
            "description": "Find all usages of a symbol via LSP. More accurate than find_symbol_references when you have an exact file position.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path where the symbol is defined" },
                    "line": { "type": "integer", "description": "Line number (0-indexed)" },
                    "character": { "type": "integer", "description": "Character offset (0-indexed)" }
                },
                "required": ["path", "line", "character"]
            }
        }),
        serde_json::json!({
            "name": "lsp_hover",
            "description": "Get type information and documentation for a symbol at a specific position via LSP.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "line": { "type": "integer", "description": "Line number (0-indexed)" },
                    "character": { "type": "integer", "description": "Character offset (0-indexed)" }
                },
                "required": ["path", "line", "character"]
            }
        }),
        // Replan
        serde_json::json!({
            "name": "replan",
            "description": "Replace the current plan with a new one when the original approach isn't working. Include the reason for replanning.",
            "parameters": {
                "type": "object",
                "properties": {
                    "reason": { "type": "string", "description": "Why the current plan failed or needs to change" },
                    "steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "New ordered list of steps"
                    }
                },
                "required": ["reason", "steps"]
            }
        }),
        serde_json::json!({
            "name": "attempt_completion",
            "description": "Signal that the task is fully complete. Call this when all work is done and verified. Include a concise summary of what was accomplished.",
            "parameters": {
                "type": "object",
                "properties": {
                    "result": { "type": "string", "description": "Summary of what was done" }
                },
                "required": ["result"]
            }
        }),
        // Planning tools
        serde_json::json!({
            "name": "create_plan",
            "description": "Create a step-by-step plan for a complex multi-file task. Call after exploring the codebase. Each step should be atomic and verifiable.",
            "parameters": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Ordered list of step descriptions"
                    }
                },
                "required": ["steps"]
            }
        }),
        serde_json::json!({
            "name": "update_plan",
            "description": "Update the status or description of a plan step as you execute it.",
            "parameters": {
                "type": "object",
                "properties": {
                    "step_number": { "type": "integer", "description": "The step number to update (1-indexed)" },
                    "status": { "type": "string", "enum": ["pending", "in_progress", "done", "failed"], "description": "New status for the step" },
                    "new_description": { "type": "string", "description": "Optional updated description" }
                },
                "required": ["step_number", "status"]
            }
        }),
        serde_json::json!({
            "name": "add_plan_step",
            "description": "Insert a new step into the plan after a given position.",
            "parameters": {
                "type": "object",
                "properties": {
                    "after_step": { "type": "integer", "description": "Insert after this step number (0 = prepend)" },
                    "description": { "type": "string", "description": "Description of the new step" }
                },
                "required": ["after_step", "description"]
            }
        }),
        serde_json::json!({
            "name": "remove_plan_step",
            "description": "Remove a step from the plan.",
            "parameters": {
                "type": "object",
                "properties": {
                    "step_number": { "type": "integer", "description": "The step number to remove (1-indexed)" }
                },
                "required": ["step_number"]
            }
        }),
        serde_json::json!({
            "name": "discard_plan",
            "description": "Discard the entire current plan and start fresh.",
            "parameters": {
                "type": "object",
                "properties": {}
            }
        }),
    ];

    // Filter out mutating tools in plan mode
    if plan_mode {
        tools.retain(|t| {
            let name = t["name"].as_str().unwrap_or("");
            !matches!(name, "run" | "write" | "replace" | "apply_patch" | "execute_background" | "kill_process" | "kill_port")
        });
    }

    tools
}

// Re-export functions needed by api/mod.rs
pub use search::{init_embedding_provider, start_background_indexing, semantic as codebase_search, grep};
