/// Message role enum replacing bare string comparisons like `role == "You"`.
/// Message roles used in transcript entries and render dispatch.
///
/// Centralizes the string-to-variant mapping to eliminate ~134 bare
/// `role == "You"` scattered across the TUI layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MessageRole {
    User,
    Agent,
    System,
    Runtime,
    Responding,
    Tool,
    ToolResult,
    ToolError,
    ToolProgress,
    Exploring,
    Planning,
    Running,
    Thinking,
}

impl MessageRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Agent => "Agent",
            Self::System => "System",
            Self::Runtime => "Runtime",
            Self::Responding => "Responding",
            Self::Tool => "Tool",
            Self::ToolResult => "Tool Result",
            Self::ToolError => "Tool Error",
            Self::ToolProgress => "Tool Progress",
            Self::Exploring => "Exploring",
            Self::Planning => "Planning",
            Self::Running => "Running",
            Self::Thinking => "Thinking",
        }
    }

    pub(crate) fn try_from_str(role: &str) -> Option<Self> {
        match role {
            "You" => Some(Self::User),
            "Agent" => Some(Self::Agent),
            "System" => Some(Self::System),
            "Runtime" => Some(Self::Runtime),
            "Responding" => Some(Self::Responding),
            "Tool" => Some(Self::Tool),
            "Tool Result" => Some(Self::ToolResult),
            "Tool Error" => Some(Self::ToolError),
            "Tool Progress" => Some(Self::ToolProgress),
            "Exploring" => Some(Self::Exploring),
            "Planning" => Some(Self::Planning),
            "Running" => Some(Self::Running),
            "Thinking" => Some(Self::Thinking),
            _ => None,
        }
    }
}
