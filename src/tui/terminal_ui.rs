use std::io;

use anyhow::Result;
use crossterm::{cursor::Show, execute, terminal::disable_raw_mode, terminal::size as terminal_size};
use ratatui::{
    backend::CrosstermBackend,
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
    Terminal, TerminalOptions, Viewport,
};
use unicode_width::UnicodeWidthStr;

use super::render::committed_turn_lines;
use super::state::TuiApp;

pub(super) fn handle_paste(text: String, app: &mut TuiApp) {
    if matches!(app.overlay, Some(super::state::Overlay::BaseUrlEditor)) {
        app.base_url_input.push_str(&text);
        return;
    }

    if matches!(app.overlay, Some(super::state::Overlay::ApiKeyEditor)) {
        app.api_key_input.push_str(&text);
        return;
    }

    app.input.push_str(&text);
    app.sync_command_palette_with_input();
}

pub(super) fn build_terminal(
    viewport_height: u16,
) -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    match Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height.max(1)),
        },
    ) {
        Ok(terminal) => Ok(terminal),
        Err(inline_err) => {
            let terminal = Terminal::new(CrosstermBackend::new(io::stdout())).map_err(
                |fallback_err| {
                    anyhow::anyhow!(
                        "failed to build inline terminal: {inline_err}; fullscreen fallback also failed: {fallback_err}"
                    )
                },
            )?;
            Ok(terminal)
        }
    }
}

pub(super) fn teardown_terminal(
    mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), Show)?;
    terminal.show_cursor()?;
    Ok(())
}

pub(super) fn flush_committed_history(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut TuiApp,
) -> Result<()> {
    if !app.startup_card_inserted {
        let width = terminal_size()?.0;
        let lines = startup_card_lines(app, width);
        if !lines.is_empty() {
            let line_count = wrapped_history_line_count(lines.as_slice(), width);
            terminal.insert_before(line_count, |buf| {
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .render(buf.area, buf);
            })?;
        }
        app.startup_card_inserted = true;
    }
    while app.inserted_turns < app.committed_turns.len() {
        let turn = &app.committed_turns[app.inserted_turns];
        let cwd = (!app.snapshot.cwd.is_empty()).then(|| std::path::Path::new(app.snapshot.cwd.as_str()));
        let mut lines = committed_turn_lines(turn.entries.as_slice(), cwd);
        if app.inserted_turns > 0 && !lines.is_empty() {
            lines.insert(0, Line::from(""));
        }
        if !lines.is_empty() {
            let width = terminal_size()?.0;
            let line_count = wrapped_history_line_count(lines.as_slice(), width);
            terminal.insert_before(line_count, |buf| {
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .render(buf.area, buf);
            })?;
        }
        app.inserted_turns += 1;
    }
    Ok(())
}

fn wrapped_history_line_count(lines: &[Line<'static>], width: u16) -> u16 {
    let wrap_width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(wrap_width))
        .sum::<usize>()
        .max(1) as u16
}

fn startup_card_lines(app: &TuiApp, width: u16) -> Vec<Line<'static>> {
    let Some(inner_width) = startup_card_inner_width(width) else {
        return Vec::new();
    };

    let model_label = "model:";
    let directory_label = "directory:";
    let label_width = directory_label.len();
    let model_prefix = format!("{model_label:<label_width$} ");
    let hint = "/model to change";
    let hint_width = display_width(hint);
    let model_prefix_width = display_width(&model_prefix);
    let model_available_width = inner_width
        .saturating_sub(model_prefix_width)
        .saturating_sub(1)
        .saturating_sub(hint_width);
    let model_value = truncate_for_startup_card(app.current_model_label(), model_available_width);
    let model_value_width = display_width(&model_value);
    let gap_width = inner_width
        .saturating_sub(model_prefix_width)
        .saturating_sub(model_value_width)
        .saturating_sub(hint_width)
        .max(1);
    let directory_prefix = format!("{directory_label:<label_width$} ");
    let directory_max_width = inner_width.saturating_sub(display_width(&directory_prefix));

    let lines = vec![
        Line::from(vec![Span::from(">_ "), Span::from("RARA")]),
        Line::from(""),
        Line::from(vec![
            Span::from(model_prefix),
            Span::from(model_value),
            Span::from(" ".repeat(gap_width)),
            Span::from(hint),
        ]),
        Line::from(vec![
            Span::from(directory_prefix),
            Span::from(truncate_path_middle(
                &display_directory_for_startup(app),
                directory_max_width,
            )),
        ]),
    ];

    with_border(lines, inner_width)
}

fn display_directory_for_startup(app: &TuiApp) -> String {
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

fn truncate_for_startup_card(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let kept = value.chars().take(width - 1).collect::<String>();
    format!("{kept}…")
}

fn truncate_path_middle(value: &str, width: usize) -> String {
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

fn startup_card_inner_width(width: u16) -> Option<usize> {
    if width < 8 {
        return None;
    }
    Some(std::cmp::min(width.saturating_sub(4) as usize, 56))
}

fn with_border(lines: Vec<Line<'static>>, inner_width: usize) -> Vec<Line<'static>> {
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

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

pub(crate) fn is_ssh_session() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
}
