//! CLI output handling - Event receiver loop.
//!
//! This module is CRITICAL for the CLI to work. It receives events from the
//! agent via the runtime channel and renders them appropriately based on
//! output mode (terminal, JSON, or quiet).
//!
//! ## Output Modes
//!
//! - **Terminal mode**: Human-readable output with box-drawing formatting
//! - **JSON mode**: Standardized JSONL format for programmatic parsing (NO TRUNCATION)
//! - **Quiet mode**: Only final response output
//!
//! ## Truncation Policy
//!
//! | Output Mode | Tool Input | Tool Output | Reasoning | Text Deltas |
//! |-------------|------------|-------------|-----------|-------------|
//! | Terminal    | No trunc   | 500 chars   | 2000 chars| No trunc    |
//! | JSON        | No trunc   | No trunc    | No trunc  | No trunc    |
//! | Quiet       | Not shown  | Not shown   | Not shown | Final only  |

mod cli_json;
mod event_loop;
pub(crate) mod formatting;
mod terminal;

#[cfg(test)]
mod tests;

pub use cli_json::{convert_to_cli_json, CliJsonEvent};
pub use event_loop::run_event_loop;
pub use formatting::truncate;
