use std::sync::Arc;

use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode};
use crate::oauth::OAuthManager;

use super::super::state::{
    HelpTab, LocalCommand, LocalCommandKind, Overlay, RuntimePhase, TuiApp,
};
use super::tasks::{start_compact_task, start_oauth_task};

pub(super) async fn execute_local_command(
    command: LocalCommand,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<bool> {
    app.remember_command(match command.kind {
        LocalCommandKind::Help => "help",
        LocalCommandKind::Status => "status",
        LocalCommandKind::Clear => "clear",
        LocalCommandKind::Resume => "resume",
        LocalCommandKind::Plan => "plan",
        LocalCommandKind::Approval => "approval",
        LocalCommandKind::Compact => "compact",
        LocalCommandKind::Setup => "setup",
        LocalCommandKind::Model => "model",
        LocalCommandKind::BaseUrl => "base-url",
        LocalCommandKind::Login => "login",
        LocalCommandKind::Quit => "quit",
    });
    match command.kind {
        LocalCommandKind::Help => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening help".into()));
            app.open_overlay(Overlay::Help(HelpTab::General));
        }
        LocalCommandKind::Status => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening status".into()));
            app.open_overlay(Overlay::Status);
        }
        LocalCommandKind::Clear => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("clearing transcript".into()));
            app.reset_transcript();
        }
        LocalCommandKind::Resume => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening resume picker".into()));
            app.open_overlay(Overlay::ResumePicker);
        }
        LocalCommandKind::Plan => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("entering planning mode".into()));
            app.set_pending_plan_approval(false);
            app.set_agent_execution_mode(AgentExecutionMode::Plan);
            if let Some(agent) = agent_slot.as_mut() {
                agent.set_execution_mode(AgentExecutionMode::Plan);
            }
            app.push_notice(
                "Planning mode enabled. Use the next prompt to analyze, refine, or finalize a plan.",
            );
        }
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
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("updating approval mode".into()));
            app.push_notice(notice);
        }
        LocalCommandKind::Compact => {
            if let Some(agent) = agent_slot.take() {
                start_compact_task(app, agent);
            } else {
                app.push_notice("No active agent available for compaction.");
            }
        }
        LocalCommandKind::Setup => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening setup".into()));
            app.open_overlay(Overlay::Setup);
        }
        LocalCommandKind::Model => handle_model_command(command.arg.as_deref(), app)?,
        LocalCommandKind::BaseUrl => handle_base_url_command(command.arg.as_deref(), app)?,
        LocalCommandKind::Login => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                start_oauth_task(app, Arc::clone(oauth_manager));
            }
        }
        LocalCommandKind::Quit => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("quitting".into()));
            return Ok(true);
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

fn handle_base_url_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    if arg.map(str::trim).filter(|arg| !arg.is_empty()).is_some() {
        app.push_notice("/base-url does not accept arguments. Edit the value in the TUI.");
    }
    app.open_overlay(Overlay::BaseUrlEditor);
    Ok(())
}
