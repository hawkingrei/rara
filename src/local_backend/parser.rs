use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(super) struct ToolAwareReply {
    pub kind: Option<String>,
    pub text: Option<String>,
    pub calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

pub(super) fn parse_tool_aware_reply(raw: &str) -> Result<ToolAwareReply> {
    let payload = extract_json_object(raw).unwrap_or(raw).trim();
    serde_json::from_str(payload).context("parse local model JSON reply")
}

pub(super) fn extract_json_object(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return raw.get(start..=idx);
                }
            }
            _ => {}
        }
    }

    None
}
