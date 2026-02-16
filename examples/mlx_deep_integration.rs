#!/usr/bin/env cargo run --example mlx_deep_integration --
//! Example demonstrating deep MLX integration with automatic server management
//! 
//! This example shows how the CLI automatically:
//! 1. Detects MLX availability
//! 2. Starts the MLX server in the background
//! 3. Manages the server lifecycle
//! 4. Provides seamless AI coding experience
//! 
//! No manual server startup required!

use forge::api::Agent;
use forge::config::Config;
use anyhow::Result;
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    
    println!("🚀 MLX Deep Integration Demo");
    println!("=============================");
    
    // Create MLX config - the system will automatically:
    // - Detect if MLX is available
    // - Start the MLX server if needed
    // - Configure the OpenAI client to use the local server
    let config = Config {
        provider: "mlx".to_string(),
        model: "mlx-community/Llama-3.2-3B-Instruct-4bit".to_string(),
        plan_mode: false,
        base_url: None, // Will be auto-configured
        ..Default::default()
    };
    
    println!("📋 Configuration:");
    println!("  Provider: {}", config.provider);
    println!("  Model: {}", config.model);
    println!("  Local Model: {}", config.is_local_model());
    
    // Create the agent - this will automatically start the MLX server
    println!("\n🔧 Initializing agent (this will start MLX server automatically)...");
    let mut agent = Agent::new(
        config,
        std::env::current_dir()?,
    ).await?;
    
    println!("✅ Agent initialized successfully!");
    
    // Test a simple coding prompt
    let prompt = "Write a simple Rust function that calculates the factorial of a number.";
    println!("\n💬 Sending prompt: {}", prompt);
    println!("⏳ Generating response...\n");
    
    match agent.run_prompt(prompt).await {
        Ok(_) => {
            println!("🤖 MLX Response generated successfully!");
            println!("   (Response would be displayed in interactive mode)");
        }
        Err(e) => {
            println!("❌ Error: {}", e);
        }
    }
    
    // The MLX server will be automatically stopped when the agent is dropped
    println!("\n🧹 Cleaning up...");
    agent.shutdown().await?;
    println!("✅ Shutdown complete!");
    
    Ok(())
}