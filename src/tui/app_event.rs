use super::state::{HelpTab, Overlay};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Noop,
    OpenOverlay(Overlay),
    CloseOverlay,
    SubmitComposer,
    InputChar(char),
    Backspace,
    MoveCommandSelection(i32),
    MoveGuideSelection(i32),
    MoveProviderSelection(i32),
    MoveModelSelection(i32),
    SetGuideSelection(usize),
    SetProviderSelection(usize),
    SetModelSelection(usize),
    CycleModelSelection,
    SelectHelpTab(HelpTab),
    StartOAuth,
    ApplyOverlaySelection,
}
