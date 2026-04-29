mod app_event;
mod auth_mode_picker;
mod command;
mod custom_terminal;
mod event_stream;
mod highlight;
mod insert_history;
mod interaction_text;
mod keymap;
mod line_utils;
mod markdown;
mod markdown_render;
mod markdown_stream;
mod plan_display;
mod provider_flow;
mod queued_input;
mod render;
mod runtime;
mod session_restore;
mod state;
mod terminal_event;
mod terminal_ui;
#[cfg(test)]
mod tests;
mod tool_text;

use std::sync::Arc;

use crossterm::{event::EventStream, terminal::enable_raw_mode, terminal::size as terminal_size};
use futures::StreamExt;
use tokio::time::{Duration, interval};

use crate::agent::Agent;
use crate::oauth::OAuthManager;
use crate::state_db::StateDb;

use self::app_event::AppEvent;
use self::command::{palette_command_by_index, palette_commands, parse_local_command};
use self::event_stream::{UiEvent, translate_event};
use self::keymap::map_key_to_event;
use self::provider_flow::{
    open_provider_family_overlay, should_open_codex_auth_guide,
    sync_codex_credential_from_auth_store,
};
use self::render::{desired_viewport_height, render};
use self::runtime::{
    execute_local_command, finish_running_task_if_ready, request_running_task_cancellation,
    start_deepseek_model_list_task, start_oauth_task, start_pending_approval_task,
    start_plan_approval_resume_task, start_query_task, start_rebuild_task,
};
use self::session_restore::{
    provider_requires_api_key, restore_latest_thread, restore_thread_by_id,
};
use self::state::{
    LocalCommandKind, OpenAiModelPickerAction, Overlay, PROVIDER_FAMILIES, TaskKind, TuiApp,
};
use self::terminal_ui::{
    build_terminal, flush_committed_history, handle_paste, is_ssh_session, teardown_terminal,
    update_terminal_viewport,
};
use crate::agent::AgentExecutionMode;
use crate::agent::BashApprovalDecision;

#[derive(Debug, Clone)]
pub enum StartupResumeTarget {
    Fresh,
    Latest,
    ThreadId(String),
    Picker,
}

pub async fn run_tui(
    agent: Agent,
    oauth_manager: OAuthManager,
    startup_resume: StartupResumeTarget,
) -> anyhow::Result<Option<String>> {
    enable_raw_mode()?;
    let initial_size = terminal_size()?;
    let mut app = TuiApp::new(crate::config::ConfigManager::new()?)?;
    app.terminal_width = initial_size.0;
    let viewport_height = desired_viewport_height(&app, initial_size.0, initial_size.1);
    let mut terminal = build_terminal(viewport_height)?;
    let mut agent_slot = Some(agent);
    match StateDb::new() {
        Ok(state_db) => {
            let state_db = Arc::new(state_db);
            app.attach_state_db(state_db);
            match &startup_resume {
                StartupResumeTarget::Fresh => {}
                StartupResumeTarget::Latest => {
                    if let Some(state_db) = app.state_db.as_ref().cloned() {
                        restore_latest_thread(&state_db, &mut app, &mut agent_slot)?;
                    }
                }
                StartupResumeTarget::ThreadId(thread_id) => {
                    restore_thread_by_id(thread_id.as_str(), &mut app, &mut agent_slot)?;
                }
                StartupResumeTarget::Picker => {
                    app.open_overlay(Overlay::ResumePicker);
                }
            }
        }
        Err(err) => app.set_state_db_error(err.to_string()),
    }
    let oauth_manager = Arc::new(oauth_manager);
    app.codex_auth_mode = oauth_manager.saved_auth_mode().ok().flatten();
    let mut events = EventStream::new();
    let mut tick = interval(Duration::from_millis(100));

    if let Some(agent_ref) = agent_slot.as_ref() {
        app.sync_snapshot(agent_ref);
    }
    app.start_repo_context_detection();

    let result: anyhow::Result<()> = loop {
        app.finish_repo_context_task_if_ready().await;
        finish_running_task_if_ready(&mut app, &mut agent_slot).await?;
        clamp_command_palette_selection(&mut app);
        let size = terminal_size()?;
        app.terminal_width = size.0;
        let desired_height = desired_viewport_height(&app, size.0, size.1);
        match update_terminal_viewport(&mut terminal, desired_height) {
            Ok(()) => {}
            Err(err) => app.push_notice(format!("Skipped viewport update: {err}")),
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
                                Ok(()) => {}
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

    if let Some(handle) = app.repo_context_task.take() {
        handle.abort();
    }
    teardown_terminal(terminal)?;
    result?;

    let session_id = agent_slot
        .as_ref()
        .map(|agent| agent.session_id.clone())
        .filter(|session_id| !session_id.is_empty())
        .or_else(|| (!app.snapshot.session_id.is_empty()).then(|| app.snapshot.session_id.clone()));
    Ok(session_id)
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
        AppEvent::CancelRunningTask => request_running_task_cancellation(app),
        AppEvent::SubmitComposer => {
            if handle_submit(app, agent_slot, oauth_manager).await? {
                return Ok(true);
            }
        }
        AppEvent::InsertNewline => {
            app.insert_newline_in_composer();
        }
        AppEvent::InputChar(c) => {
            app.insert_active_input_char(c);
        }
        AppEvent::Backspace => {
            app.backspace_active_input();
        }
        AppEvent::DeleteForward => {
            app.delete_forward_active_input();
        }
        AppEvent::MoveCursorLeft => {
            app.move_active_input_cursor_left();
        }
        AppEvent::MoveCursorRight => {
            app.move_active_input_cursor_right();
        }
        AppEvent::MoveCursorHome => {
            app.move_active_input_cursor_home();
        }
        AppEvent::MoveCursorEnd => {
            app.move_active_input_cursor_end();
        }
        AppEvent::MoveCursorUp => {
            app.move_composer_cursor_up();
        }
        AppEvent::MoveCursorDown => {
            app.move_composer_cursor_down();
        }
        AppEvent::NavigateInputHistory(delta) => {
            app.navigate_input_history(delta);
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
            let len = app.recent_threads.len();
            if len > 0 {
                let next = (app.resume_picker_idx as i32 + delta).clamp(0, len as i32 - 1);
                app.resume_picker_idx = next as usize;
            }
        }
        AppEvent::MoveModelSelection(delta) => {
            let len = if matches!(app.overlay, Some(Overlay::OpenAiEndpointKindPicker)) {
                app.openai_endpoint_kind_count()
            } else {
                app.current_model_picker_len()
            };
            if len > 0 {
                let next = delta.clamp(-(len as i32), len as i32);
                if matches!(app.overlay, Some(Overlay::OpenAiEndpointKindPicker)) {
                    let idx = (app.openai_endpoint_kind_picker_idx as i32 + next)
                        .clamp(0, len as i32 - 1);
                    app.openai_endpoint_kind_picker_idx = idx as usize;
                } else {
                    let idx = (app.model_picker_idx as i32 + next).clamp(0, len as i32 - 1);
                    app.model_picker_idx = idx as usize;
                }
            }
        }
        AppEvent::MoveOpenAiProfileSelection(delta) => {
            let len = app.selected_openai_profiles().len() + 1;
            if len > 0 {
                let next = (app.openai_profile_picker_idx as i32 + delta).clamp(0, len as i32 - 1);
                app.openai_profile_picker_idx = next as usize;
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
            if !app.recent_threads.is_empty() {
                app.resume_picker_idx = idx.min(app.recent_threads.len() - 1);
            }
        }
        AppEvent::SetModelSelection(idx) => {
            let len = if matches!(app.overlay, Some(Overlay::OpenAiEndpointKindPicker)) {
                app.openai_endpoint_kind_count()
            } else {
                app.current_model_picker_len()
            };
            if len == 0 {
                return Ok(false);
            }
            if matches!(app.overlay, Some(Overlay::OpenAiEndpointKindPicker)) {
                app.openai_endpoint_kind_picker_idx = idx.min(len - 1);
                return Ok(true);
            }
            app.model_picker_idx = idx.min(len - 1);
            if matches!(app.overlay, Some(Overlay::ModelPicker))
                && app.selected_provider_family() != self::state::ProviderFamily::Codex
                && !app.is_busy()
            {
                if app.selected_provider_family() == self::state::ProviderFamily::OpenAiCompatible {
                    return Ok(false);
                } else if app.selected_provider_family() == self::state::ProviderFamily::DeepSeek {
                    return Ok(false);
                } else if should_open_codex_auth_guide(app, oauth_manager.as_ref()) {
                    app.select_local_model(app.model_picker_idx);
                    app.open_overlay(Overlay::AuthModePicker);
                } else {
                    app.select_local_model(app.model_picker_idx);
                    start_rebuild_task(app);
                }
            }
        }
        AppEvent::SetOpenAiProfileSelection(idx) => {
            let len = app.selected_openai_profiles().len() + 1;
            if len > 0 {
                app.openai_profile_picker_idx = idx.min(len - 1);
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
                                0 => BashApprovalDecision::Once,
                                1 => BashApprovalDecision::Prefix,
                                2 => BashApprovalDecision::Always,
                                _ => BashApprovalDecision::Suggestion,
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
                            app.set_input(label);
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
                if app.openai_setup_steps.is_empty() {
                    app.close_overlay();
                } else {
                    app.advance_openai_profile_setup();
                }
            }
        }
        AppEvent::SaveApiKeyInput => {
            let value = app.api_key_input.trim();
            if app.is_busy() {
                app.push_notice("Wait for the current task before saving the API key.");
            } else if value.is_empty() && app.config.provider == "codex" {
                app.push_notice("Enter a Codex API key or press Esc to go back.");
            } else if value.is_empty()
                && app.selected_provider_family() == self::state::ProviderFamily::DeepSeek
            {
                app.push_notice("Enter a DeepSeek API key or press Esc to go back.");
            } else if value.is_empty() && app.openai_setup_keep_empty_api_key {
                app.notice = Some("Kept existing API key for the current profile.".into());
                app.advance_openai_profile_setup();
            } else if value.is_empty() {
                app.config.clear_api_key();
                if app.config.provider == "codex" {
                    app.codex_auth_mode = None;
                }
                app.config_manager.save(&app.config)?;
                app.notice = Some("Cleared API key for the current provider.".into());
                if app.openai_setup_steps.is_empty() {
                    app.close_overlay();
                } else {
                    app.advance_openai_profile_setup();
                }
            } else {
                let was_deepseek =
                    app.selected_provider_family() == self::state::ProviderFamily::DeepSeek;
                app.config.set_api_key(value.to_string());
                if app.config.provider == "codex" {
                    app.codex_auth_mode = Some(crate::oauth::SavedCodexAuthMode::ApiKey);
                    app.config
                        .apply_codex_defaults_for_base_url(crate::config::DEFAULT_CODEX_BASE_URL);
                }
                app.config_manager.save(&app.config)?;
                if app.config.provider == "codex" {
                    app.notice = Some("Saved Codex API key. Rebuilding backend.".into());
                    app.overlay = None;
                    start_rebuild_task(app);
                } else if was_deepseek {
                    app.notice = Some("Saved DeepSeek API key. Loading models.".into());
                    app.overlay = None;
                    start_deepseek_model_list_task(app);
                } else {
                    app.notice = Some("Saved API key for the current provider.".into());
                    if app.openai_setup_steps.is_empty() {
                        app.close_overlay();
                    } else {
                        app.advance_openai_profile_setup();
                    }
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
                    if app.openai_setup_steps.is_empty() {
                        app.close_overlay();
                    } else {
                        app.advance_openai_profile_setup();
                    }
                }
            }
        }
        AppEvent::SaveOpenAiProfileLabelInput => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before creating a profile.");
            } else if app.selected_provider_family()
                != self::state::ProviderFamily::OpenAiCompatible
            {
                app.push_notice(
                    "OpenAI-compatible profiles are only available in that provider family.",
                );
            } else {
                let label = app.openai_profile_label_input.trim();
                if label.is_empty() {
                    app.push_notice("Enter a profile label or press Esc to go back.");
                } else if let Some(kind) = app
                    .openai_profile_label_kind
                    .or_else(|| app.selected_openai_profile_kind())
                {
                    let profile_id = app.next_openai_profile_id(kind, label);
                    app.config.select_openai_profile(profile_id, label, kind);
                    app.config_manager.save(&app.config)?;
                    app.notice = Some(format!("Created endpoint profile: {label}"));
                    app.openai_profile_label_kind = None;
                    app.begin_created_openai_profile_setup();
                }
            }
        }
        AppEvent::CreateOpenAiProfile => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before creating a profile.");
            } else if app.selected_provider_family()
                == self::state::ProviderFamily::OpenAiCompatible
            {
                app.begin_openai_profile_setup();
            }
        }
        AppEvent::EditOpenAiProfile => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before editing a profile.");
            } else if app.selected_provider_family()
                == self::state::ProviderFamily::OpenAiCompatible
            {
                if app.select_openai_model_picker_profile().is_some() {
                    app.config_manager.save(&app.config)?;
                    app.begin_edit_openai_profile_setup();
                }
            }
        }
        AppEvent::DeleteOpenAiProfile => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before deleting a profile.");
            } else {
                apply_openai_model_picker_action(app, OpenAiModelPickerAction::DeleteProfile)?;
            }
        }
        AppEvent::RefreshDeepSeekModels => {
            if app.is_busy() {
                app.push_notice("Wait for the current task before refreshing DeepSeek models.");
            } else if app.selected_provider_family() != self::state::ProviderFamily::DeepSeek {
                app.push_notice("DeepSeek model refresh is only available in DeepSeek.");
            } else if !app.config.has_api_key() {
                app.open_overlay(Overlay::ApiKeyEditor);
            } else {
                start_deepseek_model_list_task(app);
            }
        }
        AppEvent::SelectHelpTab(tab) => {
            app.open_overlay(Overlay::Help(tab));
        }
        AppEvent::ApplyOverlaySelection => match app.overlay {
            Some(Overlay::CommandPalette) => {
                let query = app.input.trim_start().trim_start_matches('/');
                if let Some(spec) = palette_command_by_index(app, query, app.command_palette_idx) {
                    app.set_input(spec.usage.to_string());
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
                    if app.selected_provider_family() == self::state::ProviderFamily::DeepSeek
                        && app.config.has_api_key()
                        && matches!(app.overlay, Some(Overlay::ModelPicker))
                    {
                        start_deepseek_model_list_task(app);
                    }
                }
            }
            Some(Overlay::OpenAiEndpointKindPicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else {
                    let kind = app.selected_openai_setup_kind();
                    app.set_openai_setup_kind(kind);
                    app.config_manager.save(&app.config)?;
                }
            }
            Some(Overlay::ResumePicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else if let Some(thread_id) = app
                    .recent_threads
                    .get(app.resume_picker_idx)
                    .map(|session| session.metadata.session_id.clone())
                {
                    restore_thread_by_id(thread_id.as_str(), app, agent_slot)?;
                    app.close_overlay();
                }
            }
            Some(Overlay::OpenAiProfilePicker) => {
                if app.is_busy() {
                    app.push_notice("A task is already running. Wait for it to finish.");
                } else if app.openai_profile_picker_idx == 0 {
                    app.openai_profile_label_kind = app.selected_openai_profile_kind();
                    app.open_overlay(Overlay::OpenAiProfileLabelEditor);
                } else if let Some((profile_id, label)) = app
                    .selected_openai_profiles()
                    .get(app.openai_profile_picker_idx - 1)
                    .cloned()
                {
                    if let Some(kind) = app.selected_openai_profile_kind() {
                        app.config
                            .select_openai_profile(profile_id, label.clone(), kind);
                        app.config_manager.save(&app.config)?;
                        app.notice = Some(format!("Selected endpoint profile: {label}"));
                        app.overlay = Some(Overlay::ModelPicker);
                    }
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
                    } else if app.selected_provider_family()
                        == self::state::ProviderFamily::OpenAiCompatible
                    {
                        if let Some(action) = app.selected_openai_model_picker_action() {
                            apply_openai_model_picker_action(app, action)?;
                        }
                    } else if app.selected_provider_family()
                        == self::state::ProviderFamily::DeepSeek
                    {
                        if app.selected_deepseek_api_key_action() {
                            app.open_overlay(Overlay::ApiKeyEditor);
                        } else if app.config.has_api_key() {
                            app.select_local_model(app.model_picker_idx);
                            start_rebuild_task(app);
                        } else {
                            app.open_overlay(Overlay::ApiKeyEditor);
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
                        app.codex_auth_mode = None;
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
            app.set_input(spec.usage.to_string());
        }
        app.close_overlay();
    }

    if app.input.is_empty() {
        return Ok(false);
    }
    let input = std::mem::take(&mut app.input);
    app.input_cursor_offset = None;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        return Ok(false);
    }
    app.record_input_history(&trimmed);

    if app.is_busy() {
        if trimmed.starts_with('/') {
            if let Some(command) = parse_local_command(&trimmed) {
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
                app.queue_follow_up_message_after_next_tool_boundary(trimmed.clone())
            } else {
                app.queue_follow_up_message(trimmed.clone())
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

    if app.has_pending_plan_approval() && !trimmed.starts_with('/') {
        if handle_pending_plan_approval_submit(app, agent_slot, &trimmed).await? {
            return Ok(false);
        }
    }
    if let Some(command) = parse_local_command(&trimmed) {
        if execute_local_command(command, app, agent_slot, oauth_manager).await? {
            return Ok(true);
        }
    } else if trimmed.starts_with('/') {
        app.push_notice(format!("Unknown command '{}'. Use /help.", trimmed));
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
                    let answer = trimmed.clone();
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
                agent.consume_pending_user_input(&trimmed);
            }
        }
        let prompt = trimmed;
        app.clear_pending_planning_suggestion();
        start_query_task(app, prompt, agent);
    }
    Ok(false)
}

fn apply_openai_model_picker_action(
    app: &mut TuiApp,
    action: OpenAiModelPickerAction,
) -> anyhow::Result<()> {
    match action {
        OpenAiModelPickerAction::SelectProfile => {
            if let Some(label) = app.select_openai_model_picker_profile() {
                app.config_manager.save(&app.config)?;
                if app.openai_profile_needs_setup() {
                    app.notice = Some(format!("Selected endpoint profile: {label}"));
                    app.begin_active_openai_profile_setup();
                } else {
                    start_rebuild_task(app);
                }
            }
        }
        OpenAiModelPickerAction::DeleteProfile => {
            if let Some(label) = app.delete_active_openai_profile() {
                app.config_manager.save(&app.config)?;
                if app.openai_profile_needs_setup() {
                    app.notice = Some(format!("Deleted endpoint profile: {label}"));
                    app.begin_active_openai_profile_setup();
                } else {
                    app.notice = Some(format!("Deleted endpoint profile: {label}"));
                    start_rebuild_task(app);
                }
            } else {
                app.push_notice("Cannot delete the only endpoint profile.");
            }
        }
    }
    Ok(())
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
        "执行计划",
        "开始实现",
        "实现计划",
        "approve plan",
        "implement plan",
        "start implementation",
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
            "A plan is waiting for approval. Press 1/2 or type '执行计划' to implement, '继续规划' to refine the plan.",
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
