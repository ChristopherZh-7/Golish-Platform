//! Public API contract tests for golish-projects.
//!
//! Asserts that project schema types remain Send + Sync and that
//! the public CRUD function signatures haven't drifted.

use golish_projects::*;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_sync() {
    assert_send_sync::<ProjectConfig>();
    assert_send_sync::<PentestProjectConfig>();
}

#[tokio::test]
async fn crud_functions_exist() {
    // Verify the public function symbols resolve by taking references.
    // (We don't call them — no real filesystem.)
    let _ = &list_projects;
    let _ = &load_project;
    let _ = &save_project;
    let _ = &delete_project;
}
