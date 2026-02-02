//! Context7 Documentation Prefetcher
//!
//! Uses Context7's free public API to dynamically discover and fetch library documentation.
//! Runs in background without blocking the main request flow.
//!
//! Flow:
//! 1. User query arrives → fire-and-forget background search
//! 2. Search Context7 API for relevant libraries
//! 3. If high-confidence match found (>0.65 score), fetch docs
//! 4. Cache results for session
//! 5. On next turn, inject cached docs into system prompt

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const CONTEXT7_API_BASE: &str = "https://context7.com/api/v1";
const MIN_RELEVANCE_SCORE: f64 = 0.65;
const MAX_DOCS_PER_TURN: usize = 2;
const MAX_DOC_LENGTH: usize = 10000;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes

#[derive(Clone)]
struct CachedDoc {
    content: String,
    fetched_at: Instant,
    library_path: String,
    title: String,
}

#[derive(Clone, serde::Deserialize)]
struct SearchResult {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
    score: f64,
    #[serde(default)]
    verified: bool,
}

#[derive(serde::Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResult>,
}

/// Documentation prefetcher with session-scoped cache
#[derive(Clone)]
pub struct DocPrefetcher {
    cache: Arc<RwLock<HashMap<String, CachedDoc>>>,
    relevant_libs: Arc<RwLock<Vec<String>>>,
    client: reqwest::Client,
}

impl Default for DocPrefetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl DocPrefetcher {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            relevant_libs: Arc::new(RwLock::new(Vec::new())),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Start background prefetch for a query (non-blocking)
    pub fn prefetch_async(&self, query: String) {
        if query.len() < 5 {
            return;
        }

        let prefetcher = self.clone();
        tokio::spawn(async move {
            if let Err(e) = prefetcher.search_and_fetch(&query).await {
                // Silent failure - this is background enhancement
                eprintln!("[Context7] Search failed: {}", e);
            }
        });
    }

    /// Search Context7 and fetch docs for top matches
    async fn search_and_fetch(&self, query: &str) -> anyhow::Result<()> {
        let url = format!("{}/search?query={}", CONTEXT7_API_BASE, urlencoding::encode(query));
        
        let response = self.client
            .get(&url)
            .header("Accept", "application/json")
            .header("User-Agent", "forge-cli/1.0")
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(());
        }

        let data: SearchResponse = response.json().await?;
        
        // Filter by relevance
        let relevant: Vec<_> = data.results
            .into_iter()
            .filter(|r| r.score >= MIN_RELEVANCE_SCORE && self.is_relevant(query, &r.title))
            .take(MAX_DOCS_PER_TURN)
            .collect();

        if relevant.is_empty() {
            return Ok(());
        }

        // Update relevant libraries list
        {
            let mut libs = self.relevant_libs.write().unwrap();
            *libs = relevant.iter().map(|r| r.id.clone()).collect();
        }

        // Fetch docs for each
        for lib in relevant {
            // Skip if already cached and fresh
            {
                let cache = self.cache.read().unwrap();
                if let Some(cached) = cache.get(&lib.id) {
                    if cached.fetched_at.elapsed() < CACHE_TTL {
                        continue;
                    }
                }
            }

            self.fetch_library_docs(&lib.id, &lib.title).await?;
        }

        Ok(())
    }

    /// Check if library title is relevant to query
    fn is_relevant(&self, query: &str, title: &str) -> bool {
        let query_lower = query.to_lowercase();
        let title_lower = title.to_lowercase();

        // Exact match or title in query
        if query_lower == title_lower || query_lower.contains(&title_lower) {
            return true;
        }

        // Word overlap check
        let title_words: Vec<_> = title_lower.split(|c: char| c.is_whitespace() || c == '-' || c == '_')
            .filter(|w| w.len() > 2)
            .collect();
        let query_words: Vec<_> = query_lower.split(|c: char| c.is_whitespace() || c == '-' || c == '_')
            .filter(|w| w.len() > 2)
            .collect();

        title_words.iter().any(|tw| {
            query_words.iter().any(|qw| *qw == *tw || (tw.len() > 4 && qw.contains(tw)))
        })
    }

    /// Fetch documentation for a specific library
    async fn fetch_library_docs(&self, library_id: &str, title: &str) -> anyhow::Result<()> {
        let clean_id = library_id.trim_start_matches('/');
        let url = format!("{}/{}", CONTEXT7_API_BASE, clean_id);

        let response = self.client
            .get(&url)
            .header("Accept", "text/plain")
            .header("User-Agent", "forge-cli/1.0")
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(());
        }

        let mut content = response.text().await?;

        // Truncate if too long
        if content.len() > MAX_DOC_LENGTH {
            content.truncate(MAX_DOC_LENGTH);
            content.push_str("\n\n... [Documentation truncated]");
        }

        // Cache the doc
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(library_id.to_string(), CachedDoc {
                content,
                fetched_at: Instant::now(),
                library_path: library_id.to_string(),
                title: title.to_string(),
            });
        }

        Ok(())
    }

    /// Get cached documentation formatted for system prompt injection
    pub fn get_cached_docs_for_prompt(&self) -> String {
        let libs = self.relevant_libs.read().unwrap();
        let cache = self.cache.read().unwrap();

        let docs: Vec<String> = libs.iter()
            .filter_map(|lib_id| cache.get(lib_id))
            .map(|doc| {
                format!(
                    "<documentation source=\"{}\" path=\"{}\">\n{}\n</documentation>",
                    doc.title, doc.library_path, doc.content
                )
            })
            .collect();

        if docs.is_empty() {
            return String::new();
        }

        format!(
            r#"
<prefetched_documentation>
The following documentation was automatically fetched based on the user's query.
Use this information to provide accurate, up-to-date answers.

{}
</prefetched_documentation>
"#,
            docs.join("\n\n")
        )
    }

    /// Clear all cached docs
    pub fn clear_cache(&self) {
        self.cache.write().unwrap().clear();
        self.relevant_libs.write().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relevance_check() {
        let prefetcher = DocPrefetcher::new();
        
        assert!(prefetcher.is_relevant("React hooks tutorial", "React"));
        assert!(prefetcher.is_relevant("how to use typescript", "TypeScript"));
        assert!(!prefetcher.is_relevant("cooking recipes", "React"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_search_and_fetch() {
        let prefetcher = DocPrefetcher::new();
        
        // Directly call search_and_fetch (not async prefetch)
        let result = prefetcher.search_and_fetch("React hooks").await;
        assert!(result.is_ok(), "Search should succeed");
        
        // Give it a moment to populate cache
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        
        // Check if docs were cached
        let docs = prefetcher.get_cached_docs_for_prompt();
        println!("Cached docs length: {} chars", docs.len());
        println!("Preview: {}...", &docs[..docs.len().min(500)]);
        
        assert!(!docs.is_empty(), "Should have cached docs");
        assert!(docs.contains("<prefetched_documentation>"), "Should have doc wrapper");
    }
}
