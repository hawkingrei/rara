use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use textwrap::Options;
use unicode_width::UnicodeWidthStr;

use super::super::custom_terminal::Frame;
use super::super::interaction_text::pending_interaction_hint_text;
use super::super::queued_input::{
    pending_follow_up_heading, pending_follow_up_hint, queued_follow_up_heading,
    queued_follow_up_hint,
};
use super::super::state::{ActivePendingInteractionKind, TaskKind, TuiApp};
use super::badge;

pub(crate) fn desired_viewport_height(app: &TuiApp, _width: u16, rows: u16) -> u16 {
    if app.overlay.is_some() || app.transcript_scroll > 0 {
        return rows.max(1);
    }

    let bottom_pane_height = 5u16;
    let has_active_content =
        !app.active_turn.entries.is_empty() || app.has_pending_planning_suggestion();
    if !app.has_any_transcript() && !has_active_content {
        return bottom_pane_height.clamp(1, rows.max(1));
    }

    let history_reserve = if rows >= 18 {
        6
    } else if rows >= 12 {
        4
    } else {
        2
    };

    rows.saturating_sub(history_reserve)
        .max(bottom_pane_height)
        .max(1)
}

pub(super) fn render_bottom_pane(f: &mut Frame, app: &TuiApp, area: Rect) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    render_activity_bar(f, app, chunks[0]);
    let cursor = render_composer(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);
    cursor
}

fn render_activity_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
    let (label, color, detail) = activity_status_line(app);
    let mut spans = vec![Span::styled(
        animated_activity_label(app, label),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )];
    let label_already_reflects_planning = matches!(
        app.active_pending_interaction().map(|item| item.kind),
        Some(
            ActivePendingInteractionKind::PlanApproval
                | ActivePendingInteractionKind::PlanningQuestion
        )
    ) || matches!(label, "Planning");

    if app.agent_execution_mode_label() == "plan" && !label_already_reflects_planning {
        spans.push(Span::raw("  "));
        spans.push(badge("mode", "plan", Color::Cyan));
    }
    if !detail.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(detail, Style::default().fg(Color::DarkGray)));
    }
    let status = Paragraph::new(Line::from(spans));
    f.render_widget(status, area);
}

fn activity_status_line(app: &TuiApp) -> (&'static str, Color, String) {
    if matches!(
        app.runtime_phase,
        super::super::state::RuntimePhase::RebuildingBackend
    ) {
        return (
            "Downloading",
            Color::LightBlue,
            app.runtime_phase_detail
                .as_deref()
                .unwrap_or("preparing backend")
                .to_string(),
        );
    }

    if let Some(pending) = app.active_pending_interaction() {
        let (label, color) = match pending.kind {
            ActivePendingInteractionKind::PlanApproval => ("Plan Approval", Color::Cyan),
            ActivePendingInteractionKind::ShellApproval => ("Shell Approval", Color::Yellow),
            ActivePendingInteractionKind::PlanningQuestion => ("Planning Question", Color::Cyan),
            ActivePendingInteractionKind::ExplorationQuestion => {
                ("Exploration Question", Color::Yellow)
            }
            ActivePendingInteractionKind::SubAgentQuestion => {
                ("Sub-agent Question", Color::LightGreen)
            }
            ActivePendingInteractionKind::RequestInput => ("Request Input", Color::LightGreen),
        };
        let detail = match pending.kind {
            ActivePendingInteractionKind::PlanApproval => {
                "choose whether to start implementation or continue planning".to_string()
            }
            ActivePendingInteractionKind::ShellApproval => app
                .pending_command_approval()
                .map(|interaction| interaction.summary.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "review the pending shell command".to_string()),
            ActivePendingInteractionKind::PlanningQuestion
            | ActivePendingInteractionKind::ExplorationQuestion
            | ActivePendingInteractionKind::SubAgentQuestion
            | ActivePendingInteractionKind::RequestInput => app
                .pending_request_input()
                .map(|interaction| interaction.title.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "answer the pending question".to_string()),
        };
        return (label, color, detail);
    }

    if app.has_pending_planning_suggestion() {
        return (
            "Planning Suggested",
            Color::Cyan,
            "enter planning mode first or continue in execute mode".to_string(),
        );
    }

    if app.is_busy() {
        let mut detail = app
            .runtime_phase_detail
            .as_deref()
            .unwrap_or("waiting for model response")
            .to_string();
        if app.has_queued_follow_up_messages() {
            detail.push_str(&format!(
                " · {} queued follow-up",
                app.queued_follow_up_count()
            ));
        }
        return ("Working", Color::Yellow, detail);
    }

    if app.agent_execution_mode_label() == "plan" {
        return (
            "Planning",
            Color::Cyan,
            "analyze, refine, or finalize a plan".to_string(),
        );
    }

    (
        "Ready",
        Color::Green,
        app.notice
            .as_deref()
            .unwrap_or("waiting for input")
            .to_string(),
    )
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
    let composer_lines = if app.input.is_empty() && app.has_queued_follow_up_messages() {
        queued_follow_up_preview_lines(app)
    } else if app.input.is_empty() {
        vec![Line::from(vec![
            Span::styled(
                "› ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
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
                    Span::styled(
                        "› ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
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
    let hint = composer_hint_line(app);
    f.render_widget(Paragraph::new(hint).alignment(Alignment::Left), chunks[1]);
    Some(composer_cursor_position(app.input.as_str(), chunks[0]))
}

fn queued_follow_up_preview_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if let Some(preview) = app
        .pending_follow_up_preview()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(Line::from(vec![
            Span::styled(
                "› ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                pending_follow_up_heading(),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::raw(preview.to_string()),
        ]));
        let remaining = app.pending_follow_up_count().saturating_sub(1);
        if remaining > 0 {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("... {remaining} more pending"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    if let Some(preview) = app
        .queued_end_of_turn_preview()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled(
                "› ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                queued_follow_up_heading(),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::raw(preview.to_string()),
        ]));
        let remaining = app.queued_follow_up_messages.len().saturating_sub(1);
        if remaining > 0 {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("... {remaining} more queued"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    lines
}

fn composer_hint(app: &TuiApp) -> &'static str {
    if matches!(
        app.overlay,
        Some(super::super::state::Overlay::CommandPalette)
    ) {
        ""
    } else if app.input.trim_start().starts_with('/') {
        "slash command  Enter run  Esc close"
    } else if app.has_pending_follow_up_messages() {
        pending_follow_up_hint()
    } else if app.has_queued_follow_up_messages() {
        queued_follow_up_hint()
    } else if app.is_busy() {
        "busy  wait for the current task to finish"
    } else if app.has_pending_planning_suggestion() {
        "planning suggested  1 enter planning mode  2 continue in execute mode"
    } else if let Some(pending) = app.active_pending_interaction() {
        pending_interaction_hint_text(pending.kind)
    } else if app.agent_execution_mode_label() == "plan" {
        "planning mode  analyze, refine, or finalize a plan"
    } else {
        "/compact summarize history  /plan enter planning mode  /quit exit"
    }
}

fn composer_hint_line(app: &TuiApp) -> Line<'static> {
    let hint = composer_hint(app);
    let mut spans = Vec::new();
    if !hint.is_empty() {
        spans.push(Span::styled(hint, Style::default().fg(Color::Gray)));
    }
    if let Some(repo_context) = app.repo_context_hint() {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            repo_context,
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn render_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let summary = footer_summary_text(app);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            summary,
            Style::default().fg(Color::DarkGray),
        )))
        .alignment(Alignment::Right),
        area,
    );
}

fn footer_summary_text(app: &TuiApp) -> String {
    let context = match app.snapshot.context_window_tokens {
        Some(window) => format!("ctx~={}/{}", app.snapshot.estimated_history_tokens, window),
        None => format!("ctx~={}", app.snapshot.estimated_history_tokens),
    };
    if shows_live_task_stats(app) {
        format!(
            "{}  tokens={} in / {} out",
            context, app.snapshot.total_input_tokens, app.snapshot.total_output_tokens
        )
    } else if app.snapshot.compaction_count > 0 {
        format!("{}  compactions={}", context, app.snapshot.compaction_count)
    } else {
        context
    }
}

fn shows_live_task_stats(app: &TuiApp) -> bool {
    app.is_busy()
        || matches!(
            app.runtime_phase,
            super::super::state::RuntimePhase::SendingPrompt
                | super::super::state::RuntimePhase::ProcessingResponse
                | super::super::state::RuntimePhase::RunningTool
        )
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

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use tempfile::tempdir;

    use crate::config::ConfigManager;
    use crate::tui::state::{
        InteractionKind, PendingInteractionSnapshot, RuntimePhase, RuntimeSnapshot, TuiApp,
    };

    use super::{
        activity_status_line, composer_hint, composer_hint_line, footer_summary_text,
        queued_follow_up_preview_lines,
    };

    #[test]
    fn footer_summary_text_prefers_minimal_idle_context() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.snapshot = RuntimeSnapshot {
            estimated_history_tokens: 1234,
            context_window_tokens: Some(32768),
            ..RuntimeSnapshot::default()
        };

        let rendered = footer_summary_text(&app);
        assert_eq!(rendered, "ctx~=1234/32768");
    }

    #[test]
    fn footer_summary_text_shows_tokens_only_while_busy() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::ProcessingResponse;
        app.snapshot = RuntimeSnapshot {
            estimated_history_tokens: 2048,
            context_window_tokens: Some(32768),
            total_input_tokens: 111,
            total_output_tokens: 22,
            ..RuntimeSnapshot::default()
        };

        let rendered = footer_summary_text(&app);
        assert_eq!(rendered, "ctx~=2048/32768  tokens=111 in / 22 out");
        assert!(!rendered.contains("history="));
        assert!(!rendered.contains("local="));
        assert!(!rendered.contains("key="));
    }

    #[test]
    fn activity_status_line_prefers_pending_interactions() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.snapshot
            .pending_interactions
            .push(PendingInteractionSnapshot {
                kind: InteractionKind::PlanApproval,
                title: "Approve plan".into(),
                summary: "ready".into(),
                options: Vec::new(),
                note: None,
                approval: None,
                source: None,
            });

        let (label, _, detail) = activity_status_line(&app);
        assert_eq!(label, "Plan Approval");
        assert!(detail.contains("start implementation"));
        assert!(detail.contains("continue planning"));
    }

    #[test]
    fn queued_follow_up_preview_shows_first_message_and_remainder() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.begin_running_turn();
        app.queue_follow_up_message_after_next_tool_boundary("first follow-up");
        app.queue_follow_up_message("second follow-up");

        let rendered = queued_follow_up_preview_lines(&app)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Messages to be submitted after next tool call"));
        assert!(rendered.contains("Queued follow-up messages"));
        assert!(rendered.contains("first follow-up"));
        assert!(rendered.contains("second follow-up"));
    }

    #[test]
    fn queued_follow_up_preview_snapshot() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.begin_running_turn();
        app.queue_follow_up_message_after_next_tool_boundary(
            "apply the feedback to the auth picker and rerun focused tests",
        );
        app.queue_follow_up_message("then summarize the remaining TODOs");

        let rendered = queued_follow_up_preview_lines(&app)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert_snapshot!("queued_follow_up_preview", rendered);
    }

    #[test]
    fn queued_follow_up_hint_overrides_busy_hint() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::ProcessingResponse;
        app.begin_running_turn();
        app.queue_follow_up_message_after_next_tool_boundary("follow-up");

        assert_eq!(
            composer_hint(&app),
            "pending follow-up  will submit after next tool call"
        );
    }

    #[test]
    fn composer_hint_line_includes_repo_context_when_available() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.repo_slug = Some("hawkingrei/rara".into());
        app.current_pr_url = Some("https://github.com/hawkingrei/rara/pull/46".into());
        app.snapshot.branch = "feat/test".into();

        let rendered = composer_hint_line(&app).to_string();
        assert!(rendered.contains("repo: hawkingrei/rara"));
        assert!(rendered.contains("branch: feat/test"));
        assert!(rendered.contains("PR: https://github.com/hawkingrei/rara/pull/46"));
    }

    #[test]
    fn composer_hint_line_hides_slash_hint_while_palette_is_open() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.input = "/".into();
        app.overlay = Some(crate::tui::state::Overlay::CommandPalette);
        app.repo_slug = Some("hawkingrei/rara".into());
        app.snapshot.branch = "main".into();

        let rendered = composer_hint_line(&app).to_string();
        assert!(!rendered.contains("slash command"));
        assert!(rendered.contains("repo: hawkingrei/rara"));
        assert!(rendered.contains("branch: main"));
    }
}
