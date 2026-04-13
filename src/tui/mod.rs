mod app_event;
mod command;
mod event_stream;
mod render;
mod runtime;
mod state;

use std::io;
use std::sync::Arc;

use crossterm::{
    cursor::{Hide, Show},
    event::{EventStream, KeyCode},
    execute,
    terminal::size as terminal_size,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    widgets::{Paragraph, Widget},
    Terminal, TerminalOptions, Viewport,
};
use tokio::time::{interval, Duration};

use crate::agent::Agent;
use crate::oauth::OAuthManager;

use self::app_event::AppEvent;
use self::command::{palette_command_by_index, palette_commands, parse_local_command};
use self::event_stream::{translate_event, UiEvent};
use self::render::{committed_turn_lines, render};
use self::runtime::{
    execute_local_command, finish_running_task_if_ready, start_oauth_task, start_pending_approval_task, start_query_task,
    start_rebuild_task,
};
use self::state::{current_model_presets, HelpTab, Overlay, PROVIDER_FAMILIES, TuiApp};
use crate::agent::BashApprovalMode;

pub async fn run_tui(agent: Agent, oauth_manager: OAuthManager) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, Hide)?;
    let mut terminal = build_terminal()?;
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
        clamp_command_palette_selection(&mut app);
        flush_committed_history(&mut terminal, &mut app)?;
        terminal.draw(|f| render(f, &app))?;

        tokio::select! {
            _ = tick.tick() => {}
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => match translate_event(event, &app) {
                        Some(UiEvent::App(event)) => {
                            if dispatch_event(event, &mut app, &mut agent_slot, &oauth_manager).await? {
                                break Ok(());
                            }
                        }
                        Some(UiEvent::Draw) => {
                            terminal = build_terminal()?;
                        }
                        Some(UiEvent::Paste(text)) => {
                            handle_paste(text, &mut app);
                        }
                        Some(UiEvent::FocusChanged(focused)) => {
                            app.terminal_focused = focused;
                        }
                        None => {}
                    },
                    Some(Err(err)) => break Err(err.into()),
                    None => break Ok(()),
                }
            }
        }
    };

    teardown_terminal(terminal)?;
    result
}

fn handle_paste(text: String, app: &mut TuiApp) {
    if matches!(app.overlay, Some(Overlay::BaseUrlEditor)) {
        app.base_url_input.push_str(&text);
        return;
    }

    app.input.push_str(&text);
    if app.input.trim_start().starts_with('/') {
        app.open_overlay(Overlay::CommandPalette);
    } else if matches!(app.overlay, Some(Overlay::CommandPalette)) {
        app.close_overlay();
    }
}

fn build_terminal() -> anyhow::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    let (_, rows) = terminal_size()?;
    let viewport_height = rows.saturating_sub(1).max(14);
    let terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height),
        },
    )?;
    Ok(terminal)
}

fn map_key_to_event(key: KeyCode, app: &TuiApp) -> AppEvent {
    match app.overlay {
        Some(Overlay::Help(_)) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Char('1') => AppEvent::SelectHelpTab(HelpTab::General),
            KeyCode::Char('2') => AppEvent::SelectHelpTab(HelpTab::Commands),
            KeyCode::Char('3') => AppEvent::SelectHelpTab(HelpTab::Runtime),
            _ => AppEvent::Noop,
        },
        Some(Overlay::CommandPalette) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveCommandSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveCommandSelection(1),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::Status) => match key {
            KeyCode::Esc | KeyCode::Enter => AppEvent::CloseOverlay,
            _ => AppEvent::Noop,
        },
        Some(Overlay::Setup) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Char('1') => AppEvent::SetModelSelection(0),
            KeyCode::Char('2') => AppEvent::SetModelSelection(1),
            KeyCode::Char('3') => AppEvent::SetModelSelection(2),
            KeyCode::Char('m') => AppEvent::CycleModelSelection,
            KeyCode::Char('l') => AppEvent::StartOAuth,
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ProviderPicker) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveProviderSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveProviderSelection(1),
            KeyCode::Char('1') => AppEvent::SetProviderSelection(0),
            KeyCode::Char('2') => AppEvent::SetProviderSelection(1),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ModelPicker) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveModelSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveModelSelection(1),
            KeyCode::Char('1') => AppEvent::SetModelSelection(0),
            KeyCode::Char('2') => AppEvent::SetModelSelection(1),
            KeyCode::Char('3') => AppEvent::SetModelSelection(2),
            KeyCode::Char('b') if app.provider_picker_idx == 1 => {
                AppEvent::OpenOverlay(Overlay::BaseUrlEditor)
            }
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::BaseUrlEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveBaseUrlInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        None => match key {
            KeyCode::Esc => AppEvent::Noop,
            KeyCode::Enter => AppEvent::SubmitComposer,
            KeyCode::Up | KeyCode::Char('k') if app.input.is_empty() => AppEvent::ScrollTranscript(-1),
            KeyCode::Down | KeyCode::Char('j') if app.input.is_empty() => AppEvent::ScrollTranscript(1),
            KeyCode::PageUp if app.input.is_empty() => AppEvent::ScrollTranscript(-8),
            KeyCode::PageDown if app.input.is_empty() => AppEvent::ScrollTranscript(8),
            KeyCode::Char('1') if app.input.is_empty() && app.snapshot.pending_question.is_some() => {
                AppEvent::SelectPendingOption(0)
            }
            KeyCode::Char('2') if app.input.is_empty() && app.snapshot.pending_question.is_some() => {
                AppEvent::SelectPendingOption(1)
            }
            KeyCode::Char('3') if app.input.is_empty() && app.snapshot.pending_question.is_some() => {
                AppEvent::SelectPendingOption(2)
            }
            KeyCode::Char('s') => AppEvent::OpenOverlay(Overlay::Setup),
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
    }
}

async fn dispatch_event(
    event: AppEvent,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<bool> {
    match event {
        AppEvent::Noop => {}
        AppEvent::OpenOverlay(overlay) => app.open_overlay(overlay),
        AppEvent::CloseOverlay => app.close_overlay(),
        AppEvent::SubmitComposer => {
            if handle_submit(app, agent_slot, oauth_manager).await? {
                return Ok(true);
            }
        }
        AppEvent::InputChar(c) => {
            if matches!(app.overlay, Some(Overlay::BaseUrlEditor)) {
                app.base_url_input.push(c);
            } else {
                app.input.push(c);
                if app.input.trim_start().starts_with('/') {
                    app.open_overlay(Overlay::CommandPalette);
                } else if matches!(app.overlay, Some(Overlay::CommandPalette)) {
                    app.close_overlay();
                }
            }
        }
        AppEvent::Backspace => {
            if matches!(app.overlay, Some(Overlay::BaseUrlEditor)) {
                app.base_url_input.pop();
            } else {
                app.input.pop();
            }
            if app.input.trim().is_empty() && matches!(app.overlay, Some(Overlay::CommandPalette)) {
                app.close_overlay();
            }
        }
        AppEvent::ScrollTranscript(delta) => app.scroll_transcript(delta),
        AppEvent::MoveCommandSelection(delta) => {
            let len = palette_commands(app, app.input.trim_start().trim_start_matches('/')).len();
            if len > 0 {
                let next = (app.command_palette_idx as i32 + delta).clamp(0, len as i32 - 1);
                app.command_palette_idx = next as usize;
            }
        }
        AppEvent::MoveProviderSelection(delta) => {
            let next = (app.provider_picker_idx as i32 + delta)
                .clamp(0, PROVIDER_FAMILIES.len() as i32 - 1);
            app.provider_picker_idx = next as usize;
        }
        AppEvent::MoveModelSelection(delta) => {
            let next = (app.model_picker_idx as i32 + delta)
                .clamp(0, current_model_presets(app.provider_picker_idx).len() as i32 - 1);
            app.model_picker_idx = next as usize;
        }
        AppEvent::SetProviderSelection(idx) => {
            app.provider_picker_idx = idx.min(PROVIDER_FAMILIES.len() - 1);
            app.model_picker_idx = 0;
        }
        AppEvent::SetModelSelection(idx) => {
            app.model_picker_idx = idx.min(current_model_presets(app.provider_picker_idx).len() - 1);
            if matches!(app.overlay, Some(Overlay::Setup)) {
                app.select_local_model(app.model_picker_idx);
            } else if matches!(app.overlay, Some(Overlay::ModelPicker)) && !app.is_busy() {
                app.select_local_model(app.model_picker_idx);
                start_rebuild_task(app);
            }
        }
        AppEvent::SelectPendingOption(idx) => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before answering the structured question.");
            } else if app.has_pending_approval() {
                if let Some(agent) = agent_slot.take() {
                    let selection = match idx {
                        0 => BashApprovalMode::Once,
                        1 => BashApprovalMode::Always,
                        _ => BashApprovalMode::Suggestion,
                    };
                    start_pending_approval_task(app, selection, agent);
                }
            } else if let Some(label) = app.pending_question_option_label(idx) {
                if let Some(agent) = agent_slot.as_mut() {
                    agent.consume_pending_user_input(&label);
                    app.sync_snapshot(agent);
                }
                app.input = label;
                if handle_submit(app, agent_slot, oauth_manager).await? {
                    return Ok(true);
                }
            }
        }
        AppEvent::CycleModelSelection => {
            app.cycle_local_model();
        }
        AppEvent::SaveBaseUrlInput => {
            let value = app.base_url_input.trim();
            app.config.base_url = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
            app.config_manager.save(&app.config)?;
            app.notice = Some(format!(
                "Saved base URL: {}",
                app.config.base_url.as_deref().unwrap_or("unset")
            ));
            app.close_overlay();
        }
        AppEvent::SelectHelpTab(tab) => {
            app.open_overlay(Overlay::Help(tab));
        }
        AppEvent::StartOAuth => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before starting login.");
            } else {
                start_oauth_task(app, Arc::clone(oauth_manager));
            }
        }
        AppEvent::ApplyOverlaySelection => match app.overlay {
            Some(Overlay::CommandPalette) => {
                let query = app.input.trim_start().trim_start_matches('/');
                if let Some(spec) = palette_command_by_index(app, query, app.command_palette_idx) {
                    app.input = spec.usage.to_string();
                    app.close_overlay();
                    if handle_submit(app, agent_slot, oauth_manager).await? {
                        return Ok(true);
                    }
                }
            }
            Some(Overlay::ProviderPicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else {
                    app.open_overlay(Overlay::ModelPicker);
                }
            }
            Some(Overlay::BaseUrlEditor) => {
                let value = app.base_url_input.trim();
                app.config.base_url = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
                app.config_manager.save(&app.config)?;
                app.notice = Some(format!(
                    "Saved base URL: {}",
                    app.config.base_url.as_deref().unwrap_or("unset")
                ));
                app.close_overlay();
            }
            Some(Overlay::ModelPicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else {
                    app.select_local_model(app.model_picker_idx);
                    start_rebuild_task(app);
                }
            }
            Some(Overlay::Setup) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else {
                    start_rebuild_task(app);
                }
            }
            _ => {}
        },
    }
    Ok(false)
}

async fn handle_submit(
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<bool> {
    if matches!(app.overlay, Some(Overlay::CommandPalette)) {
        let query = app.input.trim_start().trim_start_matches('/');
        if let Some(spec) = palette_command_by_index(app, query, app.command_palette_idx) {
            app.input = spec.usage.to_string();
        }
        app.close_overlay();
    }

    if app.input.is_empty() {
        return Ok(false);
    }
    if app.is_busy() {
        app.push_notice("A task is already running. Wait for it to finish.");
        return Ok(false);
    }

    let input = std::mem::take(&mut app.input);
    if let Some(command) = parse_local_command(&input) {
        if execute_local_command(command, app, agent_slot, oauth_manager).await? {
            return Ok(true);
        }
    } else if input.trim_start().starts_with('/') {
        app.push_notice(format!("Unknown command '{}'. Use /help.", input.trim()));
    } else if let Some(agent) = agent_slot.take() {
        let mut agent = agent;
        if app.snapshot.pending_question.is_some() {
            agent.consume_pending_user_input(input.trim());
        }
        start_query_task(app, input.trim().to_string(), agent);
    }
    Ok(false)
}

fn clamp_command_palette_selection(app: &mut TuiApp) {
    let len = palette_commands(app, app.input.trim_start().trim_start_matches('/')).len();
    if len == 0 {
        app.command_palette_idx = 0;
    } else if app.command_palette_idx >= len {
        app.command_palette_idx = len - 1;
    }
}

fn teardown_terminal(
    mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), Show)?;
    terminal.show_cursor()?;
    Ok(())
}

fn flush_committed_history(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut TuiApp,
) -> anyhow::Result<()> {
    while app.inserted_turns < app.committed_turns.len() {
        let turn = &app.committed_turns[app.inserted_turns];
        let mut lines = committed_turn_lines(turn.entries.as_slice());
        if app.inserted_turns > 0 && !lines.is_empty() {
            lines.insert(0, ratatui::text::Line::from(""));
        }
        if !lines.is_empty() {
            let line_count = lines.len() as u16;
            terminal.insert_before(line_count, |buf| {
                Paragraph::new(lines).render(buf.area, buf);
            })?;
        }
        app.inserted_turns += 1;
    }
    Ok(())
}

pub(crate) fn provider_requires_api_key(provider: &str) -> bool {
    !matches!(
        provider,
        "mock" | "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3" | "ollama"
    )
}
