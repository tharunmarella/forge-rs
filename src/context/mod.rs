use anyhow::Result;
use std::path::Path;
use tokio::fs;
use walkdir::WalkDir;

/// Project context - files, structure, etc.
pub struct Context {
    pub files: Vec<FileInfo>,
    pub total_files: usize,
    pub total_lines: usize,
}

pub struct FileInfo {
    pub path: String,
    pub lines: usize,
    pub language: String,
}

impl Context {
    pub async fn new(workdir: &Path) -> Result<Self> {
        let mut files = Vec::new();
        let mut total_lines = 0;

        for entry in WalkDir::new(workdir)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let rel_path = path.strip_prefix(workdir).unwrap_or(path);
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Skip hidden and binary files
            if file_name.starts_with('.') || is_binary(file_name) || is_ignored(rel_path) {
                continue;
            }

            // Count lines asynchronously
            let lines = match fs::read_to_string(path).await {
                Ok(content) => content.lines().count(),
                Err(_) => 0,
            };

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let language = language_from_ext(ext);

            files.push(FileInfo {
                path: rel_path.display().to_string(),
                lines,
                language: language.to_string(),
            });

            total_lines += lines;
        }

        let total_files = files.len();

        Ok(Self {
            files,
            total_files,
            total_lines,
        })
    }

    /// Generate summary for system prompt
    pub fn file_summary(&self) -> String {
        if self.files.is_empty() {
            return "No files found".to_string();
        }

        // Group by language
        let mut by_lang: std::collections::HashMap<&str, (usize, usize)> = std::collections::HashMap::new();
        
        for f in &self.files {
            let entry = by_lang.entry(&f.language).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += f.lines;
        }

        let mut parts: Vec<String> = by_lang
            .iter()
            .map(|(lang, (count, lines))| format!("{} {} ({} lines)", count, lang, lines))
            .collect();
        
        parts.sort();
        
        format!("{} files, {} lines: {}", 
            self.total_files, 
            self.total_lines,
            parts.join(", ")
        )
    }
}

fn language_from_ext(ext: &str) -> &'static str {
    match ext {
        "rs" => "Rust",
        "py" => "Python",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "tsx" => "TypeScript/React",
        "jsx" => "JavaScript/React",
        "go" => "Go",
        "java" => "Java",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" => "C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "kt" | "kts" => "Kotlin",
        "scala" => "Scala",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" => "CSS",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "md" | "markdown" => "Markdown",
        "xml" => "XML",
        "proto" => "Protobuf",
        "dockerfile" => "Dockerfile",
        _ => "Other",
    }
}

fn is_binary(name: &str) -> bool {
    let exts = [
        ".png", ".jpg", ".jpeg", ".gif", ".ico", ".webp", ".svg",
        ".exe", ".dll", ".so", ".dylib", ".a", ".o",
        ".zip", ".tar", ".gz", ".rar", ".7z",
        ".pdf", ".doc", ".docx", ".xls", ".xlsx",
        ".mp3", ".mp4", ".avi", ".mov", ".wav",
        ".wasm", ".pyc", ".class",
        ".lock", ".db", ".sqlite",
    ];
    exts.iter().any(|e| name.ends_with(e))
}

fn is_ignored(path: &Path) -> bool {
    let ignored = [
        "node_modules", "target", "build", "dist", ".git",
        "__pycache__", ".pytest_cache", ".venv", "venv",
        "vendor", "coverage", ".next", ".nuxt",
        "reference-repos", ".cargo", "pkg/mod",
    ];
    
    path.components().any(|c| {
        c.as_os_str().to_str().map(|s| ignored.contains(&s)).unwrap_or(false)
    })
}
