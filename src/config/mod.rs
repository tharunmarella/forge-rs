use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: String,
    pub model: String,
    pub plan_mode: bool,
    
    /// Custom base URL for OpenAI-compatible APIs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    
    /// Available models per provider (customizable)
    #[serde(default)]
    pub models: ModelsConfig,
    
    /// Auto-approve these tools without prompting
    #[serde(default)]
    pub auto_approve: AutoApproveConfig,
    
    // API keys for various providers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gemini_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groq_api_key: Option<String>,
    
    
    /// Enable self-correction loop (lint → fix → retry)
    #[serde(default)]
    pub self_correction: bool,
    
    /// Max retries for self-correction
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    
    /// Edit format for code modifications
    /// Options: "auto", "whole-file", "search-replace", "unified-diff"
    #[serde(default)]
    pub edit_format: String,
    
    /// Disable RepoMap building on startup
    #[serde(default)]
    pub no_repomap: bool,
    
    /// Session timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    
    /// Max turn limit for agent tool-calling loops (default: 10)
    #[serde(default = "default_max_turns")]
    pub max_turns: u64,
    
    /// Local model server URL (for Ollama, LocalAI, MLX, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_server_url: Option<String>,
}

fn default_max_turns() -> u64 {
    10
}

fn default_max_retries() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default = "default_gemini_models")]
    pub gemini: Vec<String>,
    #[serde(default = "default_anthropic_models")]
    pub anthropic: Vec<String>,
    #[serde(default = "default_openai_models")]
    pub openai: Vec<String>,
    #[serde(default = "default_groq_models")]
    pub groq: Vec<String>,
    #[serde(default = "default_mlx_models")]
    pub mlx: Vec<String>,
}


fn default_gemini_models() -> Vec<String> {
    vec![
        "gemini-3-flash-preview".into(),
        "gemini-3-pro-preview".into(),
        "gemini-2.5-flash".into(),
        "gemini-2.5-pro".into(),
        "gemini-2.0-flash-exp".into(),
    ]
}

fn default_anthropic_models() -> Vec<String> {
    vec![
        "claude-sonnet-4-20250514".into(),
        "claude-opus-4-20250514".into(),
        "claude-haiku-4-20250514".into(),
    ]
}

fn default_openai_models() -> Vec<String> {
    vec![
        "gpt-4o".into(),
        "gpt-4o-mini".into(),
        "gpt-4.1".into(),
        "o3-mini".into(),
    ]
}

fn default_groq_models() -> Vec<String> {
    vec![
        "llama-3.3-70b-versatile".into(),
        "llama-3-groq-70b-tool-use".into(),
        "llama-3.1-8b-instant".into(),
    ]
}

fn default_mlx_models() -> Vec<String> {
    vec![
        "mlx-community/Qwen3-Coder-30B-A3B-Instruct-4bit-dwq-v2".into(),
        "mlx-community/Qwen3-Coder-30B-A3B-Instruct-6bit-DWQ-lr3e-7".into(),
        "mlx-community/Qwen3-Coder-30B-A3B-Instruct-8bit".into(),
        "mlx-community/Qwen2.5-Coder-32B-Instruct-4bit".into(),
        "mlx-community/IQuest-Coder-V1-40B-Loop-Instruct-4bit".into(),
        "mlx-community/Qwen2.5-Coder-14B-Instruct-8bit".into(),
        "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit".into(),
        "mlx-community/Qwen2.5-Coder-1.5B-Instruct-4bit".into(),
    ]
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            gemini: default_gemini_models(),
            anthropic: default_anthropic_models(),
            openai: default_openai_models(),
            groq: default_groq_models(),
            mlx: default_mlx_models(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoApproveConfig {
    /// Auto-approve read operations (read, ls, search_files, etc.)
    #[serde(default = "default_true")]
    pub read_operations: bool,
    
    /// Auto-approve write operations (write, replace)
    #[serde(default)]
    pub write_operations: bool,
    
    /// Auto-approve command execution
    #[serde(default)]
    pub commands: bool,
    
    /// Auto-approve all tool calls (YOLO mode)
    #[serde(default)]
    pub yolo: bool,
    
    /// Specific tools to always auto-approve
    #[serde(default)]
    pub tools: HashSet<String>,
    
    /// Command patterns to auto-approve (regex)
    #[serde(default)]
    pub command_patterns: Vec<String>,
}

fn default_true() -> bool { true }

impl Default for AutoApproveConfig {
    fn default() -> Self {
        Self {
            read_operations: true,  // Auto-approve reads by default
            write_operations: false,
            commands: false,
            yolo: false,
            tools: HashSet::new(),
            command_patterns: Vec::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: "gemini".to_string(),
            model: "gemini-2.5-flash".to_string(),
            plan_mode: false,
            base_url: None,
            models: ModelsConfig::default(),
            auto_approve: AutoApproveConfig::default(),
            gemini_api_key: None,
            anthropic_api_key: None,
            openai_api_key: None,
            groq_api_key: None,
            self_correction: true, // Enable by default for local models
            max_retries: 3,
            edit_format: "auto".to_string(), // Auto-detect based on model
            no_repomap: false,
            timeout: None,
            max_turns: 10,
            local_server_url: None,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        
        let mut config = if config_path.exists() {
            let contents = std::fs::read_to_string(&config_path)?;
            serde_json::from_str(&contents).unwrap_or_default()
        } else {
            Self::default_with_auto_detection()
        };

        // Override with environment variables
        if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            config.gemini_api_key = Some(key);
        }
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            config.anthropic_api_key = Some(key);
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            config.openai_api_key = Some(key);
        }

        Ok(config)
    }
    
    /// Create default config with auto-detection of best available provider
    pub fn default_with_auto_detection() -> Self {
        let mut config = Self::default();
        
        // Prefer local models if available (no API key required)
        if Self::is_candle_available() {
            config.provider = "mlx".to_string();
            
            // Apple Silicon optimized defaults - prefer native MLX with Qwen models
            #[cfg(target_arch = "aarch64")]
            {
                config.model = "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit".to_string();
                config.local_server_url = Some("native-mlx".to_string()); // Native MLX integration
            }
            
            // Non-Apple Silicon defaults (fallback to Gemini/Cloud or local MLX if possible)
            #[cfg(not(target_arch = "aarch64"))]
            {
                config.model = "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit".to_string();
                config.local_server_url = Some("native-mlx".to_string());
            }
            
            return config;
        }
        
        // Check for available API keys and prefer them in order of capability
        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            config.provider = "anthropic".to_string();
            config.model = "claude-sonnet-4-20250514".to_string();
        } else if std::env::var("OPENAI_API_KEY").is_ok() {
            config.provider = "openai".to_string();
            config.model = "gpt-4o".to_string();
        } else if std::env::var("GEMINI_API_KEY").is_ok() {
            config.provider = "gemini".to_string();
            config.model = "gemini-2.5-flash".to_string();
        }
        // Otherwise keep default (gemini)
        
        config
    }
    
    /// Check if local models (Candle) are available on this system
    pub fn is_candle_available() -> bool {
        // Candle is built into the binary, so it's always available
        true
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(config_path, contents)?;
        Ok(())
    }

    fn config_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
        Ok(home.join(".forge").join("config.json"))
    }

    pub fn api_key(&self) -> Option<&str> {
        match self.provider.as_str() {
            "gemini" => self.gemini_api_key.as_deref(),
            "anthropic" => self.anthropic_api_key.as_deref(),
            "openai" => self.openai_api_key.as_deref(),
            "groq" => self.groq_api_key.as_deref(),
            "mlx" => Some("local-models"), // Uses local server
            _ if self.is_local_model() => Some("local-auth-bypass"),
            _ => None,
        }
    }
    
    /// Get the base URL for OpenAI-compatible APIs
    pub fn api_base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }
    
    /// Get available models for a provider
    pub fn get_models(&self, provider: &str) -> &[String] {
        match provider {
            "gemini" => &self.models.gemini,
            "anthropic" => &self.models.anthropic,
            "openai" => &self.models.openai,
            "groq" => &self.models.groq,
            "mlx" => &self.models.mlx,
            _ => &self.models.gemini,
        }
    }
    
    /// Check if using a local model (MLX, Ollama, etc.)
    pub fn is_local_model(&self) -> bool {
        self.provider == "mlx" || self.local_server_url.is_some()
    }

    /// Check if a tool should be auto-approved
    pub fn should_auto_approve(&self, tool_name: &str) -> bool {
        // Global YOLO override
        if self.auto_approve.yolo {
            return true;
        }

        // Specific tool override
        if self.auto_approve.tools.contains(tool_name) {
            return true;
        }

        // Category-based approval
        match tool_name {
            // Read operations
            "read" | "ls" | "search" 
            | "list_code_definition_names" | "get_symbol_definition" 
            | "find_symbol_references" | "web_search" | "web_fetch" | "docs"
            | "grep" | "glob" | "diagnostics" | "check_process_status" | "read_process_output"
            | "check_port" | "trace_call_chain" | "impact_analysis" | "scan_files"
            | "list_traces" | "get_trace" | "trace_dashboard" | "generate_repo_map" => {
                self.auto_approve.read_operations
            }
            
            // Write operations
            "write" | "replace" | "apply_patch" | "delete_file"
            | "kill_process" | "kill_port" | "index_files" | "reindex_workspace" 
            | "watch_files" | "stop_watching" => {
                self.auto_approve.write_operations
            }
            
            // Commands
            "run" | "execute_background" | "wait_for_port" => {
                self.auto_approve.commands
            }
            
            // Always auto-approve these (no side effects)
            "attempt_completion" | "ask_followup_question" | "think"
            | "plan_mode_respond" | "act_mode_respond" | "focus_chain" => true,
            
            _ => false,
        }
    }
}
