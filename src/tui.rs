use ratatui::{
    backend::CrosstermBackend, layout::{Constraint, Direction, Layout},
    style::{Color, Style, Stylize}, text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph}, Terminal, Frame,
};
use crossterm::{
    event::{self, Event, KeyCode}, execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use crate::agent::Agent;
use crate::oauth::OAuthManager;
use crate::config::{ConfigManager, RaraConfig};

#[derive(PartialEq)] enum Screen { Chat, Setup }
pub struct TuiApp { input: String, history: Vec<(String, String)>, is_thinking: bool, screen: Screen, config: RaraConfig, config_manager: ConfigManager }
impl TuiApp {
    pub fn new(cm: ConfigManager) -> Self {
        let cfg = cm.load();
        let s = if cfg.api_key.is_none() && cfg.provider != "mock" { Screen::Setup } else { Screen::Chat };
        Self { input: String::new(), history: Vec::new(), is_thinking: false, screen: s, config: cfg, config_manager: cm }
    }
}

pub async fn run_tui(mut agent: Agent, oauth_manager: OAuthManager) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout(); execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let mut app = TuiApp::new(ConfigManager::new()?);
    let (cwd, branch) = agent.workspace.get_env_info();

    loop {
        terminal.draw(|f| { if app.screen == Screen::Setup { render_setup(f, &app); } else { render_chat(f, &app, &agent, &cwd, &branch); } })?;
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => { if app.screen == Screen::Setup { app.screen = Screen::Chat; } else { break; } }
                    KeyCode::Char('s') if app.screen == Screen::Chat => app.screen = Screen::Setup,
                    KeyCode::Char('l') if app.screen == Screen::Setup => {
                        let (v, ch) = oauth_manager.generate_pkce();
                        let (p, rx) = oauth_manager.start_callback_server().await?;
                        let _ = open::that(oauth_manager.get_authorize_url(&ch, p));
                        if let Ok(code) = rx.await {
                            if let Ok(token) = oauth_manager.exchange_code(&code, &v, p).await {
                                app.config.api_key = Some(token.access_token); 
                                app.config.provider = "codex_oauth".into();
                                let _ = app.config_manager.save(&app.config); 
                                app.screen = Screen::Chat;
                            }
                        }
                    }
                    KeyCode::Enter if app.screen == Screen::Chat && !app.input.is_empty() => {
                        let p = std::mem::take(&mut app.input); app.history.push(("User".into(), p.clone())); app.is_thinking = true;
                        let _ = agent.query(p).await;
                        app.history.clear();
                        for m in &agent.history {
                            let role = if m.role == "user" { "You" } else { "Agent" };
                            let txt = if m.content.is_array() { m.content.get(0).and_then(|b| b.get("text")).and_then(|v| v.as_str()).unwrap_or("") } else { m.content.as_str().unwrap_or("") };
                            if !txt.is_empty() { app.history.push((role.into(), txt.into())); }
                        }
                        app.is_thinking = false;
                    }
                    KeyCode::Char(c) => app.input.push(c),
                    KeyCode::Backspace => { app.input.pop(); },
                    _ => {}
                }
            }
        }
    }
    disable_raw_mode()?; execute!(terminal.backend_mut(), LeaveAlternateScreen)?; Ok(())
}

fn render_setup(f: &mut Frame, app: &TuiApp) {
    let block = Block::default().borders(Borders::ALL).title(" RARA Setup ");
    let text = format!(
        "Current Provider: {}\nAPI Key: {}\n\n[L] Login via Codex OAuth\n[Esc] Back to Chat", 
        app.config.provider.clone().bold().yellow(), 
        app.config.api_key.as_ref().map(|_| "****").unwrap_or("None").red()
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
