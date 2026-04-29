use crossterm::event::{Event, KeyEventKind, MouseEvent, MouseEventKind};

use super::app_event::AppEvent;
use super::state::TuiApp;

const MOUSE_WHEEL_SCROLL_LINES: i32 = 3;

#[derive(Debug)]
pub enum UiEvent {
    App(AppEvent),
    Draw,
    Paste(String),
    FocusChanged(bool),
}

pub fn translate_event(event: Event, app: &TuiApp) -> Option<UiEvent> {
    match event {
        Event::Key(key_event) => {
            if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                Some(UiEvent::App(super::map_key_to_event(key_event, app)))
            } else {
                None
            }
        }
        Event::Mouse(mouse_event) => Some(UiEvent::App(map_mouse_to_event(mouse_event, app))),
        Event::Resize(_, _) => Some(UiEvent::Draw),
        Event::Paste(text) => Some(UiEvent::Paste(text)),
        Event::FocusGained => Some(UiEvent::FocusChanged(true)),
        Event::FocusLost => Some(UiEvent::FocusChanged(false)),
    }
}

fn map_mouse_to_event(mouse_event: MouseEvent, app: &TuiApp) -> AppEvent {
    if app.overlay.is_some() {
        return AppEvent::Noop;
    }

    match mouse_event.kind {
        MouseEventKind::ScrollUp => AppEvent::ScrollTranscript(-MOUSE_WHEEL_SCROLL_LINES),
        MouseEventKind::ScrollDown => AppEvent::ScrollTranscript(MOUSE_WHEEL_SCROLL_LINES),
        _ => AppEvent::Noop,
    }
}
