//! Workflow definitions.
//!
//! Each workflow type is defined in its own submodule and implements
//! the `WorkflowDefinition` trait.

pub mod git_commit;
pub mod recon_basic;

use std::sync::Arc;

use crate::registry::WorkflowRegistry;

// Re-export workflow definitions for convenience
pub use git_commit::GitCommitWorkflow;
pub use recon_basic::ReconBasicWorkflow;

/// Register all built-in workflows with the registry.
pub fn register_builtin_workflows(registry: &mut WorkflowRegistry) {
    registry.register(Arc::new(GitCommitWorkflow));
    registry.register(Arc::new(ReconBasicWorkflow));
}

/// Create a registry with all built-in workflows pre-registered.
pub fn create_default_registry() -> WorkflowRegistry {
    let mut registry = WorkflowRegistry::new();
    register_builtin_workflows(&mut registry);
    registry
}
