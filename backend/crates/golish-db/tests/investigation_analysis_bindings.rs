use std::sync::Arc;

use golish_db::{
    repo::{
        investigation_analysis_bindings::{
            BindInvestigationAnalysisAttemptInput, InvestigationAnalysisBindingStoreError,
            PgInvestigationAnalysisBindingRepository,
        },
        unified_investigation_runtime::{
            InvestigationStageIdentity, InvestigationUnitIdentity, InvestigationWorkKind,
            InvestigationWorkState, PgUnifiedInvestigationRuntimeRepository,
            RegisterInvestigationWorkInput, StartInvestigationRunInput,
        },
    },
    DbConfig, GolishDb,
};
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

async fn migrated_db(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("investigation_binding_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[derive(Debug, Clone)]
struct Fixture {
    identity: InvestigationUnitIdentity,
    work_id: Uuid,
    candidate_snapshot_id: Uuid,
    analysis_attempt_id: Uuid,
    foreign_organization_id: Uuid,
    foreign_stage_execution_id: Uuid,
    foreign_candidate_snapshot_id: Uuid,
    foreign_analysis_attempt_id: Uuid,
}

async fn seed_fixture(pool: &PgPool, label: &str) -> Fixture {
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!(
        "/tmp/investigation-binding-{label}-{}",
        Uuid::new_v4().simple()
    );
    let organization_id = Uuid::new_v4();
    let foreign_organization_id = Uuid::new_v4();
    let scope_stage_execution_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let authority_id = Uuid::new_v4();
    let asset_lane_id = Uuid::new_v4();
    let owning_request_id = format!("investigation-binding-request-{label}");

    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest('1'))
    .execute(pool)
    .await
    .expect("insert project scope");

    let mut deployment = pool.begin().await.expect("begin rollout selection");
    for statement in [
        "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
        "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment)
            .await
            .expect("disable isolated rollout guard");
    }
    sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='receipt_v1',row_version=row_version+1 WHERE singleton=TRUE",
    )
    .execute(&mut *deployment)
    .await
    .expect("select receipt Tool Truth");
    sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version='hypothesis_registry_v1',rollout_mode='new_only',
                  mode_rank=4,row_version=row_version+1 WHERE singleton=TRUE"#,
    )
    .execute(&mut *deployment)
    .await
    .expect("select unified Investigation");
    for statement in [
        "ALTER TABLE investigation_rollout ENABLE TRIGGER investigation_rollout_direct_mutation_guard",
        "ALTER TABLE tool_truth_rollout ENABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
    ] {
        sqlx::query(statement)
            .execute(&mut *deployment)
            .await
            .expect("restore isolated rollout guard");
    }
    deployment.commit().await.expect("commit rollout selection");

    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id,
               tool_truth_contract,investigation_contract_version,investigation_rollout_mode
           ) VALUES($1,'red_team','application_understanding','legacy_v1',$2,
                    'receipt_v1','hypothesis_registry_v1','new_only')"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert unified operation");
    for (id, name) in [
        (organization_id, "Binding Org"),
        (foreign_organization_id, "Foreign Binding Org"),
    ] {
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,$3)")
            .bind(id)
            .bind(&project_path)
            .bind(name)
            .execute(pool)
            .await
            .expect("insert organization");
    }
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'application_understanding','started')",
    )
    .bind(scope_stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert scope authority stage");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest('2'))
    .execute(pool)
    .await
    .expect("insert scope decision");
    let mut scope_tx = pool.begin().await.expect("begin scope snapshot");
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
    .execute(&mut *scope_tx)
    .await
    .expect("insert scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,parent_organization_id,
               organization_name_at_freeze,role,depth,ordinal,
               decision_row_id,approval_source
           ) VALUES($1,$2,NULL,'Binding Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source":"investigation_analysis_binding_fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal scope snapshot");
    scope_tx.commit().await.expect("commit scope snapshot");

    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'investigation','started')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert Investigation execution");
    sqlx::query("UPDATE operation_state SET current_stage='investigation' WHERE operation_id=$1")
        .bind(operation_id)
        .execute(pool)
        .await
        .expect("enter Investigation");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'investigation',0,'running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert Investigation unit");
    sqlx::query(
        r#"INSERT INTO investigation_stage_run_authorities(
               authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&owning_request_id)
    .bind(scope_snapshot_id)
    .execute(pool)
    .await
    .expect("insert Investigation authority");
    let mut asset_tx = pool
        .begin()
        .await
        .expect("begin analysis binding asset fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *asset_tx)
        .await
        .expect("isolate analysis binding asset fixture");
    sqlx::query(
        r#"INSERT INTO investigation_asset_lanes(
               asset_lane_id,asset_queue_id,company_queue_id,company_member_id,authority_id,
               operation_id,stage_execution_id,scope_snapshot_id,organization_id,target_id,
               target_type_at_freeze,target_value_at_freeze,target_source_at_freeze,
               target_created_at,target_identity_sha256,ordinal,state,evolution_epoch,
               max_evolution_epochs)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'domain','binding.example','fixture',
                  NOW(),$11,0,'analyzing',0,2)"#,
    )
    .bind(asset_lane_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(Uuid::new_v4())
    .bind(digest('5'))
    .execute(&mut *asset_tx)
    .await
    .expect("insert analysis binding asset lane");
    asset_tx
        .commit()
        .await
        .expect("commit analysis binding asset fixture");

    let identity = InvestigationUnitIdentity {
        stage: InvestigationStageIdentity {
            authority_id,
            operation_id,
            stage_execution_id,
            owning_stage_run_request_id: owning_request_id,
            scope_snapshot_id,
        },
        stage_run_unit_id,
        organization_id,
    };
    let runtime = PgUnifiedInvestigationRuntimeRepository::new(Arc::new(pool.clone()));
    runtime
        .start_run(&StartInvestigationRunInput {
            identity: identity.stage.clone(),
            stable_start_request_id: Uuid::new_v4(),
            initial_change_seq: 0,
        })
        .await
        .expect("start unified Investigation run");
    let work_id = Uuid::new_v4();
    runtime
        .register_work(&RegisterInvestigationWorkInput {
            identity: identity.clone(),
            work_id,
            asset_lane_id,
            stable_work_key_sha256: digest('4'),
            work_kind: InvestigationWorkKind::Analysis,
            external_identity_sha256: digest('5'),
            initial_state: InvestigationWorkState::Running,
            observed_stop_epoch: 0,
        })
        .await
        .expect("register analysis work");

    let candidate_snapshot_id = Uuid::new_v4();
    let analysis_attempt_id = Uuid::new_v4();
    let foreign_candidate_snapshot_id = Uuid::new_v4();
    let foreign_analysis_attempt_id = Uuid::new_v4();
    let mut candidate_tx = pool.begin().await.expect("begin Candidate fixture");
    sqlx::query("SET LOCAL session_replication_role='replica'")
        .execute(&mut *candidate_tx)
        .await
        .expect("isolate Candidate fixture authority");
    for (snapshot_id, org_id, attempt_id, nibble) in [
        (
            candidate_snapshot_id,
            organization_id,
            analysis_attempt_id,
            '6',
        ),
        (
            foreign_candidate_snapshot_id,
            foreign_organization_id,
            foreign_analysis_attempt_id,
            '7',
        ),
    ] {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_snapshots(
                   snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
                   genesis,source_set_hash,capability_revision_hash,policy_revision_hash,
                   credential_revision_hash,snapshot_status,tool_truth_authority_bundle_seal_id,
                   stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
                   bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
                   freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                   temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                   observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash,
                   asset_lane_id
               ) VALUES($1,$2,$3,0,$4,TRUE,$5,$6,$7,$8,'sealed_ready',$9,$10,4,$11,4,$12,
                        $13,$14,$15,$16,$17,$18,NOW(),$19,$20)"#,
        )
        .bind(snapshot_id)
        .bind(operation_id)
        .bind(org_id)
        .bind(scope_snapshot_id)
        .bind(digest(nibble))
        .bind(digest('8'))
        .bind(digest('9'))
        .bind(digest('a'))
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(digest('b'))
        .bind(digest('c'))
        .bind(digest('d'))
        .bind(digest('e'))
        .bind(digest('f'))
        .bind(digest('0'))
        .bind(digest('1'))
        .bind(digest('2'))
        .bind(digest('3'))
        .bind(asset_lane_id)
        .execute(&mut *candidate_tx)
        .await
        .expect("insert Candidate snapshot fixture");
        sqlx::query(
            r#"INSERT INTO candidate_analysis_attempts(
                   analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
                   attempt_input_hash,attack_class_checklist_version,
                   attack_class_checklist_digest,trust_boundary_checklist_version,
                   trust_boundary_checklist_digest,coverage_sampling_contract_version,
                   coverage_sampling_contract_digest,retry_limit,asset_lane_id
               ) VALUES($1,$2,$3,$4,0,$5,'1',$6,'1',$7,'1',$8,1,$9)"#,
        )
        .bind(attempt_id)
        .bind(snapshot_id)
        .bind(operation_id)
        .bind(org_id)
        .bind(digest('4'))
        .bind(digest('5'))
        .bind(digest('6'))
        .bind(digest('7'))
        .bind(asset_lane_id)
        .execute(&mut *candidate_tx)
        .await
        .expect("insert Candidate attempt fixture");
    }
    candidate_tx
        .commit()
        .await
        .expect("commit Candidate fixture");

    Fixture {
        identity,
        work_id,
        candidate_snapshot_id,
        analysis_attempt_id,
        foreign_organization_id,
        foreign_stage_execution_id: scope_stage_execution_id,
        foreign_candidate_snapshot_id,
        foreign_analysis_attempt_id,
    }
}

#[tokio::test]
#[serial]
async fn binding_is_exact_replayable_and_rejects_foreign_authority() {
    let (db, _data_dir) = migrated_db("exact-authority").await;
    let fixture = seed_fixture(db.pool(), "exact-authority").await;
    let repository = PgInvestigationAnalysisBindingRepository::new(Arc::new(db.pool().clone()));
    let request = BindInvestigationAnalysisAttemptInput {
        binding_id: Uuid::new_v4(),
        stable_request_id: Uuid::new_v4(),
        identity: fixture.identity.clone(),
        work_id: fixture.work_id,
        candidate_snapshot_id: fixture.candidate_snapshot_id,
        analysis_attempt_id: fixture.analysis_attempt_id,
    };

    let first = repository
        .bind(&request)
        .await
        .expect("bind exact authority");
    assert!(!first.replayed);
    let replay = repository.bind(&request).await.expect("replay exact bind");
    assert!(replay.replayed);
    assert_eq!(first.binding, replay.binding);
    assert_eq!(
        repository
            .load(&fixture.identity, fixture.work_id)
            .await
            .expect("load exact binding"),
        Some(first.binding.clone())
    );

    let mut replay_mismatch = request.clone();
    replay_mismatch.analysis_attempt_id = Uuid::new_v4();
    assert!(matches!(
        repository.bind(&replay_mismatch).await,
        Err(InvestigationAnalysisBindingStoreError::IdentityConflict(
            "stable_request_replay_mismatch"
        ))
    ));

    let mut foreign_org = request.clone();
    foreign_org.binding_id = Uuid::new_v4();
    foreign_org.stable_request_id = Uuid::new_v4();
    foreign_org.identity.organization_id = fixture.foreign_organization_id;
    assert!(repository.bind(&foreign_org).await.is_err());

    let mut foreign_stage = request.clone();
    foreign_stage.binding_id = Uuid::new_v4();
    foreign_stage.stable_request_id = Uuid::new_v4();
    foreign_stage.identity.stage.stage_execution_id = fixture.foreign_stage_execution_id;
    assert!(repository.bind(&foreign_stage).await.is_err());

    let mut foreign_snapshot = request.clone();
    foreign_snapshot.binding_id = Uuid::new_v4();
    foreign_snapshot.stable_request_id = Uuid::new_v4();
    foreign_snapshot.candidate_snapshot_id = fixture.foreign_candidate_snapshot_id;
    foreign_snapshot.analysis_attempt_id = fixture.foreign_analysis_attempt_id;
    assert!(repository.bind(&foreign_snapshot).await.is_err());

    let update_error = sqlx::query(
        "UPDATE investigation_analysis_attempt_bindings SET created_at=created_at WHERE binding_id=$1",
    )
    .bind(request.binding_id)
    .execute(db.pool())
    .await
    .expect_err("binding rows are append-only");
    assert!(update_error
        .to_string()
        .contains("UNIFIED_INVESTIGATION_APPEND_ONLY"));
}
