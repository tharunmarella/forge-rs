//! Repository Map - Aider-style codebase understanding
//!
//! Builds a semantic map of the codebase by:
//! 1. Extracting symbol definitions and references using tree-sitter
//! 2. Building a graph of file relationships
//! 3. Ranking files/symbols using PageRank
//! 4. Fitting the most important symbols into a token budget

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::algo::page_rank;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use walkdir::WalkDir;

/// A symbol tag extracted from source code
#[derive(Debug, Clone)]
pub struct Tag {
    pub rel_fname: String,
    pub abs_fname: PathBuf,
    pub line: usize,
    pub name: String,
    pub kind: TagKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TagKind {
    Definition,
    Reference,
}

/// The main RepoMap builder
pub struct RepoMap {
    root: PathBuf,
    max_tokens: usize,
    tags_cache: HashMap<PathBuf, (std::time::SystemTime, Vec<Tag>)>,
}

impl RepoMap {
    pub fn new(root: PathBuf, max_tokens: usize) -> Self {
        Self {
            root,
            max_tokens,
            tags_cache: HashMap::new(),
        }
    }

    /// Build the repo map for the given files
    pub fn build(&mut self, chat_files: &[PathBuf], other_files: &[PathBuf]) -> String {
        let start = Instant::now();
        
        // Collect all tags from files
        let mut all_tags: Vec<Tag> = Vec::new();
        let mut defines: HashMap<String, HashSet<String>> = HashMap::new(); // symbol -> files that define it
        let mut references: HashMap<String, Vec<String>> = HashMap::new(); // symbol -> files that reference it
        
        let all_files: HashSet<_> = chat_files.iter().chain(other_files.iter()).collect();
        
        for file in &all_files {
            if let Some(tags) = self.get_tags(file) {
                for tag in &tags {
                    match tag.kind {
                        TagKind::Definition => {
                            defines
                                .entry(tag.name.clone())
                                .or_default()
                                .insert(tag.rel_fname.clone());
                        }
                        TagKind::Reference => {
                            references
                                .entry(tag.name.clone())
                                .or_default()
                                .push(tag.rel_fname.clone());
                        }
                    }
                }
                all_tags.extend(tags);
            }
        }

        // Build graph: nodes = files, edges = references
        let mut graph: DiGraph<String, f64> = DiGraph::new();
        let mut node_indices: HashMap<String, NodeIndex> = HashMap::new();
        
        // Add all files as nodes
        for file in &all_files {
            let rel = self.get_rel_fname(file);
            let idx = graph.add_node(rel.clone());
            node_indices.insert(rel, idx);
        }

        // Add edges based on references
        let symbols: HashSet<_> = defines.keys().collect();
        for symbol in symbols {
            if let (Some(definers), Some(referencers)) = (defines.get(symbol), references.get(symbol)) {
                for referencer in referencers {
                    if let Some(&ref_idx) = node_indices.get(referencer) {
                        for definer in definers {
                            if let Some(&def_idx) = node_indices.get(definer) {
                                if ref_idx != def_idx {
                                    // Weight by symbol importance
                                    let weight = self.symbol_weight(symbol);
                                    graph.add_edge(ref_idx, def_idx, weight);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Run PageRank
        let ranks = if graph.node_count() > 0 {
            page_rank(&graph, 0.85, 20)
        } else {
            vec![]
        };

        // Boost chat files in ranking
        let chat_rel_fnames: HashSet<_> = chat_files.iter()
            .map(|f| self.get_rel_fname(f))
            .collect();

        // Create ranked list of (file, rank, tags)
        let mut file_ranks: Vec<(String, f64, Vec<&Tag>)> = Vec::new();
        for (node_idx, &rank) in ranks.iter().enumerate() {
            let node_idx = NodeIndex::new(node_idx);
            if let Some(fname) = graph.node_weight(node_idx) {
                // Skip files already in chat
                if chat_rel_fnames.contains(fname) {
                    continue;
                }
                
                let file_tags: Vec<_> = all_tags.iter()
                    .filter(|t| &t.rel_fname == fname && t.kind == TagKind::Definition)
                    .collect();
                
                file_ranks.push((fname.clone(), rank, file_tags));
            }
        }

        // Sort by rank descending
        file_ranks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Render to string, fitting within token budget
        let output = self.render_map(&file_ranks);
        
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 100 {
            eprintln!("[RepoMap] Built in {:?}", elapsed);
        }

        output
    }

    /// Get tags for a file, using cache if available
    fn get_tags(&mut self, path: &Path) -> Option<Vec<Tag>> {
        let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
        
        // Check cache
        if let Some((cached_mtime, cached_tags)) = self.tags_cache.get(path) {
            if *cached_mtime == mtime {
                return Some(cached_tags.clone());
            }
        }

        // Parse file
        let tags = self.extract_tags(path)?;
        self.tags_cache.insert(path.to_path_buf(), (mtime, tags.clone()));
        Some(tags)
    }

    /// Extract tags using tree-sitter
    fn extract_tags(&self, path: &Path) -> Option<Vec<Tag>> {
        let ext = path.extension()?.to_str()?;
        let content = std::fs::read_to_string(path).ok()?;
        let rel_fname = self.get_rel_fname(path);
        
        let mut tags = Vec::new();

        // Use regex-based extraction for common patterns
        // (Full tree-sitter would be better but this is faster to implement)
        match ext {
            "rs" => self.extract_rust_tags(&content, &rel_fname, path, &mut tags),
            "py" => self.extract_python_tags(&content, &rel_fname, path, &mut tags),
            "js" | "ts" | "jsx" | "tsx" => self.extract_js_tags(&content, &rel_fname, path, &mut tags),
            "go" => self.extract_go_tags(&content, &rel_fname, path, &mut tags),
            _ => return None,
        }

        Some(tags)
    }

    fn extract_rust_tags(&self, content: &str, rel_fname: &str, abs_path: &Path, tags: &mut Vec<Tag>) {
        let patterns = [
            (r"(?m)^[[:space:]]*pub\s+fn\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*fn\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*pub\s+struct\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*struct\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*pub\s+enum\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*enum\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*pub\s+trait\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*trait\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*impl\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*mod\s+(\w+)", TagKind::Definition),
        ];
        self.extract_with_patterns(content, rel_fname, abs_path, tags, &patterns);
    }

    fn extract_python_tags(&self, content: &str, rel_fname: &str, abs_path: &Path, tags: &mut Vec<Tag>) {
        let patterns = [
            (r"(?m)^[[:space:]]*def\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*class\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*async\s+def\s+(\w+)", TagKind::Definition),
        ];
        self.extract_with_patterns(content, rel_fname, abs_path, tags, &patterns);
    }

    fn extract_js_tags(&self, content: &str, rel_fname: &str, abs_path: &Path, tags: &mut Vec<Tag>) {
        let patterns = [
            (r"(?m)^[[:space:]]*function\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*const\s+(\w+)\s*=\s*(?:async\s*)?\(", TagKind::Definition),
            (r"(?m)^[[:space:]]*let\s+(\w+)\s*=\s*(?:async\s*)?\(", TagKind::Definition),
            (r"(?m)^[[:space:]]*class\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*export\s+(?:default\s+)?function\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*export\s+(?:default\s+)?class\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*(?:export\s+)?interface\s+(\w+)", TagKind::Definition),
            (r"(?m)^[[:space:]]*(?:export\s+)?type\s+(\w+)", TagKind::Definition),
        ];
        self.extract_with_patterns(content, rel_fname, abs_path, tags, &patterns);
    }

    fn extract_go_tags(&self, content: &str, rel_fname: &str, abs_path: &Path, tags: &mut Vec<Tag>) {
        let patterns = [
            (r"(?m)^func\s+(\w+)", TagKind::Definition),
            (r"(?m)^func\s+\([^)]+\)\s+(\w+)", TagKind::Definition),
            (r"(?m)^type\s+(\w+)\s+struct", TagKind::Definition),
            (r"(?m)^type\s+(\w+)\s+interface", TagKind::Definition),
        ];
        self.extract_with_patterns(content, rel_fname, abs_path, tags, &patterns);
    }

    fn extract_with_patterns(
        &self,
        content: &str,
        rel_fname: &str,
        abs_path: &Path,
        tags: &mut Vec<Tag>,
        patterns: &[(&str, TagKind)],
    ) {
        for (pattern, kind) in patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                for cap in re.captures_iter(content) {
                    if let Some(name_match) = cap.get(1) {
                        let name = name_match.as_str().to_string();
                        // Calculate line number
                        let line = content[..name_match.start()].matches('\n').count() + 1;
                        tags.push(Tag {
                            rel_fname: rel_fname.to_string(),
                            abs_fname: abs_path.to_path_buf(),
                            line,
                            name,
                            kind: *kind,
                        });
                    }
                }
            }
        }

        // Extract references (identifiers that look like they reference definitions)
        // This is a simplified version - full implementation would use tree-sitter
        if let Ok(re) = regex::Regex::new(r"\b([A-Z][a-zA-Z0-9_]*)\b") {
            let mut seen = HashSet::new();
            for cap in re.captures_iter(content) {
                if let Some(name_match) = cap.get(1) {
                    let name = name_match.as_str().to_string();
                    if seen.insert(name.clone()) {
                        tags.push(Tag {
                            rel_fname: rel_fname.to_string(),
                            abs_fname: abs_path.to_path_buf(),
                            line: 0,
                            name,
                            kind: TagKind::Reference,
                        });
                    }
                }
            }
        }
    }

    /// Calculate weight for a symbol based on naming conventions
    fn symbol_weight(&self, symbol: &str) -> f64 {
        let mut weight = 1.0;
        
        // Longer, more specific names are more valuable
        if symbol.len() >= 8 {
            weight *= 2.0;
        }
        
        // CamelCase or snake_case indicates intentional naming
        let is_camel = symbol.chars().any(|c| c.is_uppercase()) && symbol.chars().any(|c| c.is_lowercase());
        let is_snake = symbol.contains('_');
        if is_camel || is_snake {
            weight *= 2.0;
        }
        
        // Private symbols (underscore prefix) are less important
        if symbol.starts_with('_') {
            weight *= 0.5;
        }
        
        weight
    }

    fn get_rel_fname(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    }

    /// Render the map to a string, fitting within token budget
    fn render_map(&self, file_ranks: &[(String, f64, Vec<&Tag>)]) -> String {
        let mut output = String::new();
        let mut estimated_tokens = 0;
        
        for (fname, _rank, tags) in file_ranks {
            if estimated_tokens >= self.max_tokens {
                break;
            }

            // Group tags by file
            if tags.is_empty() {
                // Just list the file
                let line = format!("{}\n", fname);
                estimated_tokens += line.len() / 4; // rough token estimate
                output.push_str(&line);
            } else {
                // List file with symbols
                let header = format!("\n{}:\n", fname);
                estimated_tokens += header.len() / 4;
                output.push_str(&header);

                // Sort tags by line number
                let mut sorted_tags: Vec<_> = tags.iter().collect();
                sorted_tags.sort_by_key(|t| t.line);
                sorted_tags.dedup_by_key(|t| (&t.name, t.line));

                for tag in sorted_tags.iter().take(10) { // max 10 symbols per file
                    let line = format!("│ {} (line {})\n", tag.name, tag.line);
                    estimated_tokens += line.len() / 4;
                    output.push_str(&line);
                    
                    if estimated_tokens >= self.max_tokens {
                        break;
                    }
                }
            }
        }

        output
    }

    /// Scan directory and build map for all source files
    pub fn build_from_directory(&mut self) -> String {
        let start = Instant::now();
        let mut source_files: Vec<PathBuf> = Vec::new();
        
        let extensions = ["rs", "py", "js", "ts", "jsx", "tsx", "go", "java", "cpp", "c", "h"];
        const MAX_FILES: usize = 500; // Limit to prevent slowdown on huge repos
        
        for entry in WalkDir::new(&self.root)
            .max_depth(10) // Limit directory depth
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            if source_files.len() >= MAX_FILES {
                break;
            }
            
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    // Skip common non-source directories
                    let path_str = path.to_string_lossy();
                    if !path_str.contains("node_modules") 
                        && !path_str.contains("/target/")
                        && !path_str.contains("/.git/")
                        && !path_str.contains("/vendor/")
                        && !path_str.contains("/dist/")
                        && !path_str.contains("/build/")
                        && !path_str.contains("/__pycache__/")
                        && !path_str.contains("/.venv/")
                        && !path_str.contains("/venv/")
                        && !path_str.contains("/reference-repos/")
                        && !path_str.contains("/.cargo/")
                        && !path_str.contains("/pkg/mod/")
                        && !path_str.contains("/test_data/")
                        && !path_str.contains("/fixtures/")
                    {
                        source_files.push(path.to_path_buf());
                    }
                }
            }
        }
        
        let scan_time = start.elapsed();
        if scan_time.as_millis() > 100 {
            eprintln!("[RepoMap] Scanned {} files in {:?}", source_files.len(), scan_time);
        }

        self.build(&[], &source_files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_weight() {
        let rm = RepoMap::new(PathBuf::from("."), 1024);
        
        assert!(rm.symbol_weight("MyClassName") > rm.symbol_weight("x"));
        assert!(rm.symbol_weight("authenticate_user") > rm.symbol_weight("a"));
        assert!(rm.symbol_weight("_private") < rm.symbol_weight("public_func"));
    }
}
