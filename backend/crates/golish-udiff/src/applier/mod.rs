//! Applier subsystem for `UdiffApplier`.
//!
//! Splits the previously single-file applier into thematic submodules:
//!
//! - [`errors`]: [`ApplyResult`] (public) + `HunkApplyError` (internal).
//! - [`direct`]: exact + normalized-whitespace matching strategies.
//! - [`fuzzy`]: similarity-window matching with `similar::TextDiff`.
//! - [`apply`]: the top-level dispatcher that runs the strategies in order.
//!
//! `UdiffApplier` is a unit struct that serves as the namespace for all
//! associated functions; each submodule contributes one or more methods via
//! its own `impl UdiffApplier { ... }` block.

mod apply;
mod direct;
mod errors;
mod fuzzy;

#[cfg(test)]
mod tests;

pub use errors::ApplyResult;

/// Applier for unified diffs.
pub struct UdiffApplier;
