use super::{
    input_requests_command_palette, state_db_status_error, ActivePendingInteractionKind,
    InteractionKind, Overlay, PendingInteractionSnapshot, ProviderFamily, RuntimeSnapshot,
    TranscriptEntry, TranscriptTurn, TuiApp,
};
use crate::codex_model_catalog::{CodexModelOption, CodexReasoningOption};
use crate::config::{ConfigManager, RaraConfig};
use crate::config::{DEFAULT_CODEX_BASE_URL, DEFAULT_CODEX_MODEL};
use tempfile::tempdir;

#[test]
fn detects_slash_command_input() {
    assert!(input_requests_command_palette("/"));
    assert!(input_requests_command_palette("/help"));
    assert!(input_requests_command_palette("   /help"));
    assert!(!input_requests_command_palette(""));
    assert!(!input_requests_command_palette("help"));
    assert!(!input_requests_command_palette("   help"));
}

#[test]
fn redacts_secrets_in_state_db_status_messages() {
    let rendered = state_db_status_error(
        "write failed",
        "token=supersecretvalue Authorization: Bearer abcdefghijklmnopqrstuvwxyz",
    );
    assert!(rendered.contains("write failed:"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("supersecretvalue"));
    assert!(!rendered.contains("abcdefghijklmnopqrstuvwxyz"));
}

#[test]
fn prioritizes_active_pending_interaction_in_ui_order() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");
    app.config = RaraConfig::default();
    app.snapshot = RuntimeSnapshot {
        pending_interactions: vec![
            PendingInteractionSnapshot {
                kind: InteractionKind::RequestInput,
                title: "Question".to_string(),
                summary: String::new(),
                options: Vec::new(),
                note: None,
                approval: None,
                source: Some("plan_agent".to_string()),
            },
            PendingInteractionSnapshot {
                kind: InteractionKind::Approval,
                title: "Pending Approval".to_string(),
                summary: "run cargo test".to_string(),
                options: Vec::new(),
                note: None,
                approval: None,
                source: None,
            },
            PendingInteractionSnapshot {
                kind: InteractionKind::PlanApproval,
                title: "Plan Ready".to_string(),
                summary: "Review the plan.".to_string(),
                options: Vec::new(),
                note: None,
                approval: None,
                source: None,
            },
        ],
        ..RuntimeSnapshot::default()
    };

    let active = app
        .active_pending_interaction()
        .expect("pending interaction");
    assert_eq!(active.kind, ActivePendingInteractionKind::PlanApproval);
    assert_eq!(active._snapshot.title, "Plan Ready");
}

#[test]
fn queued_follow_up_messages_preserve_fifo_order() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");

    assert_eq!(app.queue_follow_up_message("first"), 1);
    assert_eq!(app.queue_follow_up_message("second"), 2);
    assert_eq!(app.queued_follow_up_preview(), Some("first"));
    assert_eq!(app.pop_queued_follow_up_message().as_deref(), Some("first"));
    assert_eq!(
        app.pop_queued_follow_up_message().as_deref(),
        Some("second")
    );
    assert_eq!(app.pop_queued_follow_up_message(), None);
}

#[test]
fn pending_follow_up_messages_release_on_tool_boundary() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");

    app.begin_running_turn();
    assert_eq!(
        app.queue_follow_up_message_after_next_tool_boundary("first pending"),
        1
    );
    assert_eq!(app.pending_follow_up_preview(), Some("first pending"));
    assert_eq!(app.queued_end_of_turn_preview(), None);

    app.advance_running_tool_boundary();

    assert_eq!(app.pending_follow_up_preview(), None);
    assert_eq!(app.queued_end_of_turn_preview(), Some("first pending"));
    assert_eq!(
        app.pop_queued_follow_up_message().as_deref(),
        Some("first pending")
    );
}

#[test]
fn openai_compatible_preset_sets_default_connection_fields() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");

    app.provider_picker_idx = 1;
    assert_eq!(
        app.selected_provider_family(),
        ProviderFamily::OpenAiCompatible
    );

    app.select_local_model(0);

    assert_eq!(app.config.provider, "openai-compatible");
    assert_eq!(app.config.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(
        app.config.base_url.as_deref(),
        Some("https://api.openai.com/v1")
    );
    assert_eq!(app.config.revision, None);
}

#[test]
fn openai_compatible_preset_preserves_custom_model_name() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");

    app.config.set_provider("openai-compatible");
    app.config.set_model(Some("custom-model".to_string()));
    app.provider_picker_idx = 1;

    app.select_local_model(0);

    assert_eq!(app.config.provider, "openai-compatible");
    assert_eq!(app.config.model.as_deref(), Some("custom-model"));
}

#[test]
fn codex_preset_keeps_the_codex_model_label() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");

    app.provider_picker_idx = 0;
    app.set_codex_model_options(vec![CodexModelOption {
        id: DEFAULT_CODEX_MODEL.to_string(),
        model: DEFAULT_CODEX_MODEL.to_string(),
        label: "gpt-5.4".to_string(),
        description: "Latest frontier agentic coding model.".to_string(),
        reasoning_options: vec![CodexReasoningOption {
            value: "medium".to_string(),
            label: "Medium".to_string(),
            description: "Default reasoning effort.".to_string(),
            is_default: true,
        }],
        default_reasoning_effort: Some("medium".to_string()),
        is_default: true,
    }]);
    app.select_local_model(0);

    assert_eq!(app.config.provider, "codex");
    assert_eq!(app.config.model.as_deref(), Some(DEFAULT_CODEX_MODEL));
    assert_eq!(app.config.base_url.as_deref(), Some(DEFAULT_CODEX_BASE_URL));
}

#[test]
fn opening_openai_compatible_model_picker_restores_provider_scoped_state() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");

    app.config.set_provider("openai-compatible");
    app.config
        .set_base_url(Some("http://proxy.local/v1".to_string()));
    app.config.set_model(Some("custom-model".to_string()));
    app.config.set_provider("codex");
    app.config.set_model(Some("codex".to_string()));

    app.provider_picker_idx = 1;
    app.open_overlay(Overlay::ModelPicker);

    assert_eq!(app.config.provider, "openai-compatible");
    assert_eq!(
        app.config.base_url.as_deref(),
        Some("http://proxy.local/v1")
    );
    assert_eq!(app.config.model.as_deref(), Some("custom-model"));
}

#[test]
fn model_name_editor_seeds_from_selected_provider_state() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");

    app.config.set_provider("openai-compatible");
    app.config.set_model(Some("custom-model".to_string()));
    app.config.set_provider("codex");
    app.provider_picker_idx = 1;

    app.open_overlay(Overlay::ModelPicker);
    app.open_overlay(Overlay::ModelNameEditor);

    assert_eq!(app.model_name_input, "custom-model");
}

#[test]
fn finalize_agent_stream_updates_latest_committed_turn_when_final_text_arrives_late() {
    let dir = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: dir.path().join("config.json"),
    };
    let mut app = TuiApp::new(cm).expect("app");
    app.committed_turns.push(TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "你好".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "你好！".into(),
            },
        ],
    });

    app.finalize_agent_stream(Some("你好！有什么我可以帮你的？".into()));

    assert!(app.active_turn.entries.is_empty());
    assert_eq!(
        app.committed_turns
            .last()
            .and_then(|turn| turn.entries.last())
            .map(|entry| entry.message.as_str()),
        Some("你好！有什么我可以帮你的？")
    );
}
