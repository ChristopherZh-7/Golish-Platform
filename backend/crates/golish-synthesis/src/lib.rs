//! Synthesis module - LLM-based generation for commit messages, state updates, and session titles.

pub mod prompts;
pub mod config;
pub mod commit;
pub mod state;
pub mod title;
pub mod template;

pub use prompts::*;
pub use config::*;
pub use commit::*;
pub use state::*;
pub use title::*;
pub use template::*;

#[cfg(test)]
mod tests;
