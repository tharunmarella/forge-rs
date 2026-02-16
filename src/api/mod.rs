use crate::llm::{self, tools::ForgeToolAdapter};
use rig::completion::{Chat, Message as RigMessage};
use rig::agent::Agent as RigAgent;
use rig::tool::ToolDyn;
use rig::providers::{openai, anthropic, gemini};

use crate::config::Config;
use crate::context::Context;
use crate::context7::DocPrefetcher;
use crate::repomap::RepoMap;
use crate::session::Session;
use crate::tools::{self, ToolCall, ToolResult};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

pub enum RigAgentEnum {
    OpenAI(RigAgent<openai::responses_api::ResponsesCompletionModel>),
    Anthropic(RigAgent<anthropic::completion::CompletionModel>),
    Gemini(RigAgent<gemini::completion::CompletionModel>),
    MLX(RigAgent<openai::responses_api::ResponsesCompletionModel>),
}

/// The main agent that orchestrates everything
pub struct Agent {
    pub config: Config,
    workdir: PathBuf,
    context: Context,
    pub messages: Vec<Message>,
    doc_prefetcher: DocPrefetcher,
    repo_map: String,
    session: Option<Session>,
    pub rig_agent: Option<RigAgentEnum>,
}

impl Agent {
    pub async fn new(config: Config, workdir: PathBuf) -> Result<Self> {
        let context = Context::new(&workdir).await?;
        
        // Build repo map on startup (1024 token budget)
        let mut repo_map_builder = RepoMap::new(workdir.clone(), 1024);
        let repo_map = repo_map_builder.build_from_directory();
        
        // Initialize embedding provider based on LLM provider
        tools::init_embedding_provider(
            &config.provider,
            config.api_key().as_deref(),
            config.base_url.as_deref(),
        );
        
        // Start background indexing
        tools::start_background_indexing(workdir.clone());
        
        // Initialize trace system
        if let Err(e) = crate::tools::trace::initialize().await {
            tracing::warn!("Failed to initialize trace system: {}", e);
        }
        
        // Create new session
        let session = Session::new(workdir.clone(), &config.provider, &config.model);
        
        let mut agent = Self {
            config,
            workdir,
            context,
            messages: Vec::new(),
            doc_prefetcher: DocPrefetcher::new(),
            repo_map,
            session: Some(session),
            rig_agent: None,
        };
        
        agent.init_rig_agent().await?;
        
        Ok(agent)
    }
    
    async fn init_rig_agent(&mut self) -> Result<()> {
        // Add tools
        let tool_defs = tools::definitions(self.config.plan_mode);
        let mut rig_tools: Vec<Box<dyn ToolDyn>> = Vec::new();
        
        for def in tool_defs {
            let name = def["name"].as_str().unwrap().to_string();
            let description = def["description"].as_str().unwrap().to_string();
            let parameters = def["parameters"].clone();
            
            rig_tools.push(Box::new(ForgeToolAdapter {
                name,
                description,
                parameters,
                workdir: self.workdir.clone(),
                plan_mode: self.config.plan_mode,
            }));
        }
        
        let preamble = self.build_system_prompt();
        
        match self.config.provider.as_str() {
            "openai" | "groq" | "together" | "openrouter" => {
                let rig_agent = llm::create_openai_agent_builder(&self.config)?
                    .preamble(&preamble)
                    .tools(rig_tools)
                    .build();
                self.rig_agent = Some(RigAgentEnum::OpenAI(rig_agent));
            }
            "anthropic" => {
                let rig_agent = llm::create_anthropic_agent_builder(&self.config)?
                    .preamble(&preamble)
                    .tools(rig_tools)
                    .build();
                self.rig_agent = Some(RigAgentEnum::Anthropic(rig_agent));
            }
            "gemini" => {
                let rig_agent = llm::create_gemini_agent_builder(&self.config)?
                    .preamble(&preamble)
                    .tools(rig_tools)
                    .build();
                self.rig_agent = Some(RigAgentEnum::Gemini(rig_agent));
            }
            "mlx" => {
                let rig_agent = llm::create_mlx_agent_builder(&self.config).await?
                    .preamble(&preamble)
                    .tools(rig_tools)
                    .build();
                self.rig_agent = Some(RigAgentEnum::MLX(rig_agent));
            }
            _ => return Err(anyhow::anyhow!("Unknown provider: {}", self.config.provider)),
        }
        
        Ok(())
    }

    /// Create agent and resume from a previous session
    pub async fn resume(config: Config, workdir: PathBuf, session_id: &str) -> Result<Self> {
        let context = Context::new(&workdir).await?;
        
        // Build repo map
        let mut repo_map_builder = RepoMap::new(workdir.clone(), 1024);
        let repo_map = repo_map_builder.build_from_directory();
        
        // Initialize embedding provider
        tools::init_embedding_provider(
            &config.provider,
            config.api_key().as_deref(),
            config.base_url.as_deref(),
        );
        
        // Start background indexing
        tools::start_background_indexing(workdir.clone());
        
        // Load existing session
        let session = Session::load(session_id)?;
        let messages = session.messages.clone();
        
        let mut agent = Self {
            config,
            workdir,
            context,
            messages,
            doc_prefetcher: DocPrefetcher::new(),
            repo_map,
            session: Some(session),
            rig_agent: None,
        };
        
        agent.init_rig_agent().await?;
        
        Ok(agent)
    }
    
    /// Resume the latest session for this workdir
    pub async fn resume_latest(config: Config, workdir: PathBuf) -> Result<Option<Self>> {
        if let Some(session) = Session::load_latest(&workdir)? {
            let agent = Self::resume(config, workdir, &session.id).await?;
            Ok(Some(agent))
        } else {
            Ok(None)
        }
    }
    
    /// Save current session
    pub fn save_session(&mut self) -> Result<()> {
        if let Some(ref mut session) = self.session {
            session.update(&self.messages)?;
        }
        Ok(())
    }
    
    /// Get session ID
    pub fn session_id(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.id.as_str())
    }

    /// Run a single prompt with streaming output
    pub async fn run_prompt(&mut self, prompt: &str) -> Result<()> {
        println!("📝 Task: {prompt}\n");

        // Start background doc prefetch for this query
        self.doc_prefetcher.prefetch_async(prompt.to_string());

        let rig_agent = self.rig_agent.as_ref().ok_or_else(|| anyhow::anyhow!("Rig agent not initialized"))?;
        
        // Convert existing messages to Rig format
        let mut rig_history: Vec<RigMessage> = Vec::new();
        for msg in &self.messages {
            match msg.role {
                Role::User => rig_history.push(RigMessage::user(&msg.content)),
                Role::Assistant => rig_history.push(RigMessage::assistant(&msg.content)),
                _ => {} // Rig handles tool messages internally during chat
            }
        }

        // Use Rig's chat interface
        let response = match rig_agent {
            RigAgentEnum::OpenAI(agent) => agent.chat(prompt, rig_history).await?,
            RigAgentEnum::Anthropic(agent) => agent.chat(prompt, rig_history).await?,
            RigAgentEnum::Gemini(agent) => agent.chat(prompt, rig_history).await?,
            RigAgentEnum::MLX(agent) => agent.chat(prompt, rig_history).await?,
        };
        
        println!("{}", response);
        
        self.messages.push(Message {
            role: Role::User,
            content: prompt.to_string(),
            tool_calls: None,
            tool_results: None,
        });
        
        self.messages.push(Message {
            role: Role::Assistant,
            content: response,
            tool_calls: None,
            tool_results: None,
        });
        
        self.save_session().ok();

        Ok(())
    }

    /// Get completion from LLM (Legacy, now handled by Rig)
    async fn get_completion_streaming(&self) -> Result<AgentResponse> {
        Err(anyhow::anyhow!("Legacy completion method called. Use run_prompt instead."))
    }

    fn build_system_prompt(&self) -> String {
        let mode = if self.config.plan_mode { "PLAN" } else { "ACT" };
        
        // Get any prefetched documentation
        let prefetched_docs = self.doc_prefetcher.get_cached_docs_for_prompt();
        
        // Include repo map if available
        let repo_map_section = if !self.repo_map.is_empty() {
            format!(r#"
# Codebase Map
The following shows key symbols and their locations in the codebase:
```
{}
```
"#, self.repo_map.trim())
        } else {
            String::new()
        };
        
        format!(r#"You are Forge, a CLI coding agent. Mode: {mode}

# Environment
- Working directory: {}
- Files: {}
{repo_map_section}
# Tools
You have access to tools for file operations, code search, and web access.

## Search Strategy
- `codebase_search`: Use for conceptual/semantic queries ("how does X work", "find code related to Y")
- `grep`: Use ONLY for exact text/literal matches (function names, error strings, TODOs)
- `glob`: Use to find files by name pattern (*.rs, test_*.py)
- `get_symbol_definition`: Use to jump to a specific symbol's definition

Always read files before editing.

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
    pub fn repo_map(&self) -> &str { &self.repo_map }
    
    /// Trigger doc prefetch for a query (called from TUI)
    pub fn prefetch_docs(&self, query: &str) {
        self.doc_prefetcher.prefetch_async(query.to_string());
    }
    
    /// Rebuild repo map (call after file changes)
    pub fn refresh_repo_map(&mut self) {
        let mut builder = RepoMap::new(self.workdir.clone(), 1024);
        self.repo_map = builder.build_from_directory();
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

impl Agent {
    /// Gracefully shutdown the agent and any managed resources
    pub async fn shutdown(&mut self) -> Result<()> {
        // Stop MLX server if it was started by this agent
        if matches!(self.config.provider.as_str(), "mlx") {
            if let Err(e) = crate::llm::mlx_manager::stop_mlx_server().await {
                tracing::warn!("Failed to stop MLX server during shutdown: {}", e);
            }
        }
        Ok(())
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Best effort cleanup - can't use async in Drop
        if matches!(self.config.provider.as_str(), "mlx") {
            // Try to stop the server synchronously
            let _ = futures::executor::block_on(crate::llm::mlx_manager::stop_mlx_server());
        }
    }
}
