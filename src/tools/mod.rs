mod execute;
mod files;
mod search;
mod code;
mod web;
mod embeddings;
mod treesitter;
pub mod ide;

pub use embeddings::{EmbeddingProvider, EmbeddingStore};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub use execute::*;
pub use files::*;
pub use search::*;
pub use code::*;
pub use web::*;

/// All available tools
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    // Essential
    ExecuteCommand,
    ReadFile,
    WriteToFile,
    ReplaceInFile,
    ApplyPatch,
    ListFiles,
    SearchFiles,
    AttemptCompletion,
    AskFollowupQuestion,
    
    // Code intelligence
    CodebaseSearch,
    ListCodeDefinitions,
    GetSymbolDefinition,
    FindSymbolReferences,
    
    // Web
    WebSearch,
    WebFetch,
    
    // Mode control
    PlanModeRespond,
    ActModeRespond,
    FocusChain,
}

impl Tool {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ExecuteCommand => "execute_command",
            Self::ReadFile => "read_file",
            Self::WriteToFile => "write_to_file",
            Self::ReplaceInFile => "replace_in_file",
            Self::ApplyPatch => "apply_patch",
            Self::ListFiles => "list_files",
            Self::SearchFiles => "search_files",
            Self::AttemptCompletion => "attempt_completion",
            Self::AskFollowupQuestion => "ask_followup_question",
            Self::CodebaseSearch => "codebase_search",
            Self::ListCodeDefinitions => "list_code_definition_names",
            Self::GetSymbolDefinition => "get_symbol_definition",
            Self::FindSymbolReferences => "find_symbol_references",
            Self::WebSearch => "web_search",
            Self::WebFetch => "web_fetch",
            Self::PlanModeRespond => "plan_mode_respond",
            Self::ActModeRespond => "act_mode_respond",
            Self::FocusChain => "focus_chain",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "execute_command" => Some(Self::ExecuteCommand),
            "read_file" => Some(Self::ReadFile),
            "write_to_file" => Some(Self::WriteToFile),
            "replace_in_file" => Some(Self::ReplaceInFile),
            "apply_patch" => Some(Self::ApplyPatch),
            "list_files" => Some(Self::ListFiles),
            "search_files" => Some(Self::SearchFiles),
            "attempt_completion" => Some(Self::AttemptCompletion),
            "ask_followup_question" => Some(Self::AskFollowupQuestion),
            "codebase_search" => Some(Self::CodebaseSearch),
            "list_code_definition_names" => Some(Self::ListCodeDefinitions),
            "get_symbol_definition" => Some(Self::GetSymbolDefinition),
            "find_symbol_references" => Some(Self::FindSymbolReferences),
            "web_search" => Some(Self::WebSearch),
            "web_fetch" => Some(Self::WebFetch),
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
        )
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
    let Some(t) = Tool::from_name(&tool.name) else {
        return ToolResult::err(format!("Unknown tool: {}", tool.name));
    };

    // Block mutating tools in plan mode
    if plan_mode && t.is_mutating() {
        return ToolResult::err("Cannot modify files in plan mode");
    }

    match t {
        Tool::ExecuteCommand => execute::run(&tool.arguments, workdir).await,
        Tool::ReadFile => files::read(&tool.arguments, workdir).await,
        Tool::WriteToFile => files::write(&tool.arguments, workdir).await,
        Tool::ReplaceInFile => files::replace(&tool.arguments, workdir).await,
        Tool::ApplyPatch => files::apply_patch(&tool.arguments, workdir).await,
        Tool::ListFiles => files::list(&tool.arguments, workdir).await,
        Tool::SearchFiles => search::grep(&tool.arguments, workdir).await,
        Tool::CodebaseSearch => search::semantic(&tool.arguments, workdir).await,
        Tool::ListCodeDefinitions => code::list_definitions(&tool.arguments, workdir).await,
        Tool::GetSymbolDefinition => code::get_definition(&tool.arguments, workdir).await,
        Tool::FindSymbolReferences => code::find_references(&tool.arguments, workdir).await,
        Tool::WebSearch => web::search(&tool.arguments).await,
        Tool::WebFetch => web::fetch(&tool.arguments).await,
        
        // These are handled specially by the agent
        Tool::AttemptCompletion 
        | Tool::AskFollowupQuestion
        | Tool::PlanModeRespond
        | Tool::ActModeRespond
        | Tool::FocusChain => ToolResult::ok(""),
    }
}

/// Generate tool definitions for LLM
pub fn definitions(plan_mode: bool) -> Vec<Value> {
    let mut tools = vec![
        // Essential tools
        serde_json::json!({
            "name": "execute_command",
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
            "name": "read_file",
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
            "name": "write_to_file",
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
            "name": "replace_in_file",
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
            "name": "list_files",
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
            "name": "search_files",
            "description": "Search for text patterns in files using regex",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search" },
                    "path": { "type": "string", "description": "Directory to search in" },
                    "file_pattern": { "type": "string", "description": "Glob pattern for files (e.g., *.rs)" }
                },
                "required": ["pattern"]
            }
        }),
        serde_json::json!({
            "name": "codebase_search",
            "description": "Semantic search - find code by meaning, not exact text",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language query" },
                    "path": { "type": "string", "description": "Directory to search" }
                },
                "required": ["query"]
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
            "name": "attempt_completion",
            "description": "Signal task completion with a result message",
            "parameters": {
                "type": "object",
                "properties": {
                    "result": { "type": "string", "description": "Summary of what was done" }
                },
                "required": ["result"]
            }
        }),
    ];

    // Filter out mutating tools in plan mode
    if plan_mode {
        tools.retain(|t| {
            let name = t["name"].as_str().unwrap_or("");
            !matches!(name, "execute_command" | "write_to_file" | "replace_in_file" | "apply_patch")
        });
    }

    tools
}
