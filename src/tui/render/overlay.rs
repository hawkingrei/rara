#[path = "overlay_setup.rs"]
mod overlay_setup;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

use self::overlay_setup::{
    render_api_key_editor_modal, render_auth_mode_picker_modal, render_base_url_editor_modal,
    render_model_name_editor_modal, render_model_picker_modal, render_provider_picker_modal,
    render_reasoning_effort_picker_modal, render_resume_picker_modal,
};
use super::super::command::{
    current_turn_preview, download_status_text, general_help_text, matching_commands,
    model_help_text, palette_commands, quick_actions_text, recent_transcript_preview,
    status_context_text, status_prompt_sources_text, status_resources_text, status_runtime_text,
    status_workspace_text,
};
use super::super::custom_terminal::Frame;
use super::super::interaction_text::status_active_pending_interaction_text;
use super::super::plan_display::status_plan_text;
use super::super::state::{CommandSpec, HelpTab, Overlay, TuiApp};

pub(super) fn render_overlay(
    f: &mut Frame,
    app: &TuiApp,
    overlay: Overlay,
    bottom_pane_area: Rect,
) -> Option<(u16, u16)> {
    match overlay {
        Overlay::Help(tab) => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_help_modal(f, app, popup, tab);
            None
        }
        Overlay::CommandPalette => {
            let popup = command_palette_rect(f.area(), bottom_pane_area, app);
            f.render_widget(Clear, popup);
            render_command_palette(f, app, popup);
            None
        }
        Overlay::Status => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_status_modal(f, app, popup);
            None
        }
        Overlay::Context => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_context_modal(f, app, popup);
            None
        }
        Overlay::ProviderPicker => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_provider_picker_modal(f, app, popup);
            None
        }
        Overlay::ResumePicker => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_resume_picker_modal(f, app, popup);
            None
        }
        Overlay::ModelPicker => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_model_picker_modal(f, app, popup);
            None
        }
        Overlay::ReasoningEffortPicker => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_reasoning_effort_picker_modal(f, app, popup);
            None
        }
        Overlay::BaseUrlEditor => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_base_url_editor_modal(f, app, popup)
        }
        Overlay::AuthModePicker => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_auth_mode_picker_modal(f, app, popup);
            None
        }
        Overlay::ApiKeyEditor => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_api_key_editor_modal(f, app, popup)
        }
        Overlay::ModelNameEditor => {
            let popup = centered_rect(78, 70, f.area());
            f.render_widget(Clear, popup);
            render_model_name_editor_modal(f, app, popup)
        }
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
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .select(selected)
            .style(Style::default().fg(Color::DarkGray))
            .highlight_style(help_selected_tab_style()),
        chunks[0],
    );
    match tab {
        HelpTab::General => {
            f.render_widget(
                Paragraph::new(panel_text("general", general_help_text()))
                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                    .wrap(Wrap { trim: false }),
                chunks[1],
            );
        }
        HelpTab::Commands => {
            let query = app.input.trim_start().trim_start_matches('/');
            let items = help_command_items(query)
                .into_iter()
                .map(command_palette_item)
                .collect::<Vec<_>>();
            let mut state = command_palette_list_state(app.command_palette_idx);
            f.render_stateful_widget(
                List::new(items)
                    .highlight_style(command_list_highlight_style())
                    .highlight_symbol("› ")
                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
                chunks[1],
                &mut state,
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
                Paragraph::new(panel_text("runtime", &status_runtime_text(app)))
                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                    .wrap(Wrap { trim: false }),
                left[0],
            );
            f.render_widget(
                Paragraph::new(panel_text("workspace", &status_workspace_text(app)))
                    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                    .wrap(Wrap { trim: false }),
                left[1],
            );
            f.render_widget(
                Paragraph::new(panel_text(
                    "prompt sources",
                    &status_prompt_sources_text(app),
                ))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                .wrap(Wrap { trim: false }),
                left[2],
            );
            f.render_widget(
                Paragraph::new(panel_text("resources", &status_resources_text(app)))
                    .block(Block::default().borders(Borders::RIGHT))
                    .wrap(Wrap { trim: false }),
                right[0],
            );
            f.render_widget(
                Paragraph::new(panel_text(
                    "models / recent",
                    &format!(
                        "{}\n\n{}",
                        model_help_text(app),
                        recent_transcript_preview(app, 4)
                    ),
                ))
                .block(Block::default().borders(Borders::RIGHT))
                .wrap(Wrap { trim: false }),
                right[1],
            );
        }
    }
    f.render_widget(
        Paragraph::new("Esc close  1 general  2 commands  3 runtime  / open slash menu")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_command_palette(f: &mut Frame, app: &TuiApp, area: Rect) {
    let query = app.input.trim_start().trim_start_matches('/');
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let items = if query.is_empty() {
        palette_items_for_empty_query(app)
    } else {
        palette_items_for_matches(app, query)
    };
    let mut state = command_palette_list_state(app.command_palette_idx);
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(command_list_highlight_style())
            .highlight_symbol("› "),
        chunks[0],
        &mut state,
    );
    f.render_widget(
        Paragraph::new(command_palette_footer_text(query)).alignment(Alignment::Left),
        chunks[1],
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

fn help_command_items(query: &str) -> Vec<&'static CommandSpec> {
    matching_commands(query)
}

fn command_palette_item(spec: &CommandSpec) -> ListItem<'static> {
    ListItem::new(command_palette_line(spec))
}

fn command_palette_line(spec: &CommandSpec) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<11}", spec.usage),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(spec.summary.to_string(), Style::default().fg(Color::Gray)),
    ])
}

fn command_palette_footer_text(query: &str) -> &'static str {
    if query.is_empty() {
        "enter run  esc close"
    } else {
        "up/down move  enter run  esc close"
    }
}

fn panel_text(title: &str, body: &str) -> String {
    format!("{title}\n\n{body}")
}

fn command_list_highlight_style() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

fn help_selected_tab_style() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use ratatui::widgets::StatefulWidget;
    use ratatui::{buffer::Buffer, layout::Rect};
    use tempfile::tempdir;

    use super::*;
    use crate::config::ConfigManager;
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
        assert!(!line.contains('\n'));
    }

    #[test]
    fn help_command_items_are_alphabetical_for_empty_query() {
        let items = help_command_items("");

        assert_eq!(items.len(), COMMAND_SPECS.len());
        assert_eq!(items.first().map(|spec| spec.name), Some("approval"));
        assert_eq!(items.last().map(|spec| spec.name), Some("status"));
    }

    #[test]
    fn command_palette_footer_is_minimal_for_empty_query() {
        assert_eq!(command_palette_footer_text(""), "enter run  esc close");
        assert_eq!(
            command_palette_footer_text("mod"),
            "up/down move  enter run  esc close"
        );
    }

    #[test]
    fn panel_text_prefixes_body_with_lightweight_heading() {
        assert_eq!(
            panel_text("runtime", "provider=codex"),
            "runtime\n\nprovider=codex"
        );
    }

    #[test]
    fn command_palette_rect_anchors_above_bottom_pane() {
        let temp = tempdir().unwrap();
        let app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        let area = Rect::new(0, 0, 120, 40);
        let bottom_pane = Rect::new(0, 35, 120, 5);

        let popup = command_palette_rect(area, bottom_pane, &app);

        assert!(popup.bottom() <= bottom_pane.y);
        assert_eq!(popup.x, 1);
        assert!(popup.width <= 56);
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
        Paragraph::new(panel_text("runtime", &status_runtime_text(app)))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        top[0],
    );
    f.render_widget(
        Paragraph::new(panel_text("workspace", &status_workspace_text(app)))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        top[1],
    );
    f.render_widget(
        Paragraph::new(panel_text("resources", &status_resources_text(app)))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        top[2],
    );
    f.render_widget(
        Paragraph::new(panel_text(
            "prompt sources",
            &status_prompt_sources_text(app),
        ))
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
        .wrap(Wrap { trim: false }),
        chunks[1],
    );
    let right_panel = download_status_text(app).unwrap_or_else(|| quick_actions_text().to_string());
    f.render_widget(
        Paragraph::new(panel_text("models", &model_help_text(app)))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        middle[0],
    );
    f.render_widget(
        Paragraph::new(panel_text("downloads / actions", &right_panel))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        middle[1],
    );
    f.render_widget(
        Paragraph::new(panel_text("updated plan", &status_plan_text(app)))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);
    let interaction_text = status_active_pending_interaction_text(app)
        .map(|(_, text)| text)
        .unwrap_or_else(|| "No pending interaction.".to_string());
    f.render_widget(
        Paragraph::new(panel_text("request input", &interaction_text))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        lower[0],
    );
    f.render_widget(
        Paragraph::new(panel_text("current turn", &current_turn_preview(app, 10)))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        lower[1],
    );
    f.render_widget(
        Paragraph::new(panel_text(
            "recent activity",
            &recent_transcript_preview(app, 8),
        ))
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
        .wrap(Wrap { trim: false }),
        chunks[4],
    );
    f.render_widget(
        Paragraph::new("esc close  enter close  /help  /model").alignment(Alignment::Center),
        chunks[5],
    );
}

fn render_context_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(2)])
        .split(area);

    f.render_widget(
        Paragraph::new(panel_text("context", &status_context_text(app)))
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new("esc close  /status").alignment(Alignment::Center),
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

fn command_palette_rect(area: Rect, bottom_pane_area: Rect, app: &TuiApp) -> Rect {
    let query = app.input.trim_start().trim_start_matches('/');
    let item_count = if query.is_empty() {
        palette_commands(app, "").len()
    } else {
        matching_commands(query).len()
    };
    let visible_rows = item_count.clamp(1, 8) as u16;
    let height = visible_rows
        .saturating_add(1)
        .min(area.height.saturating_sub(2).max(3));
    let max_width = area.width.saturating_sub(4).max(24);
    let width = 56.min(max_width);
    let x = bottom_pane_area
        .x
        .saturating_add(1)
        .min(area.right().saturating_sub(width).max(area.x));
    let max_y = bottom_pane_area.y.saturating_sub(1);
    let y = max_y.saturating_sub(height).max(area.y);

    Rect::new(x, y, width, height)
}
