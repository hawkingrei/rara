use crate::defaults::{
    DEFAULT_REASONING_SUMMARY, REASONING_SUMMARY_NONE,
};
use crate::serde_helpers::normalize_reasoning_summary;

pub fn migrate_reasoning_summary(
    reasoning_summary: Option<String>,
    legacy_thinking: Option<bool>,
) -> Option<String> {
    normalize_reasoning_summary(reasoning_summary).or_else(|| {
        Some(match legacy_thinking {
            Some(false) => REASONING_SUMMARY_NONE.to_string(),
            Some(true) | None => DEFAULT_REASONING_SUMMARY.to_string(),
        })
    })
}
