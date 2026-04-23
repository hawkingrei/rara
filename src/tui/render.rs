mod bottom_pane;
pub(crate) mod cells;
mod history_pipeline;
mod overlay;
mod viewport;
#[cfg(test)]
mod tests;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

pub(crate) use self::bottom_pane::desired_viewport_height;
use self::bottom_pane::render_bottom_pane;
pub(crate) use self::cells::{ActiveCell, HistoryCell};
use self::cells::{ActiveTurnCell, CommittedTurnCell, StartupCardCell};
use self::overlay::render_overlay;
use self::viewport::TranscriptViewport;
use super::custom_terminal::Frame;
use super::line_utils::prefix_lines;
use super::state::{TranscriptEntry, TuiApp};

pub fn render(f: &mut Frame, app: &TuiApp) {
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
    let viewport = transcript_viewport(app, area.width, area.height);
    if !app.has_any_transcript() && viewport.lines.is_empty() {
        if app.startup_card_inserted {
            f.render_widget(Paragraph::new(Vec::<Line<'static>>::new()), area);
            return;
        }
        let lines = vec![
            Line::from(Span::styled(
                "Ready.",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("Use the input bar below to start a task or run a local command."),
            Line::from(""),
            Line::from(Span::styled(
                "Start with:",
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  /help    browse built-in commands and runtime hints"),
            Line::from("  /model   choose provider first, then switch models"),
            Line::from("  /status  inspect runtime, tokens, cache, and session"),
            Line::from("  /quit    leave the TUI and restore the terminal"),
            Line::from(""),
            Line::from(Span::styled(
                "Prompt ideas:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Explain this repository structure."),
            Line::from("  Find the main agent loop and summarize it."),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        return;
    }

    viewport.render(f, area);
}

pub(crate) fn transcript_viewport(
    app: &TuiApp,
    width: u16,
    viewport_height: u16,
) -> TranscriptViewport {
    let lines = renderable_transcript_lines(app, width);
    let visual_row_count = transcript_visual_row_count(&lines, width);
    let scroll_offset = transcript_scroll_offset(app, viewport_height, visual_row_count);
    TranscriptViewport::new(lines, scroll_offset)
}

fn renderable_transcript_lines(app: &TuiApp, width: u16) -> Vec<Line<'static>> {
    let mut lines = committed_transcript_lines(app, width);

    let mut active_lines = active_turn_cell(app).display_lines(width);
    if !active_lines.is_empty() {
        if !lines.is_empty() {
            lines.push(turn_divider_line(width));
        }
        lines.append(&mut active_lines);
    }

    lines
}

fn committed_transcript_lines(app: &TuiApp, width: u16) -> Vec<Line<'static>> {
    {
        let cache = app.committed_render_cache.borrow();
        if cache.generation == app.committed_render_generation && cache.width == width {
            return cache.lines.clone();
        }
    }

    let cwd = (!app.snapshot.cwd.is_empty()).then(|| Path::new(app.snapshot.cwd.as_str()));
    let mut lines = Vec::new();
    for turn in &app.committed_turns {
        let mut turn_lines = committed_turn_lines(turn.entries.as_slice(), cwd, width);
        if turn_lines.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(turn_divider_line(width));
        }
        lines.append(&mut turn_lines);
    }

    let mut cache = app.committed_render_cache.borrow_mut();
    cache.generation = app.committed_render_generation;
    cache.width = width;
    cache.lines = lines.clone();
    lines
}

fn transcript_scroll_offset(app: &TuiApp, viewport_height: u16, transcript_line_count: usize) -> u16 {
    let max_offset = transcript_line_count.saturating_sub(viewport_height as usize);
    let top_offset = max_offset.saturating_sub(app.transcript_scroll);
    top_offset.min(u16::MAX as usize) as u16
}

fn transcript_visual_row_count(lines: &[Line<'static>], width: u16) -> usize {
    let wrap_width = width.max(1) as usize;
    lines.iter()
        .map(|line| line.width().max(1).div_ceil(wrap_width))
        .sum()
}

fn turn_divider_line(width: u16) -> Line<'static> {
    let divider_width = usize::from(width.max(8));
    Line::from(Span::styled(
        "─".repeat(divider_width),
        Style::default().fg(Color::DarkGray),
    ))
}

pub fn committed_turn_cell<'a>(
    entries: &'a [TranscriptEntry],
    cwd: Option<&'a Path>,
) -> CommittedTurnCell<'a> {
    CommittedTurnCell::new(entries, cwd)
}

pub(crate) fn committed_turn_lines(
    entries: &[TranscriptEntry],
    cwd: Option<&Path>,
    width: u16,
) -> Vec<Line<'static>> {
    committed_turn_cell(entries, cwd).display_lines(width)
}

pub fn active_turn_cell<'a>(app: &'a TuiApp) -> ActiveTurnCell<'a> {
    let cwd = (!app.snapshot.cwd.is_empty()).then(|| Path::new(app.snapshot.cwd.as_str()));
    ActiveTurnCell::new(app, cwd)
}

pub fn startup_card_cell(app: &TuiApp) -> StartupCardCell {
    StartupCardCell::new(
        app.current_model_label().to_string(),
        display_directory_for_startup(app),
    )
}

pub(crate) fn startup_card_lines(app: &TuiApp, width: u16) -> Vec<Line<'static>> {
    startup_card_cell(app).display_lines(width)
}

fn current_turn_exploration_summary(
    app: &TuiApp,
    current_turn: &[&TranscriptEntry],
    prefer_live_label: bool,
) -> Option<String> {
    current_turn_exploration_summary_from_entries(
        current_turn,
        app.is_busy() && prefer_live_label,
        app.runtime_phase_detail.as_deref(),
    )
}

pub(crate) fn current_turn_exploration_summary_from_entries(
    current_turn: &[&TranscriptEntry],
    _show_live_detail: bool,
    _live_detail: Option<&str>,
) -> Option<String> {
    let mut actions = Vec::new();
    for entry in current_turn {
        if entry.role != "Tool" {
            continue;
        }
        if let Some(action) = exploration_action_label(&entry.message) {
            if !actions.iter().any(|existing| existing == &action) {
                actions.push(action);
            }
        }
    }
    if actions.is_empty() {
        return None;
    }
    Some(compact_summary_lines(actions.as_slice(), 4, "more file(s) inspected"))
}

pub(crate) fn current_turn_tool_summary(
    current_turn: &[&TranscriptEntry],
    _show_live_detail: bool,
    _live_detail: Option<&str>,
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

    let lines = actions
        .into_iter()
        .map(|action| format!("└ {action}"))
        .collect::<Vec<_>>();

    Some(lines.join("\n"))
}

pub(crate) fn compact_summary_lines(
    items: &[String],
    max_visible: usize,
    more_label: &str,
) -> String {
    if items.is_empty() {
        return String::new();
    }

    let visible_count = items.len().min(max_visible);
    let hidden_count = items.len().saturating_sub(visible_count);
    let start = items.len().saturating_sub(visible_count);
    let mut lines = items[start..]
        .iter()
        .map(|item| format!("└ {item}"))
        .collect::<Vec<_>>();

    if hidden_count > 0 {
        lines.insert(0, format!("└ ... {hidden_count} {more_label}"));
    }

    lines.join("\n")
}

pub(crate) fn compact_summary_text(
    summary: &str,
    max_visible: usize,
    more_label: &str,
) -> String {
    let items = summary
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.trim_start_matches("└")
                .trim_start_matches("•")
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    if items.is_empty() {
        return summary.trim().to_string();
    }

    compact_summary_lines(items.as_slice(), max_visible, more_label)
}

pub(crate) fn prefixed_message_lines(
    role: &str,
    message: &str,
    max_lines: usize,
) -> Vec<Line<'static>> {
    if role == "You" {
        return user_message_lines(message, max_lines);
    }

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

fn user_message_lines(message: &str, max_lines: usize) -> Vec<Line<'static>> {
    let message_lines = message.lines().collect::<Vec<_>>();
    if message_lines.is_empty() {
        return vec![Line::from("›")];
    }

    let capped = if max_lines == usize::MAX {
        message_lines.len()
    } else {
        max_lines
    };

    let mut lines = Vec::new();
    if let Some(first) = message_lines.first() {
        lines.push(Line::from(format!("› {first}")));
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

pub(crate) fn formatted_message_lines(
    role: &str,
    message: &str,
    max_lines: usize,
    cwd: Option<&Path>,
) -> Vec<Line<'static>> {
    if role == "Agent" {
        return bulleted_markdown_message_lines(message, max_lines, cwd);
    }
    if role == "System" {
        return markdown_message_lines(role, message, max_lines, cwd);
    }
    prefixed_message_lines(role, message, max_lines)
}

fn bulleted_markdown_message_lines(
    message: &str,
    max_lines: usize,
    cwd: Option<&Path>,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    super::markdown::append_markdown(message, None, cwd, &mut rendered);
    let rendered_len = rendered.len();

    if rendered.is_empty() {
        return vec![Line::from("•")];
    }

    let capped = if max_lines == usize::MAX {
        rendered.len()
    } else {
        max_lines.min(rendered.len())
    };

    let mut lines = prefix_lines(
        rendered.into_iter().take(capped).collect(),
        Span::styled("• ", Style::default().add_modifier(Modifier::DIM)),
        Span::raw("  "),
    );
    if capped < rendered_len {
        lines.push(Line::from(Span::styled(
            format!("  ... {} more line(s)", rendered_len - capped),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn markdown_message_lines(
    role: &str,
    message: &str,
    max_lines: usize,
    cwd: Option<&Path>,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    super::markdown::append_markdown(message, None, cwd, &mut rendered);
    let rendered_len = rendered.len();

    if rendered.is_empty() {
        return vec![Line::from(role.to_string())];
    }

    let capped = if max_lines == usize::MAX {
        rendered.len()
    } else {
        max_lines.min(rendered.len())
    };

    let mut lines = vec![Line::from(role.to_string())];
    let prefixed = prefix_lines(
        rendered.into_iter().take(capped).collect(),
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

pub(crate) fn rendered_markdown_lines(
    role: &str,
    rendered: &[Line<'static>],
    max_lines: usize,
) -> Vec<Line<'static>> {
    if rendered.is_empty() {
        return vec![Line::from(role.to_string())];
    }

    let rendered_len = rendered.len();
    let capped = if max_lines == usize::MAX {
        rendered_len
    } else {
        max_lines.min(rendered_len)
    };

    let mut lines = vec![Line::from(role.to_string())];
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
    matches!(name, "list_files" | "read_file" | "glob" | "grep")
}

fn exploration_action_label(message: &str) -> Option<String> {
    let mut parts = message.split_whitespace();
    let name = parts.next()?;
    let rest = parts.collect::<Vec<_>>().join(" ");
    match name {
        "read_file" => Some(format!(
            "Read {}",
            if rest.is_empty() {
                "file"
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


pub(crate) fn section_span<'a>(title: &'a str, color: Color) -> Span<'a> {
    Span::styled(
        format!(" {} ", title),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn wrapped_history_line_count(lines: &[Line<'static>], width: u16) -> u16 {
    let wrap_width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(wrap_width))
        .sum::<usize>()
        .max(1) as u16
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

pub(crate) fn display_directory_for_startup(app: &TuiApp) -> String {
    let cwd = if app.snapshot.cwd.is_empty() {
        std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ".".to_string())
    } else {
        app.snapshot.cwd.clone()
    };
    if let Ok(home) = std::env::var("HOME") {
        if let Some(stripped) = cwd.strip_prefix(&home) {
            return format!("~{stripped}");
        }
    }
    cwd
}

pub(crate) fn truncate_for_startup_card(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let kept = value.chars().take(width - 1).collect::<String>();
    format!("{kept}…")
}

pub(crate) fn truncate_path_middle(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    if width <= 5 {
        return truncate_for_startup_card(value, width);
    }

    let keep_left = (width - 1) / 2;
    let keep_right = width - 1 - keep_left;
    let chars = value.chars().collect::<Vec<_>>();
    let left = chars.iter().take(keep_left).collect::<String>();
    let right = chars
        .iter()
        .rev()
        .take(keep_right)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{left}…{right}")
}

pub(crate) fn startup_card_inner_width(width: u16) -> Option<usize> {
    if width < 8 {
        return None;
    }
    Some(std::cmp::min(width.saturating_sub(4) as usize, 56))
}

pub(crate) fn with_border(lines: Vec<Line<'static>>, inner_width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(lines.len() + 3);
    let border_inner_width = inner_width + 2;
    out.push(Line::from(format!("╭{}╮", "─".repeat(border_inner_width))));

    for line in lines {
        let used_width = line
            .iter()
            .map(|span| display_width(span.content.as_ref()))
            .sum::<usize>();
        let mut spans = Vec::with_capacity(line.spans.len() + 3);
        spans.push(Span::from("│ "));
        spans.extend(line.into_iter());
        if used_width < inner_width {
            spans.push(Span::from(" ".repeat(inner_width - used_width)));
        }
        spans.push(Span::from(" │"));
        out.push(Line::from(spans));
    }

    out.push(Line::from(format!("╰{}╯", "─".repeat(border_inner_width))));
    out
}

pub(crate) fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}
