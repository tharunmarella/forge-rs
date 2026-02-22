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

use std::sync::{Arc, Mutex};

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

mod types;
mod prompts;
pub use types::{AgentPhase, PlanStep};

pub enum RigAgentEnum {
    OpenAI(RigAgent<openai::responses_api::ResponsesCompletionModel>),
    Anthropic(RigAgent<anthropic::completion::CompletionModel>),
    Gemini(RigAgent<gemini::completion::CompletionModel>),
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

    pub agent: Option<RigAgentEnum>,

    pub current_phase: AgentPhase,
    pub plan: Vec<PlanStep>,
    pub tool_state: Arc<Mutex<crate::llm::tools::AgentState>>,
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
        
        let tool_state = Arc::new(Mutex::new(crate::llm::tools::AgentState::default()));

        let mut agent = Self {
            config,
            workdir,
            context,
            messages: Vec::new(),
            doc_prefetcher: DocPrefetcher::new(),
            repo_map,
            session: Some(session),
            agent: None,
            current_phase: AgentPhase::Explore,
            plan: Vec::new(),
            tool_state,
        };

        agent.init_rig_agent().await?;

        Ok(agent)
    }

    async fn init_rig_agent(&mut self) -> Result<()> {
        let preamble = self.build_system_prompt();
        let tools = self.create_rig_tools();
        self.agent = Some(self.create_agent_enum(&self.config.clone(), &preamble, tools).await?);
        Ok(())
    }

    fn create_rig_tools(&self) -> Vec<Box<dyn ToolDyn>> {
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
                state: self.tool_state.clone(),
            }));
        }
        rig_tools
    }

    /// Helper to create a RigAgentEnum based on config
    async fn create_agent_enum(&self, config: &Config, preamble: &str, tools: Vec<Box<dyn ToolDyn>>) -> Result<RigAgentEnum> {
        match config.provider.as_str() {
            "openai" | "groq" | "together" | "openrouter" => {
                let agent = llm::create_openai_agent_builder(config)?
                    .preamble(preamble)
                    .tools(tools)
                    .build();
                Ok(RigAgentEnum::OpenAI(agent))
            }
            "anthropic" => {
                let agent = llm::create_anthropic_agent_builder(config)?
                    .preamble(preamble)
                    .tools(tools)
                    .build();
                Ok(RigAgentEnum::Anthropic(agent))
            }
            "gemini" => {
                let agent = llm::create_gemini_agent_builder(config)?
                    .preamble(preamble)
                    .tools(tools)
                    .build();
                Ok(RigAgentEnum::Gemini(agent))
            }
            "mlx" => {
                // MLX runs via mlx_lm.server — an OpenAI-compatible HTTP server on localhost
                // Start with: python -m mlx_lm.server --model <model>
                let mut mlx_config = config.clone();
                if mlx_config.base_url.is_none() || mlx_config.base_url.as_deref() == Some("native-mlx") {
                    mlx_config.base_url = Some("http://localhost:8000/v1".to_string());
                }
                let agent = llm::create_openai_agent_builder(&mlx_config)?
                    .preamble(preamble)
                    .tools(tools)
                    .build();
                Ok(RigAgentEnum::OpenAI(agent))
            }
            _ => Err(anyhow::anyhow!("Unknown provider: {}", config.provider)),
        }
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
        
        let tool_state = Arc::new(Mutex::new(crate::llm::tools::AgentState::default()));

        let mut agent = Self {
            config,
            workdir,
            context,
            messages,
            doc_prefetcher: DocPrefetcher::new(),
            repo_map,
            session: Some(session),
            agent: None,
            current_phase: AgentPhase::Explore,
            plan: Vec::new(),
            tool_state,
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

    /// Run a single prompt.
    ///
    /// Rig's `agent.chat()` drives the full agentic loop: it calls
    /// `completion()`, executes any tool calls the model returns, feeds the
    /// results back, and repeats until the model produces a plain-text answer.
    /// We therefore only need to call it once per user prompt.
    pub async fn run_prompt(&mut self, prompt: &str) -> Result<()> {
        println!("📝 Task: {prompt}\n");

        // Start background doc prefetch for this query
        self.doc_prefetcher.prefetch_async(prompt.to_string());

        // Sliding window: keep last 30 User/Assistant messages to stay within context limits
        let filtered: Vec<&Message> = self.messages.iter()
            .filter(|m| matches!(m.role, Role::User | Role::Assistant))
            .collect();
        let window = &filtered[filtered.len().saturating_sub(30)..];
        let rig_history: Vec<RigMessage> = window.iter().map(|msg| match msg.role {
            Role::User      => RigMessage::user(&msg.content),
            Role::Assistant => RigMessage::assistant(&msg.content),
            _               => unreachable!(),
        }).collect();

        // Explore phase: pre-fetch relevant context before the LLM call
        let explore_ctx = self.explore(prompt).await;
        let augmented_prompt = if explore_ctx.is_empty() {
            prompt.to_string()
        } else {
            format!("# Pre-fetched Context\n{}\n\n# Task\n{}", explore_ctx, prompt)
        };

        let rig_agent = self.agent.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Agent not initialized"))?;

        // One call — rig handles the internal tool-call / result loop
        let response = match rig_agent {
            RigAgentEnum::OpenAI(agent) => agent.chat(&augmented_prompt, rig_history).await?,
            RigAgentEnum::Anthropic(agent) => agent.chat(&augmented_prompt, rig_history).await?,
            RigAgentEnum::Gemini(agent) => agent.chat(&augmented_prompt, rig_history).await?,
        };

        let response = strip_thinking(&response);
        println!("\n{}\n", response);

        // Sync any plan updates written by tools
        {
            let state = self.tool_state.lock().unwrap();
            self.plan = state.plan.clone();
        }

        // Persist the exchange
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

    /// Pre-fetch relevant code context before the main LLM call (explore phase).
    /// Runs semantic search + a keyword grep in parallel and returns a compact
    /// context string (≤ 2000 chars) to prepend to the user prompt.
    async fn explore(&self, prompt: &str) -> String {
        // Extract the first "significant" word for the keyword grep
        let keyword = prompt
            .split_whitespace()
            .find(|w| w.len() > 3 && !matches!(*w, "what" | "how" | "does" | "the" | "this" | "that" | "with" | "from" | "into" | "when" | "where" | "which"))
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string();

        let semantic_args = serde_json::json!({ "query": prompt });
        let grep_args = serde_json::json!({ "pattern": keyword, "context": 1 });
        let workdir = self.workdir.clone();

        let (semantic_result, grep_result) = tokio::join!(
            tools::codebase_search(&semantic_args, &workdir),
            tools::grep(&grep_args, &workdir),
        );

        let mut parts: Vec<String> = Vec::new();

        if semantic_result.success && !semantic_result.output.trim().is_empty() {
            parts.push(format!("## Semantic search\n{}", semantic_result.output.trim()));
        }
        if grep_result.success && !grep_result.output.trim().is_empty() && !keyword.is_empty() {
            parts.push(format!("## Grep: `{}`\n{}", keyword, grep_result.output.trim()));
        }

        if parts.is_empty() {
            return String::new();
        }

        let combined = parts.join("\n\n");
        // Cap at 2000 chars so we don't bloat the context window
        if combined.len() > 2000 {
            format!("{}…", &combined[..2000])
        } else {
            combined
        }
    }

    /// Get completion from LLM (Legacy, now handled by Rig)
    async fn get_completion_streaming(&self) -> Result<AgentResponse> {
        Err(anyhow::anyhow!("Legacy completion method called. Use run_prompt instead."))
    }

    fn build_system_prompt(&self) -> String {
        let mode = if self.config.plan_mode { "PLAN" } else { "ACT" };
        let phase_str = format!("{:?}", self.current_phase).to_uppercase();
        
        let core_preamble = match self.current_phase {
            AgentPhase::Explore => prompts::MASTER_PLANNING_PROMPT,
            AgentPhase::Think => prompts::THINK_PROMPT,
            AgentPhase::Execute => prompts::SYSTEM_PROMPT,
            AgentPhase::Verify => prompts::REPLAN_PROMPT,
        };

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

        // Include plan if present
        let plan_section = if !self.plan.is_empty() {
            let mut s = String::from("\n# Current Plan\n");
            for step in &self.plan {
                let status = match step.status.as_str() {
                    "done" => "✅",
                    "in_progress" => "🟡",
                    "failed" => "❌",
                    _ => "⏳",
                };
                s.push_str(&format!("{} {}. {}\n", status, step.number, step.description));
            }
            s
        } else {
            String::new()
        };
        
        format!(r#"{core_preamble}

# Context: {mode} | Phase: {phase_str}

# Environment
- Working directory: {}
- Files: {}
{repo_map_section}{plan_section}

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

/// Strip thinking/reasoning blocks emitted by thinking models (Qwen3, etc.)
/// Handles both <think>...</think> and bare content...</think> (no opening tag).
fn strip_thinking(text: &str) -> String {
    if let Some(end) = text.rfind("</think>") {
        return text[end + "</think>".len()..].trim().to_string();
    }
    let mut result = String::new();
    let mut remaining = text;
    loop {
        match remaining.find("<think>") {
            None => { result.push_str(remaining); break; }
            Some(start) => {
                result.push_str(&remaining[..start]);
                match remaining[start..].find("</think>") {
                    None => break,
                    Some(rel_end) => {
                        remaining = &remaining[start + rel_end + "</think>".len()..];
                    }
                }
            }
        }
    }
    result.trim().to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } 
    else { format!("{}...", &s[..max]) }
}

impl Agent {
    /// Gracefully shutdown the agent and any managed resources
    pub async fn shutdown(&mut self) -> Result<()> {
        // MLX native implementation doesn't require explicit shutdown
        // Resources are automatically cleaned up when dropped
        Ok(())
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // MLX native implementation handles cleanup automatically
    }
}
