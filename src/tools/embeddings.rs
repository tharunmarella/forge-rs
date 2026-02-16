use anyhow::Result;
use rig::client::EmbeddingsClient;
use rig::embeddings::EmbeddingsBuilder;
use rig::providers::{openai, gemini};
use serde_json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid;
use walkdir::WalkDir;

/// Embedding provider type using rig-core models and MLX
#[derive(Clone)]
pub enum EmbeddingProvider {
    /// OpenAI embeddings (works with OpenAI-compatible APIs)
    OpenAI(openai::EmbeddingModel),
    /// Gemini embeddings  
    Gemini(gemini::EmbeddingModel),
    /// Anthropic (uses OpenAI-compatible endpoint)
    Anthropic(openai::EmbeddingModel),
    /// MLX local embeddings (Apple Silicon optimized)
    MLX { model_name: String },
    /// Disabled (no embeddings - semantic search unavailable)
    None,
}

impl std::fmt::Debug for EmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingProvider::OpenAI(_) => write!(f, "OpenAI"),
            EmbeddingProvider::Gemini(_) => write!(f, "Gemini"),
            EmbeddingProvider::Anthropic(_) => write!(f, "Anthropic"),
            EmbeddingProvider::MLX { model_name } => write!(f, "MLX({})", model_name),
            EmbeddingProvider::None => write!(f, "None"),
        }
    }
}

impl EmbeddingProvider {
    /// Create provider based on LLM provider config
    pub fn from_config(provider: &str, api_key: Option<&str>, _base_url: Option<&str>) -> Self {
        let api_key = api_key.unwrap_or_default();
        
        match provider {
            "gemini" => {
                if api_key.is_empty() {
                    tracing::warn!("Gemini API key not provided, embeddings disabled");
                    return EmbeddingProvider::None;
                }
                match gemini::Client::new(api_key) {
                    Ok(client) => {
                        let model = client.embedding_model("text-embedding-004");
                        EmbeddingProvider::Gemini(model)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create Gemini client: {}", e);
                        EmbeddingProvider::None
                    }
                }
            },
            "openai" => {
                if api_key.is_empty() {
                    tracing::warn!("OpenAI API key not provided, embeddings disabled");
                    return EmbeddingProvider::None;
                }
                match openai::Client::new(api_key) {
                    Ok(client) => {
                        let model = client.embedding_model("text-embedding-3-small");
                        EmbeddingProvider::OpenAI(model)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create OpenAI client: {}", e);
                        EmbeddingProvider::None
                    }
                }
            },
            "groq" | "together" | "openrouter" => {
                if api_key.is_empty() {
                    tracing::warn!("{} API key not provided, embeddings disabled", provider);
                    return EmbeddingProvider::None;
                }
                // Note: rig-core doesn't support custom base URLs for OpenAI-compatible providers yet
                // For now, we disable embeddings for these providers
                tracing::warn!("Provider {} doesn't support embeddings via rig-core yet, disabled", provider);
                EmbeddingProvider::None
            },
            "anthropic" => {
                if api_key.is_empty() {
                    tracing::warn!("Anthropic API key not provided, embeddings disabled");
                    return EmbeddingProvider::None;
                }
                // Anthropic doesn't have native embeddings, use OpenAI-compatible fallback
                match openai::Client::new(api_key) {
                    Ok(client) => {
                        let model = client.embedding_model("text-embedding-3-small");
                        EmbeddingProvider::Anthropic(model)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create Anthropic fallback client: {}", e);
                        EmbeddingProvider::None
                    }
                }
            },
            "mlx" => {
                // MLX local embeddings for Apple Silicon
                let model_name = _base_url.unwrap_or("mlx-community/all-MiniLM-L6-v2-4bit").to_string();
                EmbeddingProvider::MLX { model_name }
            },
            // Default: try MLX first (for Apple Silicon), then disable
            _ => {
                #[cfg(target_os = "macos")]
                {
                    tracing::info!("Provider {} doesn't support embeddings, trying MLX fallback", provider);
                    EmbeddingProvider::MLX { 
                        model_name: "mlx-community/all-MiniLM-L6-v2-4bit".to_string() 
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    tracing::info!("Provider {} doesn't support embeddings, disabled", provider);
                    EmbeddingProvider::None
                }
            },
        }
    }
}

/// Code chunk with embedding
#[derive(Clone)]
pub struct CodeChunk {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub embedding: Vec<f32>,
}

/// Embedding store for semantic search
pub struct EmbeddingStore {
    provider: EmbeddingProvider,
    chunks: Arc<RwLock<Vec<CodeChunk>>>,
}

impl EmbeddingStore {
    /// Create new store with the specified provider
    pub fn new(provider: EmbeddingProvider) -> Result<Self> {
        Ok(Self {
            provider,
            chunks: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Generate embeddings for texts (public for use by EmbeddingDb)
    pub async fn embed_texts_public(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.embed_texts(texts).await
    }
    
    /// Generate embeddings for texts using rig-core
    async fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        match &self.provider {
            EmbeddingProvider::OpenAI(model) | EmbeddingProvider::Anthropic(model) => {
                self.embed_with_rig_model(texts, model).await
            }
            EmbeddingProvider::Gemini(model) => {
                self.embed_with_rig_gemini(texts, model).await
            }
            EmbeddingProvider::MLX { model_name } => {
                self.embed_with_mlx(texts, model_name).await
            }
            EmbeddingProvider::None => {
                // Return zero vectors (embeddings disabled)
                Ok(texts.iter().map(|_| vec![0.0; 768]).collect())
            }
        }
    }

    /// Generate embeddings using rig-core OpenAI model
    async fn embed_with_rig_model(&self, texts: &[&str], model: &openai::EmbeddingModel) -> Result<Vec<Vec<f32>>> {
        let mut builder = EmbeddingsBuilder::new(model.clone());
        
        for text in texts {
            builder = builder.document(*text)?;
        }
        
        let embeddings = builder.build().await?;
        
        let mut results = Vec::new();
        for (_, embedding) in embeddings {
            // OneOrMany is a struct, not an enum - iterate over all embeddings
            for emb in embedding {
                // Convert f64 to f32
                let f32_vec: Vec<f32> = emb.vec.into_iter().map(|x| x as f32).collect();
                results.push(f32_vec);
            }
        }
        
        Ok(results)
    }

    /// Generate embeddings using rig-core Gemini model  
    async fn embed_with_rig_gemini(&self, texts: &[&str], model: &gemini::EmbeddingModel) -> Result<Vec<Vec<f32>>> {
        let mut builder = EmbeddingsBuilder::new(model.clone());
        
        for text in texts {
            builder = builder.document(*text)?;
        }
        
        let embeddings = builder.build().await?;
        
        let mut results = Vec::new();
        for (_, embedding) in embeddings {
            // OneOrMany is a struct, not an enum - iterate over all embeddings
            for emb in embedding {
                // Convert f64 to f32
                let f32_vec: Vec<f32> = emb.vec.into_iter().map(|x| x as f32).collect();
                results.push(f32_vec);
            }
        }
        
        Ok(results)
    }

    /// Generate embeddings using MLX (Apple Silicon optimized)
    async fn embed_with_mlx(
        &self,
        texts: &[&str],
        model_name: &str,
    ) -> Result<Vec<Vec<f32>>> {
        // Find the MLX embeddings script
        let script_path = self.find_mlx_script()?;
        
        // Prepare input data
        let input_data = serde_json::json!({
            "texts": texts
        });
        
        // Create temporary files for communication
        let temp_dir = std::env::temp_dir();
        let input_file = temp_dir.join(format!("mlx_input_{}.json", uuid::Uuid::new_v4()));
        let output_file = temp_dir.join(format!("mlx_output_{}.json", uuid::Uuid::new_v4()));
        
        // Write input data
        tokio::fs::write(&input_file, serde_json::to_string(&input_data)?).await?;
        
        // Execute MLX embeddings script
        let output = Command::new("python3")
            .arg(&script_path)
            .arg("--model")
            .arg(model_name)
            .arg("--input")
            .arg(&input_file)
            .arg("--output")
            .arg(&output_file)
            .output()?;
        
        // Clean up input file
        let _ = tokio::fs::remove_file(&input_file).await;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("MLX embeddings failed: {}", stderr));
        }
        
        // Read output data
        let output_data = tokio::fs::read_to_string(&output_file).await?;
        let _ = tokio::fs::remove_file(&output_file).await;
        
        let result: serde_json::Value = serde_json::from_str(&output_data)?;
        
        // Check for errors
        if let Some(error) = result.get("error") {
            return Err(anyhow::anyhow!("MLX embeddings error: {}", error));
        }
        
        // Extract embeddings
        let embeddings = result.get("embeddings")
            .ok_or_else(|| anyhow::anyhow!("No embeddings in MLX response"))?
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Embeddings is not an array"))?;
        
        let mut result_embeddings = Vec::new();
        for embedding in embeddings {
            let embedding_array = embedding.as_array()
                .ok_or_else(|| anyhow::anyhow!("Embedding is not an array"))?;
            
            let embedding_vec: Result<Vec<f32>, _> = embedding_array
                .iter()
                .map(|v| v.as_f64().ok_or_else(|| anyhow::anyhow!("Invalid embedding value")).map(|f| f as f32))
                .collect();
            
            result_embeddings.push(embedding_vec?);
        }
        
        Ok(result_embeddings)
    }
    
    /// Find the MLX embeddings script path
    fn find_mlx_script(&self) -> Result<PathBuf> {
        // Try relative to current executable
        let exe_path = std::env::current_exe()?;
        let exe_dir = exe_path.parent().ok_or_else(|| anyhow::anyhow!("No parent directory"))?;
        
        // Check ../scripts/mlx_embeddings.py (development)
        let dev_script = exe_dir.parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("scripts").join("mlx_embeddings.py"));
        
        if let Some(script) = dev_script {
            if script.exists() {
                return Ok(script);
            }
        }
        
        // Check ./scripts/mlx_embeddings.py (relative to exe)
        let rel_script = exe_dir.join("scripts").join("mlx_embeddings.py");
        if rel_script.exists() {
            return Ok(rel_script);
        }
        
        // Check current working directory
        let cwd_script = std::env::current_dir()?.join("scripts").join("mlx_embeddings.py");
        if cwd_script.exists() {
            return Ok(cwd_script);
        }
        
        Err(anyhow::anyhow!("MLX embeddings script not found. Expected at scripts/mlx_embeddings.py"))
    }


    /// Index a workspace directory
    pub async fn index_workspace(&self, workdir: &Path) -> Result<usize> {
        let mut all_chunks = Vec::new();
        const MAX_FILES: usize = 200; // Limit for API calls
        let mut file_count = 0;

        for entry in WalkDir::new(workdir)
            .max_depth(8)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            if file_count >= MAX_FILES {
                break;
            }
            
            let path = entry.path();
            let path_str = path.to_string_lossy();
            
            // Skip common non-source directories
            if path_str.contains("node_modules")
                || path_str.contains("/target/")
                || path_str.contains("/.git/")
                || path_str.contains("/vendor/")
                || path_str.contains("/reference-repos/")
                || path_str.contains("/__pycache__/")
                || path_str.contains("/.venv/")
                || path_str.contains("/dist/")
                || path_str.contains("/build/")
            {
                continue;
            }
            
            let rel_path = path.strip_prefix(workdir).unwrap_or(path);
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Skip non-code files
            if !is_code_file(file_name) {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                let chunks = chunk_code(&content, &rel_path.display().to_string());
                all_chunks.extend(chunks);
                file_count += 1;
            }
        }

        // Generate embeddings in batches
        let batch_size = 16; // Smaller batch for API calls
        let mut indexed = 0;

        for batch in all_chunks.chunks_mut(batch_size) {
            let texts: Vec<&str> = batch.iter().map(|c| c.content.as_str()).collect();

            match self.embed_texts(&texts).await {
                Ok(embeddings) => {
                    for (chunk, emb) in batch.iter_mut().zip(embeddings) {
                        chunk.embedding = emb;
                        indexed += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!("Embedding batch failed: {}", e);
                }
            }
        }

        // Store chunks
        let mut store = self.chunks.write().await;
        *store = all_chunks
            .into_iter()
            .filter(|c| !c.embedding.is_empty())
            .collect();

        Ok(indexed)
    }

    /// Search for similar code
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(f32, CodeChunk)>> {
        // Embed query
        let query_embeddings = self.embed_texts(&[query]).await?;
        if query_embeddings.is_empty() {
            return Ok(Vec::new());
        }
        let query_vec = &query_embeddings[0];

        // Search chunks
        let chunks = self.chunks.read().await;

        let mut results: Vec<(f32, CodeChunk)> = chunks
            .iter()
            .map(|chunk| {
                let score = cosine_similarity(query_vec, &chunk.embedding);
                (score, chunk.clone())
            })
            .collect();

        // Sort by similarity (descending)
        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(results.into_iter().take(limit).collect())
    }
}

/// Chunk code into meaningful segments
fn chunk_code(content: &str, file_path: &str) -> Vec<CodeChunk> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    // Simple chunking: ~50 lines per chunk with overlap
    let chunk_size = 50;
    let overlap = 10;

    let mut i = 0;
    while i < lines.len() {
        let end = (i + chunk_size).min(lines.len());
        let chunk_content = lines[i..end].join("\n");

        if !chunk_content.trim().is_empty() {
            chunks.push(CodeChunk {
                file_path: file_path.to_string(),
                start_line: i + 1,
                end_line: end,
                content: chunk_content,
                embedding: Vec::new(),
            });
        }

        i += chunk_size - overlap;
    }

    chunks
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn is_code_file(name: &str) -> bool {
    let code_ext = [
        ".rs", ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
        ".cs", ".rb", ".php", ".swift", ".kt", ".scala", ".sh", ".sql", ".proto",
    ];
    code_ext.iter().any(|ext| name.ends_with(ext))
}
