use super::*;

#[test]
fn committed_turn_cell_keeps_user_summary_and_agent_sections_in_order() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Review this repo".into(),
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "list_files .".into(),
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "bash cargo check".into(),
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "Final recommendation".into(),
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let you_idx = rendered.find("You: Review this repo").unwrap();
    let ran_idx = rendered.find(" Ran ").unwrap();
    let agent_idx = rendered.find("Agent\n  Final recommendation").unwrap();

    assert!(!rendered.contains(" Explored "));
    assert!(ran_idx < agent_idx);
    assert!(you_idx < ran_idx);
}

#[test]
fn committed_turn_cell_ignores_routine_system_notices() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Review this repo".into(),
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "Final recommendation".into(),
        },
        TranscriptEntry {
            role: "System".into(),
            message: "prompt finished".into(),
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Agent\n  Final recommendation"));
    assert!(!rendered.contains("System"));
    assert!(!rendered.contains("prompt finished"));
}

#[test]
fn committed_turn_cell_renders_materialized_sidecar_sections() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Review the workspace logic".into(),
        },
        TranscriptEntry {
            role: "Exploring".into(),
            message:
                "Delegate repository exploration: inspect instruction discovery\nSub-agent summary: current discovery is hardcoded"
                    .into(),
        },
        TranscriptEntry {
            role: "Planning".into(),
            message:
                "Delegate plan refinement: generalize instruction discovery\nSub-agent summary: reuse the workspace traversal helper"
                    .into(),
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "Here is the final recommendation.".into(),
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let you_idx = rendered.find("You: Review the workspace logic").unwrap();
    let explored_idx = rendered.find(" Explored ").unwrap();
    let planning_idx = rendered.find(" Planned ").unwrap();
    let agent_idx = rendered
        .find("Agent\n  Here is the final recommendation.")
        .unwrap();

    assert!(you_idx < explored_idx);
    assert!(explored_idx < planning_idx);
    assert!(planning_idx < agent_idx);
    assert!(rendered.contains("Sub-agent summary: current discovery is hardcoded"));
    assert!(rendered.contains("Sub-agent summary: reuse the workspace traversal helper"));
}

#[test]
fn committed_turn_cell_places_completion_records_before_final_agent_message() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Inspect the repo and decide whether to run the migration".into(),
        },
        TranscriptEntry {
            role: "Exploring".into(),
            message: "└ Read crates/instructions/src/workspace.rs".into(),
        },
        TranscriptEntry {
            role: "Shell Approval Completed".into(),
            message: "Bash approval: Approved once for command: bash ./scripts/migrate.sh"
                .into(),
        },
        TranscriptEntry {
            role: "Agent".into(),
            message:
                "I approved the one-off shell step and can now continue with the final recommendation."
                    .into(),
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let explored_idx = rendered.find(" Explored ").unwrap();
    let approval_idx = rendered.find(" Shell Approval Completed ").unwrap();
    let agent_idx = rendered
        .find("Agent\n  I approved the one-off shell step")
        .unwrap();

    assert!(explored_idx < approval_idx);
    assert!(approval_idx < agent_idx);
}

#[test]
fn committed_turn_cell_orders_completion_records_by_interaction_kind() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Inspect the workflow and capture the decision trail".into(),
        },
        TranscriptEntry {
            role: "Question Answered".into(),
            message: "Captured the generic answer.".into(),
        },
        TranscriptEntry {
            role: "Plan Decision".into(),
            message: "Approved the proposed implementation plan.".into(),
        },
        TranscriptEntry {
            role: "Shell Approval Completed".into(),
            message: "Approved the one-off shell command.".into(),
        },
        TranscriptEntry {
            role: "Planning Question Answered".into(),
            message: "Chose the plan_agent option.".into(),
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "Here is the final narrative summary.".into(),
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let shell_idx = rendered.find(" Shell Approval Completed ").unwrap();
    let plan_idx = rendered.find(" Plan Decision ").unwrap();
    let planning_question_idx = rendered.find(" Planning Question Answered ").unwrap();
    let generic_question_idx = rendered.find(" Question Answered ").unwrap();
    let agent_idx = rendered
        .find("Agent\n  Here is the final narrative summary.")
        .unwrap();

    assert!(shell_idx < plan_idx);
    assert!(plan_idx < planning_question_idx);
    assert!(planning_question_idx < generic_question_idx);
    assert!(generic_question_idx < agent_idx);
}
