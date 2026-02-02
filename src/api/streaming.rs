use futures::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use serde_json::Value;
use std::pin::Pin;
use tokio::sync::mpsc;

/// Streaming event from LLM
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Text chunk
    Text(String),
    /// Thinking/reasoning text
    Thinking(String),
    /// Tool call detected (with optional Gemini 3 thought signature)
    ToolCall { name: String, arguments: Value, thought_signature: Option<String> },
    /// Stream completed
    Done,
    /// Error occurred
    Error(String),
}

/// Stream handle for receiving events
pub type StreamReceiver = mpsc::Receiver<StreamEvent>;

/// Create a channel for streaming
pub fn create_stream() -> (mpsc::Sender<StreamEvent>, StreamReceiver) {
    mpsc::channel(100)
}

/// Print streaming text to terminal with typewriter effect
pub async fn print_stream(mut rx: StreamReceiver) {
    use std::io::{stdout, Write};
    
    let mut in_thinking = false;
    
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Text(text) => {
                if in_thinking {
                    // End thinking block
                    println!("\x1b[0m"); // Reset color
                    in_thinking = false;
                }
                print!("{}", text);
                stdout().flush().ok();
            }
            StreamEvent::Thinking(text) => {
                if !in_thinking {
                    print!("\x1b[90m"); // Dim gray for thinking
                    in_thinking = true;
                }
                print!("{}", text);
                stdout().flush().ok();
            }
            StreamEvent::ToolCall { name, arguments, thought_signature: _ } => {
                if in_thinking {
                    println!("\x1b[0m");
                    in_thinking = false;
                }
                println!("\n\x1b[36m🔧 {}: {:?}\x1b[0m", name, arguments);
            }
            StreamEvent::Done => {
                if in_thinking {
                    println!("\x1b[0m");
                }
                println!();
                break;
            }
            StreamEvent::Error(e) => {
                println!("\n\x1b[31mError: {}\x1b[0m", e);
                break;
            }
        }
    }
}
