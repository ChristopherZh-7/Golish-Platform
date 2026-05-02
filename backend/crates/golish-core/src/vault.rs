//! Simple XOR-based value obfuscation for credential vault storage.
//!
//! NOT cryptographically secure — provides only basic obfuscation so
//! values are not stored in plaintext on disk.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

fn derive_key() -> Vec<u8> {
    let seed = format!(
        "golish-vault-{}",
        dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default()
    );
    let mut key = Vec::with_capacity(32);
    let bytes = seed.as_bytes();
    for i in 0..32 {
        key.push(bytes[i % bytes.len()].wrapping_add(i as u8).wrapping_mul(7));
    }
    key
}

pub fn obfuscate(plain: &str) -> String {
    let key = derive_key();
    let encrypted: Vec<u8> = plain
        .as_bytes()
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    B64.encode(&encrypted)
}

pub fn deobfuscate(encoded: &str) -> Result<String, String> {
    let key = derive_key();
    let data = B64.decode(encoded).map_err(|e| e.to_string())?;
    let plain: Vec<u8> = data
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    String::from_utf8(plain).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: VaultEntryType,
    pub value: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Sanitized vault entry returned to the UI (no plaintext value).
///
/// Field names are deliberately snake_case (matches storage schema) and
/// `entry_type` is exposed as `type` over the wire via `#[serde(rename = "type")]`.
/// The `#[derive(TS)]` + `#[ts(export)]` exports a TypeScript counterpart to
/// `frontend/lib/generated/VaultEntrySafe.ts` so the frontend never has to
/// hand-mirror this shape again. Run `just generate-types` after editing.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "generated/")]
pub struct VaultEntrySafe {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub entry_type: VaultEntryType,
    pub username: String,
    pub notes: String,
    pub project: String,
    pub tags: Vec<String>,
    pub status: String,
    pub source_url: String,
    /// Unix seconds. Override the default `bigint` mapping because timestamps
    /// fit comfortably in JS `number` (Number.MAX_SAFE_INTEGER = 2^53 is well
    /// beyond seconds since epoch for the next ~285M years) and the frontend
    /// arithmetic (`entry.created_at * 1000`) treats them as `number`.
    #[ts(type = "number | null")]
    pub last_validated_at: Option<u64>,
    #[ts(type = "number")]
    pub created_at: u64,
    #[ts(type = "number")]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "generated/")]
pub enum VaultEntryType {
    Password,
    Token,
    #[serde(rename = "ssh_key")]
    #[ts(rename = "ssh_key")]
    SshKey,
    #[serde(rename = "api_key")]
    #[ts(rename = "api_key")]
    ApiKey,
    Cookie,
    Certificate,
    Other,
}

impl VaultEntryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Token => "token",
            Self::SshKey => "ssh_key",
            Self::ApiKey => "api_key",
            Self::Cookie => "cookie",
            Self::Certificate => "certificate",
            Self::Other => "other",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "token" => Self::Token,
            "ssh_key" => Self::SshKey,
            "api_key" => Self::ApiKey,
            "cookie" => Self::Cookie,
            "certificate" => Self::Certificate,
            "other" => Self::Other,
            _ => Self::Password,
        }
    }
}
