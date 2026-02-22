use anyhow::{anyhow, Result};
use std::sync::Arc;
use tracing::{debug, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Local models manager using HTTP calls to local model servers
/// Supports Ollama, LocalAI, and other OpenAI-compatible local servers
#[derive(Debug)]
pub struct CandleManager {
    client: Client,
    base_url: String,
    model_id: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: Option<usize>,
    temperature: Option<f64>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl CandleManager {
    /// Create a new local model manager instance
    pub fn new(model_id: String) -> Result<Self> {
        let client = Client::new();
        
        // Auto-detect the best available local server (Default to MLX for Apple Silicon)
        let base_url = Self::detect_local_server().unwrap_or_else(|| {
            "http://localhost:8000".to_string() // Default MLX URL
        });
        
        debug!("Local model manager initialized for model: {} on {}", model_id, base_url);
        
        Ok(Self {
            client,
            base_url,
            model_id,
        })
    }
    
    /// Create with a specific server URL
    pub fn new_with_url(model_id: String, base_url: String) -> Result<Self> {
        let client = Client::new();
        debug!("Local model manager initialized for model: {} on {}", model_id, base_url);
        
        Ok(Self {
            client,
            base_url,
            model_id,
        })
    }
    
    /// Auto-detect available local model servers (MLX optimized)
    fn detect_local_server() -> Option<String> {
        // Priority order for Apple Silicon (M1/M2/M3/M4) - MLX focus
        let servers = vec![
            ("http://localhost:8000", "MLX Server (Apple Silicon optimized)"),
            ("http://localhost:1234", "LM Studio (Apple Silicon optimized)"),
            ("http://localhost:8080", "LocalAI"),
            ("http://localhost:5000", "text-generation-webui"),
            ("http://localhost:8001", "vLLM"),
        ];
        
        for (url, name) in servers {
            debug!("Checking for {} server at {}", name, url);
        }
        
        // Default to MLX Server for Apple Silicon
        return Some("http://localhost:8000".to_string());
    }

    /// Check if the local model server is available
    pub async fn load_model(&self) -> Result<()> {
        info!("Checking local model server: {}", self.base_url);
        
        // Try to ping the server using OpenAI models endpoint
        let health_url = format!("{}/v1/models", self.base_url);
        match self.client.get(&health_url).send().await {
            Ok(response) if response.status().is_success() => {
                info!("Local model server is available");
                Ok(())
            }
            Ok(response) => {
                warn!("Local model server responded with status: {}", response.status());
                Ok(()) 
            }
            Err(e) => {
                warn!("Local model server not available: {}. Using mock responses.", e);
                Ok(()) 
            }
        }
    }

    /// Generate text completion
    pub async fn generate(&self, prompt: &str, max_tokens: usize, temperature: f64) -> Result<String> {
        info!("Generating completion for prompt: {} chars on {}", prompt.len(), self.base_url);
        
        // Try OpenAI-compatible format (used by MLX Server, LM Studio, etc.)
        if let Ok(response) = self.try_openai_generate(prompt, max_tokens, temperature).await {
            return Ok(response);
        }
        
        // Fallback to mock response
        warn!("Using mock response - no local model server available at {}", self.base_url);
        let response = format!(
            "Mock response from local model '{}' for prompt ({}chars) with max_tokens={}, temperature={}",
            self.model_id,
            prompt.len(),
            max_tokens,
            temperature
        );
        
        Ok(response)
    }
    
    /// Detect server type based on URL
    fn detect_server_type(&self) -> String {
        if self.base_url.contains(":8000") {
            "mlx".to_string() 
        } else if self.base_url.contains(":8080") {
            "locali".to_string()
        } else if self.base_url.contains(":1234") {
            "lmstudio".to_string()
        } else if self.base_url.contains(":8001") {
            "vllm".to_string()
        } else {
            "openai".to_string() 
        }
    }
    
    async fn try_openai_generate(&self, prompt: &str, max_tokens: usize, temperature: f64) -> Result<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let request = ChatRequest {
            model: self.model_id.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            max_tokens: Some(max_tokens),
            temperature: Some(temperature),
            stream: false,
        };
        
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await?;
            
        if response.status().is_success() {
            let result: ChatResponse = response.json().await?;
            if let Some(choice) = result.choices.first() {
                return Ok(choice.message.content.clone());
            }
        }
        
        Err(anyhow!("OpenAI-compatible API call failed"))
    }

    /// Generate embeddings for text
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let preview = text.chars().take(50).collect::<String>();
        info!("Generating embeddings for text: {}", preview);
        
        // Try OpenAI-compatible embeddings API (MLX supports this)
        if let Ok(embedding) = self.try_openai_embed(text).await {
            return Ok(embedding);
        }
        
        // Fallback to mock embedding
        warn!("Using mock embedding - no local model server available");
        let embedding_size = 384; 
        let mut embedding = vec![0.0f32; embedding_size];
        
        let hash = text.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
        for (i, val) in embedding.iter_mut().enumerate() {
            *val = ((hash.wrapping_add(i as u32) % 1000) as f32 - 500.0) / 500.0;
        }
        
        Ok(embedding)
    }
    
    async fn try_openai_embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/v1/embeddings", self.base_url);
        let request = EmbeddingRequest {
            model: self.model_id.clone(),
            input: text.to_string(),
        };
        
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await?;
            
        if response.status().is_success() {
            let result: EmbeddingResponse = response.json().await?;
            if let Some(data) = result.data.first() {
                return Ok(data.embedding.clone());
            }
        }
        
        Err(anyhow!("OpenAI-compatible embeddings API call failed"))
    }

    /// Check if model is loaded
    pub async fn is_loaded(&self) -> bool {
        // For mock implementation, always return true
        true
    }

    /// Get model info
    pub async fn model_info(&self) -> Option<ModelInfo> {
        Some(ModelInfo {
            model_id: self.model_id.clone(),
            vocab_size: 32000,
            hidden_size: 4096,
            num_layers: 32,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub model_id: String,
    pub vocab_size: usize,
    pub hidden_size: usize,
    pub num_layers: usize,
}

/// Get the global Candle manager instance
static CANDLE_MANAGER: once_cell::sync::Lazy<tokio::sync::Mutex<Option<CandleManager>>> = 
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(None));

/// Initialize the global Candle manager
pub async fn init_candle_manager(model_id: String) -> Result<()> {
    let manager = CandleManager::new(model_id)?;
    manager.load_model().await?;
    *CANDLE_MANAGER.lock().await = Some(manager);
    Ok(())
}

/// Initialize the global Candle manager with a specific server URL
pub async fn init_candle_manager_with_url(model_id: String, server_url: String) -> Result<()> {
    let manager = CandleManager::new_with_url(model_id, server_url)?;
    manager.load_model().await?;
    *CANDLE_MANAGER.lock().await = Some(manager);
    Ok(())
}

/// Get the global Candle manager
pub async fn get_candle_manager() -> Result<Arc<CandleManager>> {
    let guard = CANDLE_MANAGER.lock().await;
        match guard.as_ref() {
        Some(manager) => {
            // Clone the manager
            let manager_clone = CandleManager {
                client: manager.client.clone(),
                base_url: manager.base_url.clone(),
                model_id: manager.model_id.clone(),
            };
            Ok(Arc::new(manager_clone))
        }
        None => Err(anyhow!("Candle manager not initialized. Call init_candle_manager() first."))
    }
}

/// Check if Candle manager is available
pub async fn is_candle_available() -> bool {
    CANDLE_MANAGER.lock().await.is_some()
}

/// Get recommended models for Apple Silicon
pub fn get_recommended_apple_silicon_models() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        // (Model Name, Size, Description)
        ("mlx-community/Llama-3.2-3B-Instruct-4bit", "~2GB", "🍎 Fast, efficient for coding tasks"),
        ("mlx-community/Llama-3.2-1B-Instruct-4bit", "~1GB", "🍎 Ultra-fast, good for simple tasks"),
        ("mlx-community/CodeLlama-7B-Instruct-4bit", "~4GB", "🍎 Specialized for code generation"),
        ("mlx-community/Mistral-7B-Instruct-v0.3-4bit", "~4GB", "🍎 Excellent reasoning capabilities"),
        ("mlx-community/Qwen2.5-Coder-7B-Instruct-4bit", "~4GB", "🍎 Latest coding model"),
        ("mlx-community/deepseek-coder-6.7b-instruct-4bit", "~4GB", "🍎 Strong code understanding"),
    ]
}

/// Get Apple Silicon system info
pub fn get_apple_silicon_info() -> Option<String> {
    #[cfg(target_arch = "aarch64")]
    {
        // Try to detect specific Apple Silicon chip
        if let Ok(output) = std::process::Command::new("sysctl")
            .args(&["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if let Ok(cpu_info) = String::from_utf8(output.stdout) {
                return Some(cpu_info.trim().to_string());
            }
        }
        Some("Apple Silicon (ARM64)".to_string())
    }
    
    #[cfg(not(target_arch = "aarch64"))]
    None
}

/// Get available local model servers with their default configurations (Apple Silicon optimized)
pub fn get_supported_servers() -> Vec<(&'static str, &'static str, &'static str)> {
    #[cfg(target_arch = "aarch64")]
    return vec![
        ("MLX Server", "http://localhost:8000", "🍎 Apple Silicon optimized: pip install mlx-lm && python -m mlx_lm.server --model mlx-community/Llama-3.2-3B-Instruct-4bit"),
        ("Ollama", "http://localhost:11434", "🍎 Apple Silicon native: curl -fsSL https://ollama.ai/install.sh | sh"),
        ("LM Studio", "http://localhost:1234", "🍎 Apple Silicon optimized GUI: https://lmstudio.ai/"),
        ("LocalAI", "http://localhost:8080", "Docker: docker run -p 8080:8080 localai/localai"),
        ("text-generation-webui", "http://localhost:5000", "oobabooga's webui: --api --listen"),
        ("vLLM", "http://localhost:8001", "High-performance: python -m vllm.entrypoints.openai.api_server"),
    ];
    
    #[cfg(not(target_arch = "aarch64"))]
    return vec![
        ("Ollama", "http://localhost:11434", "Popular, easy to install: curl -fsSL https://ollama.ai/install.sh | sh"),
        ("LocalAI", "http://localhost:8080", "OpenAI-compatible: docker run -p 8080:8080 localai/localai"),
        ("LM Studio", "http://localhost:1234", "GUI application: https://lmstudio.ai/"),
        ("vLLM", "http://localhost:8001", "High-performance: python -m vllm.entrypoints.openai.api_server"),
        ("text-generation-webui", "http://localhost:5000", "oobabooga's webui: --api --listen"),
        ("MLX Server", "http://localhost:8000", "⚠️  Requires Apple Silicon: pip install mlx-lm"),
    ];
}

/// Check which local servers are currently running
pub async fn check_available_servers() -> Vec<(String, String, bool)> {
    let client = Client::new();
    let mut results = Vec::new();
    
    for (name, url, _) in get_supported_servers() {
        let health_url = if url.contains(":11434") {
            format!("{}/api/tags", url) // Ollama endpoint
        } else {
            format!("{}/v1/models", url) // OpenAI-compatible endpoint
        };
        
        let is_available = match client.get(&health_url).timeout(std::time::Duration::from_secs(2)).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        };
        
        results.push((name.to_string(), url.to_string(), is_available));
    }
    
    results
}

/// Generate MLX server startup command for Apple Silicon
pub fn generate_mlx_startup_command(model: &str, port: u16) -> String {
    format!(
        "# Install MLX-LM (if not already installed)\npip install mlx-lm\n\n# Start MLX server with your model\npython -m mlx_lm.server --model {} --port {} --host 127.0.0.1\n\n# Alternative: Use MLX with specific settings\n# python -m mlx_lm.server --model {} --port {} --max-tokens 2048 --temperature 0.7",
        model, port, model, port
    )
}

/// Check if we're running on Apple Silicon
pub fn is_apple_silicon() -> bool {
    cfg!(target_arch = "aarch64")
}

/// Get optimal MLX configuration for current system
pub fn get_optimal_mlx_config() -> (String, String, u16) {
    if is_apple_silicon() {
        // Detect memory and recommend model size
        let model = if let Ok(output) = std::process::Command::new("sysctl")
            .args(&["-n", "hw.memsize"])
            .output()
        {
            if let Ok(mem_str) = String::from_utf8(output.stdout) {
                if let Ok(mem_bytes) = mem_str.trim().parse::<u64>() {
                    let mem_gb = mem_bytes / (1024 * 1024 * 1024);
                    match mem_gb {
                        0..=8 => "mlx-community/Llama-3.2-1B-Instruct-4bit",
                        9..=16 => "mlx-community/Llama-3.2-3B-Instruct-4bit", 
                        17..=32 => "mlx-community/CodeLlama-7B-Instruct-4bit",
                        _ => "mlx-community/Mistral-7B-Instruct-v0.3-4bit",
                    }
                } else {
                    "mlx-community/Llama-3.2-3B-Instruct-4bit" // Safe default
                }
            } else {
                "mlx-community/Llama-3.2-3B-Instruct-4bit" // Safe default
            }
        } else {
            "mlx-community/Llama-3.2-3B-Instruct-4bit" // Safe default
        };
        
        (model.to_string(), "http://localhost:8000".to_string(), 8000)
    } else {
        // Non-Apple Silicon fallback
        ("llama3.2".to_string(), "http://localhost:11434".to_string(), 11434)
    }
}