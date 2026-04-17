mod bottom_pane;
mod overlay;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;
use std::path::Path;

use super::custom_terminal::Frame;
use super::line_utils::prefix_lines;
use super::state::{TranscriptEntry, TuiApp};
pub(crate) use bottom_pane::desired_viewport_height;
use bottom_pane::render_bottom_pane;
use overlay::render_overlay;

pub fn render(f: &mut Frame, app: &TuiApp) {
    let browsing_history = app.overlay.is_none() && app.transcript_scroll > 0;
    if browsing_history {
        render_transcript(f, app, f.area());
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(5)])
        .split(f.area());

    render_transcript(f, app, layout[0]);
    let mut cursor = render_bottom_pane(f, app, layout[1]);

    if let Some(overlay) = app.overlay {
        cursor = render_overlay(f, app, overlay).or(cursor);
    }

    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

fn render_transcript(f: &mut Frame, app: &TuiApp, area: Rect) {
    if !app.has_any_transcript() {
        if app.startup_card_inserted {
            f.render_widget(Paragraph::new(Vec::<Line<'static>>::new()), area);
            return;
        }
        let lines = vec![
            Line::from(Span::styled(
                "Ready.",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from("Use the input bar below to start a task or run a local command."),
            Line::from(""),
            Line::from(Span::styled(
                "Start with:",
                Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD),
            )),
            Line::from("  /help    browse built-in commands and runtime hints"),
            Line::from("  /search  grep the workspace without going through the model"),
            Line::from("  /model   choose provider first, then switch models"),
            Line::from("  /status  inspect runtime, tokens, cache, and session"),
            Line::from("  /quit    leave the TUI and restore the terminal"),
            Line::from(""),
            Line::from(Span::styled(
                "Prompt ideas:",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from("  Explain this repository structure."),
            Line::from("  Find the main agent loop and summarize it."),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        return;
    }

    let mut lines = Vec::new();
    lines.extend(current_turn_lines(app));
    let scroll_y = bottom_anchored_scroll(lines.as_slice(), area, app.transcript_scroll);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll_y, 0)),
        area,
    );
}

fn bottom_anchored_scroll(lines: &[Line<'static>], area: Rect, scroll_from_bottom: usize) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 0;
    }

    let wrap_width = area.width as usize;
    let total_rows = lines
        .iter()
        .map(|line| {
            let width = line
                .spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>()
                .max(1);
            width.div_ceil(wrap_width)
        })
        .sum::<usize>();
    let max_scroll = total_rows.saturating_sub(area.height as usize);
    max_scroll.saturating_sub(scroll_from_bottom) as u16
}

pub fn committed_turn_lines(entries: &[TranscriptEntry], cwd: Option<&Path>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(user) = entries.iter().find(|entry| entry.role == "You") {
        lines.extend(prefixed_message_lines("You", &user.message, 4));
        lines.push(Line::from(""));
    }

    let entry_refs = entries.iter().collect::<Vec<_>>();
    let has_tool_activity = entry_refs
        .iter()
        .any(|entry| matches!(entry.role.as_str(), "Tool" | "Tool Result" | "Tool Error"));
    if let Some(summary) =
        current_turn_exploration_summary_from_entries(entry_refs.as_slice(), false, None)
    {
        lines.push(active_heading("Explored", Color::Rgb(231, 201, 92)));
        lines.extend(summary.lines().map(|line| Line::from(format!("  {line}"))));
        lines.push(Line::from(""));
    }

    if let Some(summary) = current_turn_tool_summary(entry_refs.as_slice(), false, None) {
        lines.push(active_heading("Ran", Color::LightYellow));
        lines.extend(summary.lines().map(|line| Line::from(format!("  {line}"))));
        lines.push(Line::from(""));
    }

    let tail_entries: Vec<&TranscriptEntry> = if has_tool_activity {
        entries
            .iter()
            .rev()
            .filter(|entry| matches!(entry.role.as_str(), "Agent" | "System"))
            .take(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        entries
            .iter()
            .filter(|entry| matches!(entry.role.as_str(), "Agent" | "System"))
            .collect()
    };

    for entry in tail_entries {
        let max_lines = if entry.role == "Agent" { 8 } else { 4 };
        lines.extend(formatted_message_lines(
            &entry.role,
            &entry.message,
            max_lines,
            cwd,
            false,
        ));
        lines.push(Line::from(""));
    }

    while matches!(lines.last(), Some(line) if line.spans.iter().all(|span| span.content == "")) {
        lines.pop();
    }
    lines
}

fn current_turn_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let current_turn = app.active_turn.entries.iter().collect::<Vec<_>>();
    if current_turn.is_empty() {
        return Vec::new();
    }
    let has_tool_activity = current_turn
        .iter()
        .any(|entry| matches!(entry.role.as_str(), "Tool" | "Tool Result" | "Tool Error"));
    let user_message = current_turn
        .iter()
        .find(|entry| entry.role == "You")
        .map(|entry| entry.message.as_str())
        .unwrap_or("");
    let latest_agent = current_turn
        .iter()
        .rev()
        .find(|entry| entry.role == "Agent")
        .map(|entry| entry.message.as_str());
    let streaming_agent_lines = app.agent_stream_lines();
    let latest_system = current_turn
        .iter()
        .rev()
        .find(|entry| entry.role == "System")
        .map(|entry| entry.message.as_str());
    let latest_tool_result = current_turn
        .iter()
        .rev()
        .find(|entry| entry.role == "Tool Result" || entry.role == "Tool Error")
        .map(|entry| (entry.role.as_str(), entry.message.as_str()));
    let mut lines = Vec::new();
    let cwd = (!app.snapshot.cwd.is_empty()).then(|| Path::new(app.snapshot.cwd.as_str()));

    if !user_message.is_empty() {
        lines.extend(prefixed_message_lines("You", user_message, 4));
        lines.push(Line::from(""));
    }

    if app.agent_execution_mode_label() == "plan" {
        lines.push(active_heading("Plan Mode", Color::LightBlue));
        lines.push(Line::from(""));
    }

    if let Some(summary) =
        current_turn_exploration_summary(app, current_turn.as_slice(), latest_agent.is_none())
    {
        let (title, color) = if app.is_busy() && latest_agent.is_none() {
            ("Exploring", Color::Yellow)
        } else {
            ("Explored", Color::Rgb(231, 201, 92))
        };
        lines.push(active_heading(title, color));
        lines.extend(summary.lines().map(|line| Line::from(format!("  {line}"))));
        lines.push(Line::from(""));
    }

    if let Some(summary) = current_turn_tool_summary(
        current_turn.as_slice(),
        app.is_busy() && latest_agent.is_none(),
        app.runtime_phase_detail.as_deref(),
    ) {
        let (title, color) = if app.is_busy() && latest_agent.is_none() {
            ("Running", Color::Yellow)
        } else {
            ("Ran", Color::LightYellow)
        };
        lines.push(active_heading(title, color));
        lines.extend(summary.lines().map(|line| Line::from(format!("  {line}"))));
        lines.push(Line::from(""));
    }

    if !app.snapshot.plan_steps.is_empty() {
        lines.push(active_heading("Plan", Color::LightBlue));
        for line in super::command::status_plan_text(app).lines().take(8) {
            lines.push(Line::from(format!("  {line}")));
        }
        lines.push(Line::from(""));
    }

    if app.snapshot.pending_question.is_some() {
        let (title, color) = if app.has_pending_approval() {
            ("Approval", Color::Yellow)
        } else {
            ("Request Input", Color::LightGreen)
        };
        lines.push(active_heading(title, color));
        for line in super::command::status_request_user_input_text(app).lines().take(8) {
            lines.push(Line::from(format!("  {line}")));
        }
        lines.push(Line::from("  shortcuts: press 1/2/3 to answer immediately"));
        lines.push(Line::from(""));
    }

    if let Some((title, summary)) = app.snapshot.completed_approval.as_ref() {
        lines.push(active_heading("Approval Completed", Color::LightGreen));
        lines.push(Line::from(format!("  {}: {}", title, summary)));
        lines.push(Line::from(""));
    }

    if let Some((title, summary)) = app.snapshot.completed_question.as_ref() {
        lines.push(active_heading("Question Answered", Color::LightGreen));
        lines.push(Line::from(format!("  {}: {}", title, summary)));
        lines.push(Line::from(""));
    }

    let suppress_intermediate_agent = app.is_busy()
        && has_tool_activity
        && matches!(
            app.runtime_phase,
            super::state::RuntimePhase::RunningTool | super::state::RuntimePhase::SendingPrompt
        );

    if let Some(stream_lines) = streaming_agent_lines.filter(|_| !suppress_intermediate_agent) {
        let role = if app.is_busy() { "Responding" } else { "Agent" };
        lines.extend(rendered_markdown_lines(role, stream_lines, usize::MAX, true));
    } else if let Some(agent_message) = latest_agent.filter(|_| !suppress_intermediate_agent) {
        let role = if app.is_busy() { "Responding" } else { "Agent" };
        lines.extend(formatted_message_lines(
            role,
            agent_message,
            usize::MAX,
            cwd,
            true,
        ));
    } else if let Some(system_message) = latest_system {
        lines.extend(formatted_message_lines(
            "System",
            system_message,
            14,
            cwd,
            false,
        ));
    } else if let Some((role, tool_result)) = latest_tool_result {
        lines.extend(prefixed_message_lines(role, tool_result, 14));
    } else if app.is_busy() {
        lines.push(active_heading("Working", Color::Yellow));
        lines.push(Line::from(format!(
            "  {}",
            summarize_live_detail(app.runtime_phase_detail.as_deref())
                .as_deref()
                .unwrap_or("waiting for the current turn to finish")
        )));
    }

    lines
}

fn current_turn_exploration_summary(
    app: &TuiApp,
    current_turn: &[&TranscriptEntry],
    prefer_live_label: bool,
) -> Option<String> {
    current_turn_exploration_summary_from_entries(
        current_turn,
        app.is_busy() && prefer_live_label,
        summarize_live_detail(app.runtime_phase_detail.as_deref()).as_deref(),
    )
}

fn current_turn_exploration_summary_from_entries(
    current_turn: &[&TranscriptEntry],
    show_live_detail: bool,
    live_detail: Option<&str>,
) -> Option<String> {
    let mut actions = Vec::new();
    for entry in current_turn {
        if entry.role != "Tool" {
            continue;
        }
        if let Some(action) = exploration_action_label(&entry.message) {
            actions.push(action);
        }
    }
    if actions.is_empty() {
        return None;
    }

    let mut lines = actions
        .into_iter()
        .map(|action| format!("└ {action}"))
        .collect::<Vec<_>>();

    if show_live_detail {
        lines.push(format!(
            "└ {}",
            live_detail.unwrap_or("waiting for more exploration output")
        ));
    }

    Some(lines.join("\n"))
}

fn current_turn_tool_summary(
    current_turn: &[&TranscriptEntry],
    show_live_detail: bool,
    live_detail: Option<&str>,
) -> Option<String> {
    let actions = current_turn
        .iter()
        .filter_map(|entry| {
            if entry.role != "Tool" {
                return None;
            }
            tool_action_label(&entry.message)
        })
        .collect::<Vec<_>>();
    if actions.is_empty() {
        return None;
    }

    let mut lines = actions
        .into_iter()
        .map(|action| format!("└ {action}"))
        .collect::<Vec<_>>();

    if show_live_detail {
        lines.push(format!(
            "└ {}",
            live_detail.unwrap_or("waiting for tool output")
        ));
    }

    Some(lines.join("\n"))
}

fn summarize_live_detail(detail: Option<&str>) -> Option<String> {
    let detail = detail?.trim();
    if detail.is_empty() || looks_like_bottom_pane_chrome(detail) {
        return None;
    }
    Some(detail.to_string())
}

fn looks_like_bottom_pane_chrome(detail: &str) -> bool {
    detail.contains("/compact")
        || detail.contains("/quit")
        || detail.contains("/plan")
        || detail.contains("key=")
        || detail.contains("history=")
        || detail.contains("tokens=")
        || detail.contains("ctx~=")
}

fn prefixed_message_lines(role: &str, message: &str, max_lines: usize) -> Vec<Line<'static>> {
    let message_lines = message.lines().collect::<Vec<_>>();
    if message_lines.is_empty() {
        return vec![Line::from(format!("{role}:"))];
    }

    let capped = if max_lines == usize::MAX {
        message_lines.len()
    } else {
        max_lines
    };

    let mut lines = Vec::new();
    if let Some(first) = message_lines.first() {
        lines.push(Line::from(format!("{role}: {first}")));
    }
    for line in message_lines.iter().skip(1).take(capped.saturating_sub(1)) {
        lines.push(Line::from(format!("  {line}")));
    }
    if message_lines.len() > capped {
        lines.push(Line::from(Span::styled(
            format!("  ... {} more line(s)", message_lines.len() - capped),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn formatted_message_lines(
    role: &str,
    message: &str,
    max_lines: usize,
    cwd: Option<&Path>,
    active: bool,
) -> Vec<Line<'static>> {
    if matches!(role, "Agent" | "System") {
        return markdown_message_lines(role, message, max_lines, cwd, active);
    }
    prefixed_message_lines(role, message, max_lines)
}

fn markdown_message_lines(
    role: &str,
    message: &str,
    max_lines: usize,
    cwd: Option<&Path>,
    active: bool,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    super::markdown::append_markdown(message, None, cwd, &mut rendered);
    rendered_markdown_lines(role, rendered.as_slice(), max_lines, active)
}

fn rendered_markdown_lines(
    role: &str,
    rendered: &[Line<'static>],
    max_lines: usize,
    active: bool,
) -> Vec<Line<'static>> {
    if rendered.is_empty() {
        return vec![if active {
            active_heading(role, Color::Cyan)
        } else {
            Line::from(role.to_string())
        }];
    }

    let rendered_len = rendered.len();
    let capped = if max_lines == usize::MAX {
        rendered_len
    } else {
        max_lines.min(rendered_len)
    };

    let mut lines = vec![if active {
        active_heading(role, Color::Cyan)
    } else {
        Line::from(role.to_string())
    }];
    let prefixed = prefix_lines(
        rendered.iter().take(capped).cloned().collect(),
        Span::raw("  "),
        Span::raw("  "),
    );
    lines.extend(prefixed);
    if capped < rendered_len {
        lines.push(Line::from(Span::styled(
            format!("  ... {} more line(s)", rendered_len - capped),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn is_exploration_tool(name: &str) -> bool {
    matches!(name, "list_files" | "read_file" | "glob" | "grep" | "search_files")
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
            if rest.is_empty() { "file" } else { rest.as_str() }
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
        "search_files" => Some(format!(
            "Search files {}",
            if rest.is_empty() {
                "workspace"
            } else {
                rest.as_str()
            }
        )),
        _ => None,
    }
}

fn tool_action_label(message: &str) -> Option<String> {
    let mut parts = message.split_whitespace();
    let name = parts.next()?;
    if is_exploration_tool(name) {
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
        "apply_patch" => Some("Apply patch".to_string()),
        "write_file" => Some(format!(
            "Write {}",
            if rest.is_empty() { "file" } else { rest.as_str() }
        )),
        "replace" => Some(format!(
            "Edit {}",
            if rest.is_empty() { "file" } else { rest.as_str() }
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

#[cfg(test)]
mod tests {
    use super::{looks_like_bottom_pane_chrome, summarize_live_detail};

    #[test]
    fn filters_bottom_pane_chrome_from_live_detail() {
        assert!(summarize_live_detail(Some("/compact summarize history  /plan next turn  /quit exit")).is_none());
        assert!(summarize_live_detail(Some("key=not-required  history=0  tokens=0 in / 0 out  ctx~=0")).is_none());
        assert_eq!(
            summarize_live_detail(Some("waiting for model response · 34s elapsed")).as_deref(),
            Some("waiting for model response · 34s elapsed")
        );
        assert!(looks_like_bottom_pane_chrome("key=not-required  history=0"));
    }
}

fn active_heading(title: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled("•", Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(
            title.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn badge<'a>(label: &'a str, value: &'a str, color: Color) -> Span<'a> {
    let fg = match color {
        Color::Black
        | Color::DarkGray
        | Color::Gray
        | Color::Blue
        | Color::Red
        | Color::Magenta => Color::White,
        _ => Color::Black,
    };
    Span::styled(
        format!(" {}={} ", label, value),
        Style::default()
            .fg(fg)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}
