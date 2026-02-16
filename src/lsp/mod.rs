pub mod types;
mod client;
mod languages;

pub use client::LspClient;
pub use languages::{LanguageServerConfig, detect_installed_servers};
pub use types::Location;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// LSP Manager that handles multiple language server clients
pub struct LspManager {
    clients: Arc<RwLock<HashMap<String, LspClient>>>,
    configs: HashMap<String, LanguageServerConfig>,
    workdir: PathBuf,
}

impl LspManager {
    /// Create a new LSP manager
    pub fn new(workdir: PathBuf) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            configs: detect_installed_servers(),
            workdir,
        }
    }
    
    /// Get or start a client for a specific file
    pub async fn get_client_for_file(&self, file_path: &Path) -> Option<LspClient> {
        let ext = file_path.extension()?.to_str()?;
        let ext_with_dot = format!(".{}", ext);
        
        // Check if we already have a client
        {
            let clients = self.clients.read().ok()?;
            if let Some(client) = clients.get(&ext_with_dot) {
                return Some(client.clone());
            }
        }
        
        // Get config for this extension
        let config = self.configs.get(&ext_with_dot)?;
        
        // Start a new client
        let mut client = LspClient::new(config.clone(), self.workdir.clone());
        if client.start().await.is_err() {
            return None;
        }
        
        // Store it
        {
            let mut clients = self.clients.write().ok()?;
            clients.insert(ext_with_dot, client.clone());
        }
        
        Some(client)
    }

    /// Get document symbols
    pub async fn get_document_symbols(&self, file_path: &Path) -> Option<serde_json::Value> {
        let client = self.get_client_for_file(file_path).await?;
        client.document_symbols(file_path).await.ok()
    }
    
    /// Go to definition for a symbol at a position
    pub async fn go_to_definition(&self, file_path: &Path, line: u32, character: u32) -> Option<Vec<Location>> {
        let client = self.get_client_for_file(file_path).await?;
        client.go_to_definition(file_path, line, character).await.ok()
    }
    
    /// Find all references to a symbol
    pub async fn find_references(&self, file_path: &Path, line: u32, character: u32) -> Option<Vec<Location>> {
        let client = self.get_client_for_file(file_path).await?;
        client.find_references(file_path, line, character).await.ok()
    }
    
    /// Get hover information
    pub async fn hover(&self, file_path: &Path, line: u32, character: u32) -> Option<String> {
        let client = self.get_client_for_file(file_path).await?;
        client.hover(file_path, line, character).await.ok().flatten()
    }
    
    /// Shutdown all clients
    pub async fn shutdown(&self) {
        if let Ok(mut clients) = self.clients.write() {
            for (_, client) in clients.drain() {
                let _ = client.shutdown().await;
            }
        }
    }
    
    /// List installed language servers
    pub fn installed_servers(&self) -> Vec<String> {
        let mut servers: Vec<String> = self.configs.values()
            .map(|c| c.name.clone())
            .collect();
        servers.sort();
        servers.dedup();
        servers
    }
}
