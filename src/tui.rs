use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use crossterm::{
    event::{self, Event, KeyCode}, execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use std::sync::Arc;

use crate::agent::Agent;
use crate::config::{ConfigManager, RaraConfig};
use crate::llm::LlmBackend;
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

#[derive(PartialEq)]
enum Screen {
    Chat,
    Setup,
}

const LOCAL_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("Gemma 4 E4B", "gemma4", "gemma4-e4b"),
    ("Gemma 4 E2B", "gemma4", "gemma4-e2b"),
    ("Qwn3 8B", "qwn3", "qwn3-8b"),
];

pub struct TuiApp {
    input: String,
    history: Vec<(String, String)>,
    is_thinking: bool,
    screen: Screen,
    config: RaraConfig,
    config_manager: ConfigManager,
    setup_status: Option<String>,
}

impl TuiApp {
    pub fn new(cm: ConfigManager) -> Self {
        let cfg = cm.load();
        let s = if cfg.api_key.is_none() && provider_requires_api_key(&cfg.provider) {
            Screen::Setup
        } else {
            Screen::Chat
        };
        Self {
            input: String::new(),
            history: Vec::new(),
            is_thinking: false,
            screen: s,
            config: cfg,
            config_manager: cm,
            setup_status: None,
        }
    }

    fn selected_preset_idx(&self) -> usize {
        LOCAL_MODEL_PRESETS
            .iter()
            .position(|(_, provider, model)| {
                self.config.provider == *provider && self.config.model.as_deref() == Some(*model)
            })
            .unwrap_or(0)
    }

    fn cycle_local_model(&mut self) {
        let next = (self.selected_preset_idx() + 1) % LOCAL_MODEL_PRESETS.len();
        self.select_local_model(next);
    }

    fn select_local_model(&mut self, idx: usize) {
        let (_, provider, model) = LOCAL_MODEL_PRESETS[idx];
        self.config.provider = provider.to_string();
        self.config.model = Some(model.to_string());
        self.config.revision = Some("main".to_string());
    }
}

pub async fn run_tui(mut agent: Agent, oauth_manager: OAuthManager) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let mut app = TuiApp::new(ConfigManager::new()?);

    loop {
        let (cwd, branch) = agent.workspace.get_env_info();
        terminal.draw(|f| {
            if app.screen == Screen::Setup {
                render_setup(f, &app);
            } else {
                render_chat(f, &app, &agent, &cwd, &branch);
            }
        })?;
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => {
                        if app.screen == Screen::Setup {
                            app.screen = Screen::Chat;
                        } else {
                            break;
                        }
                    }
                    KeyCode::Char('s') if app.screen == Screen::Chat => app.screen = Screen::Setup,
                    KeyCode::Char('1') if app.screen == Screen::Setup => app.select_local_model(0),
                    KeyCode::Char('2') if app.screen == Screen::Setup => app.select_local_model(1),
                    KeyCode::Char('3') if app.screen == Screen::Setup => app.select_local_model(2),
                    KeyCode::Char('m') if app.screen == Screen::Setup => app.cycle_local_model(),
                    KeyCode::Char('l') if app.screen == Screen::Setup => {
                        let (v, ch) = oauth_manager.generate_pkce();
                        let (p, rx) = oauth_manager.start_callback_server().await?;
                        let _ = open::that(oauth_manager.get_authorize_url(&ch, p));
                        if let Ok(code) = rx.await {
                            if let Ok(token) = oauth_manager.exchange_code(&code, &v, p).await {
                                app.config.api_key = Some(token.access_token);
                                app.config.provider = "codex_oauth".into();
                                let _ = app.config_manager.save(&app.config);
                                app.setup_status = Some("Saved Codex OAuth token".into());
                                app.screen = Screen::Chat;
                            }
                        }
                    }
                    KeyCode::Enter if app.screen == Screen::Setup => {
                        match rebuild_agent(&app.config).await {
                            Ok(new_agent) => {
                                app.config_manager.save(&app.config)?;
                                app.setup_status = Some(format!(
                                    "Applied {} / {}",
                                    app.config.provider,
                                    app.config.model.clone().unwrap_or_else(|| "-".into())
                                ));
                                app.history.clear();
                                agent = new_agent;
                                app.screen = Screen::Chat;
                            }
                            Err(err) => {
                                app.setup_status = Some(format!("Failed to apply config: {err}"));
                            }
                        }
                    }
                    KeyCode::Enter if app.screen == Screen::Chat && !app.input.is_empty() => {
                        let p = std::mem::take(&mut app.input);
                        app.history.push(("User".into(), p.clone()));
                        app.is_thinking = true;
                        let _ = agent.query(p).await;
                        app.history.clear();
                        for m in &agent.history {
                            let role = if m.role == "user" { "You" } else { "Agent" };
                            let txt = if m.content.is_array() {
                                m.content
                                    .get(0)
                                    .and_then(|b| b.get("text"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                            } else {
                                m.content.as_str().unwrap_or("")
                            };
                            if !txt.is_empty() {
                                app.history.push((role.into(), txt.into()));
                            }
                        }
                        app.is_thinking = false;
                    }
                    KeyCode::Char(c) => app.input.push(c),
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    _ => {}
                }
            }
        }
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn render_setup(f: &mut Frame, app: &TuiApp) {
    let block = Block::default().borders(Borders::ALL).title(" RARA Setup ");
    let current_model = app.config.model.as_deref().unwrap_or("None");
    let preset_lines = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let marker = if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                ">"
            } else {
                " "
            };
            format!("{marker} [{}] {label} ({provider} / {model})", idx + 1)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        "Current Provider: {}\nCurrent Model: {}\nAPI Key: {}\nRevision: {}\n\n\
         Local Model Presets:\n{}\n\n\
         [1/2/3] Select preset\n[M] Cycle local preset\n[Enter] Apply and reload backend\n[L] Login via Codex OAuth\n[Esc] Back to Chat{}\n",
        app.config.provider.clone().bold().yellow(),
        current_model.bold().cyan(),
        app.config.api_key.as_ref().map(|_| "****").unwrap_or("None").red(),
        app.config.revision.as_deref().unwrap_or("main"),
        preset_lines,
        app.setup_status
            .as_ref()
            .map(|status| format!("\nStatus: {status}"))
            .unwrap_or_default()
    );
    f.render_widget(Paragraph::new(text).block(block), f.area());
}

fn render_chat(f: &mut Frame, app: &TuiApp, agent: &Agent, cwd: &str, branch: &str) {
    let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(5), Constraint::Length(3), Constraint::Length(1)]).split(f.area());
    let header = Line::from(vec![Span::styled(" RARA ", Style::default().bg(Color::Cyan).bold()), Span::raw(format!(" | 📂 {} | 🌿 {}", cwd, branch))]);
    f.render_widget(Paragraph::new(header), chunks[0]);
    let items: Vec<ListItem> = app.history.iter().map(|(r, m)| ListItem::new(vec![Line::from(r.as_str().bold()), Line::from(m.as_str()), Line::from("")])).collect();
    f.render_widget(List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)), chunks[1]);
    let input_title = if app.is_thinking { "Thinking..." } else { "Input (S: Setup)" };
    f.render_widget(Paragraph::new(app.input.as_str()).block(Block::default().borders(Borders::ALL).title(input_title)), chunks[2]);
    let status = format!(" Tokens: {} in / {} out ", agent.total_input_tokens, agent.total_output_tokens);
    f.render_widget(Paragraph::new(status), chunks[3]);
}

async fn rebuild_agent(config: &RaraConfig) -> anyhow::Result<Agent> {
    let backend = crate::build_backend(config).await;
    let backend_arc: Arc<dyn LlmBackend> = match backend {
        Ok(backend) => backend.into(),
        Err(err) => {
            return Err(err);
        }
    };

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
