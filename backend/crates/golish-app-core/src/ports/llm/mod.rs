//! LLM-facing inbound ports (servitization): a minimal contract so app/tool
//! crates can request LLM work without depending on provider crates.

pub mod one_shot;

pub use one_shot::LlmOneShot;
