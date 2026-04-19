use crate::agent::AgentEvent;
use crate::tools::bash::BashCommandInput;

use super::super::state::{
    contains_structured_planning_output, RuntimePhase, TuiApp, TuiEvent,
};

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
                if role == "Tool" {
                    if let Some(action) = exploration_action_label(&message) {
                        app.record_exploration_action(action);
                    } else if let Some(action) = planning_action_label(&message) {
                        app.record_planning_action(action);
                    } else if let Some(action) = tool_action_label(&message) {
                        app.record_running_action(action);
                    }
                } else if let Some(note) = exploration_result_note(&message) {
                    app.record_exploration_note(note);
                    app.set_runtime_phase(
                        RuntimePhase::RunningTool,
                        Some(message.lines().next().unwrap_or(role).trim().to_string()),
                    );
                    return;
                } else if let Some(note) = planning_result_note(&message) {
                    app.record_planning_note(note);
                    app.set_runtime_phase(
                        RuntimePhase::RunningTool,
                        Some(message.lines().next().unwrap_or(role).trim().to_string()),
                    );
                    if let Some(request) = subagent_request_input(&message) {
                        app.record_local_request_input(
                            "plan_agent",
                            request.question,
                            request.options,
                            request.note,
                        );
                    }
                    return;
                } else if let Some(request) = subagent_request_input(&message) {
                    let source = if message.starts_with("explore_agent ") {
                        "explore_agent"
                    } else {
                        "plan_agent"
                    };
                    app.record_local_request_input(
                        source,
                        request.question,
                        request.options,
                        request.note,
                    );
                    app.set_runtime_phase(
                        RuntimePhase::RunningTool,
                        Some(message.lines().next().unwrap_or(role).trim().to_string()),
                    );
                    return;
                }
                app.set_runtime_phase(
                    RuntimePhase::RunningTool,
                    Some(message.lines().next().unwrap_or(role).trim().to_string()),
                );
            } else if role == "Agent" {
                let planning_mode =
                    matches!(app.agent_execution_mode, crate::agent::AgentExecutionMode::Plan);
                let has_live_exploration = !app.active_live.exploration_actions.is_empty()
                    || !app.active_live.exploration_notes.is_empty();
                if !app.active_live.exploration_actions.is_empty()
                    && matches!(
                        app.runtime_phase,
                        RuntimePhase::RunningTool | RuntimePhase::SendingPrompt
                    )
                {
                    for note in exploration_note_lines(&message, planning_mode) {
                        app.record_exploration_note(note);
                    }
                }
                app.set_runtime_phase(
                    RuntimePhase::ProcessingResponse,
                    Some("receiving model output".into()),
                );
                if planning_mode && !contains_structured_planning_output(&message) {
                    for note in planning_note_lines(&message) {
                        app.record_planning_note(note);
                    }
                    if has_live_exploration
                        || !app.active_live.planning_actions.is_empty()
                        || !app.active_live.planning_notes.is_empty()
                    {
                        app.agent_markdown_stream = None;
                        return;
                    }
                }
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
                role: if is_error {
                    "Tool Error"
                } else {
                    "Tool Result"
                },
                message: format_tool_result(&name, &content),
            })
        }
    }
}

fn is_exploration_tool_name(name: &str) -> bool {
    matches!(name, "list_files" | "read_file" | "glob" | "grep")
}

fn exploration_action_label(message: &str) -> Option<String> {
    let mut parts = message.split_whitespace();
    let name = parts.next()?;
    let rest = parts.collect::<Vec<_>>().join(" ");
    match name {
        "list_files" => Some(format!(
            "List {}",
            if rest.is_empty() { "." } else { rest.as_str() }
        )),
        "read_file" => Some(format!(
            "Read {}",
            if rest.is_empty() {
                "file"
            } else {
                rest.as_str()
            }
        )),
        "glob" => Some(format!(
            "Glob {}",
            if rest.is_empty() {
                "workspace"
            } else {
                rest.as_str()
            }
        )),
        "grep" => Some(format!(
            "Search {}",
            if rest.is_empty() {
                "workspace"
            } else {
                rest.as_str()
            }
        )),
        "explore_agent" => Some(if rest.is_empty() {
            "Delegate repository exploration".to_string()
        } else {
            format!("Delegate repository exploration: {rest}")
        }),
        _ => None,
    }
}

fn planning_action_label(message: &str) -> Option<String> {
    let mut parts = message.split_whitespace();
    let name = parts.next()?;
    let rest = parts.collect::<Vec<_>>().join(" ");
    match name {
        "plan_agent" => Some(if rest.is_empty() {
            "Delegate plan refinement".to_string()
        } else {
            format!("Delegate plan refinement: {rest}")
        }),
        _ => None,
    }
}

fn tool_action_label(message: &str) -> Option<String> {
    let mut parts = message.split_whitespace();
    let name = parts.next()?;
    if is_exploration_tool_name(name) {
        return None;
    }
    let rest = parts.collect::<Vec<_>>().join(" ");
    match name {
        "bash" => Some(format!(
            "Run {}",
            if rest.is_empty() {
                "command"
            } else {
                rest.as_str()
            }
        )),
        "apply_patch" => Some(format!(
            "Apply patch {}",
            if rest.is_empty() {
                "changes"
            } else {
                rest.as_str()
            }
        )),
        "write_file" => Some(format!(
            "Write {}",
            if rest.is_empty() {
                "file"
            } else {
                rest.as_str()
            }
        )),
        "replace" => Some(format!(
            "Edit {}",
            if rest.is_empty() {
                "file"
            } else {
                rest.as_str()
            }
        )),
        "web_fetch" => Some(format!(
            "Fetch {}",
            if rest.is_empty() {
                "resource"
            } else {
                rest.as_str()
            }
        )),
        other => Some(format!(
            "Run {}",
            if rest.is_empty() { other } else { message }
        )),
    }
}

fn exploration_note_lines(message: &str, planning_mode: bool) -> Vec<String> {
    let mut notes = Vec::new();
    for line in message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("/search ")
            || line.starts_with("/compact ")
            || line.starts_with("/plan ")
            || line.starts_with("/quit ")
            || line.starts_with("key=")
            || line.starts_with("history=")
            || line.starts_with("tokens=")
            || line.starts_with("ctx~=")
            || line.starts_with("waiting for model response")
        {
            continue;
        }
        if planning_mode && is_planning_chatter(line) {
            continue;
        }
        if !notes.iter().any(|existing| existing == line) {
            notes.push(line.to_string());
        }
    }
    notes
}

fn is_planning_chatter(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with("i will now ")
        || lower.starts_with("i will start by ")
        || lower.starts_with("i will use ")
        || lower.starts_with("i am ready to ")
        || lower.starts_with("i'm ready to ")
        || lower.starts_with("i need to ")
        || lower.starts_with("to continue ")
        || lower.starts_with("to fully ")
        || lower.starts_with("to understand ")
        || lower.starts_with("this is the final step")
        || lower.starts_with("based on the existing code")
}

fn planning_note_lines(message: &str) -> Vec<String> {
    let mut notes = Vec::new();
    for line in message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("/compact ")
            || line.starts_with("/plan ")
            || line.starts_with("/quit ")
            || line.starts_with("waiting for model response")
            || is_planning_chatter(line)
            || mentions_mutating_plan_action(line)
        {
            continue;
        }
        if !notes.iter().any(|existing| existing == line) {
            notes.push(line.to_string());
        }
    }
    notes
}

fn mentions_mutating_plan_action(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("apply_patch")
        || lower.contains("write_file")
        || lower.contains("apply the patch")
        || lower.contains("write the file")
        || lower.contains("edit ")
        || lower.contains("replace ")
        || lower.contains("modify ")
        || lower.contains("write ")
        || lower.contains("edit files")
        || lower.contains("modify the code")
        || lower.contains("implement the change")
}

fn format_tool_use(name: &str, input: &serde_json::Value) -> String {
    match name {
        "bash" => BashCommandInput::from_value(input.clone())
            .map(|request| format!("bash {}", request.summary()))
            .unwrap_or_else(|_| format!("{name} {input}")),
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
        "apply_patch" => format_apply_patch_use(input),
        _ => format!("{name} {input}"),
    }
}

fn format_apply_patch_use(input: &serde_json::Value) -> String {
    let Some(patch) = input.get("patch").and_then(serde_json::Value::as_str) else {
        return "apply_patch".to_string();
    };

    let files = apply_patch_targets(patch);
    if files.is_empty() {
        "apply_patch".to_string()
    } else {
        format!("apply_patch {}", files.join(", "))
    }
}

fn apply_patch_targets(patch: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in patch.lines().map(str::trim) {
        let path = line
            .strip_prefix("*** Add File: ")
            .or_else(|| line.strip_prefix("*** Delete File: "))
            .or_else(|| line.strip_prefix("*** Update File: "))
            .or_else(|| line.strip_prefix("*** Move to: "));
        let Some(path) = path else {
            continue;
        };
        if !files.iter().any(|existing| existing == path) {
            files.push(path.to_string());
        }
    }
    files
}

fn format_tool_result(name: &str, content: &str) -> String {
    if matches!(name, "explore_agent" | "plan_agent") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            let summary = value
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| first_non_empty_line(content));
            let mut rendered = format!("{name} {summary}");
            if let Some(request) = value.get("request_user_input") {
                if let Some(question) = request.get("question").and_then(serde_json::Value::as_str) {
                    rendered.push_str(&format!("\nrequest_user_input: {}", question.trim()));
                }
                if let Some(options) = request.get("options").and_then(serde_json::Value::as_array) {
                    for option in options {
                        if let Some((label, description)) = option
                            .as_array()
                            .and_then(|pair| {
                                Some((
                                    pair.first()?.as_str()?,
                                    pair.get(1).and_then(serde_json::Value::as_str).unwrap_or(""),
                                ))
                            })
                        {
                            rendered.push_str(&format!("\noption: {} | {}", label.trim(), description.trim()));
                        }
                    }
                }
                if let Some(note) = request.get("note").and_then(serde_json::Value::as_str) {
                    if !note.trim().is_empty() {
                        rendered.push_str(&format!("\nnote: {}", note.trim()));
                    }
                }
            }
            return rendered;
        }
        return format!("{name} {}", first_non_empty_line(content));
    }
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
            if let Some(stdout_preview) = output_tail_preview(stdout) {
                summary.push_str(&format!("\nstdout:\n{stdout_preview}"));
            }
            if let Some(stderr_preview) = output_tail_preview(stderr) {
                summary.push_str(&format!("\nstderr:\n{stderr_preview}"));
            }
            return summary;
        }
    }

    if name == "apply_patch" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            return format_apply_patch_result(&value);
        }
    }

    if name == "write_file" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            return format_write_file_result(&value);
        }
    }

    if name == "replace" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            return format_replace_result(&value);
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

fn format_apply_patch_result(value: &serde_json::Value) -> String {
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let files_changed = value
        .get("files_changed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let added = value
        .get("line_delta")
        .and_then(|delta| delta.get("added"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let removed = value
        .get("line_delta")
        .and_then(|delta| delta.get("removed"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();

    let mut lines = vec![format!(
        "apply_patch {status} {files_changed} file(s) (+{added} -{removed})"
    )];

    append_path_group(&mut lines, "updated", value.get("updated_files"));
    append_path_group(&mut lines, "created", value.get("created_files"));
    append_path_group(&mut lines, "deleted", value.get("deleted_files"));

    if let Some(moves) = value
        .get("moved_files")
        .and_then(serde_json::Value::as_array)
    {
        let rendered = moves
            .iter()
            .filter_map(|entry| {
                let from = entry.get("from").and_then(serde_json::Value::as_str)?;
                let to = entry.get("to").and_then(serde_json::Value::as_str)?;
                Some(format!("{from} -> {to}"))
            })
            .collect::<Vec<_>>();
        if !rendered.is_empty() {
            lines.push(format!("moved: {}", rendered.join(", ")));
        }
    }

    if let Some(summary) = value.get("summary").and_then(serde_json::Value::as_array) {
        let preview = summary
            .iter()
            .filter_map(serde_json::Value::as_str)
            .take(4)
            .collect::<Vec<_>>();
        if !preview.is_empty() {
            let remaining = summary.len().saturating_sub(preview.len());
            lines.push("changes:".to_string());
            for line in preview {
                lines.push(format!("  {line}"));
            }
            if remaining > 0 {
                lines.push(format!("  ... {remaining} more change(s)"));
            }
        }
    }

    lines.join("\n")
}

fn append_path_group(lines: &mut Vec<String>, label: &str, value: Option<&serde_json::Value>) {
    let Some(paths) = value.and_then(serde_json::Value::as_array) else {
        return;
    };
    let rendered = paths
        .iter()
        .filter_map(serde_json::Value::as_str)
        .take(4)
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        return;
    }
    lines.push(format!("{label}: {}", rendered.join(", ")));
    let remaining = paths.len().saturating_sub(rendered.len());
    if remaining > 0 {
        lines.push(format!("  ... {remaining} more"));
    }
}

fn output_tail_preview(output: &str) -> Option<String> {
    let lines = output
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let preview = lines
        .iter()
        .rev()
        .take(6)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    Some(preview.join("\n"))
}

fn format_write_file_result(value: &serde_json::Value) -> String {
    let path = value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<unknown>");
    let operation = value
        .get("operation")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("updated");
    let line_count = value
        .get("line_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let bytes_written = value
        .get("bytes_written")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();

    let mut lines = vec![format!(
        "write_file {operation} {path} ({line_count} lines, {bytes_written} bytes)"
    )];

    if let Some(previous_bytes) = value
        .get("previous_bytes")
        .and_then(serde_json::Value::as_u64)
    {
        let previous_lines = value
            .get("previous_line_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        lines.push(format!(
            "previous: {previous_lines} lines, {previous_bytes} bytes"
        ));
    }

    lines.join("\n")
}

fn format_replace_result(value: &serde_json::Value) -> String {
    let path = value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<unknown>");
    let replacements = value
        .get("replacements")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let line_delta = value
        .get("line_delta")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    let mut lines = vec![format!(
        "replace {path} {replacements} replacement(s) (Δlines={line_delta})"
    )];
    if let Some(old_preview) = value.get("old_preview").and_then(serde_json::Value::as_str) {
        lines.push(format!("old: {old_preview}"));
    }
    if let Some(new_preview) = value.get("new_preview").and_then(serde_json::Value::as_str) {
        lines.push(format!("new: {new_preview}"));
    }
    lines.join("\n")
}

fn exploration_result_note(message: &str) -> Option<String> {
    message
        .strip_prefix("explore_agent ")
        .map(|summary| {
            let first_line = summary.lines().next().unwrap_or(summary).trim();
            format!("Sub-agent summary: {first_line}")
        })
}

fn planning_result_note(message: &str) -> Option<String> {
    message
        .strip_prefix("plan_agent ")
        .map(|summary| {
            let first_line = summary.lines().next().unwrap_or(summary).trim();
            format!("Sub-agent summary: {first_line}")
        })
}

struct DelegatedRequestInput {
    question: String,
    options: Vec<(String, String)>,
    note: Option<String>,
}

fn subagent_request_input(message: &str) -> Option<DelegatedRequestInput> {
    if !(message.starts_with("explore_agent ") || message.starts_with("plan_agent ")) {
        return None;
    }

    let mut question = None;
    let mut options = Vec::new();
    let mut note = None;
    for line in message.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = line.strip_prefix("request_user_input:") {
            question = Some(value.trim().to_string());
            continue;
        }
        if let Some(value) = line.strip_prefix("option:") {
            let value = value.trim();
            if let Some((label, description)) = value.split_once('|') {
                options.push((label.trim().to_string(), description.trim().to_string()));
            } else {
                options.push((value.to_string(), String::new()));
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("note:") {
            let value = value.trim();
            if !value.is_empty() {
                note = Some(value.to_string());
            }
        }
    }

    Some(DelegatedRequestInput {
        question: question?,
        options,
        note,
    })
}

fn first_non_empty_line(text: &str) -> &str {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(text)
}

pub(super) fn format_error_chain(err: &anyhow::Error) -> String {
    let mut lines = Vec::new();
    for (idx, cause) in err.chain().enumerate() {
        let rendered = redact_secrets(cause.to_string());
        if idx == 0 {
            lines.push(rendered);
        } else {
            lines.push(format!("caused by: {rendered}"));
        }
    }
    lines.join("\n")
}
use crate::redaction::redact_secrets;

#[cfg(test)]
mod tests {
    use super::{
        format_apply_patch_result, format_apply_patch_use, format_tool_result, planning_note_lines,
        subagent_request_input,
    };
    use serde_json::json;

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
    fn formats_apply_patch_tool_use_with_target_files() {
        let rendered = format_apply_patch_use(&json!({
            "patch": "*** Begin Patch\n*** Update File: src/tui/render.rs\n@@\n-old\n+new\n*** Update File: src/tui/runtime/events.rs\n@@\n-old\n+new\n*** End Patch"
        }));
        assert_eq!(rendered, "apply_patch src/tui/render.rs, src/tui/runtime/events.rs");
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

        assert!(rendered.contains("bash exit_code=0"));
        assert!(rendered.contains("stdout:\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7"));
        assert!(rendered.contains("stderr:\nwarn 1\nwarn 2"));
    }
}
