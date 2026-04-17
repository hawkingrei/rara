use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use textwrap::Options;
use unicode_width::UnicodeWidthStr;

use super::super::custom_terminal::Frame;
use super::super::command::api_key_status;
use super::super::state::{TaskKind, TuiApp};
use super::badge;

pub(crate) fn desired_viewport_height(app: &TuiApp, _width: u16, rows: u16) -> u16 {
    if app.overlay.is_some() || app.transcript_scroll > 0 {
        return rows.max(1);
    }

    if app.has_any_transcript() {
        return rows.max(1);
    }

    let bottom_pane_height = 5u16;
    let has_active_content = !app.active_turn.entries.is_empty();
    if !has_active_content {
        return bottom_pane_height.clamp(1, rows.max(1));
    }
    rows.max(1)
}

pub(super) fn render_bottom_pane(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    render_activity_bar(f, app, chunks[0]);
    let cursor = render_composer(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);
    cursor
}

fn render_activity_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
    let (label, color) = if matches!(
        app.runtime_phase,
        super::super::state::RuntimePhase::RebuildingBackend
    ) {
        ("Downloading", Color::LightBlue)
    } else if app.is_busy() {
        ("Working", Color::Yellow)
    } else {
        ("Ready", Color::Green)
    };
    let detail = app
        .runtime_phase_detail
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| app.notice.as_deref().unwrap_or("waiting for input"));
    let animated_label = animated_activity_label(app, label);
    let mode_color = if app.agent_execution_mode_label() == "plan" {
        Color::LightBlue
    } else {
        Color::LightGreen
    };
    let status = Paragraph::new(Line::from(vec![
        Span::styled(
            animated_label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        badge("mode", app.agent_execution_mode_label(), mode_color),
        Span::raw("  "),
        Span::styled(app.runtime_phase_label(), Style::default().fg(Color::Gray)),
        Span::raw("  "),
        Span::styled(detail, Style::default().fg(Color::DarkGray)),
    ]));
    f.render_widget(status, area);
}

fn animated_activity_label(app: &TuiApp, label: &str) -> String {
    let Some(task) = app.running_task.as_ref() else {
        return label.to_string();
    };
    if !matches!(task.kind, TaskKind::Query | TaskKind::Rebuild) {
        return label.to_string();
    }

    let dots = match (task.started_at.elapsed().as_millis() / 450) % 3 {
        0 => ".",
        1 => "..",
        _ => "...",
    };
    format!("{label}{dots}")
}

fn render_composer(f: &mut Frame, app: &TuiApp, area: Rect) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(1)])
        .split(area);
    let composer_lines = if app.input.is_empty() {
        vec![Line::from(vec![
            Span::styled("› ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(
                "Ask about the repo, request a code change, or type ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "/help",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to browse commands.", Style::default().fg(Color::DarkGray)),
        ])]
    } else {
        app.input
            .lines()
            .map(|line| {
                Line::from(vec![
                    Span::styled("› ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(line.to_string()),
                ])
            })
            .collect::<Vec<_>>()
    };
    f.render_widget(
        Paragraph::new(composer_lines)
            .block(Block::default())
            .style(Style::default().bg(Color::Rgb(18, 20, 24)))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
    let hint = if app.input.trim_start().starts_with('/') {
        "slash command  Enter run  Esc close"
    } else if app.is_busy() {
        "busy  wait for the current task to finish"
    } else if app.has_pending_approval() {
        "approval pending  1 once  2 always  3 suggestion"
    } else if app.snapshot.pending_question.is_some() {
        "question pending  press 1/2/3 or type a reply"
    } else if app.agent_execution_mode_label() == "plan" {
        "plan mode  /plan return to execute"
    } else {
        "/search grep  /compact summarize history  /plan toggle  /quit exit"
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::Gray)))
            .alignment(Alignment::Left),
        chunks[1],
    );
    Some(composer_cursor_position(app.input.as_str(), chunks[0]))
}

fn render_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let context = match app.snapshot.context_window_tokens {
        Some(window) => format!("ctx~={}/{}", app.snapshot.estimated_history_tokens, window),
        None => format!("ctx~={}", app.snapshot.estimated_history_tokens),
    };
    let summary = format!(
        "key={}  history={}  local={}  tokens={} in / {} out  {}",
        api_key_status(&app.config),
        app.snapshot.history_len,
        app.transcript_entry_count(),
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        context,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            summary,
            Style::default().fg(Color::DarkGray),
        )))
        .alignment(Alignment::Right),
        area,
    );
}

fn composer_cursor_position(input: &str, area: Rect) -> (u16, u16) {
    wrapped_text_cursor_position(input, area, Some("› "), None)
}

pub(super) fn editor_cursor_position(input: &str, area: Rect) -> (u16, u16) {
    wrapped_text_cursor_position(input, inner_rect(area), None, None)
}

fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn wrapped_text_cursor_position(
    input: &str,
    area: Rect,
    initial_indent: Option<&str>,
    subsequent_indent: Option<&str>,
) -> (u16, u16) {
    if area.width == 0 || area.height == 0 {
        return (area.x, area.y);
    }

    let initial_indent = initial_indent.unwrap_or("");
    let subsequent_indent = subsequent_indent.unwrap_or("");
    let mut wrapped_rows: Vec<String> = Vec::new();

    if input.is_empty() {
        wrapped_rows.push(initial_indent.to_string());
    } else {
        for logical_line in input.split('\n') {
            let options = Options::new(area.width as usize)
                .initial_indent(initial_indent)
                .subsequent_indent(subsequent_indent)
                .break_words(false);
            let wraps = textwrap::wrap(logical_line, options);
            if wraps.is_empty() {
                wrapped_rows.push(initial_indent.to_string());
            } else {
                wrapped_rows.extend(wraps.into_iter().map(|line| line.into_owned()));
            }
        }
    }

    let last_row = wrapped_rows
        .last()
        .cloned()
        .unwrap_or_else(|| initial_indent.to_string());
    let row_index = wrapped_rows.len().saturating_sub(1);
    let cursor_y = area
        .y
        .saturating_add(row_index.min(area.height.saturating_sub(1) as usize) as u16);
    let display_width = UnicodeWidthStr::width(last_row.as_str()) as u16;
    let max_x_offset = area.width.saturating_sub(1);
    let cursor_x = area.x.saturating_add(display_width.min(max_x_offset));

    (cursor_x, cursor_y)
}
