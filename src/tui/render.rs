use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};
use std::path::Path;
use textwrap::Options;
use unicode_width::UnicodeWidthStr;

use super::command::{
    api_key_status, command_detail_text, command_spec_by_index, general_help_text, help_text,
    current_turn_preview, download_status_text, matching_commands, model_help_text, palette_command_by_index,
    palette_commands, quick_actions_text, recent_transcript_preview, status_prompt_sources_text,
    status_plan_text, status_request_user_input_text, status_resources_text, status_runtime_text, status_workspace_text,
};
use super::line_utils::prefix_lines;
use super::state::{
    current_model_presets, HelpTab, Overlay, PROVIDER_FAMILIES, TaskKind, TranscriptEntry, TuiApp,
};

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

pub fn desired_viewport_height(app: &TuiApp, _width: u16, rows: u16) -> u16 {
    if app.overlay.is_some() {
        return rows.max(1);
    }

    let bottom_pane_height = 5u16;
    let has_active_content = !app.active_turn.entries.is_empty();
    if !has_active_content {
        return bottom_pane_height.clamp(1, rows.max(1));
    }
    rows.max(1)
}

fn render_bottom_pane(f: &mut Frame, app: &TuiApp, area: Rect) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    render_activity_bar(f, app, chunks[0]);
    let cursor = render_composer(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);
    cursor
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
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.transcript_scroll as u16, 0)),
        area,
    );
}

pub fn committed_turn_lines(entries: &[TranscriptEntry], cwd: Option<&Path>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(user) = entries.iter().find(|entry| entry.role == "You") {
        lines.extend(prefixed_message_lines("You", &user.message, 4));
        lines.push(Line::from(""));
    }

    let entry_refs = entries.iter().collect::<Vec<_>>();
    let has_tool_activity = entry_refs.iter().any(|entry| {
        matches!(
            entry.role.as_str(),
            "Tool" | "Tool Result" | "Tool Error"
        )
    });
    if let Some(summary) = current_turn_exploration_summary_from_entries(entry_refs.as_slice(), false, None) {
        lines.push(Line::from(section_span("Explored", Color::Rgb(231, 201, 92))));
        lines.extend(summary.lines().map(|line| Line::from(format!("  {line}"))));
        lines.push(Line::from(""));
    }

    if let Some(summary) = current_turn_tool_summary(entry_refs.as_slice(), false, None) {
        lines.push(Line::from(section_span("Ran", Color::LightYellow)));
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
        ));
        lines.push(Line::from(""));
    }

    while matches!(lines.last(), Some(line) if line.spans.iter().all(|span| span.content == ""))
    {
        lines.pop();
    }
    lines
}

fn current_turn_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let current_turn = app.active_turn.entries.iter().collect::<Vec<_>>();
    if current_turn.is_empty() {
        return Vec::new();
    }
    let has_tool_activity = current_turn.iter().any(|entry| {
        matches!(
            entry.role.as_str(),
            "Tool" | "Tool Result" | "Tool Error"
        )
    });
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
        lines.push(Line::from(section_span("Plan Mode", Color::LightBlue)));
        lines.push(Line::from(""));
    }

    if let Some(summary) = current_turn_exploration_summary(app, current_turn.as_slice(), latest_agent.is_none()) {
        let (title, color) = if app.is_busy() && latest_agent.is_none() {
            ("Exploring", Color::Yellow)
        } else {
            ("Explored", Color::Rgb(231, 201, 92))
        };
        lines.push(Line::from(section_span(title, color)));
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
        lines.push(Line::from(section_span(title, color)));
        lines.extend(summary.lines().map(|line| Line::from(format!("  {line}"))));
        lines.push(Line::from(""));
    }

    if !app.snapshot.plan_steps.is_empty() {
        lines.push(Line::from(section_span("Plan", Color::LightBlue)));
        for line in status_plan_text(app).lines().take(8) {
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
        lines.push(Line::from(section_span(title, color)));
        for line in status_request_user_input_text(app).lines().take(8) {
            lines.push(Line::from(format!("  {line}")));
        }
        lines.push(Line::from("  shortcuts: press 1/2/3 to answer immediately"));
        lines.push(Line::from(""));
    }

    if let Some((title, summary)) = app.snapshot.completed_approval.as_ref() {
        lines.push(Line::from(section_span("Approval Completed", Color::LightGreen)));
        lines.push(Line::from(format!("  {}: {}", title, summary)));
        lines.push(Line::from(""));
    }

    if let Some((title, summary)) = app.snapshot.completed_question.as_ref() {
        lines.push(Line::from(section_span("Question Answered", Color::LightGreen)));
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
        lines.extend(rendered_markdown_lines("Agent", stream_lines, usize::MAX));
    } else if let Some(agent_message) = latest_agent.filter(|_| !suppress_intermediate_agent) {
        lines.extend(formatted_message_lines("Agent", agent_message, usize::MAX, cwd));
    } else if let Some(system_message) = latest_system {
        lines.extend(formatted_message_lines("System", system_message, 14, cwd));
    } else if let Some((role, tool_result)) = latest_tool_result {
        lines.extend(prefixed_message_lines(role, tool_result, 14));
    } else if app.is_busy() {
        lines.push(Line::from(section_span("Working", Color::Yellow)));
        lines.push(Line::from(format!(
            "  {}",
            app.runtime_phase_detail
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
        app.runtime_phase_detail.as_deref(),
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
) -> Vec<Line<'static>> {
    if matches!(role, "Agent" | "System") {
        return markdown_message_lines(role, message, max_lines, cwd);
    }
    prefixed_message_lines(role, message, max_lines)
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

fn rendered_markdown_lines(
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
    matches!(name, "list_files" | "read_file" | "glob" | "grep" | "search_files")
}

fn exploration_action_label(message: &str) -> Option<String> {
    let mut parts = message.split_whitespace();
    let name = parts.next()?;
    let rest = parts.collect::<Vec<_>>().join(" ");
    match name {
        "list_files" => Some(format!("List {}", if rest.is_empty() { "." } else { rest.as_str() })),
        "read_file" => Some(format!("Read {}", if rest.is_empty() { "file" } else { rest.as_str() })),
        "glob" => Some(format!("Glob {}", if rest.is_empty() { "workspace" } else { rest.as_str() })),
        "grep" => Some(format!("Search {}", if rest.is_empty() { "workspace" } else { rest.as_str() })),
        "search_files" => Some(format!("Search files {}", if rest.is_empty() { "workspace" } else { rest.as_str() })),
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
        "bash" => Some(format!("Run {}", if rest.is_empty() { "command" } else { rest.as_str() })),
        "apply_patch" => Some("Apply patch".to_string()),
        "write_file" => Some(format!("Write {}", if rest.is_empty() { "file" } else { rest.as_str() })),
        "replace" => Some(format!("Edit {}", if rest.is_empty() { "file" } else { rest.as_str() })),
        "web_fetch" => Some(format!("Fetch {}", if rest.is_empty() { "resource" } else { rest.as_str() })),
        other => Some(format!("Run {}", if rest.is_empty() { other } else { message })),
    }
}

fn render_activity_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
    let (label, color) = if matches!(app.runtime_phase, super::state::RuntimePhase::RebuildingBackend) {
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
    } else {
        "/search grep  /compact summarize history  /plan plan next turn  /quit exit"
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::Gray))).alignment(Alignment::Left),
        chunks[1],
    );
    Some(composer_cursor_position(app.input.as_str(), chunks[0]))
}

fn render_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let context = match app.snapshot.context_window_tokens {
        Some(window) => format!(
            "ctx~={}/{}",
            app.snapshot.estimated_history_tokens,
            window
        ),
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
        Paragraph::new(Line::from(Span::styled(summary, Style::default().fg(Color::DarkGray))))
            .alignment(Alignment::Right),
        area,
    );
}

fn render_overlay(f: &mut Frame, app: &TuiApp, overlay: Overlay) -> Option<(u16, u16)> {
    let popup = centered_rect(78, 70, f.area());
    f.render_widget(Clear, popup);
    match overlay {
        Overlay::Help(tab) => {
            render_help_modal(f, app, popup, tab);
            None
        }
        Overlay::CommandPalette => {
            render_command_palette(f, app, popup);
            None
        }
        Overlay::Status => {
            render_status_modal(f, app, popup);
            None
        }
        Overlay::Setup => {
            render_setup_modal(f, app, popup);
            None
        }
        Overlay::ProviderPicker => {
            render_provider_picker_modal(f, app, popup);
            None
        }
        Overlay::ResumePicker => {
            render_resume_picker_modal(f, app, popup);
            None
        }
        Overlay::ModelPicker => {
            render_model_picker_modal(f, app, popup);
            None
        }
        Overlay::BaseUrlEditor => render_base_url_editor_modal(f, app, popup),
        Overlay::CodexAuthGuide => {
            render_codex_auth_guide_modal(f, app, popup);
            None
        }
        Overlay::ApiKeyEditor => render_api_key_editor_modal(f, app, popup),
    }
}

fn render_help_modal(f: &mut Frame, app: &TuiApp, area: Rect, tab: HelpTab) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(10), Constraint::Length(2)])
        .split(area);
    let titles = ["General", "Commands", "Runtime"]
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    let selected = match tab {
        HelpTab::General => 0,
        HelpTab::Commands => 1,
        HelpTab::Runtime => 2,
    };
    f.render_widget(
        Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title(" Help "))
            .select(selected)
            .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        chunks[0],
    );
    match tab {
        HelpTab::General => {
            f.render_widget(
                Paragraph::new(general_help_text())
                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        HelpTab::Commands => {
            let inner = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
                .split(chunks[1]);
            let query = app.input.trim_start().trim_start_matches('/');
            let items = matching_commands(query)
                .into_iter()
                .enumerate()
                .map(|(idx, spec)| {
                    let style = if idx == app.command_palette_idx {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(format!("{}  {}", spec.usage, spec.summary)).style(style)
                })
                .collect::<Vec<_>>();
            let detail = command_spec_by_index(query, app.command_palette_idx)
                .map(command_detail_text)
                .unwrap_or_else(help_text);
            f.render_widget(
                List::new(items)
                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Commands ")),
                inner[0],
            );
            f.render_widget(
                Paragraph::new(detail)
                    .block(Block::default().borders(Borders::RIGHT).title(" Detail "))
                    .wrap(Wrap { trim: false }),
                inner[1],
            );
        }
        HelpTab::Runtime => {
            let inner = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            let left = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(8), Constraint::Length(6), Constraint::Min(5)])
                .split(inner[0]);
            let right = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(6), Constraint::Min(8)])
                .split(inner[1]);
            f.render_widget(
                Paragraph::new(status_runtime_text(app))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Runtime "))
                .wrap(Wrap { trim: false }),
                left[0],
            );
            f.render_widget(
                Paragraph::new(status_workspace_text(app))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Workspace "))
                .wrap(Wrap { trim: false }),
                left[1],
            );
            f.render_widget(
                Paragraph::new(status_prompt_sources_text(app))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Prompt Sources "))
                .wrap(Wrap { trim: false }),
                left[2],
            );
            f.render_widget(
                Paragraph::new(status_resources_text(app))
                .block(Block::default().borders(Borders::RIGHT).title(" Resources "))
                .wrap(Wrap { trim: false }),
                right[0],
            );
            f.render_widget(
                Paragraph::new(format!(
                    "{}\n\n{}",
                    model_help_text(app),
                    recent_transcript_preview(app, 4)
                ))
                .block(Block::default().borders(Borders::RIGHT).title(" Models / Recent "))
                .wrap(Wrap { trim: false }),
                right[1],
            );
        }
    }
    f.render_widget(
        Paragraph::new("Esc close  1 general  2 commands  3 runtime  Up/Down move in command lists")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_command_palette(f: &mut Frame, app: &TuiApp, area: Rect) {
    let query = app.input.trim_start().trim_start_matches('/');
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(2)])
        .split(area);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);
    let items = if query.is_empty() {
        palette_items_for_empty_query(app)
    } else {
        palette_items_for_matches(app, query)
    };
    let intro = if query.is_empty() {
        "Recommended and recent commands are listed first. Enter runs the highlighted command immediately."
    } else {
        "Use Up/Down to inspect matches. Enter runs the highlighted command immediately."
    };
    f.render_widget(
        Paragraph::new(intro)
            .block(Block::default().borders(Borders::ALL).title(format!(" Commands matching /{} ", query))),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Matches ")),
        body[0],
    );
    let detail = palette_command_by_index(app, query, app.command_palette_idx)
        .map(command_preview_text)
        .unwrap_or_else(help_text);
    f.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::RIGHT).title(" Detail "))
            .wrap(Wrap { trim: false }),
        body[1],
    );
    f.render_widget(
        Paragraph::new("Esc close  Enter run highlighted command  Keep typing to refine")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn palette_items_for_empty_query(app: &TuiApp) -> Vec<ListItem<'static>> {
    palette_commands(app, "")
        .into_iter()
        .enumerate()
        .map(|(idx, spec)| command_palette_item(idx, app.command_palette_idx, spec))
        .collect()
}

fn palette_items_for_matches(app: &TuiApp, query: &str) -> Vec<ListItem<'static>> {
    matching_commands(query)
        .into_iter()
        .enumerate()
        .scan(None::<&'static str>, |last_category, (idx, spec)| {
            let mut lines = Vec::new();
            if *last_category != Some(spec.category) {
                lines.push(Line::from(Span::styled(
                    format!("{} commands", spec.category),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                *last_category = Some(spec.category);
            }
            let style = if idx == app.command_palette_idx {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(format!("{}  {}", spec.usage, spec.summary)));
            lines.push(Line::from(""));
            Some(ListItem::new(lines).style(style))
        })
        .collect()
}

fn command_palette_item(
    index: usize,
    selected_index: usize,
    spec: &super::state::CommandSpec,
) -> ListItem<'static> {
    let style = if index == selected_index {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    ListItem::new(vec![
        Line::from(format!("{}  {}", spec.usage, spec.summary)),
        Line::from(""),
    ])
    .style(style)
}

fn render_status_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Length(6), Constraint::Length(8), Constraint::Length(8), Constraint::Min(6), Constraint::Length(2)])
        .split(area);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(33), Constraint::Percentage(33)])
        .split(chunks[0]);
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);
    f.render_widget(
        Paragraph::new(status_runtime_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Runtime "))
            .wrap(Wrap { trim: false }),
        top[0],
    );
    f.render_widget(
        Paragraph::new(status_workspace_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Workspace "))
            .wrap(Wrap { trim: false }),
        top[1],
    );
    f.render_widget(
        Paragraph::new(status_resources_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Resources "))
            .wrap(Wrap { trim: false }),
        top[2],
    );
    f.render_widget(
        Paragraph::new(status_prompt_sources_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Prompt Sources "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    let right_panel = download_status_text(app).unwrap_or_else(|| quick_actions_text().to_string());
    let right_title = if matches!(app.runtime_phase, super::state::RuntimePhase::RebuildingBackend | super::state::RuntimePhase::BackendReady) {
        " Download "
    } else {
        " Quick Actions "
    };
    f.render_widget(
        Paragraph::new(model_help_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Models "))
            .wrap(Wrap { trim: false }),
        middle[0],
    );
    f.render_widget(
        Paragraph::new(right_panel)
            .block(Block::default().borders(Borders::ALL).title(right_title))
            .wrap(Wrap { trim: false }),
        middle[1],
    );
    f.render_widget(
        Paragraph::new(status_plan_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Plan "))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);
    f.render_widget(
        Paragraph::new(status_request_user_input_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Request Input "))
            .wrap(Wrap { trim: false }),
        lower[0],
    );
    f.render_widget(
        Paragraph::new(current_turn_preview(app, 10))
            .block(Block::default().borders(Borders::ALL).title(" Current Turn "))
            .wrap(Wrap { trim: false }),
        lower[1],
    );
    f.render_widget(
        Paragraph::new(recent_transcript_preview(app, 8))
            .block(Block::default().borders(Borders::ALL).title(" Recent Activity "))
            .wrap(Wrap { trim: false }),
        chunks[4],
    );
    f.render_widget(
        Paragraph::new("Esc close  Enter close  /help commands  /model switch runtime")
            .alignment(Alignment::Center),
        chunks[5],
    );
}

fn render_setup_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let preset_lines = current_model_presets(app.provider_picker_idx)
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let marker =
                if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                    ">"
                } else {
                    " "
                };
            format!("{marker} [{}] {label} ({provider} / {model})", idx + 1)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        "Provider: {}\nModel: {}\nBase URL: {}\nAPI key: {}\nRevision: {}\n\n\
         Presets:\n{}\n\n\
         [1/2/3] Select preset\n[M] Cycle preset\n[Enter] Apply and rebuild\n[L] OAuth login\n[Esc] Close\n\n\
         Use /model for the full provider menu.\n\
         Codex supports both OAuth login and API key auth.\n\
         Recommended: Qwn3 8B for stable local use.",
        app.config.provider,
        app.current_model_label(),
        app.config.base_url.as_deref().unwrap_or("-"),
        api_key_status(&app.config),
        app.config.revision.as_deref().unwrap_or("main"),
        preset_lines,
    );
    f.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Setup "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_provider_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let items = PROVIDER_FAMILIES
        .iter()
        .enumerate()
        .map(|(idx, (_, label, detail))| {
            let style = if idx == app.provider_picker_idx {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(vec![
                Line::from(format!("[{}] {}", idx + 1, label)),
                Line::from(*detail),
                Line::from(""),
            ])
            .style(style)
        })
        .collect::<Vec<_>>();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(2)])
        .split(area);
    f.render_widget(
        Paragraph::new("Select a provider family first, then choose a concrete runtime or auth path.")
            .block(Block::default().borders(Borders::ALL).title(" Provider Menu ")),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1/2/3 select  Up/Down move  Enter continue  Esc close")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_resume_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(2)])
        .split(area);
    let intro = if app.recent_sessions.is_empty() {
        "No persisted sessions found yet."
    } else {
        "Choose a recent session to restore its transcript, plan state, and interaction cards."
    };
    f.render_widget(
        Paragraph::new(intro)
            .block(Block::default().borders(Borders::ALL).title(" Resume Session ")),
        chunks[0],
    );
    let items = if app.recent_sessions.is_empty() {
        vec![ListItem::new("No sessions available.")]
    } else {
        app.recent_sessions
            .iter()
            .enumerate()
            .map(|(idx, session)| {
                let style = if idx == app.resume_picker_idx {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let when = format!("updated_at={}", session.updated_at);
                let preview = if session.preview.is_empty() {
                    "(no preview)".to_string()
                } else {
                    session.preview.clone()
                };
                ListItem::new(vec![
                    Line::from(format!(
                        "{}  {} / {}  branch={}",
                        session.session_id, session.provider, session.model, session.branch
                    )),
                    Line::from(format!("  {when}")),
                    Line::from(format!("  {preview}")),
                    Line::from(""),
                ])
                .style(style)
            })
            .collect::<Vec<_>>()
    };
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("Esc close  Up/Down move  Enter restore")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_model_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let presets = current_model_presets(app.provider_picker_idx);
    let provider_label = PROVIDER_FAMILIES[app.provider_picker_idx].1;
    let items = presets
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let style = if idx == app.model_picker_idx {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let current =
                if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                    " current"
                } else {
                    ""
            };
            let recommendation = if provider_label == "Candle Local" && idx == 2 {
                " recommended"
            } else {
                ""
            };
            ListItem::new(format!(
                "[{}] {} ({}/{}){}{}",
                idx + 1,
                label,
                provider,
                model,
                current,
                recommendation,
            ))
            .style(style)
        })
        .collect::<Vec<_>>();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(2)])
        .split(area);
    let help = if provider_label == "Codex" && api_key_status(&app.config) == "missing" {
        "Provider: Codex\nAuthentication is required before this preset can be used.\nEnter opens the Codex login guide."
    } else {
        &format!(
            "Provider: {provider_label}\nBase URL: {}\nSelect a concrete model preset. Enter applies immediately.",
            app.config.base_url.as_deref().unwrap_or("http://localhost:11434"),
        )
    };
    f.render_widget(
        Paragraph::new(help)
            .block(Block::default().borders(Borders::ALL).title(" Model Picker ")),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(if provider_label == "Codex" {
            "1/2 choose  Up/Down move  Enter continue  Esc close"
        } else {
            "1/2/3 apply directly  Up/Down move  B edit base URL  Enter apply  Esc close"
        })
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_base_url_editor_modal(f: &mut Frame, app: &TuiApp, area: Rect) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(3), Constraint::Length(2)])
        .split(area);
    let intro = Paragraph::new(
        "Edit the Ollama base URL for this provider.\nLeave it empty to clear the override. Default: http://localhost:11434",
    )
    .block(Block::default().borders(Borders::ALL).title(" Base URL "));
    let editor = Paragraph::new(app.base_url_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer = Paragraph::new("Enter save  Esc back to model picker")
        .alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(app.base_url_input.as_str(), chunks[1]))
}

fn render_codex_auth_guide_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(6), Constraint::Length(2)])
        .split(area);
    let ssh_hint = if super::is_ssh_session() {
        "\n\nSSH session detected. Browser OAuth on a remote shell usually cannot complete the localhost callback. Use API key in SSH/headless sessions."
    } else {
        ""
    };
    let intro = format!(
        "Codex needs authentication before this preset can be used.\n\n\
         [1] OAuth login\n\
         [2] API key\n\n\
         OAuth matches the Codex desktop flow when this TUI is running locally.{ssh_hint}"
    );
    f.render_widget(
        Paragraph::new(intro)
            .block(Block::default().borders(Borders::ALL).title(" Codex Login "))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    let body = Paragraph::new(format!(
        "Current model: {}\nProvider: codex\nKey status: {}\n\n\
         Pick OAuth for local desktop login, or API key for headless / SSH usage.",
        app.current_model_label(),
        api_key_status(&app.config),
    ))
    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Details "))
    .wrap(Wrap { trim: false });
    f.render_widget(body, chunks[1]);

    f.render_widget(
        Paragraph::new("1 OAuth  2 API key  Enter OAuth  Esc back")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_api_key_editor_modal(f: &mut Frame, app: &TuiApp, area: Rect) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(3), Constraint::Length(2)])
        .split(area);
    let intro = Paragraph::new(
        "Paste a Codex-compatible API key. This is the recommended path for SSH/headless sessions.",
    )
    .block(Block::default().borders(Borders::ALL).title(" Codex API Key "))
    .wrap(Wrap { trim: false });
    let editor = Paragraph::new(app.api_key_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer = Paragraph::new("Enter save and rebuild  Esc back to login guide")
        .alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(app.api_key_input.as_str(), chunks[1]))
}

fn composer_cursor_position(input: &str, area: Rect) -> (u16, u16) {
    wrapped_text_cursor_position(input, area, Some("› "), None)
}

fn editor_cursor_position(input: &str, area: Rect) -> (u16, u16) {
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

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .flex(Flex::Center)
        .split(vertical[1]);
    horizontal[1]
}

fn command_preview_text(spec: &super::state::CommandSpec) -> String {
    format!("{}\n\n{}", spec.usage, spec.summary)
}

fn section_span<'a>(title: &'a str, color: Color) -> Span<'a> {
    Span::styled(
        format!(" {} ", title),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

fn badge<'a>(label: &'a str, value: &'a str, color: Color) -> Span<'a> {
    let fg = match color {
        Color::Black | Color::DarkGray | Color::Gray | Color::Blue | Color::Red | Color::Magenta => {
            Color::White
        }
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
