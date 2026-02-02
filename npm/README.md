# Forge CLI (Rust Edition)

Terminal-first AI coding agent with high-performance context assembly.

## Installation

```bash
npm install -g forge-cli-rs
```

## Usage

```bash
# Interactive TUI mode
forge

# Single prompt mode
forge "describe this project"

# With specific provider
GEMINI_API_KEY="your-key" forge
ANTHROPIC_API_KEY="your-key" forge
OPENAI_API_KEY="your-key" forge
```

## Features

- **Multi-provider support**: Gemini, Anthropic, OpenAI, Groq, Together AI, OpenRouter
- **Interactive TUI**: Beautiful terminal interface with ratatui
- **Git-based checkpointing**: Undo/restore workspace changes
- **Tool execution**: File operations, code search, command execution
- **Auto-approval settings**: Configure which tools run automatically

## Commands

- `/model` - Change AI model
- `/undo` - Undo last change
- `/clear` - Clear chat history
- `/help` - Show help
- `Tab` - Toggle between ask/agent mode
- `Ctrl+C` - Exit
