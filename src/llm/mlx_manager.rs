use anyhow::Result;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// MLX server process manager
#[derive(Debug)]
pub struct MLXManager {
    model_name: String,
    port: u16,
    process: Arc<Mutex<Option<Child>>>,
    base_url: String,
    script_path: Option<PathBuf>,
    startup_timeout: Duration,
    health_check_interval: Duration,
}

impl MLXManager {
    pub fn new(model_name: String, port: Option<u16>) -> Self {
        let port = port.unwrap_or(Self::find_available_port());
        let base_url = format!("http://127.0.0.1:{}/v1", port);
        
        Self {
            model_name,
            port,
            process: Arc::new(Mutex::new(None)),
            base_url,
            script_path: None,
            startup_timeout: Duration::from_secs(30),
            health_check_interval: Duration::from_secs(5),
        }
    }
    
    /// Get the base URL for the MLX server
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    
    /// Get the port the server is running on
    pub fn port(&self) -> u16 {
        self.port
    }
    
    /// Start the MLX server if not already running
    pub async fn ensure_server_running(&mut self) -> Result<()> {
        // Check if server is already healthy
        if self.is_server_healthy().await {
            debug!("MLX server already running and healthy on port {}", self.port);
            return Ok(());
        }
        
        // Stop any existing process
        self.stop_server().await?;
        
        // Start new server
        self.start_server().await?;
        
        // Wait for server to be ready
        self.wait_for_server_ready().await?;
        
        info!("MLX server started successfully on port {}", self.port);
        Ok(())
    }
    
    /// Start the MLX server process
    async fn start_server(&mut self) -> Result<()> {
        let script_path = self.find_mlx_script()?;
        self.script_path = Some(script_path.clone());
        
        info!("Starting MLX server with model: {}", self.model_name);
        
        let mut cmd = Command::new("python3");
        cmd.arg(&script_path)
            .arg("--model")
            .arg(&self.model_name)
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--host")
            .arg("127.0.0.1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        
        let child = cmd.spawn()?;
        
        {
            let mut process_guard = self.process.lock().await;
            *process_guard = Some(child);
        }
        
        debug!("MLX server process spawned on port {}", self.port);
        Ok(())
    }
    
    /// Stop the MLX server process
    pub async fn stop_server(&mut self) -> Result<()> {
        let mut process_guard = self.process.lock().await;
        
        if let Some(mut child) = process_guard.take() {
            info!("Stopping MLX server on port {}", self.port);
            
            // Try graceful shutdown first
            if let Err(e) = child.kill() {
                warn!("Failed to kill MLX server process: {}", e);
            }
            
            // Wait for process to exit
            match child.wait() {
                Ok(status) => {
                    debug!("MLX server exited with status: {}", status);
                }
                Err(e) => {
                    warn!("Error waiting for MLX server to exit: {}", e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Check if the server is healthy
    pub async fn is_server_healthy(&self) -> bool {
        let client = reqwest::Client::new();
        let health_url = format!("http://127.0.0.1:{}/health", self.port);
        
        match client.get(&health_url).timeout(Duration::from_secs(2)).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    debug!("MLX server health check passed");
                    true
                } else {
                    debug!("MLX server health check failed with status: {}", response.status());
                    false
                }
            }
            Err(e) => {
                debug!("MLX server health check failed: {}", e);
                false
            }
        }
    }
    
    /// Wait for the server to be ready
    async fn wait_for_server_ready(&self) -> Result<()> {
        let start_time = Instant::now();
        let mut attempts = 0;
        
        info!("Waiting for MLX server to be ready...");
        
        while start_time.elapsed() < self.startup_timeout {
            attempts += 1;
            
            if self.is_server_healthy().await {
                info!("MLX server is ready after {} attempts ({:.1}s)", 
                      attempts, start_time.elapsed().as_secs_f32());
                return Ok(());
            }
            
            if attempts % 5 == 0 {
                debug!("Still waiting for MLX server... (attempt {})", attempts);
            }
            
            sleep(Duration::from_millis(500)).await;
        }
        
        Err(anyhow::anyhow!(
            "MLX server failed to start within {} seconds",
            self.startup_timeout.as_secs()
        ))
    }
    
    /// Find the MLX server script
    fn find_mlx_script(&self) -> Result<PathBuf> {
        // Try relative to current executable
        let exe_path = std::env::current_exe()?;
        let exe_dir = exe_path.parent().ok_or_else(|| anyhow::anyhow!("No parent directory"))?;
        
        // Check ../scripts/mlx_server.py (development)
        let dev_script = exe_dir.parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("scripts").join("mlx_server.py"));
        
        if let Some(script) = dev_script {
            if script.exists() {
                return Ok(script);
            }
        }
        
        // Check ./scripts/mlx_server.py (relative to exe)
        let rel_script = exe_dir.join("scripts").join("mlx_server.py");
        if rel_script.exists() {
            return Ok(rel_script);
        }
        
        // Check current working directory
        let cwd_script = std::env::current_dir()?.join("scripts").join("mlx_server.py");
        if cwd_script.exists() {
            return Ok(cwd_script);
        }
        
        Err(anyhow::anyhow!("MLX server script not found. Expected at scripts/mlx_server.py"))
    }
    
    /// Find an available port starting from 8000
    fn find_available_port() -> u16 {
        for port in 8000..8100 {
            if Self::is_port_available(port) {
                return port;
            }
        }
        8000 // Fallback
    }
    
    /// Check if a port is available
    fn is_port_available(port: u16) -> bool {
        match std::net::TcpListener::bind(format!("127.0.0.1:{}", port)) {
            Ok(_) => true,
            Err(_) => false,
        }
    }
    
    /// Start background health monitoring with auto-recovery
    pub async fn start_health_monitoring(&self) {
        let manager = get_mlx_manager(&self.model_name);
        let port = self.port;
        let interval = self.health_check_interval;
        
        tokio::spawn(async move {
            let mut consecutive_failures = 0;
            const MAX_FAILURES: u32 = 3;
            
            loop {
                sleep(interval).await;
                
                let client = reqwest::Client::new();
                let health_url = format!("http://127.0.0.1:{}/health", port);
                
                match client.get(&health_url).timeout(Duration::from_secs(2)).send().await {
                    Ok(response) if response.status().is_success() => {
                        if consecutive_failures > 0 {
                            info!("MLX server recovered on port {}", port);
                            consecutive_failures = 0;
                        }
                    }
                    _ => {
                        consecutive_failures += 1;
                        
                        if consecutive_failures == 1 {
                            warn!("MLX server health check failed on port {}", port);
                        } else if consecutive_failures >= MAX_FAILURES {
                            error!("MLX server appears to be down after {} failed health checks, attempting restart", consecutive_failures);
                            
                            // Attempt to restart the server
                            let manager_clone = manager.clone();
                            tokio::spawn(async move {
                                let mut manager_guard = manager_clone.lock().await;
                                match manager_guard.ensure_server_running().await {
                                    Ok(_) => {
                                        info!("Successfully restarted MLX server on port {}", port);
                                    }
                                    Err(e) => {
                                        error!("Failed to restart MLX server: {}", e);
                                    }
                                }
                            });
                            
                            consecutive_failures = 0; // Reset to avoid immediate retry
                        }
                    }
                }
            }
        });
    }
    
    /// Get server information
    pub async fn get_info(&self) -> MLXServerInfo {
        let is_running = {
            let process_guard = self.process.lock().await;
            process_guard.is_some()
        };
        
        MLXServerInfo {
            model_name: self.model_name.clone(),
            port: self.port,
            base_url: self.base_url.clone(),
            is_running,
            script_path: self.script_path.clone(),
        }
    }
}

impl Drop for MLXManager {
    fn drop(&mut self) {
        // Clean shutdown when manager is dropped
        let _ = futures::executor::block_on(self.stop_server());
    }
}

/// Information about the MLX server
#[derive(Debug, Clone)]
pub struct MLXServerInfo {
    pub model_name: String,
    pub port: u16,
    pub base_url: String,
    pub is_running: bool,
    pub script_path: Option<PathBuf>,
}

/// Global MLX manager instance
static MLX_MANAGER: once_cell::sync::OnceCell<Arc<Mutex<MLXManager>>> = once_cell::sync::OnceCell::new();

/// Get or create the global MLX manager
pub fn get_mlx_manager(model_name: &str) -> Arc<Mutex<MLXManager>> {
    MLX_MANAGER.get_or_init(|| {
        Arc::new(Mutex::new(MLXManager::new(model_name.to_string(), None)))
    }).clone()
}

/// Initialize MLX server for the given model
pub async fn ensure_mlx_server_running(model_name: &str) -> Result<String> {
    let manager = get_mlx_manager(model_name);
    let mut manager_guard = manager.lock().await;
    
    manager_guard.ensure_server_running().await?;
    let base_url = manager_guard.base_url().to_string();
    
    // Start health monitoring
    manager_guard.start_health_monitoring().await;
    
    Ok(base_url)
}

/// Stop the MLX server
pub async fn stop_mlx_server() -> Result<()> {
    if let Some(manager) = MLX_MANAGER.get() {
        let mut manager_guard = manager.lock().await;
        manager_guard.stop_server().await?;
    }
    Ok(())
}

/// Get MLX server information
pub async fn get_mlx_server_info() -> Option<MLXServerInfo> {
    if let Some(manager) = MLX_MANAGER.get() {
        let manager_guard = manager.lock().await;
        Some(manager_guard.get_info().await)
    } else {
        None
    }
}