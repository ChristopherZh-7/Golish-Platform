//! Shared cross-service domain DTOs (servitization S1-3).
//!
//! Remote-ready contract types that more than one app service needs but no
//! single service should own (moving them here breaks sibling-crate cycles).
//! Currently: the recon `targets` surface (`Target` / `Scope` / `ReconUpdate`
//! / `DirectoryEntry` / …) consumed by recon, pentest and agent services.

pub mod operator;
pub mod targets;
