use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};

use super::command::{
    api_key_status, command_detail_text, command_spec_by_index, general_help_text, help_text,
    matching_commands, model_help_text,
};
use super::state::{HelpTab, Overlay, TaskKind, TuiApp, LOCAL_MODEL_PRESETS};

pub fn render(f: &mut Frame, app: &TuiApp) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(3),
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
            ListItem::new(Line::from("No messages yet. Use the composer below.")),
            ListItem::new(Line::from("")),
            ListItem::new(Line::from("Try:")),
            ListItem::new(Line::from("  /help")),
            ListItem::new(Line::from("  /model")),
            ListItem::new(Line::from("  Explain this repository structure.")),
        ]
    } else {
        app.transcript
            .iter()
            .map(|(role, message)| {
                ListItem::new(vec![
                    Line::from(Span::styled(
                        role.as_str(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(message.as_str()),
                    Line::from(""),
                ])
            })
            .collect::<Vec<_>>()
    };

    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        area,
    );
}

fn render_composer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let title = if app.is_busy() {
        " Composer (busy) "
    } else {
        " Composer "
    };
    f.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
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
    let notice = app
        .notice
        .as_ref()
        .map(|value| format!("  notice={value}"))
        .unwrap_or_default();
    let text = format!(
        "state={}  mode={}  key={}  messages={}  tokens={} in / {} out{}",
        activity,
        mode,
        api_key_status(&app.config),
        app.snapshot.history_len,
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        notice,
    );
    f.render_widget(Paragraph::new(text), area);
}

fn render_header(f: &mut Frame, app: &TuiApp, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " RARA ",
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  provider={}  model={}  revision={}  key={}",
                app.config.provider,
                app.current_model_label(),
                app.config.revision.as_deref().unwrap_or("main"),
                api_key_status(&app.config),
            )),
        ]),
        Line::from(format!(
            " workspace={}  branch={}  session={} ",
            app.snapshot.cwd, app.snapshot.branch, app.snapshot.session_id
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
            f.render_widget(
                Paragraph::new(format!(
                    "{}\n\n{}",
                    super::command::status_text(app),
                    model_help_text(app)
                ))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                .wrap(Wrap { trim: false }),
                chunks[1],
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
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
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
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Commands matching /{} ", query)),
        ),
        chunks[0],
    );
    let detail = command_spec_by_index(query, app.command_palette_idx)
        .map(command_detail_text)
        .unwrap_or_else(help_text);
    f.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::ALL).title(" Detail "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn render_status_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    f.render_widget(
        Paragraph::new(super::command::status_text(app))
            .block(Block::default().borders(Borders::ALL).title(" Status "))
            .wrap(Wrap { trim: false }),
        area,
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
         [1/2/3] Select preset\n[M] Cycle preset\n[Enter] Apply and rebuild\n[L] OAuth login\n[Esc] Close",
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
            ListItem::new(format!(
                "[{}] {} ({}/{}){}",
                idx + 1,
                label,
                provider,
                model,
                current
            ))
            .style(style)
        })
        .collect::<Vec<_>>();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(2)])
        .split(area);
    f.render_widget(
        Paragraph::new("Select a local model preset. Enter applies it immediately.")
            .block(Block::default().borders(Borders::ALL).title(" Model Picker ")),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("Esc close  Up/Down move  Enter apply")
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
