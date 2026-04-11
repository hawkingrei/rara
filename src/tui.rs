use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::task::JoinHandle;

use crate::agent::{Agent, AgentEvent, AgentOutputMode};
use crate::config::{ConfigManager, RaraConfig};
use crate::llm::LlmBackend;
use crate::local_backend::{default_local_model_cache_dir, LocalProgressReporter};
use crate::oauth::OAuthManager;
use crate::sandbox::SandboxManager;
use crate::session::SessionManager;
use crate::skill::SkillManager;
use crate::tool::ToolManager;
use crate::tools::agent::{AgentTool, TeamCreateTool};
use crate::tools::bash::BashTool;
use crate::tools::context::RetrieveSessionContextTool;
use crate::tools::file::{
    ListFilesTool, ReadFileTool, ReplaceTool, SearchFilesTool, WriteFileTool,
};
use crate::tools::search::{GlobTool, GrepTool};
use crate::tools::skill::SkillTool;
use crate::tools::vector::{RememberExperienceTool, RetrieveExperienceTool};
use crate::tools::web::WebFetchTool;
use crate::tools::workspace::UpdateProjectMemoryTool;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Chat,
    Setup,
    ModelPicker,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LocalCommandKind {
    Help,
    Status,
    Clear,
    Setup,
    Model,
    Login,
}

struct LocalCommand {
    kind: LocalCommandKind,
    arg: Option<String>,
}

struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    summary: &'static str,
}

#[derive(Default, Clone)]
struct RuntimeSnapshot {
    cwd: String,
    branch: String,
    session_id: String,
    history_len: usize,
    total_input_tokens: u32,
    total_output_tokens: u32,
}

enum TaskKind {
    Query,
    Rebuild,
}

enum TaskCompletion {
    Query {
        agent: Agent,
        result: anyhow::Result<()>,
    },
    Rebuild {
        result: anyhow::Result<Agent>,
    },
}

enum TuiEvent {
    Transcript {
        role: &'static str,
        message: String,
    },
}

struct RunningTask {
    kind: TaskKind,
    receiver: UnboundedReceiver<TuiEvent>,
    handle: JoinHandle<TaskCompletion>,
}

const COMMAND_SPECS: [CommandSpec; 6] = [
    CommandSpec {
        name: "help",
        usage: "/help",
        summary: "Show built-in commands and keyboard hints.",
    },
    CommandSpec {
        name: "status",
        usage: "/status",
        summary: "Show current provider, model, revision, workspace, and runtime counters.",
    },
    CommandSpec {
        name: "clear",
        usage: "/clear",
        summary: "Clear the visible transcript and keep the current backend.",
    },
    CommandSpec {
        name: "setup",
        usage: "/setup",
        summary: "Open the fallback setup screen.",
    },
    CommandSpec {
        name: "model",
        usage: "/model [name|1|2|3|next|list]",
        summary: "Open the picker or switch local model presets in place.",
    },
    CommandSpec {
        name: "login",
        usage: "/login",
        summary: "Run OAuth login for hosted Codex mode.",
    },
];

const LOCAL_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("Gemma 4 E4B", "gemma4", "gemma4-e4b"),
    ("Gemma 4 E2B", "gemma4", "gemma4-e2b"),
    ("Qwn3 8B", "qwn3", "qwn3-8b"),
];

pub struct TuiApp {
    input: String,
    transcript: Vec<(String, String)>,
    screen: Screen,
    config: RaraConfig,
    config_manager: ConfigManager,
    setup_status: Option<String>,
    notice: Option<String>,
    snapshot: RuntimeSnapshot,
    model_picker_idx: usize,
    running_task: Option<RunningTask>,
}

impl TuiApp {
    pub fn new(cm: ConfigManager) -> Self {
        let cfg = cm.load();
        let screen = if cfg.api_key.is_none() && provider_requires_api_key(&cfg.provider) {
            Screen::Setup
        } else {
            Screen::Chat
        };
        let model_picker_idx = selected_preset_idx_for_config(&cfg);
        Self {
            input: String::new(),
            transcript: Vec::new(),
            screen,
            config: cfg,
            config_manager: cm,
            setup_status: None,
            notice: None,
            snapshot: RuntimeSnapshot::default(),
            model_picker_idx,
            running_task: None,
        }
    }

    fn is_busy(&self) -> bool {
        self.running_task.is_some()
    }

    fn current_model_label(&self) -> &str {
        self.config.model.as_deref().unwrap_or("-")
    }

    fn selected_preset_idx(&self) -> usize {
        selected_preset_idx_for_config(&self.config)
    }

    fn select_local_model(&mut self, idx: usize) {
        let (_, provider, model) = LOCAL_MODEL_PRESETS[idx];
        self.model_picker_idx = idx;
        self.config.provider = provider.to_string();
        self.config.model = Some(model.to_string());
        self.config.revision = Some("main".to_string());
    }

    fn cycle_local_model(&mut self) {
        let next = (self.selected_preset_idx() + 1) % LOCAL_MODEL_PRESETS.len();
        self.select_local_model(next);
    }

    fn sync_snapshot(&mut self, agent: &Agent) {
        let (cwd, branch) = agent.workspace.get_env_info();
        self.snapshot = RuntimeSnapshot {
            cwd,
            branch,
            session_id: agent.session_id.clone(),
            history_len: agent.history.len(),
            total_input_tokens: agent.total_input_tokens,
            total_output_tokens: agent.total_output_tokens,
        };
    }

    fn push_entry(&mut self, role: &'static str, message: impl Into<String>) {
        self.transcript.push((role.into(), message.into()));
    }

    fn push_notice(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.notice = Some(message.clone());
        self.push_entry("System", message);
    }

    fn reset_transcript(&mut self) {
        self.transcript.clear();
        self.notice = Some("Cleared local transcript view.".into());
    }
}

pub async fn run_tui(agent: Agent, oauth_manager: OAuthManager) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let mut app = TuiApp::new(ConfigManager::new()?);
    let mut agent_slot = Some(agent);
    if let Some(agent_ref) = agent_slot.as_ref() {
        app.sync_snapshot(agent_ref);
    }

    loop {
        finish_running_task_if_ready(&mut app, &mut agent_slot).await?;

        terminal.draw(|f| match app.screen {
            Screen::Chat => render_chat(f, &app),
            Screen::Setup => render_setup(f, &app),
            Screen::ModelPicker => render_model_picker(f, &app),
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.screen {
                    Screen::Chat => {
                        handle_chat_key(key.code, &mut app, &mut agent_slot, &oauth_manager).await?;
                    }
                    Screen::Setup => {
                        handle_setup_key(key.code, &mut app, &mut agent_slot, &oauth_manager).await?;
                    }
                    Screen::ModelPicker => {
                        handle_model_picker_key(key.code, &mut app, &mut agent_slot).await?;
                    }
                }
            }
        }
    }
}

async fn handle_chat_key(
    key: KeyCode,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &OAuthManager,
) -> anyhow::Result<()> {
    match key {
        KeyCode::Esc => {
            teardown_terminal()?;
            std::process::exit(0);
        }
        KeyCode::Char('s') => app.screen = Screen::Setup,
        KeyCode::Enter if !app.input.trim().is_empty() => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
                return Ok(());
            }
            let input = std::mem::take(&mut app.input);
            if let Some(command) = parse_local_command(&input) {
                execute_local_command(command, app, agent_slot, oauth_manager).await?;
            } else if input.trim_start().starts_with('/') {
                app.push_notice(format!("Unknown command '{}'. Use /help.", input.trim()));
            } else if let Some(agent) = agent_slot.take() {
                start_query_task(app, prompt_display(input.trim()), agent);
            }
        }
        KeyCode::Char(c) => app.input.push(c),
        KeyCode::Backspace => {
            app.input.pop();
        }
        _ => {}
    }
    Ok(())
}

async fn handle_setup_key(
    key: KeyCode,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &OAuthManager,
) -> anyhow::Result<()> {
    match key {
        KeyCode::Esc => app.screen = Screen::Chat,
        KeyCode::Char('1') => app.select_local_model(0),
        KeyCode::Char('2') => app.select_local_model(1),
        KeyCode::Char('3') => app.select_local_model(2),
        KeyCode::Char('m') => app.cycle_local_model(),
        KeyCode::Char('l') => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before starting login.");
            } else {
                run_oauth_login(app, oauth_manager).await?;
            }
        }
        KeyCode::Enter => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                app.screen = Screen::Chat;
                start_rebuild_task(app);
            }
        }
        _ => {}
    }
    if let Some(agent) = agent_slot.as_ref() {
        app.sync_snapshot(agent);
    }
    Ok(())
}

async fn handle_model_picker_key(
    key: KeyCode,
    app: &mut TuiApp,
    _agent_slot: &mut Option<Agent>,
) -> anyhow::Result<()> {
    match key {
        KeyCode::Esc => app.screen = Screen::Chat,
        KeyCode::Up | KeyCode::Char('k') => {
            app.model_picker_idx = app.model_picker_idx.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.model_picker_idx = (app.model_picker_idx + 1).min(LOCAL_MODEL_PRESETS.len() - 1);
        }
        KeyCode::Char('1') => app.model_picker_idx = 0,
        KeyCode::Char('2') => app.model_picker_idx = 1,
        KeyCode::Char('3') => app.model_picker_idx = 2,
        KeyCode::Enter => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                app.select_local_model(app.model_picker_idx);
                app.screen = Screen::Chat;
                start_rebuild_task(app);
            }
        }
        _ => {}
    }
    Ok(())
}

async fn execute_local_command(
    command: LocalCommand,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &OAuthManager,
) -> anyhow::Result<()> {
    match command.kind {
        LocalCommandKind::Help => app.push_notice(help_text()),
        LocalCommandKind::Status => app.push_notice(status_text(app)),
        LocalCommandKind::Clear => app.reset_transcript(),
        LocalCommandKind::Setup => app.screen = Screen::Setup,
        LocalCommandKind::Model => handle_model_command(command.arg.as_deref(), app).await?,
        LocalCommandKind::Login => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                run_oauth_login(app, oauth_manager).await?;
            }
        }
    }
    if let Some(agent) = agent_slot.as_ref() {
        app.sync_snapshot(agent);
    }
    Ok(())
}

async fn handle_model_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    let Some(raw_arg) = arg.map(str::trim).filter(|arg| !arg.is_empty()) else {
        app.model_picker_idx = app.selected_preset_idx();
        app.screen = Screen::ModelPicker;
        app.notice = Some("Opened model picker.".into());
        return Ok(());
    };

    let selected_idx = match raw_arg {
        "1" => Some(0),
        "2" => Some(1),
        "3" => Some(2),
        "list" => {
            app.push_notice(model_help_text(app));
            return Ok(());
        }
        "next" => Some((app.selected_preset_idx() + 1) % LOCAL_MODEL_PRESETS.len()),
        _ => LOCAL_MODEL_PRESETS.iter().position(|(label, provider, model)| {
            *model == raw_arg
                || *provider == raw_arg
                || normalize_command_token(label) == normalize_command_token(raw_arg)
                || (*model == "qwn3-8b" && raw_arg.eq_ignore_ascii_case("qwen3-8b"))
                || (*model == "gemma4-e4b" && raw_arg.eq_ignore_ascii_case("gemma-4-e4b"))
                || (*model == "gemma4-e2b" && raw_arg.eq_ignore_ascii_case("gemma-4-e2b"))
        }),
    };

    let Some(idx) = selected_idx else {
        app.push_notice(format!("Unknown model preset '{raw_arg}'. Try /model or /help."));
        return Ok(());
    };

    app.select_local_model(idx);
    start_rebuild_task(app);
    Ok(())
}

fn start_query_task(app: &mut TuiApp, prompt: String, mut agent: Agent) {
    let (sender, receiver) = mpsc::unbounded_channel();
    app.notice = Some("Running prompt.".into());
    app.push_entry("You", prompt.clone());

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let result = agent
            .query_with_mode_and_events(prompt, AgentOutputMode::Silent, move |event| {
                let _ = tx.send(convert_agent_event(event));
            })
            .await;
        TaskCompletion::Query { agent, result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Query,
        receiver,
        handle,
    });
}

fn start_rebuild_task(app: &mut TuiApp) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let config = app.config.clone();
    let provider = config.provider.clone();
    let model = config.model.clone().unwrap_or_else(|| "-".to_string());
    app.notice = Some(format!("Rebuilding backend for {provider} / {model}."));
    app.push_entry("Runtime", format!("Reloading backend for {provider} / {model}."));

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let progress: LocalProgressReporter = Arc::new(move |message| {
            let _ = tx.send(TuiEvent::Transcript {
                role: "Runtime",
                message,
            });
        });
        let result = rebuild_agent_with_progress(&config, Some(progress)).await;
        TaskCompletion::Rebuild { result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Rebuild,
        receiver,
        handle,
    });
}

async fn finish_running_task_if_ready(
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> anyhow::Result<()> {
    if app.running_task.is_none() {
        return Ok(());
    }

    let (pending_events, is_finished) = {
        let task = app.running_task.as_mut().expect("task should exist");
        let mut pending_events = Vec::new();
        while let Ok(event) = task.receiver.try_recv() {
            pending_events.push(event);
        }
        let is_finished = task.handle.is_finished();
        (pending_events, is_finished)
    };

    for event in pending_events {
        apply_tui_event(app, event);
    }

    if !is_finished {
        return Ok(());
    }

    let task = app.running_task.take().expect("task should exist");
    let completion = task.handle.await?;
    match completion {
        TaskCompletion::Query { agent, result } => {
            *agent_slot = Some(agent);
            if let Some(agent) = agent_slot.as_ref() {
                app.sync_snapshot(agent);
            }
            match result {
                Ok(_) => {
                    app.notice = Some("Prompt finished.".into());
                }
                Err(err) => {
                    app.push_notice(format!("Query failed: {err}"));
                }
            }
        }
        TaskCompletion::Rebuild { result } => match result {
            Ok(agent) => {
                app.config_manager.save(&app.config)?;
                app.setup_status = Some(format!(
                    "Applied {} / {}",
                    app.config.provider,
                    app.current_model_label()
                ));
                app.notice = app.setup_status.clone();
                app.transcript.clear();
                *agent_slot = Some(agent);
                if let Some(agent) = agent_slot.as_ref() {
                    app.sync_snapshot(agent);
                }
                app.push_entry("Runtime", app.setup_status.clone().unwrap_or_default());
            }
            Err(err) => {
                let message = format!("Failed to apply config: {err}");
                app.setup_status = Some(message.clone());
                app.push_notice(message);
            }
        },
    }

    Ok(())
}

fn apply_tui_event(app: &mut TuiApp, event: TuiEvent) {
    match event {
        TuiEvent::Transcript { role, message } => app.push_entry(role, message),
    }
}

fn convert_agent_event(event: AgentEvent) -> TuiEvent {
    match event {
        AgentEvent::Status(message) => TuiEvent::Transcript {
            role: "Status",
            message,
        },
        AgentEvent::AssistantText(text) => TuiEvent::Transcript {
            role: "Agent",
            message: text,
        },
        AgentEvent::ToolUse { name, input } => TuiEvent::Transcript {
            role: "Tool",
            message: format!("{name} {input}"),
        },
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
        } => TuiEvent::Transcript {
            role: if is_error { "Tool Error" } else { "Tool Result" },
            message: format!("{name}: {content}"),
        },
    }
}

async fn run_oauth_login(app: &mut TuiApp, oauth_manager: &OAuthManager) -> anyhow::Result<()> {
    let (verifier, challenge) = oauth_manager.generate_pkce();
    let (port, receiver) = oauth_manager.start_callback_server().await?;
    let _ = open::that(oauth_manager.get_authorize_url(&challenge, port));

    match receiver.await {
        Ok(code) => match oauth_manager.exchange_code(&code, &verifier, port).await {
            Ok(token) => {
                app.config.api_key = Some(token.access_token);
                app.config.provider = "codex_oauth".into();
                app.config_manager.save(&app.config)?;
                app.setup_status = Some("Saved OAuth token.".into());
                app.notice = Some("Saved OAuth token.".into());
                app.screen = Screen::Chat;
            }
            Err(err) => app.push_notice(format!("OAuth exchange failed: {err}")),
        },
        Err(err) => app.push_notice(format!("OAuth callback failed: {err}")),
    }

    Ok(())
}

fn render_setup(f: &mut Frame, app: &TuiApp) {
    let block = Block::default().borders(Borders::ALL).title(" RARA Setup ");
    let preset_lines = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let marker =
                if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                    ">"
                } else {
                    " "
                };
            format!("{marker} [{}] {label} ({provider} / {model})", idx + 1)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let status_suffix = app
        .setup_status
        .as_ref()
        .map(|status| format!("\nStatus: {status}"))
        .unwrap_or_default();
    let busy_suffix = if app.is_busy() {
        "\nA background task is running. Setup changes will apply when it finishes."
    } else {
        ""
    };
    let text = format!(
        "Provider: {}\nModel: {}\nAPI key: {}\nRevision: {}\n\n\
         Presets:\n{}\n\n\
         [1/2/3] Select preset\n[M] Cycle preset\n[Enter] Apply and rebuild\n[L] OAuth login\n[Esc] Back to chat{}\n{}\n",
        app.config.provider.clone().bold().yellow(),
        app.current_model_label().bold().cyan(),
        api_key_status(&app.config),
        app.config.revision.as_deref().unwrap_or("main"),
        preset_lines,
        status_suffix,
        busy_suffix,
    );
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false }),
        f.area(),
    );
}

fn render_model_picker(f: &mut Frame, app: &TuiApp) {
    let items = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let prefix = if idx == app.model_picker_idx { ">" } else { " " };
            let current =
                if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                    " current"
                } else {
                    ""
                };
            ListItem::new(format!(
                "{prefix} [{}] {} ({}/{}){}",
                idx + 1,
                label,
                provider,
                model,
                current
            ))
        })
        .collect::<Vec<_>>();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(3)])
        .split(f.area());
    let header = Paragraph::new("Select a local model preset. Enter applies it immediately.")
        .block(Block::default().borders(Borders::ALL).title(" Model Picker "));
    let footer = Paragraph::new("Use Up/Down or j/k. Enter apply. Esc cancel.")
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, layout[0]);
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL)),
        layout[1],
    );
    f.render_widget(footer, layout[2]);
}

fn render_chat(f: &mut Frame, app: &TuiApp) {
    let show_command_menu = app.input.trim_start().starts_with('/');
    let command_height = if show_command_menu { 6 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(command_height),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);

    if app.transcript.is_empty() {
        f.render_widget(
            Paragraph::new(onboarding_text(app))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Session "))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
    } else {
        let items = app
            .transcript
            .iter()
            .map(|(role, message)| {
                ListItem::new(vec![
                    Line::from(role.as_str().bold()),
                    Line::from(message.as_str()),
                    Line::from(""),
                ])
            })
            .collect::<Vec<_>>();
        f.render_widget(
            List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
            chunks[1],
        );
    }

    if show_command_menu {
        render_command_menu(f, app, chunks[2]);
    }

    let input_title = if app.is_busy() {
        " Running "
    } else {
        " Prompt (/help for commands) "
    };
    let input_index = if show_command_menu { 3 } else { 2 };
    let footer_index = if show_command_menu { 4 } else { 3 };
    f.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(input_title)),
        chunks[input_index],
    );

    let mode = if provider_requires_api_key(&app.config.provider) {
        "hosted"
    } else {
        "local"
    };
    let activity = match app.running_task.as_ref().map(|task| &task.kind) {
        Some(TaskKind::Query) => "query",
        Some(TaskKind::Rebuild) => "reload",
        None => "idle",
    };
    let status = format!(
        "state={}  mode={}  key={}  messages={}  tokens={} in / {} out{}",
        activity,
        mode,
        api_key_status(&app.config),
        app.snapshot.history_len,
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        app.notice
            .as_ref()
            .map(|notice| format!("  notice={notice}"))
            .unwrap_or_default()
    );
    f.render_widget(Paragraph::new(status), chunks[footer_index]);
}

fn render_header(f: &mut Frame, app: &TuiApp, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled(" RARA ", Style::default().bg(Color::Cyan).fg(Color::Black).bold()),
            Span::raw(format!(
                "  provider={}  model={}  revision={}  key={}",
                app.config.provider,
                app.current_model_label(),
                app.config.revision.as_deref().unwrap_or("main"),
                api_key_status(&app.config),
            )),
        ]),
        Line::from(format!(
            " workspace={}  branch={}  session={} ",
            app.snapshot.cwd, app.snapshot.branch, app.snapshot.session_id
        )),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_command_menu(f: &mut Frame, app: &TuiApp, area: Rect) {
    let query = app.input.trim_start().trim_start_matches('/');
    let items = matching_commands(query)
        .into_iter()
        .map(|spec| ListItem::new(format!("{}  {}", spec.usage, spec.summary)))
        .collect::<Vec<_>>();
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" Commands ")),
        area,
    );
}

fn onboarding_text(app: &TuiApp) -> String {
    let local_model_help = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, _, model))| format!("  {}. {} ({})", idx + 1, label, model))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "This TUI uses chat as the primary control surface.\n\n\
         Start here:\n\
           - Ask for a task directly.\n\
           - Use /help for built-in commands.\n\
           - Use /model to open the model picker or switch presets.\n\
           - Use /status to inspect runtime state.\n\
           - Use /clear to reset the local transcript view.\n\n\
         Local presets:\n{}\n\n\
         Current runtime:\n\
           provider={}\n\
           model={}\n\
           revision={}\n\
           workspace={}\n\
           branch={}\n",
        local_model_help,
        app.config.provider,
        app.current_model_label(),
        app.config.revision.as_deref().unwrap_or("main"),
        app.snapshot.cwd,
        app.snapshot.branch,
    )
}

fn help_text() -> String {
    let commands = COMMAND_SPECS
        .iter()
        .map(|spec| format!("  {}  {}", spec.usage, spec.summary))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Built-in commands:\n{}\n\nKeyboard:\n  Enter submit\n  Esc quit or leave current panel\n  S open setup\n\nModel switching examples:\n  /model\n  /model list\n  /model 2\n  /model qwen3-8b\n  /model next",
        commands
    )
}

fn status_text(app: &TuiApp) -> String {
    let local_cache = if is_local_provider(&app.config.provider) {
        format!("\ncache={}", default_local_model_cache_dir().display())
    } else {
        String::new()
    };
    format!(
        "provider={}\nmodel={}\nrevision={}\nworkspace={}\nbranch={}\nsession={}\nmessages={}\ntranscript={}\napi_key={}\ntokens={} in / {} out{}",
        app.config.provider,
        app.current_model_label(),
        app.config.revision.as_deref().unwrap_or("main"),
        app.snapshot.cwd,
        app.snapshot.branch,
        app.snapshot.session_id,
        app.snapshot.history_len,
        app.transcript.len(),
        api_key_status(&app.config),
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        local_cache,
    )
}

fn model_help_text(app: &TuiApp) -> String {
    let lines = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let marker =
                if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                    "*"
                } else {
                    " "
                };
            format!("{marker} {}. {} ({}/{})", idx + 1, label, provider, model)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Current model: {} / {}\n\nAvailable presets:\n{}\n\nUse /model to open the picker, or /model <name>, /model <1|2|3>, /model list, /model next.",
        app.config.provider,
        app.current_model_label(),
        lines
    )
}

fn matching_commands(query: &str) -> Vec<&'static CommandSpec> {
    let query = query.trim();
    let mut matches = COMMAND_SPECS
        .iter()
        .filter(|spec| query.is_empty() || spec.name.starts_with(query))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        matches = COMMAND_SPECS.iter().collect();
    }
    matches
}

fn parse_local_command(input: &str) -> Option<LocalCommand> {
    let trimmed = input.trim();
    let command = trimmed.strip_prefix('/')?;
    let mut parts = command.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    let arg = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let kind = match name {
        "help" => LocalCommandKind::Help,
        "status" => LocalCommandKind::Status,
        "clear" => LocalCommandKind::Clear,
        "setup" => LocalCommandKind::Setup,
        "model" => LocalCommandKind::Model,
        "login" => LocalCommandKind::Login,
        _ => return None,
    };

    Some(LocalCommand { kind, arg })
}

fn api_key_status(config: &RaraConfig) -> &'static str {
    if !provider_requires_api_key(&config.provider) {
        "not-required"
    } else if config.api_key.as_ref().is_some() {
        "configured"
    } else {
        "missing"
    }
}

fn is_local_provider(provider: &str) -> bool {
    matches!(provider, "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3")
}

fn normalize_command_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn selected_preset_idx_for_config(config: &RaraConfig) -> usize {
    LOCAL_MODEL_PRESETS
        .iter()
        .position(|(_, provider, model)| {
            config.provider == *provider && config.model.as_deref() == Some(*model)
        })
        .unwrap_or(0)
}

fn prompt_display(input: &str) -> String {
    input.trim().to_string()
}

async fn rebuild_agent_with_progress(
    config: &RaraConfig,
    progress: Option<LocalProgressReporter>,
) -> anyhow::Result<Agent> {
    let backend = crate::build_backend_with_progress(config, progress).await?;
    let backend_arc: Arc<dyn LlmBackend> = backend.into();

    let vdb = Arc::new(VectorDB::new("data/lancedb"));
    let session_manager = Arc::new(SessionManager::new()?);
    let workspace = Arc::new(WorkspaceMemory::new()?);
    let sandbox_manager = Arc::new(SandboxManager::new()?);

    let mut skill_manager = SkillManager::new();
    let _ = skill_manager.load_all();
    let skill_manager_arc = Arc::new(skill_manager);

    let tool_manager = create_full_tool_manager(
        backend_arc.clone(),
        vdb.clone(),
        session_manager.clone(),
        workspace.clone(),
        sandbox_manager.clone(),
        skill_manager_arc,
    );

    Ok(Agent::new(
        tool_manager,
        backend_arc,
        vdb,
        session_manager,
        workspace,
    ))
}

fn provider_requires_api_key(provider: &str) -> bool {
    !matches!(
        provider,
        "mock" | "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3"
    )
}

fn create_full_tool_manager(
    backend: Arc<dyn LlmBackend>,
    vdb: Arc<VectorDB>,
    session_manager: Arc<SessionManager>,
    workspace: Arc<WorkspaceMemory>,
    sandbox: Arc<SandboxManager>,
    skill_manager: Arc<SkillManager>,
) -> ToolManager {
    let mut tm = ToolManager::new();
    tm.register(Box::new(BashTool {
        sandbox: sandbox.clone(),
    }));
    tm.register(Box::new(ReadFileTool));
    tm.register(Box::new(WriteFileTool));
    tm.register(Box::new(ListFilesTool));
    tm.register(Box::new(SearchFilesTool));
    tm.register(Box::new(ReplaceTool));
    tm.register(Box::new(WebFetchTool));
    tm.register(Box::new(GlobTool));
    tm.register(Box::new(GrepTool));
    tm.register(Box::new(RememberExperienceTool {
        backend: backend.clone(),
        db_uri: "data/lancedb".into(),
    }));
    tm.register(Box::new(RetrieveExperienceTool {
        backend: backend.clone(),
        db_uri: "data/lancedb".into(),
    }));
    tm.register(Box::new(RetrieveSessionContextTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
    }));
    tm.register(Box::new(UpdateProjectMemoryTool {
        workspace: workspace.clone(),
    }));
    tm.register(Box::new(SkillTool {
        skill_manager: skill_manager.clone(),
    }));
    tm.register(Box::new(AgentTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
    }));
    tm.register(Box::new(TeamCreateTool {
        backend,
        vdb,
        session_manager,
        workspace,
    }));
    tm
}

fn teardown_terminal() -> anyhow::Result<()> {
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, LeaveAlternateScreen)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{matching_commands, normalize_command_token, parse_local_command, LocalCommandKind};

    #[test]
    fn parses_model_command_argument() {
        let command = parse_local_command("/model qwen3-8b").expect("command should parse");
        assert!(matches!(command.kind, LocalCommandKind::Model));
        assert_eq!(command.arg.as_deref(), Some("qwen3-8b"));
    }

    #[test]
    fn returns_none_for_unknown_command() {
        assert!(parse_local_command("/unknown").is_none());
    }

    #[test]
    fn matches_commands_by_prefix() {
        let names = matching_commands("st")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["status"]);
    }

    #[test]
    fn normalizes_model_labels_for_command_matching() {
        assert_eq!(normalize_command_token("Gemma 4 E4B"), "gemma4e4b");
        assert_eq!(normalize_command_token("Qwn3 8B"), "qwn38b");
    }
}
