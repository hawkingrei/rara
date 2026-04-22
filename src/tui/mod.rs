mod app_event;
mod auth_mode_picker;
mod command;
mod custom_terminal;
mod event_stream;
mod highlight;
mod insert_history;
mod interaction_text;
mod line_utils;
mod markdown;
mod markdown_render;
mod markdown_stream;
mod plan_display;
mod queued_input;
mod render;
mod runtime;
mod session_restore;
mod state;
mod terminal_ui;

use std::sync::Arc;

use crossterm::{
    event::{EventStream, KeyCode},
    terminal::enable_raw_mode,
    terminal::size as terminal_size,
};
use futures::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use tokio::time::{interval, Duration};

use crate::agent::Agent;
use crate::codex_model_catalog::load_codex_model_catalog;
use crate::oauth::OAuthManager;
use crate::state_db::StateDb;
use codex_models_manager::manager::RefreshStrategy;

use self::app_event::AppEvent;
use self::command::{palette_command_by_index, palette_commands, parse_local_command};
use self::event_stream::{translate_event, UiEvent};
use self::render::{desired_viewport_height, render};
use self::runtime::{
    execute_local_command, finish_running_task_if_ready, should_suggest_planning_mode,
    start_oauth_task, start_pending_approval_task, start_plan_approval_resume_task,
    start_query_task, start_rebuild_task,
};
use self::session_restore::{
    provider_requires_api_key, restore_latest_session, restore_session_by_id,
};
use self::state::{HelpTab, LocalCommandKind, Overlay, TaskKind, TuiApp, PROVIDER_FAMILIES};
use self::terminal_ui::{
    build_terminal, flush_committed_history, handle_paste, is_ssh_session, teardown_terminal,
    update_terminal_viewport,
};
use crate::agent::AgentExecutionMode;
use crate::agent::BashApprovalMode;

pub async fn run_tui(agent: Agent, oauth_manager: OAuthManager) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let initial_size = terminal_size()?;
    let mut app = TuiApp::new(crate::config::ConfigManager::new()?)?;
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
                                if let Some(task) = app.running_task.take() {
                                    task.handle.abort();
                                }
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
            KeyCode::Char('4') => AppEvent::SetModelSelection(3),
            KeyCode::Char('5') => AppEvent::SetModelSelection(4),
            KeyCode::Char('6') => AppEvent::SetModelSelection(5),
            KeyCode::Char('7') => AppEvent::SetModelSelection(6),
            KeyCode::Char('8') => AppEvent::SetModelSelection(7),
            KeyCode::Char('9') => AppEvent::SetModelSelection(8),
            KeyCode::Char('m') => AppEvent::CycleModelSelection,
            KeyCode::Char('l')
                if matches!(
                    app.selected_provider_family(),
                    self::state::ProviderFamily::Codex
                ) =>
            {
                AppEvent::OpenOverlay(Overlay::AuthModePicker)
            }
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ProviderPicker) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveProviderSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveProviderSelection(1),
            KeyCode::Char('1') => AppEvent::SetProviderSelection(0),
            KeyCode::Char('2') => AppEvent::SetProviderSelection(1),
            KeyCode::Char('3') => AppEvent::SetProviderSelection(2),
            KeyCode::Char('4') => AppEvent::SetProviderSelection(3),
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
            KeyCode::Char('4') => AppEvent::SetModelSelection(3),
            KeyCode::Char('5') => AppEvent::SetModelSelection(4),
            KeyCode::Char('6') => AppEvent::SetModelSelection(5),
            KeyCode::Char('7') => AppEvent::SetModelSelection(6),
            KeyCode::Char('8') => AppEvent::SetModelSelection(7),
            KeyCode::Char('9') => AppEvent::SetModelSelection(8),
            KeyCode::Char('b')
                if matches!(
                    app.selected_provider_family(),
                    self::state::ProviderFamily::OpenAiCompatible
                        | self::state::ProviderFamily::Ollama
                ) =>
            {
                AppEvent::OpenOverlay(Overlay::BaseUrlEditor)
            }
            KeyCode::Char('a')
                if matches!(
                    app.selected_provider_family(),
                    self::state::ProviderFamily::OpenAiCompatible
                ) =>
            {
                AppEvent::OpenOverlay(Overlay::ApiKeyEditor)
            }
            KeyCode::Char('n')
                if matches!(
                    app.selected_provider_family(),
                    self::state::ProviderFamily::OpenAiCompatible
                ) =>
            {
                AppEvent::OpenOverlay(Overlay::ModelNameEditor)
            }
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::AuthModePicker) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveAuthModeSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveAuthModeSelection(1),
            KeyCode::Char('1') => AppEvent::SetAuthModeSelection(0),
            KeyCode::Char('2') => AppEvent::SetAuthModeSelection(1),
            KeyCode::Char('3') => AppEvent::SetAuthModeSelection(2),
            KeyCode::Char('4') => AppEvent::SetAuthModeSelection(3),
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
        Some(Overlay::ApiKeyEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveApiKeyInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ModelNameEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveModelNameInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ReasoningEffortPicker) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveReasoningEffortSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveReasoningEffortSelection(1),
            KeyCode::Char('1') => AppEvent::SetReasoningEffortSelection(0),
            KeyCode::Char('2') => AppEvent::SetReasoningEffortSelection(1),
            KeyCode::Char('3') => AppEvent::SetReasoningEffortSelection(2),
            KeyCode::Char('4') => AppEvent::SetReasoningEffortSelection(3),
            KeyCode::Char('5') => AppEvent::SetReasoningEffortSelection(4),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        None => match key {
            KeyCode::Esc => AppEvent::Noop,
            KeyCode::Enter => AppEvent::SubmitComposer,
            KeyCode::Up | KeyCode::Char('k') if app.input.is_empty() => {
                AppEvent::ScrollTranscript(-1)
            }
            KeyCode::Down | KeyCode::Char('j') if app.input.is_empty() => {
                AppEvent::ScrollTranscript(1)
            }
            KeyCode::PageUp if app.input.is_empty() => AppEvent::ScrollTranscript(-8),
            KeyCode::PageDown if app.input.is_empty() => AppEvent::ScrollTranscript(8),
            KeyCode::Char('1')
                if app.input.is_empty()
                    && (app.active_pending_interaction().is_some()
                        || app.has_pending_planning_suggestion()) =>
            {
                AppEvent::SelectPendingOption(0)
            }
            KeyCode::Char('2')
                if app.input.is_empty()
                    && (app.active_pending_interaction().is_some()
                        || app.has_pending_planning_suggestion()) =>
            {
                AppEvent::SelectPendingOption(1)
            }
            KeyCode::Char('3')
                if app.input.is_empty()
                    && app.active_pending_interaction().is_some_and(|pending| {
                        pending.kind != self::state::ActivePendingInteractionKind::PlanApproval
                    }) =>
            {
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
            } else if matches!(app.overlay, Some(Overlay::ModelNameEditor)) {
                app.model_name_input.push(c);
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
            } else if matches!(app.overlay, Some(Overlay::ModelNameEditor)) {
                app.model_name_input.pop();
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
            let len = app.current_model_picker_len();
            if len > 0 {
                let next = (app.model_picker_idx as i32 + delta).clamp(0, len as i32 - 1);
                app.model_picker_idx = next as usize;
            }
        }
        AppEvent::MoveReasoningEffortSelection(delta) => {
            let len = app.selected_codex_reasoning_options().len();
            if len > 0 {
                let next =
                    (app.reasoning_effort_picker_idx as i32 + delta).clamp(0, len as i32 - 1);
                app.reasoning_effort_picker_idx = next as usize;
            }
        }
        AppEvent::MoveAuthModeSelection(delta) => {
            let next = (app.auth_mode_idx as i32 + delta).clamp(0, 3);
            app.auth_mode_idx = next as usize;
        }
        AppEvent::SetProviderSelection(idx) => {
            app.provider_picker_idx = idx.min(PROVIDER_FAMILIES.len() - 1);
            app.model_picker_idx = 0;
        }
        AppEvent::SetAuthModeSelection(idx) => {
            app.auth_mode_idx = idx.min(3);
        }
        AppEvent::SetReasoningEffortSelection(idx) => {
            let len = app.selected_codex_reasoning_options().len();
            if len > 0 {
                app.reasoning_effort_picker_idx = idx.min(len - 1);
            }
        }
        AppEvent::SetResumeSelection(idx) => {
            if !app.recent_sessions.is_empty() {
                app.resume_picker_idx = idx.min(app.recent_sessions.len() - 1);
            }
        }
        AppEvent::SetModelSelection(idx) => {
            let len = app.current_model_picker_len();
            if len == 0 {
                return Ok(false);
            }
            app.model_picker_idx = idx.min(len - 1);
            if matches!(app.overlay, Some(Overlay::Setup)) {
                app.select_local_model(app.model_picker_idx);
            } else if matches!(app.overlay, Some(Overlay::ModelPicker))
                && app.selected_provider_family() != self::state::ProviderFamily::Codex
                && !app.is_busy()
            {
                if should_open_codex_auth_guide(app, oauth_manager.as_ref()) {
                    app.select_local_model(app.model_picker_idx);
                    app.open_overlay(Overlay::AuthModePicker);
                } else {
                    app.select_local_model(app.model_picker_idx);
                    start_rebuild_task(app);
                }
            }
        }
        AppEvent::SelectPendingOption(idx) => {
            if app.is_busy() {
                app.push_notice(
                    "Wait for the current task before answering the structured question.",
                );
            } else if app.has_pending_planning_suggestion() {
                match idx {
                    0 => {
                        let Some(prompt) = app.take_pending_planning_suggestion() else {
                            return Ok(false);
                        };
                        app.set_agent_execution_mode(AgentExecutionMode::Plan);
                        if let Some(agent) = agent_slot.as_mut() {
                            agent.set_execution_mode(AgentExecutionMode::Plan);
                        }
                        if let Some(agent) = agent_slot.take() {
                            start_query_task(app, prompt, agent);
                        } else {
                            app.queue_planning_suggestion(prompt);
                            app.push_notice("No active agent is available to enter planning mode.");
                        }
                    }
                    1 => {
                        let Some(prompt) = app.take_pending_planning_suggestion() else {
                            return Ok(false);
                        };
                        app.set_agent_execution_mode(AgentExecutionMode::Execute);
                        if let Some(agent) = agent_slot.as_mut() {
                            agent.set_execution_mode(AgentExecutionMode::Execute);
                        }
                        if let Some(agent) = agent_slot.take() {
                            start_query_task(app, prompt, agent);
                        } else {
                            app.queue_planning_suggestion(prompt);
                            app.push_notice(
                                "No active agent is available to continue in execute mode.",
                            );
                        }
                    }
                    _ => {
                        app.push_notice(
                            "Select 1 to enter planning mode or 2 to continue in execute mode.",
                        );
                    }
                }
            } else if let Some(pending) = app.active_pending_interaction() {
                match pending.kind {
                    self::state::ActivePendingInteractionKind::PlanApproval => match idx {
                        0 => {
                            if let Some(agent) = agent_slot.take() {
                                start_plan_approval_resume_task(app, false, agent);
                            }
                        }
                        1 => {
                            if let Some(agent) = agent_slot.take() {
                                start_plan_approval_resume_task(app, true, agent);
                            }
                        }
                        _ => {
                            app.push_notice("Select 1 to implement now or 2 to continue planning.");
                        }
                    },
                    self::state::ActivePendingInteractionKind::ShellApproval => {
                        if let Some(agent) = agent_slot.take() {
                            let selection = match idx {
                                0 => BashApprovalMode::Once,
                                1 => BashApprovalMode::Always,
                                _ => BashApprovalMode::Suggestion,
                            };
                            start_pending_approval_task(app, selection, agent);
                        }
                    }
                    self::state::ActivePendingInteractionKind::PlanningQuestion
                    | self::state::ActivePendingInteractionKind::ExplorationQuestion
                    | self::state::ActivePendingInteractionKind::SubAgentQuestion
                    | self::state::ActivePendingInteractionKind::RequestInput => {
                        if let Some(label) = app.pending_question_option_label(idx) {
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
                }
            }
        }
        AppEvent::CycleModelSelection => {
            app.cycle_local_model();
        }
        AppEvent::SaveBaseUrlInput => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before saving the base URL.");
            } else {
                let value = app.base_url_input.trim();
                app.config
                    .set_base_url((!value.is_empty()).then(|| value.to_string()));
                app.config_manager.save(&app.config)?;
                app.notice = Some(format!(
                    "Saved base URL: {}",
                    app.config.base_url.as_deref().unwrap_or("unset")
                ));
                app.close_overlay();
            }
        }
        AppEvent::SaveApiKeyInput => {
            let value = app.api_key_input.trim();
            if app.is_busy() {
                app.push_notice("Wait for the current task before saving the API key.");
            } else if value.is_empty() && app.config.provider == "codex" {
                app.push_notice("Enter a Codex API key or press Esc to go back.");
            } else if value.is_empty() {
                app.config.clear_api_key();
                app.config_manager.save(&app.config)?;
                app.notice = Some("Cleared API key for the current provider.".into());
                app.close_overlay();
            } else {
                app.config.set_api_key(value.to_string());
                if app.config.provider == "codex" {
                    app.config.apply_codex_defaults();
                }
                app.config_manager.save(&app.config)?;
                if app.config.provider == "codex" {
                    app.notice = Some("Saved Codex API key. Rebuilding backend.".into());
                    app.overlay = None;
                    start_rebuild_task(app);
                } else {
                    app.notice = Some("Saved API key for the current provider.".into());
                    app.close_overlay();
                }
            }
        }
        AppEvent::SaveModelNameInput => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before saving the model name.");
            } else {
                let value = app.model_name_input.trim();
                if value.is_empty() {
                    app.push_notice("Enter a model name or press Esc to go back.");
                } else {
                    app.config.set_model(Some(value.to_string()));
                    app.config_manager.save(&app.config)?;
                    app.notice = Some(format!("Saved model name: {}", value));
                    app.close_overlay();
                }
            }
        }
        AppEvent::SelectHelpTab(tab) => {
            app.open_overlay(Overlay::Help(tab));
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
                    open_provider_family_overlay(app, oauth_manager.as_ref()).await?;
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
                if app.is_busy() {
                    app.push_notice("Wait for the current task before saving the base URL.");
                } else {
                    let value = app.base_url_input.trim();
                    app.config
                        .set_base_url((!value.is_empty()).then(|| value.to_string()));
                    app.config_manager.save(&app.config)?;
                    app.notice = Some(format!(
                        "Saved base URL: {}",
                        app.config.base_url.as_deref().unwrap_or("unset")
                    ));
                    app.close_overlay();
                }
            }
            Some(Overlay::ModelPicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else {
                    if app.selected_provider_family() == self::state::ProviderFamily::Codex {
                        let _ = sync_codex_credential_from_auth_store(app, oauth_manager.as_ref())?;
                    }
                    if should_open_codex_auth_guide(app, oauth_manager.as_ref()) {
                        app.select_local_model(app.model_picker_idx);
                        app.open_overlay(Overlay::AuthModePicker);
                    } else if app.selected_provider_family() == self::state::ProviderFamily::Codex {
                        app.select_local_model(app.model_picker_idx);
                        if app.selected_codex_reasoning_options().len() <= 1 {
                            app.apply_selected_codex_reasoning_effort();
                            start_rebuild_task(app);
                        } else {
                            app.open_overlay(Overlay::ReasoningEffortPicker);
                        }
                    } else {
                        app.select_local_model(app.model_picker_idx);
                        start_rebuild_task(app);
                    }
                }
            }
            Some(Overlay::ReasoningEffortPicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else {
                    app.select_local_model(app.model_picker_idx);
                    app.apply_selected_codex_reasoning_effort();
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
            Some(Overlay::AuthModePicker) => match app.auth_mode_idx {
                0 => {
                    if app.is_busy() {
                        app.push_notice("A task is already running. Wait for it to finish.");
                    } else if is_ssh_session() {
                        app.push_notice("Browser login is unavailable in SSH/headless sessions. Choose device code or API key instead.");
                    } else {
                        app.close_overlay();
                        start_oauth_task(
                            app,
                            Arc::clone(oauth_manager),
                            self::state::OAuthLoginMode::Browser,
                        );
                    }
                }
                1 => {
                    if app.is_busy() {
                        app.push_notice("A task is already running. Wait for it to finish.");
                    } else {
                        app.close_overlay();
                        start_oauth_task(
                            app,
                            Arc::clone(oauth_manager),
                            self::state::OAuthLoginMode::DeviceCode,
                        );
                    }
                }
                2 => app.open_overlay(Overlay::ApiKeyEditor),
                3 => {
                    if app.is_busy() {
                        app.push_notice("A task is already running. Wait for it to finish.");
                    } else {
                        let removed = oauth_manager.clear_saved_auth()?;
                        app.config.clear_provider_api_key("codex");
                        app.config_manager.save(&app.config)?;
                        app.notice = Some(if removed {
                            "Cleared the saved provider credential.".into()
                        } else {
                            "No saved provider credential was present.".into()
                        });
                        if app.config.provider == "codex" {
                            start_rebuild_task(app);
                        }
                    }
                }
                _ => {}
            },
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
        let input = std::mem::take(&mut app.input);
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        if trimmed.starts_with('/') {
            if let Some(command) = parse_local_command(trimmed) {
                if matches!(command.kind, LocalCommandKind::Quit) {
                    return execute_local_command(command, app, agent_slot, oauth_manager).await;
                }
            }
            app.push_notice(
                "A task is already running. Wait for it to finish before running a slash command.",
            );
        } else {
            let pending_for_tool_boundary = app
                .running_task
                .as_ref()
                .is_some_and(|task| matches!(&task.kind, TaskKind::Query));
            let queued = if pending_for_tool_boundary {
                app.queue_follow_up_message_after_next_tool_boundary(trimmed.to_string())
            } else {
                app.queue_follow_up_message(trimmed.to_string())
            };
            let suffix = if queued > 1 {
                format!(" {queued} follow-up messages are queued.")
            } else {
                " 1 follow-up message is queued.".to_string()
            };
            app.notice = Some(format!(
                "{}{suffix}",
                if pending_for_tool_boundary {
                    "Queued for after the next tool call boundary."
                } else {
                    "Queued for after the current task finishes."
                }
            ));
        }
        return Ok(false);
    }

    let input = std::mem::take(&mut app.input);
    if app.has_pending_plan_approval() && !input.trim_start().starts_with('/') {
        if handle_pending_plan_approval_submit(app, agent_slot, input.trim()).await? {
            return Ok(false);
        }
    }
    if let Some(command) = parse_local_command(&input) {
        if execute_local_command(command, app, agent_slot, oauth_manager).await? {
            return Ok(true);
        }
    } else if input.trim_start().starts_with('/') {
        app.push_notice(format!("Unknown command '{}'. Use /help.", input.trim()));
    } else if let Some(agent) = agent_slot.take() {
        let mut agent = agent;
        if app.pending_request_input().is_some() {
            if app.has_local_pending_request_input() {
                let interaction = app.pending_request_input().cloned();
                if let Some(interaction) = interaction {
                    let source = interaction
                        .source
                        .clone()
                        .unwrap_or_else(|| "sub-agent".to_string());
                    let answer = input.trim().to_string();
                    app.record_completed_interaction(
                        crate::tui::state::InteractionKind::RequestInput,
                        interaction.title.clone(),
                        format!("Answered with: {}", answer),
                        interaction.source.clone(),
                    );
                    app.clear_local_request_input();
                    let mut prompt = format!(
                        "Continue the same task. A delegated {source} requested additional user input.\nQuestion: {}\nAnswer: {}",
                        interaction.title, answer
                    );
                    if let Some(note) = interaction.note.as_deref() {
                        if !note.trim().is_empty() {
                            prompt.push_str(&format!("\nContext: {}", note.trim()));
                        }
                    }
                    start_query_task(app, prompt, agent);
                    return Ok(false);
                }
            } else {
                agent.consume_pending_user_input(input.trim());
            }
        }
        let prompt = input.trim().to_string();
        if should_suggest_planning_mode(app, prompt.as_str()) {
            app.queue_planning_suggestion(prompt);
            *agent_slot = Some(agent);
        } else {
            app.clear_pending_planning_suggestion();
            start_query_task(app, prompt, agent);
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingPlanApprovalAction {
    StartImplementation,
    ContinuePlanning,
}

fn classify_pending_plan_approval_input(input: &str) -> Option<PendingPlanApprovalAction> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lowered = trimmed.to_ascii_lowercase();
    let continue_planning_keywords = [
        "继续规划",
        "继续计划",
        "继续 refine",
        "refine plan",
        "continue planning",
        "keep planning",
        "revise plan",
        "调整计划",
        "完善计划",
    ];
    if continue_planning_keywords
        .iter()
        .any(|keyword| lowered.contains(&keyword.to_ascii_lowercase()) || trimmed.contains(keyword))
    {
        return Some(PendingPlanApprovalAction::ContinuePlanning);
    }

    let approve_keywords = [
        "继续",
        "继续吧",
        "好的",
        "好",
        "开始",
        "开始吧",
        "执行",
        "执行吧",
        "实现",
        "实现吧",
        "可以",
        "行",
        "ok",
        "okay",
        "yes",
        "y",
        "go",
        "proceed",
        "continue",
        "ship it",
    ];
    if approve_keywords
        .iter()
        .any(|keyword| lowered == keyword.to_ascii_lowercase() || trimmed == *keyword)
    {
        return Some(PendingPlanApprovalAction::StartImplementation);
    }

    None
}

async fn handle_pending_plan_approval_submit(
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    input: &str,
) -> anyhow::Result<bool> {
    let Some(action) = classify_pending_plan_approval_input(input) else {
        app.push_notice(
            "A plan is waiting for approval. Press 1/2 or type '继续' to implement, '继续规划' to refine the plan.",
        );
        return Ok(true);
    };

    match action {
        PendingPlanApprovalAction::StartImplementation => {
            if let Some(agent) = agent_slot.take() {
                start_plan_approval_resume_task(app, false, agent);
            } else {
                app.push_notice("No active agent is available to start implementation.");
            }
        }
        PendingPlanApprovalAction::ContinuePlanning => {
            if let Some(agent) = agent_slot.take() {
                start_plan_approval_resume_task(app, true, agent);
            } else {
                app.push_notice("No active agent is available to continue planning.");
            }
        }
    }

    Ok(true)
}

fn clamp_command_palette_selection(app: &mut TuiApp) {
    let len = palette_commands(app, app.input.trim_start().trim_start_matches('/')).len();
    if len == 0 {
        app.command_palette_idx = 0;
    } else if app.command_palette_idx >= len {
        app.command_palette_idx = len - 1;
    }
}

fn sync_codex_credential_from_auth_store(
    app: &mut TuiApp,
    oauth_manager: &OAuthManager,
) -> anyhow::Result<bool> {
    let codex_state = app.config.provider_states.get("codex");
    let has_ready_codex_state = codex_state
        .and_then(|state| state.api_key.as_ref())
        .is_some_and(|api_key| !api_key.expose_secret().trim().is_empty())
        && codex_state
            .and_then(|state| state.model.as_deref())
            .is_some_and(|model| !crate::config::should_reset_codex_model(Some(model)))
        && codex_state
            .map(|state| !crate::config::should_reset_codex_base_url(state.base_url.as_deref()))
            .unwrap_or(false);
    if has_ready_codex_state {
        return Ok(true);
    }

    if !oauth_manager.has_saved_auth()? {
        return Ok(false);
    }

    let credential = oauth_manager.load_saved_credential()?;
    let credential = credential.expose_secret().trim().to_string();
    let mut changed = false;

    if app.config.provider == "codex" {
        let current_key = app
            .config
            .api_key
            .as_ref()
            .map(|value| value.expose_secret().trim());
        if current_key != Some(credential.as_str()) {
            app.config.set_api_key(credential.clone());
            changed = true;
        }
        if crate::config::should_reset_codex_model(app.config.model.as_deref()) {
            app.config
                .set_model(Some(crate::config::DEFAULT_CODEX_MODEL.to_string()));
            changed = true;
        }
        if crate::config::should_reset_codex_base_url(app.config.base_url.as_deref()) {
            app.config
                .set_base_url(Some(crate::config::DEFAULT_CODEX_BASE_URL.to_string()));
            changed = true;
        }
    } else {
        let mut codex_state = app
            .config
            .provider_states
            .get("codex")
            .cloned()
            .unwrap_or_default();
        let current_key = codex_state
            .api_key
            .as_ref()
            .map(|value| value.expose_secret().trim());
        if current_key != Some(credential.as_str()) {
            codex_state.api_key = Some(SecretString::from(credential));
            changed = true;
        }
        if crate::config::should_reset_codex_model(codex_state.model.as_deref()) {
            codex_state.model = Some(crate::config::DEFAULT_CODEX_MODEL.to_string());
            changed = true;
        }
        if crate::config::should_reset_codex_base_url(codex_state.base_url.as_deref()) {
            codex_state.base_url = Some(crate::config::DEFAULT_CODEX_BASE_URL.to_string());
            changed = true;
        }
        if changed {
            app.config
                .provider_states
                .insert("codex".to_string(), codex_state);
        }
    }

    if changed {
        app.config_manager.save(&app.config)?;
    }

    Ok(true)
}

fn codex_auth_is_available(app: &TuiApp, oauth_manager: &OAuthManager) -> bool {
    if app.config.provider == "codex" && app.config.has_api_key() {
        return true;
    }
    if app
        .config
        .provider_states
        .get("codex")
        .and_then(|state| state.api_key.as_ref())
        .is_some_and(|api_key| !api_key.expose_secret().trim().is_empty())
    {
        return true;
    }
    oauth_manager.has_saved_auth().is_ok_and(|saved| saved)
}

async fn refresh_codex_model_picker(
    app: &mut TuiApp,
    oauth_manager: &OAuthManager,
    refresh_strategy: RefreshStrategy,
) -> anyhow::Result<()> {
    let options = load_codex_model_catalog(oauth_manager.codex_home(), refresh_strategy).await?;
    if options.is_empty() {
        app.push_notice("Codex model catalog is empty. Check the saved login or try again.");
    }
    app.set_codex_model_options(options);
    Ok(())
}

async fn open_provider_family_overlay(
    app: &mut TuiApp,
    oauth_manager: &OAuthManager,
) -> anyhow::Result<()> {
    let entering_codex_family = matches!(
        app.selected_provider_family(),
        self::state::ProviderFamily::Codex
    );
    if entering_codex_family {
        oauth_manager.invalidate_saved_auth_cache();
    }
    let has_synced_codex_auth = if entering_codex_family {
        sync_codex_credential_from_auth_store(app, oauth_manager)?
    } else {
        false
    };

    if entering_codex_family
        && !has_synced_codex_auth
        && !codex_auth_is_available(app, oauth_manager)
    {
        app.config.set_provider("codex");
        app.open_overlay(Overlay::AuthModePicker);
    } else {
        if entering_codex_family {
            refresh_codex_model_picker(app, oauth_manager, RefreshStrategy::OnlineIfUncached)
                .await?;
        }
        app.open_overlay(Overlay::ModelPicker);
    }
    Ok(())
}

fn should_open_codex_auth_guide(app: &TuiApp, oauth_manager: &OAuthManager) -> bool {
    app.selected_provider_family() == self::state::ProviderFamily::Codex
        && !codex_auth_is_available(app, oauth_manager)
}

#[cfg(test)]
mod tests {
    use super::{
        classify_pending_plan_approval_input, codex_auth_is_available, dispatch_event,
        map_key_to_event, open_provider_family_overlay, sync_codex_credential_from_auth_store,
        PendingPlanApprovalAction,
    };
    use crate::codex_model_catalog::{CodexModelOption, CodexReasoningOption};
    use crate::config::ConfigManager;
    use crate::config::{DEFAULT_CODEX_BASE_URL, DEFAULT_CODEX_MODEL};
    use crate::tui::app_event::AppEvent;
    use crate::tui::state::{Overlay, ProviderFamily, RunningTask, TaskKind, TuiApp};
    use crossterm::event::KeyCode;
    use secrecy::ExposeSecret;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    #[test]
    fn pending_plan_approval_treats_generic_continue_as_approval() {
        assert_eq!(
            classify_pending_plan_approval_input("继续吧"),
            Some(PendingPlanApprovalAction::StartImplementation)
        );
        assert_eq!(
            classify_pending_plan_approval_input("ok"),
            Some(PendingPlanApprovalAction::StartImplementation)
        );
    }

    #[test]
    fn pending_plan_approval_supports_explicit_refine_signal() {
        assert_eq!(
            classify_pending_plan_approval_input("继续规划"),
            Some(PendingPlanApprovalAction::ContinuePlanning)
        );
        assert_eq!(
            classify_pending_plan_approval_input("continue planning"),
            Some(PendingPlanApprovalAction::ContinuePlanning)
        );
    }

    #[tokio::test]
    async fn busy_submit_queues_follow_up_message() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        app.input = "continue with the follow-up".into();

        let (_sender, receiver) = mpsc::unbounded_channel();
        app.running_task = Some(RunningTask {
            kind: TaskKind::Query,
            receiver,
            handle: tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                unreachable!()
            }),
            started_at: Instant::now(),
            next_heartbeat_after_secs: 2,
        });

        let mut agent_slot = None;
        let oauth_manager = Arc::new(
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager"),
        );
        let should_quit = super::handle_submit(&mut app, &mut agent_slot, &oauth_manager)
            .await
            .expect("submit");

        assert!(!should_quit);
        assert_eq!(
            app.queued_follow_up_preview(),
            Some("continue with the follow-up")
        );
        assert!(app
            .notice
            .as_deref()
            .is_some_and(|value| value.contains("Queued for after the next tool call boundary")));
        assert_eq!(
            app.pending_follow_up_preview(),
            Some("continue with the follow-up")
        );

        if let Some(task) = app.running_task.take() {
            task.handle.abort();
        }
    }

    #[tokio::test]
    async fn busy_submit_allows_quit_command() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        app.input = "/quit".into();

        let (_sender, receiver) = mpsc::unbounded_channel();
        app.running_task = Some(RunningTask {
            kind: TaskKind::OAuth,
            receiver,
            handle: tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                unreachable!()
            }),
            started_at: Instant::now(),
            next_heartbeat_after_secs: u64::MAX,
        });

        let mut agent_slot = None;
        let oauth_manager = Arc::new(
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager"),
        );
        let should_quit = super::handle_submit(&mut app, &mut agent_slot, &oauth_manager)
            .await
            .expect("submit");

        assert!(should_quit);

        if let Some(task) = app.running_task.take() {
            task.handle.abort();
        }
    }

    #[test]
    fn auth_mode_picker_prefers_selection_navigation() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        app.open_overlay(Overlay::AuthModePicker);

        assert!(matches!(
            map_key_to_event(KeyCode::Down, &app),
            AppEvent::MoveAuthModeSelection(1)
        ));
        assert!(matches!(
            map_key_to_event(KeyCode::Enter, &app),
            AppEvent::ApplyOverlaySelection
        ));
        assert!(matches!(
            map_key_to_event(KeyCode::Char('3'), &app),
            AppEvent::SetAuthModeSelection(2)
        ));
    }

    #[test]
    fn openai_compatible_model_picker_exposes_connection_edit_shortcuts() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");

        app.provider_picker_idx = 1;
        app.open_overlay(Overlay::ModelPicker);

        assert!(matches!(
            map_key_to_event(KeyCode::Char('b'), &app),
            AppEvent::OpenOverlay(Overlay::BaseUrlEditor)
        ));
        assert!(matches!(
            map_key_to_event(KeyCode::Char('a'), &app),
            AppEvent::OpenOverlay(Overlay::ApiKeyEditor)
        ));
        assert!(matches!(
            map_key_to_event(KeyCode::Char('n'), &app),
            AppEvent::OpenOverlay(Overlay::ModelNameEditor)
        ));
    }

    #[tokio::test]
    async fn save_api_key_input_allows_clearing_openai_compatible_credentials() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        app.config.set_provider("openai-compatible");
        app.config.set_api_key("sk-existing");
        app.open_overlay(Overlay::ApiKeyEditor);
        app.api_key_input.clear();

        let oauth_manager = Arc::new(
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager"),
        );
        let mut agent_slot = None;

        let should_quit = dispatch_event(
            AppEvent::SaveApiKeyInput,
            &mut app,
            &mut agent_slot,
            &oauth_manager,
        )
        .await
        .expect("save api key");

        assert!(!should_quit);
        assert_eq!(app.config.api_key(), None);
        assert!(app
            .notice
            .as_deref()
            .is_some_and(|value| value.contains("Cleared API key")));
    }

    #[test]
    fn codex_auth_detection_uses_saved_auth_storage() {
        let temp = tempdir().expect("tempdir");
        let app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        let oauth_manager =
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager");

        assert!(!codex_auth_is_available(&app, &oauth_manager));

        oauth_manager
            .save_api_key("sk-test-codex")
            .expect("save api key");
        assert!(codex_auth_is_available(&app, &oauth_manager));
    }

    #[tokio::test]
    async fn codex_provider_family_routes_to_auth_picker_without_saved_login() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        let oauth_manager =
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager");
        app.provider_picker_idx = 0;

        assert_eq!(app.selected_provider_family(), ProviderFamily::Codex);

        open_provider_family_overlay(&mut app, &oauth_manager)
            .await
            .expect("open overlay");
        assert_eq!(app.config.provider, "codex");
        assert!(matches!(app.overlay, Some(Overlay::AuthModePicker)));
    }

    #[tokio::test]
    async fn codex_provider_family_routes_to_model_picker_with_saved_login() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        let oauth_manager =
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager");
        oauth_manager
            .save_api_key("sk-test-codex")
            .expect("save api key");
        app.provider_picker_idx = 0;

        open_provider_family_overlay(&mut app, &oauth_manager)
            .await
            .expect("open overlay");
        assert!(matches!(app.overlay, Some(Overlay::ModelPicker)));
        assert!(!app.codex_model_options.is_empty());
    }

    #[tokio::test]
    async fn codex_provider_family_uses_saved_codex_provider_state() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        let oauth_manager =
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager");

        app.config.set_provider("ollama");
        app.config.set_api_key("sk-ollama");
        app.config.set_provider("codex");
        app.config.set_api_key("sk-codex");
        app.config.set_provider("ollama");
        app.provider_picker_idx = 0;

        assert!(codex_auth_is_available(&app, &oauth_manager));

        open_provider_family_overlay(&mut app, &oauth_manager)
            .await
            .expect("open overlay");
        assert!(matches!(app.overlay, Some(Overlay::ModelPicker)));
    }

    #[tokio::test]
    async fn codex_model_picker_opens_reasoning_level_overlay_before_rebuild() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        let oauth_manager = Arc::new(
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager"),
        );
        oauth_manager
            .save_api_key("sk-test-codex")
            .expect("save api key");

        app.provider_picker_idx = 0;
        open_provider_family_overlay(&mut app, &oauth_manager)
            .await
            .expect("open overlay");
        app.overlay = Some(Overlay::ModelPicker);

        let mut agent_slot = None;
        dispatch_event(
            AppEvent::ApplyOverlaySelection,
            &mut app,
            &mut agent_slot,
            &oauth_manager,
        )
        .await
        .expect("apply model selection");

        assert!(matches!(app.overlay, Some(Overlay::ReasoningEffortPicker)));
    }

    #[tokio::test]
    async fn codex_model_picker_applies_single_reasoning_level_without_overlay() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        app.provider_picker_idx = 0;
        app.config.set_provider("codex");
        app.set_codex_model_options(vec![CodexModelOption {
            id: "gpt-5.2-codex".to_string(),
            model: "gpt-5.2-codex".to_string(),
            label: "gpt-5.2-codex".to_string(),
            description: "Frontier agentic coding model.".to_string(),
            default_reasoning_effort: Some("high".to_string()),
            reasoning_options: vec![CodexReasoningOption {
                value: "high".to_string(),
                label: "High".to_string(),
                description: "Maximize reasoning depth.".to_string(),
                is_default: true,
            }],
            is_default: true,
        }]);
        app.overlay = Some(Overlay::ModelPicker);

        let oauth_manager = Arc::new(
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager"),
        );
        oauth_manager
            .save_api_key("sk-test-codex")
            .expect("save api key");
        let mut agent_slot = None;

        dispatch_event(
            AppEvent::ApplyOverlaySelection,
            &mut app,
            &mut agent_slot,
            &oauth_manager,
        )
        .await
        .expect("apply model selection");

        assert_eq!(app.config.model.as_deref(), Some("gpt-5.2-codex"));
        assert_eq!(app.config.reasoning_effort.as_deref(), Some("high"));
        assert!(matches!(
            app.running_task.as_ref(),
            Some(task) if matches!(task.kind, TaskKind::Rebuild)
        ));
        if let Some(task) = app.running_task.take() {
            task.handle.abort();
        }
    }

    #[test]
    fn codex_auth_store_is_synced_into_config_before_model_flow() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        let oauth_manager =
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager");
        oauth_manager
            .save_api_key("sk-test-codex")
            .expect("save api key");

        app.config.set_provider("ollama");
        app.provider_picker_idx = 0;

        assert!(
            sync_codex_credential_from_auth_store(&mut app, &oauth_manager).expect("sync auth")
        );
        assert_eq!(
            app.config
                .provider_states
                .get("codex")
                .and_then(|state| state.api_key.as_ref())
                .map(|value| value.expose_secret()),
            Some("sk-test-codex")
        );
        assert_eq!(app.config.provider, "ollama");

        let persisted = app.config_manager.load().expect("load saved config");
        assert_eq!(persisted.provider, "ollama");
        assert_eq!(
            persisted
                .provider_states
                .get("codex")
                .and_then(|state| state.api_key.as_ref())
                .map(|value| value.expose_secret()),
            Some("sk-test-codex")
        );
    }

    #[tokio::test]
    async fn save_api_key_input_sets_codex_defaults_before_rebuild() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        app.config.set_provider("codex");
        app.open_overlay(Overlay::ApiKeyEditor);
        app.api_key_input = "sk-codex".into();

        let oauth_manager = Arc::new(
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager"),
        );
        let mut agent_slot = None;

        let should_quit = dispatch_event(
            AppEvent::SaveApiKeyInput,
            &mut app,
            &mut agent_slot,
            &oauth_manager,
        )
        .await
        .expect("save codex api key");

        assert!(!should_quit);
        assert_eq!(app.config.model.as_deref(), Some(DEFAULT_CODEX_MODEL));
        assert_eq!(app.config.base_url.as_deref(), Some(DEFAULT_CODEX_BASE_URL));
    }
}
