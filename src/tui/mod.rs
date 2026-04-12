mod command;
mod render;
mod runtime;
mod state;

use std::io;
use std::sync::Arc;

use crossterm::{
    cursor::{Hide, Show},
    event::{Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::time::{interval, Duration};

use crate::agent::Agent;
use crate::oauth::OAuthManager;

use self::command::parse_local_command;
use self::render::{render_chat, render_model_picker, render_setup};
use self::runtime::{
    execute_local_command, finish_running_task_if_ready, start_oauth_task, start_query_task,
    start_rebuild_task,
};
use self::state::{Screen, TuiApp, LOCAL_MODEL_PRESETS};

pub async fn run_tui(agent: Agent, oauth_manager: OAuthManager) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let mut app = TuiApp::new(crate::config::ConfigManager::new()?);
    let mut agent_slot = Some(agent);
    let oauth_manager = Arc::new(oauth_manager);
    let mut events = EventStream::new();
    let mut tick = interval(Duration::from_millis(100));

    if let Some(agent_ref) = agent_slot.as_ref() {
        app.sync_snapshot(agent_ref);
    }

    let result = loop {
        finish_running_task_if_ready(&mut app, &mut agent_slot).await?;

        terminal.draw(|f| match app.screen {
            Screen::Chat => render_chat(f, &app),
            Screen::Setup => render_setup(f, &app),
            Screen::ModelPicker => render_model_picker(f, &app),
        })?;

        tokio::select! {
            _ = tick.tick() => {}
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        let should_exit = match app.screen {
                            Screen::Chat => {
                                handle_chat_key(
                                    key.code,
                                    &mut app,
                                    &mut agent_slot,
                                    &oauth_manager,
                                )
                                .await?
                            }
                            Screen::Setup => handle_setup_key(
                                key.code,
                                &mut app,
                                &mut agent_slot,
                                &oauth_manager,
                            )
                            .await?,
                            Screen::ModelPicker => {
                                handle_model_picker_key(key.code, &mut app).await?
                            }
                        };
                        if should_exit {
                            break Ok(());
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => break Err(err.into()),
                    None => break Ok(()),
                }
            }
        }
    };

    teardown_terminal(terminal)?;
    result
}

async fn handle_chat_key(
    key: KeyCode,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<bool> {
    match key {
        KeyCode::Esc => return Ok(true),
        KeyCode::Char('s') => app.screen = Screen::Setup,
        KeyCode::Enter if !app.input.trim().is_empty() => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
                return Ok(false);
            }
            let input = std::mem::take(&mut app.input);
            if let Some(command) = parse_local_command(&input) {
                execute_local_command(command, app, agent_slot, oauth_manager).await?;
            } else if input.trim_start().starts_with('/') {
                app.push_notice(format!("Unknown command '{}'. Use /help.", input.trim()));
            } else if let Some(agent) = agent_slot.take() {
                start_query_task(app, input.trim().to_string(), agent);
            }
        }
        KeyCode::Char(c) => app.input.push(c),
        KeyCode::Backspace => {
            app.input.pop();
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_setup_key(
    key: KeyCode,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<bool> {
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
                start_oauth_task(app, Arc::clone(oauth_manager));
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
    Ok(false)
}

async fn handle_model_picker_key(key: KeyCode, app: &mut TuiApp) -> anyhow::Result<bool> {
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
    Ok(false)
}

fn teardown_terminal(
    mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), Show, LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

pub(crate) fn provider_requires_api_key(provider: &str) -> bool {
    !matches!(
        provider,
        "mock" | "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3"
    )
}
