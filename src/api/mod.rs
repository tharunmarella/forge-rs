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
    MLX(RigAgent<crate::llm::mlx_client::MLXNativeCompletionModel>),
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
    
    // Tri-Model Architecture
    pub planner: Option<RigAgentEnum>,
    pub reasoner: Option<RigAgentEnum>,
    pub tool_caller: Option<RigAgentEnum>,
    
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
        
        // Initialize local embeddings if using a local model
        if config.is_local_model() {
            let _ = crate::llm::candle_manager::init_candle_manager("sentence-transformers/all-MiniLM-L6-v2".to_string()).await;
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
                        planner: None,
                        reasoner: None,
                        tool_caller: None,
                        current_phase: AgentPhase::Explore,
                        plan: Vec::new(),
                        tool_state,
                    };
        
        agent.init_rig_agent().await?;
        
        Ok(agent)
    }
    
    async fn init_rig_agent(&mut self) -> Result<()> {
        let preamble = self.build_system_prompt();
        
        // Initialize Planner
        let mut planner_config = self.config.clone();
        if let Some(m) = &self.config.planner_model {
            planner_config.model = m.clone();
        }
        self.planner = Some(self.create_agent_enum(&planner_config, &preamble, self.create_rig_tools()).await?);

        // Initialize Reasoner
        let mut reasoner_config = self.config.clone();
        if let Some(m) = &self.config.reasoner_model {
            reasoner_config.model = m.clone();
        }
        self.reasoner = Some(self.create_agent_enum(&reasoner_config, &preamble, self.create_rig_tools()).await?);

        // Initialize Tool Caller
        let mut tool_config = self.config.clone();
        if let Some(m) = &self.config.tool_model {
            tool_config.model = m.clone();
        }
        self.tool_caller = Some(self.create_agent_enum(&tool_config, &preamble, self.create_rig_tools()).await?);
        
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
                let agent = llm::create_mlx_native_agent_builder(config).await?
                    .preamble(preamble)
                    .tools(tools)
                    .build();
                Ok(RigAgentEnum::MLX(agent))
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
            planner: None,
            reasoner: None,
            tool_caller: None,
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

        // Reset phase
        self.current_phase = AgentPhase::Explore;
        {
            let mut state = self.tool_state.lock().unwrap();
            state.current_phase = AgentPhase::Explore;
        }

        // Convert stored conversation history to rig Message format
        let rig_history: Vec<RigMessage> = self.messages.iter().filter_map(|msg| {
            match msg.role {
                Role::User => Some(RigMessage::user(&msg.content)),
                Role::Assistant => Some(RigMessage::assistant(&msg.content)),
                _ => None,
            }
        }).collect();

        // Pick the right agent for the current phase
        let rig_agent = match self.current_phase {
            AgentPhase::Explore | AgentPhase::Think => self.planner.as_ref(),
            AgentPhase::Verify => self.reasoner.as_ref(),
            AgentPhase::Execute => self.tool_caller.as_ref(),
        }.ok_or_else(|| anyhow::anyhow!("Rig agent not initialized"))?;

        // One call — rig handles the internal tool-call / result loop
        let response = match rig_agent {
            RigAgentEnum::OpenAI(agent) => agent.chat(prompt, rig_history).await?,
            RigAgentEnum::Anthropic(agent) => agent.chat(prompt, rig_history).await?,
            RigAgentEnum::Gemini(agent) => agent.chat(prompt, rig_history).await?,
            RigAgentEnum::MLX(agent) => agent.chat(prompt, rig_history).await?,
        };

        println!("\n{}\n", response);

        // Sync any phase / plan updates written by tools
        {
            let state = self.tool_state.lock().unwrap();
            self.current_phase = state.current_phase;
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
