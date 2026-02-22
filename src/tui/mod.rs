use crate::api::{Agent, Message, Role};
use crate::checkpoint::CheckpointManager;
use crate::tools::ToolCall;
use anyhow::Result;
use rig::completion::Chat;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io::{self, Stdout};

// Forge color palette
const ORANGE: Color = Color::Indexed(208);
const YELLOW: Color = Color::Indexed(3);
const BLUE: Color = Color::Indexed(39);
const WHITE: Color = Color::Indexed(15);
const DIM: Color = Color::Indexed(240);
const GREEN: Color = Color::Indexed(34);
const RED: Color = Color::Indexed(196);

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

#[derive(Clone)]
pub struct ChatMessage {
    role: ChatRole,
    content: String,
    tools: Vec<ToolStatus>,
}

pub struct PendingApproval {
    tool: ToolCall,
    idx: usize,
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
    thinking_count: usize,
    should_quit: bool,
    
    session_yolo: bool,
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
            thinking_count: 0,
            should_quit: false,
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
    let session_start = std::time::Instant::now();

    if let Some(ref mut ckpt) = app.checkpoint {
        ckpt.create("session-start").ok();
    }

    let result = run_app(&mut terminal, &mut app, session_start).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App, session_start: std::time::Instant) -> Result<()> {
    loop {
        if let Some(timeout_secs) = app.agent.config.timeout {
            if session_start.elapsed().as_secs() >= timeout_secs {
                app.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: format!("Session timeout reached ({}s). Saving and exiting...", timeout_secs),
                    tools: vec![],
                });
                terminal.draw(|f| draw_ui(f, app))?;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                app.should_quit = true;
            }
        }

        terminal.draw(|f| draw_ui(f, app))?;

        if app.is_thinking {
            app.thinking_count = app.thinking_count.wrapping_add(1);
        }

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                handle_key_event(terminal, app, key).await?;
            }
        }

        if app.should_quit {
            app.agent.save_session().ok();
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

    if app.command_palette_open {
        match key.code {
            KeyCode::Esc => app.command_palette_open = false,
            KeyCode::Up if app.command_palette_selected > 0 => app.command_palette_selected -= 1,
            KeyCode::Down if app.command_palette_selected < COMMANDS.len() - 1 => app.command_palette_selected += 1,
            KeyCode::Enter => {
                match app.command_palette_selected {
                    0 => {
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
                app.command_palette_open = false;
            }
            _ => {}
        }
        return Ok(());
    }

    if let Some(pending) = app.pending_approval.take() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                execute_pending_tool(terminal, app, pending.tool, pending.idx).await?;
                process_remaining_tools(terminal, app).await?;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                app.session_yolo = true;
                execute_pending_tool(terminal, app, pending.tool, pending.idx).await?;
                process_remaining_tools(terminal, app).await?;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(msg) = app.messages.last_mut() {
                    if pending.idx < msg.tools.len() {
                        msg.tools[pending.idx].status = ToolState::Skipped;
                    }
                }
                process_remaining_tools(terminal, app).await?;
            }
            _ => {
                app.pending_approval = Some(pending);
            }
        }
        return Ok(());
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => app.should_quit = true,
            KeyCode::Char('k') => {
                app.command_palette_open = true;
                app.command_palette_selected = 0;
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

    match key.code {
        KeyCode::Enter if !app.input.is_empty() && !app.is_thinking => {
            let input = app.input.trim().to_string();
            app.input.clear();
            app.input_cursor = 0;
            if input.starts_with('/') {
                handle_slash_command(app, &input).await?;
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
        KeyCode::Tab => {
            app.agent.config.plan_mode = !app.agent.config.plan_mode;
        }
        KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
        KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::Down => app.scroll = app.scroll.saturating_add(1),
        _ => {}
    }

    Ok(())
}

async fn handle_slash_command(app: &mut App, cmd: &str) -> Result<()> {
    match cmd {
        "/model" | "/m" => { app.model_picker_open = true; app.model_picker_selected = 0; }
        "/undo" => {
            if let Some(ref ckpt) = app.checkpoint {
                if ckpt.undo().is_ok() {
                    app.messages.push(ChatMessage { role: ChatRole::System, content: "Restored to previous checkpoint".to_string(), tools: vec![] });
                }
            }
        }
        "/clear" => { app.messages.clear(); app.scroll = 0; }
        "/help" | "/?" => {
            app.messages.push(ChatMessage { role: ChatRole::System, content: "Commands: /model /undo /clear /help | Tab: mode | Up/Down: scroll".to_string(), tools: vec![] });
        }
        _ => { app.messages.push(ChatMessage { role: ChatRole::System, content: format!("Unknown command: {}", cmd), tools: vec![] }); }
    }
    Ok(())
}

async fn send_message(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App, prompt: &str) -> Result<()> {

    app.agent.prefetch_docs(prompt);
    app.messages.push(ChatMessage { role: ChatRole::User, content: prompt.to_string(), tools: vec![] });
    app.agent.messages.push(Message { role: Role::User, content: prompt.to_string(), tool_calls: None, tool_results: None });

    if let Some(ref mut ckpt) = app.checkpoint { ckpt.create(&format!("before: {}", &prompt[..prompt.len().min(30)])).ok(); }

    // Redraw immediately so the user message appears before we block on the model call
    terminal.draw(|f| draw_ui(f, app))?;

    app.is_thinking = true;
    app.messages.push(ChatMessage { role: ChatRole::Assistant, content: String::new(), tools: vec![] });

    // Redraw again to show the thinking spinner before blocking
    terminal.draw(|f| draw_ui(f, app))?;
    
    // Convert existing messages to Rig format
    let mut rig_history: Vec<rig::completion::Message> = Vec::new();
    for msg in &app.agent.messages {
        match msg.role {
            Role::User => rig_history.push(rig::completion::Message::user(&msg.content)),
            Role::Assistant => rig_history.push(rig::completion::Message::assistant(&msg.content)),
            _ => {}
        }
    }

    let agent_enum = app.agent.agent.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Agent not initialized"))?;

    let response: String = match agent_enum {
        crate::api::RigAgentEnum::OpenAI(agent) => agent.chat(prompt, rig_history).await?,
        crate::api::RigAgentEnum::Local(agent) => agent.chat(prompt, rig_history).await?,
        crate::api::RigAgentEnum::Anthropic(agent) => agent.chat(prompt, rig_history).await?,
        crate::api::RigAgentEnum::Gemini(agent) => agent.chat(prompt, rig_history).await?,
    };
    
    app.is_thinking = false;
    let response = strip_thinking(&response);
    if let Some(msg) = app.messages.last_mut() { msg.content = response.clone(); }
    app.agent.messages.push(Message { role: Role::Assistant, content: response, tool_calls: None, tool_results: None });
    
    app.agent.save_session().ok();
    Ok(())
}

async fn process_remaining_tools(_terminal: &mut Terminal<CrosstermBackend<Stdout>>, _app: &mut App) -> Result<()> {
    Ok(())
}

async fn execute_pending_tool(_terminal: &mut Terminal<CrosstermBackend<Stdout>>, _app: &mut App, _tool: ToolCall, _idx: usize) -> Result<()> {
    Ok(())
}

fn draw_ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(5), Constraint::Length(3), Constraint::Length(1)]).split(f.area());
    draw_messages(f, app, chunks[0]);
    draw_input(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);
    if app.command_palette_open { draw_command_palette(f, app, f.area()); }
    if app.model_picker_open { draw_model_picker(f, app, f.area()); }
    if let Some(ref pending) = app.pending_approval { draw_approval_dialog(f, pending, f.area()); }
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    if app.messages.is_empty() && !app.is_thinking { draw_welcome(f, app, area); return; }
    let mut lines = Vec::new();
    for msg in &app.messages {
        lines.push(Line::from(""));
        match msg.role {
            ChatRole::User => {
                lines.push(Line::from(Span::styled("You", Style::default().fg(BLUE).add_modifier(Modifier::BOLD))));
                for l in msg.content.lines() { lines.push(Line::from(l)); }
            }
            ChatRole::Assistant => {
                lines.push(Line::from(Span::styled("Forge", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))));
                for l in msg.content.lines() { lines.push(Line::from(l)); }
                for tool in &msg.tools {
                    let color = match tool.status { ToolState::Running => ORANGE, ToolState::Success => GREEN, ToolState::Failed => RED, _ => DIM };
                    lines.push(Line::from(vec![Span::styled("  ● ", Style::default().fg(color)), Span::styled(&tool.name, Style::default().fg(WHITE))]));
                }
            }
            ChatRole::System => { lines.push(Line::from(Span::styled(&msg.content, Style::default().fg(YELLOW).add_modifier(Modifier::ITALIC)))); }
        }
    }
    if app.is_thinking {
        lines.push(Line::from(""));
        let spinner_chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner = spinner_chars[app.thinking_count % spinner_chars.len()];
        lines.push(Line::from(vec![Span::styled(format!("Forge {} ", spinner), Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)), Span::styled("thinking...", Style::default().fg(DIM))]));
    }
    
    // Auto-scroll logic
    let content_height = lines.len();
    let visible_height = area.height as usize;
    let max_scroll = content_height.saturating_sub(visible_height);
    let scroll = if app.is_thinking { max_scroll } else { app.scroll.min(max_scroll) };

    let para = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll as u16, 0));
    f.render_widget(para, area);
}

fn draw_welcome(f: &mut Frame, _app: &App, area: Rect) {
    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" ███████╗ ██████╗ ██████╗  ██████╗ ███████╗", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(Span::styled(" ██╔════╝██╔═══██╗██╔══██╗██╔════╝ ██╔════╝", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(Span::styled(" █████╗  ██║   ██║██████╔╝██║  ███╗█████╗  ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(Span::styled(" ██╔══╝  ██║   ██║██╔══██╗██║   ██║██╔══╝  ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(Span::styled(" ██║     ╚██████╔╝██║  ██║╚██████╔╝███████╗", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" Terminal-first AI coding agent", Style::default().fg(DIM))));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" What would you like to build?", Style::default().fg(WHITE))));
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let placeholder = if app.input.is_empty() { " Type a message..." } else { "" };
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(app.mode_color())).title(Span::styled(placeholder, Style::default().fg(DIM)));
    let input = Paragraph::new(app.input.as_str()).style(Style::default().fg(WHITE)).block(block);
    f.render_widget(input, area);
    if !app.is_thinking && app.pending_approval.is_none() {
        f.set_cursor_position((area.x + app.input_cursor as u16 + 1, area.y + 1));
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let status = Line::from(vec![Span::styled(format!(" {} ", app.agent.config.model), Style::default().fg(ORANGE)), Span::styled("  •  ", Style::default().fg(DIM)), Span::styled(format!("{} mode", app.mode_text()), Style::default().fg(app.mode_color()).add_modifier(Modifier::BOLD)), Span::styled("  │  Ctrl+K: commands  │  Tab: toggle mode", Style::default().fg(DIM))]);
    f.render_widget(Paragraph::new(status), area);
}

fn draw_model_picker(f: &mut Frame, app: &App, area: Rect) {
    let models = app.agent.config.get_models(&app.agent.config.provider);
    let rect = centered_rect(45.min(area.width - 4), (models.len() as u16 + 2).min(area.height - 4), area);
    f.render_widget(Clear, rect);
    let items: Vec<ListItem> = models.iter().enumerate().map(|(i, m)| {
        let style = if i == app.model_picker_selected { Style::default().bg(ORANGE).fg(Color::Black) } else { Style::default().fg(WHITE) };
        ListItem::new(Line::from(Span::styled(format!("  {} ", m), style)))
    }).collect();
    f.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(" Models ")), rect);
}

fn draw_command_palette(f: &mut Frame, app: &App, area: Rect) {
    let rect = centered_rect(30.min(area.width - 4), (COMMANDS.len() as u16 + 2).min(area.height - 4), area);
    f.render_widget(Clear, rect);
    let items: Vec<ListItem> = COMMANDS.iter().enumerate().map(|(i, c)| {
        let style = if i == app.command_palette_selected { Style::default().bg(ORANGE).fg(Color::Black) } else { Style::default().fg(WHITE) };
        ListItem::new(Line::from(Span::styled(format!("  {} ", c), style)))
    }).collect();
    f.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(" Commands ")), rect);
}

fn draw_approval_dialog(f: &mut Frame, pending: &PendingApproval, area: Rect) {
    let rect = centered_rect(60, 7, area);
    f.render_widget(Clear, rect);
    let tool_info = format!("  Approve: {}?", pending.tool.name);
    let content = vec![Line::from(""), Line::from(Span::styled(tool_info, Style::default().fg(WHITE))), Line::from(""), Line::from(vec![Span::styled("  [", Style::default().fg(DIM)), Span::styled("y", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)), Span::styled("]es  [", Style::default().fg(DIM)), Span::styled("n", Style::default().fg(RED).add_modifier(Modifier::BOLD)), Span::styled("]o  [", Style::default().fg(DIM)), Span::styled("a", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)), Span::styled("]ll yes", Style::default().fg(DIM))])];
    f.render_widget(Paragraph::new(content).block(Block::default().borders(Borders::ALL).title(format!(" {} ", pending.tool.name))), rect);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    Rect::new((area.width - width) / 2, (area.height - height) / 2, width, height)
}

/// Strip thinking/reasoning blocks that thinking models (Qwen3, etc.) emit.
/// Handles both <think>...</think> and bare ...content...</think> (no opening tag).
fn strip_thinking(text: &str) -> String {
    // If </think> exists, everything before the last </think> is reasoning — discard it
    if let Some(end) = text.rfind("</think>") {
        return text[end + "</think>".len()..].trim().to_string();
    }
    // No closing tag — strip <think>...</think> blocks if present
    let mut result = String::new();
    let mut remaining = text;
    loop {
        match remaining.find("<think>") {
            None => { result.push_str(remaining); break; }
            Some(start) => {
                result.push_str(&remaining[..start]);
                match remaining[start..].find("</think>") {
                    None => break,
                    Some(rel_end) => {
                        remaining = &remaining[start + rel_end + "</think>".len()..];
                    }
                }
            }
        }
    }
    result.trim().to_string()
}
