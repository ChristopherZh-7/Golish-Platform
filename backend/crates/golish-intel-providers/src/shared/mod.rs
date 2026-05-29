//! Shared infrastructure for all provider implementations.
//!
//! - [`api_key`] · `KeyStore` trait + `EnvKeyStore` impl
//! - [`rate_limit`] · per-provider request pacing
//! - [`http_common`] · shared reqwest client builder + simple JSON decoder

pub mod api_key;
pub mod http_common;
pub mod rate_limit;

pub use api_key::{EnvKeyStore, KeyStore};
pub use rate_limit::RateLimiter;
