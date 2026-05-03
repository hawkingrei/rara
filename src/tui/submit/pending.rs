use crate::agent::{Agent, BashApprovalDecision};
use crate::tui::runtime::{
    start_pending_approval_task, start_plan_approval_resume_task, start_query_task,
};
use crate::tui::state::{ActivePendingInteractionKind, InteractionKind, TuiApp};

pub(super) fn handle_pending_option_submit(
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    trimmed: &str,
) -> bool {
    let Some(index) = pending_option_index_from_text(trimmed) else {
        return false;
    };
    if index >= app.active_pending_option_count() {
        return false;
    }
    let Some(interaction) = app.active_pending_interaction() else {
        return false;
    };
    match interaction.kind {
        ActivePendingInteractionKind::PlanApproval => {
            if let Some(agent) = agent_slot.take() {
                start_plan_approval_resume_task(app, index == 1, agent);
            } else {
                app.push_notice("Approval is still preparing. Try the shortcut again.");
            }
            true
        }
        ActivePendingInteractionKind::ShellApproval => {
            if let Some(agent) = agent_slot.take() {
                let selection = match index {
                    0 => BashApprovalDecision::Once,
                    1 => BashApprovalDecision::Prefix,
                    2 => BashApprovalDecision::Always,
                    _ => BashApprovalDecision::Suggestion,
                };
                start_pending_approval_task(app, selection, agent);
            } else {
                app.push_notice("Approval is still preparing. Try the shortcut again.");
            }
            true
        }
        ActivePendingInteractionKind::PlanningQuestion
        | ActivePendingInteractionKind::ExplorationQuestion
        | ActivePendingInteractionKind::SubAgentQuestion
        | ActivePendingInteractionKind::RequestInput => {
            if let Some(label) = app.pending_question_option_label(index) {
                if let Some(agent) = agent_slot.take() {
                    handle_request_input_answer(app, agent_slot, agent, label);
                } else {
                    app.push_notice("Request input is still preparing. Try the shortcut again.");
                }
                return true;
            }
            false
        }
    }
}

pub(super) fn handle_request_input_answer(
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    mut agent: Agent,
    answer: String,
) {
    if app.has_local_pending_request_input() {
        start_local_request_input_continuation(app, agent, answer);
        return;
    }

    agent.consume_pending_user_input(&answer);
    app.sync_snapshot(&agent);
    app.clear_pending_planning_suggestion();
    start_query_task(app, answer, agent);
    *agent_slot = None;
}

fn start_local_request_input_continuation(app: &mut TuiApp, agent: Agent, answer: String) {
    let Some(interaction) = app.pending_request_input().cloned() else {
        app.clear_pending_planning_suggestion();
        start_query_task(app, answer, agent);
        return;
    };

    let source = interaction
        .source
        .clone()
        .unwrap_or_else(|| "sub-agent".to_string());
    app.record_completed_interaction(
        InteractionKind::RequestInput,
        interaction.title.clone(),
        format!("Answered with: {}", answer),
        interaction.source.clone(),
    );
    app.clear_local_request_input();

    let mut prompt = format!(
        "Continue the parent task after a delegated {source} requested additional user input.\nQuestion: {}\nAnswer: {}\n\nUse the delegated result already present in the transcript as context; do not assume the child sub-agent session is still attached.",
        interaction.title, answer
    );
    if let Some(note) = interaction.note.as_deref()
        && !note.trim().is_empty()
    {
        prompt.push_str(&format!("\nContext: {}", note.trim()));
    }
    start_query_task(app, prompt, agent);
}

fn pending_option_index_from_text(input: &str) -> Option<usize> {
    match input.trim().parse::<usize>() {
        Ok(index @ 1..=9) => Some(index - 1),
        _ => None,
    }
}
