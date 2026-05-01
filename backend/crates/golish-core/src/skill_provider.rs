//! Skill provider abstraction trait.
//!
//! Provides a provider-agnostic interface for skill discovery, matching,
//! and body loading. The concrete implementation lives in `golish-skills`.

use serde::{Deserialize, Serialize};

/// Lightweight skill metadata for caching and matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub path: String,
    pub source: String,
    pub allowed_tools: Option<Vec<String>>,
    pub keywords: Vec<String>,
}

/// A matched skill with its score and reason.
pub type SkillMatch = (SkillMetadata, f32, String);

/// Provider trait for skill operations.
///
/// `golish-ai` depends on this trait; the concrete implementation
/// (directory scanning, keyword matching) is injected by the application layer.
pub trait SkillProvider: Send + Sync {
    /// Discover skills from global and workspace directories.
    /// This is a synchronous operation (intended for `spawn_blocking`).
    fn discover_skills(&self, workspace: Option<&str>) -> Vec<SkillMetadata>;

    /// Match skills against a user prompt using keyword/semantic matching.
    fn match_skills(&self, prompt: &str, cache: &[SkillMetadata]) -> Vec<SkillMatch>;

    /// Load the body text of a skill from its path.
    fn load_skill_body(&self, path: &str) -> anyhow::Result<String>;
}
