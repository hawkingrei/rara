pub(crate) fn cache_hit_rate_label(hit_tokens: u32, miss_tokens: u32) -> Option<String> {
    let total = hit_tokens.saturating_add(miss_tokens);
    (total > 0).then(|| format!("{:.1}%", hit_tokens as f64 * 100.0 / total as f64))
}
