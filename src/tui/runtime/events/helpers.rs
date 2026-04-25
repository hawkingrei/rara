use crate::tool::ToolOutputStream;
use crate::tools::bash::BashCommandInput;
use crate::tui::state::TuiApp;

pub(super) fn is_oauth_prompt_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("open this url in a browser and enter the one-time code:")
        || lower.contains("open this url if the browser does not launch automatically:")
}

pub(super) fn is_exploration_tool_name(name: &str) -> bool {
    matches!(name, "list_files" | "read_file" | "glob" | "grep")
}

pub(super) fn exploration_action_label(message: &str) -> Option<String> {
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

pub(super) fn planning_action_label(message: &str) -> Option<String> {
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

pub(super) fn tool_action_label(message: &str) -> Option<String> {
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

pub(super) fn exploration_note_lines(message: &str, planning_mode: bool) -> Vec<String> {
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
        if planning_mode && (is_planning_chatter(line) || looks_like_planning_summary(line)) {
            continue;
        }
        if !notes.iter().any(|existing| existing == line) {
            notes.push(line.to_string());
        }
    }
    notes
}

pub(super) fn is_planning_chatter(line: &str) -> bool {
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

pub(super) fn planning_note_lines(message: &str) -> Vec<String> {
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
            || looks_like_planning_summary(line)
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

pub(super) fn scrub_internal_control_tokens(message: &str) -> String {
    let mut cleaned = String::with_capacity(message.len());
    let mut chars = message.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if ch == '<' {
            if let Some(end) = message[idx..].find("|>") {
                let end_idx = idx + end;
                let candidate = &message[idx + 1..end_idx];
                if !candidate.is_empty()
                    && candidate
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    if cleaned.chars().last().is_some_and(|c| !c.is_whitespace()) {
                        cleaned.push('\n');
                    }
                    let skip_to = end_idx + 2;
                    while let Some(&(next_idx, _)) = chars.peek() {
                        if next_idx < skip_to {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    continue;
                }
            }
        }
        cleaned.push(ch);
    }

    cleaned
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

pub(super) fn looks_like_planning_summary(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with("i propose the following plan")
        || lower.starts_with("i propose the following")
        || lower.starts_with("based on the inspection")
        || lower.starts_with("based on the existing code")
        || lower.starts_with("the primary suggestions center on")
        || lower.starts_with("the core logic")
        || lower.starts_with("to provide comprehensive suggestions")
}

pub(super) fn format_tool_use(name: &str, input: &serde_json::Value) -> String {
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

pub(super) fn format_apply_patch_use(input: &serde_json::Value) -> String {
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

pub(super) fn format_tool_result(name: &str, content: &str) -> String {
    if matches!(name, "explore_agent" | "plan_agent") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            let summary = value
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| first_non_empty_line(content));
            let mut rendered = format!("{name} {summary}");
            if let Some(request) = value.get("request_user_input") {
                if let Some(question) = request.get("question").and_then(serde_json::Value::as_str)
                {
                    rendered.push_str(&format!("\nrequest_user_input: {}", question.trim()));
                }
                if let Some(options) = request.get("options").and_then(serde_json::Value::as_array)
                {
                    for option in options {
                        if let Some((label, description)) = option.as_array().and_then(|pair| {
                            Some((
                                pair.first()?.as_str()?,
                                pair.get(1)
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or(""),
                            ))
                        }) {
                            rendered.push_str(&format!(
                                "\noption: {} | {}",
                                label.trim(),
                                description.trim()
                            ));
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
            let live_streamed = value
                .get("live_streamed")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let mut summary = format!("bash completed exit_code={exit_code}");
            if live_streamed {
                summary.push_str("\nstreamed output shown above");
                return summary;
            }
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
            rendered.push_str("\\nfull result stored on disk");
        } else if content.lines().nth(1).is_some() {
            rendered.push_str("\\npreview available");
        }
        return rendered;
    }

    format!("{name}: {content}")
}

pub(super) fn format_tool_progress(name: &str, stream: ToolOutputStream, chunk: &str) -> String {
    let trimmed = chunk.trim_end_matches('\n');
    if trimmed.trim().is_empty() {
        return String::new();
    }
    let stream_label = match stream {
        ToolOutputStream::Stdout => "stdout",
        ToolOutputStream::Stderr => "stderr",
    };
    format!("{name} {stream_label}:\n{trimmed}\n")
}

pub(super) fn append_tool_progress(
    app: &mut TuiApp,
    name: &str,
    stream: ToolOutputStream,
    chunk: &str,
) -> bool {
    let rendered = format_tool_progress(name, stream, chunk);
    if rendered.is_empty() {
        return false;
    }

    if let Some(last) = app.active_turn.entries.last_mut() {
        if last.role == "Tool Progress" {
            last.message.push_str(&rendered);
            limit_tool_progress_entry(&mut last.message);
            return true;
        }
    }

    app.push_entry("Tool Progress", rendered);
    if let Some(last) = app.active_turn.entries.last_mut() {
        limit_tool_progress_entry(&mut last.message);
    }
    true
}

fn limit_tool_progress_entry(message: &mut String) {
    let line_count = message
        .as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count();
    if line_count <= super::TOOL_PROGRESS_LINE_LIMIT {
        return;
    }

    let remove_lines = line_count - super::TOOL_PROGRESS_LINE_LIMIT;
    let mut removed = 0_usize;
    let mut cutoff = 0_usize;
    for (index, byte) in message.bytes().enumerate() {
        if byte == b'\n' {
            removed += 1;
            if removed == remove_lines {
                cutoff = index + 1;
                break;
            }
        }
    }

    let mut folded = String::from("... live output truncated ...\n");
    folded.push_str(&message[cutoff..]);
    *message = folded;
}

pub(super) fn format_apply_patch_result(value: &serde_json::Value) -> String {
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

pub(super) fn exploration_result_note(message: &str) -> Option<String> {
    message.strip_prefix("explore_agent ").map(|summary| {
        let first_line = summary.lines().next().unwrap_or(summary).trim();
        format!("Sub-agent summary: {first_line}")
    })
}

pub(super) fn planning_result_note(message: &str) -> Option<String> {
    message.strip_prefix("plan_agent ").map(|summary| {
        let first_line = summary.lines().next().unwrap_or(summary).trim();
        format!("Sub-agent summary: {first_line}")
    })
}

pub(super) struct DelegatedRequestInput {
    pub(super) question: String,
    pub(super) options: Vec<(String, String)>,
    pub(super) note: Option<String>,
}

pub(super) fn subagent_request_input(message: &str) -> Option<DelegatedRequestInput> {
    if !(message.starts_with("explore_agent ") || message.starts_with("plan_agent ")) {
        return None;
    }

    let mut question = None;
    let mut options = Vec::new();
    let mut note = None;
    for line in message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
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
    use crate::redaction::redact_secrets;

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
