use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};

use super::command::{
    api_key_status, command_detail_text, command_spec_by_index, general_help_text, help_text,
    current_turn_preview, download_status_text, matching_commands, model_help_text, palette_command_by_index,
    palette_commands, quick_actions_text, recent_transcript_preview, status_prompt_sources_text,
    status_plan_text, status_request_user_input_text, status_resources_text, status_runtime_text, status_workspace_text,
};
use super::state::{
    current_model_presets, HelpTab, Overlay, PROVIDER_FAMILIES, TaskKind, TranscriptEntry, TuiApp,
};

pub fn render(f: &mut Frame, app: &TuiApp) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(6),
        ])
        .split(f.area());

    render_header(f, app, layout[0]);
    render_transcript(f, app, layout[1]);
    render_bottom_pane(f, app, layout[2]);

    if let Some(overlay) = app.overlay {
        render_overlay(f, app, overlay);
    }
}

fn render_bottom_pane(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(4), Constraint::Length(1)])
        .split(area);
    render_activity_bar(f, app, chunks[0]);
    render_composer(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);
}

fn render_transcript(f: &mut Frame, app: &TuiApp, area: Rect) {
    if !app.has_any_transcript() {
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

    let mut lines = vec![Line::from(section_span("Current Turn", Color::LightBlue)), Line::from("")];
    lines.extend(current_turn_lines(app));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

pub fn committed_turn_lines(entries: &[TranscriptEntry]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| matches!(entry.role.as_str(), "You" | "Agent" | "System"))
    {
        lines.push(Line::from(role_badge_span(&entry.role)));
        let max_lines = if entry.role == "Agent" { 8 } else { 4 };
        let message_lines = entry.message.lines().collect::<Vec<_>>();
        if message_lines.is_empty() {
            lines.push(Line::from("  "));
        } else {
            for line in message_lines.iter().take(max_lines) {
                lines.push(Line::from(format!("  {line}")));
            }
            if message_lines.len() > max_lines {
                lines.push(Line::from(Span::styled(
                    format!("  ... {} more line(s)", message_lines.len() - max_lines),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
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
        return vec![
            Line::from(section_span("Ready", Color::Green)),
            Line::from("  No active turn."),
        ];
    }
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

    if !user_message.is_empty() {
        lines.push(Line::from(role_badge_span("You")));
        lines.push(Line::from(format!("  {}", user_message)));
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

    if let Some(summary) = current_turn_tool_summary(current_turn.as_slice()) {
        lines.push(Line::from(section_span("Actions", Color::LightYellow)));
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

    if let Some(agent_message) = latest_agent {
        lines.push(Line::from(role_badge_span("Agent")));
        for line in agent_message.lines() {
            lines.push(Line::from(format!("  {line}")));
        }
    } else if let Some(system_message) = latest_system {
        lines.push(Line::from(role_badge_span("System")));
        for line in system_message.lines().take(14) {
            lines.push(Line::from(format!("  {line}")));
        }
    } else if let Some((role, tool_result)) = latest_tool_result {
        lines.push(Line::from(role_badge_span(role)));
        for line in tool_result.lines().take(14) {
            lines.push(Line::from(format!("  {line}")));
        }
    } else if app.is_busy() {
        lines.push(Line::from(section_span("Working", Color::Yellow)));
        lines.push(Line::from(format!(
            "  {}",
            app.runtime_phase_detail
                .as_deref()
                .unwrap_or("waiting for the current turn to finish")
        )));
    } else {
        lines.push(Line::from(section_span("Ready", Color::Green)));
        lines.push(Line::from("  No final answer yet."));
    }

    lines
}

fn current_turn_exploration_summary(
    app: &TuiApp,
    current_turn: &[&TranscriptEntry],
    prefer_live_label: bool,
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

    if app.is_busy() && prefer_live_label {
        lines.push(format!(
            "└ {}",
            app.runtime_phase_detail
                .as_deref()
                .unwrap_or("waiting for more exploration output")
        ));
    }

    Some(lines.join("\n"))
}

fn current_turn_tool_summary(current_turn: &[&TranscriptEntry]) -> Option<String> {
    let tools = current_turn
        .iter()
        .filter_map(|entry| {
            if entry.role != "Tool" {
                return None;
            }
            let name = entry.message.split_whitespace().next()?;
            if is_exploration_tool(name) {
                return None;
            }
            Some(name.to_string())
        })
        .collect::<Vec<_>>();
    if tools.is_empty() {
        return None;
    }

    let mut parts = vec![format!("Used {} tool call(s): {}", tools.len(), tools.join(", "))];
    let results = current_turn
        .iter()
        .filter(|entry| entry.role == "Tool Result" || entry.role == "Tool Error")
        .count();
    if results > 0 {
        parts.push(format!("Recorded {} tool result(s).", results));
    }
    Some(parts.join("\n"))
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

fn render_composer(f: &mut Frame, app: &TuiApp, area: Rect) {
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
        "/search grep  terminal scrollback for history  /plan toggle  /quit exit"
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::Gray))).alignment(Alignment::Left),
        chunks[1],
    );
}

fn render_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let summary = format!(
        "key={}  history={}  local={}  tokens={} in / {} out",
        api_key_status(&app.config),
        app.snapshot.history_len,
        app.transcript_entry_count(),
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(summary, Style::default().fg(Color::DarkGray))))
            .alignment(Alignment::Right),
        area,
    );
}

fn render_header(f: &mut Frame, app: &TuiApp, area: Rect) {
    let provider_color = if super::provider_requires_api_key(&app.config.provider) {
        Color::Magenta
    } else {
        Color::LightBlue
    };
    let key_status = api_key_status(&app.config);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " RARA ",
                Style::default()
                    .bg(Color::LightBlue)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            badge("provider", &app.config.provider, provider_color),
        ]),
        Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                format!(
                    "model={}  branch={}  key={}",
                    app.current_model_label(),
                    app.snapshot.branch,
                    key_status,
                ),
                Style::default().fg(Color::Gray),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_overlay(f: &mut Frame, app: &TuiApp, overlay: Overlay) {
    let popup = centered_rect(78, 70, f.area());
    f.render_widget(Clear, popup);
    match overlay {
        Overlay::Help(tab) => render_help_modal(f, app, popup, tab),
        Overlay::CommandPalette => render_command_palette(f, app, popup),
        Overlay::Status => render_status_modal(f, app, popup),
        Overlay::Setup => render_setup_modal(f, app, popup),
        Overlay::ProviderPicker => render_provider_picker_modal(f, app, popup),
        Overlay::ModelPicker => render_model_picker_modal(f, app, popup),
        Overlay::BaseUrlEditor => render_base_url_editor_modal(f, app, popup),
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
                Paragraph::new(status_prompt_sources_text())
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
        Paragraph::new(status_prompt_sources_text())
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
        Paragraph::new("Select a provider family first, then choose a concrete model.")
            .block(Block::default().borders(Borders::ALL).title(" Provider Menu ")),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1/2 select  Up/Down move  Enter continue  Esc close")
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
    f.render_widget(
        Paragraph::new(format!(
            "Provider: {provider_label}\nBase URL: {}\nSelect a concrete model preset. Enter applies immediately.",
            app.config.base_url.as_deref().unwrap_or("http://localhost:11434"),
        ))
            .block(Block::default().borders(Borders::ALL).title(" Model Picker ")),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1/2/3 apply directly  Up/Down move  B edit base URL  Enter apply  Esc close")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_base_url_editor_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
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

fn role_badge_span(role: &str) -> Span<'static> {
    let (fg, bg) = match role {
        "You" => (Color::Black, Color::LightBlue),
        "Agent" => (Color::Black, Color::White),
        "Tool" => (Color::Black, Color::Rgb(231, 201, 92)),
        "Tool Result" => (Color::Black, Color::LightGreen),
        "Tool Error" => (Color::White, Color::Red),
        "Download" => (Color::Black, Color::LightBlue),
        "Runtime" => (Color::Black, Color::LightBlue),
        "Status" => (Color::White, Color::DarkGray),
        "System" => (Color::Black, Color::Magenta),
        _ => (Color::White, Color::DarkGray),
    };
    Span::styled(
        format!(" {} ", role),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
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
