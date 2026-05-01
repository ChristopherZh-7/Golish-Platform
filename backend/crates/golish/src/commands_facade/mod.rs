//! Command facades — domain-grouped command organization.
//!
//! The `commands_registry.rs` uses glob imports via `include!` for the
//! `generate_handler!` macro. This directory preserves the per-domain
//! grouping as documentation and future API boundary when sub-modules
//! are made `pub(crate)`.
