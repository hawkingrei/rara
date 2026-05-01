#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFollowUpMessage {
    pub text: String,
    pub release_after_boundary: u64,
}

pub fn pending_follow_up_heading() -> &'static str {
    "Messages to be submitted after next tool call"
}

pub fn queued_follow_up_heading() -> &'static str {
    "Queued follow-up messages"
}

pub fn pending_follow_up_hint() -> &'static str {
    "pending follow-up  will submit after next tool call"
}

pub fn queued_follow_up_hint() -> &'static str {
    "queued follow-up  will submit after current turn"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedFollowUpSection {
    pub title: &'static str,
    pub preview: String,
    pub remaining: usize,
    pub remaining_label: &'static str,
}

pub fn queued_follow_up_sections(
    pending_preview: Option<&str>,
    pending_count: usize,
    queued_preview: Option<&str>,
    queued_count: usize,
) -> Vec<QueuedFollowUpSection> {
    let mut sections = Vec::new();
    if let Some(preview) = pending_preview
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(QueuedFollowUpSection {
            title: pending_follow_up_heading(),
            preview: preview.to_string(),
            remaining: pending_count.saturating_sub(1),
            remaining_label: "more pending",
        });
    }
    if let Some(preview) = queued_preview
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(QueuedFollowUpSection {
            title: queued_follow_up_heading(),
            preview: preview.to_string(),
            remaining: queued_count.saturating_sub(1),
            remaining_label: "more queued",
        });
    }
    sections
}
