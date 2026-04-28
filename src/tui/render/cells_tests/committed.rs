use super::*;

#[test]
fn committed_turn_cell_keeps_user_summary_and_agent_sections_in_order() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Review this repo".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "list_files .".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "bash cargo check".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "Final recommendation".into(),
            payload: None,
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let you_idx = rendered.find("› Review this repo").unwrap();
    let ran_idx = rendered.find(" Ran ").unwrap();
    let agent_idx = rendered.find("• Final recommendation").unwrap();

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
            payload: None,
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "Final recommendation".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "System".into(),
            message: "prompt finished".into(),
            payload: None,
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("• Final recommendation"));
    assert!(!rendered.contains("System"));
    assert!(!rendered.contains("prompt finished"));
}

#[test]
fn committed_turn_cell_renders_materialized_sidecar_sections() {
    let entries = vec![
        TranscriptEntry { role: "You".into(), message: "Review the workspace logic".into(), payload: None },
        TranscriptEntry {
            role: "Exploring".into(),
            message:
                "Delegate repository exploration: inspect instruction discovery\nSub-agent summary: current discovery is hardcoded"
                    .into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Planning".into(),
            message:
                "Delegate plan refinement: generalize instruction discovery\nSub-agent summary: reuse the workspace traversal helper"
                    .into(),
            payload: None,
        },
        TranscriptEntry { role: "Agent".into(), message: "Here is the final recommendation.".into(), payload: None },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let you_idx = rendered.find("› Review the workspace logic").unwrap();
    let explored_idx = rendered.find(" Explored ").unwrap();
    let planning_idx = rendered.find(" Planned ").unwrap();
    let agent_idx = rendered
        .find("• Here is the final recommendation.")
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
        TranscriptEntry { role: "You".into(), message: "Inspect the repo and decide whether to run the migration".into(), payload: None },
        TranscriptEntry { role: "Exploring".into(), message: "└ Read crates/instructions/src/workspace.rs".into(), payload: None },
        TranscriptEntry { role: "Shell Approval Completed".into(), message: "Bash approval: Approved once for command: bash ./scripts/migrate.sh"
                .into(), payload: None },
        TranscriptEntry {
            role: "Agent".into(),
            message:
                "I approved the one-off shell step and can now continue with the final recommendation."
                    .into(),
            payload: None,
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
        .find("• I approved the one-off shell step")
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
            payload: None,
        },
        TranscriptEntry {
            role: "Question Answered".into(),
            message: "Captured the generic answer.".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Plan Decision".into(),
            message: "Approved the proposed implementation plan.".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Shell Approval Completed".into(),
            message: "Approved the one-off shell command.".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Planning Question Answered".into(),
            message: "Chose the plan_agent option.".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "Here is the final narrative summary.".into(),
            payload: None,
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
        .find("• Here is the final narrative summary.")
        .unwrap();

    assert!(shell_idx < plan_idx);
    assert!(plan_idx < planning_question_idx);
    assert!(planning_question_idx < generic_question_idx);
    assert!(generic_question_idx < agent_idx);
}

#[test]
fn committed_turn_cell_renders_terminal_result_as_terminal_cell() {
    let entries = vec![
        TranscriptEntry { role: "You".into(), message: "Run tests in the background".into(), payload: None },
        TranscriptEntry { role: "Tool".into(), message: "background_task_status bash-123".into(), payload: None },
        TranscriptEntry { role: "Tool Result".into(), message: "background task bash-123 completed: cargo test\nexit_code: 0\noutput:\ncompile\nrunning tests\nok".into(), payload: None },
        TranscriptEntry { role: "Agent".into(), message: "The background test task completed.".into(), payload: None },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Ran background cargo test"));
    assert!(rendered.contains("└ compile"));
    assert!(rendered.contains("running tests"));
    assert!(rendered.contains("ok"));
    assert!(rendered.contains("• The background test task completed."));
    assert!(!rendered.contains("background_task_status bash-123"));
}

#[test]
fn committed_turn_cell_renders_terminal_result_with_inline_output_path() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Run tests in the background".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Tool Result".into(),
            message:
                "background task bash-123 running\noutput: /tmp/rara/background-tasks/bash-123.log"
                    .into(),
            payload: None,
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Running background bash-123"));
    assert!(rendered.contains("/tmp/rara/background-tasks/bash-123.log"));
    assert!(!rendered.contains("background bash-123 running"));
}

#[test]
fn committed_turn_cell_renders_typed_terminal_event_as_terminal_cell() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Run tests in the background".into(),
            payload: None,
        },
        TranscriptEntry::terminal_event(TerminalEvent::End(TerminalCommandEvent {
            target: TerminalTarget::BackgroundTask,
            id: Some("bash-123".into()),
            status: "completed".into(),
            command: Some("cargo test".into()),
            exit_code: Some(0),
            output: vec!["compile".into(), "running tests".into(), "ok".into()],
            output_path: None,
            is_error: false,
        })),
        TranscriptEntry {
            role: "Agent".into(),
            message: "The background test task completed.".into(),
            payload: None,
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Ran background cargo test"));
    assert!(rendered.contains("└ compile"));
    assert!(rendered.contains("running tests"));
    assert!(rendered.contains("ok"));
    assert!(rendered.contains("• The background test task completed."));
    assert!(!rendered.contains("Terminal Event"));
}

#[test]
fn committed_turn_cell_keeps_final_agent_response_when_system_notice_arrives_after_tool_turn() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Inspect the repository and summarize the result".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "bash cargo check".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "The repository is healthy and the check passed.".into(),
            payload: None,
        },
        TranscriptEntry {
            role: "System".into(),
            message: "Waiting for device-code confirmation.".into(),
            payload: None,
        },
    ];

    let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("• The repository is healthy and the check passed."));
    assert!(!rendered.contains("Waiting for device-code confirmation."));
}
