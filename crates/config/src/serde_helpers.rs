use crate::defaults::{
    DEFAULT_REASONING_SUMMARY, REASONING_SUMMARY_DETAILED, REASONING_SUMMARY_NONE,
};

pub fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub fn normalize_reasoning_summary(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" => None,
            DEFAULT_REASONING_SUMMARY | REASONING_SUMMARY_NONE | REASONING_SUMMARY_DETAILED => {
                Some(normalized)
            }
            _ => Some(DEFAULT_REASONING_SUMMARY.to_string()),
        }
    })
}
