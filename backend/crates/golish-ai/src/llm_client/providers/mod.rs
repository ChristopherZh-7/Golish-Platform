//! Per-provider component builders.
//!
//! Each submodule exposes a single `pub async fn create_*_components(...)`. They
//! are flattened back up to `crate::llm_client::*` so callers
//! (`agent_bridge::constructors`) keep using the original symbol paths.

mod anthropic;
mod gemini;
mod groq;
mod nvidia;
mod ollama;
mod openai;
mod openrouter;
mod vertex_anthropic;
mod vertex_gemini;
mod xai;
mod zai_sdk;

pub use anthropic::create_anthropic_components;
pub use gemini::create_gemini_components;
pub use groq::create_groq_components;
pub use nvidia::create_nvidia_components;
pub use ollama::create_ollama_components;
pub use openai::create_openai_components;
pub use openrouter::create_openrouter_components;
pub use vertex_anthropic::create_vertex_components;
pub use vertex_gemini::create_vertex_gemini_components;
pub use xai::create_xai_components;
pub use zai_sdk::create_zai_sdk_components;
