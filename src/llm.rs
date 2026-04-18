mod ollama;
mod openai_compatible;
mod shared;
#[cfg(test)]
mod tests;

pub use self::ollama::OllamaBackend;
pub use self::openai_compatible::{CodexBackend, GeminiBackend, OpenAiCompatibleBackend};
pub(crate) use self::shared::hashed_embedding;
pub use self::shared::{ContextBudget, LlmBackend, MockLlm};
