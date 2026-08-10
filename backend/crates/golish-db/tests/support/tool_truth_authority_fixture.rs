use golish_db::repo::{
    capability_execution_receipts, legacy_report_authority, legacy_security_verdict,
    stage_asset_waves,
};
use golish_pentest_domain::tool_truth::ToolTruthRootFamilyV1;
use sqlx::PgPool;
use uuid::Uuid;

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

fn compile_root_denominator(
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

/// Seed the four real receipt roots consumed by the request-scoped Tool Truth
/// authority host. The helper deliberately uses the production denominator,
/// destination-policy, managed-begin and finalization APIs; it does not insert
/// bundle headers or members directly.
pub async fn seed_all_fresh_tool_truth_roots(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    project_path: &str,
) -> i64 {
    let current_contract: String =
        sqlx::query_scalar("SELECT tool_truth_contract FROM operation_state WHERE operation_id=$1")
            .bind(operation_id)
            .fetch_one(pool)
            .await
            .expect("load fixture Tool Truth contract");
    if current_contract == "legacy_v1" {
        let mut contract_tx = pool.begin().await.expect("begin shadow receipt fixture");
        sqlx::query(
            "ALTER TABLE operation_state DISABLE TRIGGER operation_state_tool_truth_contract_immutable",
        )
        .execute(&mut *contract_tx)
        .await
        .expect("disable operation Tool Truth immutability in isolated fixture");
        sqlx::query(
            "UPDATE operation_state SET tool_truth_contract='shadow_v1' WHERE operation_id=$1",
        )
        .bind(operation_id)
        .execute(&mut *contract_tx)
        .await
        .expect("select the legal shadow_v1 + legacy Investigation fixture pair");
        sqlx::query(
            "ALTER TABLE operation_state ENABLE TRIGGER operation_state_tool_truth_contract_immutable",
        )
        .execute(&mut *contract_tx)
        .await
        .expect("restore operation Tool Truth immutability");
        contract_tx
            .commit()
            .await
            .expect("commit isolated shadow receipt fixture");
    }
    let target_id = Uuid::new_v4();
    let target_host = format!("report-authority-{}.example.test", target_id.simple());
    let exact_origin = format!("https://{target_host}:443");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source,ports
           ) VALUES($1,$2,'domain',$2,'in',$3,$4,'report_authority_fixture',$5)"#,
    )
    .bind(target_id)
    .bind(&target_host)
    .bind(project_path)
    .bind(organization_id)
    .bind(serde_json::json!([{
        "port":443,"state":"open","service":"https",
        "url":format!("{exact_origin}/")
    }]))
    .execute(pool)
    .await
    .expect("insert Tool Truth authority target");
    let web_origin_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO web_origins(
               id,organization_id,project_path,scheme,host,host_type,port,origin,
               source,confidence,last_confirmed_at
           ) VALUES($1,$2,$3,'https',$4,'domain',443,$5,'httpx',1.0,NOW())"#,
    )
    .bind(web_origin_id)
    .bind(organization_id)
    .bind(project_path)
    .bind(&target_host)
    .bind(&exact_origin)
    .execute(pool)
    .await
    .expect("insert Tool Truth authority web origin");
    sqlx::query(
        r#"INSERT INTO web_origin_observations(
               id,organization_id,project_path,web_origin_id,target_id,status_code,
               confidence,source,raw
           ) VALUES($1,$2,$3,$4,$5,200,1.0,'httpx','{}')"#,
    )
    .bind(Uuid::new_v4())
    .bind(organization_id)
    .bind(project_path)
    .bind(web_origin_id)
    .bind(target_id)
    .execute(pool)
    .await
    .expect("bind Tool Truth target to its exact origin");

    for family in ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS {
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
        .expect("create Tool Truth root wave")
        .expect("Tool Truth root wave has a target");
        let denominator = capability_execution_receipts::seal_source_denominator(
            pool,
            &capability_execution_receipts::SealSourceDenominator {
                stable_seal_request_id: Uuid::new_v5(
                    &stage_execution_id,
                    format!("report-authority-root:{}", family.as_str()).as_bytes(),
                ),
                stage_execution_id,
                source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                    wave.wave.id,
                ),
            },
            compile_root_denominator,
        )
        .await
        .expect("seal Tool Truth root denominator");
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
        .expect("seal Tool Truth destination policy");
        let receipt = capability_execution_receipts::begin_managed(
            pool,
            &capability_execution_receipts::BeginManagedCapabilityReceipt {
                id: Uuid::new_v4(),
                denominator_id: denominator.id,
                capability: "intel.dns".to_owned(),
                attempt_ordinal: 1,
                destination_policy_id: policy.id,
            },
        )
        .await
        .expect("begin Tool Truth receipt with exact destination policy");
        let input_keys: Vec<String> = sqlx::query_scalar(
            "SELECT input_key FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY input_key",
        )
        .bind(denominator.id)
        .fetch_all(pool)
        .await
        .expect("load Tool Truth denominator members");
        let normalized_record_count =
            i64::try_from(input_keys.len()).expect("fixture denominator count fits i64");
        capability_execution_receipts::finalize_target_intel_receipt(
            pool,
            &capability_execution_receipts::FinalizeTargetIntelReceipt {
                receipt_id: receipt.id,
                expected_row_version: receipt.row_version,
                attempt_fence: None,
                raw_witness: capability_execution_receipts::RawWitnessArtifactInput {
                    artifact_id: Uuid::new_v5(&receipt.id, b"report-authority-witness"),
                    content_key: digest('1'),
                    vault_object_ref_token: vec![7; 32],
                    vault_object_ref_token_hash: digest('2'),
                    sha256: digest('1'),
                    ciphertext_sha256: digest('3'),
                    operation_key_ref_hash: digest('4'),
                    key_generation: 1,
                    retention_policy_id: Uuid::new_v5(&receipt.id, b"report-authority-retention"),
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
                    path_and_query: "/v1/query?input=report-authority.example.test".to_owned(),
                    addresses: vec!["1.1.1.1".parse().expect("fixture IP")],
                    selected_address: "1.1.1.1".parse().expect("fixture IP"),
                    send_ordinal: 1,
                }],
                request_count: 1,
                response_byte_count: 2,
                wall_clock_ms: 1,
                retry_count: 0,
                parser_complete: true,
                normalized_record_count,
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
                typed_landing: serde_json::json!({
                    "kind":"report_authority_test",
                    "version":1,
                    "project_path":project_path,
                }),
                failure_reason_code: None,
            },
        )
        .await
        .expect("finalize one fresh receipt for each Tool Truth root");
    }
    seed_legacy_report_authority(pool, operation_id, organization_id, project_path, target_id).await
}

async fn seed_legacy_report_authority(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    project_path: &str,
    target_id: Uuid,
) -> i64 {
    let (project_scope_id, scope_snapshot_id): (Uuid, Uuid) = sqlx::query_as(
        r#"SELECT operation.project_scope_id,snapshot.id
             FROM operation_state operation
             JOIN operation_org_scope_snapshots snapshot
               ON snapshot.operation_id=operation.operation_id
              AND snapshot.sealed_at IS NOT NULL
            WHERE operation.operation_id=$1
            ORDER BY snapshot.sealed_at DESC,snapshot.id DESC LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await
    .expect("load legacy report authority scope");
    let existing_evidence_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM audit_log
            WHERE run_id=$1 AND audit_role='evidence'
              AND detail->>'organization_id'=$2
            ORDER BY id LIMIT 1"#,
    )
    .bind(operation_id)
    .bind(organization_id.to_string())
    .fetch_optional(pool)
    .await
    .expect("load existing operation evidence for legacy authority");
    let evidence_id = if let Some(evidence_id) = existing_evidence_id {
        evidence_id
    } else {
        sqlx::query_scalar(
            r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,audit_role
           ) VALUES(
               'legacy-report-authority-fixture','harness',
               'retained legacy report authority evidence',$1,'harness','completed',$2,$3,
               'evidence'
           ) RETURNING id"#,
        )
        .bind(project_path)
        .bind(serde_json::json!({"organization_id":organization_id}))
        .bind(operation_id)
        .fetch_one(pool)
        .await
        .expect("insert legacy report authority evidence")
    };
    let candidate_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let hypothesis_root_id = Uuid::new_v4();
    let hypothesis_revision_id = Uuid::new_v4();
    let target_identity_hash = digest('6');
    let candidate_plan_hash = digest('7');
    let target_value = format!(
        "https://report-authority-{}.example.test/login",
        target_id.simple()
    );
    let mut tx = pool
        .begin()
        .await
        .expect("begin legacy report authority fixture");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *tx)
        .await
        .expect("isolate legacy report authority parents");
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash
           ) VALUES($1,$2,$3,'initial',$4,$5)"#,
    )
    .bind(hypothesis_root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(serde_json::json!({"fixture":"legacy-report-authority"}))
    .bind(digest('b'))
    .execute(&mut *tx)
    .await
    .expect("insert legacy report hypothesis root");
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,revision_ordinal,
               semantic_key,semantic_key_hash,subject_kind,subject_identity_hash,
               target_live_id,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,
               structured_claim,assumptions,missing_facts,priority,risk_impact,
               origin_decision_hash,revision_ingredients_hash,revision_hash
           ) VALUES(
               $1,$2,$3,$4,0,$5,$6,'url',$7,$8,'url',$9,
               'legacy_report_fixture',1,$10,'external','positive','refuted','closed',
               'deferred',$11,'[]','[]',0,$12,$13,$14,$15
           )"#,
    )
    .bind(hypothesis_revision_id)
    .bind(hypothesis_root_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(serde_json::json!({"target":target_value,"predicate":"legacy_report_fixture"}))
    .bind(digest('c'))
    .bind(&target_identity_hash)
    .bind(target_id)
    .bind(&target_value)
    .bind(serde_json::json!({}))
    .bind(serde_json::json!({"disposition":"refuted"}))
    .bind(serde_json::json!({"impact":"none"}))
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(digest('f'))
    .execute(&mut *tx)
    .await
    .expect("insert legacy report hypothesis revision");
    sqlx::query(
        r#"INSERT INTO attack_candidates(
               candidate_id,operation_id,organization_id,target,hypothesis,
               hypothesis_hash,technique,rationale,priority,wave,disposition,
               operation_uuid,scope_snapshot_id,wave_run_id,wave_unit_id,
               source_work_item_id,decision_stage_execution_id,
               decision_stage_run_unit_id,decision_deliverable_submission_id,
               decision_stage_kind,target_live_id,target_id_at_time,live_target_id,
               canonical_target_snapshot,target_type_at_time,
               target_value_at_time,target_identity_hash,execution_plan,
               candidate_plan_hash,risk_class,hypothesis_revision_id
           ) VALUES(
               $1,$2,$3,$4,'legacy report authority hypothesis',$5,'WSTG-INFO-01',
               'report authority fixture','medium',0,'refuted',$6,$7,$8,$9,$10,$11,
               $12,$13,'attack_candidate',$14,$14,$14,$19,'url',$4,$15,$16,$17,
               'deterministic_safe',$18
           )"#,
    )
    .bind(candidate_id)
    .bind(operation_id.to_string())
    .bind(organization_id)
    .bind(&target_value)
    .bind(digest('8'))
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(target_id)
    .bind(&target_identity_hash)
    .bind(serde_json::json!({"schema_version":"report-authority-fixture-v1"}))
    .bind(&candidate_plan_hash)
    .bind(hypothesis_revision_id)
    .bind(serde_json::json!({
        "targetIdAtTime":target_id,
        "targetTypeAtTime":"url",
        "targetValueAtTime":target_value,
        "targetIdentityHash":target_identity_hash,
    }))
    .execute(&mut *tx)
    .await
    .expect("insert legacy report Candidate authority parent");
    sqlx::query(
        r#"INSERT INTO candidate_attempts(
               id,candidate_id,approval_id,operation_id,scope_snapshot_id,
               wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               candidate_plan_hash,ordinal,status,result_json,result_hash,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'url',$10,$11,$12,0,'refuted',
               $13,$14,NOW()
           )"#,
    )
    .bind(attempt_id)
    .bind(candidate_id)
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(organization_id)
    .bind(target_id)
    .bind(&target_value)
    .bind(&target_identity_hash)
    .bind(&candidate_plan_hash)
    .bind(serde_json::json!({"disposition":"refuted"}))
    .bind(digest('9'))
    .execute(&mut *tx)
    .await
    .expect("insert legacy report terminal Attempt");
    sqlx::query(
        "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role) VALUES($1,$2,'proof')",
    )
    .bind(attempt_id)
    .bind(evidence_id)
    .execute(&mut *tx)
    .await
    .expect("link evidence to legacy report Attempt");
    sqlx::query("SET LOCAL session_replication_role = 'origin'")
        .execute(&mut *tx)
        .await
        .expect("restore legacy report authority guards");
    legacy_security_verdict::seal_legacy_attempt_authority_on(
        &mut tx,
        legacy_security_verdict::SealLegacyAttemptAuthorityV1 {
            operation_id,
            project_scope_id,
            organization_id,
            attempt_id,
            hypothesis_revision_id,
            adapter_version: "report-authority-fixture-v1".to_owned(),
            adapter_digest: digest('a'),
        },
    )
    .await
    .expect("seal legacy Attempt report authority");
    legacy_report_authority::seal_legacy_report_authority_on(
        &mut tx,
        legacy_report_authority::SealLegacyReportAuthorityV1 {
            operation_id,
            project_scope_id,
            adapter_version: "report-authority-fixture-v1".to_owned(),
            adapter_digest: digest('a'),
        },
    )
    .await
    .expect("seal operation-wide legacy report authority");
    tx.commit()
        .await
        .expect("commit legacy report authority fixture");
    evidence_id
}
