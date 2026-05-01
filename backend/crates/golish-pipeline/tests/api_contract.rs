//! Public API contract tests for golish-pipeline.
//!
//! Asserts that the `PipelineStorage` trait and key DTOs remain
//! Send + Sync and maintain expected signatures.

use golish_pipeline::*;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_sync() {
    assert_send_sync::<Pipeline>();
    assert_send_sync::<PipelineStep>();
    assert_send_sync::<PipelineConnection>();
    assert_send_sync::<PipelineRunResult>();
    assert_send_sync::<StepResult>();
    assert_send_sync::<PipelineStepInfo>();
    assert_send_sync::<OutputParserConfig>();
    assert_send_sync::<ParsedItem>();
    assert_send_sync::<StoreStats>();
    assert_send_sync::<NoopStorage>();
}

#[test]
fn pipeline_storage_trait_is_object_safe() {
    assert_send_sync::<Box<dyn PipelineStorage>>();
}
