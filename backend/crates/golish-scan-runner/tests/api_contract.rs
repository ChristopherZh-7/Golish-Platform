//! Public API contract tests for golish-scan-runner.
//!
//! Asserts that the `ScanStorage` trait and key scanner option types
//! remain Send + Sync and maintain expected signatures.

use golish_scan_runner::*;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_sync() {
    assert_send_sync::<ScanProgress>();
    assert_send_sync::<ScanResult>();
    assert_send_sync::<NucleiTemplateSelection>();
    assert_send_sync::<NucleiTemplateRationale>();
    assert_send_sync::<WhatWebOptions>();
    assert_send_sync::<FeroxScanOptions>();
}

#[test]
fn scan_storage_trait_is_object_safe() {
    assert_send_sync::<Box<dyn ScanStorage>>();
}
