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
    Local(RigAgent<openai::completion::CompletionModel>),  // for mlx_lm.server and other local OpenAI-compatible servers
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

    pub planner: Option<RigAgentEnum>,
    pub tool_caller: Option<RigAgentEnum>,
    pub reasoner: Option<RigAgentEnum>,
    pub agent: Option<RigAgentEnum>,

    pub current_phase: AgentPhase,
    pub plan: Vec<PlanStep>,
    pub tool_state: Arc<Mutex<crate::llm::tools::AgentState>>,
    pub changed_files: Vec<String>,
    pub insights: Vec<String>,
    pub smart_enrich_count: usize,
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
            planner: None,
            tool_caller: None,
            reasoner: None,
            agent: None,
            current_phase: AgentPhase::Explore,
            plan: Vec::new(),
            tool_state,
            changed_files: Vec::new(),
            insights: Vec::new(),
            smart_enrich_count: 0,
        };

        agent.init_rig_agent().await?;

        Ok(agent)
    }

    async fn init_rig_agent(&mut self) -> Result<()> {
        let preamble = self.build_system_prompt();
        let tools = self.create_rig_tools();
        
        // Planner agent (has search and planning tools)
        let mut search_tools = Vec::new();
        for t in self.create_rig_tools() {
            if ["search", "grep", "read", "ls", "repomap", "list_code_definition_names", "search_functions", "search_classes",
                "create_plan", "update_plan", "add_plan_step", "remove_plan_step", "replan", "discard_plan"].contains(&t.name().as_str()) {
                search_tools.push(t);
            }
        }
        self.planner = Some(self.create_agent_enum(&self.config.clone(), &preamble, search_tools).await?);
        
        // Tool caller (has all tools)
        let all_tools = self.create_rig_tools();
        self.tool_caller = Some(self.create_agent_enum(&self.config.clone(), &preamble, all_tools).await?);
        
        // Reasoner (no tools)
        self.reasoner = Some(self.create_agent_enum(&self.config.clone(), &preamble, vec![]).await?);
        
        // Default agent (for backward compatibility)
        self.agent = Some(self.create_agent_enum(&self.config.clone(), &preamble, tools).await?);
        
        Ok(())
    }

    fn get_rig_history(&self) -> Vec<RigMessage> {
        let filtered: Vec<&Message> = self.messages.iter()
            .filter(|m| matches!(m.role, Role::User | Role::Assistant))
            .collect();
        let window = &filtered[filtered.len().saturating_sub(30)..];
        window.iter().map(|msg| match msg.role {
            Role::User      => RigMessage::user(&msg.content),
            Role::Assistant => RigMessage::assistant(&msg.content),
            _               => unreachable!(),
        }).collect()
    }

    async fn agent_chat(&self, role: &str, prompt: &str, history: Vec<RigMessage>) -> Result<String> {
        let agent = match role {
            "planner" => self.planner.as_ref(),
            "tool_caller" => self.tool_caller.as_ref(),
            "reasoner" => self.reasoner.as_ref(),
            _ => self.agent.as_ref(),
        }.ok_or_else(|| anyhow::anyhow!("Agent for role {} not initialized", role))?;

        let response = match agent {
            RigAgentEnum::OpenAI(agent) => agent.chat(prompt, history).await?,
            RigAgentEnum::Local(agent) => agent.chat(prompt, history).await?,
            RigAgentEnum::Anthropic(agent) => agent.chat(prompt, history).await?,
            RigAgentEnum::Gemini(agent) => agent.chat(prompt, history).await?,
        };
        Ok(response)
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
                // mlx_lm.server — uses /v1/chat/completions (NOT the OpenAI Responses API)
                let base_url = config.base_url.as_deref()
                    .filter(|u| *u != "native-mlx")
                    .unwrap_or("http://localhost:8080/v1");
                // Auto-start the server if it isn't already running
                llm::ensure_mlx_server(base_url, &config.model).await?;
                let agent = llm::create_local_agent_builder(base_url, &config.model)?
                    .preamble(preamble)
                    .tools(tools)
                    .build();
                Ok(RigAgentEnum::Local(agent))
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
            tool_caller: None,
            reasoner: None,
            agent: None,
            current_phase: AgentPhase::Explore,
            plan: Vec::new(),
            tool_state,
            changed_files: Vec::new(),
            insights: Vec::new(),
            smart_enrich_count: 0,
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

        let rig_history = self.get_rig_history();

        // 1. EXPLORE
        self.current_phase = AgentPhase::Explore;
        println!("🔍 Phase: Explore - Analyzing codebase...");
        let explore_prompt = format!(
            "You are in the EXPLORE phase. Your goal is to gather context to solve the user's task.\n\
             Task: {prompt}\n\n\
             Use search, grep, and other search tools to understand the relevant code and architecture. \n\
             When you have a good understanding, provide a summary of your findings and what needs to be changed."
        );
        let explore_results = self.agent_chat("planner", &explore_prompt, vec![]).await?;
        let explore_results = strip_thinking(&explore_results);
        println!("✅ Exploration complete.\n");

        // 2. THINK (Planning)
        self.current_phase = AgentPhase::Think;
        println!("🧠 Phase: Think - Planning solution...");
        let plan_prompt = format!(
            "You are in the THINK phase. Based on the exploration findings, create a detailed step-by-step plan to solve the task.\n\
             Task: {prompt}\n\n\
             Exploration Findings:\n{explore_results}\n\n\
             Use the `create_plan` tool to define the plan. Then provide a brief summary of the plan."
        );
        let _plan_summary = self.agent_chat("planner", &plan_prompt, vec![]).await?;
        println!("✅ Plan created.\n");

        // 3. EXECUTE
        self.current_phase = AgentPhase::Execute;
        println!("🚀 Phase: Execute - Implementation...");
        
        let plan_str = {
            let state = self.tool_state.lock().unwrap();
            state.plan.iter().map(|s| format!("{}. {} ({})", s.number, s.description, s.status)).collect::<Vec<_>>().join("\n")
        };

        let execute_prompt = format!(
            "You are in the EXECUTE phase. Follow the plan to solve the task.\n\
             Task: {prompt}\n\n\
             Exploration Findings:\n{explore_results}\n\n\
             Current Plan:\n{plan_str}\n\n\
             Execute the steps. Update the plan status as you go using `update_plan`. \n\
             If you encounter issues, you can search for more info or adjust the plan. \n\
             Finalize with a summary of what was accomplished."
        );
        
        let response = self.agent_chat("tool_caller", &execute_prompt, rig_history).await?;
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

    fn build_system_prompt(&self) -> String {
        let mode = if self.config.plan_mode { "PLAN" } else { "ACT" };
        let phase_str = format!("{:?}", self.current_phase).to_uppercase();
        
        let core_preamble = match self.current_phase {
            AgentPhase::Explore => prompts::MASTER_PLANNING_PROMPT,
            AgentPhase::Think => prompts::THINK_PROMPT,
            AgentPhase::Execute => prompts::SYSTEM_PROMPT,
            AgentPhase::Verify => prompts::REPLAN_PROMPT,
            AgentPhase::Reflect => prompts::REFLECT_PROMPT,
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
