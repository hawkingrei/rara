pub(crate) fn compact_delegate_rest(rest: &str) -> Option<String> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(rest) {
        if let Some(name) = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let instruction = value
                .get("instruction")
                .and_then(serde_json::Value::as_str)
                .map(compact_instruction)
                .unwrap_or_else(|| "instruction unavailable".to_string());
            return Some(format!("{name}: {instruction}"));
        }
        return value
            .get("instruction")
            .and_then(serde_json::Value::as_str)
            .map(compact_instruction);
    }
    Some(compact_instruction(rest))
}

pub(crate) fn compact_instruction(instruction: &str) -> String {
    const MAX_CHARS: usize = 120;
    let normalized = instruction.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }
    let mut truncated = normalized.chars().take(MAX_CHARS).collect::<String>();
    truncated.push('…');
    truncated
}
