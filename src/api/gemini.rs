use super::streaming::{StreamEvent, create_stream, StreamReceiver};
use super::{AgentResponse, Message, Role};
use crate::config::Config;
use crate::tools::ToolCall;
use anyhow::Result;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;

const API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// Streaming completion - returns a receiver for events
pub async fn complete_streaming(
    config: &Config,
    system_prompt: &str,
    messages: &[Message],
    tools: &[Value],
) -> Result<StreamReceiver> {
    let api_key = config.api_key().ok_or_else(|| anyhow::anyhow!("GEMINI_API_KEY not set"))?;
    let (tx, rx) = create_stream();

    let url = format!(
        "{}/{}:streamGenerateContent?key={}&alt=sse",
        API_URL, config.model, api_key
    );

    let request_body = build_request(system_prompt, messages, tools, &config.model);
    
    // Spawn streaming task
    let client = reqwest::Client::new();
    tokio::spawn(async move {
        match stream_response(&client, &url, request_body, tx.clone()).await {
            Ok(_) => { tx.send(StreamEvent::Done).await.ok(); }
            Err(e) => { tx.send(StreamEvent::Error(e.to_string())).await.ok(); }
        }
    });

    Ok(rx)
}

async fn stream_response(
    client: &reqwest::Client,
    url: &str,
    body: Value,
    tx: mpsc::Sender<StreamEvent>,
) -> Result<()> {
    let response = client.post(url).json(&body).send().await?;
    
    if !response.status().is_success() {
        let error: Value = response.json().await?;
        let msg = error["error"]["message"].as_str().unwrap_or("Unknown error");
        return Err(anyhow::anyhow!("Gemini API error: {}", msg));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete SSE events
        while let Some(pos) = buffer.find("\n\n") {
            let event = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            if let Some(data) = event.strip_prefix("data: ") {
                if let Ok(json) = serde_json::from_str::<Value>(data) {
                    process_chunk(&json, &tx).await?;
                }
            }
        }
    }

    Ok(())
}

async fn process_chunk(json: &Value, tx: &mpsc::Sender<StreamEvent>) -> Result<()> {
    if let Some(candidates) = json["candidates"].as_array() {
        for candidate in candidates {
            if let Some(parts) = candidate["content"]["parts"].as_array() {
                for part in parts {
                    // Handle text
                    if let Some(text) = part["text"].as_str() {
                        tx.send(StreamEvent::Text(text.to_string())).await.ok();
                    }
                    
                    // Handle function calls
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc["name"].as_str().unwrap_or("").to_string();
                        let args = fc.get("args").cloned().unwrap_or(Value::Object(Default::default()));
                        // Extract thought signature for Gemini 3
                        let thought_signature = part.get("thoughtSignature")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        tx.send(StreamEvent::ToolCall { name, arguments: args, thought_signature }).await.ok();
                    }
                }
            }
        }
    }
    Ok(())
}

/// Non-streaming completion (for backward compatibility)
pub async fn complete(
    config: &Config,
    system_prompt: &str,
    messages: &[Message],
    tools: &[Value],
) -> Result<AgentResponse> {
    let api_key = config.api_key().ok_or_else(|| anyhow::anyhow!("GEMINI_API_KEY not set"))?;
    let url = format!("{}/{}:generateContent?key={}", API_URL, config.model, api_key);
    let request = build_request(system_prompt, messages, tools, &config.model);

    let client = reqwest::Client::new();
    let response = client.post(&url).json(&request).send().await?;

    let status = response.status();
    let body: Value = response.json().await?;

    if !status.is_success() {
        let error = body["error"]["message"].as_str().unwrap_or("Unknown error");
        return Err(anyhow::anyhow!("Gemini API error: {}", error));
    }

    parse_response(&body)
}

fn build_request(system_prompt: &str, messages: &[Message], tools: &[Value], model: &str) -> Value {
    let mut contents = Vec::new();
    
    for msg in messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "model",
            Role::Tool => "user",
            Role::System => continue,
        };

        if let Some(results) = &msg.tool_results {
            for (name, result) in results {
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": { "content": result.output }
                        }
                    }]
                }));
            }
        } else if let Some(calls) = &msg.tool_calls {
            let parts: Vec<Value> = calls.iter().map(|c| {
                let mut part = serde_json::json!({
                    "functionCall": { "name": c.name, "args": c.arguments }
                });
                // Include thought signature for Gemini 3 (required for function calling)
                if let Some(sig) = &c.thought_signature {
                    part["thoughtSignature"] = serde_json::json!(sig);
                }
                part
            }).collect();
            contents.push(serde_json::json!({ "role": "model", "parts": parts }));
        } else if !msg.content.is_empty() {
            contents.push(serde_json::json!({
                "role": role,
                "parts": [{ "text": msg.content }]
            }));
        }
    }

    let gemini_tools: Vec<Value> = tools.iter().map(|t| {
        serde_json::json!({
            "name": t["name"],
            "description": t["description"],
            "parameters": t["parameters"]
        })
    }).collect();

    let mut request = serde_json::json!({
        "system_instruction": { "parts": [{ "text": system_prompt }] },
        "contents": contents,
        "tools": [{ "function_declarations": gemini_tools }],
        "generationConfig": { 
            "temperature": 0.7, 
            "maxOutputTokens": 8192 
        }
    });
    
    // For Gemini 3 models, disable thinking to avoid thought_signature requirement
    // (thought signatures require complex state tracking across turns)
    if model.contains("gemini-3") {
        request["generationConfig"]["thinkingConfig"] = serde_json::json!({
            "thinkingBudget": 0
        });
    }
    
    request
}

fn parse_response(body: &Value) -> Result<AgentResponse> {
    // Check for API errors first
    if let Some(error) = body.get("error") {
        let msg = error["message"].as_str().unwrap_or("Unknown API error");
        return Err(anyhow::anyhow!("Gemini API error: {}", msg));
    }

    let candidates = body["candidates"].as_array()
        .ok_or_else(|| anyhow::anyhow!("No candidates in response: {}", body))?;

    if candidates.is_empty() {
        return Err(anyhow::anyhow!("Empty candidates"));
    }

    // Handle blocked responses
    if let Some(reason) = candidates[0]["finishReason"].as_str() {
        if reason == "SAFETY" || reason == "BLOCKED" {
            return Ok(AgentResponse::Text("Response was blocked by safety filters.".to_string()));
        }
    }

    // Parts might be empty for some responses
    let parts = match candidates[0]["content"]["parts"].as_array() {
        Some(p) if !p.is_empty() => p,
        _ => {
            // No content parts - return empty text
            return Ok(AgentResponse::Text(String::new()));
        }
    };

    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for part in parts {
        if let Some(text) = part["text"].as_str() {
            text_parts.push(text.to_string());
        }
        
        if let Some(fc) = part.get("functionCall") {
            let name = fc["name"].as_str().unwrap_or("").to_string();
            let args = fc.get("args").cloned().unwrap_or(Value::Object(Default::default()));
            
            // Extract thought signature for Gemini 3 (required for function calling)
            let thought_signature = part.get("thoughtSignature")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
            
            if name == "attempt_completion" {
                let result = args["result"].as_str().unwrap_or("Done").to_string();
                return Ok(AgentResponse::Completion(result));
            }
            
            if name == "ask_followup_question" {
                let question = args["question"].as_str().unwrap_or("").to_string();
                return Ok(AgentResponse::Question(question));
            }
            
            tool_calls.push(ToolCall { name, arguments: args, thought_signature });
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
