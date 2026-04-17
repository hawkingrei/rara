mod app_event;
mod command;
mod custom_terminal;
mod event_stream;
mod highlight;
mod insert_history;
mod line_utils;
mod markdown;
mod markdown_render;
mod markdown_stream;
mod render;
mod runtime;
mod session_restore;
mod state;
mod terminal_ui;

use std::sync::Arc;

use crossterm::{
    event::{EventStream, KeyCode},
    terminal::size as terminal_size,
    terminal::enable_raw_mode,
};
use futures::StreamExt;
use tokio::time::{interval, Duration};

use crate::agent::Agent;
use crate::oauth::OAuthManager;
use crate::state_db::StateDb;

use self::app_event::AppEvent;
use self::command::{palette_command_by_index, palette_commands, parse_local_command};
use self::event_stream::{translate_event, UiEvent};
use self::render::{desired_viewport_height, render};
use self::runtime::{
    execute_local_command, finish_running_task_if_ready, start_oauth_task, start_pending_approval_task, start_query_task,
    start_rebuild_task,
};
use self::session_restore::{provider_requires_api_key, restore_latest_session, restore_session_by_id};
use self::state::{current_model_presets, HelpTab, Overlay, PROVIDER_FAMILIES, TuiApp};
use self::terminal_ui::{
    build_terminal, flush_committed_history, handle_paste, is_ssh_session, teardown_terminal,
    update_terminal_viewport,
};
use crate::agent::BashApprovalMode;

pub async fn run_tui(agent: Agent, oauth_manager: OAuthManager) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let initial_size = terminal_size()?;
    let mut app = TuiApp::new(crate::config::ConfigManager::new()?);
    let mut viewport_height = desired_viewport_height(&app, initial_size.0, initial_size.1);
    let mut terminal = build_terminal(viewport_height)?;
    let mut agent_slot = Some(agent);
    match StateDb::new() {
        Ok(state_db) => {
            let state_db = Arc::new(state_db);
            restore_latest_session(&state_db, &mut app, &mut agent_slot)?;
            app.attach_state_db(state_db);
        }
        Err(err) => app.set_state_db_error(err.to_string()),
    }
    let oauth_manager = Arc::new(oauth_manager);
    let mut events = EventStream::new();
    let mut tick = interval(Duration::from_millis(100));

    if let Some(agent_ref) = agent_slot.as_ref() {
        app.sync_snapshot(agent_ref);
    }

    let result = loop {
        finish_running_task_if_ready(&mut app, &mut agent_slot).await?;
        clamp_command_palette_selection(&mut app);
        let size = terminal_size()?;
        let desired_height = desired_viewport_height(&app, size.0, size.1);
        if desired_height != viewport_height {
            match update_terminal_viewport(&mut terminal, desired_height) {
                Ok(()) => {
                    viewport_height = desired_height;
                }
                Err(err) => app.push_notice(format!("Skipped viewport update: {err}")),
            }
        }
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
                            let size = terminal_size()?;
                            let desired_height = desired_viewport_height(&app, size.0, size.1);
                            match update_terminal_viewport(&mut terminal, desired_height) {
                                Ok(()) => {
                                    viewport_height = desired_height;
                                }
                                Err(err) => app.push_notice(format!("Skipped viewport redraw update: {err}")),
                            }
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
        Some(Overlay::ResumePicker) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveResumeSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveResumeSelection(1),
            KeyCode::Char('1') => AppEvent::SetResumeSelection(0),
            KeyCode::Char('2') => AppEvent::SetResumeSelection(1),
            KeyCode::Char('3') => AppEvent::SetResumeSelection(2),
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
        Some(Overlay::CodexAuthGuide) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Char('1') | KeyCode::Char('o') => AppEvent::StartOAuth,
            KeyCode::Char('2') | KeyCode::Char('a') => AppEvent::OpenOverlay(Overlay::ApiKeyEditor),
            KeyCode::Enter => AppEvent::StartOAuth,
            _ => AppEvent::Noop,
        },
        Some(Overlay::BaseUrlEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveBaseUrlInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ApiKeyEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveApiKeyInput,
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
            } else if matches!(app.overlay, Some(Overlay::ApiKeyEditor)) {
                app.api_key_input.push(c);
            } else {
                app.input.push(c);
                app.sync_command_palette_with_input();
            }
        }
        AppEvent::Backspace => {
            if matches!(app.overlay, Some(Overlay::BaseUrlEditor)) {
                app.base_url_input.pop();
            } else if matches!(app.overlay, Some(Overlay::ApiKeyEditor)) {
                app.api_key_input.pop();
            } else {
                app.input.pop();
                app.sync_command_palette_with_input();
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
        AppEvent::MoveResumeSelection(delta) => {
            let len = app.recent_sessions.len();
            if len > 0 {
                let next = (app.resume_picker_idx as i32 + delta).clamp(0, len as i32 - 1);
                app.resume_picker_idx = next as usize;
            }
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
        AppEvent::SetResumeSelection(idx) => {
            if !app.recent_sessions.is_empty() {
                app.resume_picker_idx = idx.min(app.recent_sessions.len() - 1);
            }
        }
        AppEvent::SetModelSelection(idx) => {
            app.model_picker_idx = idx.min(current_model_presets(app.provider_picker_idx).len() - 1);
            if matches!(app.overlay, Some(Overlay::Setup)) {
                app.select_local_model(app.model_picker_idx);
            } else if matches!(app.overlay, Some(Overlay::ModelPicker)) && !app.is_busy() {
                if should_open_codex_auth_guide(app) {
                    app.select_local_model(app.model_picker_idx);
                    app.open_overlay(Overlay::CodexAuthGuide);
                } else {
                    app.select_local_model(app.model_picker_idx);
                    start_rebuild_task(app);
                }
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
        AppEvent::SaveApiKeyInput => {
            let value = app.api_key_input.trim();
            if value.is_empty() {
                app.push_notice("Enter a Codex API key or press Esc to go back.");
            } else if app.is_busy() {
                app.push_notice("Wait for the current task before saving the API key.");
            } else {
                app.config.api_key = Some(value.to_string());
                app.config.provider = "codex".into();
                if app.config.model.is_none() {
                    app.config.model = Some("codex".into());
                }
                app.config_manager.save(&app.config)?;
                app.notice = Some("Saved Codex API key. Rebuilding backend.".into());
                app.overlay = None;
                start_rebuild_task(app);
            }
        }
        AppEvent::SelectHelpTab(tab) => {
            app.open_overlay(Overlay::Help(tab));
        }
        AppEvent::StartOAuth => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before starting login.");
            } else if is_ssh_session() {
                app.push_notice("OAuth browser login is unavailable in SSH/headless sessions. Use Codex API key instead.");
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
            Some(Overlay::ResumePicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else if let Some(session_id) = app
                    .recent_sessions
                    .get(app.resume_picker_idx)
                    .map(|session| session.session_id.clone())
                {
                    restore_session_by_id(session_id.as_str(), app, agent_slot)?;
                    app.close_overlay();
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
                    if should_open_codex_auth_guide(app) {
                        app.open_overlay(Overlay::CodexAuthGuide);
                    } else {
                        start_rebuild_task(app);
                    }
                }
            }
            Some(Overlay::Setup) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else {
                    start_rebuild_task(app);
                }
            }
            Some(Overlay::CodexAuthGuide) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else if is_ssh_session() {
                    app.push_notice("OAuth browser login is unavailable in SSH/headless sessions. Use Codex API key instead.");
                } else {
                    start_oauth_task(app, Arc::clone(oauth_manager));
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

fn should_open_codex_auth_guide(app: &TuiApp) -> bool {
    let presets = current_model_presets(app.provider_picker_idx);
    let Some((_, provider, _)) = presets.get(app.model_picker_idx) else {
        return false;
    };
    *provider == "codex" && app.config.api_key.as_deref().is_none_or(str::is_empty)
}
