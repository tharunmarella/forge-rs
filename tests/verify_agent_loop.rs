use forge::api::{Agent, AgentPhase};
use forge::config::Config;
use std::path::PathBuf;

#[tokio::test]
async fn test_agent_reasoning_loop() {
    let mut config = Config::default();
    // Use a real API key from environment for the test if available
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        config.openai_api_key = Some(key);
    } else {
        println!("Skipping test: OPENAI_API_KEY not set");
        return;
    }

    let workdir = PathBuf::from(".");
    let mut agent = Agent::new(config, workdir).await.expect("Failed to create agent");

    println!("Starting multi-turn task...");
    agent.run_prompt("Analyze the project structure and suggest a 3-step plan to improve documentation.")
        .await
        .expect("Agent loop failed");

    // After run_prompt, we expect the phase to have moved beyond Explore
    let state = agent.tool_state.lock().unwrap();
    println!("Final Phase: {:?}", state.current_phase);
    println!("Plan Steps: {}", state.plan.len());

    assert!(state.plan.len() > 0, "Agent should have created a plan");
}
