use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use candle::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::gemma4::{
    config::{Gemma4Config, Gemma4TextConfig},
    text::TextModel as Gemma4TextModel,
    Model as Gemma4Model,
};
use candle_transformers::models::qwen3::{Config as Qwen3Config, ModelForCausalLM as Qwen3Model};
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

pub type LocalProgressReporter = Arc<dyn Fn(String) + Send + Sync>;

pub struct LocalLlmBackend {
    runtime: Arc<Mutex<LocalRuntime>>,
    max_new_tokens: usize,
}

struct LocalRuntime {
    spec: LocalModelSpec,
    model: LocalTextModel,
    tokenizer: Tokenizer,
    device: Device,
    eos_token_ids: Vec<u32>,
}

enum LocalTextModel {
    Gemma4(Gemma4TextModel),
    Gemma4Multimodal(Gemma4Model),
    Qwen3(Qwen3Model),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalModelSpec {
    Gemma4E2B,
    Gemma4E4B,
    Qwen3_8B,
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

struct GenerationResult {
    text: String,
    input_tokens: u32,
    output_tokens: u32,
}

impl LocalLlmBackend {
    pub fn from_config(config: &RaraConfig) -> Result<Self> {
        Self::from_config_with_progress(config, None)
    }

    pub fn from_config_with_progress(
        config: &RaraConfig,
        progress: Option<LocalProgressReporter>,
    ) -> Result<Self> {
        let spec = LocalModelSpec::from_config(config)?;
        let revision = config
            .revision
            .clone()
            .unwrap_or_else(|| "main".to_string());
        let cache_dir = default_local_model_cache_dir();
        report_progress(
            &progress,
            format!("Download setup · {} ({revision})", spec.alias()),
        );
        report_progress(
            &progress,
            format!("Cache · {}", cache_dir.display()),
        );
        let api = build_hf_api(config, &cache_dir)?;
        let repo = api.repo(Repo::with_revision(
            spec.model_id().to_string(),
            RepoType::Model,
            revision,
        ));

        report_progress(
            &progress,
            format!("Manifest · resolving {}", spec.model_id()),
        );

        report_progress(&progress, "Artifact · tokenizer.json".to_string());
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("download tokenizer.json")?;
        report_progress(&progress, "Artifact · config.json".to_string());
        let config_path = repo.get("config.json").context("download config.json")?;
        report_progress(&progress, "Weights · downloading model weights".to_string());
        let weight_paths = load_safetensors(&repo)
            .or_else(|_| repo.get("model.safetensors").map(|p| vec![p]))
            .context("download model weights")?;
        report_progress(
            &progress,
            format!("Weights · {} file(s) ready", weight_paths.len()),
        );

        let raw_config: Value =
            serde_json::from_slice(&std::fs::read(&config_path).context("read config.json")?)
                .context("parse config.json")?;
        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow!("load tokenizer: {e}"))?;

        let device = select_device()?;
        let dtype = preferred_dtype(&device);
        report_progress(
            &progress,
            format!("Runtime · initializing on {:?} with {:?}", device, dtype),
        );
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&weight_paths, dtype, &device)? };
        let model = spec.build_model(&raw_config, vb)?;
        let eos_token_ids = spec.eos_token_ids(&tokenizer)?;
        report_progress(
            &progress,
            format!("Ready · {} loaded", spec.alias()),
        );

        Ok(Self {
            runtime: Arc::new(Mutex::new(LocalRuntime {
                spec,
                model,
                tokenizer,
                device,
                eos_token_ids,
            })),
            max_new_tokens: 1024,
        })
    }
}

fn report_progress(progress: &Option<LocalProgressReporter>, message: String) {
    if let Some(callback) = progress {
        callback(message);
    }
}

#[async_trait]
impl LlmBackend for LocalLlmBackend {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
        let runtime = Arc::clone(&self.runtime);
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        let max_new_tokens = self.max_new_tokens;
        let result = tokio::task::spawn_blocking(move || {
            runtime
                .lock()
                .map_err(|_| anyhow!("local model runtime mutex poisoned"))?
                .generate(&messages, &tools, max_new_tokens)
        })
        .await
        .context("local model worker task join failed")??;
        let raw = result.text;

        let content = match parse_tool_aware_reply(&raw) {
            Ok(parsed) => {
                let mut blocks = Vec::new();
                if let Some(text) = parsed.text.filter(|t| !t.trim().is_empty()) {
                    blocks.push(ContentBlock::Text { text });
                }
                if parsed.kind.as_deref() == Some("tool") {
                    for (idx, call) in parsed.calls.unwrap_or_default().into_iter().enumerate() {
                        blocks.push(ContentBlock::ToolUse {
                            id: format!("local-tool-{}", idx + 1),
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
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
            }),
        })
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(hashed_embedding(text, 256))
    }

    async fn summarize(&self, messages: &[Message]) -> Result<String> {
        let runtime = Arc::clone(&self.runtime);
        let messages = messages.to_vec();
        tokio::task::spawn_blocking(move || {
            runtime
                .lock()
                .map_err(|_| anyhow!("local model runtime mutex poisoned"))?
                .summarize(&messages)
        })
        .await
        .context("local model summary task join failed")?
    }
}

impl LocalRuntime {
    fn generate(
        &mut self,
        messages: &[Message],
        tools: &[Value],
        max_new_tokens: usize,
    ) -> Result<GenerationResult> {
        let base_prompt = build_agent_prompt(messages, tools);
        let prompt = self.spec.format_prompt(&base_prompt);
        self.model.clear_kv_cache();

        let prompt_tokens = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|e| anyhow!("tokenize prompt: {e}"))?
            .get_ids()
            .to_vec();
        if prompt_tokens.is_empty() {
            return Ok(GenerationResult {
                text: String::new(),
                input_tokens: 0,
                output_tokens: 0,
            });
        }

        let mut tokens = prompt_tokens.clone();
        let mut generated = Vec::new();
        let mut processor = LogitsProcessor::from_sampling(rand::random(), Sampling::ArgMax);

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
            if self.eos_token_ids.contains(&next_token) {
                break;
            }
            tokens.push(next_token);
            generated.push(next_token);
        }

        let text = self
            .tokenizer
            .decode(&generated, true)
            .map_err(|e| anyhow!("decode output: {e}"))?;
        Ok(GenerationResult {
            text,
            input_tokens: prompt_tokens.len() as u32,
            output_tokens: generated.len() as u32,
        })
    }

    fn summarize(&mut self, messages: &[Message]) -> Result<String> {
        let prompt = format!(
            "You are summarizing an agent conversation.\n\
             Produce a concise summary in plain English with no markdown fence.\n\
             Keep the result under 180 words.\n\n{}",
            render_messages(messages)
        );
        let prompt = self.spec.format_prompt(&prompt);
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
        let mut processor = LogitsProcessor::from_sampling(rand::random(), Sampling::ArgMax);

        for index in 0..256 {
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
            if self.eos_token_ids.contains(&next_token) {
                break;
            }
            tokens.push(next_token);
            generated.push(next_token);
        }

        self.tokenizer
            .decode(&generated, true)
            .map_err(|e| anyhow!("decode output: {e}"))
            .map(|s| s.trim().to_string())
    }
}

impl LocalTextModel {
    fn forward(&mut self, input: &Tensor, offset: usize) -> candle::Result<Tensor> {
        match self {
            Self::Gemma4(model) => model.forward(input, offset),
            Self::Gemma4Multimodal(model) => model.forward(input, offset),
            Self::Qwen3(model) => model.forward(input, offset),
        }
    }

    fn clear_kv_cache(&mut self) {
        match self {
            Self::Gemma4(model) => model.clear_kv_cache(),
            Self::Gemma4Multimodal(model) => model.clear_kv_cache(),
            Self::Qwen3(model) => model.clear_kv_cache(),
        }
    }
}

impl LocalModelSpec {
    fn from_config(config: &RaraConfig) -> Result<Self> {
        let provider = config.provider.trim();
        let model = config.model.as_deref().unwrap_or_default().trim();

        if provider == "qwen3" || provider == "qwn3" {
            return Ok(Self::Qwen3_8B);
        }
        if provider == "gemma4" {
            if model.eq_ignore_ascii_case("gemma4-e2b") || model.eq_ignore_ascii_case("gemma-4-e2b") {
                return Ok(Self::Gemma4E2B);
            }
            return Ok(Self::Gemma4E4B);
        }
        if provider == "local" || provider == "local-candle" {
            return Self::from_alias_or_model_id(model);
        }

        Self::from_alias_or_model_id(model)
    }

    fn from_alias_or_model_id(value: &str) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" | "gemma4-e4b" | "gemma-4-e4b" | "google/gemma-4-e4b-it" => Ok(Self::Gemma4E4B),
            "gemma4-e2b" | "gemma-4-e2b" | "google/gemma-4-e2b-it" => Ok(Self::Gemma4E2B),
            "qwen3-8b" | "qwen-3-8b" | "qwen/qwen3-8b" | "qwn3-8b" | "qwn3 8b" => {
                Ok(Self::Qwen3_8B)
            }
            _ => Err(anyhow!(
                "unsupported local model '{value}', expected one of: gemma4-e4b, gemma4-e2b, qwen3-8b, qwn3-8b"
            )),
        }
    }

    fn alias(self) -> &'static str {
        match self {
            Self::Gemma4E2B => "gemma4-e2b",
            Self::Gemma4E4B => "gemma4-e4b",
            Self::Qwen3_8B => "qwen3-8b",
        }
    }

    fn model_id(self) -> &'static str {
        match self {
            Self::Gemma4E2B => "google/gemma-4-E2B-it",
            Self::Gemma4E4B => "google/gemma-4-E4B-it",
            Self::Qwen3_8B => "Qwen/Qwen3-8B",
        }
    }

    fn format_prompt(self, prompt: &str) -> String {
        match self {
            Self::Qwen3_8B => {
                format!(
                    "<|im_start|>user\n{prompt} /no_think<|im_end|>\n<|im_start|>assistant\n"
                )
            }
            Self::Gemma4E2B | Self::Gemma4E4B => prompt.to_string(),
        }
    }

    fn eos_token_ids(self, tokenizer: &Tokenizer) -> Result<Vec<u32>> {
        let vocab = tokenizer.get_vocab(true);
        let ids = match self {
            Self::Gemma4E2B | Self::Gemma4E4B => vec!["</s>", "<eos>"],
            Self::Qwen3_8B => vec!["<|endoftext|>", "<|im_end|>"],
        };
        let resolved: Vec<u32> = ids
            .into_iter()
            .filter_map(|token| vocab.get(token).copied())
            .collect();
        if resolved.is_empty() {
            return Err(anyhow!(
                "model '{}' tokenizer does not expose a known EOS token",
                self.alias()
            ));
        }
        Ok(resolved)
    }

    fn build_model(self, raw_config: &Value, vb: VarBuilder) -> Result<LocalTextModel> {
        match self {
            Self::Gemma4E2B | Self::Gemma4E4B => build_gemma4_model(raw_config, vb),
            Self::Qwen3_8B => {
                let config: Qwen3Config =
                    serde_json::from_value(raw_config.clone()).context("parse Qwen3Config")?;
                Ok(LocalTextModel::Qwen3(
                    Qwen3Model::new(&config, vb).context("build Qwen3 model")?,
                ))
            }
        }
    }
}

fn is_multimodal_gemma4_checkpoint(raw_config: &Value) -> bool {
    raw_config
        .get("architectures")
        .and_then(Value::as_array)
        .map(|architectures| {
            architectures.iter().any(|value| {
                value
                    .as_str()
                    .map(|name| name == "Gemma4ForConditionalGeneration")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
        || raw_config.get("vision_config").is_some()
        || raw_config.get("audio_config").is_some()
}

fn build_gemma4_model(raw_config: &Value, vb: VarBuilder) -> Result<LocalTextModel> {
    let is_multimodal_layout = is_multimodal_gemma4_checkpoint(raw_config);
    if is_multimodal_layout {
        let config: Gemma4Config =
            serde_json::from_value(raw_config.clone()).context("parse Gemma4Config")?;
        match Gemma4Model::new(&config, vb.clone()) {
            Ok(model) => return Ok(LocalTextModel::Gemma4Multimodal(model)),
            Err(err) if should_fallback_to_text_only(&err) => {}
            Err(err) => return Err(err).context("build Gemma4 multimodal model"),
        }
    }

    let mut text_config: Gemma4TextConfig = if let Some(text_cfg) = raw_config.get("text_config") {
        serde_json::from_value(text_cfg.clone()).context("parse text_config")?
    } else {
        serde_json::from_value(raw_config.clone()).context("parse Gemma4TextConfig")?
    };
    text_config.use_flash_attn = false;
    let text_vb = if is_multimodal_layout {
        vb.rename_f(|name| remap_multimodal_gemma4_text_tensor(name))
    } else {
        vb
    };
    Ok(LocalTextModel::Gemma4(
        Gemma4TextModel::new(&text_config, text_vb).context("build Gemma4 text model")?,
    ))
}

fn should_fallback_to_text_only(err: &candle::Error) -> bool {
    let message = err.to_string();
    message.contains("cannot find tensor model.vision_tower")
        || message.contains("cannot find tensor model.audio_tower")
        || message.contains("cannot find tensor model.vision_encoder")
        || message.contains("cannot find tensor model.audio_encoder")
}

fn remap_multimodal_gemma4_text_tensor(name: &str) -> String {
    if let Some(suffix) = name.strip_prefix("model.") {
        format!("model.language_model.{suffix}")
    } else if let Some(suffix) = name.strip_prefix("lm_head.") {
        format!("model.language_model.lm_head.{suffix}")
    } else {
        name.to_string()
    }
}

fn build_hf_api(config: &RaraConfig, cache_dir: &PathBuf) -> Result<hf_hub::api::sync::Api> {
    let mut builder = ApiBuilder::new()
        .with_cache_dir(cache_dir.clone())
        .with_progress(true)
        .with_retries(3);
    if let Some(token) = config
        .api_key
        .clone()
        .or_else(|| std::env::var("HF_TOKEN").ok())
    {
        builder = builder.with_token(Some(token));
    }
    builder.build().context("build Hugging Face API client")
}

pub fn default_local_model_cache_dir() -> PathBuf {
    if let Ok(path) = std::env::var("RARA_MODEL_CACHE") {
        return PathBuf::from(path);
    }
    let mut base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".rara"));
    base.push("rara");
    base.push("huggingface");
    base
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
        "You are the local model backend for RARA.\n\
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
                    item.get("tool_use_id").and_then(Value::as_str).unwrap_or(""),
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
    serde_json::from_str(payload).context("parse local model JSON reply")
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
    use super::{default_local_model_cache_dir, extract_json_object, parse_tool_aware_reply, render_content, LocalModelSpec};
    use serde_json::json;

    #[test]
    fn resolves_supported_aliases() {
        assert_eq!(
            LocalModelSpec::from_alias_or_model_id("gemma4-e2b").unwrap().model_id(),
            "google/gemma-4-E2B-it"
        );
        assert_eq!(
            LocalModelSpec::from_alias_or_model_id("qwen3-8b").unwrap().model_id(),
            "Qwen/Qwen3-8B"
        );
        assert_eq!(
            LocalModelSpec::from_alias_or_model_id("qwn3-8b").unwrap().model_id(),
            "Qwen/Qwen3-8B"
        );
    }

    #[test]
    fn builds_global_cache_path() {
        let path = default_local_model_cache_dir();
        assert!(path.to_string_lossy().contains("rara"));
        assert!(path.to_string_lossy().contains("huggingface"));
    }

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
        let raw =
            "{\"kind\":\"tool\",\"calls\":[{\"name\":\"read_file\",\"input\":{\"path\":\"Cargo.toml\"}}]}";
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
