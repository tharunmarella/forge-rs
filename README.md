# 🔨 Forge

> **Terminal-first AI coding agent powered by Rust**

[![Version](https://img.shields.io/badge/version-0.4.1-blue.svg)](https://github.com/tharunmarella/forge-rs)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

Forge is a blazingly fast, terminal-native AI coding agent that helps you write, refactor, and understand code through natural language. Built with Rust for performance and reliability, it features a three-phase approach to code generation: **Explore → Think → Execute**.

![Forge Demo](demo.png)

## ✨ Features

### 🎯 **Three-Phase Agentic Loop**
- **Explore**: Searches and analyzes your codebase to gather context
- **Think**: Creates a detailed plan based on findings
- **Execute**: Implements changes with full tool access

### 🚀 **Multiple LLM Providers**
- **Gemini** (Google AI) - Default, fast and capable
- **Claude** (Anthropic) - Excellent for complex reasoning
- **GPT-4** (OpenAI) - Powerful general-purpose model
- **Groq** - Ultra-fast inference
- **MLX** - Native Apple Silicon support with local models

### 🛠️ **Intelligent Tools**
- **Code Analysis**: grep, search, tree-sitter parsing, symbol resolution
- **File Operations**: Read, write, edit with smart replace
- **Git Integration**: Automatic checkpoints, undo/restore
- **Repository Mapping**: Auto-generated codebase structure
- **Self-Correction**: Lint detection and automatic fixes

### ⚡ **Built for Speed**
- Written in Rust for maximum performance
- Async architecture with tokio
- Efficient tool-calling loops with configurable `max_turns`
- Smart caching and context management

### 🎨 **Developer Experience**
- Interactive TUI with real-time tool execution visibility
- Session management with resume capability
- Plan mode (read-only) for safe exploration
- Auto-approve configurations for trusted operations
- Loop detection to prevent infinite tool calls

## 📦 Installation

### From Source

```bash
git clone https://github.com/tharunmarella/forge-rs
cd forge-rs
cargo install --path .
```

### Requirements
- Rust 1.75 or later
- OpenSSL (bundled)
- Git (for checkpoint features)

## 🚀 Quick Start

### 1. Setup

Run the interactive setup wizard to configure your AI provider:

```bash
forge setup
```

Or manually set your API key:

```bash
export GEMINI_API_KEY="your-key-here"
# or ANTHROPIC_API_KEY, OPENAI_API_KEY, GROQ_API_KEY
```

### 2. Run Your First Task

```bash
forge "add error handling to the parse function"
```

Forge will:
1. 🔍 **Explore** your codebase to find the parse function
2. 🧠 **Think** and create a plan for adding error handling
3. 🚀 **Execute** the changes with proper Result types and error propagation

### 3. Interactive Mode

Launch the TUI for multi-turn conversations:

```bash
forge
```

## 🎮 Usage Examples

### Code Generation
```bash
forge "create a REST API endpoint for user registration"
```

### Refactoring
```bash
forge "refactor the database module to use async/await"
```

### Bug Fixing
```bash
forge "fix the memory leak in the cache implementation"
```

### Code Review
```bash
forge --plan "review the authentication module for security issues"
```

### With Specific Model
```bash
forge --provider anthropic --model claude-sonnet-4 "explain this codebase"
```

### Resume Previous Session
```bash
forge --resume
```

## ⚙️ Configuration

Forge stores configuration in `~/.forge/config.json`:

```json
{
  "provider": "gemini",
  "model": "gemini-2.5-flash",
  "max_turns": 10,
  "auto_approve": {
    "read_operations": true,
    "write_operations": false,
    "commands": false,
    "yolo": false
  },
  "self_correction": true,
  "max_retries": 3
}
```

### Key Settings

| Setting | Description | Default |
|---------|-------------|---------|
| `provider` | AI provider (gemini, anthropic, openai, groq, mlx) | `gemini` |
| `model` | Specific model to use | `gemini-2.5-flash` |
| `max_turns` | Maximum tool-calling iterations per phase | `10` |
| `auto_approve.read_operations` | Auto-approve read-only tools | `true` |
| `auto_approve.write_operations` | Auto-approve file modifications | `false` |
| `auto_approve.commands` | Auto-approve shell commands | `false` |
| `self_correction` | Enable automatic lint fixes | `true` |

### Modify Config

```bash
# Via CLI
forge config provider=anthropic
forge config auto-approve.write_operations=true

# Or edit directly
vim ~/.forge/config.json
```

## 🏗️ Architecture

### Multi-Agent System

Forge uses specialized agents for different phases:

```
┌─────────────────────────────────────────┐
│  User Prompt: "fix the bug"            │
└─────────────────┬───────────────────────┘
                  │
    ┌─────────────▼──────────────┐
    │  Phase 1: EXPLORE          │
    │  Agent: planner            │
    │  Tools: grep, search, read │
    │  Max Turns: 10             │
    └─────────────┬──────────────┘
                  │
    ┌─────────────▼──────────────┐
    │  Phase 2: THINK            │
    │  Agent: planner            │
    │  Tools: create_plan        │
    │  Max Turns: 10             │
    └─────────────┬──────────────┘
                  │
    ┌─────────────▼──────────────┐
    │  Phase 3: EXECUTE          │
    │  Agent: tool_caller        │
    │  Tools: ALL (read, write)  │
    │  Max Turns: 10             │
    └─────────────┬──────────────┘
                  │
                  ▼
            Final Answer
```

### Tool Categories

**Planner Agent** (Explore & Think):
- `grep`, `search` - Find code patterns
- `read`, `ls` - Read files and directories
- `list_code_definitions` - Symbol analysis
- `create_plan`, `update_plan` - Task planning

**Tool Caller Agent** (Execute):
- All planner tools +
- `write`, `replace` - File modifications
- `run` - Execute commands
- `diagnostics` - Linter integration

**Reasoner Agent** (Optional):
- No tools - pure reasoning

## 🔧 Advanced Features

### Session Management

```bash
# List all sessions
forge sessions list

# Resume a specific session
forge --session <ID>

# Resume latest session
forge --resume

# Delete a session
forge sessions delete <ID>
```

### Git Checkpoints

Forge automatically creates git checkpoints before making changes:

```bash
# View checkpoints
forge checkpoints

# Undo last change
forge undo

# Restore to specific checkpoint
forge restore <commit-hash>

# View diff
forge diff <commit-hash>
```

### YOLO Mode (Auto-approve Everything)

⚠️ **Use with caution!**

```bash
forge --yolo "refactor entire codebase to use dependency injection"
```

### Plan Mode (Read-only)

Safe exploration without file modifications:

```bash
forge --plan "analyze the architecture and suggest improvements"
```

### Custom Timeouts

```bash
forge --timeout 300 "long-running analysis task"
```

## 🍎 Apple Silicon (MLX) Support

Forge has native support for running local models on Apple Silicon:

```bash
# Configure MLX
forge config provider=mlx
forge config model=mlx-community/Qwen2.5-Coder-7B-Instruct-4bit

# Run with local model
forge "add comments to this file"
```

**Supported MLX Models:**
- `Qwen2.5-Coder-7B-Instruct-4bit` (recommended)
- `Qwen2.5-Coder-14B-Instruct-8bit`
- `Qwen2.5-Coder-32B-Instruct-4bit`
- Custom mlx-community models

## 🤝 Contributing

Contributions are welcome! Here are some ways to help:

- 🐛 Report bugs and issues
- 💡 Suggest new features
- 📝 Improve documentation
- 🔧 Submit pull requests

### Development Setup

```bash
git clone https://github.com/tharunmarella/forge-rs
cd forge-rs
cargo build
cargo test
cargo run -- "test prompt"
```

## 📊 Performance

Forge is designed for speed:

- **Fast startup**: < 100ms to first prompt
- **Efficient tool execution**: Async operations throughout
- **Smart caching**: Repository maps, embeddings, symbols
- **Minimal memory**: Rust's zero-cost abstractions

## 🔒 Security

- **Read-only by default**: Write operations require approval (unless configured)
- **Command sandboxing**: Shell commands require explicit approval
- **Git checkpoints**: All changes are tracked and reversible
- **API key security**: Stored locally, never transmitted

## 📝 License

MIT License - see [LICENSE](LICENSE) for details

## 🙏 Acknowledgments

- Built with [Rig](https://github.com/0xPlaygrounds/rig-rs) - Rust LLM framework
- Powered by [Gemini](https://deepmind.google/technologies/gemini/), [Claude](https://anthropic.com/), [GPT-4](https://openai.com/), and [Groq](https://groq.com/)
- MLX support via [mlx-rs](https://github.com/oxideai/mlx-rs)
- TUI built with [Ratatui](https://ratatui.rs/)

## 📚 Documentation

- [Architecture Deep Dive](docs/ARCHITECTURE.md)
- [Tool Reference](docs/TOOLS.md)
- [MLX Integration Guide](docs/MLX.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)

## 💬 Support

- **Issues**: [GitHub Issues](https://github.com/tharunmarella/forge-rs/issues)
- **Discussions**: [GitHub Discussions](https://github.com/tharunmarella/forge-rs/discussions)

---

<p align="center">
  <b>Built with ❤️ and 🦀 by <a href="https://github.com/tharunmarella">Tharun Marella</a></b>
</p>

<p align="center">
  <sub>Forge: Where AI meets terminal excellence</sub>
</p>
