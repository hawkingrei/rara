use crate::agent::AgentEvent;

use super::super::state::{RuntimePhase, TuiApp, TuiEvent};

pub(super) fn apply_tui_event(app: &mut TuiApp, event: TuiEvent) {
    match event {
        TuiEvent::Transcript { role, message } => {
            if role == "Status" {
                app.set_runtime_phase(
                    RuntimePhase::ProcessingResponse,
                    Some(message.lines().next().unwrap_or(role).trim().to_string()),
                );
                return;
            } else if role == "Agent Delta" {
                app.set_runtime_phase(
                    RuntimePhase::ProcessingResponse,
                    Some("streaming model output".into()),
                );
                app.append_agent_delta(&message);
                return;
            } else if role == "Tool" || role == "Tool Result" || role == "Tool Error" {
                app.set_runtime_phase(
                    RuntimePhase::RunningTool,
                    Some(message.lines().next().unwrap_or(role).trim().to_string()),
                );
            } else if role == "Agent" {
                app.set_runtime_phase(
                    RuntimePhase::ProcessingResponse,
                    Some("receiving model output".into()),
                );
                app.finalize_agent_stream(Some(message));
                return;
            } else if role == "Download" {
                let detail = message.lines().next().unwrap_or(role).trim().to_string();
                if detail.starts_with("Ready ·") {
                    app.set_runtime_phase(RuntimePhase::BackendReady, Some(detail));
                } else {
                    app.set_runtime_phase(RuntimePhase::RebuildingBackend, Some(detail));
                }
            } else if role == "Runtime" {
                let detail = message.lines().next().unwrap_or(role).trim().to_string();
                if detail.contains("OAuth flow") {
                    app.set_runtime_phase(RuntimePhase::OAuthWaitingCallback, Some(detail));
                } else if detail.contains("exchanging token") {
                    app.set_runtime_phase(RuntimePhase::OAuthExchangingToken, Some(detail));
                } else {
                    app.set_runtime_phase(RuntimePhase::RebuildingBackend, Some(detail));
                }
            }
            app.push_entry(role, message)
        }
    }
}

pub(super) fn convert_agent_event(event: AgentEvent) -> Option<TuiEvent> {
    match event {
        AgentEvent::Status(message) => Some(TuiEvent::Transcript {
            role: "Status",
            message,
        }),
        AgentEvent::AssistantText(text) => Some(TuiEvent::Transcript {
            role: "Agent",
            message: text,
        }),
        AgentEvent::AssistantDelta(text) => Some(TuiEvent::Transcript {
            role: "Agent Delta",
            message: text,
        }),
        AgentEvent::ToolUse { name, input } => Some(TuiEvent::Transcript {
            role: "Tool",
            message: format_tool_use(&name, &input),
        }),
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
        } => {
            if is_exploration_tool_name(&name) {
                return None;
            }
            Some(TuiEvent::Transcript {
                role: if is_error { "Tool Error" } else { "Tool Result" },
                message: format_tool_result(&name, &content),
            })
        }
    }
}

fn is_exploration_tool_name(name: &str) -> bool {
    matches!(name, "list_files" | "read_file" | "glob" | "grep" | "search_files")
}

fn format_tool_use(name: &str, input: &serde_json::Value) -> String {
    match name {
        "bash" => input
            .get("command")
            .and_then(serde_json::Value::as_str)
            .map(|command| format!("bash {command}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "read_file" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("read_file {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "write_file" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("write_file {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "replace" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("replace {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "list_files" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("list_files {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "grep" => {
            let pattern = input
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<pattern>");
            let path = input
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(".");
            format!("grep {pattern} in {path}")
        }
        "glob" => {
            let pattern = input
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<pattern>");
            let path = input
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(".");
            format!("glob {pattern} in {path}")
        }
        "web_fetch" => input
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(|url| format!("web_fetch {url}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "apply_patch" => "apply_patch".to_string(),
        _ => format!("{name} {input}"),
    }
}

fn format_tool_result(name: &str, content: &str) -> String {
    if name == "bash" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            let exit_code = value
                .get("exit_code")
                .and_then(serde_json::Value::as_i64)
                .map(|code| code.to_string())
                .unwrap_or_else(|| "?".to_string());
            let stdout = value
                .get("stdout")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let stderr = value
                .get("stderr")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let mut summary = format!("bash exit_code={exit_code}");
            if !stdout.trim().is_empty() {
                summary.push_str(&format!("\nstdout: {}", first_non_empty_line(stdout)));
            }
            if !stderr.trim().is_empty() {
                summary.push_str(&format!("\nstderr: {}", first_non_empty_line(stderr)));
            }
            return summary;
        }
    }

    if name == "list_files" {
        return content.to_string();
    }

    if let Some(summary) = content
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let mut rendered = format!("{name}: {summary}");
        if content.contains("full_result_path=") {
            rendered.push_str("\nfull result stored on disk");
        } else if content.lines().nth(1).is_some() {
            rendered.push_str("\npreview available");
        }
        return rendered;
    }

    format!("{name}: {content}")
}

fn first_non_empty_line(text: &str) -> &str {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(text)
}

pub(super) fn format_error_chain(err: &anyhow::Error) -> String {
    let mut lines = Vec::new();
    for (idx, cause) in err.chain().enumerate() {
        if idx == 0 {
            lines.push(cause.to_string());
        } else {
            lines.push(format!("caused by: {cause}"));
        }
    }
    lines.join("\n")
}
