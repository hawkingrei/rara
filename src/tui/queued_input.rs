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
    "queued follow-up pending  will submit after the next tool/result boundary"
}

pub fn queued_follow_up_hint() -> &'static str {
    "queued follow-up pending  current task will finish before submission"
}
