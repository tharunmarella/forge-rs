use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Configuration for a language server
#[derive(Debug, Clone)]
pub struct LanguageServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub extensions: Vec<String>,
}

/// Get all supported language server configurations
pub fn supported_languages() -> Vec<LanguageServerConfig> {
    vec![
        LanguageServerConfig {
            name: "TypeScript".to_string(),
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            extensions: vec![".ts".to_string(), ".tsx".to_string(), ".js".to_string(), ".jsx".to_string()],
        },
        LanguageServerConfig {
            name: "Python".to_string(),
            command: "pylsp".to_string(),
            args: vec![],
            extensions: vec![".py".to_string()],
        },
        LanguageServerConfig {
            name: "Go".to_string(),
            command: "gopls".to_string(),
            args: vec!["serve".to_string()],
            extensions: vec![".go".to_string()],
        },
        LanguageServerConfig {
            name: "Rust".to_string(),
            command: "rust-analyzer".to_string(),
            args: vec![],
            extensions: vec![".rs".to_string()],
        },
        LanguageServerConfig {
            name: "C/C++".to_string(),
            command: "clangd".to_string(),
            args: vec![],
            extensions: vec![".c".to_string(), ".h".to_string(), ".cpp".to_string(), ".hpp".to_string()],
        },
    ]
}

/// Check if a language server is installed
pub fn is_server_installed(command: &str) -> bool {
    Command::new("which")
        .arg(command)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Detect which language servers are installed and return a map of extension -> config
pub fn detect_installed_servers() -> HashMap<String, LanguageServerConfig> {
    let mut installed = HashMap::new();
    
    for config in supported_languages() {
        if is_server_installed(&config.command) {
            for ext in &config.extensions {
                installed.insert(ext.clone(), config.clone());
            }
        }
    }
    
    installed
}

/// Get the appropriate language server config for a file
pub fn get_server_for_file(file_path: &Path, servers: &HashMap<String, LanguageServerConfig>) -> Option<LanguageServerConfig> {
    let ext = file_path.extension()?.to_str()?;
    let ext_with_dot = format!(".{}", ext);
    servers.get(&ext_with_dot).cloned()
}

/// Get the language ID for a file extension (used in LSP didOpen)
pub fn extension_to_language_id(ext: &str) -> &'static str {
    match ext {
        ".ts" => "typescript",
        ".tsx" => "typescriptreact",
        ".js" => "javascript",
        ".jsx" => "javascriptreact",
        ".py" => "python",
        ".go" => "go",
        ".rs" => "rust",
        ".c" | ".h" => "c",
        ".cpp" | ".hpp" | ".cc" | ".cxx" => "cpp",
        ".java" => "java",
        ".rb" => "ruby",
        ".php" => "php",
        _ => "plaintext",
    }
}

/// Get installation instructions for language servers
pub fn install_instructions() -> HashMap<&'static str, &'static str> {
    let mut instructions = HashMap::new();
    instructions.insert("TypeScript", "npm install -g typescript-language-server typescript");
    instructions.insert("Python", "pip install python-lsp-server");
    instructions.insert("Go", "go install golang.org/x/tools/gopls@latest");
    instructions.insert("Rust", "rustup component add rust-analyzer");
    instructions.insert("C/C++", "brew install llvm  # or apt install clangd");
    instructions
}
