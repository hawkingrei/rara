use std::path::{Path, PathBuf};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::tui::line_utils::prefix_lines;

pub(crate) trait HistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;
}

pub(crate) trait ActiveCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;
    fn is_empty(&self) -> bool;
}

pub(crate) trait SectionCell {
    fn lines(&self) -> Vec<Line<'static>>;
}

pub(crate) struct PrefixedMessageCell {
    role: String,
    message: String,
    max_lines: usize,
}

impl PrefixedMessageCell {
    pub(crate) fn new(role: String, message: String, max_lines: usize) -> Self {
        Self {
            role,
            message,
            max_lines,
        }
    }
}

impl SectionCell for PrefixedMessageCell {
    fn lines(&self) -> Vec<Line<'static>> {
        prefixed_message_lines(self.role.as_str(), self.message.as_str(), self.max_lines)
    }
}

pub(crate) struct MarkdownMessageCell {
    role: String,
    message: String,
    max_lines: usize,
    cwd: Option<PathBuf>,
    active: bool,
}

impl MarkdownMessageCell {
    pub(crate) fn new(
        role: String,
        message: String,
        max_lines: usize,
        cwd: Option<PathBuf>,
        active: bool,
    ) -> Self {
        Self {
            role,
            message,
            max_lines,
            cwd,
            active,
        }
    }
}

impl SectionCell for MarkdownMessageCell {
    fn lines(&self) -> Vec<Line<'static>> {
        markdown_message_lines(
            self.role.as_str(),
            self.message.as_str(),
            self.max_lines,
            self.cwd.as_deref(),
            self.active,
        )
    }
}

pub(crate) struct RenderedMarkdownCell {
    role: String,
    rendered: Vec<Line<'static>>,
    max_lines: usize,
    active: bool,
}

impl RenderedMarkdownCell {
    pub(crate) fn new(
        role: String,
        rendered: Vec<Line<'static>>,
        max_lines: usize,
        active: bool,
    ) -> Self {
        Self {
            role,
            rendered,
            max_lines,
            active,
        }
    }
}

impl SectionCell for RenderedMarkdownCell {
    fn lines(&self) -> Vec<Line<'static>> {
        rendered_markdown_lines(
            self.role.as_str(),
            self.rendered.as_slice(),
            self.max_lines,
            self.active,
        )
    }
}

pub(crate) struct SummaryCell {
    title: String,
    color: Color,
    lines: Vec<String>,
}

impl SummaryCell {
    pub(crate) fn new(title: String, color: Color, lines: Vec<String>) -> Self {
        Self { title, color, lines }
    }
}

impl SectionCell for SummaryCell {
    fn lines(&self) -> Vec<Line<'static>> {
        let mut rendered = vec![active_heading(self.title.as_str(), self.color)];
        rendered.extend(self.lines.iter().map(|line| Line::from(format!("  {line}"))));
        rendered
    }
}

#[derive(Default)]
pub(crate) struct CommittedTurnCell {
    sections: Vec<Box<dyn SectionCell>>,
}

impl CommittedTurnCell {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push<S: SectionCell + 'static>(&mut self, section: S) {
        self.sections.push(Box::new(section));
    }
}

#[derive(Default)]
pub(crate) struct ActiveTurnCell {
    sections: Vec<Box<dyn SectionCell>>,
}

impl ActiveTurnCell {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push<S: SectionCell + 'static>(&mut self, section: S) {
        self.sections.push(Box::new(section));
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StartupCardCell {
    title: String,
    model: String,
    directory: String,
}

impl StartupCardCell {
    pub(crate) fn new(title: String, model: String, directory: String) -> Self {
        Self {
            title,
            model,
            directory,
        }
    }
}

impl HistoryCell for CommittedTurnCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        flatten_sections(self.sections.iter().map(|section| section.as_ref()))
    }
}

impl ActiveCell for ActiveTurnCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        flatten_sections(self.sections.iter().map(|section| section.as_ref()))
    }

    fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

impl HistoryCell for StartupCardCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
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
        let model_value = truncate_for_startup_card(self.model.as_str(), model_available_width);
        let model_value_width = display_width(&model_value);
        let gap_width = inner_width
            .saturating_sub(model_prefix_width)
            .saturating_sub(model_value_width)
            .saturating_sub(hint_width)
            .max(1);
        let directory_prefix = format!("{directory_label:<label_width$} ");
        let directory_max_width = inner_width.saturating_sub(display_width(&directory_prefix));

        let lines = vec![
            Line::from(vec![Span::from(">_ "), Span::from(self.title.clone())]),
            Line::from(""),
            Line::from(vec![
                Span::from(model_prefix),
                Span::from(model_value),
                Span::from(" ".repeat(gap_width)),
                Span::from(hint),
            ]),
            Line::from(vec![
                Span::from(directory_prefix),
                Span::from(truncate_path_middle(self.directory.as_str(), directory_max_width)),
            ]),
        ];

        with_border(lines, inner_width)
    }
}

fn flatten_sections<'a>(sections: impl IntoIterator<Item = &'a dyn SectionCell>) -> Vec<Line<'static>> {
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

#[cfg(test)]
mod tests {
    use super::{HistoryCell as _, StartupCardCell};

    #[test]
    fn startup_card_cell_renders_model_and_directory() {
        let lines = StartupCardCell::new(
            "RARA".to_string(),
            "gemma4".to_string(),
            "~/devel/opensource/rara".to_string(),
        )
        .display_lines(60);

        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(">_ RARA"));
        assert!(rendered.contains("model:"));
        assert!(rendered.contains("gemma4"));
        assert!(rendered.contains("directory:"));
        assert!(rendered.contains("rara"));
    }
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

fn startup_card_inner_width(width: u16) -> Option<usize> {
    if width < 8 {
        return None;
    }
    Some(std::cmp::min(width.saturating_sub(4) as usize, 56))
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
    value.chars().map(unicode_width::UnicodeWidthChar::width).sum::<Option<usize>>().unwrap_or(0)
}
