//! Event emission abstraction.
//!
//! This trait allows business logic crates to emit events to the frontend
//! without depending on `tauri::AppHandle`. The concrete Tauri implementation
//! lives in the application layer (`golish` main crate).
//!
//! Design goals:
//! - Zero tauri dependency for business logic crates.
//! - Simple wire format: JSON payload keyed by event name.
//! - Thread-safe and `Clone`-friendly (backed by `Arc`) so emitters can be
//!   cheaply cloned into spawned tasks.

use std::sync::Arc;

use serde::Serialize;

/// Low-level event emitter trait: send a named event with a JSON payload.
///
/// Implementations should be non-blocking and silently swallow errors
/// (the frontend channel may disappear during shutdown, and emit failures
/// are informational rather than fatal).
pub trait EventEmitter: Send + Sync + std::fmt::Debug {
    fn emit_json(&self, event: &str, payload: serde_json::Value);
}

/// Handle that wraps an [`EventEmitter`] implementation.
///
/// This is the type all business logic should accept (typically as
/// `Option<&EventEmitterHandle>` for headless/test scenarios).
#[derive(Debug, Clone)]
pub struct EventEmitterHandle {
    inner: Arc<dyn EventEmitter>,
}

impl EventEmitterHandle {
    pub fn new<E: EventEmitter + 'static>(emitter: E) -> Self {
        Self {
            inner: Arc::new(emitter),
        }
    }

    pub fn from_arc(inner: Arc<dyn EventEmitter>) -> Self {
        Self { inner }
    }

    /// Emit a typed event. Serialization errors are silently dropped,
    /// matching the fire-and-forget semantics of the underlying transport.
    /// (Callers that need richer diagnostics should implement custom emitters.)
    pub fn emit<P: Serialize + ?Sized>(&self, event: &str, payload: &P) {
        if let Ok(v) = serde_json::to_value(payload) {
            self.inner.emit_json(event, v);
        }
    }

    /// Emit a raw JSON value.
    pub fn emit_value(&self, event: &str, value: serde_json::Value) {
        self.inner.emit_json(event, value);
    }
}

/// Convenience helper: emit via an optional handle (common pattern where
/// headless/CLI runs have no frontend).
pub fn emit_opt<P: Serialize + ?Sized>(
    handle: Option<&EventEmitterHandle>,
    event: &str,
    payload: &P,
) {
    if let Some(h) = handle {
        h.emit(event, payload);
    }
}

/// A no-op emitter useful for tests, CLI runs, and any code path where
/// frontend events are not needed.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullEmitter;

impl EventEmitter for NullEmitter {
    fn emit_json(&self, _event: &str, _payload: serde_json::Value) {}
}

impl NullEmitter {
    pub fn handle() -> EventEmitterHandle {
        EventEmitterHandle::new(Self)
    }
}
