use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
};

use super::super::auth_mode_picker::build_auth_mode_picker_view;
use super::super::custom_terminal::Frame;
use super::super::command::{
    api_key_status, command_detail_text, command_spec_by_index, current_turn_preview,
    download_status_text, general_help_text, help_text, matching_commands, model_help_text,
    palette_command_by_index, palette_commands, quick_actions_text, recent_transcript_preview,
    status_prompt_sources_text, status_resources_text, status_runtime_text, status_workspace_text,
};
use super::super::state::{
    current_model_presets, CommandSpec, HelpTab, Overlay, PROVIDER_FAMILIES, TuiApp,
};
use super::super::interaction_text::status_active_pending_interaction_text;
use super::super::plan_display::status_plan_text;
use super::bottom_pane::editor_cursor_position;

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
        Overlay::ModelNameEditor => render_model_name_editor_modal(f, app, popup),
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

fn command_palette_item(index: usize, selected_index: usize, spec: &CommandSpec) -> ListItem<'static> {
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
            .block(Block::default().borders(Borders::ALL).title(" Prompt Sources "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
    let right_panel = download_status_text(app).unwrap_or_else(|| quick_actions_text().to_string());
    let right_title = if matches!(
        app.runtime_phase,
        super::super::state::RuntimePhase::RebuildingBackend
            | super::super::state::RuntimePhase::BackendReady
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
            .block(Block::default().borders(Borders::ALL).title(" Updated Plan "))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);
    let (interaction_title, interaction_text) =
        status_active_pending_interaction_text(app).unwrap_or((
            " Request Input ",
            "No pending interaction.".to_string(),
        ));
    f.render_widget(
        Paragraph::new(interaction_text)
            .block(Block::default().borders(Borders::ALL).title(interaction_title))
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
         [1/2/3] Select preset\n[M] Cycle preset\n[Enter] Apply and rebuild\n[L] Codex auth modes\n[Esc] Close\n\n\
         Use /model for the full provider menu.\n\
         OpenAI-compatible endpoints can be configured from the model picker with base URL, API key, and model name.\n\
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
        Paragraph::new(
            "Select a provider family first, then choose a concrete runtime or auth path.",
        )
        .block(Block::default().borders(Borders::ALL).title(" Provider Menu ")),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1/2/3/4 select  Up/Down move  Enter continue  Esc close")
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
    } else if provider_label == "OpenAI-compatible" {
        &format!(
            "Provider: {provider_label}\nBase URL: {}\nModel name: {}\nAPI key: {}\nUse B/A/N to edit connection settings, then Enter to apply and rebuild.",
            app.config
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1"),
            app.current_model_label(),
            api_key_status(&app.config),
        )
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
        } else if provider_label == "OpenAI-compatible" {
            "1 choose  Up/Down move  B edit base URL  A edit API key  N edit model name  Enter apply  Esc close"
        } else {
            "1/2/3 apply directly  Up/Down move  B edit base URL  Enter apply  Esc close"
        })
        .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_base_url_editor_modal(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(3), Constraint::Length(2)])
        .split(area);
    let intro = Paragraph::new(if app.config.provider == "openai-compatible" {
        "Edit the OpenAI-compatible base URL for this provider.\nLeave it empty to restore the default. Default: https://api.openai.com/v1"
    } else {
        "Edit the Ollama base URL for this provider.\nLeave it empty to clear the override. Default: http://localhost:11434"
    })
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

fn render_auth_mode_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(6), Constraint::Length(2)])
        .split(area);
    let view = build_auth_mode_picker_view(app, super::super::is_ssh_session());
    f.render_widget(
        Paragraph::new(view.intro)
            .block(Block::default().borders(Borders::ALL).title(" Codex Login "))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    let body = Paragraph::new(view.lines.join("\n"))
    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Details "))
    .wrap(Wrap { trim: false });
    f.render_widget(body, chunks[1]);

    f.render_widget(
        Paragraph::new(view.footer).alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_api_key_editor_modal(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(3), Constraint::Length(2)])
        .split(area);
    let (intro_text, title, footer_text) = if app.config.provider == "openai-compatible" {
        (
            "Paste the API key for the selected OpenAI-compatible endpoint.",
            " API Key ",
            "Enter save  Esc back to model picker",
        )
    } else {
        (
            "Paste a Codex-compatible API key. This is the recommended path for SSH/headless sessions.",
            " Codex API Key ",
            "Enter save and rebuild  Esc back to login guide",
        )
    };
    let intro = Paragraph::new(intro_text)
    .block(Block::default().borders(Borders::ALL).title(title))
    .wrap(Wrap { trim: false });
    let editor = Paragraph::new(app.api_key_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer = Paragraph::new(footer_text)
        .alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(app.api_key_input.as_str(), chunks[1]))
}

fn render_model_name_editor_modal(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(3), Constraint::Length(2)])
        .split(area);
    let intro = Paragraph::new(
        "Set the model name to send to the OpenAI-compatible endpoint.\nExample: gpt-4.1-mini, kimi-k2, or any server-specific model id.",
    )
    .block(Block::default().borders(Borders::ALL).title(" Model Name "))
    .wrap(Wrap { trim: false });
    let editor = Paragraph::new(app.model_name_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer = Paragraph::new("Enter save  Esc back to model picker")
        .alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(app.model_name_input.as_str(), chunks[1]))
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
