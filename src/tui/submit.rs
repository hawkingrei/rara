use std::sync::Arc;

use crate::agent::Agent;

use super::command::{palette_command_by_index, parse_local_command};
use super::runtime::{execute_local_command, start_plan_approval_resume_task, start_query_task};
use super::state::{LocalCommandKind, OpenAiModelPickerAction, Overlay, TaskKind, TuiApp};

pub(crate) async fn handle_submit(
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<crate::oauth::OAuthManager>,
) -> anyhow::Result<bool> {
    if matches!(app.overlay, Some(Overlay::CommandPalette)) {
        let query = app.command_query();
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

pub(crate) fn apply_openai_model_picker_action(
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
                    super::runtime::start_rebuild_task(app);
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
                    super::runtime::start_rebuild_task(app);
                }
            } else {
                app.push_notice("Cannot delete the only endpoint profile.");
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingPlanApprovalAction {
    StartImplementation,
    ContinuePlanning,
}

pub(crate) fn classify_pending_plan_approval_input(
    input: &str,
) -> Option<PendingPlanApprovalAction> {
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
    ];
    if continue_planning_keywords
        .iter()
        .any(|keyword| lowered.contains(&keyword.to_ascii_lowercase()) || trimmed == *keyword)
    {
        return Some(PendingPlanApprovalAction::ContinuePlanning);
    }

    let approve_keywords = [
        "执行计划",
        "开始执行",
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

pub(crate) fn clamp_command_palette_selection(app: &mut TuiApp) {
    let len =
        super::command::palette_commands(app, app.command_query()).len();
    if len == 0 {
        app.command_palette_idx = 0;
    } else if app.command_palette_idx >= len {
        app.command_palette_idx = len - 1;
    }
}
