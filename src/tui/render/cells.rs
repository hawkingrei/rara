use std::path::{Path, PathBuf};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::tui::line_utils::prefix_lines;

#[derive(Clone)]
pub(crate) enum CellSection {
    PrefixedMessage {
        role: String,
        message: String,
        max_lines: usize,
    },
    MarkdownMessage {
        role: String,
        message: String,
        max_lines: usize,
        cwd: Option<PathBuf>,
        active: bool,
    },
    RenderedMarkdown {
        role: String,
        rendered: Vec<Line<'static>>,
        max_lines: usize,
        active: bool,
    },
    Summary {
        title: String,
        color: Color,
        lines: Vec<String>,
    },
}

pub(crate) trait HistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;
}

pub(crate) trait ActiveCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;
    fn is_empty(&self) -> bool;
}

#[derive(Clone, Default)]
pub(crate) struct CommittedTurnCell {
    sections: Vec<CellSection>,
}

impl CommittedTurnCell {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, section: CellSection) {
        self.sections.push(section);
    }
}

#[derive(Clone, Default)]
pub(crate) struct ActiveTurnCell {
    sections: Vec<CellSection>,
}

impl ActiveTurnCell {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, section: CellSection) {
        self.sections.push(section);
    }
}

impl HistoryCell for CommittedTurnCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        flatten_sections(self.sections.iter().cloned())
    }
}

impl ActiveCell for ActiveTurnCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        flatten_sections(self.sections.iter().cloned())
    }

    fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

impl CellSection {
    pub(crate) fn lines(&self) -> Vec<Line<'static>> {
        match self {
            Self::PrefixedMessage {
                role,
                message,
                max_lines,
            } => prefixed_message_lines(role, message, *max_lines),
            Self::MarkdownMessage {
                role,
                message,
                max_lines,
                cwd,
                active,
            } => markdown_message_lines(role, message, *max_lines, cwd.as_deref(), *active),
            Self::RenderedMarkdown {
                role,
                rendered,
                max_lines,
                active,
            } => rendered_markdown_lines(role, rendered, *max_lines, *active),
            Self::Summary {
                title,
                color,
                lines,
            } => {
                let mut rendered = vec![active_heading(title, *color)];
                rendered.extend(lines.iter().map(|line| Line::from(format!("  {line}"))));
                rendered
            }
        }
    }
}

fn flatten_sections(sections: impl IntoIterator<Item = CellSection>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for section in sections {
        let mut section_lines = section.lines();
        if section_lines.is_empty() {
            continue;
        }
        lines.append(&mut section_lines);
        lines.push(Line::from(""));
    }
    while matches!(lines.last(), Some(line) if line.spans.iter().all(|span| span.content == "")) {
        lines.pop();
    }
    lines
}

pub(crate) fn active_heading(title: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled("•", Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(
            title.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
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

fn markdown_message_lines(
    role: &str,
    message: &str,
    max_lines: usize,
    cwd: Option<&Path>,
    active: bool,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    super::super::markdown::append_markdown(message, None, cwd, &mut rendered);
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
