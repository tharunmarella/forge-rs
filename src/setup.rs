use crate::config::Config;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;

const ORANGE: Color = Color::Indexed(208);
const WHITE: Color = Color::Indexed(15);
const DIM: Color = Color::Indexed(240);
const GREEN: Color = Color::Indexed(34);

#[derive(Clone)]
struct Provider {
    name: &'static str,
    id: &'static str,
    models: &'static [&'static str],
    env_var: &'static str,
    base_url: Option<&'static str>,
}

const PROVIDERS: &[Provider] = &[
    // === Anthropic Claude - Best for coding ===
    Provider {
        name: "Anthropic Claude",
        id: "anthropic",
        models: &[
            "claude-sonnet-4-20250514",      // Best coding model
            "claude-opus-4-20250514",        // Complex tasks
            "claude-haiku-4-20250514",       // Fast & cheap
            "claude-sonnet-4.5-20251101",    // Latest Sonnet
            "claude-opus-4.5-20251115",      // Latest Opus
            "claude-haiku-4.5-20251022",     // Latest Haiku
        ],
        env_var: "ANTHROPIC_API_KEY",
        base_url: None,
    },
    // === Google Gemini ===
    Provider {
        name: "Google Gemini",
        id: "gemini",
        models: &[
            "gemini-3-flash-preview",            // Most balanced (Dec 2025)
            "gemini-3-pro-preview",              // Most intelligent (Nov 2025)
            "gemini-2.5-flash",                  // Best price-performance (Stable)
            "gemini-2.5-pro",                    // Advanced thinking (Stable)
            "gemini-2.0-flash-exp",              // Fast experimental
        ],
        env_var: "GEMINI_API_KEY",
        base_url: None,
    },
    // === OpenAI ===
    Provider {
        name: "OpenAI",
        id: "openai",
        models: &[
            "gpt-4o",                        // Multimodal
            "gpt-4o-mini",                   // Fast & cheap
            "gpt-4.1",                       // Strong coding (55% SWE-bench)
            "gpt-4.1-mini",                  // Smaller variant
            "gpt-5",                         // Latest flagship
            "gpt-5.1-codex",                 // Optimized for coding
            "gpt-5.2",                       // Most capable
            "o3-mini",                       // Reasoning model
        ],
        env_var: "OPENAI_API_KEY",
        base_url: None,
    },
    // === Groq ===
    Provider {
        name: "Groq",
        id: "groq",
        models: &[
            "llama-3.3-70b-versatile",       // Best overall
            "llama-3-groq-70b-tool-use",     // #1 function calling
            "llama-3-groq-8b-tool-use",      // Fast tool use
            "llama-3.1-70b-instant",         // Large model
            "llama-3.1-8b-instant",          // Fast
            "compound",                       // Agentic system
            "compound-mini",                  // Fast agentic
        ],
        env_var: "GROQ_API_KEY",
        base_url: Some("https://api.groq.com/openai/v1"),
    },
    // === Local Models - Apple Silicon ===
    Provider {
        name: "Local Models - Apple Silicon",
        id: "mlx",
        models: &[
            "mlx-community/Qwen3-Coder-30B-A3B-Instruct-4bit-dwq-v2 (~8GB  | 🥇 Best — MoE 3B active, blazing fast)",
            "mlx-community/Qwen3-Coder-30B-A3B-Instruct-6bit-DWQ-lr3e-7 (~12GB | 🥈 Better quality, still fast)",
            "mlx-community/Qwen3-Coder-30B-A3B-Instruct-8bit (~22GB | 🥉 Near full quality)",
            "mlx-community/Qwen2.5-Coder-32B-Instruct-4bit (~18GB |  4  Dense 32B, solid)",
            "mlx-community/IQuest-Coder-V1-40B-Loop-Instruct-4bit (~22GB |  5  Agentic-tuned)",
            "mlx-community/Qwen2.5-Coder-14B-Instruct-8bit (~14GB |  6  Dense 14B full quality)",
            "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit (~4GB  |     Balanced — 8GB Mac)",
            "mlx-community/Qwen2.5-Coder-1.5B-Instruct-4bit (~1GB  |     Ultra fast — any Mac)",
            "mlx-community/Qwen3-Coder-Next-4bit (~40GB | Max quality — 64GB Mac)",
            "mlx-community/Qwen3-Coder-480B-A35B-Instruct-4bit (~100GB| Frontier — 192GB Mac)",
        ],
        env_var: "LOCAL_MODELS",
        base_url: None,
    },
];

enum SetupStep {
    Provider,
    ApiKey,
    Model,
    Done,
}

struct SetupState {
    step: SetupStep,
    provider_idx: usize,
    model_idx: usize,
    api_key: String,
    cursor: usize,
    error: Option<String>,
}

/// Check if setup is needed
pub fn needs_setup(config: &Config) -> bool {
    if config.is_local_model() {
        return false;
    }
    config.api_key().is_none()
}

/// Run the setup wizard
pub fn run_setup(config: &mut Config) -> Result<bool> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = SetupState {
        step: SetupStep::Provider,
        provider_idx: 0,
        model_idx: 0,
        api_key: String::new(),
        cursor: 0,
        error: None,
    };

    let result = run_setup_loop(&mut terminal, &mut state, config);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_setup_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut SetupState,
    config: &mut Config,
) -> Result<bool> {
    loop {
        terminal.draw(|f| draw_setup(f, state))?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Esc => return Ok(false), // Cancelled
                KeyCode::Enter => {
                    match state.step {
                        SetupStep::Provider => {
                            let provider = &PROVIDERS[state.provider_idx];
                            if provider.id == "mlx" || provider.id == "local" {
                                state.step = SetupStep::Model;
                            } else {
                                state.step = SetupStep::ApiKey;
                            }
                            state.error = None;
                        }
                        SetupStep::ApiKey => {
                            if state.api_key.trim().is_empty() {
                                state.error = Some("API key is required".to_string());
                            } else {
                                state.step = SetupStep::Model;
                                state.error = None;
                            }
                        }
                        SetupStep::Model => {
                            // Save config
                            let provider = &PROVIDERS[state.provider_idx];
                            config.provider = provider.id.to_string();
                            
                            // Extract model name (strip description if present)
                            let model_str = provider.models[state.model_idx];
                            let model_name = model_str.split(" (").next().unwrap_or(model_str);
                            config.model = model_name.to_string();
                            config.base_url = provider.base_url.map(|s| s.to_string());
                            
                            // MLX uses mlx_lm.server — store the local server URL
                            if provider.id == "mlx" {
                                config.base_url = Some("http://localhost:8080/v1".to_string());
                            }
                            
                            match provider.id {
                                "gemini" => config.gemini_api_key = Some(state.api_key.clone()),
                                "anthropic" => config.anthropic_api_key = Some(state.api_key.clone()),
                                "openai" => config.openai_api_key = Some(state.api_key.clone()),
                                "groq" => config.groq_api_key = Some(state.api_key.clone()),
                                _ => {}
                            }
                            
                            config.save()?;
                            
                            // For MLX, show server start instructions
                            if provider.id == "mlx" {
                                disable_raw_mode()?;
                                execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

                                println!("\n✓ MLX configured. Before running forge, start the model server:\n");
                                println!("  pip install mlx-lm");
                                println!("  python -m mlx_lm.server --model {}\n", config.model);
                                println!("Forge will connect to http://localhost:8080/v1 automatically.\n");
                                println!("Press Enter to continue...");
                                let _ = std::io::stdin().read_line(&mut String::new());

                                enable_raw_mode()?;
                                execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                                terminal.clear()?;
                            }
                            
                            state.step = SetupStep::Done;
                        }
                        SetupStep::Done => return Ok(true),
                    }
                }
                KeyCode::Up => {
                    match state.step {
                        SetupStep::Provider if state.provider_idx > 0 => {
                            state.provider_idx -= 1;
                            state.model_idx = 0;
                        }
                        SetupStep::Model if state.model_idx > 0 => {
                            state.model_idx -= 1;
                        }
                        _ => {}
                    }
                }
                KeyCode::Down => {
                    match state.step {
                        SetupStep::Provider if state.provider_idx < PROVIDERS.len() - 1 => {
                            state.provider_idx += 1;
                            state.model_idx = 0;
                        }
                        SetupStep::Model => {
                            let models = PROVIDERS[state.provider_idx].models;
                            if state.model_idx < models.len() - 1 {
                                state.model_idx += 1;
                            }
                        }
                        _ => {}
                    }
                }
                KeyCode::Char(c) => {
                    if matches!(state.step, SetupStep::ApiKey) {
                        state.api_key.insert(state.cursor, c);
                        state.cursor += 1;
                        state.error = None;
                    }
                }
                KeyCode::Backspace => {
                    if matches!(state.step, SetupStep::ApiKey) && state.cursor > 0 {
                        state.cursor -= 1;
                        state.api_key.remove(state.cursor);
                    }
                }
                KeyCode::Left if state.cursor > 0 => state.cursor -= 1,
                KeyCode::Right if state.cursor < state.api_key.len() => state.cursor += 1,
                _ => {}
            }
        }
    }
}

fn draw_setup(f: &mut Frame, state: &SetupState) {
    let area = f.area();
    
    // Center content
    let width = 60.min(area.width - 4);
    let height = 20.min(area.height - 4);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let content_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, content_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),  // Header
            Constraint::Length(2),  // Step indicator
            Constraint::Min(10),    // Content
            Constraint::Length(2),  // Footer
        ])
        .split(content_area);

    // Header
    let header = vec![
        Line::from(Span::styled("⚡ Forge Setup", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("Configure your AI provider", Style::default().fg(DIM))),
    ];
    f.render_widget(Paragraph::new(header), chunks[0]);

    // Step indicator
    let step_num = match state.step {
        SetupStep::Provider => 1,
        SetupStep::ApiKey => 2,
        SetupStep::Model => 3,
        SetupStep::Done => 4,
    };
    let steps = Line::from(vec![
        Span::styled(if step_num >= 1 { "● " } else { "○ " }, Style::default().fg(if step_num >= 1 { ORANGE } else { DIM })),
        Span::styled("Provider", Style::default().fg(if step_num == 1 { WHITE } else { DIM })),
        Span::styled("  →  ", Style::default().fg(DIM)),
        Span::styled(if step_num >= 2 { "● " } else { "○ " }, Style::default().fg(if step_num >= 2 { ORANGE } else { DIM })),
        Span::styled("API Key", Style::default().fg(if step_num == 2 { WHITE } else { DIM })),
        Span::styled("  →  ", Style::default().fg(DIM)),
        Span::styled(if step_num >= 3 { "● " } else { "○ " }, Style::default().fg(if step_num >= 3 { ORANGE } else { DIM })),
        Span::styled("Model", Style::default().fg(if step_num == 3 { WHITE } else { DIM })),
    ]);
    f.render_widget(Paragraph::new(steps), chunks[1]);

    // Content based on step
    match state.step {
        SetupStep::Provider => {
            let items: Vec<ListItem> = PROVIDERS.iter().enumerate().map(|(i, p)| {
                let style = if i == state.provider_idx {
                    Style::default().fg(Color::Black).bg(ORANGE)
                } else {
                    Style::default().fg(WHITE)
                };
                ListItem::new(Line::from(Span::styled(format!("  {}  ", p.name), style)))
            }).collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(ORANGE)).title(" Select Provider "));
            f.render_widget(list, chunks[2]);
        }
        SetupStep::ApiKey => {
            let provider = &PROVIDERS[state.provider_idx];
            
            let mut lines = vec![
                Line::from(""),
                Line::from(Span::styled(format!("Enter your {} API key:", provider.name), Style::default().fg(WHITE))),
                Line::from(Span::styled(format!("(or set {} environment variable)", provider.env_var), Style::default().fg(DIM))),
                Line::from(""),
            ];

            // Masked key input
            let masked: String = if state.api_key.is_empty() {
                String::new()
            } else {
                let visible_chars = 4.min(state.api_key.len());
                let hidden = state.api_key.len() - visible_chars;
                format!("{}{}", "*".repeat(hidden), &state.api_key[hidden..])
            };
            
            lines.push(Line::from(vec![
                Span::styled("> ", Style::default().fg(ORANGE)),
                Span::styled(&masked, Style::default().fg(WHITE)),
                Span::styled("_", Style::default().fg(ORANGE)),
            ]));

            if let Some(ref err) = state.error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(err, Style::default().fg(Color::Red))));
            }

            let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(ORANGE)).title(" API Key ");
            f.render_widget(Paragraph::new(lines).block(block), chunks[2]);
        }
        SetupStep::Model => {
            let provider = &PROVIDERS[state.provider_idx];
            let items: Vec<ListItem> = provider.models.iter().enumerate().map(|(i, m)| {
                let style = if i == state.model_idx {
                    Style::default().fg(Color::Black).bg(ORANGE)
                } else {
                    Style::default().fg(WHITE)
                };
                // Strip mlx-community/ prefix for display
                let display_name = m.strip_prefix("mlx-community/").unwrap_or(m);
                ListItem::new(Line::from(Span::styled(format!("  {}  ", display_name), style)))
            }).collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(ORANGE)).title(" Select Model "));
            f.render_widget(list, chunks[2]);
        }
        SetupStep::Done => {
            let provider = &PROVIDERS[state.provider_idx];
            // Extract clean model name without description
            let model_str = provider.models[state.model_idx];
            let model_name = model_str.split(" (").next().unwrap_or(model_str);
            
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled("✓ Setup complete!", Style::default().fg(GREEN).add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from(Span::styled(format!("Provider: {}", provider.name), Style::default().fg(WHITE))),
                Line::from(Span::styled(format!("Model: {}", model_name), Style::default().fg(WHITE))),
                Line::from(""),
                Line::from(Span::styled("Press Enter to start", Style::default().fg(DIM))),
            ];
            f.render_widget(Paragraph::new(lines), chunks[2]);
        }
    }

    // Footer
    let footer = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(WHITE)),
        Span::styled(" navigate  ", Style::default().fg(DIM)),
        Span::styled("Enter", Style::default().fg(WHITE)),
        Span::styled(" select  ", Style::default().fg(DIM)),
        Span::styled("Esc", Style::default().fg(WHITE)),
        Span::styled(" cancel", Style::default().fg(DIM)),
    ]);
    f.render_widget(Paragraph::new(footer), chunks[3]);
}
