use super::state::{ActivePendingInteractionKind, TuiApp};

pub fn pending_interaction_section_title(kind: ActivePendingInteractionKind) -> &'static str {
    match kind {
        ActivePendingInteractionKind::PlanApproval => " Plan Approval ",
        ActivePendingInteractionKind::ShellApproval => " Shell Approval ",
        ActivePendingInteractionKind::PlanningQuestion => " Planning Question ",
        ActivePendingInteractionKind::ExplorationQuestion => " Exploration Question ",
        ActivePendingInteractionKind::SubAgentQuestion => " Sub-agent Question ",
        ActivePendingInteractionKind::RequestInput => " Request Input ",
    }
}

pub fn pending_interaction_card_title(kind: ActivePendingInteractionKind) -> &'static str {
    match kind {
        ActivePendingInteractionKind::PlanApproval => "Awaiting Approval",
        ActivePendingInteractionKind::ShellApproval => "Shell Approval",
        ActivePendingInteractionKind::PlanningQuestion => "Planning Question",
        ActivePendingInteractionKind::ExplorationQuestion => "Exploration Question",
        ActivePendingInteractionKind::SubAgentQuestion => "Sub-agent Question",
        ActivePendingInteractionKind::RequestInput => "Request Input",
    }
}

pub fn pending_interaction_hint_text(kind: ActivePendingInteractionKind) -> &'static str {
    match kind {
        ActivePendingInteractionKind::PlanApproval => "type reply  1 yes  2 plan",
        ActivePendingInteractionKind::ShellApproval => "type reply  1 yes  2 on  3 no",
        ActivePendingInteractionKind::PlanningQuestion
        | ActivePendingInteractionKind::ExplorationQuestion
        | ActivePendingInteractionKind::SubAgentQuestion
        | ActivePendingInteractionKind::RequestInput => "type reply  1/2/3 shortcut",
    }
}

pub fn status_plan_approval_text(app: &TuiApp) -> String {
    let _ = app;
    "Start implementation with this plan?\n\n1. yes\n2. keep planning".to_string()
}

pub fn pending_interaction_shortcut_text(kind: ActivePendingInteractionKind) -> &'static str {
    match kind {
        ActivePendingInteractionKind::PlanApproval => "1 yes  2 plan",
        ActivePendingInteractionKind::ShellApproval => "1 yes  2 on  3 no",
        ActivePendingInteractionKind::PlanningQuestion
        | ActivePendingInteractionKind::ExplorationQuestion
        | ActivePendingInteractionKind::SubAgentQuestion
        | ActivePendingInteractionKind::RequestInput => "1/2/3 shortcut",
    }
}

pub fn pending_interaction_detail_text(app: &TuiApp, kind: ActivePendingInteractionKind) -> String {
    match kind {
        ActivePendingInteractionKind::PlanApproval => status_plan_approval_text(app),
        ActivePendingInteractionKind::ShellApproval => status_command_approval_text(app),
        ActivePendingInteractionKind::PlanningQuestion
        | ActivePendingInteractionKind::ExplorationQuestion
        | ActivePendingInteractionKind::SubAgentQuestion
        | ActivePendingInteractionKind::RequestInput => status_request_user_input_text(app),
    }
}

pub fn status_active_pending_interaction_text(app: &TuiApp) -> Option<(&'static str, String)> {
    let pending = app.active_pending_interaction()?;
    let title = pending_interaction_section_title(pending.kind);
    let text = pending_interaction_detail_text(app, pending.kind);
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
        .map(|source| format!("from: {source}\n\n"))
        .unwrap_or_default();

    if let Some(note) = interaction.note.as_deref() {
        format!(
            "{}{}\n\n{}\n\nnote: {}",
            source_block, interaction.title, options_text, note
        )
    } else {
        format!("{}{}\n\n{}", source_block, interaction.title, options_text)
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
        "command:\n{}\n\ncwd:\n{}\n\nnetwork:\n{}\n\nenv:\n{} override(s)\n\n1. yes\n2. on\n3. no",
        approval.command,
        cwd,
        if approval.allow_net {
            "allowed"
        } else {
            "disabled"
        },
        env_count,
    )
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::config::ConfigManager;
    use crate::tui::state::TuiApp;

    use super::{pending_interaction_hint_text, status_plan_approval_text};

    #[test]
    fn plan_approval_text_uses_compact_choice_labels() {
        let temp = tempdir().unwrap();
        let app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");

        let rendered = status_plan_approval_text(&app);
        assert!(rendered.contains("1. yes"));
        assert!(rendered.contains("2. keep planning"));
        assert!(!rendered.contains("Start implementation now"));
    }

    #[test]
    fn pending_interaction_hint_text_is_compact_for_plan_and_shell_approval() {
        assert_eq!(
            pending_interaction_hint_text(
                crate::tui::state::ActivePendingInteractionKind::PlanApproval
            ),
            "type reply  1 yes  2 plan"
        );
        assert_eq!(
            pending_interaction_hint_text(
                crate::tui::state::ActivePendingInteractionKind::ShellApproval
            ),
            "type reply  1 yes  2 on  3 no"
        );
    }
}
