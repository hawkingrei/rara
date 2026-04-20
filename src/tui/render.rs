mod bottom_pane;
pub(crate) mod cells;
mod history_pipeline;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

pub(crate) use self::bottom_pane::desired_viewport_height;
use self::bottom_pane::{editor_cursor_position, render_bottom_pane};
pub(crate) use self::cells::{ActiveCell, HistoryCell};
use self::cells::{ActiveTurnCell, CommittedTurnCell, StartupCardCell};
use super::auth_mode_picker::build_auth_mode_picker_view;
use super::command::{
    api_key_status, command_detail_text, command_spec_by_index, current_turn_preview,
    download_status_text, general_help_text, help_text, matching_commands, model_help_text,
    palette_command_by_index, palette_commands, quick_actions_text, recent_transcript_preview,
    status_prompt_sources_text, status_resources_text, status_runtime_text, status_workspace_text,
};
use super::custom_terminal::Frame;
use super::interaction_text::status_active_pending_interaction_text;
use super::line_utils::prefix_lines;
use super::plan_display::status_plan_text;
use super::state::{
    current_model_presets, HelpTab, Overlay, TranscriptEntry, TuiApp, PROVIDER_FAMILIES,
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

fn render_transcript(f: &mut Frame, app: &TuiApp, area: Rect) {
    let active_lines = active_turn_cell(app).display_lines(area.width);
    if !app.has_any_transcript() && active_lines.is_empty() {
        if app.startup_card_inserted {
            f.render_widget(Paragraph::new(Vec::<Line<'static>>::new()), area);
            return;
        }
        let lines = vec![
            Line::from(Span::styled(
                "Ready.",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("Use the input bar below to start a task or run a local command."),
            Line::from(""),
            Line::from(Span::styled(
                "Start with:",
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  /help    browse built-in commands and runtime hints"),
            Line::from("  /model   choose provider first, then switch models"),
            Line::from("  /status  inspect runtime, tokens, cache, and session"),
            Line::from("  /quit    leave the TUI and restore the terminal"),
            Line::from(""),
            Line::from(Span::styled(
                "Prompt ideas:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Explain this repository structure."),
            Line::from("  Find the main agent loop and summarize it."),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
        return;
    }

    f.render_widget(
        Paragraph::new(active_lines)
            .wrap(Wrap { trim: false })
            .scroll((app.transcript_scroll as u16, 0)),
        area,
    );
}

pub fn committed_turn_cell<'a>(
    entries: &'a [TranscriptEntry],
    cwd: Option<&'a Path>,
) -> CommittedTurnCell<'a> {
    CommittedTurnCell::new(entries, cwd)
}

pub(crate) fn committed_turn_lines(
    entries: &[TranscriptEntry],
    cwd: Option<&Path>,
    width: u16,
) -> Vec<Line<'static>> {
    committed_turn_cell(entries, cwd).display_lines(width)
}

pub fn active_turn_cell<'a>(app: &'a TuiApp) -> ActiveTurnCell<'a> {
    let cwd = (!app.snapshot.cwd.is_empty()).then(|| Path::new(app.snapshot.cwd.as_str()));
    ActiveTurnCell::new(app, cwd)
}

pub fn startup_card_cell(app: &TuiApp) -> StartupCardCell {
    StartupCardCell::new(
        app.current_model_label().to_string(),
        display_directory_for_startup(app),
    )
}

pub(crate) fn startup_card_lines(app: &TuiApp, width: u16) -> Vec<Line<'static>> {
    startup_card_cell(app).display_lines(width)
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

fn exploration_note_lines(current_turn: &[&TranscriptEntry]) -> Vec<String> {
    let mut notes = Vec::new();
    for entry in current_turn {
        if entry.role != "Agent" {
            continue;
        }
        for line in entry
            .message
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if line.starts_with("/search ")
                || line.starts_with("/compact ")
                || line.starts_with("/plan ")
                || line.starts_with("/quit ")
                || line.starts_with("key=")
                || line.starts_with("history=")
                || line.starts_with("tokens=")
                || line.starts_with("ctx~=")
                || line.starts_with("waiting for model response")
            {
                continue;
            }
            if notes.last().is_some_and(|existing| existing == line) {
                continue;
            }
            notes.push(line.to_string());
        }
    }
    notes
}

pub(crate) fn current_turn_exploration_summary_from_entries(
    current_turn: &[&TranscriptEntry],
    _show_live_detail: bool,
    _live_detail: Option<&str>,
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

    for note in exploration_note_lines(current_turn) {
        lines.push(format!("└ {note}"));
    }

    Some(lines.join("\n"))
}

pub(crate) fn current_turn_tool_summary(
    current_turn: &[&TranscriptEntry],
    _show_live_detail: bool,
    _live_detail: Option<&str>,
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

    let lines = actions
        .into_iter()
        .map(|action| format!("└ {action}"))
        .collect::<Vec<_>>();

    Some(lines.join("\n"))
}

pub(crate) fn prefixed_message_lines(
    role: &str,
    message: &str,
    max_lines: usize,
) -> Vec<Line<'static>> {
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

pub(crate) fn formatted_message_lines(
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

pub(crate) fn rendered_markdown_lines(
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
    matches!(name, "list_files" | "read_file" | "glob" | "grep")
}

fn exploration_action_label(message: &str) -> Option<String> {
    let mut parts = message.split_whitespace();
    let name = parts.next()?;
    let rest = parts.collect::<Vec<_>>().join(" ");
    match name {
        "list_files" => Some(format!(
            "List {}",
            if rest.is_empty() { "." } else { rest.as_str() }
        )),
        "read_file" => Some(format!(
            "Read {}",
            if rest.is_empty() {
                "file"
            } else {
                rest.as_str()
            }
        )),
        "glob" => Some(format!(
            "Glob {}",
            if rest.is_empty() {
                "workspace"
            } else {
                rest.as_str()
            }
        )),
        "grep" => Some(format!(
            "Search {}",
            if rest.is_empty() {
                "workspace"
            } else {
                rest.as_str()
            }
        )),
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
        "bash" => Some(format!(
            "Run {}",
            if rest.is_empty() {
                "command"
            } else {
                rest.as_str()
            }
        )),
        "apply_patch" => Some(format!(
            "Apply patch {}",
            if rest.is_empty() {
                "changes"
            } else {
                rest.as_str()
            }
        )),
        "write_file" => Some(format!(
            "Write {}",
            if rest.is_empty() {
                "file"
            } else {
                rest.as_str()
            }
        )),
        "replace" => Some(format!(
            "Edit {}",
            if rest.is_empty() {
                "file"
            } else {
                rest.as_str()
            }
        )),
        "web_fetch" => Some(format!(
            "Fetch {}",
            if rest.is_empty() {
                "resource"
            } else {
                rest.as_str()
            }
        )),
        other => Some(format!(
            "Run {}",
            if rest.is_empty() { other } else { message }
        )),
    }
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
        Overlay::AuthModePicker => {
            render_auth_mode_picker_modal(f, app, popup);
            None
        }
        Overlay::ApiKeyEditor => render_api_key_editor_modal(f, app, popup),
    }
}

fn render_help_modal(f: &mut Frame, app: &TuiApp, area: Rect, tab: HelpTab) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
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
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
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
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
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
                List::new(items).block(
                    Block::default()
                        .borders(Borders::LEFT | Borders::RIGHT)
                        .title(" Commands "),
                ),
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
                .constraints([
                    Constraint::Length(8),
                    Constraint::Length(6),
                    Constraint::Min(5),
                ])
                .split(inner[0]);
            let right = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(6), Constraint::Min(8)])
                .split(inner[1]);
            f.render_widget(
                Paragraph::new(status_runtime_text(app))
                    .block(
                        Block::default()
                            .borders(Borders::LEFT | Borders::RIGHT)
                            .title(" Runtime "),
                    )
                    .wrap(Wrap { trim: false }),
                left[0],
            );
            f.render_widget(
                Paragraph::new(status_workspace_text(app))
                    .block(
                        Block::default()
                            .borders(Borders::LEFT | Borders::RIGHT)
                            .title(" Workspace "),
                    )
                    .wrap(Wrap { trim: false }),
                left[1],
            );
            f.render_widget(
                Paragraph::new(status_prompt_sources_text(app))
                    .block(
                        Block::default()
                            .borders(Borders::LEFT | Borders::RIGHT)
                            .title(" Prompt Sources "),
                    )
                    .wrap(Wrap { trim: false }),
                left[2],
            );
            f.render_widget(
                Paragraph::new(status_resources_text(app))
                    .block(
                        Block::default()
                            .borders(Borders::RIGHT)
                            .title(" Resources "),
                    )
                    .wrap(Wrap { trim: false }),
                right[0],
            );
            f.render_widget(
                Paragraph::new(format!(
                    "{}\n\n{}",
                    model_help_text(app),
                    recent_transcript_preview(app, 4)
                ))
                .block(
                    Block::default()
                        .borders(Borders::RIGHT)
                        .title(" Models / Recent "),
                )
                .wrap(Wrap { trim: false }),
                right[1],
            );
        }
    }
    f.render_widget(
        Paragraph::new(
            "Esc close  1 general  2 commands  3 runtime  Up/Down move in command lists",
        )
        .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_command_palette(f: &mut Frame, app: &TuiApp, area: Rect) {
    let query = app.input.trim_start().trim_start_matches('/');
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
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
        Paragraph::new(intro).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Commands matching /{} ", query)),
        ),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .title(" Matches "),
        ),
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
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
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
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
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
        .constraints([
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
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
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Prompt Sources "),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    let right_panel = download_status_text(app).unwrap_or_else(|| quick_actions_text().to_string());
    let right_title = if matches!(
        app.runtime_phase,
        super::state::RuntimePhase::RebuildingBackend | super::state::RuntimePhase::BackendReady
    ) {
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
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Updated Plan "),
            )
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);
    let (interaction_title, interaction_text) = status_active_pending_interaction_text(app)
        .unwrap_or((" Request Input ", "No pending interaction.".to_string()));
    f.render_widget(
        Paragraph::new(interaction_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(interaction_title),
            )
            .wrap(Wrap { trim: false }),
        lower[0],
    );
    f.render_widget(
        Paragraph::new(current_turn_preview(app, 10))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Current Turn "),
            )
            .wrap(Wrap { trim: false }),
        lower[1],
    );
    f.render_widget(
        Paragraph::new(recent_transcript_preview(app, 8))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Recent Activity "),
            )
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
            let marker = if app.config.provider == *provider
                && app.config.model.as_deref() == Some(*model)
            {
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
         [1/2/3] Select preset\n[M] Cycle preset\n[Enter] Apply and rebuild\n[L] Auth modes\n[Esc] Close\n\n\
         Use /model for the full provider menu.\n\
         Codex supports browser login, device-code login, and API key auth.\n\
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
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
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
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);
    f.render_widget(
        Paragraph::new(
            "Select a provider family first, then choose a concrete runtime or auth path.",
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Provider Menu "),
        ),
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
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    let intro = if app.recent_sessions.is_empty() {
        "No persisted sessions found yet."
    } else {
        "Choose a recent session to restore its transcript, plan state, and interaction cards."
    };
    f.render_widget(
        Paragraph::new(intro).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Resume Session "),
        ),
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
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
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
        Paragraph::new("Esc close  Up/Down move  Enter restore").alignment(Alignment::Center),
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
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let current = if app.config.provider == *provider
                && app.config.model.as_deref() == Some(*model)
            {
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
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
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
        Paragraph::new(help).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Model Picker "),
        ),
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
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);
    let intro = Paragraph::new(
        "Edit the Ollama base URL for this provider.\nLeave it empty to clear the override. Default: http://localhost:11434",
    )
    .block(Block::default().borders(Borders::ALL).title(" Base URL "));
    let editor = Paragraph::new(app.base_url_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer =
        Paragraph::new("Enter save  Esc back to model picker").alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(
        app.base_url_input.as_str(),
        chunks[1],
    ))
}

fn render_auth_mode_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);
    let view = build_auth_mode_picker_view(app, super::is_ssh_session());
    f.render_widget(
        Paragraph::new(view.intro)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Codex Login "),
            )
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    let body = Paragraph::new(view.lines.join("\n"))
        .block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .title(" Details "),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(body, chunks[1]);

    f.render_widget(
        Paragraph::new(view.footer).alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_api_key_editor_modal(f: &mut Frame, app: &TuiApp, area: Rect) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);
    let intro = Paragraph::new(
        "Paste a Codex-compatible API key. This is the recommended path for SSH/headless sessions.",
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Codex API Key "),
    )
    .wrap(Wrap { trim: false });
    let editor = Paragraph::new(app.api_key_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer = Paragraph::new("Enter save and rebuild  Esc back to login guide")
        .alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(
        app.api_key_input.as_str(),
        chunks[1],
    ))
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

pub(crate) fn section_span<'a>(title: &'a str, color: Color) -> Span<'a> {
    Span::styled(
        format!(" {} ", title),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn wrapped_history_line_count(lines: &[Line<'static>], width: u16) -> u16 {
    let wrap_width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(wrap_width))
        .sum::<usize>()
        .max(1) as u16
}

fn badge<'a>(label: &'a str, value: &'a str, color: Color) -> Span<'a> {
    let fg = match color {
        Color::Black
        | Color::DarkGray
        | Color::Gray
        | Color::Blue
        | Color::Red
        | Color::Magenta => Color::White,
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

pub(crate) fn display_directory_for_startup(app: &TuiApp) -> String {
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

pub(crate) fn truncate_for_startup_card(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let kept = value.chars().take(width - 1).collect::<String>();
    format!("{kept}…")
}

pub(crate) fn truncate_path_middle(value: &str, width: usize) -> String {
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

pub(crate) fn startup_card_inner_width(width: u16) -> Option<usize> {
    if width < 8 {
        return None;
    }
    Some(std::cmp::min(width.saturating_sub(4) as usize, 56))
}

pub(crate) fn with_border(lines: Vec<Line<'static>>, inner_width: usize) -> Vec<Line<'static>> {
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

pub(crate) fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::config::ConfigManager;
    use crate::tui::state::TuiApp;
    use std::path::Path;

    use crate::tui::state::TranscriptEntry;

    use super::cells::HistoryCell;
    use super::{committed_turn_cell, current_turn_tool_summary, desired_viewport_height};

    #[test]
    fn committed_turn_does_not_truncate_agent_response() {
        let entries = vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Review the code".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: (1..=12)
                    .map(|idx| format!("Line {idx}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
        ];

        let rendered = committed_turn_cell(entries.as_slice(), Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Line 12"));
        assert!(!rendered.contains("more line(s)"));
    }

    #[test]
    fn keeps_history_reserve_once_transcript_exists() {
        let temp = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.committed_turns.push(crate::tui::state::TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Earlier prompt".into(),
            }],
        });

        let height = desired_viewport_height(&app, 120, 24);
        assert!(height > 5);
        assert!(height < 24);
    }

    #[test]
    fn tool_summary_includes_apply_patch_target_files() {
        let entries = vec![TranscriptEntry {
            role: "Tool".into(),
            message: "apply_patch src/tui/render.rs, src/tui/runtime/events.rs".into(),
        }];
        let refs = entries.iter().collect::<Vec<_>>();

        let rendered = current_turn_tool_summary(&refs, false, None).expect("tool summary");
        assert!(rendered.contains("Apply patch src/tui/render.rs, src/tui/runtime/events.rs"));
    }
}
