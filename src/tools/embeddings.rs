use anyhow::Result;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use walkdir::WalkDir;

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
    model: TextEmbedding,
    chunks: Arc<RwLock<Vec<CodeChunk>>>,
}

impl EmbeddingStore {
    /// Create new store and index the workspace
    pub fn new() -> Result<Self> {
        // Use a small, fast model
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                .with_show_download_progress(false)
        )?;

        Ok(Self {
            model,
            chunks: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Index a workspace directory
    pub async fn index_workspace(&self, workdir: &Path) -> Result<usize> {
        let mut all_chunks = Vec::new();

        for entry in WalkDir::new(workdir)
            .max_depth(8)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let rel_path = path.strip_prefix(workdir).unwrap_or(path);
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Skip non-code files
            if !is_code_file(file_name) {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                let chunks = chunk_code(&content, &rel_path.display().to_string());
                all_chunks.extend(chunks);
            }
        }

        // Generate embeddings in batches
        let batch_size = 32;
        let mut indexed = 0;

        for batch in all_chunks.chunks_mut(batch_size) {
            let texts: Vec<&str> = batch.iter().map(|c| c.content.as_str()).collect();
            
            if let Ok(embeddings) = self.model.embed(texts, None) {
                for (chunk, emb) in batch.iter_mut().zip(embeddings) {
                    chunk.embedding = emb;
                    indexed += 1;
                }
            }
        }

        // Store chunks
        let mut store = self.chunks.write().await;
        *store = all_chunks.into_iter().filter(|c| !c.embedding.is_empty()).collect();

        Ok(indexed)
    }

    /// Search for similar code
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(f32, CodeChunk)>> {
        // Embed query
        let query_embedding = self.model.embed(vec![query], None)?;
        let query_vec = &query_embedding[0];

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
        ".rs", ".py", ".js", ".ts", ".tsx", ".jsx",
        ".go", ".java", ".c", ".cpp", ".h", ".hpp",
        ".cs", ".rb", ".php", ".swift", ".kt",
        ".scala", ".sh", ".sql", ".proto",
    ];
    code_ext.iter().any(|ext| name.ends_with(ext))
}
