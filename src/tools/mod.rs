mod execute;
mod files;
mod search;
mod code;
mod web;
mod embeddings;
mod embeddings_store;
mod treesitter;
pub mod ide;
pub mod lint;

pub use embeddings::{EmbeddingProvider, EmbeddingStore};
pub use lint::{lint_file, LintResult, LintError, LintSeverity};

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
    // Essential file operations
    ExecuteCommand,
    ReadFile,
    WriteToFile,
    ReplaceInFile,
    ApplyPatch,
    ListFiles,
    DeleteFile,
    
    // Search
    Grep,
    Glob,
    CodebaseSearch,
    
    // Code intelligence
    ListCodeDefinitions,
    GetSymbolDefinition,
    FindSymbolReferences,
    Diagnostics,
    
    // Web & Documentation
    WebSearch,
    WebFetch,
    FetchDocs,
    
    // Interaction
    AttemptCompletion,
    AskFollowupQuestion,
    Think,
    
    // Mode control (internal)
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
            Self::DeleteFile => "delete_file",
            Self::Grep => "grep",
            Self::Glob => "glob",
            Self::CodebaseSearch => "codebase_search",
            Self::ListCodeDefinitions => "list_code_definition_names",
            Self::GetSymbolDefinition => "get_symbol_definition",
            Self::FindSymbolReferences => "find_symbol_references",
            Self::Diagnostics => "diagnostics",
            Self::WebSearch => "web_search",
            Self::WebFetch => "web_fetch",
            Self::FetchDocs => "fetch_documentation",
            Self::AttemptCompletion => "attempt_completion",
            Self::AskFollowupQuestion => "ask_followup_question",
            Self::Think => "think",
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
            "delete_file" => Some(Self::DeleteFile),
            "grep" => Some(Self::Grep),
            "glob" => Some(Self::Glob),
            "codebase_search" => Some(Self::CodebaseSearch),
            "list_code_definition_names" => Some(Self::ListCodeDefinitions),
            "get_symbol_definition" => Some(Self::GetSymbolDefinition),
            "find_symbol_references" => Some(Self::FindSymbolReferences),
            "diagnostics" => Some(Self::Diagnostics),
            "web_search" => Some(Self::WebSearch),
            "web_fetch" => Some(Self::WebFetch),
            "fetch_documentation" => Some(Self::FetchDocs),
            "attempt_completion" => Some(Self::AttemptCompletion),
            "ask_followup_question" => Some(Self::AskFollowupQuestion),
            "think" => Some(Self::Think),
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
        Tool::Grep => search::grep(&tool.arguments, workdir).await,
        Tool::Glob => search::glob_search(&tool.arguments, workdir).await,
        Tool::CodebaseSearch => search::semantic(&tool.arguments, workdir).await,
        Tool::ListCodeDefinitions => code::list_definitions(&tool.arguments, workdir).await,
        Tool::GetSymbolDefinition => code::get_definition(&tool.arguments, workdir).await,
        Tool::FindSymbolReferences => code::find_references(&tool.arguments, workdir).await,
        Tool::Diagnostics => lint::diagnostics(&tool.arguments, workdir).await,
        Tool::WebSearch => web::search(&tool.arguments).await,
        Tool::WebFetch => web::fetch(&tool.arguments).await,
        Tool::FetchDocs => web::fetch_docs(&tool.arguments).await,
        
        // These are handled specially by the agent
        Tool::AttemptCompletion 
        | Tool::AskFollowupQuestion
        | Tool::PlanModeRespond
        | Tool::ActModeRespond
        | Tool::FocusChain
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
        // SEARCH TOOLS - order matters for model selection
        serde_json::json!({
            "name": "codebase_search",
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
            "name": "fetch_documentation",
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
