mod app_event;
mod command;
mod event_stream;
mod highlight;
mod line_utils;
mod markdown;
mod markdown_render;
mod markdown_stream;
mod render;
mod runtime;
mod state;

use std::io;
use std::sync::Arc;

use crossterm::{
    cursor::Show,
    event::{EventStream, KeyCode},
    execute,
    terminal::size as terminal_size,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
    Terminal, TerminalOptions, Viewport,
};
use tokio::time::{interval, Duration};

use crate::agent::Agent;
use crate::agent::CompletedInteraction;
use crate::agent::PendingApproval;
use crate::agent::PendingUserInput;
use crate::agent::PlanStep;
use crate::agent::PlanStepStatus;
use crate::oauth::OAuthManager;
use crate::state_db::StateDb;

use self::app_event::AppEvent;
use self::command::{palette_command_by_index, palette_commands, parse_local_command};
use self::event_stream::{translate_event, UiEvent};
use self::render::{committed_turn_lines, desired_viewport_height, render};
use self::runtime::{
    execute_local_command, finish_running_task_if_ready, start_oauth_task, start_pending_approval_task, start_query_task,
    start_rebuild_task,
};
use self::state::{current_model_presets, HelpTab, Overlay, PROVIDER_FAMILIES, TranscriptEntry, TranscriptTurn, TuiApp};
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
            match build_terminal(desired_height) {
                Ok(new_terminal) => {
                    viewport_height = desired_height;
                    terminal = new_terminal;
                }
                Err(err) => {
                    app.push_notice(format!("Skipped viewport rebuild: {err}"));
                }
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
                            match build_terminal(desired_height) {
                                Ok(new_terminal) => {
                                    viewport_height = desired_height;
                                    terminal = new_terminal;
                                }
                                Err(err) => {
                                    app.push_notice(format!("Skipped terminal redraw rebuild: {err}"));
                                }
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

fn handle_paste(text: String, app: &mut TuiApp) {
    if matches!(app.overlay, Some(Overlay::BaseUrlEditor)) {
        app.base_url_input.push_str(&text);
        return;
    }

    if matches!(app.overlay, Some(Overlay::ApiKeyEditor)) {
        app.api_key_input.push_str(&text);
        return;
    }

    app.input.push_str(&text);
    if app.input.trim_start().starts_with('/') {
        app.open_overlay(Overlay::CommandPalette);
    } else if matches!(app.overlay, Some(Overlay::CommandPalette)) {
        app.close_overlay();
    }
}

fn build_terminal(viewport_height: u16) -> anyhow::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    match Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height.max(1)),
        },
    ) {
        Ok(terminal) => Ok(terminal),
        Err(inline_err) => {
            let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
                .map_err(|fallback_err| anyhow::anyhow!(
                    "failed to build inline terminal: {inline_err}; fullscreen fallback also failed: {fallback_err}"
                ))?;
            Ok(terminal)
        }
    }
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
            } else if matches!(app.overlay, Some(Overlay::ApiKeyEditor)) {
                app.api_key_input.pop();
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
    if !app.startup_card_inserted {
        let width = terminal_size()?.0;
        let lines = startup_card_lines(app, width);
        if !lines.is_empty() {
            let line_count = wrapped_history_line_count(lines.as_slice(), width);
            terminal.insert_before(line_count, |buf| {
                Paragraph::new(lines)
                    .wrap(ratatui::widgets::Wrap { trim: false })
                    .render(buf.area, buf);
            })?;
        }
        app.startup_card_inserted = true;
    }
    while app.inserted_turns < app.committed_turns.len() {
        let turn = &app.committed_turns[app.inserted_turns];
        let cwd = (!app.snapshot.cwd.is_empty()).then(|| std::path::Path::new(app.snapshot.cwd.as_str()));
        let mut lines = committed_turn_lines(turn.entries.as_slice(), cwd);
        if app.inserted_turns > 0 && !lines.is_empty() {
            lines.insert(0, ratatui::text::Line::from(""));
        }
        if !lines.is_empty() {
            let width = terminal_size()?.0;
            let line_count = wrapped_history_line_count(lines.as_slice(), width);
            terminal.insert_before(line_count, |buf| {
                Paragraph::new(lines)
                    .wrap(ratatui::widgets::Wrap { trim: false })
                    .render(buf.area, buf);
            })?;
        }
        app.inserted_turns += 1;
    }
    Ok(())
}

fn wrapped_history_line_count(lines: &[Line<'static>], width: u16) -> u16 {
    let wrap_width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(wrap_width))
        .sum::<usize>()
        .max(1) as u16
}

fn startup_card_lines(app: &TuiApp, width: u16) -> Vec<Line<'static>> {
    let Some(inner_width) = startup_card_inner_width(width) else {
        return Vec::new();
    };

    let model_label = "model:";
    let directory_label = "directory:";
    let label_width = directory_label.len();
    let model_prefix = format!("{model_label:<label_width$} ");
    let hint = "/model to change";
    let hint_width = display_width(hint);
    let model_prefix_width = display_width(&model_prefix);
    let model_available_width = inner_width
        .saturating_sub(model_prefix_width)
        .saturating_sub(1)
        .saturating_sub(hint_width);
    let model_value = truncate_for_startup_card(app.current_model_label(), model_available_width);
    let model_value_width = display_width(&model_value);
    let gap_width = inner_width
        .saturating_sub(model_prefix_width)
        .saturating_sub(model_value_width)
        .saturating_sub(hint_width)
        .max(1);
    let directory_prefix = format!("{directory_label:<label_width$} ");
    let directory_max_width = inner_width.saturating_sub(display_width(&directory_prefix));

    let lines = vec![
        Line::from(vec![Span::from(">_ "), Span::from("RARA")]),
        Line::from(""),
        Line::from(vec![
            Span::from(model_prefix),
            Span::from(model_value),
            Span::from(" ".repeat(gap_width)),
            Span::from(hint),
        ]),
        Line::from(vec![
            Span::from(directory_prefix),
            Span::from(truncate_path_middle(
                &display_directory_for_startup(app),
                directory_max_width,
            )),
        ]),
    ];

    with_border(lines, inner_width)
}

fn display_directory_for_startup(app: &TuiApp) -> String {
    let cwd = if app.snapshot.cwd.is_empty() {
        std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ".".to_string())
    } else {
        app.snapshot.cwd.clone()
    };
    if let Ok(home) = std::env::var("HOME") {
        if let Some(stripped) = cwd.strip_prefix(&home) {
            return format!("~{stripped}");
        }
    }
    cwd
}

fn truncate_for_startup_card(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let kept = value.chars().take(width - 1).collect::<String>();
    format!("{kept}…")
}

fn truncate_path_middle(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    if width <= 5 {
        return truncate_for_startup_card(value, width);
    }

    let keep_left = (width - 1) / 2;
    let keep_right = width - 1 - keep_left;
    let chars = value.chars().collect::<Vec<_>>();
    let left = chars.iter().take(keep_left).collect::<String>();
    let right = chars
        .iter()
        .rev()
        .take(keep_right)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{left}…{right}")
}

fn startup_card_inner_width(width: u16) -> Option<usize> {
    if width < 8 {
        return None;
    }
    Some(std::cmp::min(width.saturating_sub(4) as usize, 56))
}

fn with_border(lines: Vec<Line<'static>>, inner_width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(lines.len() + 3);
    let border_inner_width = inner_width + 2;
    out.push(Line::from(format!("╭{}╮", "─".repeat(border_inner_width))));

    for line in lines {
        let used_width = line
            .iter()
            .map(|span| display_width(span.content.as_ref()))
            .sum::<usize>();
        let mut spans = Vec::with_capacity(line.spans.len() + 3);
        spans.push(Span::from("│ "));
        spans.extend(line.into_iter());
        if used_width < inner_width {
            spans.push(Span::from(" ".repeat(inner_width - used_width)));
        }
        spans.push(Span::from(" │"));
        out.push(Line::from(spans));
    }

    out.push(Line::from(format!("╰{}╯", "─".repeat(border_inner_width))));
    out
}

fn display_width(value: &str) -> usize {
    value.chars().count()
}

fn restore_latest_session(
    state_db: &Arc<StateDb>,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> anyhow::Result<()> {
    let Some(session_id) = state_db.latest_session_id()? else {
        return Ok(());
    };
    restore_session_by_id(session_id.as_str(), app, agent_slot)
}

fn restore_session_by_id(
    session_id: &str,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> anyhow::Result<()> {
    let Some(agent) = agent_slot.as_mut() else {
        return Ok(());
    };
    let Some(state_db) = app.state_db.as_ref() else {
        return Ok(());
    };
    if let Ok(history) = agent.session_manager.load_session(session_id) {
        agent.history = history;
        agent.session_id = session_id.to_string();
    }
    let persisted_steps = state_db.load_plan_steps(session_id)?;
    if !persisted_steps.is_empty() {
        agent.current_plan = persisted_steps
            .into_iter()
            .map(|step| PlanStep {
                step: step.step,
                status: match step.status.as_str() {
                    "completed" => PlanStepStatus::Completed,
                    "in_progress" => PlanStepStatus::InProgress,
                    _ => PlanStepStatus::Pending,
                },
            })
            .collect();
    } else {
        agent.current_plan.clear();
    }
    agent.plan_explanation = state_db.load_session_plan_explanation(session_id)?;
    agent.pending_user_input = None;
    agent.pending_approval = None;
    agent.completed_user_input = None;
    agent.completed_approval = None;
    let interactions = state_db.load_interactions(session_id)?;
    for interaction in interactions {
        match (interaction.kind.as_str(), interaction.status.as_str()) {
            ("request_input", "pending") => {
                let Some(payload) = interaction.payload.as_ref() else {
                    continue;
                };
                let options = payload
                    .get("options")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let pair = item.as_array()?;
                                let label = pair.first()?.as_str()?.to_string();
                                let detail = pair.get(1)?.as_str()?.to_string();
                                Some((label, detail))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                agent.pending_user_input = Some(PendingUserInput {
                    question: payload
                        .get("question")
                        .and_then(|value| value.as_str())
                        .unwrap_or(&interaction.title)
                        .to_string(),
                    options,
                    note: payload
                        .get("note")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                });
            }
            ("approval", "pending") => {
                let payload = interaction.payload.as_ref();
                let command = payload
                    .and_then(|payload| payload.get("command"))
                    .and_then(|value| value.as_str())
                    .unwrap_or(&interaction.summary)
                    .to_string();
                agent.pending_approval = Some(PendingApproval {
                    tool_use_id: payload
                        .and_then(|payload| payload.get("tool_use_id"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("restored")
                        .to_string(),
                    command,
                    allow_net: payload
                        .and_then(|payload| payload.get("allow_net"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                });
            }
            ("request_input", "completed") => {
                agent.completed_user_input = Some(CompletedInteraction {
                    title: interaction.title,
                    summary: interaction.summary,
                });
            }
            ("approval", "completed") => {
                agent.completed_approval = Some(CompletedInteraction {
                    title: interaction.title,
                    summary: interaction.summary,
                });
            }
            _ => {}
        }
    }
    let summaries = state_db.load_turn_summaries(session_id)?;
    let mut turns = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let entries = state_db
            .load_turn_entries(session_id, summary.ordinal)?
            .into_iter()
            .map(|entry| TranscriptEntry {
                role: entry.role,
                message: entry.message,
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            turns.push(TranscriptTurn { entries });
        }
    }
    if !turns.is_empty() {
        app.restore_committed_turns(turns);
    } else {
        app.reset_transcript();
    }
    app.sync_snapshot(agent);
    app.notice = Some(format!("Resumed session {session_id}."));
    Ok(())
}

pub(crate) fn provider_requires_api_key(provider: &str) -> bool {
    !matches!(
        provider,
        "mock" | "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3" | "ollama"
    )
}

fn should_open_codex_auth_guide(app: &TuiApp) -> bool {
    let presets = current_model_presets(app.provider_picker_idx);
    let Some((_, provider, _)) = presets.get(app.model_picker_idx) else {
        return false;
    };
    *provider == "codex" && app.config.api_key.as_deref().is_none_or(str::is_empty)
}

fn is_ssh_session() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
}
