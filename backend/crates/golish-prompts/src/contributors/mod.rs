//! Prompt contributors for dynamic system prompt composition.
//!
//! Each contributor implements the `PromptContributor` trait and provides
//! context-aware prompt sections.
//!
//! # DAG note (A1)
//!
//! `SubAgentPromptContributor` used to live here, but it depended on
//! `golish-sub-agents::SubAgentRegistry`, which introduced a back-edge
//! from a pure L3 prompt-infrastructure crate into a sibling domain
//! crate. It has been moved to `golish-sub-agents::prompt_contributor`.
//!
//! The `create_default_contributors` composition helper — which needed
//! to bundle the sub-agent contributor together with the provider /
//! skill / tavily contributors — has moved to
//! `golish-agent-bridge::contributors`, since the bridge layer is the
//! natural assembly point that already depends on both crates.

mod provider_tools;
mod skills;
mod tavily_tools;

pub use provider_tools::ProviderBuiltinToolsContributor;
pub use skills::SkillsPromptContributor;
pub use tavily_tools::TavilyToolsContributor;
