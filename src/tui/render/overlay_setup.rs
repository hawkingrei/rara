use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::path::Path;

use super::Frame;
use crate::tui::auth_mode_picker::build_auth_mode_picker_view;
use crate::tui::command::api_key_status;
use crate::tui::is_ssh_session;
use crate::tui::render::bottom_pane::editor_cursor_position;
use crate::tui::state::{current_model_presets, ProviderFamily, TuiApp, PROVIDER_FAMILIES};

pub(super) fn render_provider_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
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
        Paragraph::new("1/2/3/4 select  Up/Down move  Enter continue  Esc close")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

pub(super) fn render_resume_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    let intro = if app.recent_threads.is_empty() {
        "No persisted threads found yet."
    } else {
        "Choose a recent thread to restore its transcript, plan state, and interaction cards."
    };
    f.render_widget(
        Paragraph::new(intro).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Resume Thread "),
        ),
        chunks[0],
    );
    let items = if app.recent_threads.is_empty() {
        vec![ListItem::new("No threads available.")]
    } else {
        app.recent_threads
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
                let when = format!("updated_at={}", session.metadata.updated_at);
                let preview = if session.preview.is_empty() {
                    "(no preview)".to_string()
                } else {
                    session.preview.clone()
                };
                let workspace = Path::new(&session.metadata.cwd)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("-");
                let compaction = if session.compaction.compaction_count > 0 {
                    format!("compact={}", session.compaction.compaction_count)
                } else {
                    "compact=0".to_string()
                };
                ListItem::new(vec![
                    Line::from(format!(
                        "{}  {} / {}  branch={}",
                        session.metadata.session_id,
                        session.metadata.provider,
                        session.metadata.model,
                        session.metadata.branch
                    )),
                    Line::from(format!(
                        "  {when}  mode={}  workspace={}  {}",
                        session.metadata.agent_mode, workspace, compaction
                    )),
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

pub(super) fn render_model_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let provider_label = PROVIDER_FAMILIES[app.provider_picker_idx].1;
    let items = if app.selected_provider_family() == ProviderFamily::Codex {
        app.codex_model_options
            .iter()
            .enumerate()
            .map(|(idx, preset)| {
                let style = if idx == app.model_picker_idx {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let current = if app.config.provider == "codex"
                    && app.config.model.as_deref() == Some(preset.model.as_str())
                {
                    " current"
                } else {
                    ""
                };
                let level = preset
                    .default_reasoning_effort
                    .as_deref()
                    .unwrap_or("default");
                ListItem::new(format!(
                    "[{}] {} ({})  level={}{}",
                    idx + 1,
                    preset.label,
                    preset.model,
                    level,
                    current,
                ))
                .style(style)
            })
            .collect::<Vec<_>>()
    } else {
        let presets = current_model_presets(app.provider_picker_idx);
        let displayed_preset_count = app.openai_model_picker_preset_count();
        let mut items = presets
            .iter()
            .take(displayed_preset_count)
            .enumerate()
            .map(|(idx, (label, provider, model))| {
                let style = if idx == app.model_picker_idx {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let current = if provider_label == "OpenAI-compatible" {
                    if app.config.active_openai_profile_kind()
                        == Some(crate::tui::state::openai_compatible_preset_kind(idx))
                    {
                        " current"
                    } else {
                        ""
                    }
                } else if app.config.provider == *provider
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
        if provider_label == "OpenAI-compatible" {
            let mut actions = Vec::new();
            if app.openai_profile_needs_setup() {
                actions.push((
                    "Setup endpoint",
                    "Guided setup for endpoint URL, API key, and model name.",
                ));
            }
            actions.extend([
                ("Profiles", "Switch or create endpoint profiles"),
                ("API key", "Edit the API key for the active profile"),
                ("Base URL", "Edit the base URL for the active profile"),
                ("Model name", "Edit the model id for the active profile"),
            ]);
            let offset = displayed_preset_count;
            items.extend(
                actions
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (label, detail))| {
                        let absolute_idx = offset + idx;
                        let style = if absolute_idx == app.model_picker_idx {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        ListItem::new(vec![
                            Line::from(format!("[{}] {label}", absolute_idx + 1)),
                            Line::from(format!("  {detail}")),
                        ])
                        .style(style)
                    }),
            );
        }
        items
    };
    let help = if provider_label == "Codex" && api_key_status(&app.config) == "missing" {
        "Provider: Codex\nAuthentication is required before this preset can be used.\nEnter opens the Codex login guide."
    } else if provider_label == "Codex" {
        &format!(
            "Provider: {provider_label}\nBase URL: {}\nReasoning level: {}\nChoose a model first, then Enter to select the level.",
            app.config
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1"),
            app.current_reasoning_effort_label(),
        )
    } else if provider_label == "OpenAI-compatible" {
        &format!(
            "Provider: {provider_label}\nEndpoint kind: {}\nEndpoint profile: {}\nBase URL: {}\nModel name: {}\nAPI key: {}\n{}",
            app.selected_openai_profile_kind()
                .unwrap_or(crate::config::OpenAiEndpointKind::Custom)
                .label(),
            app.config.active_openai_profile_label().unwrap_or("-"),
            app.config
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1"),
            app.current_model_label(),
            api_key_status(&app.config),
            if app.openai_profile_needs_setup() {
                "Active profile still needs setup. Press Enter on `Setup endpoint` to choose an endpoint family and walk through the required fields."
            } else {
                "The active profile is ready. Choose a preset to rebuild, or use the action rows below to edit the active profile."
            },
        )
    } else {
        &format!(
            "Provider: {provider_label}\nBase URL: {}\nSelect a concrete model preset. Enter applies immediately.",
            app.config.base_url.as_deref().unwrap_or("http://localhost:11434"),
        )
    };
    let help_height = help.lines().count().max(1) as u16 + 2;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(help_height),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);
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
            "1-9 jump  Up/Down move  Enter choose level  Esc close"
        } else if provider_label == "OpenAI-compatible" {
            if app.openai_profile_needs_setup() {
                "1-5 jump  Up/Down move  Enter continue  Esc close"
            } else {
                "1-9 jump  Up/Down move  Enter choose  Esc close"
            }
        } else {
            "1-9 apply directly  Up/Down move  B edit base URL  Enter apply  Esc close"
        })
        .alignment(Alignment::Center),
        chunks[2],
    );
}

pub(super) fn render_openai_profile_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let kind = app
        .selected_openai_profile_kind()
        .unwrap_or(crate::config::OpenAiEndpointKind::Custom);
    let mut items = vec![ListItem::new(vec![
        Line::from("[1] Create new profile"),
        Line::from(format!("  Add another {} endpoint profile.", kind.label())),
        Line::from(""),
    ])];
    items.extend(app.selected_openai_profiles().into_iter().enumerate().map(
        |(idx, (id, label))| {
            let active_suffix = if app.config.active_openai_profile_id() == Some(id.as_str()) {
                " active"
            } else {
                ""
            };
            ListItem::new(vec![
                Line::from(format!("[{}] {}{}", idx + 2, label, active_suffix)),
                Line::from(format!("  id={id}")),
                Line::from(""),
            ])
        },
    ));
    let items = items
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            if idx == app.openai_profile_picker_idx {
                item.style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                item
            }
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
        Paragraph::new(format!(
            "Choose the active {} endpoint profile, or create a new one.",
            kind.label()
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Endpoint Profiles "),
        ),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("Up/Down move  Enter choose  Esc back").alignment(Alignment::Center),
        chunks[2],
    );
}

pub(super) fn render_openai_endpoint_kind_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let kinds = [
        crate::config::OpenAiEndpointKind::Custom,
        crate::config::OpenAiEndpointKind::Deepseek,
        crate::config::OpenAiEndpointKind::Kimi,
        crate::config::OpenAiEndpointKind::Openrouter,
    ];
    let items = kinds
        .into_iter()
        .enumerate()
        .map(|(idx, kind)| {
            let current = if app.config.active_openai_profile_kind() == Some(kind) {
                " current"
            } else {
                ""
            };
            let detail = match kind {
                crate::config::OpenAiEndpointKind::Custom => {
                    "Bring your own endpoint URL, API key, and model id."
                }
                crate::config::OpenAiEndpointKind::Deepseek => {
                    "Use DeepSeek defaults, then fill in the API key."
                }
                crate::config::OpenAiEndpointKind::Kimi => {
                    "Use Kimi defaults, then fill in the API key."
                }
                crate::config::OpenAiEndpointKind::Openrouter => {
                    "Use OpenRouter defaults, then fill in the API key."
                }
            };
            let item = ListItem::new(vec![
                Line::from(format!("[{}] {}{}", idx + 1, kind.label(), current)),
                Line::from(format!("  {detail}")),
                Line::from(""),
            ]);
            if idx == app.openai_endpoint_kind_picker_idx {
                item.style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                item
            }
        })
        .collect::<Vec<_>>();
    let intro = "Choose which OpenAI-compatible endpoint family to configure first.\nThe next steps will walk through the connection fields for that endpoint.";
    let intro_height = intro.lines().count().max(1) as u16 + 2;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(intro_height),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    f.render_widget(
        Paragraph::new(intro)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Endpoint Kind "),
        )
        .wrap(Wrap { trim: false }),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1-4 jump  Up/Down move  Enter choose  Esc back")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

pub(super) fn render_reasoning_effort_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let options = app.selected_codex_reasoning_options();
    let title = app
        .selected_codex_model()
        .map(|preset| format!(" Reasoning Level · {} ", preset.label))
        .unwrap_or_else(|| " Reasoning Level ".to_string());
    let items = options
        .iter()
        .enumerate()
        .map(|(idx, option)| {
            let style = if idx == app.reasoning_effort_picker_idx {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let default_suffix = if option.is_default { " default" } else { "" };
            ListItem::new(vec![
                Line::from(format!("[{}] {}{}", idx + 1, option.label, default_suffix)),
                Line::from(option.description.clone()),
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
        Paragraph::new("Select the reasoning level for the chosen Codex model. Enter persists both the model and the level.")
            .block(Block::default().borders(Borders::ALL).title(title)),
        chunks[0],
    );
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new("1-5 jump  Up/Down move  Enter apply  Esc back")
            .alignment(Alignment::Center),
        chunks[2],
    );
}

pub(super) fn render_base_url_editor_modal(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let is_openai_compatible = matches!(
        app.selected_provider_family(),
        ProviderFamily::OpenAiCompatible
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);
    let intro = Paragraph::new(if is_openai_compatible {
        "Edit the base URL for the selected OpenAI-compatible endpoint profile.\nLeave it empty to restore that profile's default endpoint."
    } else {
        "Edit the Ollama base URL for this provider.\nLeave it empty to clear the override. Default: http://localhost:11434"
    })
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
        app.base_url_cursor_offset(),
        chunks[1],
    ))
}

pub(super) fn render_auth_mode_picker_modal(f: &mut Frame, app: &TuiApp, area: Rect) {
    let view = build_auth_mode_picker_view(app, is_ssh_session());
    let intro_height = view.intro.lines().count().max(1) as u16 + 2;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(intro_height),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);
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

pub(super) fn render_api_key_editor_modal(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let is_openai_compatible = matches!(
        app.selected_provider_family(),
        ProviderFamily::OpenAiCompatible
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);
    let (intro_text, title, footer_text) = if is_openai_compatible {
        (
            "Paste the API key for the selected OpenAI-compatible endpoint profile.",
            " API Key ",
            "Enter save  Esc back to model picker",
        )
    } else {
        (
            "Paste a Codex API key. This is the recommended path for SSH/headless sessions.",
            " Codex API Key ",
            "Enter save and rebuild  Esc back to login guide",
        )
    };
    let intro = Paragraph::new(intro_text)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    let editor = Paragraph::new(app.api_key_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer = Paragraph::new(footer_text).alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(
        app.api_key_input.as_str(),
        app.api_key_cursor_offset(),
        chunks[1],
    ))
}

pub(super) fn render_model_name_editor_modal(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);
    let intro = Paragraph::new(
        "Set the model name for the selected OpenAI-compatible endpoint profile.\nExample: gpt-4o-mini, kimi-k2, deepseek-chat, or any server-specific model id.",
    )
    .block(Block::default().borders(Borders::ALL).title(" Model Name "))
    .wrap(Wrap { trim: false });
    let editor = Paragraph::new(app.model_name_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Value "));
    let footer =
        Paragraph::new("Enter save  Esc back to model picker").alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(
        app.model_name_input.as_str(),
        app.model_name_cursor_offset(),
        chunks[1],
    ))
}

pub(super) fn render_openai_profile_label_editor_modal(
    f: &mut Frame,
    app: &TuiApp,
    area: Rect,
) -> Option<(u16, u16)> {
    let kind = app
        .selected_openai_profile_kind()
        .unwrap_or(crate::config::OpenAiEndpointKind::Custom);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);
    let intro = Paragraph::new(format!(
        "Create a new {} endpoint profile.\nThis label is only used locally in the picker and status surfaces.",
        kind.label()
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" New Endpoint Profile "),
    );
    let editor = Paragraph::new(app.openai_profile_label_input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Label "));
    let footer = Paragraph::new("Enter create  Esc back to profiles").alignment(Alignment::Center);
    f.render_widget(intro, chunks[0]);
    f.render_widget(editor, chunks[1]);
    f.render_widget(footer, chunks[2]);
    Some(editor_cursor_position(
        app.openai_profile_label_input.as_str(),
        app.openai_profile_label_cursor_offset(),
        chunks[1],
    ))
}
