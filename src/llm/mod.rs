use rig::providers::{openai, anthropic, gemini};
use rig::agent::AgentBuilder;
use rig::client::{ProviderClient, CompletionClient};
use crate::config::Config;
use anyhow::Result;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

pub mod tools;
pub mod candle_manager;
pub mod candle_client;

// Keep the spawned MLX server process alive as long as forge is running
static MLX_SERVER_PROCESS: Lazy<Mutex<Option<tokio::process::Child>>> =
    Lazy::new(|| Mutex::new(None));

/// Ensure the mlx_lm.server is running at `base_url`.
/// If not running, spawns it and waits up to 120s for the model to load.
pub async fn ensure_mlx_server(base_url: &str, model: &str) -> Result<()> {
    let health = format!("{}/models", base_url.trim_end_matches('/'));

    let http = reqwest::Client::new();

    // Check if already up and healthy
    if http.get(&health)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
    {
        tracing::info!("MLX server already running at {}", base_url);
        return Ok(());
    }

    // Port in use but not responding — kill the stale process
    let port = base_url
        .rsplit(':').next()
        .and_then(|p| p.split('/').next())
        .unwrap_or("8000");
    if let Ok(out) = tokio::process::Command::new("lsof")
        .args(["-ti", &format!(":{}", port)])
        .output().await
    {
        let pids = String::from_utf8_lossy(&out.stdout);
        for pid in pids.split_whitespace() {
            tracing::info!("Killing stale process on port {}: {}", port, pid);
            let _ = tokio::process::Command::new("kill").args(["-9", pid]).status().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Find python binary with mlx_lm installed
    let python = ["python3", "python"]
        .iter()
        .find(|&&cmd| {
            std::process::Command::new(cmd)
                .args(["-c", "import mlx_lm"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
        .copied()
        .ok_or_else(|| anyhow::anyhow!(
            "mlx_lm not found. Install with:\n  pip install mlx-lm"
        ))?;

    println!("Starting MLX server for {}...", model);
    println!("(first run downloads the model — this may take several minutes)\n");

    let child = tokio::process::Command::new(python)
        .args(["-m", "mlx_lm.server", "--model", model])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true) // kill child when forge exits
        .spawn()?;

    *MLX_SERVER_PROCESS.lock().await = Some(child);

    // Poll until ready — allow up to 10 min for large model downloads
    for i in 0..600 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if http.get(&health)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            println!("\nMLX server ready.\n");
            return Ok(());
        }
    }

    anyhow::bail!(
        "MLX server did not become ready within 10 minutes.\n\
         Try starting it manually:\n  python -m mlx_lm.server --model {}",
        model
    )
}

/// Create an agent builder for local OpenAI-compatible servers (mlx_lm.server, Ollama, LM Studio).
/// Uses /v1/chat/completions — NOT the OpenAI Responses API.
pub fn create_local_agent_builder(base_url: &str, model: &str) -> Result<AgentBuilder<openai::completion::CompletionModel>> {
    let client = openai::Client::builder()
        .api_key("local")
        .base_url(base_url)
        .build()?;
    Ok(client.completion_model(model).completions_api().into_agent_builder())
}

pub fn create_openai_agent_builder(config: &Config) -> Result<AgentBuilder<openai::responses_api::ResponsesCompletionModel>> {
    let api_key = config.api_key().unwrap_or("local");
    
    let client: openai::Client = if let Some(url) = &config.base_url {
        openai::Client::builder()
            .api_key(api_key)
            .base_url(url)
            .build()?
    } else if let Some(key) = &config.openai_api_key {
        openai::Client::new(key)?
    } else {
        openai::Client::from_env()
    };
    
    Ok(client.agent(&config.model))
}

pub fn create_anthropic_agent_builder(config: &Config) -> Result<AgentBuilder<anthropic::completion::CompletionModel>> {
    let client = if let Some(key) = &config.anthropic_api_key {
        anthropic::Client::new(key)?
    } else {
        anthropic::Client::from_env()
    };
    Ok(client.agent(&config.model))
}

pub fn create_gemini_agent_builder(config: &Config) -> Result<AgentBuilder<gemini::completion::CompletionModel>> {
    let client = if let Some(key) = &config.gemini_api_key {
        gemini::Client::new(key)?
    } else {
        gemini::Client::from_env()
    };
    Ok(client.agent(&config.model))
}

pub async fn create_candle_agent_builder(config: &Config) -> Result<rig::agent::AgentBuilder<candle_client::CandleCompletionModel>> {
    // Initialize Candle manager if not already done
    if !candle_manager::is_candle_available().await {
        // Use custom server URL if provided, otherwise auto-detect
        if let Some(server_url) = &config.local_server_url {
            candle_manager::init_candle_manager_with_url(config.model.clone(), server_url.clone()).await?;
        } else {
            candle_manager::init_candle_manager(config.model.clone()).await?;
        }
    }
    
    Ok(candle_client::create_candle_agent_builder(&config.model))
}

