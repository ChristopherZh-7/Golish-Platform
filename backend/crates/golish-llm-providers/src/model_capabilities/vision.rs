//! Vision/multimodal capability detection per provider+model.

use serde::{Deserialize, Serialize};


// ============================================================================
// Vision Capabilities
// ============================================================================

/// Vision/image capabilities for LLM providers.
///
/// Used to determine if a model supports image inputs and what constraints apply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisionCapabilities {
    /// Whether the model supports image inputs.
    pub supports_vision: bool,
    /// Maximum image size in bytes (provider-specific limit).
    pub max_image_size_bytes: usize,
    /// Supported MIME types (e.g., "image/png", "image/jpeg").
    pub supported_formats: Vec<String>,
}

impl VisionCapabilities {
    /// Detect vision capabilities based on provider and model name.
    ///
    /// # Arguments
    /// * `provider_name` - The provider identifier (e.g., "openai", "anthropic", "vertex_ai")
    /// * `model_name` - The model identifier (e.g., "gpt-4o", "claude-sonnet-4-5")
    ///
    /// # Examples
    /// ```
    /// use golish_llm_providers::VisionCapabilities;
    ///
    /// // Claude models support vision
    /// let caps = VisionCapabilities::detect("vertex_ai", "claude-sonnet-4-5");
    /// assert!(caps.supports_vision);
    ///
    /// // Ollama doesn't support vision in our implementation
    /// let caps = VisionCapabilities::detect("ollama", "llama3.2");
    /// assert!(!caps.supports_vision);
    /// ```
    pub fn detect(provider_name: &str, model_name: &str) -> Self {
        let model_lower = model_name.to_lowercase();
        let standard_formats = vec![
            "image/png".to_string(),
            "image/jpeg".to_string(),
            "image/gif".to_string(),
            "image/webp".to_string(),
        ];

        match provider_name {
            // Anthropic (Vertex AI or direct) - Claude 3+ models support vision
            "vertex_ai" | "vertex_ai_anthropic" | "anthropic_vertex" | "anthropic" => {
                let supports_vision = model_lower.contains("claude-3")
                    || model_lower.contains("claude-4")
                    || model_lower.contains("claude-sonnet")
                    || model_lower.contains("claude-opus")
                    || model_lower.contains("claude-haiku");

                Self {
                    supports_vision,
                    max_image_size_bytes: 5 * 1024 * 1024, // 5MB for Anthropic
                    supported_formats: if supports_vision {
                        standard_formats
                    } else {
                        vec![]
                    },
                }
            }

            // OpenAI - GPT-4+ and o-series support vision
            "openai" | "openai_responses" | "openai_reasoning" => {
                let supports_vision = model_lower.contains("gpt-4")
                    || model_lower.contains("gpt-5")
                    || model_lower.starts_with("o1")
                    || model_lower.starts_with("o3")
                    || model_lower.starts_with("o4");

                Self {
                    supports_vision,
                    max_image_size_bytes: 20 * 1024 * 1024, // 20MB for OpenAI
                    supported_formats: if supports_vision {
                        standard_formats
                    } else {
                        vec![]
                    },
                }
            }

            // Gemini - All models support vision (direct API or Vertex AI)
            "gemini" | "vertex_ai_gemini" | "vertex_gemini" => Self {
                supports_vision: true,
                max_image_size_bytes: 20 * 1024 * 1024, // 20MB
                supported_formats: standard_formats,
            },

            // Z.AI - Vision models have "v" suffix (e.g., glm-4.6v, glm-4v)
            "zai" | "zai_sdk" => {
                // Vision models: glm-4v, glm-4.6v, etc.
                let supports_vision = model_lower.ends_with("v")
                    || model_lower.contains("-v-")
                    || model_lower.ends_with("-v");

                Self {
                    supports_vision,
                    max_image_size_bytes: 10 * 1024 * 1024, // 10MB for Z.AI
                    supported_formats: if supports_vision {
                        standard_formats
                    } else {
                        vec![]
                    },
                }
            }

            // Providers without vision support in our implementation
            "ollama" | "groq" | "xai" | "openrouter" | "mock" => Self::default(),

            // Unknown providers - no vision support
            _ => Self::default(),
        }
    }
}
