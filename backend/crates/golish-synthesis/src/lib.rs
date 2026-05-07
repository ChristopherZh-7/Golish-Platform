//! Synthesis module - LLM-based generation for commit messages, state updates, and session titles.

pub mod commit;
pub mod config;
pub mod prompts;
pub mod state;
pub mod template;
pub mod title;

pub use commit::*;
pub use config::*;
pub use prompts::*;
pub use state::*;
pub use template::*;
pub use title::*;

#[cfg(test)]
mod tests;
