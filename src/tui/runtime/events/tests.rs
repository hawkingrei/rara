use serde_json::json;
use tempfile::tempdir;

use super::apply_tui_event;
use super::helpers::{
    format_apply_patch_result, format_apply_patch_use, format_tool_progress, format_tool_result,
    format_tool_use, is_oauth_prompt_message, planning_note_lines, scrub_internal_control_tokens,
    subagent_request_input,
};
use crate::agent::AgentExecutionMode;
use crate::config::ConfigManager;
use crate::tool::ToolOutputStream;
use crate::tui::state::{RuntimePhase, TuiApp, TuiEvent};

#[test]
fn parses_delegated_request_input_from_subagent_result() {
    let parsed = subagent_request_input(
        "plan_agent refine the workspace logic\nrequest_user_input: Which discovery strategy should we keep?\noption: Minimal | Keep the current root-level files.\noption: Generic | Scan all instruction markdown files.\nnote: We need one product decision before editing.",
    )
    .expect("delegated request input should parse");

    assert_eq!(parsed.question, "Which discovery strategy should we keep?");
    assert_eq!(parsed.options.len(), 2);
    assert_eq!(parsed.options[0].0, "Minimal");
    assert_eq!(parsed.options[1].0, "Generic");
    assert_eq!(
        parsed.note.as_deref(),
        Some("We need one product decision before editing.")
    );
}

#[test]
fn planning_note_lines_drop_meta_and_mutating_chatter() {
    let notes = planning_note_lines(
        "I will use apply_patch on crates/instructions/src/workspace.rs.\nThe current discovery is hardcoded to root-level markdown files.\nThis is the final step: applying the patch.",
    );
    assert_eq!(
        notes,
        vec!["The current discovery is hardcoded to root-level markdown files.".to_string()]
    );
}

#[test]
fn scrub_internal_channel_markers_preserves_text_boundaries() {
    let cleaned = scrub_internal_control_tokens(
        "Inspecting prompt sources.<channel|>I have a concrete implementation plan.",
    );
    assert_eq!(
        cleaned,
        "Inspecting prompt sources.\nI have a concrete implementation plan."
    );
}

#[test]
fn scrub_internal_control_tokens_removes_dsml_tool_blocks() {
    let cleaned = scrub_internal_control_tokens(
        "Before\n<｜DSML｜tool_calls>\n<｜DSML｜invoke name=\"apply_patch\">\n<｜DSML｜parameter name=\"path\" string=\"true\">src/lib.rs</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜tool_calls>\nAfter",
    );

    assert_eq!(cleaned.trim(), "Before\n\nAfter");
    assert!(!cleaned.contains("DSML"));
    assert!(!cleaned.contains("apply_patch"));
}

#[test]
fn scrub_internal_control_tokens_drops_orphaned_dsml_payload() {
    let cleaned = scrub_internal_control_tokens(
        "kind: format!(\"unknown_retrieval_{tool_name}\"),\nlabel: format!(\"Unknown Retrieval ({tool_name})\"),\n}\n<｜DSML｜parameter name=\"path\" string=\"true\">src/context/selection.rs</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜tool_calls>",
    );

    assert!(cleaned.trim().is_empty());
}

#[test]
fn plan_mode_routes_planning_prose_to_planning_not_exploring() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.set_agent_execution_mode(AgentExecutionMode::Plan);
    app.record_exploration_action("Read crates/instructions/src/workspace.rs");
    app.runtime_phase = RuntimePhase::RunningTool;

    apply_tui_event(
        &mut app,
        TuiEvent::Transcript {
            role: "Agent",
            message: "Based on the inspection of `crates/instructions/src/workspace.rs`, I propose the following plan:<channel|>\n1. Generalize prompt discovery.\n2. Keep the current merge semantics.".into(),
        },
    );

    assert!(app.active_live.exploration_notes.is_empty());
    assert_eq!(
        app.active_live.planning_notes,
        vec![
            "1. Generalize prompt discovery.".to_string(),
            "2. Keep the current merge semantics.".to_string()
        ]
    );
}

#[test]
fn agent_dsml_only_message_does_not_enter_transcript() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");

    apply_tui_event(
        &mut app,
        TuiEvent::Transcript {
            role: "Agent".into(),
            message: "<｜DSML｜tool_calls>\n<｜DSML｜invoke name=\"replace\"></｜DSML｜invoke>\n</｜DSML｜tool_calls>".into(),
        },
    );

    assert!(app.active_turn.entries.is_empty());
}

#[test]
fn bash_rg_tool_use_is_shown_as_exploration() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");

    apply_tui_event(
        &mut app,
        TuiEvent::Transcript {
            role: "Tool".into(),
            message: "bash rg --files src/tui".into(),
        },
    );
    apply_tui_event(
        &mut app,
        TuiEvent::Transcript {
            role: "Tool".into(),
            message: "bash cd src && rg -n \"render\" tui".into(),
        },
    );

    assert_eq!(
        app.active_live.exploration_actions,
        vec![
            "Find files rg --files src/tui".to_string(),
            "Search cd src && rg -n \"render\" tui".to_string()
        ]
    );
    assert!(app.active_live.running_actions.is_empty());
}

#[test]
fn formats_apply_patch_tool_use_with_target_files() {
    let rendered = format_apply_patch_use(&json!({
        "patch": "*** Begin Patch\n*** Update File: src/tui/render.rs\n@@\n-old\n+new\n*** Update File: src/tui/runtime/events.rs\n@@\n-old\n+new\n*** End Patch"
    }));
    assert_eq!(
        rendered,
        "apply_patch src/tui/render.rs, src/tui/runtime/events.rs"
    );
}

#[test]
fn formats_apply_patch_tool_result_as_diff_summary() {
    let rendered = format_apply_patch_result(&json!({
        "status": "ok",
        "files_changed": 2,
        "line_delta": { "added": 12, "removed": 3 },
        "updated_files": ["src/tui/render.rs"],
        "created_files": ["src/tui/render/bottom_pane.rs"],
        "summary": [
            "updated src/tui/render.rs",
            "created src/tui/render/bottom_pane.rs"
        ]
    }));

    assert!(rendered.contains("apply_patch ok 2 file(s) (+12 -3)"));
    assert!(rendered.contains("updated: src/tui/render.rs"));
    assert!(rendered.contains("created: src/tui/render/bottom_pane.rs"));
    assert!(rendered.contains("changes:"));
}

#[test]
fn formats_replace_lines_tool_use_as_file_range() {
    let rendered = format_tool_use(
        "replace_lines",
        &json!({
            "path": "src/context/assembler.rs",
            "start_line": 426,
            "end_line": 1263,
            "new_string": ""
        }),
    );

    assert_eq!(rendered, "replace_lines src/context/assembler.rs:426-1263");
}

#[test]
fn formats_spawn_agent_tool_use_without_dumping_instruction_json() {
    let rendered = format_tool_use(
        "spawn_agent",
        &json!({
            "name": "fix-assembler",
            "instruction": "Fix src/context/assembler.rs by removing the orphaned code block between the two cfg(test) markers.\nRead the file in small chunks and do not use a giant replace old_string payload."
        }),
    );

    assert!(rendered.starts_with("spawn_agent fix-assembler: Fix src/context/assembler.rs"));
    assert!(rendered.ends_with('…'));
    assert!(!rendered.contains("\"instruction\""));
    assert!(!rendered.contains('\n'));
}

#[test]
fn formats_spawn_agent_tool_result_with_agent_name() {
    let rendered = format_tool_result(
        "spawn_agent",
        &json!({
            "name": "fix-assembler",
            "status": "done",
            "summary": "Removed the orphaned code block."
        })
        .to_string(),
    );

    assert_eq!(
        rendered,
        "spawn_agent fix-assembler: Removed the orphaned code block."
    );
}

#[test]
fn formats_replace_lines_tool_result_as_edit_summary() {
    let rendered = format_tool_result(
        "replace_lines",
        &json!({
            "status": "ok",
            "path": "src/context/assembler.rs",
            "start_line": 426,
            "end_line": 1263,
            "removed_lines": 838,
            "inserted_lines": 0,
            "line_delta": -838
        })
        .to_string(),
    );

    assert_eq!(
        rendered,
        "replace_lines src/context/assembler.rs:426-1263\nremoved=838 inserted=0 line_delta=-838"
    );
}

#[test]
fn formats_bash_tool_result_with_output_tail() {
    let rendered = format_tool_result(
        "bash",
        &json!({
            "exit_code": 0,
            "stdout": "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\n",
            "stderr": "warn 1\nwarn 2\n"
        })
        .to_string(),
    );

    assert!(rendered.contains("bash finished with exit code 0"));
    assert!(rendered.contains("stdout:"));
    assert!(rendered.contains("line 7"));
    assert!(rendered.contains("line 6"));
    assert!(!rendered.contains("line 1"));
}

#[test]
fn formats_live_bash_tool_result_without_duplicate_tail() {
    let rendered = format_tool_result(
        "bash",
        &json!({
            "exit_code": 0,
            "stdout": "line 1\nline 2\n",
            "stderr": "",
            "live_streamed": true
        })
        .to_string(),
    );

    assert!(rendered.contains("bash finished with exit code 0"));
    assert!(rendered.contains("output streamed above"));
    assert!(!rendered.contains("stdout:"));
}

#[test]
fn formats_generic_tool_result_without_preview_available_marker() {
    let rendered = format_tool_result(
        "bash",
        "Tool bash completed with exit_code, live_streamed, stderr, stdout.\nline 1\nline 2",
    );

    assert!(rendered.contains("bash: bash finished"));
    assert!(rendered.contains("line 1"));
    assert!(rendered.contains("line 2"));
    assert!(!rendered.contains("preview available"));
}

#[test]
fn formats_tool_progress_with_stream_label() {
    let rendered = format_tool_progress("bash", ToolOutputStream::Stderr, "warn 1\nwarn 2\n");
    assert_eq!(rendered, "bash stderr:\nwarn 1\nwarn 2\n");
}

#[test]
fn runtime_device_code_messages_update_prompt_and_polling_phases() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");

    apply_tui_event(
        &mut app,
        TuiEvent::Transcript {
            role: "Runtime",
            message: "Open this URL in a browser and enter the one-time code:\nhttps://example.test\n\nCode: ABCD".into(),
        },
    );
    assert_eq!(app.runtime_phase, RuntimePhase::OAuthDeviceCodePrompt);
    let prompt_entry = app
        .active_turn
        .entries
        .last()
        .expect("persisted oauth prompt entry");
    assert_eq!(prompt_entry.role, "System");
    assert!(prompt_entry.message.contains("https://example.test"));
    assert!(prompt_entry.message.contains("Code: ABCD"));

    apply_tui_event(
        &mut app,
        TuiEvent::Transcript {
            role: "Runtime",
            message: "Waiting for device-code confirmation.".into(),
        },
    );
    assert_eq!(app.runtime_phase, RuntimePhase::OAuthPollingDeviceCode);
}

#[test]
fn detects_persistent_oauth_prompt_messages() {
    assert!(is_oauth_prompt_message(
        "Open this URL in a browser and enter the one-time code:\nhttps://example.test\n\nCode: ABCD"
    ));
    assert!(is_oauth_prompt_message(
        "Starting Codex browser login.\nOpen this URL if the browser does not launch automatically:\nhttps://example.test"
    ));
    assert!(!is_oauth_prompt_message(
        "Waiting for device-code confirmation."
    ));
}
