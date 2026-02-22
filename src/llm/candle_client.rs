use anyhow::Result;
use async_trait::async_trait;
use rig::completion::{CompletionModel, CompletionRequest, Message, GetTokenUsage, AssistantContent};
use rig::OneOrMany;
use rig::client::CompletionClient;
use serde::{Deserialize, Serialize};

use super::candle_manager::get_candle_manager;

/// Custom Candle-based client for rig-core
#[derive(Clone)]
pub struct CandleClient {
    model_id: String,
}

impl CandleClient {
    pub fn new(model_id: String) -> Self {
        Self { model_id }
    }
}

impl CompletionClient for CandleClient {
    type CompletionModel = CandleCompletionModel;
    
    fn agent(&self, model: impl Into<String>) -> rig::agent::AgentBuilder<Self::CompletionModel> {
        let model_str = model.into();
        let completion_model = CandleCompletionModel {
            model_id: model_str,
        };
        rig::agent::AgentBuilder::new(completion_model)
    }
}

/// Candle-based completion model
#[derive(Clone)]
pub struct CandleCompletionModel {
    model_id: String,
}

#[async_trait]
impl CompletionModel for CandleCompletionModel {
    type Response = CandleCompletionResponse;
    type StreamingResponse = CandleCompletionResponse;
    type Client = CandleClient;

    fn make(_client: &Self::Client, model: impl Into<String>) -> Self {
        CandleCompletionModel {
            model_id: model.into(),
        }
    }

    fn completion(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = Result<rig::completion::CompletionResponse<Self::Response>, rig::completion::CompletionError>,
    > + Send {
        async move {
            let manager = get_candle_manager().await
                .map_err(|e| rig::completion::CompletionError::RequestError(e.to_string().into()))?;
            
            // Convert messages to a single prompt
            let prompt = messages_to_prompt(&request);
            
            // Generate completion
            let generated_text = manager.generate(
                &prompt,
                request.max_tokens.unwrap_or(512).try_into().unwrap_or(512),
                request.temperature.unwrap_or(0.7),
            ).await
            .map_err(|e| rig::completion::CompletionError::RequestError(e.to_string().into()))?;
            
            let response = CandleCompletionResponse {
                content: generated_text,
                model: self.model_id.clone(),
            };
            
            // Convert our response to the expected format using AssistantContent::text()
            let assistant_content = AssistantContent::text(&response.content);
            let choice = OneOrMany::one(assistant_content);
            
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
        Output = Result<rig::streaming::StreamingCompletionResponse<Self::StreamingResponse>, rig::completion::CompletionError>,
    > + Send {
        async move {
            // For simplicity, just return the completion as a single streaming chunk
            let completion_result = self.completion(_request).await?;
            
            // Extract the text content from the choice - iterate through the OneOrMany
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
            
            let response = CandleCompletionResponse {
                content: text_content.clone(),
                model: self.model_id.clone(),
            };
            
            // Create a simple streaming response with message chunks
            use rig::streaming::RawStreamingChoice;
            let stream = async_stream::try_stream! {
                // Yield the message content
                yield RawStreamingChoice::Message(text_content);
                // Yield the final response with usage info
                yield RawStreamingChoice::FinalResponse(response);
            };
            
            Ok(rig::streaming::StreamingCompletionResponse::stream(Box::pin(stream)))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleCompletionResponse {
    pub content: String,
    pub model: String,
}

// Implement GetTokenUsage for CandleCompletionResponse
impl rig::completion::GetTokenUsage for CandleCompletionResponse {
    fn token_usage(&self) -> Option<rig::completion::Usage> {
        // Provide rough token estimates since we don't have exact counts
        let input_tokens = self.content.len() / 4; // Rough estimate
        Some(rig::completion::Usage {
            input_tokens: input_tokens as u64,
            output_tokens: self.content.len() as u64 / 4,
            cached_input_tokens: 0,
            total_tokens: input_tokens as u64 + self.content.len() as u64 / 4,
        })
    }
}

/// Convert rig-core CompletionRequest to a prompt string
fn messages_to_prompt(request: &CompletionRequest) -> String {
    let mut prompt = String::new();
    
    // Add preamble if present
    if let Some(preamble) = &request.preamble {
        prompt.push_str(&format!("System: {}\n\n", preamble));
    }
    
    // Add chat history
    for message in request.chat_history.iter() {
        match message {
            Message::User { content } => {
                // For now, just use debug format to handle complex content types
                prompt.push_str(&format!("User: {:?}\n\n", content));
            }
            Message::Assistant { content, .. } => {
                // For now, just use debug format to handle complex content types
                prompt.push_str(&format!("Assistant: {:?}\n\n", content));
            }
        }
    }
    
    prompt.push_str("Assistant: ");
    prompt
}

/// Create a Candle agent builder
pub fn create_candle_agent_builder(model_id: &str) -> rig::agent::AgentBuilder<CandleCompletionModel> {
    let client = CandleClient::new(model_id.to_string());
    client.agent(model_id)
}