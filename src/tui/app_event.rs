use super::state::{HelpTab, Overlay};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Noop,
    Quit,
    OpenOverlay(Overlay),
    CloseOverlay,
    SubmitComposer,
    InputChar(char),
    Backspace,
    MoveCommandSelection(i32),
    MoveGuideSelection(i32),
    MoveModelSelection(i32),
    SetGuideSelection(usize),
    SetModelSelection(usize),
    CycleModelSelection,
    SelectHelpTab(HelpTab),
    StartOAuth,
    ApplyOverlaySelection,
}
