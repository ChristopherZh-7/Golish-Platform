//! LLM provider settings: per-provider config blocks for Vertex Anthropic,
//! Vertex Gemini, OpenRouter (incl. provider preferences), Anthropic, OpenAI,
//! Ollama, Gemini, Groq, xAI, Z.AI SDK, NVIDIA NIM, and DeepSeek.
//!
//! Split by provider family into sibling modules; all types are re-exported
//! here so `schema::llm::VertexAiSettings` etc. stay stable.

mod google;
mod openai_compat;
mod openrouter;

pub use google::*;
pub use openai_compat::*;
pub use openrouter::*;
