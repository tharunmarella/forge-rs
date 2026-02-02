pub mod gemini;
pub mod anthropic;
pub mod openai;
pub mod streaming;

pub use streaming::{StreamEvent, StreamReceiver};

use crate::config::Config;
use crate::context::Context;
use crate::context7::DocPrefetcher;
use crate::tools::{self, ToolCall, ToolResult};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::io::{stdout, Write};

/// Message in conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<Vec<(String, ToolResult)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// The main agent that orchestrates everything
pub struct Agent {
    pub config: Config,
    workdir: PathBuf,
    context: Context,
    pub messages: Vec<Message>,
    doc_prefetcher: DocPrefetcher,
}

impl Agent {
    pub async fn new(config: Config, workdir: PathBuf) -> Result<Self> {
        let context = Context::new(&workdir).await?;
        
        Ok(Self {
            config,
            workdir,
            context,
            messages: Vec::new(),
            doc_prefetcher: DocPrefetcher::new(),
        })
    }

    /// Run a single prompt with streaming output
    pub async fn run_prompt(&mut self, prompt: &str) -> Result<()> {
        println!("📝 Task: {prompt}\n");

        // Start background doc prefetch for this query
        self.doc_prefetcher.prefetch_async(prompt.to_string());

        self.messages.push(Message {
            role: Role::User,
            content: prompt.to_string(),
            tool_calls: None,
            tool_results: None,
        });

        loop {
            let response = self.get_completion_streaming().await?;
            
            match response {
                AgentResponse::Text(text) => {
                    // Text already printed during streaming
                    self.messages.push(Message {
                        role: Role::Assistant,
                        content: text,
                        tool_calls: None,
                        tool_results: None,
                    });
                }
                AgentResponse::ToolCalls { text, calls } => {
                    if !text.is_empty() {
                        println!("{}", text);
                    }
                    
                    let mut results = Vec::new();
                    
                    for call in &calls {
                        // Check auto-approval
                        let approved = self.config.should_auto_approve(&call.name);
                        
                        if !approved {
                            print!("\n\x1b[33m🔧 {} - approve? [y/N]: \x1b[0m", call.name);
                            stdout().flush().ok();
                            
                            let mut input = String::new();
                            std::io::stdin().read_line(&mut input).ok();
                            
                            if !input.trim().eq_ignore_ascii_case("y") {
                                println!("Skipped");
                                continue;
                            }
                        } else {
                            println!("\n\x1b[36m🔧 {}\x1b[0m", call.name);
                        }
                        
                        let result = tools::execute(&call, &self.workdir, self.config.plan_mode).await;
                        
                        if result.success {
                            println!("\x1b[32m✓\x1b[0m {}", truncate(&result.output, 200));
                        } else {
                            println!("\x1b[31m✗\x1b[0m {}", result.output);
                        }
                        
                        results.push((call.name.clone(), result));
                    }
                    
                    self.messages.push(Message {
                        role: Role::Assistant,
                        content: String::new(),
                        tool_calls: Some(calls.clone()),
                        tool_results: None,
                    });
                    
                    self.messages.push(Message {
                        role: Role::Tool,
                        content: String::new(),
                        tool_calls: None,
                        tool_results: Some(results),
                    });
                }
                AgentResponse::Completion(result) => {
                    println!("\n\x1b[32m✅ {result}\x1b[0m");
                    break;
                }
                AgentResponse::Question(q) => {
                    print!("\n\x1b[33m❓ {q}\x1b[0m\n> ");
                    stdout().flush().ok();
                    
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input).ok();
                    
                    self.messages.push(Message {
                        role: Role::User,
                        content: input.trim().to_string(),
                        tool_calls: None,
                        tool_results: None,
                    });
                }
            }
        }

        Ok(())
    }

    /// Get completion from LLM
    async fn get_completion_streaming(&self) -> Result<AgentResponse> {
        let system_prompt = self.build_system_prompt();
        let tool_defs = tools::definitions(self.config.plan_mode);
        
        // Use non-streaming for stability
        match self.config.provider.as_str() {
            "gemini" => gemini::complete(&self.config, &system_prompt, &self.messages, &tool_defs).await,
            "anthropic" => anthropic::complete(&self.config, &system_prompt, &self.messages, &tool_defs).await,
            // OpenAI and OpenAI-compatible providers
            "openai" | "groq" | "together" | "openrouter" => {
                openai::complete(&self.config, &system_prompt, &self.messages, &tool_defs).await
            }
            _ => Err(anyhow::anyhow!("Unknown provider: {}", self.config.provider)),
        }
    }

    fn build_system_prompt(&self) -> String {
        let mode = if self.config.plan_mode { "PLAN" } else { "ACT" };
        
        // Get any prefetched documentation
        let prefetched_docs = self.doc_prefetcher.get_cached_docs_for_prompt();
        
        format!(r#"You are Forge, a CLI coding agent. Mode: {mode}

# Environment
- Working directory: {}
- Files: {}

# Tools
You have access to tools for file operations, code search, and web access.
Use tools to accomplish tasks. Always read files before editing.

# Rules
1. Be concise and direct
2. Use tools efficiently - batch reads when possible
3. Always verify changes with read_file after editing
4. Use attempt_completion when done
5. Ask clarifying questions if needed

{}
{}"#,
            self.workdir.display(),
            self.context.file_summary(),
            if self.config.plan_mode {
                "In PLAN mode: read-only, no file modifications allowed."
            } else {
                "In ACT mode: you can read and modify files."
            },
            prefetched_docs
        )
    }

    pub fn messages(&self) -> &[Message] { &self.messages }
    pub fn workdir(&self) -> &PathBuf { &self.workdir }
    pub fn doc_prefetcher(&self) -> &DocPrefetcher { &self.doc_prefetcher }
    
    /// Trigger doc prefetch for a query (called from TUI)
    pub fn prefetch_docs(&self, query: &str) {
        self.doc_prefetcher.prefetch_async(query.to_string());
    }
}

pub enum AgentResponse {
    Text(String),
    ToolCalls { text: String, calls: Vec<ToolCall> },
    Completion(String),
    Question(String),
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } 
    else { format!("{}...", &s[..max]) }
}
