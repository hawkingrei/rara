use std::sync::Arc;

use super::super::state::{
    HelpTab, LocalCommand, LocalCommandKind, Overlay, RuntimePhase, StatusTab, TuiApp,
};
use super::tasks::{start_compact_task, start_rebuild_task};
use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode};
use crate::oauth::OAuthManager;

pub(super) async fn execute_local_command(
    command: LocalCommand,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<bool> {
    app.remember_command(match command.kind {
        LocalCommandKind::Approval => "approval",
        LocalCommandKind::BaseUrl => "base-url",
        LocalCommandKind::Clear => "clear",
        LocalCommandKind::Compact => "compact",
        LocalCommandKind::Context => "context",
        LocalCommandKind::Help => "help",
        LocalCommandKind::Login => "login",
        LocalCommandKind::Logout => "logout",
        LocalCommandKind::Model => "model",
        LocalCommandKind::ModelName => "model-name",
        LocalCommandKind::Plan => "plan",
        LocalCommandKind::Quit => "quit",
        LocalCommandKind::Resume => "resume",
        LocalCommandKind::Status => "status",
    });
    match command.kind {
        LocalCommandKind::Approval => {
            let next_mode = match app.bash_approval_mode {
                BashApprovalMode::Suggestion => BashApprovalMode::Always,
                BashApprovalMode::Once => BashApprovalMode::Suggestion,
                BashApprovalMode::Always => BashApprovalMode::Suggestion,
            };
            app.bash_approval_mode = next_mode;
            if let Some(agent) = agent_slot.as_mut() {
                agent.set_bash_approval_mode(next_mode);
            }
            let notice = match next_mode {
                BashApprovalMode::Always => "Bash approval set to always.",
                BashApprovalMode::Once => "Bash approval set to once.",
                BashApprovalMode::Suggestion => "Bash approval set to suggestion.",
            };
            app.set_runtime_phase(
                RuntimePhase::LocalCommand,
                Some("updating approval mode".into()),
            );
            app.push_notice(notice);
        }
        LocalCommandKind::BaseUrl => handle_base_url_command(command.arg.as_deref(), app)?,
        LocalCommandKind::Help => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening help".into()));
            app.open_overlay(Overlay::Help(HelpTab::General));
        }
        LocalCommandKind::Clear => {
            app.set_runtime_phase(
                RuntimePhase::LocalCommand,
                Some("clearing transcript".into()),
            );
            app.reset_transcript();
        }
        LocalCommandKind::Compact => {
            if let Some(agent) = agent_slot.take() {
                start_compact_task(app, agent);
            } else {
                app.push_notice("No active agent available for compaction.");
            }
        }
        LocalCommandKind::Context => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening context".into()));
            app.open_overlay(Overlay::Context);
        }
        LocalCommandKind::Login => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                app.open_overlay(Overlay::AuthModePicker);
            }
        }
        LocalCommandKind::Logout => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                let removed = oauth_manager.clear_saved_auth()?;
                app.config.clear_provider_api_key("codex");
                app.config_manager.save(&app.config)?;
                app.push_notice(if removed {
                    "Cleared the saved provider credential.".to_string()
                } else {
                    "No saved provider credential was present.".to_string()
                });
                if app.config.provider == "codex" {
                    start_rebuild_task(app);
                }
            }
        }
        LocalCommandKind::Model => handle_model_command(command.arg.as_deref(), app)?,
        LocalCommandKind::ModelName => handle_model_name_command(command.arg.as_deref(), app)?,
        LocalCommandKind::Plan => {
            app.set_runtime_phase(
                RuntimePhase::LocalCommand,
                Some("entering planning mode".into()),
            );
            app.set_pending_plan_approval(false);
            app.set_agent_execution_mode(AgentExecutionMode::Plan);
            if let Some(agent) = agent_slot.as_mut() {
                agent.set_execution_mode(AgentExecutionMode::Plan);
            }
            app.push_notice("Planning mode enabled. Read-only planning; approve to execute.");
        }
        LocalCommandKind::Quit => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("quitting".into()));
            return Ok(true);
        }
        LocalCommandKind::Resume => {
            app.set_runtime_phase(
                RuntimePhase::LocalCommand,
                Some("opening resume picker".into()),
            );
            app.open_overlay(Overlay::ResumePicker);
        }
        LocalCommandKind::Status => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening status".into()));
            app.open_overlay(Overlay::Status(StatusTab::Overview));
        }
    }
    if let Some(agent) = agent_slot.as_ref() {
        app.sync_snapshot(agent);
    }
    Ok(false)
}

fn handle_model_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    if arg.map(str::trim).filter(|arg| !arg.is_empty()).is_some() {
        app.push_notice("/model does not accept arguments. Use the interactive menu.");
    }
    app.open_overlay(Overlay::ProviderPicker);
    app.notice = Some("Opened provider picker.".into());
    Ok(())
}

fn handle_model_name_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    if arg.map(str::trim).filter(|arg| !arg.is_empty()).is_some() {
        app.push_notice("/model-name does not accept arguments. Edit the value in the TUI.");
    }
    app.open_overlay(Overlay::ModelNameEditor);
    app.push_notice("Opened model name editor.");
    Ok(())
}

fn handle_base_url_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    if arg.map(str::trim).filter(|arg| !arg.is_empty()).is_some() {
        app.push_notice("/base-url does not accept arguments. Edit the value in the TUI.");
    }
    app.open_overlay(Overlay::BaseUrlEditor);
    Ok(())
}
