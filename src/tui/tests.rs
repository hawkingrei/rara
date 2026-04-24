use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use secrecy::ExposeSecret;
use tempfile::tempdir;
use tokio::sync::mpsc;

use crate::codex_model_catalog::{CodexModelOption, CodexReasoningOption};
use crate::config::ConfigManager;
use crate::config::{
    DEFAULT_CODEX_BASE_URL, DEFAULT_CODEX_CHATGPT_BASE_URL, DEFAULT_CODEX_MODEL,
};

use super::app_event::AppEvent;
use super::provider_flow::{
    codex_auth_is_available, open_provider_family_overlay, sync_codex_credential_from_auth_store,
};
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

#[test]
fn auth_mode_picker_prefers_selection_navigation() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.open_overlay(Overlay::AuthModePicker);

    assert!(matches!(
        map_key_to_event(KeyCode::Down, &app),
        AppEvent::MoveAuthModeSelection(1)
    ));
    assert!(matches!(
        map_key_to_event(KeyCode::Enter, &app),
        AppEvent::ApplyOverlaySelection
    ));
    assert!(matches!(
        map_key_to_event(KeyCode::Char('3'), &app),
        AppEvent::SetAuthModeSelection(2)
    ));
}

#[test]
fn openai_compatible_model_picker_exposes_connection_edit_shortcuts() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");

    app.provider_picker_idx = 1;
    app.open_overlay(Overlay::ModelPicker);

    assert!(matches!(
        map_key_to_event(KeyCode::Char('b'), &app),
        AppEvent::OpenOverlay(Overlay::BaseUrlEditor)
    ));
    assert!(matches!(
        map_key_to_event(KeyCode::Char('a'), &app),
        AppEvent::OpenOverlay(Overlay::ApiKeyEditor)
    ));
    assert!(matches!(
        map_key_to_event(KeyCode::Char('n'), &app),
        AppEvent::OpenOverlay(Overlay::ModelNameEditor)
    ));
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
