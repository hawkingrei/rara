use super::state::{HelpTab, Overlay};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Noop,
    OpenOverlay(Overlay),
    CloseOverlay,
    SubmitComposer,
    InputChar(char),
    Backspace,
    ScrollTranscript(i32),
    MoveCommandSelection(i32),
    MoveGuideSelection(i32),
    MoveProviderSelection(i32),
    MoveModelSelection(i32),
    SetGuideSelection(usize),
    SetProviderSelection(usize),
    SetModelSelection(usize),
    SelectPendingOption(usize),
    CycleModelSelection,
    SaveBaseUrlInput,
    SelectHelpTab(HelpTab),
    StartOAuth,
    ApplyOverlaySelection,
}
