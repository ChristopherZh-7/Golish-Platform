//! Shared infrastructure for all provider implementations.
//!
//! - [`api_key`] · `KeyStore` trait + `EnvKeyStore` impl
//! - [`rate_limit`] · per-provider request pacing

pub mod api_key;
pub mod rate_limit;

pub use api_key::{EnvKeyStore, KeyStore};
pub use rate_limit::RateLimiter;
