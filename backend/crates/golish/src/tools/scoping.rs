//! Shared project-scoping (IDOR / ownership) guards for Tauri command CRUD.
//!
//! Enforces AGENTS.md invariant **I2**: every mutation (and sensitive read) must
//! verify the target row belongs to the caller's `project_path` before acting.
//! The actual scope predicate lives in each command's SQL (so it can mirror the
//! corresponding list query exactly); these helpers turn the result of a scoped
//! query into a loud `NotFound` instead of a silent no-op.

use crate::error::GolishError;

const SCOPE_NOT_FOUND: &str = "resource not found in the current project";

/// Map a scoped mutation's affected-row count to a not-found error.
///
/// A scoped `UPDATE ... WHERE id = $1 AND project_path ...` (or `DELETE`)
/// affects zero rows when the row either does not exist or belongs to a
/// different project. Both cases are surfaced as `NotFound` so cross-project
/// access fails loudly rather than silently no-op'ing.
pub(crate) fn ensure_scoped_mutation(rows_affected: u64) -> Result<(), GolishError> {
    if rows_affected == 0 {
        Err(GolishError::NotFound(SCOPE_NOT_FOUND.to_string()))
    } else {
        Ok(())
    }
}

/// Unwrap a scoped existence-check row, mapping `None` (missing row or a row in a
/// different project) to a not-found error. Use for `SELECT ... WHERE id = $1 AND
/// project_path ...` guards in front of multi-statement updates and sensitive
/// reads.
pub(crate) fn ensure_scoped_found<T>(row: Option<T>) -> Result<T, GolishError> {
    row.ok_or_else(|| GolishError::NotFound(SCOPE_NOT_FOUND.to_string()))
}
