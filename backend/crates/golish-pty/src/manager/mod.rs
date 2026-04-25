//! PTY session management.
//!
//! [`PtyManager`] owns active sessions and exposes their public API.
//! Internally the implementation is split into:
//!
//! - [`utf8`]: UTF-8 boundary buffering ([`utf8::Utf8IncompleteBuffer`]
//!   and the [`utf8::OutputMessage`] reader→emitter channel envelope).
//! - [`emitter`]: the [`emitter::PtyEventEmitter`] trait + [`RuntimeEmitter`]
//!   adapter that forwards through `GolishRuntime`.
//! - [`core`]: the [`PtyManager`] type plus session lifecycle ([`PtySession`]),
//!   internal session state, the generic `create_session_internal`
//!   routine, and the public read/write/resize/destroy/list APIs.
//!
//! [`RuntimeEmitter`]: emitter::RuntimeEmitter

mod core;
mod emitter;
mod session_create;
mod utf8;

pub use core::{PtyManager, PtySession};
pub use emitter::CommandBlockEvent;
