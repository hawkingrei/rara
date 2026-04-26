use std::io;

use anyhow::Result;
use crossterm::{
    cursor::Show,
    execute,
    terminal::disable_raw_mode,
};
use ratatui::{backend::CrosstermBackend, layout::Rect, text::Line};

use super::custom_terminal::Terminal;
use super::insert_history::insert_history_lines;
use super::render::committed_turn_lines;
use super::state::TuiApp;

pub(super) fn handle_paste(text: String, app: &mut TuiApp) {
    app.insert_active_input_text(text.as_str());
}

pub(super) fn build_terminal(
    viewport_height: u16,
) -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let size = terminal.size()?;
    terminal.set_viewport_area(viewport_area(size.width, size.height, viewport_height));
    terminal.clear_visible_screen()?;
    Ok(terminal)
}

pub(super) fn update_terminal_viewport(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    viewport_height: u16,
) -> Result<()> {
    let size = terminal.size()?;
    let area = viewport_area(size.width, size.height, viewport_height);
    if area != terminal.viewport_area {
        terminal.clear_visible_screen()?;
        terminal.set_viewport_area(area);
    }
    Ok(())
}

pub(super) fn teardown_terminal(
    mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), Show)?;
    terminal.show_cursor()?;
    Ok(())
}

pub(super) fn flush_committed_history(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut TuiApp,
) -> Result<()> {
    while app.inserted_turns < app.committed_turns.len() {
        let turn = &app.committed_turns[app.inserted_turns];
        let cwd =
            (!app.snapshot.cwd.is_empty()).then(|| std::path::Path::new(app.snapshot.cwd.as_str()));
        let width = terminal.size()?.width;
        let mut lines = committed_turn_lines(turn.entries.as_slice(), cwd, width);
        if app.inserted_turns > 0 && !lines.is_empty() {
            lines.insert(0, Line::from(""));
        }
        if !lines.is_empty() {
            insert_history_lines(terminal, lines)?;
        }
        app.inserted_turns += 1;
    }
    Ok(())
}

fn viewport_area(width: u16, height: u16, viewport_height: u16) -> Rect {
    let viewport_height = viewport_height.max(1).min(height.max(1));
    Rect::new(
        0,
        height.saturating_sub(viewport_height),
        width,
        viewport_height,
    )
}

pub(crate) fn is_ssh_session() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
}
