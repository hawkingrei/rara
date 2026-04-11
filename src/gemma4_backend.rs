use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use candle::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::gemma4::{config::Gemma4TextConfig, text::TextModel};
use hf_hub::{
    api::sync::{ApiBuilder, ApiRepo},
    Repo, RepoType,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokenizers::Tokenizer;

use crate::agent::{AnthropicResponse, ContentBlock, Message, TokenUsage};
use crate::config::RaraConfig;
use crate::llm::LlmBackend;

pub struct Gemma4Backend {
    runtime: Mutex<Gemma4Runtime>,
    max_new_tokens: usize,
}

struct Gemma4Runtime {
    model: TextModel,
    tokenizer: Tokenizer,
    device: Device,
    eos_token_id: u32,
}

#[derive(Debug, Deserialize)]
struct ToolAwareReply {
    kind: Option<String>,
    text: Option<String>,
    calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    input: Value,
}

impl Gemma4Backend {
    pub fn from_config(config: &RaraConfig) -> Result<Self> {
        let model_id = config
            .model
            .clone()
            .unwrap_or_else(|| "google/gemma-4-E4B-it".to_string());
        let revision = config
            .revision
            .clone()
            .unwrap_or_else(|| "main".to_string());
        let api = build_hf_api(config)?;
        let repo = api.repo(Repo::with_revision(model_id, RepoType::Model, revision));

        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("download tokenizer.json")?;
        let config_path = repo.get("config.json").context("download config.json")?;
        let weight_paths = load_safetensors(&repo)
            .or_else(|_| repo.get("model.safetensors").map(|p| vec![p]))
            .context("download model weights")?;

        let raw_config: Value =
            serde_json::from_slice(&std::fs::read(&config_path).context("read config.json")?)
                .context("parse config.json")?;
        let mut text_config: Gemma4TextConfig =
            if let Some(text_cfg) = raw_config.get("text_config") {
                serde_json::from_value(text_cfg.clone()).context("parse text_config")?
            } else {
                serde_json::from_value(raw_config).context("parse Gemma4TextConfig")?
            };
        text_config.use_flash_attn = false;

        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow!("load tokenizer: {e}"))?;
        let eos_token_id = tokenizer
            .get_vocab(true)
            .get("</s>")
            .copied()
            .or_else(|| tokenizer.get_vocab(true).get("<eos>").copied())
            .ok_or_else(|| anyhow!("Gemma4 tokenizer does not expose an EOS token"))?;

        let device = select_device()?;
        let dtype = preferred_dtype(&device);
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&weight_paths, dtype, &device)? };
        let model = TextModel::new(&text_config, vb).context("build Gemma4 text model")?;

        Ok(Self {
            runtime: Mutex::new(Gemma4Runtime {
                model,
                tokenizer,
                device,
                eos_token_id,
            }),
            max_new_tokens: 1024,
        })
    }
}

#[async_trait]
impl LlmBackend for Gemma4Backend {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
        let prompt = build_agent_prompt(messages, tools);
        let raw = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("Gemma4 runtime mutex poisoned"))?
            .generate(&prompt, self.max_new_tokens)?;

        let content = match parse_tool_aware_reply(&raw) {
            Ok(parsed) => {
                let mut blocks = Vec::new();
                if let Some(text) = parsed.text.filter(|t| !t.trim().is_empty()) {
                    blocks.push(ContentBlock::Text { text });
                }
                if parsed.kind.as_deref() == Some("tool") {
                    for (idx, call) in parsed.calls.unwrap_or_default().into_iter().enumerate() {
                        blocks.push(ContentBlock::ToolUse {
                            id: format!("gemma4-tool-{}", idx + 1),
                            name: call.name,
                            input: call.input,
                        });
                    }
                }
                if blocks.is_empty() {
                    vec![ContentBlock::Text {
                        text: raw.trim().to_string(),
                    }]
                } else {
                    blocks
                }
            }
            Err(_) => vec![ContentBlock::Text {
                text: raw.trim().to_string(),
            }],
        };

        Ok(AnthropicResponse {
            stop_reason: Some("end_turn".to_string()),
            content,
            usage: Some(TokenUsage {
                input_tokens: approximate_token_count(&prompt),
                output_tokens: approximate_token_count(&raw),
            }),
        })
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(hashed_embedding(text, 256))
    }

    async fn summarize(&self, messages: &[Message]) -> Result<String> {
        let prompt = format!(
            "You are summarizing an agent conversation.\n\
             Produce a concise summary in plain English with no markdown fence.\n\
             Keep the result under 180 words.\n\n{}",
            render_messages(messages)
        );
        self.runtime
            .lock()
            .map_err(|_| anyhow!("Gemma4 runtime mutex poisoned"))?
            .generate(&prompt, 256)
            .map(|s| s.trim().to_string())
    }
}

impl Gemma4Runtime {
    fn generate(&mut self, prompt: &str, max_new_tokens: usize) -> Result<String> {
        self.model.clear_kv_cache();
        let prompt_tokens = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|e| anyhow!("tokenize prompt: {e}"))?
            .get_ids()
            .to_vec();
        if prompt_tokens.is_empty() {
            return Ok(String::new());
        }

        let mut tokens = prompt_tokens.clone();
        let mut generated = Vec::new();
        let mut processor = LogitsProcessor::from_sampling(299_792_458, Sampling::ArgMax);

        for index in 0..max_new_tokens {
            let context_size = if index == 0 { tokens.len() } else { 1 };
            let start_pos = tokens.len().saturating_sub(context_size);
            let input = Tensor::new(&tokens[start_pos..], &self.device)?.unsqueeze(0)?;
            let logits = self
                .model
                .forward(&input, start_pos)?
                .squeeze(0)?
                .squeeze(0)?
                .to_dtype(DType::F32)?;
            let next_token = processor.sample(&logits)?;
            if next_token == self.eos_token_id {
                break;
            }
            tokens.push(next_token);
            generated.push(next_token);
        }

        self.tokenizer
            .decode(&generated, true)
            .map_err(|e| anyhow!("decode output: {e}"))
    }
}

fn build_hf_api(config: &RaraConfig) -> Result<hf_hub::api::sync::Api> {
    let mut builder = ApiBuilder::new();
    if let Some(token) = config
        .api_key
        .clone()
        .or_else(|| std::env::var("HF_TOKEN").ok())
    {
        builder = builder.with_token(Some(token));
    }
    builder.build().context("build Hugging Face API client")
}

fn load_safetensors(repo: &ApiRepo) -> Result<Vec<PathBuf>> {
    let index_path = repo
        .get("model.safetensors.index.json")
        .context("download model.safetensors.index.json")?;
    let reader = std::fs::File::open(&index_path).context("open safetensors index")?;
    let json: Value = serde_json::from_reader(reader).context("parse safetensors index")?;
    let weight_map = json
        .get("weight_map")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("invalid safetensors index: missing weight_map"))?;

    let mut files = std::collections::BTreeSet::new();
    for value in weight_map.values() {
        if let Some(file) = value.as_str() {
            files.insert(
                repo.get(file)
                    .with_context(|| format!("download weight shard {file}"))?,
            );
        }
    }
    if files.is_empty() {
        return Err(anyhow!("no weight shards found in safetensors index"));
    }
    Ok(files.into_iter().collect())
}

fn preferred_dtype(device: &Device) -> DType {
    if device.is_cuda() || device.is_metal() {
        DType::BF16
    } else {
        DType::F32
    }
}

fn select_device() -> Result<Device> {
    #[cfg(feature = "cuda")]
    {
        let dev = Device::cuda_if_available(0)?;
        if !matches!(dev, Device::Cpu) {
            return Ok(dev);
        }
    }

    #[cfg(feature = "metal")]
    {
        let dev = Device::metal_if_available(0)?;
        if !matches!(dev, Device::Cpu) {
            return Ok(dev);
        }
    }

    Ok(Device::Cpu)
}

fn build_agent_prompt(messages: &[Message], tools: &[Value]) -> String {
    let tool_schemas = if tools.is_empty() {
        "[]".to_string()
    } else {
        serde_json::to_string_pretty(tools).unwrap_or_else(|_| "[]".to_string())
    };

    format!(
        "You are the local Gemma 4 backend for RARA.\n\
         You are participating in an agent loop with tools.\n\
         Return exactly one JSON object and nothing else.\n\n\
         Valid reply shapes:\n\
         {{\"kind\":\"final\",\"text\":\"final answer for the user\"}}\n\
         {{\"kind\":\"tool\",\"text\":\"optional short reasoning\",\"calls\":[{{\"name\":\"tool_name\",\"input\":{{}}}}]}}\n\n\
         Rules:\n\
         - Use kind=\"tool\" only when a tool is required.\n\
         - Tool names must match the provided schema exactly.\n\
         - Tool inputs must be valid JSON objects.\n\
         - Do not use markdown fences.\n\
         - If the task is completed, use kind=\"final\".\n\n\
         Available tools:\n{tool_schemas}\n\n\
         Conversation:\n{}",
        render_messages(messages)
    )
}

fn render_messages(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        out.push_str(&format!(
            "{}:\n{}\n\n",
            message.role.to_uppercase(),
            render_content(&message.content)
        ));
    }
    out
}

fn render_content(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(items) = content.as_array() {
        let mut rendered = Vec::new();
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => rendered.push(
                    item.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
                Some("tool_result") => rendered.push(format!(
                    "tool_result(id={}): {}",
                    item.get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    item.get("content").and_then(Value::as_str).unwrap_or("")
                )),
                Some("tool_use") => rendered.push(format!(
                    "tool_use(name={}, id={}, input={})",
                    item.get("name").and_then(Value::as_str).unwrap_or(""),
                    item.get("id").and_then(Value::as_str).unwrap_or(""),
                    item.get("input").cloned().unwrap_or_else(|| json!({}))
                )),
                _ => rendered.push(item.to_string()),
            }
        }
        return rendered.join("\n");
    }
    content.to_string()
}

fn parse_tool_aware_reply(raw: &str) -> Result<ToolAwareReply> {
    let payload = extract_json_object(raw).unwrap_or(raw).trim();
    serde_json::from_str(payload).context("parse Gemma4 JSON reply")
}

fn extract_json_object(raw: &str) -> Option<&str> {
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

fn approximate_token_count(text: &str) -> u32 {
    text.split_whitespace().count().max(1) as u32
}

fn hashed_embedding(text: &str, dim: usize) -> Vec<f32> {
    use sha2::{Digest, Sha256};

    let mut values = vec![0f32; dim];
    for token in text.split_whitespace() {
        let digest = Sha256::digest(token.as_bytes());
        let bucket = ((digest[0] as usize) << 8 | digest[1] as usize) % dim;
        let sign = if digest[2] % 2 == 0 { 1.0 } else { -1.0 };
        values[bucket] += sign;
    }

    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut values {
            *value /= norm;
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::{extract_json_object, parse_tool_aware_reply, render_content};
    use serde_json::json;

    #[test]
    fn extracts_first_json_object_from_mixed_text() {
        let raw = "```json\n{\"kind\":\"final\",\"text\":\"ok\"}\n```";
        assert_eq!(
            extract_json_object(raw),
            Some("{\"kind\":\"final\",\"text\":\"ok\"}")
        );
    }

    #[test]
    fn parses_tool_reply() {
        let raw = "{\"kind\":\"tool\",\"calls\":[{\"name\":\"read_file\",\"input\":{\"path\":\"Cargo.toml\"}}]}";
        let reply = parse_tool_aware_reply(raw).unwrap();
        assert_eq!(reply.kind.as_deref(), Some("tool"));
        assert_eq!(reply.calls.unwrap()[0].name, "read_file");
    }

    #[test]
    fn renders_tool_results_for_prompting() {
        let rendered = render_content(&json!([
            {"type": "text", "text": "hello"},
            {"type": "tool_result", "tool_use_id": "1", "content": "world"}
        ]));
        assert!(rendered.contains("hello"));
        assert!(rendered.contains("tool_result(id=1): world"));
    }
}
