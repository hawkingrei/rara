/// Message role enum replacing bare string comparisons like `role == "You"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MessageRole {
    User,
    Agent,
    System,
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
    pub(crate) fn from_str(role: &str) -> Self {
        match role {
            "You" => Self::User,
            "Agent" => Self::Agent,
            "System" => Self::System,
            "Tool" => Self::Tool,
            "Tool Result" => Self::ToolResult,
            "Tool Error" => Self::ToolError,
            "Tool Progress" => Self::ToolProgress,
            "Exploring" => Self::Exploring,
            "Planning" => Self::Planning,
            "Running" => Self::Running,
            "Thinking" => Self::Thinking,
            _ => panic!("unknown message role: {role}"),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Agent => "Agent",
            Self::System => "System",
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
}
