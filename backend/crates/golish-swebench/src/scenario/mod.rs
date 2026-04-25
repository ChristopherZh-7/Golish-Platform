//! SWE-bench scenario implementation.
//!
//! Implements the [`Scenario`] trait for SWE-bench instances.
//!
//! Originally a 685-line file. Split here by concern:
//! - This `mod.rs`: the [`SWEBenchScenario`] struct + constructor +
//!   `From<SWEBenchInstance>` conversion + the `instance` accessor.
//! - [`prompt`]: prompt construction (`build_prompt`,
//!   `build_prompt_with_workspace`, `build_test_section`).
//! - [`runner`]: the [`Scenario`] trait impl — the bulk is the
//!   `run()` method which orchestrates workspace setup, Docker testbed
//!   start, agent execution, final evaluation.
//! - [`reports`]: error/skip-report builders used by `run()`.
//!
//! [`Scenario`]: golish_evals::scenarios::Scenario

mod prompt;
mod reports;
mod runner;

#[cfg(test)]
mod tests;

use crate::types::SWEBenchInstance;

/// Scenario for a single SWE-bench instance.
pub struct SWEBenchScenario {
    /// The SWE-bench instance.
    pub(super) instance: SWEBenchInstance,
    /// Formatted prompt for the agent.
    pub(super) formatted_prompt: String,
    /// Leaked name for static lifetime (the `Scenario` trait requires
    /// `&'static str`).
    pub(super) name: &'static str,
}

impl SWEBenchScenario {
    /// Create a new SWE-bench scenario from an instance.
    pub fn new(instance: SWEBenchInstance) -> Self {
        let formatted_prompt = Self::build_prompt(&instance);
        let name = Box::leak(instance.instance_id.clone().into_boxed_str());

        Self {
            instance,
            formatted_prompt,
            name,
        }
    }

    /// Get the SWE-bench instance.
    pub fn instance(&self) -> &SWEBenchInstance {
        &self.instance
    }
}

impl From<SWEBenchInstance> for SWEBenchScenario {
    fn from(instance: SWEBenchInstance) -> Self {
        Self::new(instance)
    }
}
