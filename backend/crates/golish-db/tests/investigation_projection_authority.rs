use golish_core::{
    hypothesis_semantic_key::CanonicalJsonObject,
    investigation_projection::{
        GenerationProjectionRecordV1, ProjectionChangeKind, ProjectionSourceSnapshotV1,
        ProjectionSourceTimeStatusV1,
    },
};
use golish_db::{
    repo::investigation_projection::{
        capture_investigation_read_authority, enqueue_projection_batch_on,
        project_projection_batch, InvestigationProjectionError, ProjectionOutboxBatchInput,
        ProjectionOutboxMemberInput, ProjectionSourceStorageV1,
    },
    DbConfig, GolishDb,
};
use serial_test::serial;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("projection_authority_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

#[derive(Clone, Copy)]
struct OperationFixture {
    operation_id: Uuid,
    organization_id: Uuid,
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
}

async fn seed_operation(pool: &PgPool, label: &str) -> OperationFixture {
    let operation_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let stage_run_id = Uuid::new_v4();
    let project_path = format!(
        "/tmp/projection-authority-{label}-{}",
        Uuid::new_v4().simple()
    );
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) \
         VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest('1'))
    .execute(pool)
    .await
    .expect("insert authority project scope");
    sqlx::query(
        r#"INSERT INTO operation_state(
           operation_id,profile,current_stage,runtime_memory_contract,
           attack_execution_contract,tool_truth_contract,project_scope_id,
           investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'assessment','target_intel','legacy_v1','legacy','receipt_v1',$2,
                    'hypothesis_registry_v1','new_only')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert authority operation");
    sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,$3)")
        .bind(organization_id)
        .bind(&project_path)
        .bind(format!("Authority {label}"))
        .execute(pool)
        .await
        .expect("insert authority organization");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'target_intel','started')",
    )
    .bind(stage_run_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert authority stage run");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_run_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest('2'))
    .execute(pool)
    .await
    .expect("insert authority scope decision");
    let mut tx = pool.begin().await.expect("begin authority scope freeze");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_decision_id)
    .bind(&project_path)
    .bind(organization_id)
    .bind(digest('3'))
    .execute(&mut *tx)
    .await
    .expect("insert authority scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Authority Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "fixture"}))
    .execute(&mut *tx)
    .await
    .expect("insert authority scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *tx)
        .await
        .expect("seal authority scope snapshot");
    tx.commit().await.expect("commit authority scope freeze");
    OperationFixture {
        operation_id,
        organization_id,
        project_scope_id,
        scope_snapshot_id,
    }
}

async fn project_generation(pool: &PgPool, operation: OperationFixture, snapshot_id: Uuid) {
    let generation_id = Uuid::new_v4();
    let body = CanonicalJsonObject::try_from_value(serde_json::json!({
        "generation_id": generation_id,
        "candidate_snapshot_id": snapshot_id,
        "generation_hash": digest('4')
    }))
    .expect("canonical generation body");
    let source = ProjectionSourceSnapshotV1::Generation(
        GenerationProjectionRecordV1::try_new(generation_id.to_string(), 1, 1, body)
            .expect("typed generation projection"),
    );
    let batch_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin generation source batch");
    enqueue_projection_batch_on(
        &mut tx,
        ProjectionOutboxBatchInput {
            batch_id,
            operation_id: operation.operation_id,
            project_scope_id: Some(operation.project_scope_id),
            stable_request_id: Uuid::new_v4(),
            source_transaction_id: Uuid::new_v4(),
            source_occurred_at: None,
            source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
            members: vec![ProjectionOutboxMemberInput {
                outbox_member_id: Uuid::new_v4(),
                change_kind: ProjectionChangeKind::Insert,
                source,
                source_occurred_at: None,
                source_time_status: ProjectionSourceTimeStatusV1::HistoricalUnknown,
                invalidation_reason: None,
                storage: ProjectionSourceStorageV1::Inline,
            }],
        },
    )
    .await
    .expect("append generation source batch");
    tx.commit().await.expect("commit generation source batch");
    project_projection_batch(pool, operation.operation_id, batch_id)
        .await
        .expect("project generation source batch");
}

#[derive(Clone, Copy)]
struct RawAuthorityOptions {
    roots: usize,
    temporal_member: bool,
    target_head: bool,
    feed_contract: bool,
    feed_member_heads: usize,
    unavailable_witness_members: usize,
    expired: bool,
}

async fn force_registry_contract(tx: &mut Transaction<'_, Postgres>, operation_id: Uuid) {
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut **tx)
        .await
        .expect("disable fixture triggers");
    sqlx::query(
        r#"UPDATE operation_state
              SET tool_truth_contract='receipt_v1',
                  investigation_contract_version='hypothesis_registry_v1',
                  investigation_rollout_mode='registry_authoritative_legacy_projection',
                  stage_topology_contract='unified_investigation_v1',
                  stage_topology_canonical_json=stage_topology_canonical_json(
                      'unified_investigation_v1'
                  ),
                  stage_topology_sha256=stage_topology_contract_sha256(
                      'unified_investigation_v1'
                  ),
                  stage_topology_freeze_source='deployment_pair_v1'
            WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .execute(&mut **tx)
    .await
    .expect("freeze Registry read contract in isolated fixture");
}

async fn insert_raw_authority(
    pool: &PgPool,
    operation: OperationFixture,
    snapshot_id: Uuid,
    options: RawAuthorityOptions,
) {
    let bundle_id = Uuid::new_v4();
    let stable_request_id = Uuid::new_v4();
    let catalog_id = Uuid::new_v4();
    let trust_policy_id = Uuid::new_v4();
    let authority_set_ids = [
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    ];
    let roots = ["eas", "enum", "vuln"];
    let mut tx = pool.begin().await.expect("begin raw authority fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *tx)
        .await
        .expect("disable fixture referential triggers");
    sqlx::query(
        r#"INSERT INTO tool_truth_authority_bundle_seals(
               id,operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,consumer_kind,stable_consumer_request_id,
               relevant_root_count,relevant_root_set_hash,member_count,member_set_hash,
               semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
               temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
               target_state_epoch_set_hash,observation_window_started_at,
               observation_window_completed_at,effective_valid_until,
               consistent_fresh_count,stale_or_invalid_count,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,'candidate_analysis',$7,3,$8,3,$9,$10,$11,
                    $12,$13,$14,NOW()-INTERVAL '2 minutes',NOW()-INTERVAL '1 minute',
                    CASE WHEN $15 THEN NOW()-INTERVAL '1 second'
                         ELSE NOW()+INTERVAL '10 minutes' END,3,0,NOW())"#,
    )
    .bind(bundle_id)
    .bind(operation.operation_id)
    .bind(operation.project_scope_id)
    .bind(format!("/tmp/raw-authority-{}", operation.operation_id))
    .bind(operation.scope_snapshot_id)
    .bind(operation.organization_id)
    .bind(stable_request_id)
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(options.expired)
    .execute(&mut *tx)
    .await
    .expect("insert raw authority bundle");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshots(
               snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
               genesis,source_set_hash,capability_revision_hash,policy_revision_hash,
               credential_revision_hash,snapshot_status,tool_truth_authority_bundle_seal_id,
               stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
               bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash
           ) VALUES($1,$2,$3,0,$4,TRUE,$5,$6,$7,$8,'sealed_ready',$9,$10,3,$11,3,$12,
                    $13,$14,$15,$16,$17,$18,NOW(),$19)"#,
    )
    .bind(snapshot_id)
    .bind(operation.operation_id)
    .bind(operation.organization_id)
    .bind(operation.scope_snapshot_id)
    .bind(digest('c'))
    .bind(digest('d'))
    .bind(digest('e'))
    .bind(digest('f'))
    .bind(bundle_id)
    .bind(stable_request_id)
    .bind(digest('5'))
    .bind(digest('6'))
    .bind(digest('7'))
    .bind(digest('8'))
    .bind(digest('9'))
    .bind(digest('a'))
    .bind(digest('b'))
    .bind(digest('c'))
    .bind(digest('d'))
    .execute(&mut *tx)
    .await
    .expect("insert raw Candidate snapshot");

    for (ordinal, (root, authority_set_id)) in roots
        .iter()
        .zip(authority_set_ids)
        .take(options.roots)
        .enumerate()
    {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_snapshot_authority_bundle_members(
                   snapshot_member_id,snapshot_id,operation_id,organization_id,bundle_seal_id,
                   tool_truth_authority_bundle_member_id,ordinal,root_family,
                   root_execution_authority_id,root_denominator_id,root_denominator_hash,
                   authority_set_seal_id,authority_set_semantic_hash,authority_set_graph_hash,
                   authority_set_freshness_hash,temporal_validity_policy_set_hash,
                   target_state_epoch_set_hash,semantic_status,temporal_validity_status,
                   member_status,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                        'consistent','fresh','consistent_fresh',$18)"#,
        )
        .bind(Uuid::new_v4())
        .bind(snapshot_id)
        .bind(operation.operation_id)
        .bind(operation.organization_id)
        .bind(bundle_id)
        .bind(Uuid::new_v4())
        .bind(ordinal as i32)
        .bind(*root)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(digest('e'))
        .bind(authority_set_id)
        .bind(digest('f'))
        .bind(digest('1'))
        .bind(digest('2'))
        .bind(digest('3'))
        .bind(digest('4'))
        .bind(digest('5'))
        .execute(&mut *tx)
        .await
        .expect("insert raw snapshot root member");
    }
    sqlx::query(
        r#"INSERT INTO candidate_analysis_temporal_validity_censuses(
               census_id,snapshot_id,tool_truth_authority_bundle_seal_id,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               decision_count,decision_set_hash,census_hash
           ) VALUES($1,$2,$3,$4,$5,0,$6,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(snapshot_id)
    .bind(bundle_id)
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(&mut *tx)
    .await
    .expect("insert exact-empty temporal decision census");

    if options.temporal_member {
        let receipt_id = Uuid::new_v4();
        let target_hash = digest('7');
        let target_event_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO tool_truth_authority_set_members(
                   id,authority_set_id,execution_authority_id,denominator_id,receipt_id,
                   reconciliation_id,semantic_authority_version,semantic_hash,ordinal,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,1,$7,0,$8)"#,
        )
        .bind(Uuid::new_v4())
        .bind(authority_set_ids[0])
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(receipt_id)
        .bind(Uuid::new_v4())
        .bind(digest('8'))
        .bind(digest('9'))
        .execute(&mut *tx)
        .await
        .expect("insert raw authority receipt member");
        sqlx::query(
            r#"INSERT INTO capability_execution_temporal_census_members(
                   id,census_id,receipt_id,execution_authority_id,ordinal,input_key,
                   observation_identity_hash,temporal_validity_policy_id,policy_member_id,
                   policy_member_hash,target_state_operation_id,target_state_organization_id,
                   target_scope_identity_hash,target_state_epoch_event_id,target_state_epoch,
                   temporal_fact_class,observation_polarity,mapping_rule_id,
                   mapping_rule_version,mapping_rule_digest,selected_ttl_ms,observed_at,
                   effective_valid_until,member_hash
               ) VALUES($1,$2,$3,$4,0,'fixture',$5,$6,$7,$8,$9,$10,$11,$12,0,
                        'dns','positive','fixture','1',$13,600000,NOW()-INTERVAL '1 minute',
                        NOW()+INTERVAL '9 minutes',$14)"#,
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(receipt_id)
        .bind(Uuid::new_v4())
        .bind(digest('a'))
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(digest('b'))
        .bind(operation.operation_id)
        .bind(operation.organization_id)
        .bind(&target_hash)
        .bind(target_event_id)
        .bind(digest('c'))
        .bind(digest('d'))
        .execute(&mut *tx)
        .await
        .expect("insert raw authority temporal member");
        if options.target_head {
            sqlx::query(
                r#"INSERT INTO tool_truth_target_state_epoch_heads(
                       operation_id,organization_id,target_scope_identity_hash,
                       current_epoch,current_event_id,row_version
                   ) VALUES($1,$2,$3,0,$4,0)"#,
            )
            .bind(operation.operation_id)
            .bind(operation.organization_id)
            .bind(target_hash)
            .bind(target_event_id)
            .execute(&mut *tx)
            .await
            .expect("insert current target epoch head");
        }
    }

    let denominator_id = Uuid::new_v4();
    let feed_snapshot_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO candidate_analysis_knowledge_feed_denominators(
               denominator_id,snapshot_id,catalog_id,catalog_version,catalog_hash,
               trust_policy_id,trust_policy_version,trust_policy_hash,
               signature_algorithm_allowlist_hash,trust_store_version,trust_store_hash,
               key_revocation_epoch,key_revocation_epoch_hash,required_source_count,
               required_source_set_hash,required_member_count,required_member_set_hash,
               denominator_hash
           ) VALUES($1,$2,$3,1,$4,$5,1,$6,$7,1,$8,0,$9,5,$10,5,$11,$12)"#,
    )
    .bind(denominator_id)
    .bind(snapshot_id)
    .bind(catalog_id)
    .bind(digest('e'))
    .bind(trust_policy_id)
    .bind(digest('f'))
    .bind(digest('1'))
    .bind(digest('2'))
    .bind(digest('3'))
    .bind(digest('4'))
    .bind(digest('5'))
    .bind(digest('6'))
    .execute(&mut *tx)
    .await
    .expect("insert raw feed denominator");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_knowledge_feed_snapshots(
               feed_snapshot_id,snapshot_id,denominator_id,trust_policy_hash,
               trust_store_hash,key_revocation_epoch,member_count,member_set_hash,
               feed_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5,0,5,$6,$7)"#,
    )
    .bind(feed_snapshot_id)
    .bind(snapshot_id)
    .bind(denominator_id)
    .bind(digest('f'))
    .bind(digest('2'))
    .bind(digest('7'))
    .bind(digest('8'))
    .execute(&mut *tx)
    .await
    .expect("insert raw feed snapshot");
    let unavailable_members = [
        ("cve", "managed:cve"),
        ("cpe", "managed:cpe"),
        ("kev", "managed:kev"),
        ("vendor_advisory", "managed:vendor-advisory"),
        ("detection_rule", "managed:detection-rule"),
    ];
    for (ordinal, (source_kind, source_identity)) in unavailable_members.iter().enumerate() {
        let expected_member_id = Uuid::new_v4();
        let member_hash = digest(char::from_digit((ordinal + 10) as u32, 16).unwrap_or('a'));
        if options.unavailable_witness_members > 0 {
            sqlx::query(
                r#"INSERT INTO candidate_analysis_knowledge_feed_denominator_members(
                       expected_member_id,denominator_id,snapshot_id,ordinal,source_kind,
                       source_identity,schema_name,minimum_schema_version,member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,'managed_knowledge_feed.v1',1,$7)"#,
            )
            .bind(expected_member_id)
            .bind(denominator_id)
            .bind(snapshot_id)
            .bind(ordinal as i32)
            .bind(*source_kind)
            .bind(*source_identity)
            .bind(&member_hash)
            .execute(&mut *tx)
            .await
            .expect("insert raw unavailable denominator member");
        }
        let feed_snapshot_member_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO candidate_analysis_knowledge_feed_snapshot_members(
                   feed_snapshot_member_id,feed_snapshot_id,snapshot_id,denominator_id,
                   expected_member_id,ordinal,feed_schema,provenance,age_policy_version,
                   age_policy_digest,disposition,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,'managed_knowledge_feed.v1','{}'::JSONB,
                        '1',$7,'unavailable',$8)"#,
        )
        .bind(feed_snapshot_member_id)
        .bind(feed_snapshot_id)
        .bind(snapshot_id)
        .bind(denominator_id)
        .bind(expected_member_id)
        .bind(ordinal as i32)
        .bind(digest('f'))
        .bind(&member_hash)
        .execute(&mut *tx)
        .await
        .expect("insert raw feed snapshot member");
        if ordinal < options.unavailable_witness_members {
            sqlx::query(
                r#"INSERT INTO candidate_analysis_enrichment_obligations(
                       obligation_id,snapshot_id,obligation_kind,feed_snapshot_member_id,
                       reason_code,affected_checklist_member_key,obligation_hash
                   ) VALUES($1,$2,'feed_refresh',$3,'managed_feed_catalog_unavailable',$4,$5)"#,
            )
            .bind(Uuid::new_v4())
            .bind(snapshot_id)
            .bind(feed_snapshot_member_id)
            .bind(format!("feed:{source_kind}"))
            .bind(digest('e'))
            .execute(&mut *tx)
            .await
            .expect("insert raw unavailable feed obligation");
        }
    }

    if options.feed_contract {
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_catalog_head(
               singleton,catalog_id,catalog_version,catalog_hash,trust_policy_id,
               trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
               required_source_count,required_source_set_hash,required_member_count,
               required_member_set_hash,head_version
           ) VALUES(TRUE,$1,1,$2,$3,1,$4,$5,5,$6,5,$7,0)
           ON CONFLICT(singleton) DO UPDATE SET catalog_id=EXCLUDED.catalog_id,
             catalog_version=EXCLUDED.catalog_version,catalog_hash=EXCLUDED.catalog_hash,
             trust_policy_id=EXCLUDED.trust_policy_id,
             trust_policy_version=EXCLUDED.trust_policy_version,
             trust_policy_hash=EXCLUDED.trust_policy_hash,
             signature_algorithm_allowlist_hash=EXCLUDED.signature_algorithm_allowlist_hash,
             required_source_count=EXCLUDED.required_source_count,
             required_source_set_hash=EXCLUDED.required_source_set_hash,
             required_member_count=EXCLUDED.required_member_count,
             required_member_set_hash=EXCLUDED.required_member_set_hash"#,
        )
        .bind(catalog_id)
        .bind(digest('e'))
        .bind(trust_policy_id)
        .bind(digest('f'))
        .bind(digest('1'))
        .bind(digest('3'))
        .bind(digest('4'))
        .execute(&mut *tx)
        .await
        .expect("install raw current feed catalog head");
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_trust_store_head(
               singleton,trust_store_version,trust_store_hash,key_revocation_epoch,
               key_revocation_epoch_hash,head_version
           ) VALUES(TRUE,1,$1,0,$2,0)
           ON CONFLICT(singleton) DO UPDATE SET
             trust_store_version=EXCLUDED.trust_store_version,
             trust_store_hash=EXCLUDED.trust_store_hash,
             key_revocation_epoch=EXCLUDED.key_revocation_epoch,
             key_revocation_epoch_hash=EXCLUDED.key_revocation_epoch_hash"#,
        )
        .bind(digest('2'))
        .bind(digest('3'))
        .execute(&mut *tx)
        .await
        .expect("install raw current feed trust head");
        sqlx::query(
            r#"INSERT INTO candidate_operation_managed_feed_contracts(
                   operation_id,catalog_id,catalog_version,catalog_hash,trust_policy_id,
                   trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
                   required_source_count,required_source_set_hash,required_member_count,
                   required_member_set_hash
               ) VALUES($1,$2,1,$3,$4,1,$5,$6,5,$7,5,$8)"#,
        )
        .bind(operation.operation_id)
        .bind(catalog_id)
        .bind(digest('e'))
        .bind(trust_policy_id)
        .bind(digest('f'))
        .bind(digest('1'))
        .bind(digest('3'))
        .bind(digest('4'))
        .execute(&mut *tx)
        .await
        .expect("insert raw operation feed contract");
    }
    for _ in 0..options.feed_member_heads {
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_store_member_heads(
                   catalog_member_id,catalog_id,store_member_id,head_version
               ) VALUES($1,$2,$3,0)"#,
        )
        .bind(Uuid::new_v4())
        .bind(catalog_id)
        .bind(Uuid::new_v4())
        .execute(&mut *tx)
        .await
        .expect("insert raw feed member head");
    }
    tx.commit().await.expect("commit raw authority fixture");
}

fn assert_authority_corrupt(error: InvestigationProjectionError) {
    assert_eq!(error.code(), "INVESTIGATION_AUTHORITY_CORRUPT");
    assert!(!error.restart_required());
}

#[tokio::test]
#[serial]
async fn registry_read_rejects_each_missing_authority_census_component() {
    let (db, _data_dir) = fixture("exact-census").await;
    let scenarios = [
        (
            "three-roots",
            RawAuthorityOptions {
                roots: 2,
                temporal_member: true,
                target_head: true,
                feed_contract: true,
                feed_member_heads: 5,
                unavailable_witness_members: 0,
                expired: false,
            },
        ),
        (
            "current-epoch-head",
            RawAuthorityOptions {
                roots: 3,
                temporal_member: true,
                target_head: false,
                feed_contract: true,
                feed_member_heads: 5,
                unavailable_witness_members: 0,
                expired: false,
            },
        ),
        (
            "feed-contract",
            RawAuthorityOptions {
                roots: 3,
                temporal_member: true,
                target_head: true,
                feed_contract: false,
                feed_member_heads: 5,
                unavailable_witness_members: 0,
                expired: false,
            },
        ),
        (
            "feed-member-head",
            RawAuthorityOptions {
                roots: 3,
                temporal_member: true,
                target_head: true,
                feed_contract: true,
                feed_member_heads: 4,
                unavailable_witness_members: 0,
                expired: false,
            },
        ),
    ];

    let complete_operation = seed_operation(db.pool(), "complete").await;
    let complete_snapshot_id = Uuid::new_v4();
    project_generation(db.pool(), complete_operation, complete_snapshot_id).await;
    insert_raw_authority(
        db.pool(),
        complete_operation,
        complete_snapshot_id,
        RawAuthorityOptions {
            roots: 3,
            temporal_member: true,
            target_head: true,
            feed_contract: true,
            feed_member_heads: 5,
            unavailable_witness_members: 0,
            expired: false,
        },
    )
    .await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin Registry contract switch");
    force_registry_contract(&mut tx, complete_operation.operation_id).await;
    tx.commit().await.expect("commit Registry contract switch");
    capture_investigation_read_authority(db.pool(), complete_operation.operation_id)
        .await
        .expect("complete authority census must pass");

    let missing_snapshot_operation = seed_operation(db.pool(), "missing-snapshot").await;
    let missing_snapshot_id = Uuid::new_v4();
    project_generation(db.pool(), missing_snapshot_operation, missing_snapshot_id).await;
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin Registry contract switch");
    force_registry_contract(&mut tx, missing_snapshot_operation.operation_id).await;
    tx.commit().await.expect("commit Registry contract switch");
    assert_authority_corrupt(
        capture_investigation_read_authority(db.pool(), missing_snapshot_operation.operation_id)
            .await
            .expect_err("missing Candidate snapshot must fail closed"),
    );

    for (label, options) in scenarios {
        let operation = seed_operation(db.pool(), label).await;
        let snapshot_id = Uuid::new_v4();
        project_generation(db.pool(), operation, snapshot_id).await;
        insert_raw_authority(db.pool(), operation, snapshot_id, options).await;
        let mut tx = db
            .pool()
            .begin()
            .await
            .expect("begin Registry contract switch");
        force_registry_contract(&mut tx, operation.operation_id).await;
        tx.commit().await.expect("commit Registry contract switch");
        let error =
            match capture_investigation_read_authority(db.pool(), operation.operation_id).await {
                Ok(_) => panic!("{label} omission unexpectedly passed"),
                Err(error) => error,
            };
        assert_authority_corrupt(error);
    }
}

#[tokio::test]
#[serial]
async fn registry_read_accepts_only_the_exact_sealed_unavailable_feed_authority() {
    let (db, _data_dir) = fixture("sealed-unavailable-feed").await;
    for (label, unavailable_witness_members, expected_ok) in
        [("complete", 5, true), ("missing-obligation", 4, false)]
    {
        let operation = seed_operation(db.pool(), label).await;
        let snapshot_id = Uuid::new_v4();
        project_generation(db.pool(), operation, snapshot_id).await;
        insert_raw_authority(
            db.pool(),
            operation,
            snapshot_id,
            RawAuthorityOptions {
                roots: 3,
                temporal_member: true,
                target_head: true,
                feed_contract: false,
                feed_member_heads: 0,
                unavailable_witness_members,
                expired: false,
            },
        )
        .await;
        let mut tx = db
            .pool()
            .begin()
            .await
            .expect("begin Registry contract switch");
        force_registry_contract(&mut tx, operation.operation_id).await;
        tx.commit().await.expect("commit Registry contract switch");
        let result = capture_investigation_read_authority(db.pool(), operation.operation_id).await;
        if expected_ok {
            result.expect("exact sealed unavailable feed authority must remain readable");
        } else {
            assert_authority_corrupt(
                result.expect_err("partial unavailable feed authority must fail closed"),
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn first_page_rejects_authority_that_is_already_expired() {
    let (db, _data_dir) = fixture("first-page-expired").await;
    let operation = seed_operation(db.pool(), "first-page-expired").await;
    let snapshot_id = Uuid::new_v4();
    project_generation(db.pool(), operation, snapshot_id).await;
    insert_raw_authority(
        db.pool(),
        operation,
        snapshot_id,
        RawAuthorityOptions {
            roots: 3,
            temporal_member: false,
            target_head: false,
            feed_contract: true,
            feed_member_heads: 5,
            unavailable_witness_members: 0,
            expired: true,
        },
    )
    .await;
    let error = capture_investigation_read_authority(db.pool(), operation.operation_id)
        .await
        .expect_err("already-expired first-page authority must be stale");
    assert!(matches!(error, InvestigationProjectionError::Stale { .. }));
    assert_eq!(error.code(), "INVESTIGATION_PROJECTION_STALE");
    assert!(error.restart_required());
}
