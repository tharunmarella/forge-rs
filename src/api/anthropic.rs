use super::{AgentResponse, Message, Role};
use crate::config::Config;
use crate::tools::ToolCall;
use anyhow::Result;
use serde_json::Value;

const API_URL: &str = "https://api.anthropic.com/v1/messages";

pub async fn complete(
    config: &Config,
    system_prompt: &str,
    messages: &[Message],
    tools: &[Value],
) -> Result<AgentResponse> {
    let api_key = config.api_key().ok_or_else(|| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;

    // Build messages
    let mut api_messages = Vec::new();
    
    for msg in messages {
        match msg.role {
            Role::System => continue,
            Role::User => {
                api_messages.push(serde_json::json!({
                    "role": "user",
                    "content": msg.content
                }));
            }
            Role::Assistant => {
                if let Some(calls) = &msg.tool_calls {
                    let content: Vec<Value> = calls.iter().map(|c| {
                        serde_json::json!({
                            "type": "tool_use",
                            "id": format!("tool_{}", uuid::Uuid::new_v4()),
                            "name": c.name,
                            "input": c.arguments
                        })
                    }).collect();
                    
                    api_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content
                    }));
                } else {
                    api_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": msg.content
                    }));
                }
            }
            Role::Tool => {
                if let Some(results) = &msg.tool_results {
                    let content: Vec<Value> = results.iter().map(|(name, result)| {
                        serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": format!("tool_{}", name),
                            "content": result.output
                        })
                    }).collect();
                    
                    api_messages.push(serde_json::json!({
                        "role": "user",
                        "content": content
                    }));
                }
            }
        }
    }

    // Convert tools to Anthropic format
    let anthropic_tools: Vec<Value> = tools.iter().map(|t| {
        serde_json::json!({
            "name": t["name"],
            "description": t["description"],
            "input_schema": t["parameters"]
        })
    }).collect();

    let request = serde_json::json!({
        "model": config.model,
        "max_tokens": 8192,
        "system": system_prompt,
        "messages": api_messages,
        "tools": anthropic_tools
    });

    let client = reqwest::Client::new();
    let mut attempts = 0;
    let max_attempts = 3;

    loop {
        let response = client
            .post(API_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();

        if status.as_u16() == 429 && attempts < max_attempts {
            attempts += 1;
            let delay = std::time::Duration::from_secs(1 << (attempts - 1));
            tracing::warn!("Anthropic API rate limited (429). Retrying in {:?} (attempt {}/{})", delay, attempts, max_attempts);
            tokio::time::sleep(delay).await;
            continue;
        }

        let body: Value = response.json().await?;

        if !status.is_success() {
            let error = body["error"]["message"].as_str().unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("Anthropic API error: {}", error));
        }

        return parse_response(&body);
    }
}

fn parse_response(body: &Value) -> Result<AgentResponse> {
    let content = body["content"].as_array()
        .ok_or_else(|| anyhow::anyhow!("No content in response"))?;

    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in content {
        let block_type = block["type"].as_str().unwrap_or("");
        
        match block_type {
            "text" => {
                if let Some(text) = block["text"].as_str() {
                    text_parts.push(text.to_string());
                }
            }
            "tool_use" => {
                let name = block["name"].as_str().unwrap_or("").to_string();
                let args = block.get("input").cloned().unwrap_or(Value::Object(Default::default()));
                
                if name == "attempt_completion" {
                    let result = args["result"].as_str().unwrap_or("Done").to_string();
                    return Ok(AgentResponse::Completion(result));
                }
                
                if name == "ask_followup_question" {
                    let question = args["question"].as_str().unwrap_or("").to_string();
                    return Ok(AgentResponse::Question(question));
                }
                
                tool_calls.push(ToolCall { name, arguments: args, thought_signature: None });
            }
            _ => {}
        }
    }

    if !tool_calls.is_empty() {
        Ok(AgentResponse::ToolCalls { 
            text: text_parts.join("\n"), 
            calls: tool_calls 
        })
    } else if !text_parts.is_empty() {
        Ok(AgentResponse::Text(text_parts.join("\n")))
    } else {
        Ok(AgentResponse::Text(String::new()))
    }
}
