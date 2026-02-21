mod api;
mod checkpoint;
mod code_graph;
mod config;
mod context;
mod context7;
mod llm;
mod lsp;
mod repomap;
mod session;
mod setup;
mod tools;
mod tui;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge", version, about = "Terminal-first AI coding agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Initial prompt to send
    prompt: Option<String>,

    /// Model to use (e.g., gemini-2.5-flash, claude-sonnet-4, gpt-4o)
    #[arg(short, long, default_value = "gemini-2.5-flash")]
    model: String,

    /// API provider (gemini, anthropic, openai)
    #[arg(short, long)]
    provider: Option<String>,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    workdir: String,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Plan mode (read-only, no file modifications)
    #[arg(long)]
    plan: bool,
    
    /// Auto-approve all tool calls (YOLO mode)
    #[arg(long)]
    yolo: bool,
    
    /// Resume the last session for this directory
    #[arg(long)]
    resume: bool,
    
    /// Resume a specific session by ID
    #[arg(long)]
    session: Option<String>,

    /// Disable building the RepoMap on startup
    #[arg(long)]
    no_repomap: bool,

    /// Session timeout in seconds
    #[arg(long)]
    timeout: Option<u64>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run setup wizard to configure provider and API key
    Setup,
    
    /// Undo to the previous checkpoint
    Undo,
    
    /// List all checkpoints
    Checkpoints,
    
    /// Restore to a specific checkpoint
    Restore {
        /// Checkpoint ID (commit hash)
        id: String,
    },
    
    /// Show diff from a checkpoint
    Diff {
        /// Checkpoint ID (commit hash)
        id: String,
    },
    
    
    /// Configure forge settings
    Config {
        /// Setting to change (e.g., auto-approve.write_operations=true)
        setting: Option<String>,
    },
    
    /// List or manage saved sessions
    Sessions {
        #[command(subcommand)]
        action: Option<SessionAction>,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// List all saved sessions
    List,
    /// Resume a specific session
    Resume { id: String },
    /// Delete a session
    Delete { id: String },
    /// Clear all sessions
    Clear,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _start_time = std::time::Instant::now();
    let cli = Cli::parse();

    // Initialize logging
    let is_tui_mode = cli.prompt.is_none();
    use tracing_subscriber::EnvFilter;
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("forge=info,warn"))
    };
    
    if is_tui_mode {
        // TUI mode: log to file to avoid interfering with terminal
        let log_file = std::fs::File::create("/tmp/forge.log").ok();
        if let Some(file) = log_file {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(false)
                .with_writer(file)
                .init();
        }
    } else {
        // CLI mode: log to stdout
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .without_time()
            .init();
    }

    // Expand workdir
    let workdir = shellexpand::tilde(&cli.workdir).to_string();
    let workdir = std::path::PathBuf::from(&workdir).canonicalize()?;

    // Handle subcommands
    if let Some(cmd) = cli.command {
        return handle_command(cmd, &workdir).await;
    }

    // Load or create config
    let mut cfg = config::Config::load()?;
    
    // Run setup wizard if no API key configured
    if setup::needs_setup(&cfg) && cli.prompt.is_none() {
        if !setup::run_setup(&mut cfg)? {
            println!("Setup cancelled. Run 'forge' again to configure.");
            return Ok(());
        }
        // Reload config after setup
        cfg = config::Config::load()?;
    }
    
    // Override with CLI args (but preserve auto-detected defaults if not explicitly set)
    if let Some(provider) = cli.provider {
        cfg.provider = provider;
    }
    
    // Only override model if explicitly provided AND different from default
    // Don't override auto-detected MLX models with CLI defaults
    if cli.model != "gemini-2.5-flash" {
        cfg.model = cli.model;
    } else if cfg.provider == "mlx" && cfg.model.starts_with("mlx-community/") {
        // Keep the auto-detected MLX model
    } else if cfg.provider != "mlx" {
        // For non-MLX providers, use CLI default if no config model set
        if cfg.model.starts_with("mlx-community/") {
            // Config has MLX model but we're not using MLX provider
            cfg.model = cli.model;
        }
    }
    
    cfg.plan_mode = cli.plan;
    
    // YOLO mode - auto-approve everything
    if cli.yolo {
        cfg.auto_approve.yolo = true;
    }

    if cli.no_repomap {
        cfg.no_repomap = true;
    }

    if let Some(timeout) = cli.timeout {
        cfg.timeout = Some(timeout);
    }

    // Create checkpoint before starting
    let mut ckpt = checkpoint::CheckpointManager::new(&workdir)?;
    ckpt.create("forge-session-start")?;

    // Create or resume agent
    let mut agent = if let Some(session_id) = cli.session {
        // Resume specific session
        println!("📂 Resuming session {}...", session_id);
        api::Agent::resume(cfg, workdir.clone(), &session_id).await?
    } else if cli.resume {
        // Resume latest session for this workdir
        if let Some(agent) = api::Agent::resume_latest(cfg.clone(), workdir.clone()).await? {
            println!("📂 Resuming previous session ({} messages)", agent.messages.len());
            agent
        } else {
            println!("No previous session found, starting new");
            api::Agent::new(cfg, workdir.clone()).await?
        }
    } else {
        api::Agent::new(cfg, workdir.clone()).await?
    };

    // If prompt provided, run single-shot mode
    if let Some(prompt) = cli.prompt {
        agent.run_prompt(&prompt).await?;
        
        // Create checkpoint after task
        ckpt.create(&format!("after: {}", truncate(&prompt, 50)))?;
    } else {
        // Interactive TUI mode
        tui::run(agent).await?;
    }

    Ok(())
}

async fn handle_command(cmd: Commands, workdir: &std::path::Path) -> Result<()> {
    match cmd {
        Commands::Setup => {
            let mut cfg = config::Config::load()?;
            if setup::run_setup(&mut cfg)? {
                println!("Setup complete!");
            } else {
                println!("Setup cancelled.");
            }
            return Ok(());
        }
        _ => {}
    }
    
    let ckpt = checkpoint::CheckpointManager::new(workdir)?;
    
    match cmd {
        Commands::Setup => unreachable!(),
        Commands::Undo => {
            println!("Undoing to previous checkpoint...");
            ckpt.undo()?;
            println!("✓ Restored to previous state");
        }
        Commands::Checkpoints => {
            println!("Checkpoints:");
            for (i, id) in ckpt.list().iter().enumerate() {
                let marker = if i == 0 { " (current)" } else { "" };
                println!("  {} {}{}", i, &id[..8], marker);
            }
        }
        Commands::Restore { id } => {
            println!("Restoring to checkpoint {}...", &id[..8.min(id.len())]);
            ckpt.restore(&id)?;
            println!("✓ Restored");
        }
        Commands::Diff { id } => {
            let diff = ckpt.diff(&id)?;
            if diff.is_empty() {
                println!("No changes");
            } else {
                println!("{}", diff);
            }
        }
        Commands::Config { setting } => {
            let mut cfg = config::Config::load()?;
            
            if let Some(s) = setting {
                // Parse setting=value
                if let Some((key, value)) = s.split_once('=') {
                    match key {
                        "auto-approve.read_operations" => {
                            cfg.auto_approve.read_operations = value == "true";
                        }
                        "auto-approve.write_operations" => {
                            cfg.auto_approve.write_operations = value == "true";
                        }
                        "auto-approve.commands" => {
                            cfg.auto_approve.commands = value == "true";
                        }
                        "provider" => {
                            cfg.provider = value.to_string();
                        }
                        "model" => {
                            cfg.model = value.to_string();
                        }
                        _ => {
                            println!("Unknown setting: {}", key);
                            return Ok(());
                        }
                    }
                    cfg.save()?;
                    println!("✓ Set {} = {}", key, value);
                } else {
                    println!("Usage: forge config <key>=<value>");
                }
            } else {
                // Show current config
                println!("Current configuration:");
                println!("  provider: {}", cfg.provider);
                println!("  model: {}", cfg.model);
                println!("  auto-approve:");
                println!("    read_operations: {}", cfg.auto_approve.read_operations);
                println!("    write_operations: {}", cfg.auto_approve.write_operations);
                println!("    commands: {}", cfg.auto_approve.commands);
            }
        }
        Commands::Sessions { action } => {
            match action.unwrap_or(SessionAction::List) {
                SessionAction::List => {
                    let sessions = session::Session::list()?;
                    if sessions.is_empty() {
                        println!("No saved sessions");
                    } else {
                        println!("Saved sessions:");
                        for s in sessions {
                            let age = Utc::now() - s.updated_at;
                            let age_str = if age.num_days() > 0 {
                                format!("{}d ago", age.num_days())
                            } else if age.num_hours() > 0 {
                                format!("{}h ago", age.num_hours())
                            } else {
                                format!("{}m ago", age.num_minutes())
                            };
                            println!("  {} │ {} │ {} msgs │ {}",
                                s.id,
                                truncate(&s.title, 40),
                                s.message_count,
                                age_str
                            );
                        }
                        println!("\nResume with: forge --session <ID>");
                    }
                }
                SessionAction::Resume { id } => {
                    println!("Use: forge --session {}", id);
                }
                SessionAction::Delete { id } => {
                    session::Session::delete(&id)?;
                    println!("✓ Deleted session {}", id);
                }
                SessionAction::Clear => {
                    let sessions = session::Session::list()?;
                    for s in sessions {
                        session::Session::delete(&s.id)?;
                    }
                    println!("✓ Cleared all sessions");
                }
            }
        }
    }
    
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } 
    else { format!("{}...", &s[..max]) }
}

// Test function for repo-map
#[allow(dead_code)]
fn test_repo_map() {
    use std::path::PathBuf;
    let root = PathBuf::from(".");
    let mut rm = repomap::RepoMap::new(root, 1024);
    let map = rm.build_from_directory();
    println!("=== REPO MAP ({} chars) ===", map.len());
    println!("{}", map);
}
