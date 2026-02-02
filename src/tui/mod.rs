use crate::api::{Agent, AgentResponse, Message, Role};
use crate::checkpoint::CheckpointManager;
use crate::tools::{self, ToolCall};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use tokio::sync::mpsc;

// Forge color palette
const ORANGE: Color = Color::Indexed(208);
const YELLOW: Color = Color::Indexed(3);
const BLUE: Color = Color::Indexed(39);
const WHITE: Color = Color::Indexed(15);
const DIM: Color = Color::Indexed(240);
const GREEN: Color = Color::Indexed(34);
const RED: Color = Color::Indexed(196);

// Events from background agent task
enum AgentEvent {
    Response(Result<AgentResponse>),
    ToolStarted(usize),
    ToolFinished(usize, bool, u64),
}

pub struct App {
    agent: Agent,
    checkpoint: Option<CheckpointManager>,
    
    input: String,
    input_cursor: usize,
    messages: Vec<ChatMessage>,
    scroll: usize,
    
    pending_approval: Option<PendingApproval>,
    pending_tools: Vec<ToolCall>,
    tool_results: Vec<(String, crate::tools::ToolResult)>,
    current_tool_idx: usize,
    
    command_palette_open: bool,
    command_palette_selected: usize,
    
    model_picker_open: bool,
    model_picker_selected: usize,
    
    is_thinking: bool,
    should_quit: bool,
    
    agent_rx: Option<mpsc::Receiver<AgentEvent>>,
    
    /// Session-only auto-approve all tools (reset on restart)
    session_yolo: bool,
}

#[derive(Clone)]
pub struct ChatMessage {
    role: ChatRole,
    content: String,
    tools: Vec<ToolStatus>,
}

#[derive(Clone, PartialEq)]
pub enum ChatRole { User, Assistant, System }

#[derive(Clone)]
pub struct ToolStatus {
    name: String,
    status: ToolState,
    duration_ms: Option<u64>,
}

#[derive(Clone, PartialEq)]
pub enum ToolState { Pending, Running, Success, Failed, Skipped }

struct PendingApproval {
    tool: ToolCall,
    idx: usize,
}

const COMMANDS: &[&str] = &["Change model", "Ask mode", "Agent mode", "Undo", "Clear"];

impl App {
    pub fn new(agent: Agent) -> Self {
        let workdir = agent.workdir().clone();
        let checkpoint = CheckpointManager::new(&workdir).ok();
        
        Self {
            agent,
            checkpoint,
            input: String::new(),
            input_cursor: 0,
            messages: Vec::new(),
            scroll: 0,
            pending_approval: None,
            pending_tools: Vec::new(),
            tool_results: Vec::new(),
            current_tool_idx: 0,
            command_palette_open: false,
            command_palette_selected: 0,
            model_picker_open: false,
            model_picker_selected: 0,
            is_thinking: false,
            should_quit: false,
            agent_rx: None,
            session_yolo: false,
        }
    }

    fn mode_color(&self) -> Color {
        if self.agent.config.plan_mode { YELLOW } else { BLUE }
    }

    fn mode_text(&self) -> &str {
        if self.agent.config.plan_mode { "ask" } else { "agent" }
    }
}

pub async fn run(agent: Agent) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(agent);

    if let Some(ref mut ckpt) = app.checkpoint {
        ckpt.create("session-start").ok();
    }

    let result = run_app(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        // Draw UI
        terminal.draw(|f| draw_ui(f, app))?;

        // Check for agent events (non-blocking)
        if let Some(ref mut rx) = app.agent_rx {
            match rx.try_recv() {
                Ok(AgentEvent::Response(result)) => {
                    app.is_thinking = false;
                    handle_agent_response(app, result).await?;
                }
                Ok(AgentEvent::ToolStarted(idx)) => {
                    if let Some(msg) = app.messages.last_mut() {
                        if idx < msg.tools.len() {
                            msg.tools[idx].status = ToolState::Running;
                        }
                    }
                }
                Ok(AgentEvent::ToolFinished(idx, success, duration)) => {
                    if let Some(msg) = app.messages.last_mut() {
                        if idx < msg.tools.len() {
                            msg.tools[idx].status = if success { ToolState::Success } else { ToolState::Failed };
                            msg.tools[idx].duration_ms = Some(duration);
                        }
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    app.agent_rx = None;
                }
            }
        }

        // Poll for keyboard events
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                handle_key_event(terminal, app, key).await?;
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

async fn handle_key_event(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App, 
    key: event::KeyEvent
) -> Result<()> {
    // Model picker
    if app.model_picker_open {
        let models = app.agent.config.get_models(&app.agent.config.provider);
        match key.code {
            KeyCode::Esc => app.model_picker_open = false,
            KeyCode::Up if app.model_picker_selected > 0 => app.model_picker_selected -= 1,
            KeyCode::Down if app.model_picker_selected < models.len() - 1 => app.model_picker_selected += 1,
            KeyCode::Enter => {
                if app.model_picker_selected < models.len() {
                    let new_model = models[app.model_picker_selected].clone();
                    app.agent.config.model = new_model.clone();
                    app.agent.config.save().ok();
                    app.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Switched to {}", new_model),
                        tools: vec![],
                    });
                }
                app.model_picker_open = false;
            }
            _ => {}
        }
        return Ok(());
    }

    // Command palette
    if app.command_palette_open {
        match key.code {
            KeyCode::Esc => app.command_palette_open = false,
            KeyCode::Up if app.command_palette_selected > 0 => app.command_palette_selected -= 1,
            KeyCode::Down if app.command_palette_selected < COMMANDS.len() - 1 => app.command_palette_selected += 1,
            KeyCode::Enter => {
                match app.command_palette_selected {
                    0 => { // Change model
                        app.command_palette_open = false;
                        app.model_picker_open = true;
                        app.model_picker_selected = 0;
                    }
                    1 => app.agent.config.plan_mode = true,
                    2 => app.agent.config.plan_mode = false,
                    3 => { if let Some(ref ckpt) = app.checkpoint { ckpt.undo().ok(); } }
                    4 => { app.messages.clear(); app.scroll = 0; }
                    _ => {}
                }
                if app.command_palette_selected != 0 {
                    app.command_palette_open = false;
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // Approval dialog
    if let Some(pending) = app.pending_approval.take() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Execute the approved tool
                execute_pending_tool(terminal, app, pending.tool, pending.idx).await?;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                // Yes to ALL - enable session yolo mode
                app.session_yolo = true;
                execute_pending_tool(terminal, app, pending.tool, pending.idx).await?;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(msg) = app.messages.last_mut() {
                    if pending.idx < msg.tools.len() {
                        msg.tools[pending.idx].status = ToolState::Skipped;
                    }
                }
                // Continue with remaining tools or get next response
                process_remaining_tools(terminal, app).await?;
            }
            _ => {
                app.pending_approval = Some(pending);
            }
        }
        return Ok(());
    }

    // Global shortcuts
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => app.should_quit = true,
            KeyCode::Char('k') => {
                app.command_palette_open = true;
                app.command_palette_selected = 0;
            }
            KeyCode::Char('m') => {
                app.model_picker_open = true;
                app.model_picker_selected = 0;
            }
            KeyCode::Char('z') => {
                if let Some(ref ckpt) = app.checkpoint {
                    if ckpt.undo().is_ok() {
                        app.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: "Restored to previous checkpoint".to_string(),
                            tools: vec![],
                        });
                    }
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // Normal input
    match key.code {
        KeyCode::Enter if !app.input.is_empty() && !app.is_thinking => {
            let input = app.input.trim().to_string();
            app.input.clear();
            app.input_cursor = 0;
            
            // Handle slash commands
            if input.starts_with('/') {
                match input.as_str() {
                    "/model" | "/m" => {
                        app.model_picker_open = true;
                        app.model_picker_selected = 0;
                    }
                    "/undo" => {
                        if let Some(ref ckpt) = app.checkpoint {
                            if ckpt.undo().is_ok() {
                                app.messages.push(ChatMessage {
                                    role: ChatRole::System,
                                    content: "Restored to previous checkpoint".to_string(),
                                    tools: vec![],
                                });
                            }
                        }
                    }
                    "/clear" => {
                        app.messages.clear();
                        app.scroll = 0;
                    }
                    "/help" | "/?" => {
                        app.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: "Commands: /model /undo /clear /help | Tab to toggle mode".to_string(),
                            tools: vec![],
                        });
                    }
                    _ => {
                        app.messages.push(ChatMessage {
                            role: ChatRole::System,
                            content: format!("Unknown command: {}. Try /help", input),
                            tools: vec![],
                        });
                    }
                }
            } else {
                send_message(terminal, app, &input).await?;
            }
        }
        KeyCode::Char(c) => {
            app.input.insert(app.input_cursor, c);
            app.input_cursor += 1;
        }
        KeyCode::Backspace if app.input_cursor > 0 => {
            app.input_cursor -= 1;
            app.input.remove(app.input_cursor);
        }
        KeyCode::Left if app.input_cursor > 0 => app.input_cursor -= 1,
        KeyCode::Right if app.input_cursor < app.input.len() => app.input_cursor += 1,
        KeyCode::Home => app.input_cursor = 0,
        KeyCode::End => app.input_cursor = app.input.len(),
        KeyCode::Tab => {
            app.agent.config.plan_mode = !app.agent.config.plan_mode;
            let mode = if app.agent.config.plan_mode { "ask (read-only)" } else { "agent (full access)" };
            app.messages.push(ChatMessage {
                role: ChatRole::System,
                content: format!("Switched to {} mode", mode),
                tools: vec![],
            });
        }
        KeyCode::Esc => { app.input.clear(); app.input_cursor = 0; }
        KeyCode::Up if app.scroll > 0 => app.scroll -= 1,
        KeyCode::Down => app.scroll += 1,
        _ => {}
    }

    Ok(())
}

async fn send_message(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App, 
    prompt: &str
) -> Result<()> {
    // Start background doc prefetch for this query (Context7)
    app.agent.prefetch_docs(prompt);
    
    // Add user message
    app.messages.push(ChatMessage {
        role: ChatRole::User,
        content: prompt.to_string(),
        tools: vec![],
    });

    app.agent.messages.push(Message {
        role: Role::User,
        content: prompt.to_string(),
        tool_calls: None,
        tool_results: None,
    });

    // Checkpoint
    if let Some(ref mut ckpt) = app.checkpoint {
        ckpt.create(&format!("before: {}", &prompt[..prompt.len().min(30)])).ok();
    }

    // Start thinking
    app.is_thinking = true;
    terminal.draw(|f| draw_ui(f, app))?;

    // Get response (blocking but we redraw first)
    let response = get_completion(&app.agent).await;
    app.is_thinking = false;
    
    handle_agent_response(app, response).await?;

    // Process any pending tools
    process_remaining_tools(terminal, app).await?;

    Ok(())
}

async fn handle_agent_response(app: &mut App, response: Result<AgentResponse>) -> Result<()> {
    match response {
        Ok(AgentResponse::Text(text)) => {
            app.messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: text.clone(),
                tools: vec![],
            });
            app.agent.messages.push(Message {
                role: Role::Assistant,
                content: text,
                tool_calls: None,
                tool_results: None,
            });
        }
        Ok(AgentResponse::ToolCalls { text, calls }) => {
            app.messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: text.clone(),
                tools: calls.iter().map(|c| ToolStatus {
                    name: c.name.clone(),
                    status: ToolState::Pending,
                    duration_ms: None,
                }).collect(),
            });
            
            // Store for processing
            app.pending_tools = calls;
            app.tool_results.clear();
            app.current_tool_idx = 0;
            
            // Store text in agent history (will add tool_calls after execution)
            app.agent.messages.push(Message {
                role: Role::Assistant,
                content: text,
                tool_calls: None, // Will update later
                tool_results: None,
            });
        }
        Ok(AgentResponse::Completion(result)) => {
            app.messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: format!("✅ {}", result),
                tools: vec![],
            });
            
            if let Some(ref mut ckpt) = app.checkpoint {
                ckpt.create("task-completed").ok();
            }
        }
        Ok(AgentResponse::Question(q)) => {
            app.messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: format!("❓ {}", q),
                tools: vec![],
            });
        }
        Err(e) => {
            app.messages.push(ChatMessage {
                role: ChatRole::System,
                content: format!("Error: {}", e),
                tools: vec![],
            });
        }
    }
    
    Ok(())
}

async fn process_remaining_tools(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<()> {
    while app.current_tool_idx < app.pending_tools.len() {
        let call = app.pending_tools[app.current_tool_idx].clone();
        let idx = app.current_tool_idx;
        
        // Check session yolo OR config auto-approve
        let approved = app.session_yolo || app.agent.config.should_auto_approve(&call.name);
        
        if !approved {
            // Show diff preview in IDE for file-modifying tools
            preview_tool_diff(&call, app.agent.workdir());
            
            app.pending_approval = Some(PendingApproval { tool: call, idx });
            return Ok(()); // Wait for user input
        }
        
        execute_pending_tool(terminal, app, call, idx).await?;
    }
    
    // All tools done, get next response if we had tools
    if !app.pending_tools.is_empty() {
        let calls = std::mem::take(&mut app.pending_tools);
        let results = std::mem::take(&mut app.tool_results);
        
        // Update the last assistant message to include tool_calls
        if let Some(msg) = app.agent.messages.last_mut() {
            if msg.role == Role::Assistant {
                msg.tool_calls = Some(calls);
            }
        }
        
        // Add tool results message
        app.agent.messages.push(Message {
            role: Role::Tool,
            content: String::new(),
            tool_calls: None,
            tool_results: Some(results),
        });
        
        // Get next response
        app.is_thinking = true;
        terminal.draw(|f| draw_ui(f, app))?;
        
        let response = get_completion(&app.agent).await;
        app.is_thinking = false;
        
        handle_agent_response(app, response).await?;
        
        // Recurse for any new tools (boxed to avoid infinite future size)
        Box::pin(process_remaining_tools(terminal, app)).await?;
    }
    
    Ok(())
}

async fn execute_pending_tool(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    tool: ToolCall,
    idx: usize,
) -> Result<()> {
    // Update status to running
    if let Some(msg) = app.messages.last_mut() {
        if idx < msg.tools.len() {
            msg.tools[idx].status = ToolState::Running;
        }
    }
    terminal.draw(|f| draw_ui(f, app))?;
    
    // Execute
    let start = std::time::Instant::now();
    let result = tools::execute(&tool, app.agent.workdir(), app.agent.config.plan_mode).await;
    let duration = start.elapsed().as_millis() as u64;
    
    // Update status
    if let Some(msg) = app.messages.last_mut() {
        if idx < msg.tools.len() {
            msg.tools[idx].status = if result.success { ToolState::Success } else { ToolState::Failed };
            msg.tools[idx].duration_ms = Some(duration);
        }
    }
    
    // Store actual result for sending to LLM
    app.tool_results.push((tool.name, result));
    app.current_tool_idx += 1;
    
    Ok(())
}

/// Show diff preview in IDE for file-modifying tools (before approval)
fn preview_tool_diff(tool: &ToolCall, workdir: &std::path::Path) {
    use crate::tools::ide;
    
    let name = tool.name.as_str();
    let args = &tool.arguments;
    
    match name {
        "write_to_file" => {
            if let (Some(path), Some(content)) = (
                args.get("path").and_then(|v| v.as_str()),
                args.get("content").and_then(|v| v.as_str())
            ) {
                let full_path = workdir.join(path);
                let old_content = std::fs::read_to_string(&full_path).unwrap_or_default();
                ide::show_diff_in_ide(&full_path, &old_content, content);
            }
        }
        "replace_in_file" => {
            if let (Some(path), Some(old_str), Some(new_str)) = (
                args.get("path").and_then(|v| v.as_str()),
                args.get("old_str").and_then(|v| v.as_str()),
                args.get("new_str").and_then(|v| v.as_str())
            ) {
                let full_path = workdir.join(path);
                if let Ok(content) = std::fs::read_to_string(&full_path) {
                    let new_content = content.replacen(old_str, new_str, 1);
                    ide::show_diff_in_ide(&full_path, &content, &new_content);
                }
            }
        }
        "apply_patch" => {
            // For patches, just show the file that will be modified
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                let full_path = workdir.join(path);
                ide::open_file_in_ide(&full_path, None);
            }
        }
        _ => {}
    }
}

async fn get_completion(agent: &Agent) -> Result<AgentResponse> {
    use crate::api::{gemini, anthropic, openai};
    
    // Get any prefetched documentation from Context7
    let prefetched_docs = agent.doc_prefetcher().get_cached_docs_for_prompt();
    
    let system_prompt = format!(
        "You are Forge, a terminal-first AI coding agent.\n\
         Mode: {} ({})\n\
         Working directory: {}\n\n\
         Rules:\n\
         - Be concise and direct\n\
         - Use tools to complete tasks\n\
         - Always use attempt_completion when done\n\
         - Read files before editing them\n\
         {}",
        if agent.config.plan_mode { "PLAN" } else { "ACT" },
        if agent.config.plan_mode { "read-only, no modifications" } else { "full access" },
        agent.workdir().display(),
        prefetched_docs
    );
    let tool_defs = tools::definitions(agent.config.plan_mode);

    match agent.config.provider.as_str() {
        "gemini" => gemini::complete(&agent.config, &system_prompt, agent.messages(), &tool_defs).await,
        "anthropic" => anthropic::complete(&agent.config, &system_prompt, agent.messages(), &tool_defs).await,
        // OpenAI and OpenAI-compatible providers (Groq, Together, OpenRouter)
        "openai" | "groq" | "together" | "openrouter" => {
            openai::complete(&agent.config, &system_prompt, agent.messages(), &tool_defs).await
        }
        _ => Err(anyhow::anyhow!("Unknown provider: {}", agent.config.provider)),
    }
}


// ============ DRAWING ============

fn draw_ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_messages(f, app, chunks[0]);
    draw_input(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);

    if app.command_palette_open {
        draw_command_palette(f, app, f.area());
    }

    if app.model_picker_open {
        draw_model_picker(f, app, f.area());
    }

    if let Some(ref pending) = app.pending_approval {
        draw_approval_dialog(f, pending, f.area());
    }
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    if app.messages.is_empty() && !app.is_thinking {
        // Big FORGE ASCII art with braille fire on the right (aligned at bottom)
        let art_lines = [
            ("                                            ", "  ⠀⠀⠀⢱⣆⠀⠀⠀"),
            ("                                            ", "  ⠀⠀⠀⠈⣿⣷⡀⠀⠀"),
            ("                                            ", "  ⠀⠀⠀⢸⣿⣿⣷⣧⠀"),
            ("                                            ", "  ⠀⡀⢠⣿⡟⣿⣿⣿⡇"),
            ("                                            ", "  ⠀⣳⣼⣿⡏⢸⣿⣿⣿"),
            ("                                            ", "  ⣰⣿⣿⡿⠁⢸⣿⣿⡟"),
            (" ███████╗ ██████╗ ██████╗  ██████╗ ███████╗", " ⣾⣿⣿⠟⠀⠀⣾⢿⣿⣿"),
            (" ██╔════╝██╔═══██╗██╔══██╗██╔════╝ ██╔════╝", " ⣿⣿⡏⠀⠀⠀⠃⠸⣿⣿"),
            (" █████╗  ██║   ██║██████╔╝██║  ███╗█████╗  ", " ⳿⣿⣿⠀⠀⠀⠀⢹⣿⡿"),
            (" ██╔══╝  ██║   ██║██╔══██╗██║   ██║██╔══╝  ", " ⠀⠹⣿⣿⡄⠀⠀⢠⣿⡞"),
            (" ██║     ╚██████╔╝██║  ██║╚██████╔╝███████╗", " ⠀⠀⠈⠛⢿⣄⠀⣠⠞⠁"),
            (" ╚═╝      ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝", ""),
        ];

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from("")); // Top padding
        
        for (forge_part, fire_part) in art_lines {
            lines.push(Line::from(vec![
                Span::styled(forge_part, Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
                Span::styled(fire_part, Style::default().fg(ORANGE)),
            ]));
        }
        
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Terminal-first AI coding agent with high-performance context assembly",
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC)
        )));
        lines.push(Line::from(""));
        
        // Model info
        let model_info = format!(" {} / {}", app.agent.config.provider, app.agent.config.model);
        lines.push(Line::from(Span::styled(model_info, Style::default().fg(DIM))));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" What would you like to build?", Style::default().fg(WHITE))));

        let para = Paragraph::new(lines).alignment(ratatui::layout::Alignment::Left);
        f.render_widget(para, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        lines.push(Line::from(""));

        match msg.role {
            ChatRole::User => {
                lines.push(Line::from(Span::styled("You", Style::default().fg(BLUE).add_modifier(Modifier::BOLD))));
                for line in msg.content.lines() {
                    lines.push(Line::from(Span::styled(line, Style::default().fg(Color::White))));
                }
            }
            ChatRole::Assistant => {
                lines.push(Line::from(Span::styled("Forge", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))));
                
                if !msg.content.is_empty() {
                    for line in msg.content.lines() {
                        lines.push(Line::from(Span::styled(line, Style::default().fg(Color::White))));
                    }
                }

                for tool in &msg.tools {
                    let (icon, color) = match tool.status {
                        ToolState::Pending => ("○", DIM),
                        ToolState::Running => ("●", ORANGE),
                        ToolState::Success => ("✓", GREEN),
                        ToolState::Failed => ("✗", RED),
                        ToolState::Skipped => ("−", DIM),
                    };

                    let duration = tool.duration_ms.map(|ms| format!(" {}ms", ms)).unwrap_or_default();
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                        Span::styled(&tool.name, Style::default().fg(WHITE)),
                        Span::styled(duration, Style::default().fg(DIM)),
                    ]));
                }
            }
            ChatRole::System => {
                lines.push(Line::from(vec![
                    Span::styled("⚡ ", Style::default().fg(YELLOW)),
                    Span::styled(&msg.content, Style::default().fg(YELLOW)),
                ]));
            }
        }
    }

    if app.is_thinking {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Forge ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled("thinking...", Style::default().fg(DIM)),
        ]));
    }

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll as u16, 0));

    f.render_widget(para, area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let placeholder = if app.input.is_empty() { " Type a message..." } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.mode_color()))
        .title(Span::styled(placeholder, Style::default().fg(DIM)));

    let input = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(WHITE))
        .block(block);

    f.render_widget(input, area);
    
    if !app.is_thinking && app.pending_approval.is_none() {
        f.set_cursor_position((area.x + app.input_cursor as u16 + 1, area.y + 1));
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    // Shorten model name for display
    let model_display = if app.agent.config.model.len() > 20 {
        format!("{}...", &app.agent.config.model[..17])
    } else {
        app.agent.config.model.clone()
    };
    
    let status = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(&model_display, Style::default().fg(ORANGE)),
        Span::styled("  •  ", Style::default().fg(DIM)),
        Span::styled(format!("{} mode", app.mode_text()), Style::default().fg(app.mode_color()).add_modifier(Modifier::BOLD)),
        Span::styled(" (Tab)", Style::default().fg(DIM)),
        Span::styled("  │  ", Style::default().fg(DIM)),
        Span::styled("/model", Style::default().fg(WHITE)),
        Span::styled("  ", Style::default().fg(DIM)),
        Span::styled("/undo", Style::default().fg(WHITE)),
        Span::styled("  ", Style::default().fg(DIM)),
        Span::styled("/help", Style::default().fg(WHITE)),
    ]);

    f.render_widget(Paragraph::new(status), area);
}

fn draw_model_picker(f: &mut Frame, app: &App, area: Rect) {
    let models = app.agent.config.get_models(&app.agent.config.provider);
    
    let width = 45.min(area.width - 4);
    let height = (models.len() as u16 + 3).min(area.height - 4);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 3;

    let rect = Rect::new(x, y, width, height);
    f.render_widget(Clear, rect);

    let title = format!(" {} Models ", app.agent.config.provider.to_uppercase());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ORANGE))
        .title(Span::styled(title, Style::default().fg(ORANGE)));

    let items: Vec<ListItem> = models
        .iter()
        .enumerate()
        .map(|(i, model)| {
            let is_current = model == &app.agent.config.model;
            let prefix = if is_current { "● " } else { "  " };
            
            let style = if i == app.model_picker_selected {
                Style::default().fg(Color::Black).bg(ORANGE)
            } else if is_current {
                Style::default().fg(GREEN)
            } else {
                Style::default().fg(WHITE)
            };
            
            // Shorten long model names
            let display_name = if model.len() > 35 {
                format!("{}...", &model[..32])
            } else {
                model.clone()
            };
            
            ListItem::new(Line::from(Span::styled(format!("{}{}", prefix, display_name), style)))
        })
        .collect();

    f.render_widget(List::new(items).block(block), rect);
}

fn draw_command_palette(f: &mut Frame, app: &App, area: Rect) {
    let width = 30.min(area.width - 4);
    let height = (COMMANDS.len() as u16 + 2).min(area.height - 4);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 3;

    let rect = Rect::new(x, y, width, height);
    f.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ORANGE))
        .title(Span::styled(" Commands ", Style::default().fg(ORANGE)));

    let items: Vec<ListItem> = COMMANDS
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if i == app.command_palette_selected {
                Style::default().bg(ORANGE).fg(Color::Black)
            } else {
                Style::default().fg(WHITE)
            };
            ListItem::new(Line::from(Span::styled(format!("  {} ", name), style)))
        })
        .collect();

    f.render_widget(List::new(items).block(block), rect);
}

fn draw_approval_dialog(f: &mut Frame, pending: &PendingApproval, area: Rect) {
    let width = 60.min(area.width - 4);
    let height = 6;
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;

    let rect = Rect::new(x, y, width, height);
    f.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(YELLOW))
        .title(Span::styled(format!(" {} ", pending.tool.name), Style::default().fg(YELLOW)));

    let content = vec![
        Line::from(""),
        Line::from(Span::styled("  Diff preview opened in IDE", Style::default().fg(DIM))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [", Style::default().fg(DIM)),
            Span::styled("y", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("] yes  [", Style::default().fg(DIM)),
            Span::styled("n", Style::default().fg(RED).add_modifier(Modifier::BOLD)),
            Span::styled("] no  [", Style::default().fg(DIM)),
            Span::styled("a", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled("] yes to all (session)", Style::default().fg(DIM)),
        ]),
    ];

    f.render_widget(Paragraph::new(content).block(block), rect);
}
