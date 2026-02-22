use rig::providers::{openai, anthropic, gemini};
use rig::agent::AgentBuilder;
use rig::client::{ProviderClient, CompletionClient};
use crate::config::Config;
use anyhow::Result;

pub mod tools;
pub mod candle_manager;
pub mod candle_client;
pub mod mlx_native;
pub mod mlx_client;

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

pub async fn create_mlx_native_agent_builder(config: &Config) -> Result<rig::agent::AgentBuilder<mlx_client::MLXNativeCompletionModel>> {
    // Initialize native MLX manager if not already done
    if !mlx_native::is_mlx_native_available().await {
        mlx_native::init_mlx_native_manager(config.model.clone()).await?;
    }
    
    Ok(mlx_client::create_mlx_native_agent_builder(&config.model))
}
