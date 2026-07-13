use std::collections::BTreeSet;
use std::time::Duration;

use golish_agent_kit::harness::{
    load_embedded_stage_spec, RuntimeScopeSource, RuntimeUnitIdentity, StageKind,
    StageRuntimeContract,
};
use golish_db::models::{AgentType, NewSession};
use golish_db::repo::{
    canonical_fact_refs, operation_org_scope, organizations, project_scopes,
    runtime_memory_rollout, runtime_memory_tx, sessions, stage_deliverable_submissions,
    stage_handoffs, stage_run_units, stage_worker_runs, tool_calls,
};
use golish_db::{DbConfig, GolishDb};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FOUR_V2_STAGES: [StageKind; 4] = [
    StageKind::TargetIntel,
    StageKind::ExternalAttackSurface,
    StageKind::Enumeration,
    StageKind::VulnTriage,
];

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn fixture() -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let config = DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("runtime_four_stage_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    };
    let db = GolishDb::start(config)
        .await
        .expect("start migrated embedded postgres");
    (db, data_dir)
}

async fn advance_runtime_to_v2_only(db: &GolishDb) {
    use runtime_memory_rollout::RuntimeMemoryContract;

    let mut current = RuntimeMemoryContract::LegacyV1;
    let mut row_version = 0;
    for next in [
        RuntimeMemoryContract::DualWriteLegacyRead,
        RuntimeMemoryContract::DualWriteV2Preferred,
        RuntimeMemoryContract::V2Only,
    ] {
        let row = runtime_memory_rollout::advance(db.pool(), current, next, row_version)
            .await
            .unwrap_or_else(|error| {
                panic!("advance runtime rollout {current:?} -> {next:?}: {error}")
            });
        current = next;
        row_version = row.row_version;
    }
}

struct StageFixture {
    stage: StageKind,
    specialist: String,
    session_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    snapshot_id: Uuid,
    scope_hash: String,
    organization_ids: [Uuid; 2],
}

async fn create_stage_fixture(db: &GolishDb, stage: StageKind) -> StageFixture {
    let spec = load_embedded_stage_spec(stage)
        .unwrap_or_else(|error| panic!("{} embedded StageSpec: {error}", stage.as_str()));
    assert_eq!(
        spec.runtime_memory,
        Some(StageRuntimeContract {
            schema_version: 2,
            unit_identity: RuntimeUnitIdentity::StageExecutionOrganization,
            scope_source: RuntimeScopeSource::FrozenOperationSnapshot,
            requires_worker_lease: true,
            publishes_handoff_after_final_seal: true,
        }),
        "{} must opt into the exact embedded V2 runtime contract",
        stage.as_str()
    );
    let specialist = spec
        .specialist
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("{} embedded StageSpec specialist", stage.as_str()));

    let project_path = format!("/tmp/runtime-four-stage/{}", stage.as_str());
    let session_id = sessions::create(
        db.pool(),
        NewSession {
            title: Some(format!("{} V2 acceptance", stage.as_str())),
            workspace_path: Some(project_path.clone()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.clone()),
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{} create session: {error}", stage.as_str()))
    .id;
    let project = project_scopes::register_first_open(
        db.pool(),
        &project_path,
        &format!("{}-path-sha", stage.as_str()),
    )
    .await
    .unwrap_or_else(|error| panic!("{} register project: {error}", stage.as_str()));
    let root = organizations::create(
        db.pool(),
        &project_path,
        &format!("{} Root", stage.as_str()),
        None,
        "Runtime V2 acceptance root",
        "fixture",
    )
    .await
    .unwrap_or_else(|error| panic!("{} create root org: {error}", stage.as_str()));
    let child = organizations::create(
        db.pool(),
        &project_path,
        &format!("{} Child", stage.as_str()),
        Some(root.id),
        "Runtime V2 acceptance subsidiary",
        "fixture",
    )
    .await
    .unwrap_or_else(|error| panic!("{} create child org: {error}", stage.as_str()));
    let operation_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let created = runtime_memory_tx::create_runtime_operation(
        db.pool(),
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: stage_execution_id,
            session_id,
            title: Some(format!("{} frozen fanout", stage.as_str())),
            input: "accept the persisted runtime without provider dispatch".to_string(),
            profile: "assessment".to_string(),
            entry_stage: stage.as_str().to_string(),
            project_scope_id: project.project_scope_id,
            cli_scope: Some(runtime_memory_tx::CliRuntimeScopeRow {
                root_organization_id: root.id,
                include_subsidiaries: true,
                subsidiary_threshold: 51,
                units: vec![
                    runtime_memory_tx::CliRuntimeScopeUnitRow {
                        organization_id: root.id,
                        parent_organization_id: None,
                        organization_name: root.name.clone(),
                        depth: 0,
                        ordinal: 0,
                        ownership_percent: None,
                        approval_source: serde_json::json!({"source": "acceptance_fixture"}),
                    },
                    runtime_memory_tx::CliRuntimeScopeUnitRow {
                        organization_id: child.id,
                        parent_organization_id: Some(root.id),
                        organization_name: child.name.clone(),
                        depth: 1,
                        ordinal: 1,
                        ownership_percent: Some("75".to_string()),
                        approval_source: serde_json::json!({"source": "acceptance_fixture"}),
                    },
                ],
            }),
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{} create frozen operation: {error}", stage.as_str()));
    assert_eq!(created.operation.runtime_memory_contract, "v2_only");
    assert_eq!(created.operation.current_stage, stage.as_str());

    let frozen = operation_org_scope::load_for_operation(db.pool(), operation_id)
        .await
        .unwrap_or_else(|error| panic!("{} load frozen scope: {error}", stage.as_str()))
        .unwrap_or_else(|| panic!("{} frozen scope missing", stage.as_str()));
    assert!(
        frozen.snapshot.sealed_at.is_some(),
        "{} frozen scope must be sealed before fanout",
        stage.as_str()
    );
    assert_eq!(frozen.snapshot.operation_id, operation_id);
    assert_eq!(frozen.snapshot.project_scope_id, project.project_scope_id);
    assert_eq!(
        frozen
            .units
            .iter()
            .map(|unit| unit.organization_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([root.id, child.id]),
        "{} fanout denominator comes from the frozen snapshot",
        stage.as_str()
    );

    StageFixture {
        stage,
        specialist,
        session_id,
        operation_id,
        stage_execution_id,
        snapshot_id: frozen.snapshot.id,
        scope_hash: frozen.snapshot.scope_hash,
        organization_ids: [root.id, child.id],
    }
}

async fn claim_worker(
    db: &GolishDb,
    fixture: &StageFixture,
    seeded: &runtime_memory_tx::SeededStageRuntimeRow,
    lease_owner: &str,
    lease_seconds: i32,
) -> runtime_memory_tx::ClaimedWorkerAndChainRow {
    runtime_memory_tx::claim_worker_and_bind_chain(
        db.pool(),
        &runtime_memory_tx::ClaimWorkerAndBindChainRow {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: seeded.unit.id,
            worker_run_id: seeded.worker.id,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Queued,
            expected_unit_row_version: seeded.unit.row_version,
            expected_worker_status: stage_worker_runs::StageWorkerRunStatus::Queued,
            expected_attempt_epoch: seeded.worker.attempt_epoch,
            session_id: fixture.session_id,
            subtask_id: None,
            agent: AgentType::Pentester,
            model: None,
            provider: None,
            parent_chain_id: None,
            lease_owner: lease_owner.to_string(),
            lease_seconds,
            initial_chain: serde_json::json!([]),
            initial_checkpoint: serde_json::json!({"turn": 0}),
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{} claim {lease_owner}: {error}", fixture.stage.as_str()))
}

fn fence_for_claim(
    claimed: &runtime_memory_tx::ClaimedWorkerAndChainRow,
) -> runtime_memory_tx::RuntimeMemoryTxFence {
    runtime_memory_tx::RuntimeMemoryTxFence {
        operation_id: claimed.worker.operation_id,
        stage_execution_id: claimed.worker.stage_execution_id,
        stage_run_unit_id: claimed.worker.stage_run_unit_id,
        worker_run_id: claimed.worker.id,
        lease_token: claimed.worker.lease_token.expect("claimed lease token"),
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: claimed.worker.checkpoint_version,
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).expect("serialize JSON string")
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize object key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_text(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_json(value: &serde_json::Value) -> String {
    sha256_text(&canonical_json(value))
}

async fn persist_local_submission(
    db: &GolishDb,
    fixture: &StageFixture,
    claimed: &runtime_memory_tx::ClaimedWorkerAndChainRow,
    fence: &runtime_memory_tx::RuntimeMemoryTxFence,
) -> stage_deliverable_submissions::StageDeliverableSubmissionRow {
    let request_id = format!("{}-local-submit", fixture.stage.as_str());
    let tool_call_id = tool_calls::record_tracked_start(
        db.pool(),
        &request_id,
        fixture.session_id,
        Some(fixture.operation_id),
        None,
        "submit_stage_deliverable",
        &serde_json::json!({"stage_id": fixture.stage.as_str()}),
        Some(&tool_calls::RuntimeToolIdentity {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: Some(claimed.unit.id),
            worker_run_id: Some(claimed.worker.id),
            organization_id: Some(claimed.unit.organization_id),
            attempt_epoch: Some(claimed.worker.attempt_epoch),
            lease_token: claimed.worker.lease_token,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("{} record local submit: {error}", fixture.stage.as_str()));
    runtime_memory_tx::begin_worker_tool(db.pool(), fence, tool_call_id)
        .await
        .unwrap_or_else(|error| panic!("{} begin submit fence: {error}", fixture.stage.as_str()));

    let payload = serde_json::json!({
        "stage_id": fixture.stage.as_str(),
        "stage_run_id": fixture.stage_execution_id,
        "claims": [],
    });
    let canonical_payload_json = canonical_json(&payload);
    let submission = stage_deliverable_submissions::insert(
        db.pool(),
        &stage_deliverable_submissions::NewStageDeliverableSubmission {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: Some(claimed.unit.id),
            worker_run_id: Some(claimed.worker.id),
            organization_id: Some(claimed.unit.organization_id),
            tool_call_record_id: tool_call_id,
            tool_request_id: request_id,
            stage_kind: fixture.stage.as_str().to_string(),
            attempt_epoch: Some(claimed.worker.attempt_epoch),
            lease_token: claimed.worker.lease_token,
            payload_sha256: sha256_text(&canonical_payload_json),
            canonical_payload_json,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{} persist local submit: {error}", fixture.stage.as_str()));
    runtime_memory_tx::finish_worker_tool(db.pool(), fence, tool_call_id)
        .await
        .unwrap_or_else(|error| panic!("{} finish submit fence: {error}", fixture.stage.as_str()));
    tool_calls::record_tracked_finish(
        db.pool(),
        tool_call_id,
        fixture.session_id,
        "finished",
        "{}",
        1,
    )
    .await
    .unwrap_or_else(|error| panic!("{} finish tracked submit: {error}", fixture.stage.as_str()));
    submission
}

fn final_seal_input(
    fixture: &StageFixture,
    claimed: &runtime_memory_tx::ClaimedWorkerAndChainRow,
    fence: runtime_memory_tx::RuntimeMemoryTxFence,
    submission_id: Uuid,
) -> runtime_memory_tx::FinalizeUnitPassRow {
    let canonical_fact_keys = Vec::<canonical_fact_refs::CanonicalFactKey>::new();
    let typed_claims = Vec::<serde_json::Value>::new();
    let coverage_watermark = serde_json::json!({
        "stage": fixture.stage.as_str(),
        "organization_id": claimed.unit.organization_id,
        "terminal_cells": 0,
    });
    let evidence_ids = Vec::<i64>::new();
    let terminal_checkpoint = serde_json::json!({"terminal": true});
    let details = serde_json::json!({});
    let seal_material = serde_json::json!({
        "canonical_fact_keys": canonical_fact_keys,
        "typed_claims": typed_claims,
        "coverage_watermark": coverage_watermark,
        "evidence_ids": evidence_ids,
        "terminal_checkpoint": terminal_checkpoint,
        "deterministic_gate_details": details,
        "candidate_acceptance": serde_json::Value::Null,
    });
    let gate_decision = serde_json::json!({
        "outcome": "pass",
        "operation_id": fixture.operation_id,
        "stage_execution_id": fixture.stage_execution_id,
        "stage_run_unit_id": claimed.unit.id,
        "deliverable_submission_id": submission_id,
        "scope_hash": fixture.scope_hash,
        "seal_material_sha256": sha256_json(&seal_material),
        "details": details,
    });
    runtime_memory_tx::FinalizeUnitPassRow {
        fence,
        deliverable_submission_id: submission_id,
        expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
        expected_unit_row_version: claimed.unit.row_version,
        scope_hash: fixture.scope_hash.clone(),
        gate_decision_hash: sha256_json(&gate_decision),
        gate_decision,
        aggregate_pass_token_hash: None,
        canonical_fact_keys,
        typed_claims,
        coverage_watermark,
        evidence_ids,
        terminal_checkpoint,
        candidate_acceptance: None,
    }
}

async fn exercise_stage(db: &GolishDb, stage: StageKind) {
    let fixture = create_stage_fixture(db, stage).await;
    let seed_input = runtime_memory_tx::SeedStageRuntimeRow {
        operation_id: fixture.operation_id,
        stage_execution_id: fixture.stage_execution_id,
        stage_kind: stage.as_str().to_string(),
        unit_generation: 1,
        specialist: fixture.specialist.clone(),
        worker_generation: 1,
        work_item_kind: "organization".to_string(),
        work_item_key: stage.as_str().to_string(),
        agent_path_prefix: format!("main>stage_run:{}", stage.as_str()),
    };
    let seeded = runtime_memory_tx::seed_stage_runtime(db.pool(), &seed_input)
        .await
        .unwrap_or_else(|error| panic!("{} seed frozen fanout: {error}", stage.as_str()));
    assert_eq!(
        seeded.len(),
        2,
        "{} one Unit per frozen org",
        stage.as_str()
    );
    assert_eq!(
        seeded
            .iter()
            .map(|row| row.unit.organization_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(fixture.organization_ids),
        "{} caller cannot shrink frozen fanout",
        stage.as_str()
    );
    for row in &seeded {
        assert_eq!(row.unit.operation_id, fixture.operation_id);
        assert_eq!(row.unit.stage_execution_id, fixture.stage_execution_id);
        assert_eq!(row.unit.scope_snapshot_id, fixture.snapshot_id);
        assert_eq!(row.unit.stage_kind, stage.as_str());
        assert_eq!(
            row.unit.specialist.as_deref(),
            Some(fixture.specialist.as_str())
        );
        assert_eq!(row.unit.status, "queued");
        assert_eq!(row.worker.stage_run_unit_id, row.unit.id);
        assert_eq!(row.worker.organization_id, row.unit.organization_id);
        assert_eq!(row.worker.specialist, fixture.specialist);
        assert_eq!(row.worker.status, "queued");
    }

    let first = claim_worker(db, &fixture, &seeded[0], "acceptance-pass", 300).await;
    let second = claim_worker(db, &fixture, &seeded[1], "acceptance-incomplete", 1).await;
    for claimed in [&first, &second] {
        assert_eq!(claimed.unit.status, "running");
        assert_eq!(claimed.worker.status, "running");
        assert!(claimed.worker.lease_token.is_some());
        let heartbeat_at = claimed
            .worker
            .heartbeat_at
            .expect("claimed worker heartbeat timestamp");
        let lease_expires_at = claimed
            .worker
            .lease_expires_at
            .expect("claimed worker lease expiry");
        assert!(
            lease_expires_at > heartbeat_at,
            "{} claimed worker has a live lease",
            stage.as_str()
        );
        assert_eq!(claimed.worker.organization_id, claimed.unit.organization_id);
    }
    assert_ne!(first.unit.id, second.unit.id);
    assert_ne!(first.worker.id, second.worker.id);
    assert_ne!(first.message_chain_id, second.message_chain_id);
    assert_ne!(first.worker.lease_token, second.worker.lease_token);

    let first_fence = fence_for_claim(&first);
    let stale_fence = runtime_memory_tx::RuntimeMemoryTxFence {
        lease_token: Uuid::new_v4(),
        ..first_fence.clone()
    };
    let stale = runtime_memory_tx::heartbeat_worker(db.pool(), &stale_fence, 60).await;
    assert!(
        matches!(
            stale,
            Err(runtime_memory_tx::RuntimeMemoryStoreError::LeaseLost { .. })
        ),
        "{} wrong lease token is fenced",
        stage.as_str()
    );

    let submission = persist_local_submission(db, &fixture, &first, &first_fence).await;
    assert_eq!(submission.operation_id, fixture.operation_id);
    assert_eq!(submission.stage_execution_id, fixture.stage_execution_id);
    assert_eq!(submission.stage_run_unit_id, Some(first.unit.id));
    assert_eq!(submission.worker_run_id, Some(first.worker.id));
    assert_eq!(submission.organization_id, Some(first.unit.organization_id));
    assert_eq!(submission.attempt_epoch, Some(first.worker.attempt_epoch));
    assert_eq!(submission.lease_token, first.worker.lease_token);

    let seal_input = final_seal_input(&fixture, &first, first_fence, submission.id);
    let sealed = runtime_memory_tx::finalize_unit_pass(db.pool(), &seal_input)
        .await
        .unwrap_or_else(|error| panic!("{} exact final seal: {error}", stage.as_str()));
    assert!(!sealed.replayed);
    assert_eq!(sealed.unit.status, "passed");
    assert_eq!(sealed.worker.status, "passed");
    assert_eq!(sealed.worker.lease_token, None);
    assert_eq!(sealed.handoff.operation_id, fixture.operation_id);
    assert_eq!(sealed.handoff.organization_id, first.unit.organization_id);
    assert_eq!(sealed.handoff.scope_snapshot_id, fixture.snapshot_id);
    assert_eq!(sealed.handoff.from_stage_kind, stage.as_str());
    assert_eq!(
        sealed.handoff.stage_execution_id,
        fixture.stage_execution_id
    );
    assert_eq!(sealed.handoff.source_stage_run_unit_id, first.unit.id);
    assert_eq!(sealed.handoff.deliverable_submission_id, submission.id);
    assert_eq!(sealed.handoff.scope_hash, fixture.scope_hash);
    assert_eq!(sealed.handoff.invalidated_at, None);
    assert_eq!(
        sealed.handoff.payload_sha256,
        sha256_json(&sealed.handoff.payload)
    );
    let inherited = stage_handoffs::list_latest_final_sealed_for_sources(
        db.pool(),
        fixture.operation_id,
        first.unit.organization_id,
        &[stage.as_str().to_string()],
    )
    .await
    .unwrap_or_else(|error| panic!("{} reload exact handoff: {error}", stage.as_str()));
    assert_eq!(inherited.len(), 1);
    assert_eq!(inherited[0].id, sealed.handoff.id);
    let replayed = runtime_memory_tx::finalize_unit_pass(db.pool(), &seal_input)
        .await
        .unwrap_or_else(|error| panic!("{} replay exact final seal: {error}", stage.as_str()));
    assert!(replayed.replayed);
    assert_eq!(replayed.handoff.id, sealed.handoff.id);

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let second_fence = fence_for_claim(&second);
    let expired = runtime_memory_tx::heartbeat_worker(db.pool(), &second_fence, 60).await;
    assert!(
        matches!(
            expired,
            Err(runtime_memory_tx::RuntimeMemoryStoreError::LeaseLost { .. })
        ),
        "{} expired lease loses mutation authority",
        stage.as_str()
    );

    let restarted = runtime_memory_tx::seed_stage_runtime(db.pool(), &seed_input)
        .await
        .unwrap_or_else(|error| panic!("{} restart exact fanout: {error}", stage.as_str()));
    let passed = restarted
        .iter()
        .filter(|row| row.unit.status == "passed")
        .collect::<Vec<_>>();
    assert_eq!(
        passed.len(),
        1,
        "{} restart skips only PASS",
        stage.as_str()
    );
    assert_eq!(passed[0].unit.id, first.unit.id);
    assert_eq!(passed[0].worker.status, "passed");
    let incomplete = restarted
        .iter()
        .find(|row| row.unit.id == second.unit.id)
        .unwrap_or_else(|| panic!("{} incomplete org remains in fanout", stage.as_str()));
    assert_eq!(incomplete.unit.status, "running");
    assert_eq!(incomplete.worker.status, "running");

    let (disposition, requeued) = runtime_memory_tx::reap_expired_worker(
        db.pool(),
        &runtime_memory_tx::LoadWorkerCheckpointRow {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: incomplete.unit.id,
            worker_run_id: incomplete.worker.id,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{} reap incomplete worker: {error}", stage.as_str()));
    assert_eq!(
        disposition,
        stage_worker_runs::ExpiredWorkerDisposition::Requeued
    );
    assert_eq!(requeued.status, "queued");
    assert_eq!(requeued.lease_token, None);
    let live_unit = stage_run_units::get(db.pool(), incomplete.unit.id)
        .await
        .unwrap_or_else(|error| panic!("{} load incomplete unit: {error}", stage.as_str()))
        .unwrap_or_else(|| panic!("{} incomplete unit exists", stage.as_str()));
    assert_eq!(live_unit.status, "running");
    let reclaimed = runtime_memory_tx::claim_worker_and_bind_chain(
        db.pool(),
        &runtime_memory_tx::ClaimWorkerAndBindChainRow {
            operation_id: fixture.operation_id,
            stage_execution_id: fixture.stage_execution_id,
            stage_run_unit_id: incomplete.unit.id,
            worker_run_id: incomplete.worker.id,
            expected_unit_status: stage_run_units::StageRunUnitStatus::Running,
            expected_unit_row_version: live_unit.row_version,
            expected_worker_status: stage_worker_runs::StageWorkerRunStatus::Queued,
            expected_attempt_epoch: requeued.attempt_epoch,
            session_id: fixture.session_id,
            subtask_id: None,
            agent: AgentType::Pentester,
            model: None,
            provider: None,
            parent_chain_id: None,
            lease_owner: format!("restart:{}", stage.as_str()),
            lease_seconds: 300,
            initial_chain: serde_json::json!([{"must": "reuse existing chain"}]),
            initial_checkpoint: serde_json::json!({"must": "reuse existing checkpoint"}),
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{} reclaim incomplete worker: {error}", stage.as_str()));
    assert_eq!(reclaimed.worker.id, second.worker.id);
    assert_eq!(reclaimed.message_chain_id, second.message_chain_id);
    assert_eq!(
        reclaimed.worker.attempt_epoch,
        second.worker.attempt_epoch + 1
    );
    assert_ne!(reclaimed.worker.lease_token, second.worker.lease_token);
    assert_eq!(reclaimed.worker.status, "running");
    assert_eq!(reclaimed.unit.status, "running");
    let passed_after_reclaim = stage_run_units::get(db.pool(), first.unit.id)
        .await
        .unwrap_or_else(|error| panic!("{} reload passed unit: {error}", stage.as_str()))
        .unwrap_or_else(|| panic!("{} passed unit exists", stage.as_str()));
    assert_eq!(passed_after_reclaim.status, "passed");
}

#[tokio::test]
async fn four_embedded_v2_stages_accept_frozen_fanout_fencing_seal_and_restart() {
    let (mut db, _data_dir) = fixture().await;
    advance_runtime_to_v2_only(&db).await;

    let mut exercised = Vec::new();
    for stage in FOUR_V2_STAGES {
        exercise_stage(&db, stage).await;
        exercised.push(stage.as_str());
    }
    assert_eq!(
        exercised,
        vec![
            "target_intel",
            "external_attack_surface",
            "enumeration",
            "vuln_triage",
        ],
        "every embedded StageSpec contract must execute the full acceptance path"
    );

    db.stop().await;
}
