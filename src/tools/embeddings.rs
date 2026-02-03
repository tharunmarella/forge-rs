use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use walkdir::WalkDir;

/// Embedding provider type
#[derive(Clone, Debug)]
pub enum EmbeddingProvider {
    /// Gemini embeddings
    Gemini { api_key: String },
    /// OpenAI embeddings (works with OpenAI-compatible APIs)
    OpenAI { api_key: String, base_url: String },
    /// Ollama local embeddings (free, works offline)
    Ollama { base_url: String },
    /// Disabled (no embeddings - semantic search unavailable)
    None,
}

impl EmbeddingProvider {
    /// Create provider based on LLM provider config
    pub fn from_config(provider: &str, api_key: Option<&str>, base_url: Option<&str>) -> Self {
        match provider {
            "gemini" => EmbeddingProvider::Gemini {
                api_key: api_key.unwrap_or_default().to_string(),
            },
            "openai" => EmbeddingProvider::OpenAI {
                api_key: api_key.unwrap_or_default().to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
            },
            "groq" => EmbeddingProvider::OpenAI {
                api_key: api_key.unwrap_or_default().to_string(),
                base_url: "https://api.groq.com/openai/v1".to_string(),
            },
            "together" => EmbeddingProvider::OpenAI {
                api_key: api_key.unwrap_or_default().to_string(),
                base_url: "https://api.together.xyz/v1".to_string(),
            },
            "openrouter" => EmbeddingProvider::OpenAI {
                api_key: api_key.unwrap_or_default().to_string(),
                base_url: base_url.unwrap_or("https://openrouter.ai/api/v1").to_string(),
            },
            "ollama" => EmbeddingProvider::Ollama {
                base_url: base_url.unwrap_or("http://localhost:11434").to_string(),
            },
            // Anthropic and others: try Ollama first, fall back to None
            "anthropic" => EmbeddingProvider::Ollama {
                base_url: "http://localhost:11434".to_string(),
            },
            // Default: try local Ollama (run `ollama pull nomic-embed-text` for semantic search)
            _ => EmbeddingProvider::Ollama {
                base_url: "http://localhost:11434".to_string(),
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
    client: Client,
    chunks: Arc<RwLock<Vec<CodeChunk>>>,
}

impl EmbeddingStore {
    /// Create new store with the specified provider
    pub fn new(provider: EmbeddingProvider) -> Result<Self> {
        Ok(Self {
            provider,
            client: Client::new(),
            chunks: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Generate embeddings for texts (public for use by EmbeddingDb)
    pub async fn embed_texts_public(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.embed_texts(texts).await
    }
    
    /// Generate embeddings for texts
    async fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        match &self.provider {
            EmbeddingProvider::Gemini { api_key } => self.embed_gemini(texts, api_key).await,
            EmbeddingProvider::OpenAI { api_key, base_url } => {
                self.embed_openai(texts, api_key, base_url).await
            }
            EmbeddingProvider::Ollama { base_url } => {
                self.embed_ollama(texts, base_url).await
            }
            EmbeddingProvider::None => {
                // Return zero vectors (embeddings disabled)
                Ok(texts.iter().map(|_| vec![0.0; 768]).collect())
            }
        }
    }

    /// Gemini embeddings
    async fn embed_gemini(&self, texts: &[&str], api_key: &str) -> Result<Vec<Vec<f32>>> {
        #[derive(Serialize)]
        struct GeminiRequest<'a> {
            requests: Vec<GeminiEmbedRequest<'a>>,
        }

        #[derive(Serialize)]
        struct GeminiEmbedRequest<'a> {
            model: &'static str,
            content: GeminiContent<'a>,
        }

        #[derive(Serialize)]
        struct GeminiContent<'a> {
            parts: Vec<GeminiPart<'a>>,
        }

        #[derive(Serialize)]
        struct GeminiPart<'a> {
            text: &'a str,
        }

        #[derive(Deserialize)]
        struct GeminiResponse {
            embeddings: Vec<GeminiEmbedding>,
        }

        #[derive(Deserialize)]
        struct GeminiEmbedding {
            values: Vec<f32>,
        }

        // Gemini batch embed endpoint
        let requests: Vec<_> = texts
            .iter()
            .map(|text| GeminiEmbedRequest {
                model: "models/text-embedding-004",
                content: GeminiContent {
                    parts: vec![GeminiPart { text }],
                },
            })
            .collect();

        let resp = self
            .client
            .post(format!(
                "https://generativelanguage.googleapis.com/v1beta/models/text-embedding-004:batchEmbedContents?key={}",
                api_key
            ))
            .header("Content-Type", "application/json")
            .json(&GeminiRequest { requests })
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            return Err(anyhow::anyhow!("Gemini embeddings error: {}", err));
        }

        let response: GeminiResponse = resp.json().await?;
        Ok(response.embeddings.into_iter().map(|e| e.values).collect())
    }

    /// OpenAI-compatible embeddings
    async fn embed_openai(&self, texts: &[&str], api_key: &str, base_url: &str) -> Result<Vec<Vec<f32>>> {
        #[derive(Serialize)]
        struct OpenAIRequest<'a> {
            input: &'a [&'a str],
            model: &'static str,
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            data: Vec<OpenAIEmbedding>,
        }

        #[derive(Deserialize)]
        struct OpenAIEmbedding {
            embedding: Vec<f32>,
        }

        let resp = self
            .client
            .post(format!("{}/embeddings", base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&OpenAIRequest {
                input: texts,
                model: "text-embedding-3-small",
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            return Err(anyhow::anyhow!("OpenAI embeddings error: {}", err));
        }

        let response: OpenAIResponse = resp.json().await?;
        Ok(response.data.into_iter().map(|e| e.embedding).collect())
    }

    /// Ollama local embeddings
    async fn embed_ollama(&self, texts: &[&str], base_url: &str) -> Result<Vec<Vec<f32>>> {
        #[derive(Serialize)]
        struct OllamaRequest<'a> {
            model: &'static str,
            prompt: &'a str,
        }

        #[derive(Deserialize)]
        struct OllamaResponse {
            embedding: Vec<f32>,
        }

        let mut embeddings = Vec::new();
        
        // Ollama doesn't support batch embeddings, so we process one at a time
        for text in texts {
            let resp = self
                .client
                .post(format!("{}/api/embeddings", base_url))
                .header("Content-Type", "application/json")
                .json(&OllamaRequest {
                    model: "nomic-embed-text",  // Common embedding model for Ollama
                    prompt: text,
                })
                .send()
                .await;
            
            match resp {
                Ok(r) if r.status().is_success() => {
                    if let Ok(response) = r.json::<OllamaResponse>().await {
                        embeddings.push(response.embedding);
                    } else {
                        // Fallback: return zero vector if parsing fails
                        embeddings.push(vec![0.0; 768]);
                    }
                }
                _ => {
                    // Embedding model not available, use zero vector
                    embeddings.push(vec![0.0; 768]);
                }
            }
        }

        Ok(embeddings)
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
