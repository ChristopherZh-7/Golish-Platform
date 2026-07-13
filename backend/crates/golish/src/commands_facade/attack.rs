//! Durable Candidate review and execution-attempt command surface.

// `generate_handler!` resolves Tauri's exported command wrapper macros at the
// registry call site, so rustc does not count this facade import as a value use.
#[allow(unused_imports)]
pub use golish_agent_app::ai::commands::attack::*;
