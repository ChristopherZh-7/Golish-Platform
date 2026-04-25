//! Default sub-agent definitions.
//!
//! This module provides pre-configured sub-agents for common tasks.
//!
//! ## Layout
//!
//! - [`builder`]  — public constructors that assemble [`SubAgentDefinition`]s
//!   ([`create_default_sub_agents`] and [`create_default_sub_agents_from_registry`]).
//! - [`prompts`]  — every hardcoded `build_*_prompt` function plus the
//!   [`WORKER_PROMPT_TEMPLATE`] constant. The hardcoded prompts here serve as
//!   fallbacks for the template-driven registry version which prefers
//!   `prompts/*.tera` (and DB overrides loaded into the registry).
//!
//! [`SubAgentDefinition`]: crate::definition::SubAgentDefinition

mod builder;
mod prompts;

#[cfg(test)]
mod tests;

pub use builder::{create_default_sub_agents, create_default_sub_agents_from_registry};
pub use prompts::WORKER_PROMPT_TEMPLATE;
