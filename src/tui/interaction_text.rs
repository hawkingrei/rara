use super::state::{ActivePendingInteractionKind, TuiApp};

pub fn status_plan_approval_text(app: &TuiApp) -> String {
    let plan_summary = if app.snapshot.plan_steps.is_empty() {
        "No structured plan captured yet.".to_string()
    } else {
        app.snapshot
            .plan_steps
            .iter()
            .enumerate()
            .take(5)
            .map(|(idx, (_, step))| format!("{}. {}", idx + 1, step))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let explanation = app
        .snapshot
        .plan_explanation
        .as_deref()
        .unwrap_or("Review the proposed implementation plan before starting code changes.");

    format!(
        "ready to implement:\n{}\n\nsummary:\n{}\n\noptions:\n1. Start implementation now\n2. Continue planning",
        plan_summary, explanation
    )
}

pub fn status_active_pending_interaction_text(app: &TuiApp) -> Option<(&'static str, String)> {
    let pending = app.active_pending_interaction()?;
    let title = match pending.kind {
        ActivePendingInteractionKind::PlanApproval => " Plan Approval ",
        ActivePendingInteractionKind::ShellApproval => " Shell Approval ",
        ActivePendingInteractionKind::PlanningQuestion => " Planning Question ",
        ActivePendingInteractionKind::ExplorationQuestion => " Exploration Question ",
        ActivePendingInteractionKind::SubAgentQuestion => " Sub-agent Question ",
        ActivePendingInteractionKind::RequestInput => " Request Input ",
    };
    let text = match pending.kind {
        ActivePendingInteractionKind::PlanApproval => status_plan_approval_text(app),
        ActivePendingInteractionKind::ShellApproval => status_command_approval_text(app),
        ActivePendingInteractionKind::PlanningQuestion
        | ActivePendingInteractionKind::ExplorationQuestion
        | ActivePendingInteractionKind::SubAgentQuestion
        | ActivePendingInteractionKind::RequestInput => status_request_user_input_text(app),
    };
    Some((title, text))
}

pub fn status_planning_suggestion_text(app: &TuiApp) -> String {
    let _ = app.pending_planning_suggestion.as_deref();
    "suggestion:\nThis looks like a non-trivial task. Enter planning mode first so RARA can analyze the repository, refine the approach, and only stop once a concrete plan is ready.\n\noptions:\n1. Enter planning mode\n2. Continue in execute mode".to_string()
}

pub fn status_request_user_input_text(app: &TuiApp) -> String {
    let Some(interaction) = app.pending_request_input() else {
        return "No pending structured question.".to_string();
    };

    let options_text = if interaction.options.is_empty() {
        "No predefined options.".to_string()
    } else {
        interaction
            .options
            .iter()
            .enumerate()
            .map(|(idx, (label, description))| {
                if description.is_empty() {
                    format!("{}. {}", idx + 1, label)
                } else {
                    format!("{}. {} — {}", idx + 1, label, description)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let source_block = interaction
        .source
        .as_deref()
        .map(|source| format!("source:\n{source}\n\n"))
        .unwrap_or_default();

    if let Some(note) = interaction.note.as_deref() {
        format!(
            "{}question:\n{}\n\noptions:\n{}\n\nnote:\n{}",
            source_block, interaction.title, options_text, note
        )
    } else {
        format!(
            "{}question:\n{}\n\noptions:\n{}",
            source_block, interaction.title, options_text
        )
    }
}

pub fn status_command_approval_text(app: &TuiApp) -> String {
    let Some(interaction) = app.pending_command_approval() else {
        return "No pending shell approval.".to_string();
    };
    let Some(approval) = interaction.approval.as_ref() else {
        return "No pending shell approval.".to_string();
    };

    let cwd = approval
        .payload
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(".");
    let env_count = approval.payload.env.len();

    format!(
        "command:\n{}\n\ncwd:\n{}\n\nnetwork:\n{}\n\nenv:\n{} override(s)\n\noptions:\n1. Approve once\n2. Approve for session\n3. Keep as suggestion",
        approval.command,
        cwd,
        if approval.allow_net { "allowed" } else { "disabled" },
        env_count,
    )
}
