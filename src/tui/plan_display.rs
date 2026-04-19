use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::line_utils::prefix_lines;
use super::state::TuiApp;

#[derive(Clone, Copy)]
enum PlanStepKind {
    Completed,
    InProgress,
    Pending,
}

impl PlanStepKind {
    fn from_status(status: &str) -> Self {
        match status {
            "completed" => Self::Completed,
            "in_progress" => Self::InProgress,
            _ => Self::Pending,
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Completed => "✔ ",
            Self::InProgress | Self::Pending => "□ ",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::Completed => Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
            Self::InProgress => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Self::Pending => Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        }
    }
}

pub(crate) fn status_plan_text(app: &TuiApp) -> String {
    updated_plan_text(
        app.snapshot.plan_steps.as_slice(),
        app.snapshot.plan_explanation.as_deref(),
    )
}

pub(crate) fn updated_plan_text(
    steps: &[(String, String)],
    explanation: Option<&str>,
) -> String {
    if steps.is_empty() {
        return "No structured plan captured yet.".to_string();
    }

    let mut lines = vec!["Updated Plan".to_string()];

    if let Some(note) = explanation.map(str::trim).filter(|note| !note.is_empty()) {
        lines.push(format!("  note: {note}"));
    }

    for (status, step) in steps {
        let kind = PlanStepKind::from_status(status);
        lines.push(format!("  {}{}", kind.marker(), step));
    }

    lines.join("\n")
}

pub(crate) fn updated_plan_lines(
    steps: &[(String, String)],
    explanation: Option<&str>,
) -> Vec<Line<'static>> {
    if steps.is_empty() {
        return vec![Line::from("No structured plan captured yet.")];
    }

    let mut lines = vec![Line::from(vec![
        Span::styled("• ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Updated Plan",
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    let mut detail_lines = Vec::new();

    if let Some(note) = explanation.map(str::trim).filter(|note| !note.is_empty()) {
        detail_lines.push(Line::from(Span::styled(
            note.to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM | Modifier::ITALIC),
        )));
    }

    for (status, step) in steps {
        let kind = PlanStepKind::from_status(status);
        detail_lines.push(Line::from(vec![
            Span::styled(kind.marker(), kind.style()),
            Span::styled(step.to_string(), kind.style()),
        ]));
    }

    lines.extend(prefix_lines(
        detail_lines,
        Span::styled("  └ ", Style::default().fg(Color::DarkGray)),
        Span::raw("    "),
    ));

    lines
}

#[cfg(test)]
mod tests {
    use super::{updated_plan_lines, updated_plan_text};

    #[test]
    fn updated_plan_text_formats_note_and_checklist() {
        let rendered = updated_plan_text(
            &[
                ("completed".into(), "Inspect the current workflow".into()),
                ("in_progress".into(), "Generalize instruction discovery".into()),
                ("pending".into(), "Validate restore behavior".into()),
            ],
            Some("Keep the explanation short and decision-complete."),
        );

        assert!(rendered.contains("Updated Plan"));
        assert!(rendered.contains("note: Keep the explanation short and decision-complete."));
        assert!(rendered.contains("✔ Inspect the current workflow"));
        assert!(rendered.contains("□ Generalize instruction discovery"));
        assert!(rendered.contains("□ Validate restore behavior"));
    }

    #[test]
    fn updated_plan_lines_render_title_and_steps() {
        let rendered = updated_plan_lines(
            &[("pending".into(), "Capture the next implementation step".into())],
            Some("Do not claim execution in planning mode."),
        )
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

        assert!(rendered.contains("Updated Plan"));
        assert!(rendered.contains("Do not claim execution in planning mode."));
        assert!(rendered.contains("□ Capture the next implementation step"));
    }
}
