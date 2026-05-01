//! Public API contract tests for golish-indexer.
//!
//! Asserts that the `IndexerBackend` trait and key types remain
//! Send + Sync and maintain expected signatures.

use golish_indexer::*;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_sync() {
    assert_send_sync::<IndexerState>();
    assert_send_sync::<CodeSearchResult>();
    assert_send_sync::<CodebaseInfo>();
    assert_send_sync::<IndexResult>();
    assert_send_sync::<IndexSearchResult>();
    assert_send_sync::<ProjectInfo>();
    assert_send_sync::<RecentDirectory>();
    assert_send_sync::<WorktreeCreated>();
    assert_send_sync::<BranchInfo>();
}

#[test]
fn indexer_backend_trait_is_object_safe() {
    assert_send_sync::<Box<dyn IndexerBackend>>();
}
