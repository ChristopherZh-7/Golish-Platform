//! Project-asset Tauri commands.
//!
//! Surfaces the editable agent inputs that live alongside a project on disk:
//!
//! - [`prompts`]: reusable prompt files
//! - [`rules`]: persistent agent rules with frontmatter
//! - [`skills`]: agentskills.io directory-based skill packages
//!
//! All public items are re-exported at the parent [`crate::commands`] level.

mod prompts;
mod rules;
mod skills;

pub use prompts::*;
pub use rules::*;
pub use skills::*;
