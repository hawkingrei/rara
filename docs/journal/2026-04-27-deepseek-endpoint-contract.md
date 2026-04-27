# DeepSeek Endpoint Contract

RARA now treats DeepSeek as a dedicated OpenAI-compatible endpoint kind with
provider-specific runtime metadata and a clearer `/model` management surface.

## Changes

- Preserved DeepSeek `reasoning_content` from chat-completions responses as
  provider metadata instead of rendering it as visible assistant text.
- Rendered DeepSeek provider metadata back into later chat-completions requests
  only when the active endpoint kind is `deepseek`.
- Kept generic OpenAI-compatible endpoints on standard `content` and
  `tool_calls` fields unless their endpoint kind declares additional metadata.
- Added a selectable DeepSeek `API key` row in the model picker so the key can
  be edited even after one already exists.
- Accepted both uppercase and lowercase `A` / `R` shortcuts for DeepSeek API-key
  editing and model-list refresh.

## Validation

- `cargo test deepseek_reasoning_content_roundtrips_as_provider_metadata -- --nocapture`
- `cargo test deepseek_model_picker -- --nocapture`
- `cargo check`
- `cargo fmt --check`
- `git diff --check`
- `cargo test`
