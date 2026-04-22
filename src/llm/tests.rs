use serde_json::json;

use crate::agent::Message;

use super::ollama::{
    apply_ollama_stream_event, build_ollama_options, ensure_ollama_stream_completed,
    suggest_ollama_num_ctx, to_ollama_messages,
};
use super::openai_compatible::{parse_codex_response, to_codex_input_items, to_openai_messages};
use super::shared::{
    extract_message_text, model_context_budget, parse_tool_arguments, should_bypass_proxy,
};

#[test]
fn converts_assistant_tool_history_to_openai_messages() {
    let messages = vec![
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"text","text":"Need a tool."},
                {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"Cargo.toml"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"[package]"}
            ]),
        },
    ];

    let openai_messages = to_openai_messages(&messages);
    assert_eq!(openai_messages[0]["role"], "assistant");
    assert_eq!(
        openai_messages[0]["tool_calls"][0]["function"]["name"],
        "read_file"
    );
    assert_eq!(openai_messages[1]["role"], "tool");
    assert_eq!(openai_messages[1]["tool_call_id"], "tool-1");
}

#[test]
fn converts_history_to_codex_responses_input_items() {
    let messages = vec![
        Message {
            role: "system".to_string(),
            content: json!("Follow the repo rules."),
        },
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"text","text":"Need a tool."},
                {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"Cargo.toml"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"[package]"}
            ]),
        },
    ];

    let input = to_codex_input_items(&messages);
    assert_eq!(input[0]["type"], "message");
    assert_eq!(input[0]["role"], "system");
    assert_eq!(input[0]["content"][0]["type"], "input_text");
    assert_eq!(input[1]["type"], "message");
    assert_eq!(input[1]["role"], "assistant");
    assert_eq!(input[1]["content"][0]["type"], "output_text");
    assert_eq!(input[2]["type"], "function_call");
    assert_eq!(input[2]["call_id"], "tool-1");
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "tool-1");
    assert_eq!(input[3]["output"], "[package]");
}

#[test]
fn preserves_mixed_user_text_and_multiple_tool_results_for_codex_inputs() {
    let messages = vec![Message {
        role: "user".to_string(),
        content: json!([
            {"type":"text","text":"First result follows."},
            {"type":"tool_result","tool_use_id":"tool-1","content":"alpha"},
            {"type":"text","text":"Second result follows."},
            {"type":"tool_result","tool_use_id":"tool-2","content":"beta"}
        ]),
    }];

    let input = to_codex_input_items(&messages);
    assert_eq!(input.len(), 4);
    assert_eq!(input[0]["type"], "message");
    assert_eq!(input[0]["content"][0]["text"], "First result follows.");
    assert_eq!(input[1]["type"], "function_call_output");
    assert_eq!(input[1]["call_id"], "tool-1");
    assert_eq!(input[1]["output"], "alpha");
    assert_eq!(input[2]["type"], "message");
    assert_eq!(input[2]["content"][0]["text"], "Second result follows.");
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "tool-2");
    assert_eq!(input[3]["output"], "beta");
}

#[test]
fn parses_codex_responses_output_into_text_and_tool_use_blocks() {
    let response = parse_codex_response(&json!({
        "status": "completed",
        "usage": {
            "input_tokens": 11,
            "output_tokens": 7
        },
        "output": [
            {
                "type": "message",
                "content": [
                    {"type": "output_text", "text": "Need to inspect a file."}
                ]
            },
            {
                "type": "function_call",
                "call_id": "call-1",
                "name": "read_file",
                "arguments": "{\"path\":\"Cargo.toml\"}"
            }
        ]
    }))
    .unwrap();

    assert_eq!(response.stop_reason, Some("completed".to_string()));
    assert_eq!(response.usage.unwrap().input_tokens, 11);
    assert_eq!(response.content.len(), 2);
    match &response.content[0] {
        crate::agent::ContentBlock::Text { text } => {
            assert_eq!(text, "Need to inspect a file.");
        }
        other => panic!("expected text block, got {other:?}"),
    }
    match &response.content[1] {
        crate::agent::ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "call-1");
            assert_eq!(name, "read_file");
            assert_eq!(input, &json!({"path":"Cargo.toml"}));
        }
        other => panic!("expected tool_use block, got {other:?}"),
    }
}

#[test]
fn parses_tool_arguments_from_string_and_object() {
    assert_eq!(
        parse_tool_arguments(&json!("{\"path\":\"Cargo.toml\"}")).unwrap(),
        json!({"path":"Cargo.toml"})
    );
    assert_eq!(
        parse_tool_arguments(&json!({"path":"Cargo.toml"})).unwrap(),
        json!({"path":"Cargo.toml"})
    );
}

#[test]
fn extracts_text_from_openai_content_array() {
    assert_eq!(
        extract_message_text(Some(&json!([
            {"type":"text","text":"hello"},
            {"type":"text","text":"world"}
        ]))),
        Some("hello\n\nworld".to_string())
    );
}

#[test]
fn converts_tool_history_to_ollama_messages() {
    let messages = vec![
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"text","text":"Need a tool."},
                {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"Cargo.toml"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"[package]"}
            ]),
        },
    ];

    let ollama_messages = to_ollama_messages(&messages);
    assert_eq!(ollama_messages[0]["role"], "assistant");
    assert_eq!(
        ollama_messages[0]["tool_calls"][0]["function"]["name"],
        "read_file"
    );
    assert_eq!(ollama_messages[1]["role"], "tool");
    assert_eq!(ollama_messages[1]["tool_name"], "read_file");
}

#[test]
fn suggests_larger_num_ctx_for_plan_and_tool_heavy_turns() {
    let messages = vec![
        Message {
            role: "system".to_string(),
            content: json!("<plan>\n- [pending] Inspect src/\n</plan>"),
        },
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-1","name":"list_files","input":{"path":"src"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"src/main.rs\nsrc/agent.rs"}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!("<agent_runtime>\nphase: tool_results_available\n</agent_runtime>"),
        },
    ];

    assert_eq!(suggest_ollama_num_ctx(&messages, &[], true), Some(32768));
}

#[test]
fn prefers_explicit_num_ctx_override_for_ollama_options() {
    let messages = vec![Message {
        role: "user".to_string(),
        content: json!("hello"),
    }];

    let options = build_ollama_options(&messages, &[], true, Some(65536)).unwrap();
    assert_eq!(options["num_ctx"], json!(65536));
}

#[test]
fn derives_context_budget_for_codex_like_models() {
    let budget = model_context_budget("gpt-5.1-codex").expect("budget");
    assert_eq!(budget.context_window_tokens, 200_000);
    assert!(budget.compact_threshold_tokens < budget.context_window_tokens);
}

#[test]
fn applies_ollama_stream_event_deltas_and_tool_calls() {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut stop_reason = None;
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut deltas = Vec::new();

    let done = apply_ollama_stream_event(
        &json!({"message":{"content":"Hello"}}),
        &mut text,
        &mut tool_calls,
        &mut stop_reason,
        &mut input_tokens,
        &mut output_tokens,
        &mut |delta| deltas.push(delta),
    )
    .unwrap();
    assert!(!done);

    let done = apply_ollama_stream_event(
        &json!({
            "message":{
                "content":" world",
                "tool_calls":[{"function":{"name":"read_file","arguments":{"path":"Cargo.toml"}}}]
            },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 12,
            "eval_count": 6
        }),
        &mut text,
        &mut tool_calls,
        &mut stop_reason,
        &mut input_tokens,
        &mut output_tokens,
        &mut |delta| deltas.push(delta),
    )
    .unwrap();
    assert!(done);

    assert_eq!(text, "Hello world");
    assert_eq!(deltas, vec!["Hello".to_string(), " world".to_string()]);
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].name, "read_file");
    assert_eq!(tool_calls[0].arguments, json!({"path":"Cargo.toml"}));
    assert_eq!(stop_reason, Some("stop".to_string()));
    assert_eq!(input_tokens, 12);
    assert_eq!(output_tokens, 6);
}

#[test]
fn rejects_ollama_streams_without_final_done_event() {
    let error = ensure_ollama_stream_completed(false, "http://localhost:11434/api/chat")
        .expect_err("missing done event should fail");
    assert!(error
        .to_string()
        .contains("ended before the final done event"));
}

#[test]
fn deduplicates_repeated_ollama_stream_tool_calls() {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut stop_reason = None;
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;

    apply_ollama_stream_event(
        &json!({
            "message":{
                "tool_calls":[{"function":{"name":"read_file","arguments":{"path":"Cargo.toml"}}}]
            }
        }),
        &mut text,
        &mut tool_calls,
        &mut stop_reason,
        &mut input_tokens,
        &mut output_tokens,
        &mut |_| {},
    )
    .unwrap();

    apply_ollama_stream_event(
        &json!({
            "message":{
                "tool_calls":[{"function":{"name":"read_file","arguments":{"path":"Cargo.toml"}}}]
            }
        }),
        &mut text,
        &mut tool_calls,
        &mut stop_reason,
        &mut input_tokens,
        &mut output_tokens,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].name, "read_file");
    assert_eq!(tool_calls[0].arguments, json!({"path":"Cargo.toml"}));
}

#[test]
fn ignores_incomplete_ollama_stream_tool_calls_until_arguments_are_complete() {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut stop_reason = None;
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;

    apply_ollama_stream_event(
        &json!({
            "message":{
                "tool_calls":[{"function":{"name":"read_file","arguments":"{\"path\":"}}]
            }
        }),
        &mut text,
        &mut tool_calls,
        &mut stop_reason,
        &mut input_tokens,
        &mut output_tokens,
        &mut |_| {},
    )
    .unwrap();

    assert!(tool_calls.is_empty());

    apply_ollama_stream_event(
        &json!({
            "message":{
                "tool_calls":[{"function":{"name":"read_file","arguments":{"path":"Cargo.toml"}}}]
            }
        }),
        &mut text,
        &mut tool_calls,
        &mut stop_reason,
        &mut input_tokens,
        &mut output_tokens,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].name, "read_file");
}

#[test]
fn bypasses_proxy_for_local_ollama_hosts() {
    assert!(should_bypass_proxy("http://localhost:11434"));
    assert!(should_bypass_proxy("http://127.0.0.1:11434"));
    assert!(!should_bypass_proxy("https://openrouter.ai/api/v1"));
}
