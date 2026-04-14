use crossterm::event::{Event, KeyEventKind};

use super::app_event::AppEvent;
use super::state::TuiApp;

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
                Some(UiEvent::App(super::map_key_to_event(key_event.code, app)))
            } else {
                None
            }
        }
        Event::Resize(_, _) => Some(UiEvent::Draw),
        Event::Paste(text) => Some(UiEvent::Paste(text)),
        Event::FocusGained => Some(UiEvent::FocusChanged(true)),
        Event::FocusLost => Some(UiEvent::FocusChanged(false)),
        _ => None,
    }
}
