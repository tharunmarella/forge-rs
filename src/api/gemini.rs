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
    
    tracing::debug!("Gemini streaming URL: {}", url.replace(&api_key, "***"));
    tracing::debug!("Model: {}", config.model);

    let request_body = build_request(system_prompt, messages, tools, &config.model);
    
    // Spawn streaming task
    let client = reqwest::Client::new();
    let model_name = config.model.clone();
    tokio::spawn(async move {
        tracing::debug!("Starting Gemini stream for model: {}", model_name);
        match stream_response(&client, &url, request_body, tx.clone()).await {
            Ok(_) => { 
                tracing::debug!("Gemini stream completed successfully");
                tx.send(StreamEvent::Done).await.ok(); 
            }
            Err(e) => { 
                tracing::error!("Gemini stream error: {}", e);
                tx.send(StreamEvent::Error(e.to_string())).await.ok(); 
            }
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
    let mut attempts = 0;
    let max_attempts = 3;

    let response = loop {
        let response = client.post(url).json(&body).send().await?;
        let status = response.status();

        if status.as_u16() == 429 && attempts < max_attempts {
            attempts += 1;
            let delay = std::time::Duration::from_secs(1 << (attempts - 1));
            tracing::warn!("Gemini API rate limited (429). Retrying in {:?} (attempt {}/{})", delay, attempts, max_attempts);
            tokio::time::sleep(delay).await;
            continue;
        }

        if !status.is_success() {
            let error: Value = response.json().await?;
            let msg = error["error"]["message"].as_str().unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("Gemini API error: {}", msg));
        }

        break response;
    };

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    tracing::info!("Starting to read stream...");

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete lines only (lines ending with \n)
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();
            
            // Skip empty lines
            if line.trim().is_empty() {
                continue;
            }
            
            if let Some(data) = line.strip_prefix("data: ") {
                tracing::info!("Processing SSE line: {} chars", data.len());
                match serde_json::from_str::<Value>(data) {
                    Ok(json) => {
                        if let Some(error) = json.get("error") {
                            let msg = error["message"].as_str().unwrap_or("Unknown streaming error");
                            tx.send(StreamEvent::Error(msg.to_string())).await.ok();
                            return Err(anyhow::anyhow!("Gemini streaming error: {}", msg));
                        }
                        process_chunk(&json, &tx).await?;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse SSE: {} - {}", e, &data[..data.len().min(80)]);
                    }
                }
            }
        }
    }
    
    // Process any remaining data in buffer
    if !buffer.trim().is_empty() {
        if let Some(data) = buffer.strip_prefix("data: ") {
            if let Ok(json) = serde_json::from_str::<Value>(data.trim()) {
                process_chunk(&json, &tx).await?;
            }
        }
    }

    Ok(())
}

async fn process_chunk(json: &Value, tx: &mpsc::Sender<StreamEvent>) -> Result<()> {
    let num_candidates = json["candidates"].as_array().map(|a| a.len()).unwrap_or(0);
    tracing::info!("Processing chunk: {} candidates", num_candidates);
    
    if let Some(candidates) = json["candidates"].as_array() {
        for candidate in candidates {
            if let Some(parts) = candidate["content"]["parts"].as_array() {
                tracing::info!("Found {} parts in candidate", parts.len());
                for part in parts {
                    // Handle text
                    if let Some(text) = part["text"].as_str() {
                        tracing::info!("Part has text: {} chars, empty: {}", text.len(), text.is_empty());
                        if !text.is_empty() {
                            tx.send(StreamEvent::Text(text.to_string())).await.ok();
                        }
                    }
                    
                    // Handle function calls
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc["name"].as_str().unwrap_or("").to_string();
                        let args = fc.get("args").cloned().unwrap_or(Value::Object(Default::default()));
                        let thought_signature = part.get("thoughtSignature")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        tracing::info!("Sending tool call: {}", name);
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
    let mut attempts = 0;
    let max_attempts = 3;

    loop {
        let response = client.post(&url).json(&request).send().await?;
        let status = response.status();

        if status.as_u16() == 429 && attempts < max_attempts {
            attempts += 1;
            let delay = std::time::Duration::from_secs(1 << (attempts - 1));
            tracing::warn!("Gemini API rate limited (429). Retrying in {:?} (attempt {}/{})", delay, attempts, max_attempts);
            tokio::time::sleep(delay).await;
            continue;
        }

        let body: Value = response.json().await?;

        if !status.is_success() {
            let error = body["error"]["message"].as_str().unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("Gemini API error: {}", error));
        }

        return parse_response(&body);
    }
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
    
    // For Gemini 3 Pro models, enable thinking mode with a budget
    // Flash models may not require this
    if model.contains("gemini-3") && model.contains("pro") {
        request["generationConfig"]["thinkingConfig"] = serde_json::json!({
            "thinkingBudget": 8192
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
