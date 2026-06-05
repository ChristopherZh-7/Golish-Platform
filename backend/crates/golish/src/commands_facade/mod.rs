//! Command facades — domain-grouped command organization.
//!
//! Each `<domain>.rs` file re-exports every `#[tauri::command]`
//! function that belongs to that domain via `pub use`. The
//! `commands_registry.rs` imports the whole facade tree with
//! `use crate::commands_facade::*;`, giving `tauri::generate_handler!`
//! access to all symbols by their flat identifier at the call site.
//!
//! ### Why the `generate_handler!` call site still lists flat idents
//!
//! `tauri::generate_handler!` is a proc-macro that emits per-command
//! wrappers like `__cmd__$name`. It cannot see through a
//! `pub use A::*` re-export at proc-macro expansion time — the
//! identifiers must already be resolvable at the invocation site. So
//! the flat list in `commands_registry.rs` stays, but the **source of
//! truth for "what does domain X expose"** moves here.
//!
//! ### Adding a new domain
//!
//! 1. Create `commands_facade/<domain>.rs` with `pub use` lines for
//!    every `#[tauri::command]` in the domain.
//! 2. Declare it here: `pub mod <domain>;`.
//! 3. Add the command identifiers to `commands_registry.rs` under a
//!    `// ── <domain> ──` section header.
//!
//! ### Adding a new command to an existing domain
//!
//! 1. Write `#[tauri::command]` in its home module.
//! 2. Add one `pub use` line in `commands_facade/<domain>.rs`.
//! 3. Add the identifier to `commands_registry.rs`.
//!
//! See also: `docs/development.md#adding-a-new-tauri-command-ipc`.

pub mod ai;
pub mod asset_intel;
pub mod evidence;
pub mod findings;
pub mod git_pty;
pub mod indexer;
pub mod integrations;
pub mod intel_providers;
pub mod mcp;
pub mod pentest;
pub mod settings;
pub mod sidecar;
pub mod vault;
pub mod vuln_intel;
pub mod wiki;
pub mod workspace;
