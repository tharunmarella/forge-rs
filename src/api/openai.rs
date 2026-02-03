use super::{AgentResponse, Message, Role};
use crate::config::Config;
use crate::tools::ToolCall;
use anyhow::Result;
use serde_json::Value;

const DEFAULT_API_URL: &str = "https://api.openai.com/v1";

pub async fn complete(
    config: &Config,
    system_prompt: &str,
    messages: &[Message],
    tools: &[Value],
) -> Result<AgentResponse> {
    let api_key = config.api_key().ok_or_else(|| anyhow::anyhow!("API key not set for {}", config.provider))?;
    
    // Use custom base URL or default OpenAI
    let base_url = config.api_base_url().unwrap_or(DEFAULT_API_URL);
    let url = format!("{}/chat/completions", base_url);

    // Build messages
    let mut api_messages = vec![
        serde_json::json!({
            "role": "system",
            "content": system_prompt
        })
    ];
    
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
                    let tool_calls: Vec<Value> = calls.iter().enumerate().map(|(i, c)| {
                        serde_json::json!({
                            "id": format!("call_{}", i),
                            "type": "function",
                            "function": {
                                "name": c.name,
                                "arguments": serde_json::to_string(&c.arguments).unwrap_or_default()
                            }
                        })
                    }).collect();
                    
                    api_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": null,
                        "tool_calls": tool_calls
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
                    for (i, (name, result)) in results.iter().enumerate() {
                        api_messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": format!("call_{}", i),
                            "content": result.output
                        }));
                    }
                }
            }
        }
    }

    // Convert tools to OpenAI format
    let openai_tools: Vec<Value> = tools.iter().map(|t| {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": t["name"],
                "description": t["description"],
                "parameters": t["parameters"]
            }
        })
    }).collect();

    let request = serde_json::json!({
        "model": config.model,
        "messages": api_messages,
        "tools": openai_tools,
        "temperature": 0.7,
        "max_tokens": 8192
    });

    let client = reqwest::Client::new();
    let mut attempts = 0;
    let max_attempts = 3;

    loop {
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();

        if status.as_u16() == 429 && attempts < max_attempts {
            attempts += 1;
            let delay = std::time::Duration::from_secs(1 << (attempts - 1));
            tracing::warn!("OpenAI API rate limited (429). Retrying in {:?} (attempt {}/{})", delay, attempts, max_attempts);
            tokio::time::sleep(delay).await;
            continue;
        }

        let body: Value = response.json().await?;

        if !status.is_success() {
            let error = body["error"]["message"].as_str().unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("OpenAI API error: {}", error));
        }

        return parse_response(&body);
    }
}

fn parse_response(body: &Value) -> Result<AgentResponse> {
    let choice = body["choices"].as_array()
        .and_then(|c| c.first())
        .ok_or_else(|| anyhow::anyhow!("No choices in response"))?;

    let message = &choice["message"];
    let content = message["content"].as_str().unwrap_or("");
    
    // Check for tool calls
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        let mut calls = Vec::new();
        
        for tc in tool_calls {
            let function = &tc["function"];
            let name = function["name"].as_str().unwrap_or("").to_string();
            let args_str = function["arguments"].as_str().unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(Value::Object(Default::default()));
            
            if name == "attempt_completion" {
                let result = args["result"].as_str().unwrap_or("Done").to_string();
                return Ok(AgentResponse::Completion(result));
            }
            
            if name == "ask_followup_question" {
                let question = args["question"].as_str().unwrap_or("").to_string();
                return Ok(AgentResponse::Question(question));
            }
            
            calls.push(ToolCall { name, arguments: args, thought_signature: None });
        }
        
        if !calls.is_empty() {
            return Ok(AgentResponse::ToolCalls { 
                text: content.to_string(), 
                calls 
            });
        }
    }

    Ok(AgentResponse::Text(content.to_string()))
}
