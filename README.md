# 🔨 Forge

> **Terminal-first AI coding agent powered by Rust**

[![Version](https://img.shields.io/badge/version-0.4.1-blue.svg)](https://github.com/tharunmarella/forge-rs)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

Forge is a blazingly fast, terminal-native AI coding agent that helps you write, refactor, and understand code through natural language. Built with Rust for performance and reliability.

## ✨ Features

- **🎯 Three-Phase Agentic Loop**: Explore → Think → Execute
- **🚀 Multiple LLM Providers**: Gemini, Claude, GPT-4, Groq, MLX (local)
- **🛠️ Intelligent Tools**: Code analysis, file operations, git integration
- **⚡ Built for Speed**: Written in Rust with async architecture
- **🍎 Apple Silicon**: Native MLX support for local models
- **🎨 Session Management**: Resume conversations, undo changes with git checkpoints

## 📦 Installation

```bash
git clone https://github.com/tharunmarella/forge-rs
cd forge-rs
cargo install --path .
```

**Requirements:** Rust 1.75+, Git

## 🚀 Quick Start

### Setup

Configure your AI provider:

```bash
forge setup
```

### Start Using

```bash
forge
```

That's it! Forge will launch an interactive TUI where you can chat with the AI about your code.

### Command-Line Mode

Run one-off tasks directly:

```bash
forge "add error handling to the parse function"
forge "create a REST API endpoint for user registration"
forge "fix the memory leak in the cache implementation"
```

## ⚙️ Configuration

Config stored in `~/.forge/config.json`:

| Setting | Description | Default |
|---------|-------------|---------|
| `provider` | AI provider (gemini, anthropic, openai, groq, mlx) | `gemini` |
| `model` | Specific model to use | `gemini-2.5-flash` |
| `max_turns` | Max tool-calling iterations per phase | `10` |
| `auto_approve.read_operations` | Auto-approve read-only tools | `true` |
| `auto_approve.write_operations` | Auto-approve file modifications | `false` |

## 🏗️ How It Works

```
User Prompt: "fix the bug"
         ↓
┌────────────────────┐
│  Phase 1: EXPLORE  │  Search & analyze codebase
│  Max Turns: 10     │  Tools: grep, search, read
└────────┬───────────┘
         ↓
┌────────────────────┐
│  Phase 2: THINK    │  Create execution plan  
│  Max Turns: 10     │  Tools: create_plan
└────────┬───────────┘
         ↓
┌────────────────────┐
│  Phase 3: EXECUTE  │  Implement changes
│  Max Turns: 10     │  Tools: ALL (read, write, run)
└────────┬───────────┘
         ↓
    Final Answer
```

## 🔧 Advanced Features

### Session Management

```bash
forge --resume              # Resume latest session
forge sessions list         # List all sessions
```

### Git Checkpoints

```bash
forge checkpoints          # View all checkpoints
forge undo                 # Undo last change
```

### Plan Mode (Read-only)

```bash
forge --plan "analyze the architecture"
```

### YOLO Mode (Auto-approve everything)

```bash
forge --yolo "refactor entire codebase"  # ⚠️ Use with caution!
```

## 🍎 Apple Silicon Support

Run local models on Apple Silicon:

```bash
forge config provider=mlx
forge config model=mlx-community/Qwen2.5-Coder-7B-Instruct-4bit
forge "add comments to this file"
```

## 🤝 Contributing

Contributions welcome! Feel free to:
- Report bugs and issues
- Suggest new features
- Submit pull requests

```bash
git clone https://github.com/tharunmarella/forge-rs
cd forge-rs
cargo build
cargo test
```

## 📝 License

MIT License - see [LICENSE](LICENSE) for details

## 🙏 Acknowledgments

- Built with [Rig](https://github.com/0xPlaygrounds/rig-rs) - Rust LLM framework
- Powered by Gemini, Claude, GPT-4, and Groq
- MLX support via [mlx-rs](https://github.com/oxideai/mlx-rs)

---

<p align="center">
  <b>Built with ❤️ and 🦀 by <a href="https://github.com/tharunmarella">Tharun Marella</a></b>
</p>
