use super::languages::{extension_to_language_id, LanguageServerConfig};
use super::types::*;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

/// LSP Client for communicating with a language server
#[derive(Clone)]
pub struct LspClient {
    config: LanguageServerConfig,
    workdir: PathBuf,
    inner: Arc<Mutex<Option<LspClientInner>>>,
    request_id: Arc<AtomicI64>,
}

struct LspClientInner {
    process: Child,
    responses: HashMap<i64, Value>,
}

impl LspClient {
    /// Create a new LSP client
    pub fn new(config: LanguageServerConfig, workdir: PathBuf) -> Self {
        Self {
            config,
            workdir,
            inner: Arc::new(Mutex::new(None)),
            request_id: Arc::new(AtomicI64::new(1)),
        }
    }
    
    /// Start the language server
    pub async fn start(&mut self) -> Result<()> {
        let mut cmd = Command::new(&self.config.command);
        cmd.args(&self.config.args)
            .current_dir(&self.workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        
        let process = cmd.spawn()
            .map_err(|e| anyhow!("Failed to start {}: {}", self.config.name, e))?;
        
        {
            let mut inner = self.inner.lock().unwrap();
            *inner = Some(LspClientInner {
                process,
                responses: HashMap::new(),
            });
        }
        
        // Send initialize request
        self.initialize().await?;
        
        Ok(())
    }
    
    /// Initialize the language server
    async fn initialize(&self) -> Result<()> {
        let root_uri = format!("file://{}", self.workdir.display());
        
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    definition: Some(DefinitionClientCapabilities {
                        link_support: Some(true),
                    }),
                    references: Some(ReferencesClientCapabilities {}),
                    hover: Some(HoverClientCapabilities {
                        content_format: Some(vec!["markdown".to_string(), "plaintext".to_string()]),
                    }),
                }),
            },
        };
        
        let _result = self.send_request("initialize", serde_json::to_value(params)?).await?;
        
        // Send initialized notification
        self.send_notification("initialized", json!({}))?;
        
        Ok(())
    }
    
    /// Send a JSON-RPC request and wait for response
    async fn send_request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };
        
        let request_json = serde_json::to_string(&request)?;
        let message = format!("Content-Length: {}\r\n\r\n{}", request_json.len(), request_json);
        
        // Send the request
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(ref mut client) = *inner {
                if let Some(ref mut stdin) = client.process.stdin {
                    stdin.write_all(message.as_bytes())?;
                    stdin.flush()?;
                }
            }
        }
        
        // Read response (simplified - in production would use async channels)
        let response = self.read_response(id)?;
        
        Ok(response)
    }
    
    /// Read a response for a specific request ID
    fn read_response(&self, expected_id: i64) -> Result<Value> {
        let mut inner = self.inner.lock().unwrap();
        let client = inner.as_mut().ok_or_else(|| anyhow!("Client not started"))?;
        
        // Check if we already have this response
        if let Some(resp) = client.responses.remove(&expected_id) {
            return Ok(resp);
        }
        
        // Read from stdout
        let stdout = client.process.stdout.take()
            .ok_or_else(|| anyhow!("No stdout"))?;
        let mut reader = BufReader::new(stdout);
        
        // Read headers
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line == "\n" {
                break;
            }
            headers.push_str(&line);
        }
        
        // Parse content length
        let content_length: usize = headers
            .lines()
            .find(|l| l.starts_with("Content-Length:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        
        // Read body
        let mut body = vec![0u8; content_length];
        std::io::Read::read_exact(&mut reader, &mut body)?;
        
        // Put stdout back
        client.process.stdout = Some(reader.into_inner());
        
        // Parse response
        let response: JsonRpcResponse = serde_json::from_slice(&body)?;
        
        if let Some(error) = response.error {
            return Err(anyhow!("LSP error: {}", error.message));
        }
        
        if let Some(id) = response.id {
            if id == expected_id {
                return Ok(response.result.unwrap_or(Value::Null));
            } else {
                // Store for later
                client.responses.insert(id, response.result.unwrap_or(Value::Null));
                // Try reading more
                drop(inner);
                return self.read_response(expected_id);
            }
        }
        
        Ok(Value::Null)
    }
    
    /// Send a notification (no response expected)
    fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        };
        
        let json = serde_json::to_string(&notification)?;
        let message = format!("Content-Length: {}\r\n\r\n{}", json.len(), json);
        
        let mut inner = self.inner.lock().unwrap();
        if let Some(ref mut client) = *inner {
            if let Some(ref mut stdin) = client.process.stdin {
                stdin.write_all(message.as_bytes())?;
                stdin.flush()?;
            }
        }
        
        Ok(())
    }
    
    /// Notify the server that a file was opened
    pub fn open_file(&self, file_path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(file_path)?;
        let ext = file_path.extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_default();
        
        self.send_notification("textDocument/didOpen", json!({
            "textDocument": {
                "uri": format!("file://{}", file_path.display()),
                "languageId": extension_to_language_id(&ext),
                "version": 1,
                "text": content
            }
        }))
    }
    
    /// Go to definition
    pub async fn go_to_definition(&self, file_path: &Path, line: u32, character: u32) -> Result<Vec<Location>> {
        // Ensure file is open
        self.open_file(file_path)?;
        
        let result = self.send_request("textDocument/definition", json!({
            "textDocument": {
                "uri": format!("file://{}", file_path.display())
            },
            "position": {
                "line": line,
                "character": character
            }
        })).await?;
        
        parse_location_response(result)
    }
    
    /// Find all references
    pub async fn find_references(&self, file_path: &Path, line: u32, character: u32) -> Result<Vec<Location>> {
        self.open_file(file_path)?;
        
        let result = self.send_request("textDocument/references", json!({
            "textDocument": {
                "uri": format!("file://{}", file_path.display())
            },
            "position": {
                "line": line,
                "character": character
            },
            "context": {
                "includeDeclaration": true
            }
        })).await?;
        
        parse_location_response(result)
    }
    
    /// Get hover information
    pub async fn hover(&self, file_path: &Path, line: u32, character: u32) -> Result<Option<String>> {
        self.open_file(file_path)?;
        
        let result = self.send_request("textDocument/hover", json!({
            "textDocument": {
                "uri": format!("file://{}", file_path.display())
            },
            "position": {
                "line": line,
                "character": character
            }
        })).await?;
        
        if result.is_null() {
            return Ok(None);
        }
        
        // Parse hover response
        if let Some(contents) = result.get("contents") {
            let text = format_hover_contents(contents);
            if !text.is_empty() {
                return Ok(Some(text));
            }
        }
        
        Ok(None)
    }

    /// Get document symbols
    pub async fn document_symbols(&self, file_path: &Path) -> Result<Value> {
        self.open_file(file_path)?;
        
        self.send_request("textDocument/documentSymbol", json!({
            "textDocument": {
                "uri": format!("file://{}", file_path.display())
            }
        })).await
    }
    
    /// Shutdown the language server
    pub async fn shutdown(&self) -> Result<()> {
        let _ = self.send_request("shutdown", json!(null)).await;
        self.send_notification("exit", json!(null))?;
        
        let mut inner = self.inner.lock().unwrap();
        if let Some(ref mut client) = *inner {
            let _ = client.process.kill();
        }
        *inner = None;
        
        Ok(())
    }
}

/// Parse a location response (can be single location, array, or null)
fn parse_location_response(value: Value) -> Result<Vec<Location>> {
    if value.is_null() {
        return Ok(vec![]);
    }
    
    // Try as array
    if let Ok(locations) = serde_json::from_value::<Vec<Location>>(value.clone()) {
        return Ok(locations);
    }
    
    // Try as single location
    if let Ok(location) = serde_json::from_value::<Location>(value) {
        return Ok(vec![location]);
    }
    
    Ok(vec![])
}

/// Format hover contents to a string
fn format_hover_contents(contents: &Value) -> String {
    match contents {
        Value::String(s) => s.clone(),
        Value::Object(obj) => {
            // MarkupContent
            if let Some(Value::String(value)) = obj.get("value") {
                return value.clone();
            }
            // MarkedString with language
            if let Some(Value::String(value)) = obj.get("value") {
                if let Some(Value::String(lang)) = obj.get("language") {
                    return format!("```{}\n{}\n```", lang, value);
                }
                return value.clone();
            }
            String::new()
        }
        Value::Array(arr) => {
            arr.iter()
                .map(format_hover_contents)
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        _ => String::new(),
    }
}
