use crossterm::event::KeyCode;

use super::app_event::AppEvent;
use super::state::{HelpTab, Overlay, ProviderFamily, TuiApp};

pub(super) fn map_key_to_event(key: KeyCode, app: &TuiApp) -> AppEvent {
    match app.overlay {
        Some(Overlay::Help(_)) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Char('1') => AppEvent::SelectHelpTab(HelpTab::General),
            KeyCode::Char('2') => AppEvent::SelectHelpTab(HelpTab::Commands),
            KeyCode::Char('3') => AppEvent::SelectHelpTab(HelpTab::Runtime),
            _ => AppEvent::Noop,
        },
        Some(Overlay::CommandPalette) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveCommandSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveCommandSelection(1),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::Status) => match key {
            KeyCode::Esc | KeyCode::Enter => AppEvent::CloseOverlay,
            _ => AppEvent::Noop,
        },
        Some(Overlay::Context) => match key {
            KeyCode::Esc | KeyCode::Enter => AppEvent::CloseOverlay,
            _ => AppEvent::Noop,
        },
        Some(Overlay::Setup) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Char('1') => AppEvent::SetModelSelection(0),
            KeyCode::Char('2') => AppEvent::SetModelSelection(1),
            KeyCode::Char('3') => AppEvent::SetModelSelection(2),
            KeyCode::Char('4') => AppEvent::SetModelSelection(3),
            KeyCode::Char('5') => AppEvent::SetModelSelection(4),
            KeyCode::Char('6') => AppEvent::SetModelSelection(5),
            KeyCode::Char('7') => AppEvent::SetModelSelection(6),
            KeyCode::Char('8') => AppEvent::SetModelSelection(7),
            KeyCode::Char('9') => AppEvent::SetModelSelection(8),
            KeyCode::Char('m') => AppEvent::CycleModelSelection,
            KeyCode::Char('l')
                if matches!(app.selected_provider_family(), ProviderFamily::Codex) =>
            {
                AppEvent::OpenOverlay(Overlay::AuthModePicker)
            }
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ProviderPicker) => match key {
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
        Some(Overlay::ResumePicker) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Up | KeyCode::Char('k') => AppEvent::MoveResumeSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppEvent::MoveResumeSelection(1),
            KeyCode::Char('1') => AppEvent::SetResumeSelection(0),
            KeyCode::Char('2') => AppEvent::SetResumeSelection(1),
            KeyCode::Char('3') => AppEvent::SetResumeSelection(2),
            KeyCode::Enter => AppEvent::ApplyOverlaySelection,
            _ => AppEvent::Noop,
        },
        Some(Overlay::ModelPicker) => match key {
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
        Some(Overlay::AuthModePicker) => match key {
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
        Some(Overlay::BaseUrlEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveBaseUrlInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ApiKeyEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveApiKeyInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ModelNameEditor) => match key {
            KeyCode::Esc => AppEvent::CloseOverlay,
            KeyCode::Enter => AppEvent::SaveModelNameInput,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
        Some(Overlay::ReasoningEffortPicker) => match key {
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
        None => match key {
            KeyCode::Esc => AppEvent::Noop,
            KeyCode::Enter => AppEvent::SubmitComposer,
            KeyCode::Up | KeyCode::Char('k') if app.input.is_empty() => {
                AppEvent::ScrollTranscript(-1)
            }
            KeyCode::Down | KeyCode::Char('j') if app.input.is_empty() => {
                AppEvent::ScrollTranscript(1)
            }
            KeyCode::PageUp if app.input.is_empty() => AppEvent::ScrollTranscript(-8),
            KeyCode::PageDown if app.input.is_empty() => AppEvent::ScrollTranscript(8),
            KeyCode::Char('1')
                if app.input.is_empty()
                    && (app.active_pending_interaction().is_some()
                        || app.has_pending_planning_suggestion()) =>
            {
                AppEvent::SelectPendingOption(0)
            }
            KeyCode::Char('2')
                if app.input.is_empty()
                    && (app.active_pending_interaction().is_some()
                        || app.has_pending_planning_suggestion()) =>
            {
                AppEvent::SelectPendingOption(1)
            }
            KeyCode::Char('3')
                if app.input.is_empty()
                    && app.active_pending_interaction().is_some_and(|pending| {
                        pending.kind != super::state::ActivePendingInteractionKind::PlanApproval
                    }) =>
            {
                AppEvent::SelectPendingOption(2)
            }
            KeyCode::Char('s') if app.input.is_empty() => AppEvent::OpenOverlay(Overlay::Setup),
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Char(c) => AppEvent::InputChar(c),
            _ => AppEvent::Noop,
        },
    }
}
