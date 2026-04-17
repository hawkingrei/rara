use std::sync::Arc;

use anyhow::anyhow;
use serde_json::json;

use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode};
use crate::oauth::OAuthManager;
use crate::tools::search::GrepTool;
use crate::tool::Tool;

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
        LocalCommandKind::Search => "search",
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
            let next_mode = if matches!(app.agent_execution_mode, AgentExecutionMode::Plan) {
                AgentExecutionMode::Execute
            } else {
                AgentExecutionMode::Plan
            };
            let (detail, notice) = match next_mode {
                AgentExecutionMode::Plan => (
                    "entering plan mode".into(),
                    "Plan mode active. Read-only tools only.".to_string(),
                ),
                AgentExecutionMode::Execute => (
                    "returning to execute mode".into(),
                    "Execute mode active. Full toolset restored.".to_string(),
                ),
            };
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some(detail));
            app.set_agent_execution_mode(next_mode);
            if let Some(agent) = agent_slot.as_mut() {
                agent.set_execution_mode(next_mode);
            }
            app.push_notice(notice);
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
        LocalCommandKind::Search => handle_search_command(command.arg.as_deref(), app).await?,
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

async fn handle_search_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    let raw = arg
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("/search requires a pattern. Use /search <pattern> [in <path>]."))?;
    let (pattern, path) = parse_search_argument(raw);
    let tool = GrepTool;
    let input = json!({
        "pattern": pattern,
        "path": path,
    });
    app.set_runtime_phase(
        RuntimePhase::LocalCommand,
        Some(format!("searching {path} for {pattern}")),
    );
    app.push_entry("You", format!("/search {raw}"));
    app.push_entry("Tool", format!("grep {pattern} in {path}"));

    match tool.call(input).await {
        Ok(result) => {
            let rendered = render_local_search_result(pattern, path, &result);
            app.push_entry("Tool Result", rendered.clone());
            app.push_entry("Agent", rendered);
            app.push_notice(format!("Search completed for \"{pattern}\"."));
        }
        Err(err) => {
            let rendered = format!("grep failed: {err}");
            app.push_entry("Tool Error", rendered.clone());
            app.push_notice(rendered);
        }
    }
    Ok(())
}

fn parse_search_argument(raw: &str) -> (&str, &str) {
    if let Some((pattern, path)) = raw.rsplit_once(" in ") {
        let pattern = pattern.trim();
        let path = path.trim();
        if !pattern.is_empty() && !path.is_empty() {
            return (pattern, path);
        }
    }
    (raw, ".")
}

fn render_local_search_result(pattern: &str, path: &str, result: &serde_json::Value) -> String {
    let entries = result
        .get("results")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = entries.len();
    if total == 0 {
        return format!("Search \"{pattern}\" in {path}: no matches.");
    }

    let preview = entries
        .iter()
        .take(12)
        .map(|entry| {
            let file = entry.get("file").and_then(serde_json::Value::as_str).unwrap_or("<unknown>");
            let line = entry.get("line").and_then(serde_json::Value::as_u64).unwrap_or(0);
            let content = entry
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            format!("{file}:{line}: {content}")
        })
        .collect::<Vec<_>>();
    let remaining = total.saturating_sub(preview.len());

    if remaining > 0 {
        format!(
            "Search \"{pattern}\" in {path}: {total} match(es).\nTop hits:\n{}\n... {remaining} more match(es).",
            preview.join("\n")
        )
    } else {
        format!(
            "Search \"{pattern}\" in {path}: {total} match(es).\nTop hits:\n{}",
            preview.join("\n")
        )
    }
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
