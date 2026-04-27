use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use secrecy::ExposeSecret;
use tempfile::tempdir;
use tokio::sync::mpsc;

use crate::codex_model_catalog::{CodexModelOption, CodexReasoningOption};
use crate::config::{ConfigManager, OpenAiEndpointKind};
use crate::config::{DEFAULT_CODEX_BASE_URL, DEFAULT_CODEX_CHATGPT_BASE_URL, DEFAULT_CODEX_MODEL};
use crate::tui::command::palette_commands;

use super::app_event::AppEvent;
use super::provider_flow::{
    codex_auth_is_available, open_provider_family_overlay, sync_codex_credential_from_auth_store,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn shifted_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}
use super::state::{Overlay, ProviderFamily, RunningTask, TaskKind, TuiApp};
use super::{
    classify_pending_plan_approval_input, dispatch_event, map_key_to_event,
    PendingPlanApprovalAction,
};

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

#[tokio::test]
async fn slash_palette_model_selection_opens_provider_picker_in_local_and_ssh() {
    for ssh in [false, true] {
        let temp = tempdir().expect("tempdir");
        let old_ssh_connection = std::env::var_os("SSH_CONNECTION");
        let old_ssh_tty = std::env::var_os("SSH_TTY");
        if ssh {
            std::env::set_var("SSH_CONNECTION", "test");
        } else {
            std::env::remove_var("SSH_CONNECTION");
            std::env::remove_var("SSH_TTY");
        }

        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("app");
        app.set_input("/".to_string());
        let model_idx = palette_commands(&app, "")
            .iter()
            .position(|spec| spec.name == "model")
            .expect("model command present");
        app.command_palette_idx = model_idx;

        let oauth_manager = Arc::new(
            crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
                .expect("oauth manager"),
        );
        let mut agent_slot = None;
        dispatch_event(
            AppEvent::ApplyOverlaySelection,
            &mut app,
            &mut agent_slot,
            &oauth_manager,
        )
        .await
        .expect("apply command palette selection");

        assert!(matches!(app.overlay, Some(Overlay::ProviderPicker)));
        assert_eq!(app.notice.as_deref(), Some("Opened provider picker."));

        if let Some(value) = old_ssh_connection {
            std::env::set_var("SSH_CONNECTION", value);
        } else {
            std::env::remove_var("SSH_CONNECTION");
        }
        if let Some(value) = old_ssh_tty {
            std::env::set_var("SSH_TTY", value);
        } else {
            std::env::remove_var("SSH_TTY");
        }
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
        map_key_to_event(key(KeyCode::Down), &app),
        AppEvent::MoveAuthModeSelection(1)
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Enter), &app),
        AppEvent::ApplyOverlaySelection
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Char('3')), &app),
        AppEvent::SetAuthModeSelection(2)
    ));
}

#[test]
fn plain_input_does_not_treat_s_as_setup_shortcut() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.input = "先同步ma".into();

    assert!(matches!(
        map_key_to_event(key(KeyCode::Char('s')), &app),
        AppEvent::InputChar('s')
    ));
}

#[test]
fn shift_enter_inserts_newline_in_main_composer() {
    let temp = tempdir().expect("tempdir");
    let app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");

    assert!(matches!(
        map_key_to_event(shifted_key(KeyCode::Enter), &app),
        AppEvent::InsertNewline
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Enter), &app),
        AppEvent::SubmitComposer
    ));
}

#[test]
fn arrow_keys_and_home_end_map_to_composer_cursor_events() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.input = "hello".into();

    assert!(matches!(
        map_key_to_event(key(KeyCode::Left), &app),
        AppEvent::MoveCursorLeft
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Right), &app),
        AppEvent::MoveCursorRight
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Home), &app),
        AppEvent::MoveCursorHome
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::End), &app),
        AppEvent::MoveCursorEnd
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Up), &app),
        AppEvent::MoveCursorUp
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Down), &app),
        AppEvent::MoveCursorDown
    ));
}

#[test]
fn empty_composer_keeps_up_down_for_transcript_scroll() {
    let temp = tempdir().expect("tempdir");
    let app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");

    assert!(matches!(
        map_key_to_event(key(KeyCode::Up), &app),
        AppEvent::ScrollTranscript(-1)
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Down), &app),
        AppEvent::ScrollTranscript(1)
    ));
}

#[tokio::test]
async fn composer_supports_mid_input_insertion_and_backspace() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.set_input("helo".to_string());

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::MoveCursorLeft,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("move left");
    dispatch_event(
        AppEvent::InputChar('l'),
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("insert");
    assert_eq!(app.input, "hello");
    assert_eq!(app.composer_cursor_offset(), 4);

    dispatch_event(
        AppEvent::Backspace,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("backspace");
    assert_eq!(app.input, "helo");
    assert_eq!(app.composer_cursor_offset(), 3);
}

#[tokio::test]
async fn paste_inserts_at_current_cursor_offset() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.set_input("helo".to_string());
    app.move_active_input_cursor_left();

    super::terminal_ui::handle_paste("l".to_string(), &mut app);

    assert_eq!(app.input, "hello");
    assert_eq!(app.composer_cursor_offset(), 4);
}

#[tokio::test]
async fn composer_supports_vertical_cursor_navigation_across_lines() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.terminal_width = 12;
    app.set_input("abcd\nefgh".to_string());
    app.input_cursor_offset = Some("abcd\nef".chars().count());

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::MoveCursorUp,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("move up");
    assert_eq!(app.composer_cursor_offset(), 2);

    dispatch_event(
        AppEvent::MoveCursorDown,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("move down");
    assert_eq!(app.composer_cursor_offset(), "abcd\nef".chars().count());
}

#[test]
fn app_starts_with_warning_instead_of_api_key_editor_for_hosted_provider_without_api_key() {
    let temp = tempdir().expect("tempdir");
    let cm = ConfigManager {
        path: temp.path().join("config.json"),
    };
    let mut config = cm.load().expect("load config");
    config.set_provider("openai-compatible");
    config.clear_api_key();
    cm.save(&config).expect("save config");

    let app = TuiApp::new(cm).expect("app");
    assert!(app.overlay.is_none());
    assert!(app
        .notice
        .as_deref()
        .is_some_and(|value| value.starts_with("Warning:")));
}

#[test]
fn openai_compatible_model_picker_exposes_profile_table_shortcuts() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");

    app.provider_picker_idx = 1;
    app.open_overlay(Overlay::ModelPicker);

    assert!(matches!(
        map_key_to_event(key(KeyCode::Char('c')), &app),
        AppEvent::CreateOpenAiProfile
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Char('e')), &app),
        AppEvent::EditOpenAiProfile
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Char(' ')), &app),
        AppEvent::ApplyOverlaySelection
    ));
    assert!(matches!(
        map_key_to_event(key(KeyCode::Char('d')), &app),
        AppEvent::DeleteOpenAiProfile
    ));
}

#[tokio::test]
async fn openai_model_picker_delete_row_removes_active_profile() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.config.select_openai_profile(
        "custom-default",
        "Custom endpoint",
        OpenAiEndpointKind::Custom,
    );
    app.config.set_api_key("sk-custom");
    app.config.select_openai_profile(
        "openrouter-default",
        "OpenRouter",
        OpenAiEndpointKind::Openrouter,
    );
    app.open_overlay(Overlay::ModelPicker);

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::DeleteOpenAiProfile,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("delete profile");

    assert_eq!(
        app.config.active_openai_profile_id(),
        Some("custom-default")
    );
    assert!(matches!(app.overlay, Some(Overlay::ModelPicker)));
    assert!(matches!(
        app.running_task.as_ref(),
        Some(task) if matches!(task.kind, TaskKind::Rebuild)
    ));
    if let Some(task) = app.running_task.take() {
        task.handle.abort();
    }
}

#[tokio::test]
async fn openai_model_picker_space_activates_selected_profile_and_starts_setup_when_incomplete() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.open_overlay(Overlay::ModelPicker);

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::SetModelSelection(0),
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("set model selection");

    assert!(matches!(app.overlay, Some(Overlay::ModelPicker)));

    dispatch_event(
        AppEvent::ApplyOverlaySelection,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("activate selected profile");

    assert!(matches!(app.overlay, Some(Overlay::BaseUrlEditor)));
}

#[tokio::test]
async fn openai_model_picker_edit_shortcut_starts_wizard_for_selected_profile() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.config.select_openai_profile(
        "custom-default",
        "Custom endpoint",
        OpenAiEndpointKind::Custom,
    );
    app.config.select_openai_profile(
        "openrouter-default",
        "OpenRouter",
        OpenAiEndpointKind::Openrouter,
    );
    app.open_overlay(Overlay::ModelPicker);
    app.model_picker_idx = 1;

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::EditOpenAiProfile,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("open selected profile model editor");

    assert_eq!(
        app.config.active_openai_profile_id(),
        Some("custom-default")
    );
    assert!(matches!(app.overlay, Some(Overlay::BaseUrlEditor)));
    assert_eq!(
        app.openai_setup_steps,
        vec![Overlay::ApiKeyEditor, Overlay::ModelNameEditor]
    );
}

#[tokio::test]
async fn openai_profile_edit_wizard_keeps_existing_api_key_when_blank() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.config.select_openai_profile(
        "openrouter-default",
        "OpenRouter",
        OpenAiEndpointKind::Openrouter,
    );
    app.config.set_api_key("sk-openrouter");
    app.open_overlay(Overlay::ModelPicker);

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::EditOpenAiProfile,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("start edit wizard");
    assert!(matches!(app.overlay, Some(Overlay::BaseUrlEditor)));

    dispatch_event(
        AppEvent::SaveBaseUrlInput,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("save base url");
    assert!(matches!(app.overlay, Some(Overlay::ApiKeyEditor)));
    assert!(app.api_key_input.is_empty());

    dispatch_event(
        AppEvent::SaveApiKeyInput,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("skip api key");

    assert_eq!(app.config.api_key(), Some("sk-openrouter"));
    assert!(matches!(app.overlay, Some(Overlay::ModelNameEditor)));
}

#[tokio::test]
async fn openai_model_picker_create_shortcut_opens_endpoint_kind_picker() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.open_overlay(Overlay::ModelPicker);

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::CreateOpenAiProfile,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("open endpoint kind picker");

    assert!(matches!(
        app.overlay,
        Some(Overlay::OpenAiEndpointKindPicker)
    ));
    assert_eq!(app.openai_endpoint_kind_picker_idx, 0);
}

#[tokio::test]
async fn selecting_custom_endpoint_kind_prompts_for_profile_label() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.overlay = Some(Overlay::OpenAiEndpointKindPicker);
    app.openai_endpoint_kind_picker_idx = 0;

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::ApplyOverlaySelection,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("select endpoint kind");

    assert!(matches!(
        app.overlay,
        Some(Overlay::OpenAiProfileLabelEditor)
    ));
    assert_eq!(
        app.openai_profile_label_kind,
        Some(OpenAiEndpointKind::Custom)
    );
}

#[tokio::test]
async fn selecting_openai_profile_from_picker_switches_active_profile() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.config.select_openai_profile(
        "openrouter-main",
        "OpenRouter Main",
        OpenAiEndpointKind::Openrouter,
    );
    app.config.select_openai_profile(
        "openrouter-backup",
        "OpenRouter Backup",
        OpenAiEndpointKind::Openrouter,
    );
    app.model_picker_idx = 3;
    app.open_overlay(Overlay::OpenAiProfilePicker);
    app.openai_profile_picker_idx = 2;

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::ApplyOverlaySelection,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("apply profile selection");

    assert_eq!(
        app.config.active_openai_profile_id(),
        Some("openrouter-main")
    );
    assert!(matches!(app.overlay, Some(Overlay::ModelPicker)));
}

#[tokio::test]
async fn saving_openai_profile_label_creates_new_profile_for_selected_kind() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.provider_picker_idx = 1;
    app.config
        .select_openai_profile("deepseek-default", "DeepSeek", OpenAiEndpointKind::Deepseek);
    app.open_overlay(Overlay::OpenAiProfilePicker);
    app.openai_profile_picker_idx = 0;

    let oauth_manager = Arc::new(
        crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
            .expect("oauth manager"),
    );
    let mut agent_slot = None;

    dispatch_event(
        AppEvent::ApplyOverlaySelection,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("open profile label editor");

    app.openai_profile_label_input = "DeepSeek backup".to_string();

    dispatch_event(
        AppEvent::SaveOpenAiProfileLabelInput,
        &mut app,
        &mut agent_slot,
        &oauth_manager,
    )
    .await
    .expect("save profile label");

    assert_eq!(
        app.config.active_openai_profile_id(),
        Some("deepseek-deepseek-backup")
    );
    assert_eq!(
        app.config.active_openai_profile_kind(),
        Some(OpenAiEndpointKind::Deepseek)
    );
    assert!(app
        .config
        .openai_profiles
        .contains_key("deepseek-deepseek-backup"));
    assert!(matches!(app.overlay, Some(Overlay::ApiKeyEditor)));
    assert_eq!(app.openai_setup_steps, vec![Overlay::ModelNameEditor]);
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
    let oauth_manager = crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
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
    let oauth_manager = crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
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
    let oauth_manager = crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
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
    let oauth_manager = crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
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
    let oauth_manager = crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
        .expect("oauth manager");
    oauth_manager
        .save_api_key("sk-test-codex")
        .expect("save api key");

    app.config.set_provider("ollama");
    app.provider_picker_idx = 0;

    assert!(sync_codex_credential_from_auth_store(&mut app, &oauth_manager).expect("sync auth"));
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

#[test]
fn codex_chatgpt_auth_store_sets_chatgpt_base_url_before_model_flow() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    let oauth_manager = crate::oauth::OAuthManager::new_for_config_dir(temp.path().join(".rara"))
        .expect("oauth manager");
    codex_login::save_auth(
        &temp.path().join(".rara").join("codex-auth"),
        &codex_login::AuthDotJson {
            auth_mode: None,
            openai_api_key: Some("sk-from-oauth".into()),
            tokens: Some(codex_login::TokenData {
                id_token: codex_login::token_data::parse_chatgpt_jwt_claims(
                    "eyJhbGciOiJub25lIn0.e30.signature",
                )
                .expect("valid id token"),
                access_token: "oauth-access-token".into(),
                refresh_token: "refresh".into(),
                account_id: None,
            }),
            last_refresh: None,
            agent_identity: None,
        },
        codex_login::AuthCredentialsStoreMode::File,
    )
    .expect("save auth");

    app.config.set_provider("ollama");

    assert!(sync_codex_credential_from_auth_store(&mut app, &oauth_manager).expect("sync auth"));
    assert_eq!(
        app.config
            .provider_states
            .get("codex")
            .and_then(|state| state.api_key.as_ref())
            .map(|value| value.expose_secret()),
        Some("oauth-access-token")
    );
    assert_eq!(
        app.config
            .provider_states
            .get("codex")
            .and_then(|state| state.base_url.as_deref()),
        Some(DEFAULT_CODEX_CHATGPT_BASE_URL)
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
    assert_eq!(
        app.codex_auth_mode,
        Some(crate::oauth::SavedCodexAuthMode::ApiKey)
    );
}
