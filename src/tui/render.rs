use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::command::{
    api_key_status, help_text, matching_commands, model_help_text,
};
use super::state::{TuiApp, TaskKind, LOCAL_MODEL_PRESETS};

pub fn render_chat(f: &mut Frame, app: &TuiApp) {
    let show_command_menu = app.input.trim_start().starts_with('/');
    let command_height = if show_command_menu { 6 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(command_height),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);

    if app.transcript.is_empty() {
        f.render_widget(
            Paragraph::new(onboarding_text(app))
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT).title(" Session "))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
    } else {
        let items = app
            .transcript
            .iter()
            .map(|(role, message)| {
                ListItem::new(vec![
                    Line::from(role.as_str().bold()),
                    Line::from(message.as_str()),
                    Line::from(""),
                ])
            })
            .collect::<Vec<_>>();
        f.render_widget(
            List::new(items).block(Block::default().borders(Borders::LEFT | Borders::RIGHT)),
            chunks[1],
        );
    }

    if show_command_menu {
        render_command_menu(f, app, chunks[2]);
    }

    let input_title = if app.is_busy() {
        " Running "
    } else {
        " Prompt (/help for commands) "
    };
    let input_index = if show_command_menu { 3 } else { 2 };
    let footer_index = if show_command_menu { 4 } else { 3 };
    f.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(input_title)),
        chunks[input_index],
    );

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
    let status = format!(
        "state={}  mode={}  key={}  messages={}  tokens={} in / {} out{}",
        activity,
        mode,
        api_key_status(&app.config),
        app.snapshot.history_len,
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        app.notice
            .as_ref()
            .map(|notice| format!("  notice={notice}"))
            .unwrap_or_default()
    );
    f.render_widget(Paragraph::new(status), chunks[footer_index]);
}

pub fn render_setup(f: &mut Frame, app: &TuiApp) {
    let block = Block::default().borders(Borders::ALL).title(" RARA Setup ");
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

    let status_suffix = app
        .setup_status
        .as_ref()
        .map(|status| format!("\nStatus: {status}"))
        .unwrap_or_default();
    let busy_suffix = if app.is_busy() {
        "\nA background task is running. Setup changes will apply when it finishes."
    } else {
        ""
    };
    let text = format!(
        "Provider: {}\nModel: {}\nAPI key: {}\nRevision: {}\n\n\
         Presets:\n{}\n\n\
         [1/2/3] Select preset\n[M] Cycle preset\n[Enter] Apply and rebuild\n[L] OAuth login\n[Esc] Back to chat{}\n{}\n",
        app.config.provider.clone().bold().yellow(),
        app.current_model_label().bold().cyan(),
        api_key_status(&app.config),
        app.config.revision.as_deref().unwrap_or("main"),
        preset_lines,
        status_suffix,
        busy_suffix,
    );
    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false }),
        f.area(),
    );
}

pub fn render_model_picker(f: &mut Frame, app: &TuiApp) {
    let items = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let prefix = if idx == app.model_picker_idx { ">" } else { " " };
            let current =
                if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                    " current"
                } else {
                    ""
                };
            ListItem::new(format!(
                "{prefix} [{}] {} ({}/{}){}",
                idx + 1,
                label,
                provider,
                model,
                current
            ))
        })
        .collect::<Vec<_>>();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(3)])
        .split(f.area());
    let header = Paragraph::new("Select a local model preset. Enter applies it immediately.")
        .block(Block::default().borders(Borders::ALL).title(" Model Picker "));
    let footer = Paragraph::new("Use Up/Down or j/k. Enter apply. Esc cancel.")
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, layout[0]);
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL)),
        layout[1],
    );
    f.render_widget(footer, layout[2]);
}

fn render_header(f: &mut Frame, app: &TuiApp, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled(" RARA ", Style::default().bg(Color::Cyan).fg(Color::Black).bold()),
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

fn render_command_menu(f: &mut Frame, app: &TuiApp, area: Rect) {
    let query = app.input.trim_start().trim_start_matches('/');
    let items = matching_commands(query)
        .into_iter()
        .map(|spec| ListItem::new(format!("{}  {}", spec.usage, spec.summary)))
        .collect::<Vec<_>>();
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" Commands ")),
        area,
    );
}

fn onboarding_text(app: &TuiApp) -> String {
    let local_model_help = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, _, model))| format!("  {}. {} ({})", idx + 1, label, model))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "This TUI uses chat as the primary control surface.\n\n\
         Start here:\n\
           - Ask for a task directly.\n\
           - Use /help for built-in commands.\n\
           - Use /model to open the model picker or switch presets.\n\
           - Use /status to inspect runtime state.\n\
           - Use /clear to reset the local transcript view.\n\n\
         Local presets:\n{}\n\n\
         Current runtime:\n\
           provider={}\n\
           model={}\n\
           revision={}\n\
           workspace={}\n\
           branch={}\n\n\
         Quick hints:\n{}\n{}\n",
        local_model_help,
        app.config.provider,
        app.current_model_label(),
        app.config.revision.as_deref().unwrap_or("main"),
        app.snapshot.cwd,
        app.snapshot.branch,
        help_text(),
        model_help_text(app),
    )
}
