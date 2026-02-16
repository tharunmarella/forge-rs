# MLX Deep Integration

Forge-rs provides seamless integration with Apple's MLX framework for local AI coding on Apple Silicon. The integration is designed to be completely automatic - no manual server management required!

## Features

### 🚀 Automatic Server Management
- **Zero Configuration**: Just set `provider: "mlx"` in your config
- **Auto-Start**: MLX server starts automatically when needed
- **Auto-Stop**: Server shuts down gracefully when done
- **Port Management**: Automatically finds available ports
- **Health Monitoring**: Continuous health checks with auto-recovery

### 🔧 Seamless Experience
- **No Manual Setup**: No need to start servers manually
- **OpenAI Compatible**: Uses standard OpenAI API interface internally
- **Error Recovery**: Automatic restart on server failures
- **Resource Cleanup**: Proper cleanup on shutdown

### 🎯 Smart Configuration
- **Auto-Detection**: Automatically detects MLX availability on macOS
- **Fallback Support**: Falls back to other providers if MLX unavailable
- **Model Selection**: Pre-configured with optimized MLX models

## Quick Start

### 1. Basic Usage

```rust
use forge::api::Agent;
use forge::config::Config;

// Create MLX config - server will start automatically
let config = Config {
    provider: "mlx".to_string(),
    model: "mlx-community/Llama-3.2-3B-Instruct-4bit".to_string(),
    ..Default::default()
};

// Agent initialization starts MLX server automatically
let mut agent = Agent::new(config, workdir, None).await?;

// Use normally - server is running in background
let response = agent.run_prompt("Write a Rust function").await?;

// Server stops automatically when agent is dropped
```

### 2. Configuration File

Create `~/.forge/config.json`:

```json
{
  "provider": "mlx",
  "model": "mlx-community/Llama-3.2-3B-Instruct-4bit",
  "plan_mode": false
}
```

### 3. Auto-Detection

On macOS systems with MLX available, Forge will automatically prefer MLX:

```rust
// This will auto-detect and use MLX if available
let config = Config::default_with_auto_detection();
```

## Available Models

Pre-configured MLX models optimized for coding:

- `mlx-community/Llama-3.2-3B-Instruct-4bit` (Default - Fast, good quality)
- `mlx-community/Qwen2.5-Coder-7B-Instruct-4bit` (Specialized for coding)
- `mlx-community/DeepSeek-Coder-V2-Lite-Instruct-4bit` (Code-focused)
- `mlx-community/CodeLlama-7b-Instruct-hf-4bit` (Code generation)
- `mlx-community/Llama-3.1-8B-Instruct-4bit` (General purpose)
- `mlx-community/Mistral-7B-Instruct-v0.3-4bit` (Balanced performance)

## Requirements

### System Requirements
- **macOS**: Required for MLX framework
- **Apple Silicon**: M1, M2, M3, or M4 chip
- **Python 3.8+**: For MLX server
- **Memory**: 8GB+ RAM recommended

### Python Dependencies

Install MLX dependencies:

```bash
pip install mlx-lm fastapi uvicorn
```

Or use the provided requirements:

```bash
pip install -r scripts/requirements.txt
```

## Architecture

### Server Management Flow

```
1. Agent.new() called with MLX config
2. MLX Manager checks if server is running
3. If not running, starts Python MLX server
4. Waits for server to be healthy
5. Configures OpenAI client with local server URL
6. Starts background health monitoring
7. Agent ready for use
```

### Health Monitoring

- **Continuous Checks**: Every 5 seconds
- **Failure Detection**: 3 consecutive failures trigger restart
- **Auto-Recovery**: Automatic server restart on failure
- **Graceful Degradation**: Proper error handling

### Process Lifecycle

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Agent Init    │───▶│   Start Server   │───▶│  Health Monitor │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                                        │
                                                        ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│  Agent Shutdown │◀───│   Stop Server    │◀───│  Server Failure │
└─────────────────┘    └──────────────────┘    └─────────────────┘
```

## Troubleshooting

### Common Issues

1. **MLX Not Found**
   ```
   Error: MLX server script not found
   ```
   - Ensure `scripts/mlx_server.py` exists
   - Check Python MLX installation: `python3 -c "import mlx.core"`

2. **Port Already in Use**
   ```
   Error: address already in use
   ```
   - Automatic port selection handles this
   - Check for conflicting processes: `lsof -i :8000-8100`

3. **Server Start Timeout**
   ```
   Error: MLX server failed to start within 30 seconds
   ```
   - Check Python dependencies: `pip install mlx-lm fastapi uvicorn`
   - Verify model download: Models are downloaded on first use

4. **Memory Issues**
   ```
   Error: Failed to load MLX model
   ```
   - Try smaller models (3B instead of 7B)
   - Ensure sufficient RAM available
   - Close other memory-intensive applications

### Debug Mode

Enable debug logging:

```bash
RUST_LOG=debug forge
```

This will show:
- Server startup/shutdown events
- Health check status
- Port selection
- Model loading progress

### Manual Server Control

For debugging, you can run the server manually:

```bash
python3 scripts/mlx_server.py --model mlx-community/Llama-3.2-3B-Instruct-4bit --port 8000
```

## Performance

### Benchmarks (Apple M2 Pro)

| Model | Size | Load Time | Tokens/sec | Memory |
|-------|------|-----------|------------|--------|
| Llama-3.2-3B-4bit | 2.1GB | ~10s | ~25 | 4GB |
| Qwen2.5-Coder-7B-4bit | 4.2GB | ~15s | ~18 | 6GB |
| DeepSeek-Coder-V2-Lite-4bit | 4.1GB | ~15s | ~20 | 6GB |

### Optimization Tips

1. **Model Selection**: Start with 3B models for faster loading
2. **Memory Management**: Close other applications for better performance
3. **Persistent Server**: Keep agent alive to avoid reload overhead
4. **Batch Processing**: Process multiple requests in same session

## Examples

See `examples/mlx_deep_integration.rs` for a complete working example.

## Contributing

To improve MLX integration:

1. **Server Enhancements**: Modify `scripts/mlx_server.py`
2. **Manager Improvements**: Update `src/llm/mlx_manager.rs`
3. **Configuration**: Extend `src/config/mod.rs`
4. **Documentation**: Update this file

## License

MLX integration follows the same MIT license as forge-rs.