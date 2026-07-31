#[allow(dead_code)]
#[path = "../src/ai/candidate_analysis_gate.rs"]
mod candidate_analysis_gate;
#[allow(dead_code)]
#[path = "../src/ai/candidate_analysis_projection.rs"]
mod candidate_analysis_projection;
#[allow(dead_code)]
#[path = "../src/ai/candidate_analysis_runtime.rs"]
mod candidate_analysis_runtime;
#[allow(dead_code)]
#[path = "../src/ai/db_bridge/hypothesis_registry.rs"]
mod hypothesis_registry_bridge;

use std::sync::Arc;

use async_trait::async_trait;
use candidate_analysis_gate::{AtomicCandidateFinalizer, PgCandidateGateSnapshotSource};
use candidate_analysis_runtime::{
    PgHypothesisAnalysisRuntimeRepository, PgHypothesisAnalysisStageRuntime,
};
use golish_agent_kit::{
    db_traits::HypothesisRegistryRepository,
    harness::{load_embedded_stage_spec, StageKind},
    task_orchestrator::hypothesis_analysis::*,
};
use golish_db::{
    models::NewSession,
    repo::{
        attack_waves, candidate_analysis_runtime as runtime_db, capability_execution_receipts,
        project_scopes, runtime_memory_tx, sessions, stage_asset_waves,
    },
    DbConfig, GolishDb,
};
use golish_pentest_domain::tool_truth::ToolTruthRootFamilyV1;
use serde_json::{json, Value};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

fn db_fence(
    authority: &CandidateRuntimeWorkAuthority,
) -> golish_db::repo::candidate_analysis::CandidateWriteFenceRow {
    let fence = &authority.fence;
    golish_db::repo::candidate_analysis::CandidateWriteFenceRow {
        operation_id: fence.operation_id,
        scope_snapshot_id: fence.scope_snapshot_id,
        organization_id: fence.organization_id,
        snapshot_id: fence.snapshot_id,
        team_plan_id: fence.team_plan_id,
        work_item_id: fence.work_item_id,
        worker_run_id: fence.worker_run_id,
        lease_token: fence.lease_token,
        lease_epoch: i64::try_from(fence.lease_epoch).expect("test lease epoch fits i64"),
        analysis_attempt_id: fence.analysis_attempt_id,
        analysis_attempt_ordinal: i32::try_from(fence.analysis_attempt_ordinal)
            .expect("test attempt ordinal fits i32"),
        attempt_epoch: i64::try_from(fence.attempt_epoch).expect("test attempt epoch fits i64"),
        expected_snapshot_row_version: fence.expected_snapshot_row_version,
        expected_team_plan_row_version: fence.expected_team_plan_row_version,
        expected_work_item_row_version: fence.expected_work_item_row_version,
        expected_worker_row_version: fence.expected_worker_row_version,
        expected_attempt_row_version: fence.expected_attempt_row_version,
    }
}

async fn db_hash(pool: &PgPool, value: &Value) -> String {
    sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
        .bind(value)
        .fetch_one(pool)
        .await
        .expect("hash fixture authority")
}

/// Begins a synthetic receipt against a normally sealed 30-minute policy.
/// This is a test-only future deployment fixture: every schema/business guard
/// remains enabled and validates the policy and receipt during insertion.
async fn begin_managed_with_extended_temporal_policy_fixture(
    pool: &PgPool,
    receipt_id: Uuid,
    denominator: &capability_execution_receipts::CoverageDenominatorRow,
    capability: &str,
    attempt_ordinal: i32,
    destination_policy_id: Uuid,
) -> capability_execution_receipts::CapabilityExecutionReceiptRow {
    let destination_policy_hash: String = sqlx::query_scalar(
        r#"SELECT policy_hash FROM capability_execution_destination_policies
            WHERE id=$1 AND denominator_id=$2 AND execution_authority_id=$3
              AND sealed_at IS NOT NULL"#,
    )
    .bind(destination_policy_id)
    .bind(denominator.id)
    .bind(denominator.execution_authority_id)
    .fetch_one(pool)
    .await
    .expect("load sealed destination policy");
    let member_hash = db_hash(
        pool,
        &json!({
            "fact_class":"target_state",
            "positive_ttl_ms":1_800_000_i64,
            "negative_ttl_ms":60_000_i64,
            "refutation_ttl_ms":60_000_i64,
            "same_epoch":true,
            "required_recheck_source":"manual_only",
        }),
    )
    .await;
    let policy_hash = db_hash(
        pool,
        &json!({
            "execution_authority_id":denominator.execution_authority_id,
            "max_cross_observation_skew_ms":30_000_i64,
            "members":[&member_hash],
        }),
    )
    .await;
    let policy_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin temporal policy fixture");
    sqlx::query(
        r#"INSERT INTO evidence_temporal_validity_policies(
               id,execution_authority_id,max_cross_observation_skew_ms,policy_hash)
           VALUES($1,$2,30000,$3)"#,
    )
    .bind(policy_id)
    .bind(denominator.execution_authority_id)
    .bind(&policy_hash)
    .execute(&mut *tx)
    .await
    .expect("insert extended temporal policy header");
    sqlx::query(
        r#"INSERT INTO evidence_temporal_validity_policy_members(
               id,policy_id,ordinal,fact_class,positive_ttl_ms,negative_ttl_ms,
               refutation_ttl_ms,require_same_target_state_epoch,
               required_recheck_source,member_hash)
           VALUES($1,$2,0,'target_state',1800000,60000,60000,TRUE,'manual_only',$3)"#,
    )
    .bind(Uuid::new_v4())
    .bind(policy_id)
    .bind(&member_hash)
    .execute(&mut *tx)
    .await
    .expect("insert extended temporal policy member");
    sqlx::query(
        "UPDATE evidence_temporal_validity_policies SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(policy_id)
    .execute(&mut *tx)
    .await
    .expect("seal extended temporal policy");
    let receipt_authority_hash = db_hash(
        pool,
        &json!({
            "denominator_id":denominator.id,
            "denominator_hash":denominator.denominator_hash,
            "execution_authority_id":denominator.execution_authority_id,
            "capability":capability,
            "attempt_ordinal":attempt_ordinal,
            "input_manifest_hash":denominator.input_manifest_hash,
            "destination_policy_hash":destination_policy_hash,
            "temporal_validity_policy_hash":policy_hash,
        }),
    )
    .await;
    let typed_landing = json!({"capability":capability,"state":"running"});
    let receipt =
        sqlx::query_as::<_, capability_execution_receipts::CapabilityExecutionReceiptRow>(
            r#"INSERT INTO capability_execution_receipts(
               id,denominator_id,execution_authority_id,capability,attempt_ordinal,
               receipt_authority_hash,input_manifest_hash,destination_policy_id,
               destination_policy_hash,temporal_validity_policy_id,
               temporal_validity_policy_hash,attempt_state,landing_state,
               observation_state,coverage_extent,coverage_gap_reason,
               reconciliation_state,security_interpretation,typed_landing)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'running',
                  'not_attempted','indeterminate','none','policy_blocked',
                  'pending','not_assessed',$12)
           RETURNING id,denominator_id,execution_authority_id,capability,attempt_ordinal,
                     receipt_authority_hash,input_manifest_hash,attempt_state,landing_state,
                     observation_state,coverage_extent,coverage_gap_reason,reconciliation_state,
                     security_interpretation,typed_landing,residual,finalization_request_hash,
                     current_semantic_authority_version,current_semantic_reconciliation_id,
                     current_semantic_reconciliation_hash,row_version,finalized_at"#,
        )
        .bind(receipt_id)
        .bind(denominator.id)
        .bind(denominator.execution_authority_id)
        .bind(capability)
        .bind(attempt_ordinal)
        .bind(receipt_authority_hash)
        .bind(&denominator.input_manifest_hash)
        .bind(destination_policy_id)
        .bind(destination_policy_hash)
        .bind(policy_id)
        .bind(&policy_hash)
        .bind(typed_landing)
        .fetch_one(&mut *tx)
        .await
        .expect("insert guarded extended-policy receipt");
    tx.commit().await.expect("commit temporal policy fixture");
    receipt
}

fn compile_bundle_denominator(
    _stage: &str,
    assets: &[capability_execution_receipts::LockedDenominatorAsset],
) -> anyhow::Result<Vec<capability_execution_receipts::CompiledDenominatorItem>> {
    Ok(assets
        .iter()
        .map(
            |asset| capability_execution_receipts::CompiledDenominatorItem {
                input_key: format!(
                    "{}\u{1f}{}\u{1f}GOLISH-INTEL-DNS",
                    asset.target_id, asset.exact_asset
                ),
                target_id: asset.target_id,
                exact_asset: asset.exact_asset.clone(),
                technique: "GOLISH-INTEL-DNS".to_owned(),
                expected_capability: "intel.dns".to_owned(),
            },
        )
        .collect())
}

async fn seed_fresh_bundle_root(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    project_path: &str,
    family: ToolTruthRootFamilyV1,
) {
    let stage_execution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,$3,'started')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .bind(family.stage_kind())
    .execute(pool)
    .await
    .expect("insert Tool Truth root stage");
    let wave = stage_asset_waves::current_or_create_initial(
        pool,
        operation_id,
        organization_id,
        family.stage_kind(),
        chrono::Utc::now() + chrono::Duration::seconds(1),
        100,
    )
    .await
    .expect("create root wave")
    .expect("root wave has target");
    let denominator = capability_execution_receipts::seal_source_denominator(
        pool,
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: Uuid::new_v5(
                &stage_execution_id,
                format!("candidate-e2e:{}", family.as_str()).as_bytes(),
            ),
            stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                wave.wave.id,
            ),
        },
        compile_bundle_denominator,
    )
    .await
    .expect("seal root denominator");
    let policy = capability_execution_receipts::seal_fixed_provider_destination_policy(
        pool,
        &capability_execution_receipts::SealFixedProviderDestinationPolicy {
            denominator_id: denominator.id,
            capability: "intel.dns".to_owned(),
            endpoints: vec![capability_execution_receipts::FixedProviderEndpoint {
                scheme: "https".to_owned(),
                normalized_host: "fixed.provider.example.test".to_owned(),
                port: 443,
                path_prefix: "/v1/query".to_owned(),
            }],
        },
    )
    .await
    .expect("seal destination policy");
    let receipt = begin_managed_with_extended_temporal_policy_fixture(
        pool,
        Uuid::new_v4(),
        &denominator,
        "intel.dns",
        1,
        policy.id,
    )
    .await;
    let input_keys: Vec<String> = sqlx::query_scalar(
        "SELECT input_key FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY input_key",
    )
    .bind(denominator.id)
    .fetch_all(pool)
    .await
    .expect("load root denominator members");
    capability_execution_receipts::finalize_target_intel_receipt(
        pool,
        &capability_execution_receipts::FinalizeTargetIntelReceipt {
            receipt_id: receipt.id,
            expected_row_version: receipt.row_version,
            attempt_fence: None,
            raw_witness: capability_execution_receipts::RawWitnessArtifactInput {
                artifact_id: Uuid::new_v5(&receipt.id, b"candidate-e2e-witness"),
                content_key: digest('1'),
                vault_object_ref_token: vec![7; 32],
                vault_object_ref_token_hash: digest('2'),
                sha256: digest('1'),
                ciphertext_sha256: digest('3'),
                operation_key_ref_hash: digest('4'),
                key_generation: 1,
                retention_policy_id: Uuid::new_v5(&receipt.id, b"candidate-e2e-retention"),
                retention_policy_hash: digest('5'),
                sensitivity_disposition: "typed_derivative_ready".to_owned(),
                original_byte_count: 2,
                stored_byte_count: 2,
                truncated: false,
            },
            network_hops: vec![capability_execution_receipts::ObservedNetworkHopInput {
                hop_kind: "initial".to_owned(),
                scheme: "https".to_owned(),
                normalized_host: "fixed.provider.example.test".to_owned(),
                port: 443,
                path_and_query: "/v1/query?input=candidate.example.test".to_owned(),
                addresses: vec!["1.1.1.1".parse().expect("fixture IP")],
                selected_address: "1.1.1.1".parse().expect("fixture IP"),
                send_ordinal: 1,
            }],
            request_count: 1,
            response_byte_count: 2,
            wall_clock_ms: 1,
            retry_count: 0,
            parser_complete: true,
            normalized_record_count: 1,
            input_observations: input_keys
                .into_iter()
                .map(
                    |input_key| capability_execution_receipts::TargetIntelInputObservation {
                        input_key,
                        technique: "GOLISH-INTEL-DNS".to_owned(),
                        observation_state: "found".to_owned(),
                    },
                )
                .collect(),
            typed_landing: json!({
                "application_products":[{
                    "subject_kind":"service",
                    "subject_identity_hash":digest('a'),
                    "product_identity":"nginx",
                    "cpe_candidates":["cpe:2.3:a:nginx:nginx:*:*:*:*:*:*:*:*"],
                    "observed_version":"1.2.3"
                }],
                "project_path":project_path,
            }),
            failure_reason_code: None,
        },
    )
    .await
    .expect("finalize synthetic root receipt");
}

async fn seed_managed_feed_authority(pool: &PgPool) {
    let catalog_id = Uuid::new_v4();
    let trust_policy_id = Uuid::new_v4();
    let sources = [
        ("cve", "managed:cve"),
        ("cpe", "managed:cpe"),
        ("kev", "managed:kev"),
        ("vendor_advisory", "managed:vendor-advisory"),
        ("detection_rule", "managed:detection-rule"),
    ];
    let mut source_kinds = sources
        .iter()
        .map(|(kind, _)| (*kind).to_owned())
        .collect::<Vec<_>>();
    source_kinds.sort();
    let source_set_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(&source_kinds)
            .fetch_one(pool)
            .await
            .expect("hash feed source set");
    let member_hashes = (1..=5)
        .map(|ordinal| digest(char::from_digit(ordinal, 16).expect("hex ordinal")))
        .collect::<Vec<_>>();
    let member_set_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(&member_hashes)
            .fetch_one(pool)
            .await
            .expect("hash feed member set");
    sqlx::query(
        r#"INSERT INTO candidate_managed_feed_catalogs(
               catalog_id,catalog_version,catalog_hash,trust_policy_id,
               trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
               required_source_count,required_source_set_hash,required_member_count,
               required_member_set_hash)
           VALUES($1,1,$2,$3,1,$4,$5,5,$6,5,$7)"#,
    )
    .bind(catalog_id)
    .bind(digest('a'))
    .bind(trust_policy_id)
    .bind(digest('b'))
    .bind(digest('c'))
    .bind(source_set_hash)
    .bind(&member_set_hash)
    .execute(pool)
    .await
    .expect("insert feed catalog");
    sqlx::query(
        r#"INSERT INTO candidate_managed_feed_catalog_head
           SELECT TRUE,catalog_id,catalog_version,catalog_hash,trust_policy_id,
                  trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
                  required_source_count,required_source_set_hash,required_member_count,
                  required_member_set_hash
             FROM candidate_managed_feed_catalogs WHERE catalog_id=$1"#,
    )
    .bind(catalog_id)
    .execute(pool)
    .await
    .expect("install feed catalog head");
    sqlx::query(
        r#"INSERT INTO candidate_managed_feed_trust_stores(
               trust_store_version,trust_store_hash,key_revocation_epoch,
               key_revocation_epoch_hash) VALUES(1,$1,0,$2)"#,
    )
    .bind(digest('d'))
    .bind(digest('e'))
    .execute(pool)
    .await
    .expect("insert feed trust store");
    sqlx::query(
        r#"INSERT INTO candidate_managed_feed_trust_store_head(
               singleton,trust_store_version,trust_store_hash,key_revocation_epoch,
               key_revocation_epoch_hash) VALUES(TRUE,1,$1,0,$2)"#,
    )
    .bind(digest('d'))
    .bind(digest('e'))
    .execute(pool)
    .await
    .expect("install feed trust head");
    sqlx::query(
        r#"INSERT INTO candidate_managed_feed_signer_keys(
               signer_key_member_id,trust_store_version,trust_store_hash,
               key_revocation_epoch,key_revocation_epoch_hash,signer_id,signer_key_id,
               signature_algorithm,revoked,key_member_hash)
           VALUES($1,1,$2,0,$3,'managed-signer','managed-key','ed25519',FALSE,$4)"#,
    )
    .bind(Uuid::new_v4())
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(digest('f'))
    .execute(pool)
    .await
    .expect("insert feed signer");
    for (ordinal, ((source_kind, source_identity), member_hash)) in
        sources.into_iter().zip(member_hashes).enumerate()
    {
        let catalog_member_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_catalog_members(
                   catalog_member_id,catalog_id,ordinal,source_kind,source_identity,
                   schema_name,schema_version,member_hash)
               VALUES($1,$2,$3,$4,$5,'managed_knowledge_feed.v1',1,$6)"#,
        )
        .bind(catalog_member_id)
        .bind(catalog_id)
        .bind(i32::try_from(ordinal).expect("feed ordinal"))
        .bind(source_kind)
        .bind(source_identity)
        .bind(&member_hash)
        .execute(pool)
        .await
        .expect("insert feed catalog member");
        let store_member_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_store_members(
                   store_member_id,catalog_member_id,catalog_id,feed_id,source_id,
                   feed_schema,feed_version,published_at,host_ingested_at,
                   effective_valid_until,content_hash,signed_manifest_hash,signer_id,
                   signer_key_id,signature_algorithm,signature_verification_receipt_hash,
                   signer_key_member_hash,provenance,age_policy_version,age_policy_digest,
                   immutable_feed_body,member_hash)
               VALUES($1,$2,$3,$4,$5,'managed_knowledge_feed.v1',1,
                      statement_timestamp()-INTERVAL '1 minute',statement_timestamp(),
                      statement_timestamp()+INTERVAL '1 hour',$6,$7,'managed-signer',
                      'managed-key','ed25519',$8,$9,'{}','v1',$10,$11,$12)"#,
        )
        .bind(store_member_id)
        .bind(catalog_member_id)
        .bind(catalog_id)
        .bind(format!("feed:{source_kind}"))
        .bind(source_identity)
        .bind(digest('1'))
        .bind(digest('2'))
        .bind(digest('3'))
        .bind(digest('f'))
        .bind(digest('4'))
        .bind(json!({"entries":[{
            "entry_kind":source_kind,
            "entry_id":format!("{source_kind}:nginx:1.2.3"),
            "entry_version":"1",
            "cpe":"cpe:2.3:a:nginx:nginx:*:*:*:*:*:*:*:*",
            "affected_versions":["1.2.3"],
            "matched_range":"exact"
        }]}))
        .bind(&member_hash)
        .execute(pool)
        .await
        .expect("insert feed store member");
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_store_member_heads(
                   catalog_member_id,catalog_id,store_member_id) VALUES($1,$2,$3)"#,
        )
        .bind(catalog_member_id)
        .bind(catalog_id)
        .bind(store_member_id)
        .execute(pool)
        .await
        .expect("install feed store head");
    }
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("candidate_runtime_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

/// Install the coherent deployment defaults expected after the future
/// Candidate cutover. This is test-only deployment-state construction in a
/// brand-new ephemeral PostgreSQL instance: it deliberately does not invoke or
/// test any production rollout-promotion path. Every guard is restored in the
/// same transaction before any operation or business row can be created.
async fn install_future_candidate_deployment_fixture(pool: &PgPool) {
    let mut tx = pool.begin().await.expect("begin future deployment fixture");
    for statement in [
        "ALTER TABLE runtime_memory_rollout DISABLE TRIGGER runtime_memory_rollout_forward_only",
        "ALTER TABLE runtime_memory_rollout DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate",
        "ALTER TABLE runtime_memory_rollout DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt",
        "ALTER TABLE attack_execution_rollout DISABLE TRIGGER attack_execution_rollout_forward_only",
        "ALTER TABLE attack_execution_rollout DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect("disable deployment-state fixture guard");
    }
    sqlx::query(
        r#"UPDATE runtime_memory_rollout
              SET contract='dual_write_legacy_read',contract_rank=1,
                  row_version=1,updated_at=NOW()
            WHERE singleton_id=1"#,
    )
    .execute(&mut *tx)
    .await
    .expect("install compatible runtime deployment fixture");
    sqlx::query(
        r#"UPDATE attack_execution_rollout
              SET contract='dual_write_read_legacy',rank=1,
                  row_version=1,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *tx)
    .await
    .expect("install compatible attack deployment fixture");
    sqlx::query(
        r#"UPDATE tool_truth_rollout
              SET new_operation_contract='receipt_v1',row_version=row_version+1,
                  updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *tx)
    .await
    .expect("install receipt Tool Truth deployment fixture");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',
                  rollout_mode='registry_authoritative_legacy_projection',mode_rank=3,
                  row_version=row_version+1,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(&mut *tx)
    .await
    .expect("install Registry-authoritative deployment fixture");
    for statement in [
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE attack_execution_rollout ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt",
        "ALTER TABLE attack_execution_rollout ENABLE TRIGGER attack_execution_rollout_forward_only",
        "ALTER TABLE runtime_memory_rollout ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt",
        "ALTER TABLE runtime_memory_rollout ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate",
        "ALTER TABLE runtime_memory_rollout ENABLE TRIGGER runtime_memory_rollout_forward_only",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect("restore deployment-state fixture guard");
    }
    tx.commit()
        .await
        .expect("commit coherent future deployment fixture");

    let guard_names = [
        "runtime_memory_rollout_forward_only",
        "zz_runtime_memory_rollout_attestation_gate",
        "zz_runtime_memory_rollout_promotion_receipt",
        "attack_execution_rollout_forward_only",
        "zz_attack_execution_rollout_promotion_receipt",
        "tool_truth_rollout_direct_mutation_guard",
        "investigation_rollout_direct_mutation_guard",
        "operation_state_joint_contract_insert_guard",
        "operation_state_tool_truth_contract_immutable",
        "operation_state_investigation_contract_immutable",
        "aa_attack_candidate_stage_unit_cutoff",
    ];
    let enabled: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT FROM pg_trigger WHERE tgname=ANY($1) AND tgenabled='O'",
    )
    .bind(guard_names)
    .fetch_one(pool)
    .await
    .expect("inspect restored future fixture guards");
    assert_eq!(enabled, guard_names.len() as i64);
}

#[derive(Debug, Clone, Copy)]
struct FutureRuntimeFixture {
    operation_id: Uuid,
    organization_id: Uuid,
    scope_snapshot_id: Uuid,
    stage_execution_id: Uuid,
}

async fn seed_future_runtime_upstream_fixture(pool: &PgPool, label: &str) -> FutureRuntimeFixture {
    install_future_candidate_deployment_fixture(pool).await;
    let operation_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let candidate_stage_execution_id = Uuid::new_v4();
    let project_path = format!("/tmp/{label}-{}", Uuid::new_v4().simple());

    let session = sessions::create(
        pool,
        NewSession {
            title: Some("Candidate E2E".to_owned()),
            workspace_path: Some(project_path.clone()),
            workspace_label: None,
            model: None,
            provider: None,
            project_path: Some(project_path.clone()),
        },
    )
    .await
    .expect("create Candidate runtime session");
    let project_scope = project_scopes::register_first_open(pool, &project_path, &digest('1'))
        .await
        .expect("register Candidate project scope");
    let project_scope_id = project_scope.project_scope_id;
    runtime_memory_tx::create_runtime_operation(
        pool,
        &runtime_memory_tx::CreateRuntimeOperationRow {
            operation_id,
            initial_stage_execution_id: candidate_stage_execution_id,
            session_id: session.id,
            title: Some("Candidate E2E".to_owned()),
            input: "candidate runtime".to_owned(),
            profile: "assessment".to_owned(),
            entry_stage: "attack_candidate".to_owned(),
            project_scope_id,
            cli_scope: None,
        },
    )
    .await
    .expect("create operation through the production runtime transaction");
    let frozen_contract: (String, String, String, String) = sqlx::query_as(
        r#"SELECT runtime_memory_contract,attack_execution_contract,
                  tool_truth_contract,investigation_rollout_mode
             FROM operation_state WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await
    .expect("load frozen future operation contract");
    assert_eq!(
        frozen_contract,
        (
            "dual_write_legacy_read".to_owned(),
            "dual_write_read_legacy".to_owned(),
            "receipt_v1".to_owned(),
            "registry_authoritative_legacy_projection".to_owned(),
        )
    );
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,'Runtime Org')")
        .bind(organization_id)
        .bind(&project_path)
        .execute(pool)
        .await
        .expect("insert future organization");

    let source_stage_execution_id = Uuid::new_v4();
    let source_stage_run_unit_id = Uuid::new_v4();
    let source_worker_run_id = Uuid::new_v4();
    let source_lease_token = Uuid::new_v4();
    let source_tool_call_id = Uuid::new_v4();
    let source_session_id = session.id;
    let source_submission_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'vuln_triage','completed')",
    )
    .bind(source_stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert upstream and Candidate stage runs");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,root_organization_id,
               mode,decision_rows,decision_hash)
           VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(source_stage_execution_id)
    .bind(organization_id)
    .bind(json!([{"organization_id": organization_id}]))
    .bind(digest('2'))
    .execute(pool)
    .await
    .expect("insert future scope decision");
    let mut scope_tx = pool.begin().await.expect("begin future scope fixture");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,project_path_at_freeze,
               root_organization_id,mode,scope_hash)
           VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_decision_id)
    .bind(&project_path)
    .bind(organization_id)
    .bind(digest('3'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert future scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,role,depth,ordinal,
               decision_row_id,approval_source)
           VALUES($1,$2,'Runtime Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(json!({"source":"fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert future scope unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal future scope snapshot");
    scope_tx
        .commit()
        .await
        .expect("commit future scope fixture");

    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status,started_at,terminal_at,pass_watermark)
           VALUES($1,$2,$3,$4,$5,'vuln_triage',0,'vuln-triage-fixture','passed',
                  NOW()-INTERVAL '1 minute',NOW(),$6)"#,
    )
    .bind(source_stage_run_unit_id)
    .bind(operation_id)
    .bind(source_stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(json!({"final_gate_passed":true,"deliverable_submission_id":source_submission_id}))
    .execute(pool)
    .await
    .expect("insert final-passed vuln triage unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,organization_id,
               worker_generation,specialist,work_item_kind,work_item_key,agent_path,
               status,lease_token,lease_owner,lease_acquired_at,lease_expires_at,
               heartbeat_at,attempt_epoch,terminal_at)
           VALUES($1,$2,$3,$4,$5,0,'vuln-triage-fixture','stage_unit',$6,
                  'main>vuln_triage','passed',$7,'candidate-e2e',NOW(),
                  NOW()+INTERVAL '5 minutes',NOW(),0,NOW())"#,
    )
    .bind(source_worker_run_id)
    .bind(operation_id)
    .bind(source_stage_execution_id)
    .bind(source_stage_run_unit_id)
    .bind(organization_id)
    .bind(format!("vuln_triage:{organization_id}"))
    .bind(source_lease_token)
    .execute(pool)
    .await
    .expect("insert final-passed vuln triage worker");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token)
           VALUES($1,$2,$3,$4,'primary','submit_stage_deliverable','{}','{}','finished',
                  $4,$5,$6,$7,$8,0,$9)"#,
    )
    .bind(source_tool_call_id)
    .bind(format!("candidate-e2e-{source_tool_call_id}"))
    .bind(source_session_id)
    .bind(operation_id)
    .bind(source_stage_execution_id)
    .bind(source_stage_run_unit_id)
    .bind(source_worker_run_id)
    .bind(organization_id)
    .bind(source_lease_token)
    .execute(pool)
    .await
    .expect("insert vuln triage submit call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,tool_call_record_id,tool_request_id,stage_kind,
               attempt_epoch,lease_token,payload,payload_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,$9,$10,$11)"#,
    )
    .bind(source_submission_id)
    .bind(operation_id)
    .bind(source_stage_execution_id)
    .bind(source_stage_run_unit_id)
    .bind(source_worker_run_id)
    .bind(organization_id)
    .bind(source_tool_call_id)
    .bind(format!("candidate-e2e-{source_submission_id}"))
    .bind(source_lease_token)
    .bind(json!({"schema_version":1,"candidates":[]}))
    .bind(digest('4'))
    .execute(pool)
    .await
    .expect("insert vuln triage deliverable");
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,from_stage_kind,
               stage_execution_id,source_stage_run_unit_id,deliverable_submission_id,
               scope_hash,payload,payload_sha256,evidence_ids,
               unit_gate_decision_hash,gate_passed_at)
           VALUES($1,$2,$3,$4,'vuln_triage',$5,$6,$7,$8,$9,$10,'{}',$11,NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(scope_snapshot_id)
    .bind(source_stage_execution_id)
    .bind(source_stage_run_unit_id)
    .bind(source_submission_id)
    .bind(digest('3'))
    .bind(json!({
        "canonical_fact_refs":[],"typed_claims":[],"coverage_watermark":{},"evidence_ids":[]
    }))
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(pool)
    .await
    .expect("insert vuln triage handoff");

    let wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{operation_id}:candidate-wave:0").as_bytes(),
    );
    let wave_unit_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{wave_run_id}:{organization_id}").as_bytes(),
    );
    let policy_snapshot = json!({
        "max_attempts_total":200,"max_candidates_total":100,
        "max_chain_depth":3,"max_waves":3
    });
    let policy_hash =
        "sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326".to_owned();
    let mut wave_tx = pool
        .begin()
        .await
        .expect("begin normal Candidate admission");
    attack_waves::open_from_vuln_triage_handoff(
        &mut wave_tx,
        &attack_waves::OpenAttackWaveUnit {
            wave_run_id,
            wave_unit_id,
            operation_id,
            scope_snapshot_id,
            organization_id,
            entry_stage_execution_id: source_stage_execution_id,
            entry_stage_run_unit_id: source_stage_run_unit_id,
            entry_deliverable_submission_id: source_submission_id,
            generation: 0,
            ordinal: 0,
            policy_snapshot,
            policy_hash,
            max_waves: 3,
            max_candidates_total: 100,
            max_chain_depth: 3,
            max_attempts_total: 200,
        },
    )
    .await
    .expect("admit operation through normal attack Wave API");
    wave_tx
        .commit()
        .await
        .expect("commit normal Candidate admission");

    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,organization_id,
               stage_kind,generation,specialist,status,started_at)
           VALUES($1,$2,$3,$4,$5,'attack_candidate',0,'candidate_controller','running',NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(candidate_stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert normally admitted Candidate stage unit");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source)
           VALUES($1,'candidate-e2e.example.test','domain','candidate-e2e.example.test',
                  'in',$2,$3,'candidate_e2e_fixture')"#,
    )
    .bind(Uuid::new_v4())
    .bind(&project_path)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert authority target");
    for family in ToolTruthRootFamilyV1::ALL {
        seed_fresh_bundle_root(pool, operation_id, organization_id, &project_path, family).await;
    }
    seed_managed_feed_authority(pool).await;

    FutureRuntimeFixture {
        operation_id,
        organization_id,
        scope_snapshot_id,
        stage_execution_id: candidate_stage_execution_id,
    }
}

async fn assert_future_authority_fixture(pool: &PgPool, fixture: FutureRuntimeFixture) {
    let receipt_window: (i64, i64, bool, i64, i64) = sqlx::query_as(
        r#"SELECT count(*)::BIGINT,
                  count(DISTINCT receipt.temporal_validity_policy_id)::BIGINT,
                  bool_and(receipt.valid_until > statement_timestamp()+INTERVAL '20 minutes'),
                  min(member.positive_ttl_ms),max(member.positive_ttl_ms)
             FROM capability_execution_receipts receipt
             JOIN coverage_denominators denominator ON denominator.id=receipt.denominator_id
             JOIN evidence_temporal_validity_policy_members member
               ON member.policy_id=receipt.temporal_validity_policy_id
            WHERE denominator.operation_id=$1
              AND receipt.reconciliation_state='consistent'"#,
    )
    .bind(fixture.operation_id)
    .fetch_one(pool)
    .await
    .expect("inspect extended root temporal authority");
    assert_eq!(receipt_window, (4, 4, true, 1_800_000, 1_800_000));
    let feed_cardinality: (i64, i64) = sqlx::query_as(
        r#"SELECT count(*)::BIGINT,count(DISTINCT member_hash)::BIGINT
             FROM candidate_managed_feed_store_members"#,
    )
    .fetch_one(pool)
    .await
    .expect("inspect managed feed exact denominator");
    assert_eq!(feed_cardinality, (5, 5));
    let guard_names = [
        "capability_execution_receipt_guard",
        "evidence_temporal_policy_header_guard",
        "evidence_temporal_policy_member_guard",
    ];
    let enabled: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT FROM pg_trigger WHERE tgname=ANY($1) AND tgenabled='O'",
    )
    .bind(guard_names)
    .fetch_one(pool)
    .await
    .expect("inspect temporal authority guards");
    assert_eq!(enabled, guard_names.len() as i64);
}

#[derive(Default)]
struct DeterministicCandidateRunner {
    miss_first_coverage: std::sync::atomic::AtomicBool,
    force_duplicate_conflict: std::sync::atomic::AtomicBool,
    provider_call_count: std::sync::atomic::AtomicUsize,
    retry_controller_signals:
        std::sync::Mutex<(Vec<CandidateMissedHypothesisSignal>, Option<String>)>,
    retry_analyst_signals: std::sync::Mutex<(Vec<CandidateMissedHypothesisSignal>, Option<String>)>,
}

impl DeterministicCandidateRunner {
    fn with_first_coverage_miss() -> Self {
        Self {
            miss_first_coverage: std::sync::atomic::AtomicBool::new(true),
            ..Self::default()
        }
    }

    fn with_duplicate_conflict() -> Self {
        Self {
            force_duplicate_conflict: std::sync::atomic::AtomicBool::new(true),
            ..Self::default()
        }
    }
}

fn deterministic_leaf_finding(
    snapshot_input_id: Uuid,
    checklist_member_id: Uuid,
    proposals: &[CandidateCoverageProposalSummary],
) -> CandidateLocalCoverageFinding {
    CandidateLocalCoverageFinding {
        outcome: CandidateCriticOutcome::NoMiss,
        missed_hypothesis_refs: Vec::new(),
        blocker_codes: Vec::new(),
        context_truncated: false,
        semantic_summary: CandidateCoverageSemanticSummary {
            covered_input_ids: vec![snapshot_input_id],
            covered_checklist_member_ids: vec![checklist_member_id],
            observed_proposal_ids: proposals.iter().map(|item| item.proposal_id).collect(),
            missed_checklist_member_ids: Vec::new(),
            blocker_codes: Vec::new(),
            semantic_observations: Vec::new(),
        },
    }
}

fn deterministic_synthesis_finding(
    node: &CandidateCoverageNodeInput,
) -> CandidateLocalCoverageFinding {
    let mut missed_checklist_member_ids = node
        .child_semantic_summaries
        .iter()
        .flat_map(|child| {
            child
                .semantic_summary
                .missed_checklist_member_ids
                .iter()
                .copied()
        })
        .collect::<Vec<_>>();
    missed_checklist_member_ids.sort_unstable();
    missed_checklist_member_ids.dedup();
    let mut blocker_codes = node
        .child_semantic_summaries
        .iter()
        .flat_map(|child| child.semantic_summary.blocker_codes.iter().cloned())
        .collect::<Vec<_>>();
    blocker_codes.sort_unstable();
    blocker_codes.dedup();
    let outcome = if !blocker_codes.is_empty() {
        CandidateCriticOutcome::Blocked
    } else if !missed_checklist_member_ids.is_empty() {
        CandidateCriticOutcome::MissedHypothesis
    } else {
        CandidateCriticOutcome::NoMiss
    };
    let mut semantic_observations = node
        .child_semantic_summaries
        .iter()
        .flat_map(|child| child.semantic_summary.semantic_observations.clone())
        .collect::<Vec<_>>();
    semantic_observations.sort_by_key(|observation| {
        serde_json::to_string(observation).expect("serialize deterministic semantic observation")
    });
    semantic_observations.dedup();
    CandidateLocalCoverageFinding {
        outcome,
        missed_hypothesis_refs: missed_checklist_member_ids.clone(),
        blocker_codes: blocker_codes.clone(),
        context_truncated: false,
        semantic_summary: CandidateCoverageSemanticSummary {
            covered_input_ids: node.covered_input_ids.clone(),
            covered_checklist_member_ids: node.covered_checklist_member_ids.clone(),
            observed_proposal_ids: node
                .h1_proposal_summaries
                .iter()
                .map(|item| item.proposal_id)
                .collect(),
            missed_checklist_member_ids,
            blocker_codes,
            semantic_observations,
        },
    }
}

#[async_trait]
impl HypothesisAnalysisAgentRunner for DeterministicCandidateRunner {
    async fn run_controller_dispatch(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerDispatchInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDispatchPlan>> {
        self.provider_call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        anyhow::ensure!(input
            .missed_hypothesis_signal_set_hash
            .starts_with("sha256:"));
        if binding.analysis_attempt_ordinal == 0 {
            anyhow::ensure!(input.missed_hypothesis_signals.is_empty());
        } else {
            anyhow::ensure!(!input.missed_hypothesis_signals.is_empty());
            *self
                .retry_controller_signals
                .lock()
                .expect("lock retry controller signals") = (
                input.missed_hypothesis_signals.clone(),
                Some(input.missed_hypothesis_signal_set_hash.clone()),
            );
        }
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: Uuid::new_v5(&binding.worker_run_id, b"e2e-dispatch"),
            output: CandidateControllerDispatchPlan {
                requested_live_lanes: 8,
                requested_inputs_per_microbatch: 24,
                objective_clusters: vec!["all".to_owned()],
            },
        })
    }

    async fn run_analyst(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateAnalystInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>> {
        self.provider_call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        anyhow::ensure!(input
            .missed_hypothesis_signal_set_hash
            .starts_with("sha256:"));
        if binding.analysis_attempt_ordinal == 0 {
            anyhow::ensure!(input.missed_hypothesis_signals.is_empty());
        } else if !input.missed_hypothesis_signals.is_empty() {
            *self
                .retry_analyst_signals
                .lock()
                .expect("lock retry analyst signals") = (
                input.missed_hypothesis_signals.clone(),
                Some(input.missed_hypothesis_signal_set_hash.clone()),
            );
        }
        let proof_chunk = input.chunks.iter().find(|chunk| {
            matches!(
                chunk.input_kind,
                CandidateInputKind::ToolTruthFact
                    | CandidateInputKind::ToolTruthObservation
                    | CandidateInputKind::ToolTruthEvidence
                    | CandidateInputKind::TechniqueOutcome
                    | CandidateInputKind::FactDelta
            )
        });
        let Some(proof_chunk) = proof_chunk else {
            return Ok(CandidateAnalysisAgentAttempt {
                provider_attempt_id: Uuid::new_v5(&binding.worker_run_id, b"e2e-analyst"),
                output: HypothesisProposalArtifact {
                    proposals: Vec::new(),
                },
            });
        };
        let proposal_id = Uuid::new_v5(&input.microbatch_id, b"candidate-e2e-proposal");
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: Uuid::new_v5(&binding.worker_run_id, b"e2e-analyst"),
            output: HypothesisProposalArtifact {
                proposals: vec![CandidateHypothesisProposal {
                    proposal_id,
                    subject_kind: "service".to_owned(),
                    subject_identity_hash: digest('a'),
                    predicate_schema: "network_service_exposure".to_owned(),
                    predicate_version: 1,
                    predicate_arguments: vec![
                        ("port".to_owned(), "443".to_owned()),
                        (
                            "source_input_id".to_owned(),
                            proof_chunk.input_id.to_string(),
                        ),
                    ],
                    trust_boundary: "internet".to_owned(),
                    polarity: "positive".to_owned(),
                    structured_claim: "The frozen service may expose a TLS endpoint.".to_owned(),
                    preconditions: vec!["The frozen service remains reachable.".to_owned()],
                    impact: "Requires Plan C verification before execution.".to_owned(),
                    proof_refs: vec![CandidateProofReference {
                        input_id: proof_chunk.input_id,
                        chunk_id: proof_chunk.chunk_id,
                        source_hash: proof_chunk.source_hash.clone(),
                        role: CandidateProofReferenceRole::Support,
                    }],
                    knowledge_signals: Vec::new(),
                    readiness: CandidateProposalReadiness::ReadyForStrategy,
                }],
            },
        })
    }

    async fn run_critic(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateCriticInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>> {
        self.provider_call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let critic_input_identity = match &input {
            CandidateCriticInput::ProposalConflict {
                conflict_component_id,
                ..
            } => format!("conflict:{conflict_component_id}"),
            CandidateCriticInput::CoverageSubreview {
                subreview_census_member_id,
                ..
            } => format!("subreview:{subreview_census_member_id}"),
            CandidateCriticInput::CoverageCrossChunkSynthesis { node }
            | CandidateCriticInput::CoverageCrossInputPartition { node }
            | CandidateCriticInput::CoverageCrossInputReduce { node }
            | CandidateCriticInput::CoverageCrossDimensionReduce { node }
            | CandidateCriticInput::CoverageGlobalSemanticRoot { node } => {
                format!("synthesis:{}", node.synthesis_node_id)
            }
        };
        let output = match input {
            CandidateCriticInput::ProposalConflict {
                conflict_component_id,
                proposals,
                ..
            } => HypothesisCriticArtifact::ProposalConflict {
                conflict_component_id,
                decision: if self
                    .force_duplicate_conflict
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    CandidateConflictDecisionKind::Duplicate
                } else {
                    CandidateConflictDecisionKind::NoConflict
                },
                related_proposal_ids: proposals.into_iter().map(|item| item.proposal_id).collect(),
            },
            CandidateCriticInput::CoverageSubreview {
                subreview_census_member_id,
                snapshot_input_id,
                checklist_member_id,
                checklist,
                h1_proposal_summaries,
                ..
            } => {
                let mut finding = deterministic_leaf_finding(
                    snapshot_input_id,
                    checklist_member_id,
                    &h1_proposal_summaries,
                );
                if binding.analysis_attempt_ordinal == 0
                    && !h1_proposal_summaries.is_empty()
                    && self
                        .miss_first_coverage
                        .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    let proposal = h1_proposal_summaries
                        .first()
                        .expect("production retry fixture emits an H1 proposal");
                    finding.outcome = CandidateCriticOutcome::MissedHypothesis;
                    finding.missed_hypothesis_refs = vec![checklist_member_id];
                    finding.semantic_summary.missed_checklist_member_ids =
                        vec![checklist_member_id];
                    finding.semantic_summary.semantic_observations =
                        vec![CandidateCoverageSemanticObservation {
                            kind: CandidateCoverageSemanticObservationKind::CoverageGap,
                            subject_kind: proposal.subject_kind.clone(),
                            subject_identity_hash: proposal.subject_identity_hash.clone(),
                            predicate_schema: proposal.predicate_schema.clone(),
                            predicate_version: proposal.predicate_version,
                            polarity: proposal.polarity.clone(),
                            trust_boundary: checklist.trust_boundary_identity.clone(),
                            input_ids: vec![snapshot_input_id],
                            checklist_member_ids: vec![checklist_member_id],
                            proposal_ids: vec![proposal.proposal_id],
                        }];
                }
                HypothesisCriticArtifact::CoverageSubreview {
                    subreview_census_member_id,
                    finding,
                }
            }
            CandidateCriticInput::CoverageCrossChunkSynthesis { node } => {
                HypothesisCriticArtifact::CoverageCrossChunkSynthesis {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: deterministic_synthesis_finding(&node),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                }
            }
            CandidateCriticInput::CoverageCrossInputPartition { node } => {
                HypothesisCriticArtifact::CoverageCrossInputPartition {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: deterministic_synthesis_finding(&node),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                }
            }
            CandidateCriticInput::CoverageCrossInputReduce { node } => {
                HypothesisCriticArtifact::CoverageCrossInputReduce {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: deterministic_synthesis_finding(&node),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                }
            }
            CandidateCriticInput::CoverageCrossDimensionReduce { node } => {
                HypothesisCriticArtifact::CoverageCrossDimensionReduce {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: deterministic_synthesis_finding(&node),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                }
            }
            CandidateCriticInput::CoverageGlobalSemanticRoot { node } => {
                HypothesisCriticArtifact::CoverageGlobalSemanticRoot {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: deterministic_synthesis_finding(&node),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                }
            }
        };
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: Uuid::new_v5(
                &binding.worker_run_id,
                critic_input_identity.as_bytes(),
            ),
            output,
        })
    }

    async fn run_controller_final(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerFinalInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>> {
        self.provider_call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        anyhow::ensure!(!input.proposal_pages.is_empty());
        anyhow::ensure!(input.proposal_page_set_hash.starts_with("sha256:"));
        let mut proposals = input
            .proposal_pages
            .into_iter()
            .flat_map(|page| {
                assert_eq!(page.proposal_count as usize, page.proposals.len());
                assert!(page.page_hash.starts_with("sha256:"));
                page.proposals
                    .into_iter()
                    .map(|proposal| (proposal.proposal_id, proposal.route_kind))
            })
            .collect::<Vec<_>>();
        proposals.sort_unstable();
        proposals.dedup();
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: Uuid::new_v5(&binding.worker_run_id, b"e2e-final"),
            output: CandidateControllerDecisionArtifact {
                decisions: proposals
                    .into_iter()
                    .map(|(proposal_id, route_kind)| CandidateControllerDecision {
                        proposal_id,
                        decision: match route_kind.as_str() {
                            "create_initial" => CandidateControllerDecisionKind::Accept,
                            "attach_current" => CandidateControllerDecisionKind::AttachExisting,
                            other => panic!("unsupported server route {other}"),
                        },
                        related_proposal_ids: Vec::new(),
                        rationale: "Deterministic fixture accepts the bounded proposal.".to_owned(),
                    })
                    .collect(),
            },
        })
    }
}

async fn runtime_replay_closure_hash(pool: &PgPool, snapshot_id: Uuid) -> String {
    sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
            'work',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'id',work.stage_work_item_id,'attempt',work.analysis_attempt_id,
                'capability',work.capability,'hash',work.work_item_hash,
                'status',stage.status) ORDER BY work.stage_work_item_id)
              FROM candidate_analysis_work_items work
              JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
              JOIN stage_work_items stage ON stage.id=work.stage_work_item_id
             WHERE attempt.snapshot_id=$1),'[]'::JSONB),
            'provider',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'id',provider.provider_attempt_id,'attempt',provider.analysis_attempt_id,
                'artifact_id',provider.artifact_id,'hash',provider.artifact_hash)
                ORDER BY provider.provider_attempt_id)
              FROM candidate_analysis_provider_attempts provider
              JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
             WHERE attempt.snapshot_id=$1),'[]'::JSONB),
            'artifact',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'id',artifact.artifact_id,'attempt',artifact.analysis_attempt_id,
                'kind',artifact.artifact_kind,'hash',artifact.artifact_hash)
                ORDER BY artifact.artifact_id)
              FROM candidate_analysis_artifacts artifact
              JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
             WHERE attempt.snapshot_id=$1),'[]'::JSONB),
            'conflict_decision',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'id',decision.merge_decision_id,'attempt',decision.analysis_attempt_id,
                'component',decision.conflict_component_id,'hash',decision.decision_hash)
                ORDER BY decision.merge_decision_id)
              FROM hypothesis_merge_decisions decision
              JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
             WHERE attempt.snapshot_id=$1),'[]'::JSONB),
            'h2',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'id',census.critic_census_id,'attempt',census.analysis_attempt_id,
                'count',census.member_count,'set',census.member_set_hash,
                'hash',census.census_hash) ORDER BY census.critic_census_id)
              FROM candidate_analysis_critic_censuses census
              JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
             WHERE attempt.snapshot_id=$1),'[]'::JSONB),
            'generation',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'id',generation.generation_id,'seal',seal.seal_id,
                'member_set',seal.member_set_hash,'hash',seal.generation_hash)
                ORDER BY generation.generation_id)
              FROM hypothesis_generations generation
              JOIN hypothesis_generation_seals seal USING(generation_id)
             WHERE generation.candidate_snapshot_id=$1),'[]'::JSONB),
            'apply',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'id',receipt.apply_receipt_id,'attempt',receipt.analysis_attempt_id,
                'generation',receipt.generation_id,'seal',receipt.generation_seal_id,
                'revision_set',receipt.revision_set_hash,'hash',receipt.receipt_hash)
                ORDER BY receipt.apply_receipt_id)
              FROM hypothesis_candidate_canonical_apply_receipts receipt
              JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
             WHERE attempt.snapshot_id=$1),'[]'::JSONB)
        )::TEXT)"#,
    )
    .bind(snapshot_id)
    .fetch_one(pool)
    .await
    .expect("hash exact durable Candidate replay closure")
}

#[tokio::test]
#[serial]
async fn future_authority_fixture_keeps_extended_window_and_all_guards_enabled() {
    let (db, _data_dir) = fixture("future-authority-guard").await;
    let seeded = seed_future_runtime_upstream_fixture(db.pool(), "future-authority-guard").await;
    assert_future_authority_fixture(db.pool(), seeded).await;
    let plan_b_rows: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM candidate_analysis_snapshots WHERE operation_id=$1),
             (SELECT count(*) FROM hypothesis_generations WHERE operation_id=$1),
             (SELECT count(*) FROM investigation_projection_outbox_batches WHERE operation_id=$1)"#,
    )
    .bind(seeded.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("prove future authority setup does not preseed Plan B rows");
    assert_eq!(plan_b_rows, (0, 0, 0));
}

#[tokio::test]
#[serial]
async fn candidate_stage_work_rejects_non_server_creator_before_provider_work() {
    let (mut db, _data_dir) = fixture("candidate-stage-creator-guard").await;
    let seeded =
        seed_future_runtime_upstream_fixture(db.pool(), "candidate-stage-creator-guard").await;
    let registry = hypothesis_registry_bridge::PgHypothesisRegistryRepository::new(Arc::new(
        db.pool().clone(),
    ));
    let stable_request_id = Uuid::new_v4();
    let snapshot = registry
        .freeze_candidate_snapshot(
            golish_agent_kit::db_traits::FreezeCandidateAnalysisSnapshot {
                stable_consumer_request_id: stable_request_id,
                operation_id: seeded.operation_id,
                scope_snapshot_id: seeded.scope_snapshot_id,
                organization_id: seeded.organization_id,
            },
        )
        .await
        .expect("freeze a real four-root Candidate snapshot");
    let opened = runtime_db::open_or_replay_attempt_runtime(
        db.pool(),
        snapshot.snapshot_id,
        seeded.stage_execution_id,
        0,
    )
    .await
    .expect("open the real Candidate attempt");
    let input_id: Uuid = sqlx::query_scalar(
        "SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key LIMIT 1",
    )
    .bind(snapshot.snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("load frozen Candidate input");
    let malicious_item_id = Uuid::new_v4();
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin malicious stage insertion");
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by)
           SELECT $1,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                  scope_snapshot_id,organization_id,dispatch_epoch,'hypothesis_proposal',$2,
                  'analyst',$3,input_refs,required_for_barrier,priority,'queued',attempt_policy,
                  budget,output_schema,'accepted_worker_request'
             FROM stage_work_items WHERE id=$4"#,
    )
    .bind(malicious_item_id)
    .bind(format!("malicious-proposal:{malicious_item_id}"))
    .bind(digest('b'))
    .bind(opened.controller_fence.work_item_id)
    .execute(&mut *tx)
    .await
    .expect("insert malicious Candidate stage row before deferred guard");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_work_items(
               candidate_work_item_id,stage_work_item_id,analysis_attempt_id,phase,
               capability,microbatch_key,work_item_hash)
           VALUES($1,$2,$3,'proposal','hypothesis_proposal',$4,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(malicious_item_id)
    .bind(opened.analysis_attempt_id)
    .bind(input_id.to_string())
    .bind(digest('b'))
    .execute(&mut *tx)
    .await
    .expect("bind malicious Candidate stage row");
    let rejected = tx
        .commit()
        .await
        .expect_err("non-server Candidate proposal creator must fail closed");
    assert!(rejected
        .to_string()
        .contains("CANDIDATE_WORK_CREATOR_INVALID"));

    let generic_conflict = golish_db::repo::candidate_analysis::record_analysis_artifact(
        db.pool(),
        golish_db::repo::candidate_analysis::RecordAnalysisArtifactInput {
            fence: opened.controller_fence.clone(),
            stable_artifact_request_id: Uuid::new_v4(),
            artifact:
                golish_db::repo::candidate_analysis::AnalysisArtifactBodyRow::ProposalConflictReview {
                    conflict_component_id: Uuid::new_v4(),
                    proposal_ids: Vec::new(),
                    outcome: "duplicate".to_owned(),
                    rationale: "must use the production conflict adapter".to_owned(),
                },
        },
    )
    .await
    .expect_err("generic artifact entry must reject conflict reviews");
    assert!(generic_conflict
        .to_string()
        .contains("HYPOTHESIS_REGISTRY_ARTIFACT_KIND_FORBIDDEN"));
    let generic_proposal = golish_db::repo::candidate_analysis::record_analysis_artifact(
        db.pool(),
        golish_db::repo::candidate_analysis::RecordAnalysisArtifactInput {
            fence: opened.controller_fence.clone(),
            stable_artifact_request_id: Uuid::new_v4(),
            artifact:
                golish_db::repo::candidate_analysis::AnalysisArtifactBodyRow::HypothesisProposal {
                    proposal_id: Uuid::new_v4(),
                    subject_kind: "organization".to_owned(),
                    subject_identity_hash: digest('7'),
                    predicate: golish_core::hypothesis_semantic_key::PredicateIdentity::new(
                        "network_service_exposure".to_owned(),
                        1,
                        json!({"port":443}),
                    )
                    .expect("construct generic proposal predicate"),
                    trust_boundary: "internet".to_owned(),
                    polarity: golish_core::hypothesis_semantic_key::ClaimPolarity::Positive,
                    prose: "must use the production proposal adapter".to_owned(),
                    confidence: 50,
                    priority: 50,
                    tags: Vec::new(),
                    evidence_refs: Vec::new(),
                },
        },
    )
    .await
    .expect_err("generic artifact entry must reject hypothesis proposals");
    assert!(generic_proposal
        .to_string()
        .contains("HYPOTHESIS_REGISTRY_ARTIFACT_KIND_FORBIDDEN"));
    let controller_candidate_item_id: Uuid = sqlx::query_scalar(
        "SELECT candidate_work_item_id FROM candidate_analysis_work_items WHERE stage_work_item_id=$1",
    )
    .bind(opened.controller_fence.work_item_id)
    .fetch_one(db.pool())
    .await
    .expect("load controller Candidate work identity");
    let wrong_capability_artifact = sqlx::query(
        r#"INSERT INTO candidate_analysis_artifacts(
               artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,
               artifact_kind,artifact_body,artifact_hash)
           VALUES($1,$2,$3,$4,'proposal_conflict_review.v1',$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(opened.analysis_attempt_id)
    .bind(controller_candidate_item_id)
    .bind(opened.controller_fence.worker_run_id)
    .bind(json!({
        "mode":"proposal_conflict",
        "conflict_component_id":Uuid::new_v4(),
        "decision":"no_conflict",
        "related_proposal_ids":[],
    }))
    .bind(digest('9'))
    .execute(db.pool())
    .await
    .expect_err("artifact kind cannot borrow an unrelated Candidate capability");
    assert!(wrong_capability_artifact
        .to_string()
        .contains("CANDIDATE_ARTIFACT_CAPABILITY_MISMATCH"));

    let orphan_coverage_body = json!({
        "kind":"hypothesis_coverage_review.v1",
        "analysis_attempt_id":opened.analysis_attempt_id,
        "snapshot_input_id":input_id,
        "outcome":"adequate",
        "review_mode":"full",
        "checklist_dispositions":[],
        "typed_missed_refs":[],
    });
    let orphan_coverage_hash = db_hash(db.pool(), &orphan_coverage_body).await;
    let orphan_coverage_artifact = sqlx::query(
        r#"INSERT INTO candidate_analysis_artifacts(
               artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,
               artifact_kind,artifact_body,artifact_hash)
           VALUES($1,$2,$3,$4,'hypothesis_coverage_review.v1',$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(opened.analysis_attempt_id)
    .bind(controller_candidate_item_id)
    .bind(opened.controller_fence.worker_run_id)
    .bind(orphan_coverage_body)
    .bind(orphan_coverage_hash)
    .execute(db.pool())
    .await
    .expect_err("raw coverage artifact without a reduced review must roll back");
    assert!(orphan_coverage_artifact
        .to_string()
        .contains("CANDIDATE_COVERAGE_REVIEW_ARTIFACT_EXACT_SET_REQUIRED"));

    let forged_census_id = Uuid::new_v4();
    let forged_global_node_id = Uuid::new_v4();
    let forged_dimension_node_id = Uuid::new_v4();
    let mut forged_census_tx = db.pool().begin().await.expect("begin forged census attack");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_censuses(
               synthesis_census_id,analysis_attempt_id,relationship_cross_index_hash,
               fan_in_limit,node_count,node_set_hash,dimension_root_count,
               dimension_root_set_hash,global_root_node_id,census_hash)
           VALUES($1,$2,$3,32,1,$4,1,$5,$6,$7)"#,
    )
    .bind(forged_census_id)
    .bind(opened.analysis_attempt_id)
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(forged_global_node_id)
    .bind(digest('4'))
    .execute(&mut *forged_census_tx)
    .await
    .expect("stage an understated synthesis census header");
    for (node_id, node_kind, attack_class, boundary, node_hash) in [
        (
            forged_global_node_id,
            "global_semantic_root",
            None,
            None,
            digest('5'),
        ),
        (
            forged_dimension_node_id,
            "cross_chunk",
            Some("network_exposure"),
            Some(digest('6')),
            digest('7'),
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_census_members(
                   synthesis_node_id,synthesis_census_id,analysis_attempt_id,node_kind,
                   level,partition_ordinal,attack_class_id,trust_boundary_hash,
                   covered_input_count,covered_input_set_hash,covered_checklist_count,
                   covered_checklist_set_hash,child_receipt_count,child_receipt_set_hash,
                   relationship_cross_index_hash,descendant_worker_count,
                   descendant_worker_set_hash,node_hash)
               VALUES($1,$2,$3,$4,0,0,$5,$6,1,$7,1,$8,1,$9,$10,1,$11,$12)"#,
        )
        .bind(node_id)
        .bind(forged_census_id)
        .bind(opened.analysis_attempt_id)
        .bind(node_kind)
        .bind(attack_class)
        .bind(boundary)
        .bind(digest('8'))
        .bind(digest('9'))
        .bind(digest('a'))
        .bind(digest('1'))
        .bind(digest('b'))
        .bind(node_hash)
        .execute(&mut *forged_census_tx)
        .await
        .expect("stage forged synthesis census member");
    }
    let forged_census = sqlx::query(
        r#"SET CONSTRAINTS
             candidate_synthesis_census_header_exact_set_guard,
             candidate_synthesis_census_member_exact_set_guard IMMEDIATE"#,
    )
    .execute(&mut *forged_census_tx)
    .await
    .expect_err("understated synthesis header must fail its direct exact-set guard");
    assert!(forged_census
        .to_string()
        .contains("CANDIDATE_SYNTHESIS_CENSUS_RELATIONAL_EXACT_SET_REQUIRED"));
    forged_census_tx
        .rollback()
        .await
        .expect("roll back forged census attack");

    let late_stage_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by)
           SELECT $1,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                  scope_snapshot_id,organization_id,dispatch_epoch,'candidate_unknown_late_binding',$2,
                  'analyst',$3,input_refs,required_for_barrier,priority,'queued',attempt_policy,
                  budget,output_schema,'server_seed'
             FROM stage_work_items WHERE id=$4"#,
    )
    .bind(late_stage_id)
    .bind(format!("late-candidate:{late_stage_id}"))
    .bind(digest('c'))
    .bind(opened.controller_fence.work_item_id)
    .execute(db.pool())
    .await
    .expect("commit a non-Candidate parent before the late binding attack");
    let late_binding = sqlx::query(
        r#"INSERT INTO candidate_analysis_work_items(
               candidate_work_item_id,stage_work_item_id,analysis_attempt_id,phase,
               capability,microbatch_key,work_item_hash)
           VALUES($1,$2,$3,'proposal','hypothesis_proposal',$4,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(late_stage_id)
    .bind(opened.analysis_attempt_id)
    .bind(format!("late:{input_id}"))
    .bind(digest('c'))
    .execute(db.pool())
    .await
    .expect_err("a committed unrelated stage row cannot acquire Candidate authority later");
    assert!(late_binding
        .to_string()
        .contains("CANDIDATE_WORK_PARENT_AUTHORITY_INVALID"));

    let wrong_role_item_id = Uuid::new_v4();
    let mut wrong_role_tx = db.pool().begin().await.expect("begin wrong-role attack");
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by)
           SELECT $1,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                  scope_snapshot_id,organization_id,dispatch_epoch,'hypothesis_proposal',$2,
                  'critic',$3,input_refs,required_for_barrier,priority,'queued',attempt_policy,
                  budget,output_schema,'server_seed'
             FROM stage_work_items WHERE id=$4"#,
    )
    .bind(wrong_role_item_id)
    .bind(format!("wrong-role:{wrong_role_item_id}"))
    .bind(digest('d'))
    .bind(opened.controller_fence.work_item_id)
    .execute(&mut *wrong_role_tx)
    .await
    .expect("stage wrong-role Candidate parent before deferred guard");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_work_items(
               candidate_work_item_id,stage_work_item_id,analysis_attempt_id,phase,
               capability,microbatch_key,work_item_hash)
           VALUES($1,$2,$3,'proposal','hypothesis_proposal',$4,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(wrong_role_item_id)
    .bind(opened.analysis_attempt_id)
    .bind(format!("wrong-role:{input_id}"))
    .bind(digest('d'))
    .execute(&mut *wrong_role_tx)
    .await
    .expect("bind wrong-role Candidate parent");
    let wrong_role = wrong_role_tx
        .commit()
        .await
        .expect_err("wrong Candidate worker role must fail closed");
    assert!(wrong_role
        .to_string()
        .contains("CANDIDATE_WORK_PARENT_AUTHORITY_INVALID"));

    let late_stale = sqlx::query(
        r#"INSERT INTO candidate_analysis_stale_evidence_residuals(
               residual_id,snapshot_id,temporal_census_member_id,bundle_member_id,
               reason_code,target_state_epoch_identity_hash,required_capability,residual_hash)
           VALUES($1,$2,$3,$4,'authority_expired',$5,'late_test',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(snapshot.snapshot_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest('e'))
    .bind(digest('f'))
    .execute(db.pool())
    .await
    .expect_err("late stale residual must not extend the frozen snapshot");
    assert!(late_stale
        .to_string()
        .contains("CANDIDATE_SNAPSHOT_CHILDREN_FROZEN"));
    let late_revalidation = sqlx::query(
        r#"INSERT INTO candidate_analysis_revalidation_obligations(
               obligation_id,snapshot_id,stale_residual_id,root_family,
               evidence_identity_hash,target_state_epoch_identity_hash,
               required_capability,reason_code,obligation_hash)
           VALUES($1,$2,$3,'ti',$4,$5,'late_test','late_test',$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(snapshot.snapshot_id)
    .bind(Uuid::new_v4())
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .execute(db.pool())
    .await
    .expect_err("late revalidation obligation must not extend the frozen snapshot");
    assert!(late_revalidation
        .to_string()
        .contains("CANDIDATE_SNAPSHOT_CHILDREN_FROZEN"));
    let late_enrichment = sqlx::query(
        r#"INSERT INTO candidate_analysis_enrichment_obligations(
               obligation_id,snapshot_id,obligation_kind,product_member_id,
               reason_code,affected_checklist_member_key,obligation_hash)
           VALUES($1,$2,'product_version_enrichment',$3,'late_test','late_test',$4)"#,
    )
    .bind(Uuid::new_v4())
    .bind(snapshot.snapshot_id)
    .bind(Uuid::new_v4())
    .bind(digest('4'))
    .execute(db.pool())
    .await
    .expect_err("late enrichment obligation must not extend the frozen snapshot");
    assert!(late_enrichment
        .to_string()
        .contains("CANDIDATE_SNAPSHOT_CHILDREN_FROZEN"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn non_no_conflict_decision_durably_blocks_before_h2_or_compiler() {
    let (mut db, _data_dir) = fixture("candidate-conflict-block").await;
    let seeded = seed_future_runtime_upstream_fixture(db.pool(), "candidate-conflict-block").await;
    let pool = Arc::new(db.pool().clone());
    let registry: Arc<dyn HypothesisRegistryRepository> =
        Arc::new(hypothesis_registry_bridge::PgHypothesisRegistryRepository::new(pool.clone()));
    let gate_source = Arc::new(PgCandidateGateSnapshotSource::new(
        registry.clone(),
        pool.clone(),
    ));
    let finalizer = AtomicCandidateFinalizer::new(registry.clone(), gate_source);
    let repository = Arc::new(PgHypothesisAnalysisRuntimeRepository::new(
        pool, registry, finalizer,
    ));
    let policy = load_embedded_stage_spec(StageKind::AttackCandidate)
        .expect("load Candidate StageSpec")
        .candidate_analysis_team
        .expect("Candidate analysis policy");
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository, policy)
        .expect("construct conflict-block Candidate runtime");
    let outcome = runtime
        .run(
            HypothesisAnalysisStageRequest {
                stable_request_id: Uuid::new_v4(),
                operation_id: seeded.operation_id,
                scope_snapshot_id: seeded.scope_snapshot_id,
                organization_id: seeded.organization_id,
                stage_execution_id: seeded.stage_execution_id,
            },
            &DeterministicCandidateRunner::with_duplicate_conflict(),
        )
        .await
        .expect("non-no-conflict decision closes as a durable block");
    let HypothesisAnalysisStageOutcome::BlockedAnalysis { snapshot_id, .. } = outcome else {
        panic!("expected conflict block, got {outcome:?}");
    };
    let closure: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM hypothesis_merge_decisions decision
               JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
              WHERE attempt.snapshot_id=$1 AND decision.decision_kind='duplicate'),
             (SELECT count(*) FROM candidate_analysis_attempt_state_events event
               JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
              WHERE attempt.snapshot_id=$1 AND event.event_kind='blocked'),
             (SELECT count(*) FROM candidate_analysis_critic_censuses census
               JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
              WHERE attempt.snapshot_id=$1),
             (SELECT count(*) FROM candidate_analysis_host_compilation_materials material
              WHERE material.snapshot_id=$1),
             (SELECT count(*) FROM hypothesis_generations generation
              WHERE generation.candidate_snapshot_id=$1),
             (SELECT count(*) FROM hypothesis_candidate_canonical_apply_receipts receipt
               JOIN candidate_analysis_attempts attempt USING(analysis_attempt_id)
              WHERE attempt.snapshot_id=$1)"#,
    )
    .bind(snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect durable conflict block closure");
    assert!(closure.0 > 0);
    assert_eq!(
        (closure.1, closure.2, closure.3, closure.4, closure.5),
        (1, 0, 0, 0, 0)
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn production_runtime_reaches_generation_seal_through_recursive_gate() {
    let (mut db, _data_dir) = fixture("generation-seal-e2e").await;
    let seeded = seed_future_runtime_upstream_fixture(db.pool(), "generation-seal-e2e").await;
    assert_future_authority_fixture(db.pool(), seeded).await;
    let before: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM candidate_analysis_snapshots WHERE operation_id=$1),
             (SELECT count(*) FROM candidate_analysis_attempts WHERE operation_id=$1),
             (SELECT count(*) FROM candidate_analysis_snapshot_inputs input
               JOIN candidate_analysis_snapshots snapshot USING(snapshot_id)
              WHERE snapshot.operation_id=$1)"#,
    )
    .bind(seeded.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("prove Candidate runtime authority is not preseeded");
    assert_eq!(before, (0, 0, 0));
    let stable_request_id = Uuid::new_v4();
    let authority_request = capability_execution_receipts::CheckToolTruthAuthorityBundle {
        stable_consumer_request_id: stable_request_id,
        operation_id: seeded.operation_id,
        organization_id: seeded.organization_id,
        consumer_kind:
            capability_execution_receipts::ToolTruthAuthorityBundleConsumerV1::CandidateAnalysis,
    };
    let checked_bundle_id =
        capability_execution_receipts::with_all_fresh_tool_truth_authority_bundle(
            db.pool(),
            &authority_request,
            |_tx, all_fresh| {
                Box::pin(async move {
                    assert_eq!(all_fresh.checked().roots().len(), 4);
                    Ok(all_fresh.bundle_seal_id())
                })
            },
        )
        .await
        .expect("prove four-root authority through the all-fresh opaque guard");

    let pool = Arc::new(db.pool().clone());
    let registry: Arc<dyn HypothesisRegistryRepository> =
        Arc::new(hypothesis_registry_bridge::PgHypothesisRegistryRepository::new(pool.clone()));
    let gate_source = Arc::new(PgCandidateGateSnapshotSource::new(
        registry.clone(),
        pool.clone(),
    ));
    let finalizer = AtomicCandidateFinalizer::new(registry.clone(), gate_source);
    let repository = Arc::new(PgHypothesisAnalysisRuntimeRepository::new(
        pool, registry, finalizer,
    ));
    let policy = load_embedded_stage_spec(StageKind::AttackCandidate)
        .expect("load Candidate StageSpec")
        .candidate_analysis_team
        .expect("Candidate analysis policy");
    let request = HypothesisAnalysisStageRequest {
        stable_request_id,
        operation_id: seeded.operation_id,
        scope_snapshot_id: seeded.scope_snapshot_id,
        organization_id: seeded.organization_id,
        stage_execution_id: seeded.stage_execution_id,
    };
    let runner = DeterministicCandidateRunner::with_first_coverage_miss();

    // Exercise the production repository boundary before the coordinator is
    // allowed to proceed. Every forged proof must roll back the artifact,
    // provider receipt, proposal, and stage output as one transaction.
    let preflight_snapshot = repository
        .freeze_snapshot(request)
        .await
        .expect("freeze production Candidate snapshot");
    let preflight_attempt = repository
        .open_attempt(preflight_snapshot.snapshot_id, 0, seeded.stage_execution_id)
        .await
        .expect("open production Candidate attempt zero");
    let premature_dispatch_h1 = golish_db::repo::candidate_analysis::seal_analysis_census(
        db.pool(),
        golish_db::repo::candidate_analysis::SealAnalysisCensusInput {
            fence: db_fence(&preflight_attempt.controller_authority),
            stable_census_request_id: Uuid::new_v4(),
            census_kind: golish_db::repo::candidate_analysis::AnalysisCensusKindRow::Proposal,
        },
    )
    .await
    .expect_err("a running dispatch cannot seal H1 before its receipt is durable");
    assert!(premature_dispatch_h1
        .to_string()
        .contains("CANDIDATE_H1_CONTROLLER_DISPATCH_FENCE_REQUIRED"));
    let dispatch = runner
        .run_controller_dispatch(
            preflight_attempt.controller_binding.clone(),
            preflight_attempt.controller_dispatch_input.clone(),
        )
        .await
        .expect("run deterministic controller preflight");
    repository
        .persist_controller_dispatch(
            &preflight_attempt.controller_binding,
            &preflight_attempt.controller_authority,
            &dispatch,
        )
        .await
        .expect("persist deterministic controller preflight");
    let mut proof_work = None;
    for _ in 0..64 {
        let analyst_work = repository
            .prepare_analyst_wave(&preflight_attempt, &dispatch.output, 8)
            .await
            .expect("prepare rolling production analyst work");
        assert!(!analyst_work.is_empty(), "proof work must remain reachable");
        for work in analyst_work {
            let is_proof_work = work.input.chunks.iter().any(|chunk| {
                matches!(
                    &chunk.input_kind,
                    CandidateInputKind::ToolTruthFact
                        | CandidateInputKind::ToolTruthObservation
                        | CandidateInputKind::ToolTruthEvidence
                        | CandidateInputKind::TechniqueOutcome
                        | CandidateInputKind::FactDelta
                )
            });
            if is_proof_work {
                proof_work = Some(work);
                break;
            }
            let zero_proposal = runner
                .run_analyst(work.binding.clone(), work.input.clone())
                .await
                .expect("build deterministic non-proof analyst closure");
            assert!(zero_proposal.output.proposals.is_empty());
            repository
                .persist_analyst_artifact(&work.binding, &work.authority, &zero_proposal)
                .await
                .expect("persist deterministic non-proof analyst closure");
        }
        if proof_work.is_some() {
            break;
        }
    }
    let proof_work = proof_work.expect("production fixture has one delivered proof chunk");
    let post_dispatch_opened = runtime_db::open_or_replay_attempt_runtime(
        db.pool(),
        preflight_snapshot.snapshot_id,
        seeded.stage_execution_id,
        0,
    )
    .await
    .expect("reload exact post-dispatch Controller fence");
    let wrong_fence_h1 = golish_db::repo::candidate_analysis::seal_analysis_census(
        db.pool(),
        golish_db::repo::candidate_analysis::SealAnalysisCensusInput {
            fence: db_fence(&proof_work.authority),
            stable_census_request_id: Uuid::new_v4(),
            census_kind: golish_db::repo::candidate_analysis::AnalysisCensusKindRow::Proposal,
        },
    )
    .await
    .expect_err("an analyst fence cannot seal the Controller-owned H1 denominator");
    assert!(wrong_fence_h1
        .to_string()
        .contains("CANDIDATE_H1_CONTROLLER_DISPATCH_FENCE_REQUIRED"));
    let pending_analyst_h1 = golish_db::repo::candidate_analysis::seal_analysis_census(
        db.pool(),
        golish_db::repo::candidate_analysis::SealAnalysisCensusInput {
            fence: post_dispatch_opened.controller_fence,
            stable_census_request_id: Uuid::new_v4(),
            census_kind: golish_db::repo::candidate_analysis::AnalysisCensusKindRow::Proposal,
        },
    )
    .await
    .expect_err("H1 cannot freeze while any designated analyst work is pending");
    assert!(pending_analyst_h1
        .to_string()
        .contains("CANDIDATE_H1_PROPOSAL_WAVE_NOT_CLOSED"));
    let valid_attempt = runner
        .run_analyst(proof_work.binding.clone(), proof_work.input.clone())
        .await
        .expect("build deterministic analyst artifact");
    let proof = valid_attempt
        .output
        .proposals
        .first()
        .and_then(|proposal| proposal.proof_refs.first())
        .expect("deterministic analyst proposal has a proof reference");
    let artifact_count_before_forged_writes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_analysis_artifacts WHERE analysis_attempt_id=$1",
    )
    .bind(preflight_attempt.analysis_attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("capture durable non-proof analyst baseline");

    let mut foreign_input = valid_attempt.clone();
    foreign_input.provider_attempt_id = Uuid::new_v4();
    foreign_input.output.proposals[0].proof_refs[0].input_id = Uuid::new_v4();
    let rejected = repository
        .persist_analyst_artifact(&proof_work.binding, &proof_work.authority, &foreign_input)
        .await
        .expect_err("foreign proof input must fail closed");
    assert!(rejected
        .to_string()
        .contains("CANDIDATE_PROPOSAL_REF_FOREIGN_INPUT"));

    let mut undelivered_chunk = valid_attempt.clone();
    undelivered_chunk.provider_attempt_id = Uuid::new_v4();
    undelivered_chunk.output.proposals[0].proof_refs[0].chunk_id = Uuid::new_v4();
    let rejected = repository
        .persist_analyst_artifact(
            &proof_work.binding,
            &proof_work.authority,
            &undelivered_chunk,
        )
        .await
        .expect_err("undelivered proof chunk must fail closed");
    assert!(rejected
        .to_string()
        .contains("CANDIDATE_PROPOSAL_REF_UNDELIVERED_CHUNK"));

    let mut forged_source = valid_attempt.clone();
    forged_source.provider_attempt_id = Uuid::new_v4();
    forged_source.output.proposals[0].proof_refs[0].source_hash = digest('f');
    let rejected = repository
        .persist_analyst_artifact(&proof_work.binding, &proof_work.authority, &forged_source)
        .await
        .expect_err("forged proof source hash must fail closed");
    assert!(rejected
        .to_string()
        .contains("CANDIDATE_PROPOSAL_REF_SOURCE_HASH_MISMATCH"));
    let mut forged_knowledge = valid_attempt.clone();
    forged_knowledge.provider_attempt_id = Uuid::new_v4();
    forged_knowledge.output.proposals[0].knowledge_signals =
        vec![CandidateKnowledgeSignalReference {
            feed_snapshot_id: Uuid::new_v4(),
            feed_match_member_id: Uuid::new_v4(),
            feed_match_member_hash: digest('a'),
            product_version_match_hash: digest('b'),
            source_authority: CandidateKnowledgeSignalAuthority::KnowledgeSignalOnly,
        }];
    let rejected = repository
        .persist_analyst_artifact(
            &proof_work.binding,
            &proof_work.authority,
            &forged_knowledge,
        )
        .await
        .expect_err("an analyst cannot borrow an undelivered knowledge signal");
    assert!(rejected
        .to_string()
        .contains("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"));
    assert_eq!(proof.input_id, proof_work.input.chunks[0].input_id);
    let rollback_counts: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM candidate_analysis_artifacts WHERE analysis_attempt_id=$1),
             (SELECT count(*) FROM candidate_analysis_provider_attempts WHERE analysis_attempt_id=$1
               AND stage_work_item_id=$2),
             (SELECT count(*) FROM hypothesis_proposals WHERE analysis_attempt_id=$1),
             (SELECT count(*) FROM stage_worker_outputs WHERE work_item_id=$2)"#,
    )
    .bind(preflight_attempt.analysis_attempt_id)
    .bind(proof_work.binding.work_item_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect forged proof atomic rollback");
    assert_eq!(
        rollback_counts,
        (artifact_count_before_forged_writes, 0, 0, 0)
    );

    let runtime = PgHypothesisAnalysisStageRuntime::new(repository.clone(), policy)
        .expect("construct production Candidate runtime");
    let outcome = runtime
        .run(request, &runner)
        .await
        .expect("production Candidate runtime reaches terminal outcome");
    let artifact_cardinality: (i64, i64, i64, bool, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_reviews),
             (SELECT count(*) FROM candidate_analysis_artifacts
               WHERE artifact_kind='hypothesis_coverage_review.v1'),
             (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_reviews review
               WHERE (SELECT count(*) FROM candidate_analysis_artifacts artifact
                       WHERE artifact.analysis_attempt_id=review.analysis_attempt_id
                         AND artifact.artifact_kind='hypothesis_coverage_review.v1'
                         AND artifact.artifact_body->>'snapshot_input_id'=
                             review.snapshot_input_id::TEXT)<>1),
             EXISTS(SELECT 1 FROM candidate_analysis_artifacts
                     WHERE artifact_kind='hypothesis_coverage_review.v1'
                     GROUP BY candidate_work_item_id HAVING count(*)>1),
             (SELECT count(*) FROM candidate_analysis_work_items candidate
               WHERE candidate.phase='proposal'
                 AND (SELECT count(*) FROM candidate_analysis_artifacts artifact
                       WHERE artifact.candidate_work_item_id=candidate.candidate_work_item_id
                         AND artifact.artifact_kind='hypothesis_proposal.v1')<>1)"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect host review and analyst artifact cardinality");
    assert!(artifact_cardinality.0 > 1);
    assert_eq!(artifact_cardinality.0, artifact_cardinality.1);
    assert_eq!(artifact_cardinality.2, 0);
    assert!(artifact_cardinality.3);
    assert_eq!(artifact_cardinality.4, 0);
    let duplicate_host_artifact_groups: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM (
             SELECT candidate_work_item_id,artifact_kind,artifact_hash
               FROM candidate_analysis_artifacts
              GROUP BY candidate_work_item_id,artifact_kind,artifact_hash
             HAVING count(*)>1
           ) duplicate"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("prove host review replay keys remain unique");
    assert_eq!(duplicate_host_artifact_groups, 0);
    let predecessor_terminal_event_id: Uuid = sqlx::query_scalar(
        r#"SELECT attempt_event_id FROM candidate_analysis_attempt_state_events
            WHERE analysis_attempt_id=$1 AND event_kind='superseded_missed_hypothesis'"#,
    )
    .bind(preflight_attempt.analysis_attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("load predecessor missed terminal event");
    let forged_attempt_id = Uuid::new_v4();
    let forged_opened_event_id = Uuid::new_v4();
    let mut forged_retry_tx = db.pool().begin().await.expect("begin forged retry insert");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
               predecessor_attempt_id,attempt_input_hash,attack_class_checklist_version,
               attack_class_checklist_digest,trust_boundary_checklist_version,
               trust_boundary_checklist_digest,coverage_sampling_contract_version,
               coverage_sampling_contract_digest,retry_limit)
           SELECT $1,snapshot_id,operation_id,organization_id,2,$2,$3,
                  attack_class_checklist_version,attack_class_checklist_digest,
                  trust_boundary_checklist_version,trust_boundary_checklist_digest,
                  coverage_sampling_contract_version,coverage_sampling_contract_digest,retry_limit
             FROM candidate_analysis_attempts WHERE analysis_attempt_id=$2"#,
    )
    .bind(forged_attempt_id)
    .bind(preflight_attempt.analysis_attempt_id)
    .bind(digest('f'))
    .execute(&mut *forged_retry_tx)
    .await
    .expect("stage forged retry attempt before deferred exact-hash guard");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempt_state_events(
               attempt_event_id,analysis_attempt_id,event_ordinal,event_kind,event_hash)
           VALUES($1,$2,0,'opened',tool_truth_sha256(jsonb_build_object(
               'attempt',$2::UUID,'ordinal',0,'event','opened',
               'predecessor_attempt_id',$3::UUID,
               'predecessor_terminal_event_id',$4::UUID
           )::TEXT))"#,
    )
    .bind(forged_opened_event_id)
    .bind(forged_attempt_id)
    .bind(preflight_attempt.analysis_attempt_id)
    .bind(predecessor_terminal_event_id)
    .execute(&mut *forged_retry_tx)
    .await
    .expect("stage forged retry opened event");
    let forged_retry = forged_retry_tx
        .commit()
        .await
        .expect_err("successor attempt input hash must be recomputed from predecessor misses");
    assert!(forged_retry
        .to_string()
        .contains("CANDIDATE_RETRY_ATTEMPT_INPUT_HASH_INVALID"));
    let provider_calls_after_first_run = runner
        .provider_call_count
        .load(std::sync::atomic::Ordering::SeqCst);
    let closure_hash_before_restart =
        runtime_replay_closure_hash(db.pool(), preflight_snapshot.snapshot_id).await;
    let replay_pool = Arc::new(db.pool().clone());
    let replay_registry: Arc<dyn HypothesisRegistryRepository> = Arc::new(
        hypothesis_registry_bridge::PgHypothesisRegistryRepository::new(replay_pool.clone()),
    );
    let replay_gate_source = Arc::new(PgCandidateGateSnapshotSource::new(
        replay_registry.clone(),
        replay_pool.clone(),
    ));
    let replay_finalizer =
        AtomicCandidateFinalizer::new(replay_registry.clone(), replay_gate_source);
    let replay_repository = Arc::new(PgHypothesisAnalysisRuntimeRepository::new(
        replay_pool,
        replay_registry,
        replay_finalizer,
    ));
    let replay_policy = load_embedded_stage_spec(StageKind::AttackCandidate)
        .expect("reload Candidate StageSpec in a fresh runtime composition")
        .candidate_analysis_team
        .expect("fresh Candidate analysis policy");
    let replay_runtime = PgHypothesisAnalysisStageRuntime::new(replay_repository, replay_policy)
        .expect("construct fresh production Candidate runtime composition");
    let replayed_outcome = replay_runtime
        .run(request, &runner)
        .await
        .expect("production Candidate runtime replays the sealed result after restart");
    match (&outcome, &replayed_outcome) {
        (
            HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
                snapshot_id,
                analysis_attempt_id,
                analysis_attempt_ordinal,
                analyst_work_item_count,
                critic_work_item_count,
                peak_live_lanes,
                final_receipt,
                generation,
            },
            HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
                snapshot_id: replay_snapshot_id,
                analysis_attempt_id: replay_attempt_id,
                analysis_attempt_ordinal: replay_attempt_ordinal,
                analyst_work_item_count: replay_analyst_count,
                critic_work_item_count: replay_critic_count,
                peak_live_lanes: replay_peak_live_lanes,
                final_receipt: replay_final_receipt,
                generation: replay_generation,
            },
        ) => {
            assert_eq!(replay_snapshot_id, snapshot_id);
            assert_eq!(replay_attempt_id, analysis_attempt_id);
            assert_eq!(replay_attempt_ordinal, analysis_attempt_ordinal);
            assert_eq!(replay_final_receipt, final_receipt);
            assert!(*analyst_work_item_count > 0);
            assert!(*critic_work_item_count > 0);
            assert!(*peak_live_lanes > 0);
            assert_eq!(*replay_analyst_count, 0);
            assert_eq!(*replay_critic_count, 0);
            assert_eq!(*replay_peak_live_lanes, 0);
            assert!(!generation.replayed);
            assert!(replay_generation.replayed);
            let mut initial_generation = generation.clone();
            initial_generation.replayed = false;
            let mut durable_replay_generation = replay_generation.clone();
            durable_replay_generation.replayed = false;
            assert_eq!(durable_replay_generation, initial_generation);
        }
        _ => panic!("initial and replay outcomes must both be analysis artifacts ready"),
    }
    assert_eq!(
        runner
            .provider_call_count
            .load(std::sync::atomic::Ordering::SeqCst),
        provider_calls_after_first_run,
        "restart must not call a provider after exact durable receipts exist"
    );
    let closure_hash_after_restart =
        runtime_replay_closure_hash(db.pool(), preflight_snapshot.snapshot_id).await;
    assert_eq!(closure_hash_after_restart, closure_hash_before_restart);
    let HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
        snapshot_id,
        analysis_attempt_id,
        analysis_attempt_ordinal,
        generation,
        ..
    } = outcome
    else {
        panic!("expected generation seal, got {outcome:?}");
    };
    assert_eq!(analysis_attempt_ordinal, 1);
    assert_ne!(analysis_attempt_id, preflight_attempt.analysis_attempt_id);
    assert!(generation.generation_member_count > 0);

    let controller_retry = runner
        .retry_controller_signals
        .lock()
        .expect("lock retry controller result")
        .clone();
    let analyst_retry = runner
        .retry_analyst_signals
        .lock()
        .expect("lock retry analyst result")
        .clone();
    assert!(!controller_retry.0.is_empty());
    assert_eq!(controller_retry, analyst_retry);
    let retry_signal_set_hash = controller_retry
        .1
        .as_deref()
        .expect("retry controller receives a server-owned set hash");
    assert!(retry_signal_set_hash.starts_with("sha256:"));

    let retry_chain_exact: (bool, bool, bool, bool) = sqlx::query_as(
        r#"SELECT
             successor.predecessor_attempt_id=predecessor.analysis_attempt_id,
             successor.attempt_input_hash=tool_truth_sha256(jsonb_build_object(
               'schema','candidate_retry_attempt_input.v1',
               'predecessor_attempt_id',predecessor.analysis_attempt_id,
               'predecessor_attempt_input_hash',predecessor.attempt_input_hash,
               'predecessor_terminal_event_id',terminal.attempt_event_id,
               'predecessor_terminal_event_hash',terminal.event_hash,
               'missed_hypothesis_signal_count',$3::BIGINT,
               'missed_hypothesis_signal_set_hash',$4::TEXT
             )::TEXT),
             terminal.event_kind='superseded_missed_hypothesis',
             EXISTS(SELECT 1 FROM candidate_analysis_attempt_state_events sealed
                     WHERE sealed.analysis_attempt_id=successor.analysis_attempt_id
                       AND sealed.event_kind='sealed')
           FROM candidate_analysis_attempts successor
           JOIN candidate_analysis_attempts predecessor
             ON predecessor.analysis_attempt_id=$2
           JOIN candidate_analysis_attempt_state_events terminal
             ON terminal.analysis_attempt_id=predecessor.analysis_attempt_id
            AND terminal.event_kind='superseded_missed_hypothesis'
          WHERE successor.analysis_attempt_id=$1"#,
    )
    .bind(analysis_attempt_id)
    .bind(preflight_attempt.analysis_attempt_id)
    .bind(i64::try_from(controller_retry.0.len()).expect("retry signal count fits i64"))
    .bind(retry_signal_set_hash)
    .fetch_one(db.pool())
    .await
    .expect("inspect stateful Candidate retry chain");
    assert_eq!(retry_chain_exact, (true, true, true, true));

    let semantic_retry_closure: (bool, bool, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             first_review.outcome='missed_hypothesis',
             retry_review.outcome='no_composite_miss',
             jsonb_array_length(first_review.semantic_summary->'missed_checklist_member_ids')::BIGINT,
             jsonb_array_length(first_review.semantic_summary->'semantic_observations')::BIGINT,
             (SELECT count(*) FROM candidate_analysis_artifacts first_artifact
               JOIN candidate_analysis_artifacts retry_artifact
                 ON retry_artifact.artifact_id=first_artifact.artifact_id
              WHERE first_artifact.analysis_attempt_id=$1
                AND retry_artifact.analysis_attempt_id=$2),
             (SELECT count(*) FROM candidate_analysis_provider_attempts first_provider
               JOIN candidate_analysis_provider_attempts retry_provider
                 ON retry_provider.provider_attempt_id=first_provider.provider_attempt_id
              WHERE first_provider.analysis_attempt_id=$1
                AND retry_provider.analysis_attempt_id=$2)
           FROM candidate_analysis_hypothesis_coverage_synthesis_reviews first_review
           JOIN candidate_analysis_hypothesis_coverage_synthesis_census_members first_node
             ON first_node.synthesis_node_id=first_review.synthesis_node_id
            AND first_node.node_kind='global_semantic_root'
           JOIN candidate_analysis_hypothesis_coverage_synthesis_reviews retry_review ON TRUE
           JOIN candidate_analysis_hypothesis_coverage_synthesis_census_members retry_node
             ON retry_node.synthesis_node_id=retry_review.synthesis_node_id
            AND retry_node.node_kind='global_semantic_root'
          WHERE first_review.analysis_attempt_id=$1
            AND retry_review.analysis_attempt_id=$2"#,
    )
    .bind(preflight_attempt.analysis_attempt_id)
    .bind(analysis_attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect semantic retry propagation and cross-attempt identity isolation");
    assert!(semantic_retry_closure.0 && semantic_retry_closure.1);
    assert!(semantic_retry_closure.2 > 0 && semantic_retry_closure.3 > 0);
    assert_eq!((semantic_retry_closure.4, semantic_retry_closure.5), (0, 0));

    let conflict_closure: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM candidate_analysis_conflict_components
               WHERE analysis_attempt_id=$1),
             (SELECT count(*) FROM hypothesis_merge_decisions
               WHERE analysis_attempt_id=$1),
             (SELECT count(*) FROM candidate_analysis_conflict_components
               WHERE analysis_attempt_id=$2),
             (SELECT count(*) FROM hypothesis_merge_decisions
               WHERE analysis_attempt_id=$2)"#,
    )
    .bind(preflight_attempt.analysis_attempt_id)
    .bind(analysis_attempt_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect conflict component and exact review closure");
    assert_eq!(conflict_closure, (1, 1, 1, 1));
    let persisted: (i64, i64, i64, bool, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM hypothesis_generation_seals seal
               JOIN hypothesis_generations generation USING(generation_id)
              WHERE generation.candidate_snapshot_id=$1),
             (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
              WHERE analysis_attempt_id=$2),
             (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_node_children child
               JOIN candidate_analysis_hypothesis_coverage_synthesis_censuses census USING(synthesis_census_id)
              WHERE census.analysis_attempt_id=$2),
             EXISTS(
               SELECT 1 FROM candidate_analysis_snapshots snapshot
               JOIN tool_truth_authority_bundle_seals bundle
                 ON bundle.id=snapshot.tool_truth_authority_bundle_seal_id
              WHERE snapshot.snapshot_id=$1 AND bundle.id=$3
                AND bundle.sealed_at IS NOT NULL
                AND bundle.consistent_fresh_count=bundle.member_count
                AND bundle.stale_or_invalid_count=0
             ),
             (SELECT count(*) FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1),
             (SELECT count(*) FROM candidate_analysis_input_chunk_censuses WHERE snapshot_id=$1)"#,
    )
    .bind(snapshot_id)
    .bind(analysis_attempt_id)
    .bind(checked_bundle_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect generation and recursive coverage closure");
    assert_eq!(persisted.0, 1);
    assert!(persisted.1 > 0 && persisted.2 > 0);
    assert!(persisted.3);
    assert!(persisted.4 > 0 && persisted.5 > 0);
    let matcher_exact_five: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT census.input_product_count,census.input_feed_count,census.member_count,
                  (SELECT count(*) FROM candidate_analysis_product_version_census_members
                    WHERE snapshot_id=$1),
                  (SELECT count(*) FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND disposition='matched'),
                  (SELECT count(DISTINCT feed_snapshot_member_id)
                     FROM candidate_analysis_feed_match_census_members WHERE snapshot_id=$1),
                  (SELECT count(*) FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND disposition<>'matched')
             FROM candidate_analysis_feed_match_censuses census
            WHERE census.snapshot_id=$1"#,
    )
    .bind(snapshot_id)
    .fetch_one(db.pool())
    .await
    .expect("inspect persisted product by exact-five feed matcher closure");
    assert_eq!(matcher_exact_five, (1, 5, 5, 1, 5, 5, 0));
    db.stop().await;
}
