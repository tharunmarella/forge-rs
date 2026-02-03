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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub together_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openrouter_api_key: Option<String>,
    
    /// Ollama URL (default: http://localhost:11434)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ollama_url: Option<String>,
    
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
    #[serde(default = "default_together_models")]
    pub together: Vec<String>,
    #[serde(default = "default_openrouter_models")]
    pub openrouter: Vec<String>,
    #[serde(default = "default_ollama_models")]
    pub ollama: Vec<String>,
}

fn default_ollama_models() -> Vec<String> {
    vec![
        "qwen2.5-coder:32b".into(),
        "qwen2.5-coder:14b".into(),
        "qwen2.5-coder:7b".into(),
        "deepseek-coder-v2:16b".into(),
        "codestral:22b".into(),
        "llama3.3:70b".into(),
        "llama3.2:latest".into(),
        "mistral:latest".into(),
    ]
}

fn default_gemini_models() -> Vec<String> {
    vec![
        "gemini-2.5-flash".into(),
        "gemini-2.5-pro".into(),
        "gemini-2.0-flash".into(),
        "gemini-3-flash-preview".into(),
        "gemini-3-pro-preview".into(),
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

fn default_together_models() -> Vec<String> {
    vec![
        "Qwen/Qwen2.5-Coder-32B-Instruct".into(),
        "deepseek-ai/DeepSeek-R1".into(),
        "meta-llama/Llama-3.3-70B-Instruct-Turbo".into(),
    ]
}

fn default_openrouter_models() -> Vec<String> {
    vec![
        "anthropic/claude-sonnet-4.5".into(),
        "openai/gpt-4o".into(),
        "deepseek/deepseek-r1".into(),
    ]
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            gemini: default_gemini_models(),
            anthropic: default_anthropic_models(),
            openai: default_openai_models(),
            groq: default_groq_models(),
            together: default_together_models(),
            openrouter: default_openrouter_models(),
            ollama: default_ollama_models(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoApproveConfig {
    /// Auto-approve read operations (read_file, list_files, search_files, etc.)
    #[serde(default = "default_true")]
    pub read_operations: bool,
    
    /// Auto-approve write operations (write_to_file, replace_in_file)
    #[serde(default)]
    pub write_operations: bool,
    
    /// Auto-approve command execution
    #[serde(default)]
    pub commands: bool,
    
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
            together_api_key: None,
            openrouter_api_key: None,
            ollama_url: None,
            self_correction: true, // Enable by default for local models
            max_retries: 3,
            edit_format: "auto".to_string(), // Auto-detect based on model
            no_repomap: false,
            timeout: None,
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
            Self::default()
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
            "together" => self.together_api_key.as_deref(),
            "openrouter" => self.openrouter_api_key.as_deref(),
            "ollama" => Some("ollama"), // Ollama doesn't need API key
            _ => None,
        }
    }
    
    /// Get the base URL for OpenAI-compatible APIs
    pub fn api_base_url(&self) -> Option<&str> {
        if self.provider == "ollama" {
            // Use configured Ollama URL or default
            Some(self.ollama_url.as_deref().unwrap_or("http://localhost:11434/v1"))
        } else {
            self.base_url.as_deref()
        }
    }
    
    /// Get available models for a provider
    pub fn get_models(&self, provider: &str) -> &[String] {
        match provider {
            "gemini" => &self.models.gemini,
            "anthropic" => &self.models.anthropic,
            "openai" => &self.models.openai,
            "groq" => &self.models.groq,
            "together" => &self.models.together,
            "openrouter" => &self.models.openrouter,
            "ollama" => &self.models.ollama,
            _ => &self.models.gemini,
        }
    }
    
    /// Check if using a local model (Ollama)
    pub fn is_local_model(&self) -> bool {
        self.provider == "ollama"
    }

    /// Check if a tool should be auto-approved
    pub fn should_auto_approve(&self, tool_name: &str) -> bool {
        // Specific tool override
        if self.auto_approve.tools.contains(tool_name) {
            return true;
        }

        // Category-based approval
        match tool_name {
            // Read operations
            "read_file" | "list_files" | "codebase_search" 
            | "list_code_definition_names" | "get_symbol_definition" 
            | "find_symbol_references" | "web_search" | "web_fetch" | "fetch_documentation"
            | "grep" | "glob" | "diagnostics" => {
                self.auto_approve.read_operations
            }
            
            // Write operations
            "write_to_file" | "replace_in_file" | "apply_patch" | "delete_file" => {
                self.auto_approve.write_operations
            }
            
            // Commands
            "execute_command" => {
                self.auto_approve.commands
            }
            
            // Always auto-approve these (no side effects)
            "attempt_completion" | "ask_followup_question" | "think"
            | "plan_mode_respond" | "act_mode_respond" | "focus_chain" => true,
            
            _ => false,
        }
    }
}
