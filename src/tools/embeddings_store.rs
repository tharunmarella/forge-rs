//! Persistent embedding store with SQLite backend
//! 
//! Features:
//! - Persists embeddings to disk (survives restarts)
//! - Smart chunking using tree-sitter (function-level)
//! - Incremental indexing (only re-indexes changed files)

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::embeddings::{EmbeddingProvider, EmbeddingStore};
use super::treesitter;

/// Persistent embedding database
pub struct EmbeddingDb {
    conn: Arc<RwLock<Connection>>,
    provider: EmbeddingProvider,
    workdir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub id: i64,
    pub file_path: String,
    pub chunk_type: String, // "function", "class", "block"
    pub name: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub embedding: Vec<f32>,
    pub file_hash: String,
}

impl EmbeddingDb {
    /// Open or create the embedding database
    pub fn open(workdir: &Path, provider: EmbeddingProvider) -> Result<Self> {
        let db_path = workdir.join(".forge").join("embeddings.db");
        
        // Create .forge directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let conn = Connection::open(&db_path)?;
        
        // Initialize schema
        conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL,
                chunk_type TEXT NOT NULL,
                name TEXT,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                file_hash TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            
            CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(file_path);
            CREATE INDEX IF NOT EXISTS idx_chunks_hash ON chunks(file_hash);
            
            CREATE TABLE IF NOT EXISTS file_index (
                file_path TEXT PRIMARY KEY,
                file_hash TEXT NOT NULL,
                indexed_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
        "#)?;
        
        Ok(Self {
            conn: Arc::new(RwLock::new(conn)),
            provider,
            workdir: workdir.to_path_buf(),
        })
    }
    
    /// Check if a file needs re-indexing based on content hash
    pub async fn needs_reindex(&self, file_path: &str, content_hash: &str) -> bool {
        let conn = self.conn.read().await;
        let result: Result<String, _> = conn.query_row(
            "SELECT file_hash FROM file_index WHERE file_path = ?",
            params![file_path],
            |row| row.get(0),
        );
        
        match result {
            Ok(stored_hash) => stored_hash != content_hash,
            Err(_) => true, // Not indexed yet
        }
    }
    
    /// Index a single file using tree-sitter for smart chunking
    pub async fn index_file(&self, file_path: &Path, store: &EmbeddingStore) -> Result<usize> {
        let rel_path = file_path.strip_prefix(&self.workdir)
            .unwrap_or(file_path)
            .display()
            .to_string();
        
        let content = std::fs::read_to_string(file_path)?;
        let content_hash = compute_hash(&content);
        
        // Check if already indexed with same hash
        if !self.needs_reindex(&rel_path, &content_hash).await {
            return Ok(0);
        }
        
        // Get file extension for tree-sitter
        let ext = file_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        
        // Extract chunks using tree-sitter
        let chunks = extract_semantic_chunks(&content, ext, &rel_path);
        
        if chunks.is_empty() {
            tracing::debug!("No chunks extracted from {} (ext: {})", rel_path, ext);
            return Ok(0);
        }
        
        tracing::debug!("Extracted {} chunks from {}", chunks.len(), rel_path);
        
        // Generate embeddings
        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings = match store.embed_texts_public(&texts).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to generate embeddings for {}: {}", rel_path, e);
                return Err(e);
            }
        };
        
        // Delete old chunks for this file
        {
            let conn = self.conn.write().await;
            conn.execute("DELETE FROM chunks WHERE file_path = ?", params![rel_path])?;
        }
        
        // Insert new chunks
        let mut indexed = 0;
        {
            let conn = self.conn.write().await;
            for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
                let embedding_blob = embedding_to_blob(embedding);
                conn.execute(
                    "INSERT INTO chunks (file_path, chunk_type, name, start_line, end_line, content, embedding, file_hash)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        rel_path,
                        chunk.chunk_type,
                        chunk.name,
                        chunk.start_line,
                        chunk.end_line,
                        chunk.content,
                        embedding_blob,
                        content_hash
                    ],
                )?;
                indexed += 1;
            }
            
            // Update file index
            conn.execute(
                "INSERT OR REPLACE INTO file_index (file_path, file_hash) VALUES (?, ?)",
                params![rel_path, content_hash],
            )?;
        }
        
        Ok(indexed)
    }
    
    /// Search for similar chunks
    pub async fn search(&self, query_embedding: &[f32], limit: usize) -> Result<Vec<(f32, StoredChunk)>> {
        let conn = self.conn.read().await;
        let mut stmt = conn.prepare(
            "SELECT id, file_path, chunk_type, name, start_line, end_line, content, embedding, file_hash 
             FROM chunks WHERE embedding IS NOT NULL"
        )?;
        
        let chunks: Vec<StoredChunk> = stmt.query_map([], |row| {
            let embedding_blob: Vec<u8> = row.get(7)?;
            Ok(StoredChunk {
                id: row.get(0)?,
                file_path: row.get(1)?,
                chunk_type: row.get(2)?,
                name: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                content: row.get(6)?,
                embedding: blob_to_embedding(&embedding_blob),
                file_hash: row.get(8)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        
        // Compute similarities and sort
        let mut results: Vec<(f32, StoredChunk)> = chunks
            .into_iter()
            .map(|chunk| {
                let score = cosine_similarity(query_embedding, &chunk.embedding);
                (score, chunk)
            })
            .collect();
        
        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        
        Ok(results.into_iter().take(limit).collect())
    }
    
    /// Get total chunk count
    pub async fn chunk_count(&self) -> usize {
        let conn = self.conn.read().await;
        conn.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .unwrap_or(0)
    }
    
    /// Get indexed file count
    pub async fn file_count(&self) -> usize {
        let conn = self.conn.read().await;
        conn.query_row("SELECT COUNT(*) FROM file_index", [], |row| row.get(0))
            .unwrap_or(0)
    }
}

/// Semantic chunk extracted from code
#[derive(Debug)]
struct SemanticChunk {
    chunk_type: String,
    name: Option<String>,
    start_line: usize,
    end_line: usize,
    content: String,
}

/// Extract semantic chunks using tree-sitter
fn extract_semantic_chunks(content: &str, ext: &str, file_path: &str) -> Vec<SemanticChunk> {
    let mut chunks = Vec::new();
    
    // Try tree-sitter first
    if let Ok(symbols) = treesitter::parse_definitions(content, ext) {
        for symbol in symbols {
            // Get the actual content for this symbol
            let lines: Vec<&str> = content.lines().collect();
            let start = symbol.start_line.saturating_sub(1);
            let end = symbol.end_line.min(lines.len());
            
            if start < end {
                let chunk_content = lines[start..end].join("\n");
                
                // Skip very small chunks (less than 3 lines)
                if end - start >= 3 {
                    chunks.push(SemanticChunk {
                        chunk_type: symbol.kind.to_string(),
                        name: Some(symbol.name.clone()),
                        start_line: symbol.start_line,
                        end_line: symbol.end_line,
                        content: chunk_content,
                    });
                }
            }
        }
    }
    
    // If tree-sitter didn't find anything, fall back to block chunking
    if chunks.is_empty() {
        chunks = block_chunk(content, file_path);
    }
    
    chunks
}

/// Fallback block-based chunking (for unsupported languages)
fn block_chunk(content: &str, _file_path: &str) -> Vec<SemanticChunk> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    
    let chunk_size = 40;
    let overlap = 8;
    
    let mut i = 0;
    while i < lines.len() {
        let end = (i + chunk_size).min(lines.len());
        let chunk_content = lines[i..end].join("\n");
        
        if !chunk_content.trim().is_empty() && end - i >= 5 {
            chunks.push(SemanticChunk {
                chunk_type: "block".to_string(),
                name: None,
                start_line: i + 1,
                end_line: end,
                content: chunk_content,
            });
        }
        
        i += chunk_size - overlap;
    }
    
    chunks
}

/// Compute simple hash of content
fn compute_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Convert embedding to blob for storage
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter()
        .flat_map(|f| f.to_le_bytes())
        .collect()
}

/// Convert blob back to embedding
fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            f32::from_le_bytes(arr)
        })
        .collect()
}

/// Cosine similarity between two vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
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
