use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};

use super::command::{
    api_key_status, command_detail_text, command_spec_by_index, general_help_text, help_text,
    matching_commands, model_help_text, quick_actions_text, recent_transcript_preview,
    status_resources_text, status_runtime_text, status_workspace_text,
};
use super::state::{
    HelpTab, Overlay, TaskKind, TuiApp, LOCAL_MODEL_PRESETS, MODEL_GUIDE_OPTIONS,
};

pub fn render(f: &mut Frame, app: &TuiApp) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
            Constraint::Length(2),
        ])
        .split(f.area());

    render_header(f, app, layout[0]);
    render_transcript(f, app, layout[1]);
    render_composer(f, app, layout[2]);
    render_footer(f, app, layout[3]);

    if let Some(overlay) = app.overlay {
        render_overlay(f, app, overlay);
    }
}

fn render_transcript(f: &mut Frame, app: &TuiApp, area: Rect) {
    let items = if app.transcript.is_empty() {
        vec![
            ListItem::new(Line::from(Span::styled(
                "No messages yet.",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))),
            ListItem::new(Line::from("Use the composer below to start a task or open a local command.")),
            ListItem::new(Line::from("")),
            ListItem::new(Line::from(Span::styled(
                "Start with:",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ))),
            ListItem::new(Line::from("  /help    browse built-in commands and runtime hints")),
            ListItem::new(Line::from("  /model   switch between Qwn3 and Gemma presets")),
            ListItem::new(Line::from("  /status  inspect runtime, tokens, cache, and session")),
            ListItem::new(Line::from("")),
            ListItem::new(Line::from(Span::styled(
                "Prompt ideas:",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ))),
            ListItem::new(Line::from("  Explain this repository structure.")),
            ListItem::new(Line::from("  Find the main agent loop and summarize it.")),
        ]
    } else {
        let max_visible_entries = area.height.saturating_sub(3).max(6) as usize / 4;
        let max_visible_entries = max_visible_entries.max(3);
        let start = app.transcript.len().saturating_sub(max_visible_entries);
        let mut items = Vec::new();
        if start > 0 {
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    "Earlier entries hidden ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "showing latest {} of {} transcript events",
                    app.transcript.len() - start,
                    app.transcript.len()
                )),
            ])));
            items.push(ListItem::new(Line::from("")));
        }
        items.extend(
            app.transcript
                .iter()
                .skip(start)
                .enumerate()
                .map(|(idx, (role, message))| transcript_item(idx + start, role, message)),
        );
        items
    };

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .title(" Transcript "),
        ),
        area,
    );
}

fn render_composer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let title = if app.is_busy() {
        " Composer (busy) "
    } else {
        " Composer "
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let composer_text = if app.input.is_empty() {
        Line::from(vec![
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
        ])
    } else {
        Line::from(app.input.as_str())
    };
    f.render_widget(
        Paragraph::new(composer_text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
    let hint = if app.input.trim_start().starts_with('/') {
        "slash command mode  Enter run  Esc close overlay"
    } else if app.is_busy() {
        "runtime busy  wait for current task to finish"
    } else {
        "plain prompt mode  /help commands  /model switch models"
    };
    f.render_widget(
        Paragraph::new(hint).alignment(Alignment::Right),
        chunks[1],
    );
}

fn render_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let mode = if super::provider_requires_api_key(&app.config.provider) {
        "hosted"
    } else {
        "local"
    };
    let activity = match app.running_task.as_ref().map(|task| &task.kind) {
        Some(TaskKind::Query) => "query",
        Some(TaskKind::Rebuild) => "reload",
        Some(TaskKind::OAuth) => "oauth",
        None => "idle",
    };
    let summary = format!(
        "state={}  mode={}  key={}  messages={}  transcript={}  tokens={} in / {} out",
        activity,
        mode,
        api_key_status(&app.config),
        app.snapshot.history_len,
        app.transcript.len(),
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
    );
    let notice = if let Some(notice) = app.notice.as_ref() {
        Line::from(vec![
            Span::styled("notice ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(notice.as_str()),
        ])
    } else if app.is_busy() {
        Line::from(vec![
            Span::styled("hint ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("the UI stays responsive while the current task runs in background"),
        ])
    } else {
        Line::from(vec![
            Span::styled("hint ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("use /model to switch presets or /status to inspect the current runtime"),
        ])
    };
    f.render_widget(Paragraph::new(vec![Line::from(summary), notice]), area);
}

fn render_header(f: &mut Frame, app: &TuiApp, area: Rect) {
    let activity = match app.running_task.as_ref().map(|task| &task.kind) {
        Some(TaskKind::Query) => ("query", Color::Yellow),
        Some(TaskKind::Rebuild) => ("reload", Color::LightBlue),
        Some(TaskKind::OAuth) => ("oauth", Color::LightGreen),
        None => ("idle", Color::DarkGray),
    };
    let provider_color = if super::provider_requires_api_key(&app.config.provider) {
        Color::Magenta
    } else {
        Color::Green
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " RARA ",
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            badge("provider", &app.config.provider, provider_color),
            Span::raw(" "),
            badge("state", activity.0, activity.1),
            Span::raw(" "),
            badge("key", api_key_status(&app.config), Color::DarkGray),
        ]),
        Line::from(vec![
            Span::raw(format!(
                " model={}  revision={}  workspace={} ",
                app.current_model_label(),
                app.config.revision.as_deref().unwrap_or("main"),
                app.snapshot.cwd,
            )),
        ]),
        Line::from(format!(
            " branch={}  session={} ",
            app.snapshot.branch, app.snapshot.session_id
        )),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn render_overlay(f: &mut Frame, app: &TuiApp, overlay: Overlay) {
    let popup = centered_rect(78, 70, f.area());
    f.render_widget(Clear, popup);
    match overlay {
        Overlay::Welcome => render_welcome_modal(f, app, popup),
        Overlay::Help(tab) => render_help_modal(f, app, popup, tab),
        Overlay::CommandPalette => render_command_palette(f, app, popup),
        Overlay::Status => render_status_modal(f, app, popup),
        Overlay::Setup => render_setup_modal(f, app, popup),
        Overlay::ModelGuide => render_model_guide_modal(f, app, popup),
        Overlay::ModelPicker => render_model_picker_modal(f, app, popup),
    }
}

fn render_welcome_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let content = format!(
        "RARA now uses a single chat composer as the control surface.\n\n\
         Start here:\n\
           - Type a task directly in the composer.\n\
           - Use /help to browse built-in commands.\n\
           - Use /model to switch local presets.\n\
           - Use /status to inspect runtime state.\n\n\
         Current runtime:\n\
           provider={}\n\
           model={}\n\
           revision={}\n\
           workspace={}\n\
           branch={}\n\n\
         Press Esc to close this welcome panel.",
        app.config.provider,
        app.current_model_label(),
        app.config.revision.as_deref().unwrap_or("main"),
        app.snapshot.cwd,
        app.snapshot.branch,
    );
    f.render_widget(
        Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title(" Welcome "))
            .wrap(Wrap { trim: false }),
        area,
    );
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
                .constraints([Constraint::Length(8), Constraint::Min(6)])
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
    let items = matching_commands(query)
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
        .collect::<Vec<_>>();
    let intro = if query.is_empty() {
        "Start typing after / to narrow commands. Exact commands like /model or /status can be submitted directly."
    } else {
        "Use Up/Down to inspect matches. Enter accepts the highlighted command into the composer."
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
    let detail = command_spec_by_index(query, app.command_palette_idx)
        .map(command_detail_text)
        .unwrap_or_else(help_text);
    f.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::RIGHT).title(" Detail "))
            .wrap(Wrap { trim: false }),
        body[1],
    );
    f.render_widget(
        Paragraph::new("Esc close  Enter accept highlighted command  Keep typing to refine")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_status_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Length(6), Constraint::Min(8), Constraint::Length(2)])
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
        Paragraph::new(model_help_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Models "))
            .wrap(Wrap { trim: false }),
        middle[0],
    );
    f.render_widget(
        Paragraph::new(quick_actions_text())
            .block(Block::default().borders(Borders::ALL).title(" Quick Actions "))
            .wrap(Wrap { trim: false }),
        middle[1],
    );
    f.render_widget(
        Paragraph::new(recent_transcript_preview(app, 8))
            .block(Block::default().borders(Borders::ALL).title(" Recent Activity "))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    f.render_widget(
        Paragraph::new("Esc close  Enter close  /help commands  /model switch runtime")
            .alignment(Alignment::Center),
        chunks[3],
    );
}

fn render_setup_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let preset_lines = LOCAL_MODEL_PRESETS
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
        "Provider: {}\nModel: {}\nAPI key: {}\nRevision: {}\n\n\
         Presets:\n{}\n\n\
         [1/2/3] Select preset\n[M] Cycle preset\n[Enter] Apply and rebuild\n[L] OAuth login\n[Esc] Close\n\n\
         Recommended: Qwn3 8B for stable local use.\n\
         Gemma 4 presets are marked experimental.",
        app.config.provider,
        app.current_model_label(),
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

fn render_model_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let items = LOCAL_MODEL_PRESETS
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
            let recommendation = if idx == 2 {
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
        Paragraph::new("Select a local model preset. Qwn3 is the stable default; Gemma 4 presets are experimental. Enter applies immediately.")
            .block(Block::default().borders(Borders::ALL).title(" Model Picker ")),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1/2/3 apply directly  Up/Down move  Enter apply  Esc close")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_model_guide_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(8), Constraint::Length(2)])
        .split(area);
    f.render_widget(
        Paragraph::new(
            "Model guide\n\nWhat do you want to optimize for right now?\nGemma 4 paths are marked experimental.",
        )
        .block(Block::default().borders(Borders::ALL).title(" Model Guide ")),
        chunks[0],
    );
    let items = MODEL_GUIDE_OPTIONS
        .iter()
        .enumerate()
        .map(|(idx, (label, detail, preset_idx))| {
            let style = if idx == app.model_guide_idx {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let selected_marker = MODEL_GUIDE_OPTIONS[idx]
                .2
                .and_then(|preset_idx| {
                    let (_, provider, model) = LOCAL_MODEL_PRESETS[preset_idx];
                    (app.config.provider == provider && app.config.model.as_deref() == Some(model))
                        .then_some(" current")
                })
                .unwrap_or("");
            let target = preset_idx
                .map(|preset| format!(" -> {}", LOCAL_MODEL_PRESETS[preset].0))
                .unwrap_or_else(|| " -> open picker".to_string());
            ListItem::new(vec![
                Line::from(format!("[{}] {}{}{}", idx + 1, label, target, selected_marker)),
                Line::from(*detail),
                Line::from(""),
            ])
            .style(style)
        })
        .collect::<Vec<_>>();
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1 fast  2 balanced  3 strongest  4 manual  Enter apply  Esc close")
            .alignment(Alignment::Center),
        chunks[2],
    );
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

fn transcript_item(index: usize, role: &str, message: &str) -> ListItem<'static> {
    let mut lines = vec![Line::from(vec![
        role_badge_span(role),
        Span::raw(" "),
        Span::styled(
            format!("#{}", index + 1),
            Style::default().fg(Color::DarkGray),
        ),
    ])];
    let message_lines = message.lines().collect::<Vec<_>>();
    if message_lines.is_empty() {
        lines.push(Line::from("  "));
    } else {
        for line in message_lines {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default().fg(Color::DarkGray)),
                Span::raw(line.to_string()),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        "  ",
        Style::default().fg(Color::DarkGray),
    )));
    ListItem::new(lines)
}

fn role_badge_span(role: &str) -> Span<'static> {
    let (fg, bg) = match role {
        "You" => (Color::Black, Color::Green),
        "Agent" => (Color::Black, Color::Cyan),
        "Tool" => (Color::Black, Color::Yellow),
        "Tool Result" => (Color::Black, Color::LightGreen),
        "Tool Error" => (Color::White, Color::Red),
        "Runtime" => (Color::Black, Color::LightBlue),
        "Status" => (Color::Black, Color::Gray),
        "System" => (Color::Black, Color::Magenta),
        _ => (Color::White, Color::DarkGray),
    };
    Span::styled(
        format!(" {} ", role),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

fn badge<'a>(label: &'a str, value: &'a str, color: Color) -> Span<'a> {
    Span::styled(
        format!(" {}={} ", label, value),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}
