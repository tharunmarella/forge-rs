use anyhow::Result;
use async_trait::async_trait;
use rig::completion::{
    AssistantContent, CompletionModel, CompletionRequest, GetTokenUsage, Message,
};
use rig::message::{ToolCall, ToolFunction};
use rig::OneOrMany;
use rig::client::CompletionClient;
use serde::{Deserialize, Serialize};

use super::mlx_native::get_mlx_native_manager;

/// Native MLX client for rig-core integration
#[derive(Clone)]
pub struct MLXNativeClient {
    model_id: String,
}

impl MLXNativeClient {
    pub fn new(model_id: String) -> Self {
        Self { model_id }
    }
}

impl CompletionClient for MLXNativeClient {
    type CompletionModel = MLXNativeCompletionModel;

    fn agent(&self, model: impl Into<String>) -> rig::agent::AgentBuilder<Self::CompletionModel> {
        let model_str = model.into();
        let completion_model = MLXNativeCompletionModel {
            model_id: model_str,
        };
        rig::agent::AgentBuilder::new(completion_model)
    }
}

/// Native MLX completion model for rig-core
#[derive(Clone)]
pub struct MLXNativeCompletionModel {
    model_id: String,
}

#[async_trait]
impl CompletionModel for MLXNativeCompletionModel {
    type Response = MLXNativeCompletionResponse;
    type StreamingResponse = MLXNativeCompletionResponse;
    type Client = MLXNativeClient;

    fn make(_client: &Self::Client, model: impl Into<String>) -> Self {
        MLXNativeCompletionModel {
            model_id: model.into(),
        }
    }

    fn completion(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = Result<
            rig::completion::CompletionResponse<Self::Response>,
            rig::completion::CompletionError,
        >,
    > + Send {
        async move {
            let manager = get_mlx_native_manager()
                .await
                .map_err(|e| rig::completion::CompletionError::RequestError(e.into()))?;

            // Build structured messages list (role + content)
            let messages = build_messages(&request);

            // Convert rig ToolDefinitions to OpenAI-style function schemas
            let tools: Option<Vec<serde_json::Value>> = if request.tools.is_empty() {
                None
            } else {
                Some(
                    request
                        .tools
                        .iter()
                        .map(|t| {
                            serde_json::json!({
                                "type": "function",
                                "function": {
                                    "name": t.name,
                                    "description": t.description,
                                    "parameters": t.parameters,
                                }
                            })
                        })
                        .collect(),
                )
            };

            let max_tokens = request.max_tokens.unwrap_or(1024).try_into().unwrap_or(1024);
            let temperature = request.temperature.unwrap_or(0.7);

            let raw_text = manager
                .generate(messages, tools, max_tokens, temperature)
                .await
                .map_err(|e| rig::completion::CompletionError::RequestError(e.into()))?;

            // Strip ChatML / special stop tokens that some models emit verbatim
            let generated_text = strip_stop_tokens(&raw_text);

            // Parse any <tool_call> blocks out of the generated text
            let (text_part, tool_calls) = parse_tool_calls(&generated_text);

            let response = MLXNativeCompletionResponse {
                content: generated_text.clone(),
                model: self.model_id.clone(),
            };

            // Build the choice: text (if any) followed by tool calls (if any)
            let mut items: Vec<AssistantContent> = Vec::new();
            if !text_part.is_empty() {
                items.push(AssistantContent::text(&text_part));
            }
            for (idx, (name, arguments)) in tool_calls.into_iter().enumerate() {
                items.push(AssistantContent::ToolCall(ToolCall::new(
                    format!("call_{idx}"),
                    ToolFunction::new(name, arguments),
                )));
            }

            let choice = match items.len() {
                0 => OneOrMany::one(AssistantContent::text("")),
                1 => OneOrMany::one(items.remove(0)),
                _ => OneOrMany::many(items)
                    .expect("items is non-empty; checked above"),
            };

            Ok(rig::completion::CompletionResponse {
                choice,
                usage: response.token_usage().unwrap_or_default(),
                raw_response: response,
            })
        }
    }

    fn stream(
        &self,
        _request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = Result<
            rig::streaming::StreamingCompletionResponse<Self::StreamingResponse>,
            rig::completion::CompletionError,
        >,
    > + Send {
        async move {
            let completion_result = self.completion(_request).await?;

            let mut text_content = String::new();
            for content in completion_result.choice {
                match content {
                    AssistantContent::Text(text) => {
                        text_content.push_str(&text.text);
                    }
                    AssistantContent::ToolCall(tc) => {
                        text_content.push_str(&format!("Tool call: {}", tc.function.name));
                    }
                    AssistantContent::Reasoning(r) => {
                        text_content.push_str(&format!("Reasoning: {:?}", r.reasoning));
                    }
                    AssistantContent::Image(_) => {
                        text_content.push_str("[Image]");
                    }
                }
            }

            let response = MLXNativeCompletionResponse {
                content: text_content.clone(),
                model: self.model_id.clone(),
            };

            use rig::streaming::RawStreamingChoice;
            let stream = async_stream::try_stream! {
                yield RawStreamingChoice::Message(text_content);
                yield RawStreamingChoice::FinalResponse(response);
            };

            Ok(rig::streaming::StreamingCompletionResponse::stream(Box::pin(stream)))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MLXNativeCompletionResponse {
    pub content: String,
    pub model: String,
}

impl GetTokenUsage for MLXNativeCompletionResponse {
    fn token_usage(&self) -> Option<rig::completion::Usage> {
        let rough = (self.content.len() / 4) as u64;
        Some(rig::completion::Usage {
            input_tokens: rough,
            output_tokens: rough,
            cached_input_tokens: 0,
            total_tokens: rough * 2,
        })
    }
}

// ── Message building ──────────────────────────────────────────────────────────

/// Convert a rig `CompletionRequest` into a flat list of `{role, content}`
/// JSON objects that `mlx_server.py` can pass to `apply_chat_template`.
fn build_messages(request: &CompletionRequest) -> Vec<serde_json::Value> {
    let mut messages: Vec<serde_json::Value> = Vec::new();

    if let Some(preamble) = &request.preamble {
        messages.push(serde_json::json!({
            "role": "system",
            "content": preamble
        }));
    }

    for message in request.chat_history.iter() {
        match message {
            Message::User { content } => {
                if let Ok(json_val) = serde_json::to_value(content) {
                    if let Some(text) = extract_text_from_json(&json_val) {
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": text
                        }));
                    }
                }
            }
            Message::Assistant { content, .. } => {
                let mut parts = Vec::new();
                for item in content.iter() {
                    match item {
                        AssistantContent::Text(t) => parts.push(t.text.clone()),
                        AssistantContent::ToolCall(tc) => {
                            // Re-serialise previous tool calls so the model can
                            // see what it already requested in this conversation.
                            let call_json = serde_json::json!({
                                "name": tc.function.name,
                                "arguments": tc.function.arguments,
                            });
                            parts.push(format!("<tool_call>\n{}\n</tool_call>", call_json));
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": parts.join("\n")
                    }));
                }
            }
        }
    }

    messages
}

/// Pull text out of the serialised `UserContent` JSON that rig produces.
fn extract_text_from_json(json: &serde_json::Value) -> Option<String> {
    // Single text object: {"text": "..."}
    if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
        return Some(text.to_string());
    }
    // Array of content parts
    if let Some(arr) = json.as_array() {
        let combined: Vec<&str> = arr
            .iter()
            .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
            .collect();
        if !combined.is_empty() {
            return Some(combined.join(" "));
        }
    }
    None
}

// ── Stop-token cleanup ───────────────────────────────────────────────────────

/// Remove special tokens that local models sometimes emit verbatim at the end
/// of their output (e.g. Qwen's `<|im_end|>`, Llama's `</s>`, etc.).
fn strip_stop_tokens(text: &str) -> String {
    const STOP_TOKENS: &[&str] = &[
        "<|im_end|>",
        "<|endoftext|>",
        "</s>",
        "<|eot_id|>",     // Llama-3
        "<|end|>",
        "<|im_start|>",   // should never appear in output, but just in case
    ];
    let mut result = text.trim().to_string();
    for tok in STOP_TOKENS {
        result = result.trim_end_matches(tok).trim().to_string();
    }
    result
}

// ── Tool-call parsing ─────────────────────────────────────────────────────────

/// Extract `<tool_call>…</tool_call>` blocks from the model output.
///
/// Returns:
/// - The remaining text (with the blocks removed)
/// - A list of `(name, arguments_json)` pairs
fn parse_tool_calls(text: &str) -> (String, Vec<(String, serde_json::Value)>) {
    let mut clean = String::new();
    let mut calls: Vec<(String, serde_json::Value)> = Vec::new();
    let mut remaining = text;

    loop {
        match remaining.find("<tool_call>") {
            None => {
                clean.push_str(remaining);
                break;
            }
            Some(start) => {
                // Accumulate text before the tag
                clean.push_str(&remaining[..start]);

                match remaining[start..].find("</tool_call>") {
                    None => {
                        // Unclosed tag — treat the rest as plain text
                        clean.push_str(&remaining[start..]);
                        break;
                    }
                    Some(rel_end) => {
                        let tag_len = "<tool_call>".len();
                        let json_str = remaining[start + tag_len..start + rel_end].trim();

                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                            if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                                let args = v
                                    .get("arguments")
                                    .cloned()
                                    .unwrap_or(serde_json::json!({}));
                                calls.push((name.to_string(), args));
                            }
                        }

                        let end_tag_len = "</tool_call>".len();
                        remaining = &remaining[start + rel_end + end_tag_len..];
                    }
                }
            }
        }
    }

    (clean.trim().to_string(), calls)
}

/// Create a native MLX agent builder
pub fn create_mlx_native_agent_builder(
    model_id: &str,
) -> rig::agent::AgentBuilder<MLXNativeCompletionModel> {
    let client = MLXNativeClient::new(model_id.to_string());
    client.agent(model_id)
}
