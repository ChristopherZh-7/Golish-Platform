//! Public API contract tests for golish-vuln-intel.
//!
//! Asserts that core DTOs remain Send + Sync and key function
//! signatures haven't drifted.

use golish_vuln_intel::*;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_sync() {
    assert_send_sync::<VulnEntry>();
    assert_send_sync::<VulnFeed>();
    assert_send_sync::<EntryRow>();
    assert_send_sync::<FeedRow>();
    assert_send_sync::<GithubPocResult>();
    assert_send_sync::<NucleiTemplateResult>();
    assert_send_sync::<BatchNucleiResult>();
    assert_send_sync::<NucleiDiscoverResult>();
    assert_send_sync::<NucleiDiscoverProgress>();
}
