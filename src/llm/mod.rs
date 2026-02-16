use rig::providers::{openai, anthropic, gemini};
use rig::agent::AgentBuilder;
use rig::client::{ProviderClient, CompletionClient};
use crate::config::Config;
use anyhow::Result;

pub mod tools;
pub mod mlx_manager;

pub fn create_openai_agent_builder(config: &Config) -> Result<AgentBuilder<openai::responses_api::ResponsesCompletionModel>> {
    let client = openai::Client::from_env();
    Ok(client.agent(&config.model))
}

pub fn create_anthropic_agent_builder(config: &Config) -> Result<AgentBuilder<anthropic::completion::CompletionModel>> {
    let client = anthropic::Client::from_env();
    Ok(client.agent(&config.model))
}

pub fn create_gemini_agent_builder(config: &Config) -> Result<AgentBuilder<gemini::completion::CompletionModel>> {
    let client = gemini::Client::from_env();
    Ok(client.agent(&config.model))
}

pub async fn create_mlx_agent_builder(config: &Config) -> Result<AgentBuilder<openai::responses_api::ResponsesCompletionModel>> {
    // Ensure MLX server is running automatically
    let base_url = mlx_manager::ensure_mlx_server_running(&config.model).await?;
    
    // Set environment variables for the OpenAI client to use our local MLX server
    std::env::set_var("OPENAI_API_KEY", "mlx-local-key");
    std::env::set_var("OPENAI_BASE_URL", &base_url);
    
    let client = openai::Client::from_env();
    Ok(client.agent(&config.model))
}
