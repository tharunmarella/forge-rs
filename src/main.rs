mod api;
mod checkpoint;
mod config;
mod context;
mod setup;
mod tools;
mod tui;

use anyhow::Result;
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .init();
    }

    // Expand workdir
    let workdir = shellexpand::tilde(&cli.workdir).to_string();
    let workdir = std::path::PathBuf::from(&workdir).canonicalize()?;

    // Handle subcommands
    if let Some(cmd) = cli.command {
        return handle_command(cmd, &workdir);
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
    
    // Override with CLI args
    if let Some(provider) = cli.provider {
        cfg.provider = provider;
    }
    if cli.model != "gemini-2.5-flash" {
        cfg.model = cli.model;
    }
    cfg.plan_mode = cli.plan;
    
    // YOLO mode - auto-approve everything
    if cli.yolo {
        cfg.auto_approve.read_operations = true;
        cfg.auto_approve.write_operations = true;
        cfg.auto_approve.commands = true;
    }

    // Create checkpoint before starting
    let mut ckpt = checkpoint::CheckpointManager::new(&workdir)?;
    ckpt.create("forge-session-start")?;

    // Create agent
    let mut agent = api::Agent::new(cfg, workdir.clone()).await?;

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

fn handle_command(cmd: Commands, workdir: &std::path::Path) -> Result<()> {
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
    }
    
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } 
    else { format!("{}...", &s[..max]) }
}
