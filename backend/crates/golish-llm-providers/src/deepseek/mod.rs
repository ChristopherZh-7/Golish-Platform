//! DeepSeek-specific provider constants.
//!
//! DeepSeek exposes an OpenAI-compatible Chat Completions API. The official
//! endpoint intentionally omits `/v1`.

/// Official DeepSeek OpenAI-compatible endpoint.
pub const DEEPSEEK_DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
