#[path = "overlay_setup.rs"]
mod overlay_setup;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

use super::super::command::{
    command_detail_text, command_spec_by_index, current_turn_preview, download_status_text,
    general_help_text, help_text, matching_commands, model_help_text,
    palette_command_by_index, palette_commands, quick_actions_text, recent_transcript_preview,
    status_context_text, status_prompt_sources_text, status_resources_text, status_runtime_text,
    status_workspace_text,
};
use super::super::custom_terminal::Frame;
use super::super::interaction_text::status_active_pending_interaction_text;
use super::super::plan_display::status_plan_text;
use super::super::state::{CommandSpec, HelpTab, Overlay, TuiApp};
use self::overlay_setup::{
    render_api_key_editor_modal, render_auth_mode_picker_modal, render_base_url_editor_modal,
    render_model_name_editor_modal, render_model_picker_modal, render_provider_picker_modal,
    render_reasoning_effort_picker_modal, render_resume_picker_modal, render_setup_modal,
};

pub(super) fn render_overlay(
    f: &mut Frame,
    app: &TuiApp,
    overlay: Overlay,
) -> Option<(u16, u16)> {
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
        Overlay::Context => {
            render_context_modal(f, app, popup);
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
        Overlay::ReasoningEffortPicker => {
            render_reasoning_effort_picker_modal(f, app, popup);
            None
        }
        Overlay::BaseUrlEditor => render_base_url_editor_modal(f, app, popup),
        Overlay::AuthModePicker => {
            render_auth_mode_picker_modal(f, app, popup);
            None
        }
        Overlay::ApiKeyEditor => render_api_key_editor_modal(f, app, popup),
        Overlay::ModelNameEditor => render_model_name_editor_modal(f, app, popup),
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
                .map(command_palette_item)
                .collect::<Vec<_>>();
            let detail = command_spec_by_index(query, app.command_palette_idx)
                .map(command_detail_text)
                .unwrap_or_else(help_text);
            let mut state = command_palette_list_state(app.command_palette_idx);
            f.render_stateful_widget(
                List::new(items)
                    .highlight_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("› ")
                    .block(
                        Block::default()
                            .borders(Borders::LEFT | Borders::RIGHT)
                            .title(" Commands "),
                    ),
                inner[0],
                &mut state,
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
    let mut state = command_palette_list_state(app.command_palette_idx);
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("› ")
            .block(
                Block::default()
                    .borders(Borders::LEFT | Borders::RIGHT)
                    .title(" Matches "),
            ),
        body[0],
        &mut state,
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

fn command_palette_list_state(selected_index: usize) -> ListState {
    let mut state = ListState::default();
    state.select(Some(selected_index));
    state
}

fn palette_items_for_empty_query(app: &TuiApp) -> Vec<ListItem<'static>> {
    palette_commands(app, "")
        .into_iter()
        .map(command_palette_item)
        .collect()
}

fn palette_items_for_matches(_app: &TuiApp, query: &str) -> Vec<ListItem<'static>> {
    matching_commands(query)
        .into_iter()
        .map(command_palette_item)
        .collect()
}

fn command_palette_item(spec: &CommandSpec) -> ListItem<'static> {
    ListItem::new(command_palette_line(spec))
}

fn command_palette_line(spec: &CommandSpec) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<12}", spec.usage),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("{}  ", spec.summary)),
        Span::styled(
            format!("[{}]", spec.category),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, layout::Rect};
    use ratatui::widgets::StatefulWidget;

    use super::*;
    use crate::tui::command::COMMAND_SPECS;

    #[test]
    fn command_palette_state_scrolls_to_selected_item() {
        let items = (0..20)
            .map(|idx| ListItem::new(format!("item {idx}")))
            .collect::<Vec<_>>();
        let area = Rect::new(0, 0, 20, 5);
        let mut buffer = Buffer::empty(area);
        let mut state = command_palette_list_state(10);

        List::new(items).render(area, &mut buffer, &mut state);

        assert!(state.offset() > 0);
    }

    #[test]
    fn command_palette_line_is_compact_single_row() {
        let spec = &COMMAND_SPECS[0];
        let line = command_palette_line(spec).to_string();

        assert!(line.contains(spec.usage));
        assert!(line.contains(spec.summary));
        assert!(line.contains(spec.category));
        assert!(!line.contains('\n'));
    }
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
        crate::tui::state::RuntimePhase::RebuildingBackend
            | crate::tui::state::RuntimePhase::BackendReady
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

fn render_context_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(2)])
        .split(area);

    f.render_widget(
        Paragraph::new(status_context_text(app))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Context "),
            )
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new("Esc close  /context re-open  /status runtime details")
            .alignment(Alignment::Center),
        chunks[1],
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

fn command_preview_text(spec: &CommandSpec) -> String {
    format!("{}\n\n{}", spec.usage, spec.summary)
}
