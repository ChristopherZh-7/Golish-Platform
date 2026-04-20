#![allow(dead_code)] // Error types for future use
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GolishError {
    #[error("PTY error: {0}")]
    Pty(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

// Convert from PtyError
impl From<golish_pty::PtyError> for GolishError {
    fn from(err: golish_pty::PtyError) -> Self {
        GolishError::Pty(err.to_string())
    }
}

// Convert from SkillsError
impl From<golish_skills::SkillsError> for GolishError {
    fn from(err: golish_skills::SkillsError) -> Self {
        GolishError::Internal(err.to_string())
    }
}

// Implement Serialize for Tauri
impl Serialize for GolishError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// Convert to Tauri-compatible result
pub type Result<T> = std::result::Result<T, GolishError>;
