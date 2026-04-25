use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app_event::AppEvent;
use super::state::{HelpTab, Overlay, ProviderFamily, TuiApp};

pub(super) fn map_key_to_event(key: KeyEvent, app: &TuiApp) -> AppEvent {
    let code = key.code;
    let modifiers = key.modifiers;
    match app.overlay {
        Some(Overlay::Help(_)) => match key {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => AppEvent::CloseOverlay,
            KeyEvent {
                code: KeyCode::Char('1'),
                ..
            } => AppEvent::SelectHelpTab(HelpTab::General),
            KeyEvent {
                code: KeyCode::Char('2'),
                ..
            } => AppEvent::SelectHelpTab(HelpTab::Commands),
            KeyEvent {
                code: KeyCode::Char('3'),
                ..
            } => AppEvent::SelectHelpTab(HelpTab::Runtime),
            _ => AppEvent::Noop,
        },
        Some(Overlay::CommandPalette) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveCommandSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveCommandSelection(1),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::Status) => match code {
            KeyCode::Esc | KeyCode::Enter => AppEvent::CloseOverlay,
            _ => AppEvent::Noop,
        },
        Some(Overlay::Context) => match code {
            KeyCode::Esc | KeyCode::Enter => AppEvent::CloseOverlay,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ProviderPicker) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveProviderSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveProviderSelection(1),
            KeyCode::Char('1') => AppEvent::SetProviderSelection(0),
            KeyCode::Char('2') => AppEvent::SetProviderSelection(1),
            KeyCode::Char('3') => AppEvent::SetProviderSelection(2),
            KeyCode::Char('4') => AppEvent::SetProviderSelection(3),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ResumePicker) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveResumeSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveResumeSelection(1),
            KeyCode::Char('1') => AppEvent::SetResumeSelection(0),
            KeyCode::Char('2') => AppEvent::SetResumeSelection(1),
            KeyCode::Char('3') => AppEvent::SetResumeSelection(2),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ModelPicker) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveModelSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveModelSelection(1),
            KeyCode::Char('1') => AppEvent::SetModelSelection(0),
            KeyCode::Char('2') => AppEvent::SetModelSelection(1),
            KeyCode::Char('3') => AppEvent::SetModelSelection(2),
            KeyCode::Char('4') => AppEvent::SetModelSelection(3),
            KeyCode::Char('5') => AppEvent::SetModelSelection(4),
            KeyCode::Char('6') => AppEvent::SetModelSelection(5),
            KeyCode::Char('7') => AppEvent::SetModelSelection(6),
            KeyCode::Char('8') => AppEvent::SetModelSelection(7),
            KeyCode::Char('9') => AppEvent::SetModelSelection(8),
            KeyCode::Char('b')
                if matches!(
                    app.selected_provider_family(),
                    ProviderFamily::OpenAiCompatible | ProviderFamily::Ollama
                ) =>
            {
                AppEvent::OpenOverlay(Overlay::BaseUrlEditor)
            }
            KeyCode::Char('a')
                if matches!(
                    app.selected_provider_family(),
                    ProviderFamily::OpenAiCompatible
                ) =>
            {
                AppEvent::OpenOverlay(Overlay::ApiKeyEditor)
            }
            KeyCode::Char('n')
                if matches!(
                    app.selected_provider_family(),
                    ProviderFamily::OpenAiCompatible
                ) =>
            {
                AppEvent::OpenOverlay(Overlay::ModelNameEditor)
            }
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::AuthModePicker) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveAuthModeSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveAuthModeSelection(1),
            KeyCode::Char('1') => AppEvent::SetAuthModeSelection(0),
            KeyCode::Char('2') => AppEvent::SetAuthModeSelection(1),
            KeyCode::Char('3') => AppEvent::SetAuthModeSelection(2),
            KeyCode::Char('4') => AppEvent::SetAuthModeSelection(3),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::BaseUrlEditor) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveBaseUrlInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ApiKeyEditor) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveApiKeyInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ModelNameEditor) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveModelNameInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ReasoningEffortPicker) => match code {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveReasoningEffortSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveReasoningEffortSelection(1),
            KeyCode::Char('1') => AppEvent::SetReasoningEffortSelection(0),
            KeyCode::Char('2') => AppEvent::SetReasoningEffortSelection(1),
            KeyCode::Char('3') => AppEvent::SetReasoningEffortSelection(2),
            KeyCode::Char('4') => AppEvent::SetReasoningEffortSelection(3),
            KeyCode::Char('5') => AppEvent::SetReasoningEffortSelection(4),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        None => match (code, modifiers) {
            (KeyCode::Esc, _) => AppEvent::Noop,
            (KeyCode::Enter, KeyModifiers::SHIFT) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                AppEvent::InsertNewline
            }
            (KeyCode::Enter, _) => AppEvent::SubmitComposer,
            (KeyCode::Up | KeyCode::Char('k'), _) if app.input.is_empty() => {
                AppEvent::ScrollTranscript(-1)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) if app.input.is_empty() => {
                AppEvent::ScrollTranscript(1)
            }
            (KeyCode::PageUp, _) if app.input.is_empty() => AppEvent::ScrollTranscript(-8),
            (KeyCode::PageDown, _) if app.input.is_empty() => AppEvent::ScrollTranscript(8),
            (KeyCode::Char('1'), _)
                if app.input.is_empty()
                    && (app.active_pending_interaction().is_some()
                        || app.has_pending_planning_suggestion()) =>
            {
                AppEvent::SelectPendingOption(0)
            }
            (KeyCode::Char('2'), _)
                if app.input.is_empty()
                    && (app.active_pending_interaction().is_some()
                        || app.has_pending_planning_suggestion()) =>
            {
                AppEvent::SelectPendingOption(1)
            }
            (KeyCode::Char('3'), _)
                if app.input.is_empty()
                    && app.active_pending_interaction().is_some_and(|pending| {
                        pending.kind != super::state::ActivePendingInteractionKind::PlanApproval
                    }) =>
            {
                AppEvent::SelectPendingOption(2)
            }
            (KeyCode::Backspace, _) => AppEvent::Backspace,
            (KeyCode::Char(c), _) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
    }
}
