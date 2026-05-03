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
    "queued: after tool"
}

pub fn queued_follow_up_hint() -> &'static str {
    "queued: after turn"
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

#[cfg(test)]
mod tests {
    use super::{pending_follow_up_heading, queued_follow_up_heading, queued_follow_up_sections};

    #[test]
    fn queued_follow_up_sections_include_pending_and_end_of_turn_messages() {
        let sections =
            queued_follow_up_sections(Some("first follow-up"), 2, Some("second follow-up"), 3);

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].title, pending_follow_up_heading());
        assert_eq!(sections[0].preview, "first follow-up");
        assert_eq!(sections[0].remaining, 1);
        assert_eq!(sections[0].remaining_label, "more pending");
        assert_eq!(sections[1].title, queued_follow_up_heading());
        assert_eq!(sections[1].preview, "second follow-up");
        assert_eq!(sections[1].remaining, 2);
        assert_eq!(sections[1].remaining_label, "more queued");
    }

    #[test]
    fn queued_follow_up_sections_skip_empty_previews() {
        let sections = queued_follow_up_sections(Some("  "), 2, Some("queued"), 1);

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, queued_follow_up_heading());
        assert_eq!(sections[0].preview, "queued");
        assert_eq!(sections[0].remaining, 0);
    }
}
