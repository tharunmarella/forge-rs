use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tokio::process::{Child, Command, ChildStdin, ChildStdout};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, warn};
use serde::{Deserialize, Serialize};

const MLX_SERVER_SCRIPT: &str = include_str!("../../mlx_server.py");

#[derive(Serialize)]
struct GenerateRequest {
    messages: Vec<serde_json::Value>,
    max_tokens: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: Option<String>,
    error: Option<String>,
}

/// Native MLX manager for Apple Silicon
/// Uses Python MLX-LM server that keeps model loaded in memory
pub struct MLXNativeManager {
    model_id: String,
    python_cmd: String,
    server_process: Arc<Mutex<Option<ServerProcess>>>,
}

struct ServerProcess {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

// Make MLXNativeManager Send + Sync for thread safety
unsafe impl Send for MLXNativeManager {}
unsafe impl Sync for MLXNativeManager {}

impl MLXNativeManager {
    /// Create a new native MLX manager
    pub fn new(model_id: String) -> Result<Self> {
        // Check if we're on Apple Silicon
        #[cfg(not(target_arch = "aarch64"))]
        {
            warn!("MLX is optimized for Apple Silicon. Performance may be limited on this architecture.");
        }
        
        info!("MLX manager initialized for model: {}", model_id);
        
        Ok(Self { 
            model_id,
            python_cmd: "python".to_string(),
            server_process: Arc::new(Mutex::new(None)),
        })
    }
    
    /// Start the MLX server process
    async fn start_server(&self) -> Result<()> {
        let mut server_guard = self.server_process.lock().await;
        
        if server_guard.is_some() {
            return Ok(()); // Server already running
        }
        
        info!("Starting MLX server process...");
        
        // Get path to ~/.forge/bin/mlx_server.py
        let home = dirs::home_dir().ok_or_else(|| anyhow!("No home directory"))?;
        let forge_bin_dir = home.join(".forge").join("bin");
        std::fs::create_dir_all(&forge_bin_dir)?;
        let server_script = forge_bin_dir.join("mlx_server.py");
        
        // Write the embedded script to disk to ensure it's available and up to date
        std::fs::write(&server_script, MLX_SERVER_SCRIPT)?;
        
        let mut child = Command::new(&self.python_cmd)
            .arg(server_script)
            .arg(&self.model_id)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;
        
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("Failed to get stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to get stdout"))?;
        let mut stdout = BufReader::new(stdout);
        
        // Wait for "READY" signal
        let mut ready_line = String::new();
        stdout.read_line(&mut ready_line).await?;
        
        if !ready_line.trim().starts_with("READY") {
            return Err(anyhow!("Server failed to start: {}", ready_line));
        }
        
        info!("✓ MLX server ready");
        
        *server_guard = Some(ServerProcess {
            _child: child,
            stdin,
            stdout,
        });
        
        Ok(())
    }

    /// Load a model from HuggingFace Hub
    pub async fn load_model(&self) -> Result<()> {
        info!("Loading MLX model via Python MLX-LM: {}", self.model_id);
        
        // Try to find Python with mlx_lm installed
        let python_commands = vec!["python", "python3"];
        let mut mlx_available = false;
        let mut working_python = String::new();
        
        for cmd in &python_commands {
            let check = tokio::process::Command::new(cmd)
                .args(&["-c", "import mlx_lm"])
                .output()
                .await;
            
            if check.is_ok() && check.unwrap().status.success() {
                mlx_available = true;
                working_python = cmd.to_string();
                break;
            }
        }
        
        if !mlx_available {
            return Err(anyhow!(
                "Python MLX-LM not installed!\n\
                 Please install it:\n\
                 pip install mlx-lm\n\
                 \n\
                 This enables local models optimized for Apple Silicon."
            ));
        }
        
        // Store working Python command
        let manager = MLXNativeManager {
            model_id: self.model_id.clone(),
            python_cmd: working_python.clone(),
            server_process: self.server_process.clone(),
        };
        
        info!("✓ Python MLX-LM is installed (using '{}')", working_python);
        
        // Start the server (this will load the model and keep it in memory)
        manager.start_server().await?;
        
        Ok(())
    }

    /// Generate text completion using MLX server.
    ///
    /// `messages` is a list of `{role, content}` objects.  The Python server
    /// calls `tokenizer.apply_chat_template` so the format is always correct
    /// for whatever model is loaded.
    ///
    /// `tools` is an optional list of OpenAI-style function schemas; when
    /// provided the server passes them to the chat template so the model
    /// knows it can emit `<tool_call>` blocks.
    pub async fn generate(
        &self,
        messages: Vec<serde_json::Value>,
        tools: Option<Vec<serde_json::Value>>,
        max_tokens: usize,
        _temperature: f64,
    ) -> Result<String> {
        info!("Generating with MLX-LM: {} messages, max_tokens={}", messages.len(), max_tokens);

        // Ensure server is running
        if self.server_process.lock().await.is_none() {
            self.start_server().await?;
        }

        let mut server_guard = self.server_process.lock().await;
        let server = server_guard.as_mut()
            .ok_or_else(|| anyhow!("Server not running"))?;

        // Send request as JSON
        let request = GenerateRequest { messages, max_tokens, tools };
        let request_json = serde_json::to_string(&request)?;
        server.stdin.write_all(request_json.as_bytes()).await?;
        server.stdin.write_all(b"\n").await?;
        server.stdin.flush().await?;

        // Read response
        let mut response_line = String::new();
        server.stdout.read_line(&mut response_line).await?;

        let response: GenerateResponse = serde_json::from_str(&response_line)?;

        if let Some(error) = response.error {
            return Err(anyhow!("MLX generation failed: {}", error));
        }

        let result = response.response.ok_or_else(|| anyhow!("No response from server"))?;
        info!("✓ Generated {} chars", result.len());

        Ok(result)
    }

    /// Generate embeddings using native MLX
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // For now, return a placeholder embedding
        // TODO: Implement proper MLX embedding model once mlx-lm supports it
        info!("MLX native embedding requested for text: {} chars", text.len());
        
        // Return a mock 384-dimensional embedding (common size for sentence transformers)
        // This would be replaced with actual MLX embedding model inference
        let mock_embedding: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
        
        warn!("Using placeholder embeddings - MLX embedding models not yet implemented in mlx-lm");
        
        Ok(mock_embedding)
    }

    /// Check if model is loaded
    pub async fn is_loaded(&self) -> bool {
        // With Python MLX-LM, model is loaded on-demand
        true
    }

    /// Get model info
    pub async fn model_info(&self) -> Option<ModelInfo> {
        Some(ModelInfo {
            model_id: self.model_id.clone(),
            device: "Apple Silicon GPU (Metal via Python MLX)".to_string(),
        })
    }
}

pub struct ModelInfo {
    pub model_id: String,
    pub device: String,
}

/// Global MLX manager instance
static MLX_NATIVE_MANAGER: once_cell::sync::Lazy<tokio::sync::Mutex<Option<Arc<MLXNativeManager>>>> = 
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(None));

/// Initialize the global native MLX manager
pub async fn init_mlx_native_manager(model_id: String) -> Result<()> {
    info!("Initializing native MLX manager for model: {}", model_id);
    let manager = MLXNativeManager::new(model_id)?;
    manager.load_model().await?;
    *MLX_NATIVE_MANAGER.lock().await = Some(Arc::new(manager));
    Ok(())
}

/// Get the global native MLX manager
pub async fn get_mlx_native_manager() -> Result<Arc<MLXNativeManager>> {
    let guard = MLX_NATIVE_MANAGER.lock().await;
    match guard.as_ref() {
        Some(manager) => {
            // Return the existing manager by cloning the Arc
            Ok(Arc::clone(manager))
        }
        None => Err(anyhow!("Native MLX manager not initialized. Call init_mlx_native_manager() first."))
    }
}

/// Check if native MLX manager is available
pub async fn is_mlx_native_available() -> bool {
    MLX_NATIVE_MANAGER.lock().await.is_some()
}

/// Check if running on Apple Silicon
pub fn is_apple_silicon() -> bool {
    cfg!(target_arch = "aarch64")
}

/// Get recommended MLX models (all architectures supported via Python MLX-LM)
pub fn get_recommended_mlx_models() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("mlx-community/Qwen2.5-Coder-3B-Instruct-4bit", "3B params, 4-bit", "8-16GB RAM (Fast)"),
        ("mlx-community/Qwen2.5-Coder-7B-Instruct-4bit", "7B params, 4-bit", "16-32GB RAM (Balanced)"),
        ("mlx-community/gpt-oss-20b-MXFP4-Q4", "20B params, 4-bit", "32-64GB RAM (💎 Tools)"),
        ("mlx-community/Qwen3-Coder-30B-A3B-Instruct-4bit", "30B params, 4-bit", "64GB+ RAM (🔥 Context)"),
        ("mlx-community/Qwen3-30B-A3B-Thinking-2507-4bit", "30B params, 4-bit", "64GB+ RAM (🧠 Thinking)"),
        ("mlx-community/GLM-4.7-Flash-4bit", "30B-MoE, 4-bit", "64GB+ RAM (⚡ Agentic)"),
    ]
}

/// Get optimal MLX configuration for current Apple Silicon hardware
pub async fn get_optimal_mlx_config() -> (String, String) {
    // Detect available memory using sysctl
    let model = if let Ok(output) = tokio::process::Command::new("sysctl")
        .arg("hw.memsize")
        .output()
        .await
    {
        if let Ok(mem_str) = String::from_utf8(output.stdout) {
            if let Some(mem_bytes) = mem_str.split(':').nth(1).and_then(|s| s.trim().parse::<u64>().ok()) {
                let mem_gb = mem_bytes / (1024 * 1024 * 1024);
                match mem_gb {
                    0..=16 => "mlx-community/Qwen2.5-Coder-3B-Instruct-4bit",  // Fast
                    17..=32 => "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit", // Balanced
                    33..=63 => "mlx-community/gpt-oss-20b-MXFP4-Q4",           // Tools expert
                    _ => "mlx-community/Qwen3-Coder-30B-A3B-Instruct-4bit",    // Context (256K window)
                }
            } else {
                "mlx-community/Qwen2.5-Coder-3B-Instruct-4bit" // Safe default
            }
        } else {
            "mlx-community/Qwen2.5-Coder-3B-Instruct-4bit" // Safe default
        }
    } else {
        "mlx-community/Qwen2.5-Coder-3B-Instruct-4bit" // Safe default
    };
    
    (model.to_string(), "native-mlx".to_string())
}
