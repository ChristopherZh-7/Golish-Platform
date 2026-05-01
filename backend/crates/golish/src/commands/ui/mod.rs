//! UI / chrome-domain Tauri commands.
//!
//! - [`themes`]: theme metadata and asset CRUD
//! - [`ime`]: macOS input-method-source query/set
//! - [`logging`]: frontend log forwarder writing to `~/.golish/frontend.log`
//!
//! All public items are re-exported at the parent [`crate::commands`] level.

pub mod ime;
pub mod logging;
pub mod themes;

pub use ime::*;
pub use logging::*;
pub use themes::*;
