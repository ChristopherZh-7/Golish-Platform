//! Shared helpers to resolve the workspace root path used across the crate.
//!
//! Several layers (startup bootstrap, file commands, file watcher, command
//! completions, CLI args) all need to interpret the `QBIT_WORKSPACE`
//! environment variable and `~/`-style tilde expansion.  Centralising those
//! helpers here keeps behaviour consistent — in particular, tilde expansion
//! is always applied to `QBIT_WORKSPACE`, regardless of who calls in.
//!
//! # Resolution order
//!
//! All callers go through [`resolve_workspace_path_with_override`], which
//! applies the same priority chain:
//!
//! 1. `QBIT_WORKSPACE` environment variable (with `~/` expansion).
//! 2. The optional `cli_override` (e.g. `golish --workspace …` / the
//!    positional `[WORKSPACE]` argument).
//! 3. The process' current working directory.
//! 4. The user's home directory.
//!
//! The GUI passes `cli_override = None` and never validates existence (the
//! workspace path is also used for paths that the user is *about* to
//! create); the CLI passes `cli_override = Some(&args.workspace)` and
//! requires the resulting path to exist via
//! [`resolve_validated_workspace`].

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};

/// Resolve the active workspace path with an optional explicit override.
///
/// See the [module-level docs](self) for the full priority chain.
pub(crate) fn resolve_workspace_path_with_override(cli_override: Option<&Path>) -> PathBuf {
    if let Ok(env_workspace) = std::env::var("QBIT_WORKSPACE") {
        return expand_tilde(&env_workspace);
    }
    if let Some(path) = cli_override {
        return path.to_path_buf();
    }
    std::env::current_dir().unwrap_or_else(|_| dirs::home_dir().unwrap_or_default())
}

/// Resolve the active workspace path with no explicit override (GUI / app
/// startup path). Convenience wrapper around
/// [`resolve_workspace_path_with_override`].
pub(crate) fn resolve_workspace_path() -> PathBuf {
    resolve_workspace_path_with_override(None)
}

/// Resolve a path that may be either absolute or relative to the workspace
/// root. Used by file-CRUD commands.
pub(crate) fn resolve_workspace_path_with(path: &str) -> PathBuf {
    let target = Path::new(path);
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        resolve_workspace_path().join(target)
    }
}

/// Resolve the workspace path **and** verify it exists and is a directory.
/// Returns the canonicalised path, suitable as a stable absolute root.
///
/// Used by the headless CLI, where missing/typo'd workspaces should fail
/// fast with a clear error instead of silently degrading.
pub(crate) fn resolve_validated_workspace(cli_override: Option<&Path>) -> Result<PathBuf> {
    let raw = resolve_workspace_path_with_override(cli_override);
    let canonical = raw.canonicalize().map_err(|e| {
        anyhow!(
            "Workspace '{}' does not exist or is not accessible: {}",
            raw.display(),
            e
        )
    })?;
    if !canonical.is_dir() {
        bail!("Workspace '{}' is not a directory", canonical.display());
    }
    Ok(canonical)
}

/// Tilde-expansion helpers are the canonical ones in `golish-core::paths`;
/// re-exported here so existing `crate::app::workspace::expand_tilde*` call
/// sites stay stable.
pub(crate) use golish_core::paths::{expand_tilde, expand_tilde_string};
