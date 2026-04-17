//! Tools module - re-exports from golish-tools crate.
//!
//! This module provides a thin wrapper around the golish-tools infrastructure crate.
//!
//! # Architecture
//!
//! - **golish-tools**: Infrastructure crate with tool execution system
//! - **golish/tools/mod.rs**: Re-exports for compatibility

// Re-export everything from golish-tools
pub use golish_tools::*;

// Penetration testing tool management (ported from Golish)
pub mod pentest;

// Wiki / knowledge-base storage
pub mod wiki;

// Target / scope management
pub mod targets;

// Credential vault (encrypted storage)
pub mod vault;

// Project export/import
pub mod project_io;

// Pentest AI tools (expose installed pentest tools to the AI agent)
pub mod pentest_ai;

// Interactive PTY tool (allows AI to control visible terminal sessions)
pub mod pty_interactive;

// Pentest methodology templates
pub mod methodology;

// Terminal session recordings
pub mod recordings;

// Generic tool output parsing engine
pub mod output_parser;

// Vulnerability findings tracker
pub mod findings;

// Tool chain pipeline
pub mod pipeline;

// Quick notes
pub mod notes;

// Audit log
pub mod audit;

// Wordlist manager
pub mod wordlists;

// Vulnerability intelligence
pub mod vuln_intel;

// AI bridge tools (expose targets/findings/vault to the AI agent)
pub mod pentest_bridge;

// Execution plans (structured task tracking for AI agent continuation)
pub mod execution_plans;

// Frontend conversation & timeline persistence (replaces workspace.json)
pub mod conversation_store;

// ZAP scan queue persistence
pub mod scan_queue;

// Custom passive scan rules persistence
pub mod custom_rules;

// Security analysis (operation logs, assets, endpoints, fingerprints, scans)
pub mod security_analysis;

// Scan runner (WhatWeb, Nuclei targeted, feroxbuster)
pub mod scan_runner;
