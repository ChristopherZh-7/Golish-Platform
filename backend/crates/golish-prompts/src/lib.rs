//! Prompt composition system for the Golish AI agent stack.
//!
//! This crate is the bottom layer of the AI agent split (P2-1 in the
//! `architecture-upgrade-plan` rule). It owns the cross-cutting prompt
//! infrastructure that previously lived in `golish-ai`:
//!
//! - [`contributors`]    — pluggable `PromptContributor` implementations
//!   (provider tools, skills, sub-agents, tavily tools)
//! - [`prompt_registry`] — `PromptContributorRegistry` for assembling
//!   prompts from registered contributors in priority order
//! - [`system_prompt`]   — top-level system prompt builder + agent-mode
//!   instructions + team delegation
//! - [`codex_prompt`]    — codex-style prompt variant
//! - [`summarizer`]      — context summarisation prompts and LLM-driven
//!   conversation compaction
//!
//! # Layering
//!
//! `golish-prompts` is treated as a **Layer 2.5 / Infrastructure** crate:
//! - Depends on `golish-core` (foundation types: `AgentMode`,
//!   `PromptContributor`, `PromptSection`, `PromptContext`)
//! - Depends on `golish-llm-providers` for `LlmClient` (used by
//!   `summarizer`)
//!
//! After A1 this crate no longer depends on `golish-sub-agents`:
//! `SubAgentPromptContributor` has moved into
//! `golish-sub-agents::prompt_contributor`, and the
//! `create_default_contributors` helper lives in
//! `golish-agent-bridge::contributors`. Those changes remove the last
//! illegal back-edge from this layer.
//!
//! Consumers (`golish-agent-kit`, `golish-agent-bridge`, the main
//! `golish` Tauri app) should `use golish_prompts::*` directly. The
//! `golish-ai` umbrella crate currently re-exports this surface for
//! backward compatibility.

pub mod codex_prompt;
pub mod contributors;
pub mod prompt_registry;
pub mod summarizer;
pub mod system_prompt;

pub use codex_prompt::build_codex_style_prompt;
pub use contributors::{
    ProviderBuiltinToolsContributor, SkillsPromptContributor, TavilyToolsContributor,
};
pub use prompt_registry::PromptContributorRegistry;
pub use summarizer::{
    build_summarizer_user_prompt, generate_summary, SummaryResponse, SUMMARIZER_SYSTEM_PROMPT,
};
pub use system_prompt::{
    build_system_prompt, get_agent_mode_instructions, read_project_instructions,
};
