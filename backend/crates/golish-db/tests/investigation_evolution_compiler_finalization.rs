const COMPILER: &str = include_str!("../src/repo/investigation_hypothesis_compiler.rs");
const MIGRATION: &str =
    include_str!("../migrations/20260812000006_investigation_evolution_compiler_finalization.sql");
const APP_HOST: &str =
    include_str!("../../golish-agent-app/src/ai/db_bridge/investigation_analysis_host.rs");

#[test]
fn successor_generation_and_advanced_receipt_share_the_compiler_transaction() {
    let apply = COMPILER
        .split("pub async fn apply_investigation_compilation")
        .nth(1)
        .expect("canonical apply function");
    let advanced = apply
        .find("finalize_pending_evolution_advanced_on(")
        .expect("advanced finalizer call");
    let commit = apply[advanced..]
        .find("tx.commit().await?")
        .map(|offset| advanced + offset)
        .expect("transaction commit after advanced finalization");
    assert!(advanced < commit);
    assert!(COMPILER.contains("consolidation.disposition='advanced'"));
    assert!(COMPILER.contains("consolidation.successor_generation_id=$2"));
}

#[test]
fn zero_or_attach_only_evolution_closes_fixed_point_without_successor_generation() {
    assert!(COMPILER
        .contains("input.pending_evolution_authority_id.is_some() && new_revision_ids.is_empty()"));
    assert!(COMPILER.contains("finalize_pending_evolution_fixed_point_on"));
    assert!(COMPILER.contains("persist_proof_members_on("));
    assert!(MIGRATION.contains("investigation_evolution_fixed_point_apply_receipts"));
    assert!(MIGRATION.contains("INVESTIGATION_EVOLUTION_FIXED_POINT_AUTHORITY_INVALID"));
}

#[test]
fn replay_and_pending_authority_drift_fail_closed() {
    assert!(COMPILER.contains("load_evolution_fixed_point_replay_on"));
    assert!(COMPILER.contains("input.pending_evolution_authority_id"));
    assert!(COMPILER.contains("!= input.prepared.input.pending_evolution_authority_id"));
    assert!(COMPILER.contains("Some(row.1) != input.pending_evolution_authority_id"));
    assert!(COMPILER.contains("return Err(conflict(REPLAY_DRIFT))"));
    assert!(COMPILER.contains("source_seal.seal_id=snapshot.previous_generation_seal_id"));
    assert!(COMPILER.contains("operation.project_scope_id=pending.project_scope_id"));
}

#[test]
fn crash_recovery_accepts_only_the_exact_fixed_point_request_and_prepared_pending_authority() {
    assert!(APP_HOST.contains("load_committed_evolution_fixed_point_on"));
    assert!(APP_HOST.contains("fixed_apply.stable_request_id=$1"));
    assert!(APP_HOST.contains("fixed_apply.pending_evolution_authority_id=$2"));
    assert!(APP_HOST.contains("decision.stable_request_id=$4"));
    assert!(APP_HOST.contains("decision.binding_id=$5"));
    assert!(APP_HOST.contains("snapshot.previous_generation_seal_id"));
    assert!(APP_HOST.contains("fixed-point replay authority is partial or foreign"));
    assert!(APP_HOST.contains("recovery fixed-point replay authority is partial or foreign"));
    assert!(APP_HOST.contains("evolution_fixed_point: true"));
}

#[test]
fn terminal_pending_prepare_replay_requires_the_original_stable_authority() {
    assert!(APP_HOST.contains("replay_binding.stable_request_id=$5"));
    assert!(
        APP_HOST.contains("replay_binding.candidate_snapshot_id=candidate_snapshot.snapshot_id")
    );
    assert!(COMPILER.contains("replay_decision.stable_request_id=$5"));
    assert!(COMPILER.contains("replay_decision.candidate_snapshot_id=snapshot.snapshot_id"));
}
