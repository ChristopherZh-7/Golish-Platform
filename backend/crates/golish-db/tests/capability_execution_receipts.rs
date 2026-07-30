use golish_db::{
    repo::{capability_execution_receipts, stage_asset_waves},
    DbConfig, GolishDb,
};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use serial_test::serial;
use sqlx::{Error as SqlxError, PgPool};
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn digest_v1(nibble: char) -> String {
    assert!(nibble.is_ascii_hexdigit() && !nibble.is_ascii_uppercase());
    format!("sha256:{}", nibble.to_string().repeat(64))
}

#[test]
fn task3_receipt_repo_surface_exists() {
    assert_eq!(
        capability_execution_receipts::TABLE_NAME,
        "capability_execution_receipts"
    );
}

fn assert_database_rejection(error: &SqlxError, sqlstate: &str, stable_marker: &str) {
    let database_error = error
        .as_database_error()
        .unwrap_or_else(|| panic!("expected PostgreSQL database error, got {error}"));
    assert_eq!(
        database_error.code().as_deref(),
        Some(sqlstate),
        "unexpected SQLSTATE for {stable_marker}: {error}"
    );
    assert!(
        database_error.message().contains(stable_marker)
            || database_error.constraint() == Some(stable_marker),
        "expected stable marker {stable_marker}, got message={} constraint={:?}",
        database_error.message(),
        database_error.constraint()
    );
}

async fn fixture(label: &str) -> (GolishDb, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("tool_truth_{label}_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

#[derive(Debug, Clone)]
struct FrozenExecution {
    session_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    project_path: String,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    outside_organization_id: Uuid,
    stage_execution_id: Uuid,
    other_stage_execution_id: Uuid,
    stage_kind: &'static str,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    worker_attempt_epoch: i64,
    lease_token: Uuid,
    source_tool_call_id: Uuid,
}

#[derive(Debug)]
struct WaveDenominatorFixture {
    frozen: FrozenExecution,
    wave_id: Uuid,
}

async fn seed_frozen_execution(pool: &PgPool, label: &str) -> FrozenExecution {
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let project_path = format!("/tmp/tool-truth-{label}-{}", Uuid::new_v4().simple());
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let outside_organization_id = Uuid::new_v4();
    let stage_execution_id = Uuid::new_v4();
    let other_stage_execution_id = Uuid::new_v4();
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let worker_attempt_epoch = 0_i64;
    let lease_token = Uuid::new_v4();
    let source_tool_call_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO sessions(id,title,status,project_path) \
         VALUES($1,'tool truth fixture','running',$2)",
    )
    .bind(session_id)
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert isolated fixture session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,title,input,status) \
         VALUES($1,$2,'tool truth operation','fixture','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert isolated fixture task");
    sqlx::query(
        "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) \
         VALUES($1,$2,$3)",
    )
    .bind(project_scope_id)
    .bind(&project_path)
    .bind(digest_v1('1'))
    .execute(pool)
    .await
    .expect("insert frozen project scope");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,project_scope_id
           ) VALUES($1,'assessment','enumeration','legacy_v1',$2)"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert operation with deployment-owned Tool Truth default");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name) VALUES \
             ($1,$3,'Tool Truth Scoped Org'), \
             ($2,$3,'Tool Truth Outside Org')",
    )
    .bind(organization_id)
    .bind(outside_organization_id)
    .bind(&project_path)
    .execute(pool)
    .await
    .expect("insert scoped and outside organizations");
    sqlx::query(
        r#"INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES
               ($1,$3,'enumeration','started'),
               ($2,$3,'vuln_triage','started')"#,
    )
    .bind(stage_execution_id)
    .bind(other_stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert fixture stage executions");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(stage_execution_id)
    .bind(organization_id)
    .bind(serde_json::json!([{"organization_id": organization_id}]))
    .bind(digest_v1('2'))
    .execute(pool)
    .await
    .expect("insert fixture scope decision");

    let mut scope_tx = pool.begin().await.expect("begin frozen scope transaction");
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
    .bind(digest_v1('3'))
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Tool Truth Scoped Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "fixture"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert frozen scope member");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal fixture scope snapshot");
    scope_tx
        .commit()
        .await
        .expect("commit frozen scope transaction");

    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,started_at
           ) VALUES($1,$2,$3,$4,$5,'enumeration',0,'tool_truth_fixture','running',NOW())"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert stage Unit bound to frozen scope");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES(
               $1,$2,$3,$4,$5,0,'tool_truth_fixture','stage_unit','fixture',
               'main>enumeration','running',$6,'tool-truth-fixture',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),$7
           )"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(lease_token)
    .bind(worker_attempt_epoch)
    .execute(pool)
    .await
    .expect("insert live worker fence");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,'tool-truth-source',$2,$3,'primary','tool_truth_fixture','{}','running',
               $3,$4,$5,$6,$7,$8,$9
           )"#,
    )
    .bind(source_tool_call_id)
    .bind(session_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(worker_attempt_epoch)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert immutable source tool-call fence");

    FrozenExecution {
        session_id,
        operation_id,
        project_scope_id,
        project_path,
        scope_snapshot_id,
        organization_id,
        outside_organization_id,
        stage_execution_id,
        other_stage_execution_id,
        stage_kind: "enumeration",
        stage_run_unit_id,
        worker_run_id,
        worker_attempt_epoch,
        lease_token,
        source_tool_call_id,
    }
}

async fn insert_host_authority(pool: &PgPool, fixture: &FrozenExecution) -> (Uuid, String) {
    let id = Uuid::new_v4();
    sqlx::query_as::<_, (Uuid, String)>(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )
           RETURNING id,authority_hash"#,
    )
    .bind(id)
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(fixture.project_scope_id)
    .bind(&fixture.project_path)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(fixture.stage_execution_id)
    .bind(fixture.stage_kind)
    .bind(digest_v1('4'))
    .fetch_one(pool)
    .await
    .expect("insert server-validated host-stage execution authority")
}

async fn seed_wave_denominator_fixture(
    pool: &PgPool,
    label: &str,
    assets: &[&str],
) -> WaveDenominatorFixture {
    let frozen = seed_frozen_execution(pool, label).await;
    sqlx::query(
        "ALTER TABLE operation_state DISABLE TRIGGER operation_state_tool_truth_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("disable immutable contract trigger only inside isolated fixture");
    sqlx::query("UPDATE operation_state SET tool_truth_contract='shadow_v1' WHERE operation_id=$1")
        .bind(frozen.operation_id)
        .execute(pool)
        .await
        .expect("seed future shadow operation contract in isolated fixture");
    sqlx::query(
        "ALTER TABLE operation_state ENABLE TRIGGER operation_state_tool_truth_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("restore immutable contract trigger");

    for asset in assets {
        sqlx::query(
            r#"INSERT INTO targets(
                   id,name,target_type,value,scope,project_path,organization_id,source
               ) VALUES($1,$2,'domain',$2,'in',$3,$4,'tool_truth_fixture')"#,
        )
        .bind(Uuid::new_v4())
        .bind(*asset)
        .bind(&frozen.project_path)
        .bind(frozen.organization_id)
        .execute(pool)
        .await
        .expect("insert exact wave target");
    }
    let wave = stage_asset_waves::current_or_create_initial(
        pool,
        frozen.operation_id,
        frozen.organization_id,
        frozen.stage_kind,
        chrono::Utc::now() + chrono::Duration::seconds(1),
        100,
    )
    .await
    .expect("create server-owned stage wave")
    .expect("fixture assets produce a wave");
    WaveDenominatorFixture {
        frozen,
        wave_id: wave.wave.id,
    }
}

fn seal_wave_command(
    fixture: &WaveDenominatorFixture,
    stable_seal_request_id: Uuid,
) -> capability_execution_receipts::SealWaveDenominator {
    capability_execution_receipts::SealWaveDenominator {
        stable_seal_request_id,
        stage_execution_id: fixture.frozen.stage_execution_id,
        scope_snapshot_id: fixture.frozen.scope_snapshot_id,
        stage_asset_wave_id: fixture.wave_id,
        technique: "enumerate_dns".to_string(),
        expected_capability: "dns_enumeration".to_string(),
        contract: ToolTruthContract::ShadowV1,
    }
}

fn compile_test_denominator(
    _stage: &str,
    assets: &[capability_execution_receipts::LockedDenominatorAsset],
) -> anyhow::Result<Vec<capability_execution_receipts::CompiledDenominatorItem>> {
    let mut items = Vec::new();
    for asset in assets {
        for (technique, capability) in [
            ("GOLISH-ENUM-DIR", "enum.directory"),
            ("GOLISH-ENUM-JS", "enum.javascript"),
        ] {
            items.push(capability_execution_receipts::CompiledDenominatorItem {
                input_key: format!(
                    "{}\u{1f}{}\u{1f}{technique}",
                    asset.target_id, asset.exact_asset
                ),
                target_id: asset.target_id,
                exact_asset: asset.exact_asset.clone(),
                technique: technique.to_string(),
                expected_capability: capability.to_string(),
            });
        }
    }
    items.sort_by(|left, right| left.input_key.cmp(&right.input_key));
    Ok(items)
}

fn compile_target_intel_denominator(
    _stage: &str,
    assets: &[capability_execution_receipts::LockedDenominatorAsset],
) -> anyhow::Result<Vec<capability_execution_receipts::CompiledDenominatorItem>> {
    let mut items = Vec::new();
    for asset in assets {
        for (technique, capability) in [
            ("GOLISH-INTEL-DNS", "intel.dns"),
            ("GOLISH-INTEL-WHOIS", "intel.whois"),
        ] {
            items.push(capability_execution_receipts::CompiledDenominatorItem {
                input_key: format!(
                    "{}\u{1f}{}\u{1f}{technique}",
                    asset.target_id, asset.exact_asset
                ),
                target_id: asset.target_id,
                exact_asset: asset.exact_asset.clone(),
                technique: technique.to_string(),
                expected_capability: capability.to_string(),
            });
        }
    }
    items.sort_by(|left, right| left.input_key.cmp(&right.input_key));
    Ok(items)
}

async fn seed_target_intel_denominator(
    pool: &PgPool,
    label: &str,
) -> (
    FrozenExecution,
    capability_execution_receipts::CoverageDenominatorRow,
) {
    seed_target_intel_denominator_assets(pool, label, &["receipt.example.test"]).await
}

async fn seed_target_intel_denominator_assets(
    pool: &PgPool,
    label: &str,
    assets: &[&str],
) -> (
    FrozenExecution,
    capability_execution_receipts::CoverageDenominatorRow,
) {
    let mut frozen = seed_frozen_execution(pool, label).await;
    frozen.stage_execution_id = Uuid::new_v4();
    frozen.stage_kind = "target_intel";
    sqlx::query(
        "ALTER TABLE operation_state DISABLE TRIGGER operation_state_tool_truth_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("disable contract immutability inside isolated target_intel fixture");
    sqlx::query(
        "UPDATE operation_state SET tool_truth_contract='receipt_v1' WHERE operation_id=$1",
    )
    .bind(frozen.operation_id)
    .execute(pool)
    .await
    .expect("freeze target_intel fixture to receipt_v1");
    sqlx::query(
        "ALTER TABLE operation_state ENABLE TRIGGER operation_state_tool_truth_contract_immutable",
    )
    .execute(pool)
    .await
    .expect("restore target_intel contract immutability trigger");
    sqlx::query("UPDATE operation_state SET current_stage='target_intel' WHERE operation_id=$1")
        .bind(frozen.operation_id)
        .execute(pool)
        .await
        .expect("move isolated fixture operation to target_intel");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'target_intel','started')",
    )
    .bind(frozen.stage_execution_id)
    .bind(frozen.operation_id)
    .execute(pool)
    .await
    .expect("insert target_intel execution");
    for asset in assets {
        sqlx::query(
            r#"INSERT INTO targets(
                   id,name,target_type,value,scope,project_path,organization_id,source
               ) VALUES($1,$2,'domain',$2,'in',$3,$4,'tool_truth_fixture')"#,
        )
        .bind(Uuid::new_v4())
        .bind(asset)
        .bind(&frozen.project_path)
        .bind(frozen.organization_id)
        .execute(pool)
        .await
        .expect("insert target_intel denominator target");
    }
    let wave = stage_asset_waves::current_or_create_initial(
        pool,
        frozen.operation_id,
        frozen.organization_id,
        frozen.stage_kind,
        chrono::Utc::now() + chrono::Duration::seconds(1),
        100,
    )
    .await
    .expect("create target_intel wave")
    .expect("target_intel wave has targets");
    let denominator = capability_execution_receipts::seal_source_denominator(
        pool,
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            stage_execution_id: frozen.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                wave.wave.id,
            ),
        },
        compile_target_intel_denominator,
    )
    .await
    .expect("seal target_intel denominator");
    (frozen, denominator)
}

async fn begin_managed_target_intel_receipt(
    pool: &PgPool,
    denominator_id: Uuid,
    capability: &str,
    attempt_ordinal: i32,
) -> capability_execution_receipts::CapabilityExecutionReceiptRow {
    let policy = capability_execution_receipts::seal_fixed_provider_destination_policy(
        pool,
        &capability_execution_receipts::SealFixedProviderDestinationPolicy {
            denominator_id,
            capability: capability.to_string(),
            endpoints: vec![capability_execution_receipts::FixedProviderEndpoint {
                scheme: "https".to_string(),
                normalized_host: "fixed.provider.example.test".to_string(),
                port: 443,
                path_prefix: "/v1/query".to_string(),
            }],
        },
    )
    .await
    .expect("seal fixed provider destination policy before send");
    capability_execution_receipts::begin_managed(
        pool,
        &capability_execution_receipts::BeginManagedCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id,
            capability: capability.to_string(),
            attempt_ordinal,
            destination_policy_id: policy.id,
        },
    )
    .await
    .expect("begin receipt with exact sealed destination policy")
}

async fn target_intel_finalization(
    pool: &PgPool,
    receipt: &capability_execution_receipts::CapabilityExecutionReceiptRow,
    observations: &[(&str, &str)],
) -> capability_execution_receipts::FinalizeTargetIntelReceipt {
    let input_keys = sqlx::query_as::<_, (String, String)>(
        r#"SELECT input_key,technique FROM coverage_denominator_items
            WHERE denominator_id=$1 AND expected_capability=$2"#,
    )
    .bind(receipt.denominator_id)
    .bind(&receipt.capability)
    .fetch_all(pool)
    .await
    .expect("load exact TargetIntel receipt input keys");
    capability_execution_receipts::FinalizeTargetIntelReceipt {
        receipt_id: receipt.id,
        expected_row_version: receipt.row_version,
        attempt_fence: None,
        raw_witness: capability_execution_receipts::RawWitnessArtifactInput {
            artifact_id: Uuid::new_v5(&receipt.id, b"target-intel-test-witness"),
            content_key: digest_v1('1'),
            vault_object_ref_token: vec![7_u8; 32],
            vault_object_ref_token_hash: digest_v1('2'),
            sha256: digest_v1('1'),
            ciphertext_sha256: digest_v1('3'),
            operation_key_ref_hash: digest_v1('4'),
            key_generation: 1,
            retention_policy_id: Uuid::new_v5(&receipt.id, b"target-intel-test-retention"),
            retention_policy_hash: digest_v1('5'),
            sensitivity_disposition: "typed_derivative_ready".to_string(),
            original_byte_count: 2,
            stored_byte_count: 2,
            truncated: false,
        },
        network_hops: vec![capability_execution_receipts::ObservedNetworkHopInput {
            hop_kind: "initial".to_string(),
            scheme: "https".to_string(),
            normalized_host: "fixed.provider.example.test".to_string(),
            port: 443,
            path_and_query: "/v1/query?input=receipt.example.test".to_string(),
            addresses: vec!["1.1.1.1".parse().expect("public fixture IP")],
            selected_address: "1.1.1.1".parse().expect("public fixture IP"),
            send_ordinal: 1,
        }],
        request_count: 1,
        response_byte_count: 2,
        wall_clock_ms: 4,
        retry_count: 0,
        parser_complete: true,
        normalized_record_count: 1,
        input_observations: observations
            .iter()
            .flat_map(|(technique, observation_state)| {
                input_keys
                    .iter()
                    .filter(move |(_, candidate)| candidate == technique)
                    .map(move |(input_key, _)| {
                        capability_execution_receipts::TargetIntelInputObservation {
                            input_key: input_key.clone(),
                            technique: (*technique).to_string(),
                            observation_state: (*observation_state).to_string(),
                        }
                    })
            })
            .collect(),
        typed_landing: serde_json::json!({
            "kind": "target_intel_test",
            "version": 1,
        }),
        failure_reason_code: None,
    }
}

#[tokio::test]
#[serial]
async fn target_intel_managed_begin_requires_exact_sealed_destination_policy() {
    let (mut db, _data_dir) = fixture("target_intel_managed_policy").await;
    let (_frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-managed-policy").await;
    let error = capability_execution_receipts::begin_managed(
        db.pool(),
        &capability_execution_receipts::BeginManagedCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            attempt_ordinal: 1,
            destination_policy_id: Uuid::new_v4(),
        },
    )
    .await
    .expect_err("managed provider I/O cannot begin without an exact sealed policy");
    assert!(error.to_string().contains("TOOL_TRUTH_AUTHORITY_STALE"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_managed_claim_is_single_sender_and_policy_exact() {
    let (mut db, _data_dir) = fixture("target_intel_managed_claim").await;
    let (_frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-managed-claim").await;
    let first_policy = capability_execution_receipts::seal_fixed_provider_destination_policy(
        db.pool(),
        &capability_execution_receipts::SealFixedProviderDestinationPolicy {
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            endpoints: vec![capability_execution_receipts::FixedProviderEndpoint {
                scheme: "https".to_string(),
                normalized_host: "fixed.provider.example.test".to_string(),
                port: 443,
                path_prefix: "/v1/query".to_string(),
            }],
        },
    )
    .await
    .expect("seal first managed policy");
    let command = capability_execution_receipts::BeginManagedCapabilityReceipt {
        id: Uuid::new_v4(),
        denominator_id: denominator.id,
        capability: "intel.dns".to_string(),
        attempt_ordinal: 1,
        destination_policy_id: first_policy.id,
    };
    let first = capability_execution_receipts::begin_managed_claim(db.pool(), &command)
        .await
        .expect("first claimant owns the provider send");
    assert!(matches!(
        first,
        capability_execution_receipts::ManagedReceiptBeginOutcome::Created(_)
    ));
    let replay = capability_execution_receipts::begin_managed_claim(db.pool(), &command)
        .await
        .expect("same execution key returns an in-flight claim");
    assert!(matches!(
        replay,
        capability_execution_receipts::ManagedReceiptBeginOutcome::InFlight(_)
    ));

    let drifted_policy = capability_execution_receipts::seal_fixed_provider_destination_policy(
        db.pool(),
        &capability_execution_receipts::SealFixedProviderDestinationPolicy {
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            endpoints: vec![capability_execution_receipts::FixedProviderEndpoint {
                scheme: "https".to_string(),
                normalized_host: "fixed.provider.example.test".to_string(),
                port: 443,
                path_prefix: "/v2/query".to_string(),
            }],
        },
    )
    .await
    .expect("seal a distinct policy for drift test");
    let error = capability_execution_receipts::begin_managed(
        db.pool(),
        &capability_execution_receipts::BeginManagedCapabilityReceipt {
            destination_policy_id: drifted_policy.id,
            ..command
        },
    )
    .await
    .expect_err("an existing claim cannot be replayed under a different policy");
    assert!(error.to_string().contains("TOOL_TRUTH_MANIFEST_DRIFT"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_current_context_derives_host_attempt_without_stage_run_column() {
    let (mut db, _data_dir) = fixture("target_intel_current_context").await;
    let (frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-current-context").await;
    let context = capability_execution_receipts::current_target_intel_receipt_context(
        db.pool(),
        frozen.operation_id,
        frozen.organization_id,
        frozen.stage_execution_id,
        "intel.dns",
        None,
    )
    .await
    .expect("load current host-owned TargetIntel context")
    .expect("sealed denominator is current");
    assert_eq!(context.denominator_id, denominator.id);
    assert_eq!(context.attempt_epoch, 1);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_full_lifecycle_finalizes_exact_current_receipts_and_replays() {
    let (mut db, _data_dir) = fixture("target_intel_full_lifecycle").await;
    let (frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-full-lifecycle").await;
    let dns = begin_managed_target_intel_receipt(db.pool(), denominator.id, "intel.dns", 1).await;
    let dns_close =
        target_intel_finalization(db.pool(), &dns, &[("GOLISH-INTEL-DNS", "found")]).await;
    let dns_final =
        capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &dns_close)
            .await
            .expect("atomically finalize DNS receipt lifecycle");
    assert_eq!(dns_final.reconciliation_state, "consistent");
    assert_eq!(dns_final.coverage_extent, "complete");
    let replay =
        capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &dns_close)
            .await
            .expect("response-loss replay returns the exact finalized receipt");
    assert_eq!(replay.id, dns_final.id);
    assert_eq!(replay.row_version, dns_final.row_version);
    let mut drifted_replay = dns_close.clone();
    drifted_replay.wall_clock_ms += 1;
    let error =
        capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &drifted_replay)
            .await
            .expect_err("terminal replay must match the complete canonical request");
    assert!(error.to_string().contains("TOOL_TRUTH_MANIFEST_DRIFT"));

    let whois =
        begin_managed_target_intel_receipt(db.pool(), denominator.id, "intel.whois", 1).await;
    let whois_close =
        target_intel_finalization(db.pool(), &whois, &[("GOLISH-INTEL-WHOIS", "no_match")]).await;
    capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &whois_close)
        .await
        .expect("atomically finalize WHOIS receipt lifecycle");

    let projection = capability_execution_receipts::current_target_intel_projection(
        db.pool(),
        frozen.operation_id,
        frozen.organization_id,
    )
    .await
    .expect("read current receipt projection")
    .expect("current denominator exists");
    assert_eq!(projection.attempt_epoch, 1);
    assert_eq!(projection.receipts.len(), 2);
    assert!(projection
        .receipts
        .iter()
        .all(|receipt| receipt.authority_current));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_missing_current_technique_stays_orphaned_and_partial() {
    let (mut db, _data_dir) = fixture("target_intel_missing_technique").await;
    let (_frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-missing-technique").await;
    let receipt =
        begin_managed_target_intel_receipt(db.pool(), denominator.id, "intel.dns", 1).await;
    let close = target_intel_finalization(db.pool(), &receipt, &[]).await;
    let final_receipt =
        capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &close)
            .await
            .expect("incomplete observations are durably finalized, not promoted");
    assert_eq!(final_receipt.reconciliation_state, "orphaned");
    assert_eq!(final_receipt.coverage_extent, "partial");
    assert_eq!(final_receipt.observation_state, "indeterminate");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_one_found_input_cannot_close_sibling_same_technique() {
    let (mut db, _data_dir) = fixture("target_intel_exact_input_observation").await;
    let (_frozen, denominator) = seed_target_intel_denominator_assets(
        db.pool(),
        "target-intel-exact-input-observation",
        &["a.example.test", "b.example.test"],
    )
    .await;
    let receipt =
        begin_managed_target_intel_receipt(db.pool(), denominator.id, "intel.dns", 1).await;
    let mut close =
        target_intel_finalization(db.pool(), &receipt, &[("GOLISH-INTEL-DNS", "found")]).await;
    close.input_observations.truncate(1);
    let finalized = capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &close)
        .await
        .expect("seal partial exact-input receipt");
    assert_eq!(finalized.coverage_extent, "partial");
    assert_eq!(finalized.reconciliation_state, "orphaned");
    let inputs = sqlx::query_as::<_, (String, String, String)>(
        r#"SELECT observation_state,coverage_extent,input_key
             FROM capability_execution_receipt_inputs
            WHERE receipt_id=$1 ORDER BY input_key"#,
    )
    .bind(finalized.id)
    .fetch_all(db.pool())
    .await
    .expect("read exact sibling input closeouts");
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs
            .iter()
            .filter(|(observation, coverage, _)| observation == "found" && coverage == "complete")
            .count(),
        1
    );
    assert_eq!(
        inputs
            .iter()
            .filter(|(observation, coverage, _)| {
                observation == "indeterminate" && coverage == "partial"
            })
            .count(),
        1
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_rejects_ipv4_mapped_private_provider_address() {
    let (mut db, _data_dir) = fixture("target_intel_ipv4_mapped_private").await;
    let (_frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-ipv4-mapped-private").await;
    let receipt =
        begin_managed_target_intel_receipt(db.pool(), denominator.id, "intel.dns", 1).await;
    let mut close =
        target_intel_finalization(db.pool(), &receipt, &[("GOLISH-INTEL-DNS", "found")]).await;
    let mapped = "::ffff:127.0.0.1"
        .parse()
        .expect("IPv4-mapped loopback fixture");
    close.network_hops[0].addresses = vec![mapped];
    close.network_hops[0].selected_address = mapped;
    let error = capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &close)
        .await
        .expect_err("IPv4-mapped private destinations are never public provider hops");
    assert!(error.to_string().contains("TOOL_TRUTH_CONTRACT_INVALID"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_late_prior_managed_closeout_cannot_restore_superseded_authority() {
    let (mut db, _data_dir) = fixture("target_intel_late_managed_closeout").await;
    let (_frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-late-managed-closeout").await;
    let prior = begin_managed_target_intel_receipt(db.pool(), denominator.id, "intel.dns", 1).await;
    let late_close =
        target_intel_finalization(db.pool(), &prior, &[("GOLISH-INTEL-DNS", "found")]).await;
    begin_managed_target_intel_receipt(db.pool(), denominator.id, "intel.dns", 2).await;

    let late = capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &late_close)
        .await
        .expect("late closeout is retained as superseded instead of becoming current");
    assert_eq!(late.attempt_state, "superseded");
    assert_eq!(late.reconciliation_state, "superseded");
    assert_ne!(late.coverage_extent, "complete");
    assert!(late.finalization_request_hash.is_some());
    let raw_closeout_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM capability_raw_witness_artifacts WHERE receipt_id=$1",
    )
    .bind(late.id)
    .fetch_one(db.pool())
    .await
    .expect("read superseded raw closeout evidence");
    assert_eq!(raw_closeout_count, 1);
    db.stop().await;
}

async fn seed_revalidation_source(
    pool: &PgPool,
    label: &str,
) -> (
    FrozenExecution,
    capability_execution_receipts::CapabilityExecutionReceiptRow,
    Uuid,
    String,
    Uuid,
) {
    let (frozen, denominator) = seed_target_intel_denominator(pool, label).await;
    let receipt = begin_managed_target_intel_receipt(pool, denominator.id, "intel.dns", 1).await;
    let close = target_intel_finalization(pool, &receipt, &[("GOLISH-INTEL-DNS", "found")]).await;
    let receipt = capability_execution_receipts::finalize_target_intel_receipt(pool, &close)
        .await
        .expect("finalize revalidation source receipt");
    let (input_id, input_key): (Uuid, String) = sqlx::query_as(
        "SELECT id,input_key FROM capability_execution_receipt_inputs WHERE receipt_id=$1",
    )
    .bind(receipt.id)
    .fetch_one(pool)
    .await
    .expect("read exact revalidation source input");
    let temporal_policy_id: Uuid = sqlx::query_scalar(
        "SELECT temporal_validity_policy_id FROM capability_execution_receipts WHERE id=$1",
    )
    .bind(receipt.id)
    .fetch_one(pool)
    .await
    .expect("read frozen temporal policy");
    (frozen, receipt, input_id, input_key, temporal_policy_id)
}

async fn release_revalidation_dispatch_for_test(pool: &PgPool, operation_id: Uuid) {
    sqlx::query(
        "ALTER TABLE tool_truth_revalidation_dispatch_policies DISABLE TRIGGER tool_truth_revalidation_policy_immutable",
    )
    .execute(pool)
    .await
    .expect("enable isolated auto-policy fixture");
    sqlx::query(
        "UPDATE tool_truth_revalidation_dispatch_policies SET dispatch_mode='auto_passive_t0_t1' WHERE operation_id=$1",
    )
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("select explicit auto passive fixture policy");
    sqlx::query(
        "ALTER TABLE tool_truth_revalidation_dispatch_policies ENABLE TRIGGER tool_truth_revalidation_policy_immutable",
    )
    .execute(pool)
    .await
    .expect("restore policy immutability");
    sqlx::query(
        "ALTER TABLE tool_truth_revalidation_dispatch_heads DISABLE TRIGGER tool_truth_revalidation_dispatch_head_immutable",
    )
    .execute(pool)
    .await
    .expect("enable isolated released-head fixture");
    sqlx::query(
        "UPDATE tool_truth_revalidation_dispatch_heads SET dispatch_state='released',generation=1,row_version=1 WHERE operation_id=$1",
    )
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("release isolated dispatch head");
    sqlx::query(
        "ALTER TABLE tool_truth_revalidation_dispatch_heads ENABLE TRIGGER tool_truth_revalidation_dispatch_head_immutable",
    )
    .execute(pool)
    .await
    .expect("restore dispatch-head immutability");
}

#[tokio::test]
#[serial]
async fn tool_truth_revalidation_deduplicates_consumers_and_default_head_holds_dispatch() {
    let (mut db, _data_dir) = fixture("tool_truth_revalidation_dedupe").await;
    let (frozen, receipt, input_id, input_key, temporal_policy_id) =
        seed_revalidation_source(db.pool(), "tool-truth-revalidation-dedupe").await;
    let base = golish_db::repo::tool_truth_revalidation::RecordRevalidationObligation {
        operation_id: frozen.operation_id,
        organization_id: frozen.organization_id,
        source_receipt_id: receipt.id,
        source_receipt_input_id: input_id,
        source_input_key: input_key,
        fact_class: "target_intel_dns".to_string(),
        temporal_policy_id,
        reason_code: "expired".to_string(),
        risk_tier: "T1".to_string(),
        mandatory_axis: true,
        consumer_kind: "candidate".to_string(),
        consumer_key: "candidate:one".to_string(),
    };
    let first = golish_db::repo::tool_truth_revalidation::record_obligation(db.pool(), &base)
        .await
        .expect("record first stale authority obligation");
    let replay = golish_db::repo::tool_truth_revalidation::record_obligation(
        db.pool(),
        &golish_db::repo::tool_truth_revalidation::RecordRevalidationObligation {
            consumer_kind: "report_download".to_string(),
            consumer_key: "report:one".to_string(),
            ..base
        },
    )
    .await
    .expect("a second consumer joins the exact open obligation");
    assert_eq!(replay.id, first.id);
    let consumers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tool_truth_revalidation_consumers WHERE obligation_id=$1",
    )
    .bind(first.id)
    .fetch_one(db.pool())
    .await
    .expect("count deduplicated consumers");
    assert_eq!(consumers, 2);
    let policy: (String, String) = sqlx::query_as(
        "SELECT dispatch_mode,max_risk_tier FROM tool_truth_revalidation_dispatch_policies WHERE operation_id=$1",
    )
    .bind(frozen.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("read operation-frozen default policy");
    assert_eq!(policy, ("manual_only".to_string(), "T1".to_string()));
    let error = golish_db::repo::tool_truth_revalidation::claim_next(
        db.pool(),
        &golish_db::repo::tool_truth_revalidation::ClaimRevalidationObligation {
            operation_id: frozen.operation_id,
            owner: "background-one".to_string(),
            expected_dispatch_generation: 0,
            expected_head_row_version: 0,
        },
    )
    .await
    .expect_err("deployment-default held head performs zero dispatch");
    assert!(error
        .to_string()
        .contains("TOOL_TRUTH_REVALIDATION_DISPATCH_HELD"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn tool_truth_revalidation_expired_receipt_is_not_current_and_records_stable_obligation() {
    let (mut db, _data_dir) = fixture("tool_truth_revalidation_expired_projection").await;
    let (frozen, receipt, _input_id, _input_key, _temporal_policy_id) =
        seed_revalidation_source(db.pool(), "tool-truth-revalidation-expired-projection").await;
    sqlx::query(
        r#"UPDATE capability_execution_receipts
              SET valid_until=observation_completed_at+INTERVAL '1 millisecond',
                  row_version=row_version+1
            WHERE id=$1"#,
    )
    .bind(receipt.id)
    .execute(db.pool())
    .await
    .expect("expire the isolated source receipt");
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let first = golish_db::repo::tool_truth_revalidation::record_expired_target_intel_obligations(
        db.pool(),
        frozen.operation_id,
        frozen.organization_id,
        "candidate",
        "candidate:expired",
    )
    .await
    .expect("record exact expired inputs");
    assert_eq!(first.len(), 1);
    let replay = golish_db::repo::tool_truth_revalidation::record_expired_target_intel_obligations(
        db.pool(),
        frozen.operation_id,
        frozen.organization_id,
        "reporting",
        "reporting:expired",
    )
    .await
    .expect("a second consumer reuses the same obligation");
    assert_eq!(replay.len(), 1);
    assert_eq!(first[0].id, replay[0].id);
    let projection = capability_execution_receipts::current_target_intel_projection(
        db.pool(),
        frozen.operation_id,
        frozen.organization_id,
    )
    .await
    .expect("read current projection")
    .expect("projection exists");
    assert!(projection.receipts.iter().all(|row| !row.authority_current));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn tool_truth_revalidation_claims_with_lease_and_exhausts_same_no_progress() {
    let (mut db, _data_dir) = fixture("tool_truth_revalidation_bounded").await;
    let (frozen, receipt, input_id, input_key, temporal_policy_id) =
        seed_revalidation_source(db.pool(), "tool-truth-revalidation-bounded").await;
    let obligation = golish_db::repo::tool_truth_revalidation::record_obligation(
        db.pool(),
        &golish_db::repo::tool_truth_revalidation::RecordRevalidationObligation {
            operation_id: frozen.operation_id,
            organization_id: frozen.organization_id,
            source_receipt_id: receipt.id,
            source_receipt_input_id: input_id,
            source_input_key: input_key,
            fact_class: "target_intel_dns".to_string(),
            temporal_policy_id,
            reason_code: "expired".to_string(),
            risk_tier: "T1".to_string(),
            mandatory_axis: true,
            consumer_kind: "candidate".to_string(),
            consumer_key: "candidate:bounded".to_string(),
        },
    )
    .await
    .expect("record bounded obligation");
    release_revalidation_dispatch_for_test(db.pool(), frozen.operation_id).await;

    let claim_command = golish_db::repo::tool_truth_revalidation::ClaimRevalidationObligation {
        operation_id: frozen.operation_id,
        owner: "background-bounded".to_string(),
        expected_dispatch_generation: 1,
        expected_head_row_version: 1,
    };
    let mut current = obligation;
    for ordinal in 0..3 {
        let claim = golish_db::repo::tool_truth_revalidation::claim_next(db.pool(), &claim_command)
            .await
            .expect("claim bounded obligation")
            .expect("one obligation remains claimable");
        assert_eq!(claim.claim_owner.as_deref(), Some("background-bounded"));
        assert!(claim.claim_token.is_some());
        current = golish_db::repo::tool_truth_revalidation::record_failure(
            db.pool(),
            &golish_db::repo::tool_truth_revalidation::FailRevalidationObligation {
                obligation_id: claim.id,
                owner: "background-bounded".to_string(),
                claim_token: claim.claim_token.expect("claim token"),
                expected_row_version: claim.row_version,
                progress_fingerprint: "same-empty-result".to_string(),
                reason_code: format!("fixture_failure_{ordinal}"),
            },
        )
        .await
        .expect("record bounded failed attempt");
    }
    assert_eq!(current.status, "exhausted");
    assert!(current.residual.is_some());
    assert!(current.mandatory_axis);
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tool_truth_revalidation_events WHERE obligation_id=$1",
    )
    .bind(current.id)
    .fetch_one(db.pool())
    .await
    .expect("count append-only lifecycle events");
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tool_truth_revalidation_outbox WHERE obligation_id=$1",
    )
    .bind(current.id)
    .fetch_one(db.pool())
    .await
    .expect("count typed outbox events");
    assert_eq!(event_count, outbox_count);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn tool_truth_revalidation_success_requires_new_immutable_authority() {
    let (mut db, _data_dir) = fixture("tool_truth_revalidation_success").await;
    let (frozen, source, input_id, input_key, temporal_policy_id) =
        seed_revalidation_source(db.pool(), "tool-truth-revalidation-success").await;
    let obligation = golish_db::repo::tool_truth_revalidation::record_obligation(
        db.pool(),
        &golish_db::repo::tool_truth_revalidation::RecordRevalidationObligation {
            operation_id: frozen.operation_id,
            organization_id: frozen.organization_id,
            source_receipt_id: source.id,
            source_receipt_input_id: input_id,
            source_input_key: input_key,
            fact_class: "target_intel_dns".to_string(),
            temporal_policy_id,
            reason_code: "expired".to_string(),
            risk_tier: "T1".to_string(),
            mandatory_axis: true,
            consumer_kind: "candidate".to_string(),
            consumer_key: "candidate:success".to_string(),
        },
    )
    .await
    .expect("record successful replacement obligation");
    release_revalidation_dispatch_for_test(db.pool(), frozen.operation_id).await;
    let claim = golish_db::repo::tool_truth_revalidation::claim_next(
        db.pool(),
        &golish_db::repo::tool_truth_revalidation::ClaimRevalidationObligation {
            operation_id: frozen.operation_id,
            owner: "background-success".to_string(),
            expected_dispatch_generation: 1,
            expected_head_row_version: 1,
        },
    )
    .await
    .expect("claim successful replacement obligation")
    .expect("replacement obligation is claimable");
    assert_eq!(claim.id, obligation.id);

    let wave_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM stage_asset_waves WHERE operation_id=$1 AND organization_id=$2 AND stage_kind='target_intel'",
    )
    .bind(frozen.operation_id)
    .bind(frozen.organization_id)
    .fetch_one(db.pool())
    .await
    .expect("read frozen TargetIntel source wave");
    let replacement_denominator = capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            stage_execution_id: frozen.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(wave_id),
        },
        compile_target_intel_denominator,
    )
    .await
    .expect("seal a new immutable denominator for revalidation");
    assert_ne!(replacement_denominator.id, source.denominator_id);
    let replacement =
        begin_managed_target_intel_receipt(db.pool(), replacement_denominator.id, "intel.dns", 2)
            .await;
    let close =
        target_intel_finalization(db.pool(), &replacement, &[("GOLISH-INTEL-DNS", "found")]).await;
    let replacement =
        capability_execution_receipts::finalize_target_intel_receipt(db.pool(), &close)
            .await
            .expect("finalize replacement receipt");
    let completed = golish_db::repo::tool_truth_revalidation::complete_success(
        db.pool(),
        &golish_db::repo::tool_truth_revalidation::CompleteRevalidationObligation {
            obligation_id: claim.id,
            owner: "background-success".to_string(),
            claim_token: claim.claim_token.expect("claim token"),
            expected_row_version: claim.row_version,
            replacement_denominator_id: replacement_denominator.id,
            replacement_receipt_id: replacement.id,
        },
    )
    .await
    .expect("complete with new immutable authority");
    assert_eq!(completed.status, "succeeded");
    assert_eq!(completed.source_receipt_id, source.id);
    assert_eq!(
        completed.replacement_denominator_id,
        Some(replacement_denominator.id)
    );
    assert_eq!(completed.replacement_receipt_id, Some(replacement.id));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn late_prior_attempt_is_superseded_when_target_intel_attempt_advances() {
    let (mut db, _data_dir) = fixture("late_prior_attempt").await;
    let (_frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "late-prior-attempt").await;
    let prior = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin target_intel attempt N");
    capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            attempt_ordinal: 2,
        },
    )
    .await
    .expect("begin target_intel attempt N+1");
    let prior = capability_execution_receipts::get(db.pool(), prior.id)
        .await
        .expect("read prior receipt")
        .expect("prior receipt exists");
    assert_eq!(prior.attempt_state, "superseded");
    assert_eq!(prior.reconciliation_state, "superseded");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_current_projection_uses_only_latest_attempt() {
    let (mut db, _data_dir) = fixture("target_intel_projection").await;
    let (frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-projection").await;
    capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin old target_intel attempt");
    capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            attempt_ordinal: 2,
        },
    )
    .await
    .expect("begin current target_intel attempt");
    let projection = capability_execution_receipts::current_target_intel_projection(
        db.pool(),
        frozen.operation_id,
        frozen.organization_id,
    )
    .await
    .expect("read exact target_intel projection")
    .expect("sealed denominator is current");
    assert_eq!(projection.stage_execution_id, frozen.stage_execution_id);
    assert_eq!(projection.denominator_id, denominator.id);
    assert_eq!(projection.attempt_epoch, 2);
    assert_eq!(projection.expected.len(), 2);
    assert!(projection.receipts.is_empty());
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn target_intel_network_hop_census_rejects_forged_count_and_late_address() {
    let (mut db, _data_dir) = fixture("target_intel_network_hop").await;
    let (_frozen, denominator) =
        seed_target_intel_denominator(db.pool(), "target-intel-network-hop").await;
    let receipt = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "intel.dns".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin receipt for network-hop census");
    let (destination_policy_id, destination_policy_hash): (Uuid, String) = sqlx::query_as(
        "SELECT destination_policy_id,destination_policy_hash FROM capability_execution_receipts WHERE id=$1",
    )
    .bind(receipt.id)
    .fetch_one(db.pool())
    .await
    .expect("load exact destination policy");
    let hop_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO capability_execution_network_hops(
               id,receipt_id,execution_authority_id,receipt_authority_hash,
               hop_ordinal,hop_kind,scheme,normalized_host,port,path_and_query_hash,
               destination_policy_id,destination_policy_hash,transport_decision,
               send_ordinal,hop_hash
           ) VALUES($1,$2,$3,$4,0,'initial','https','fixed.provider.example.test',443,$5,
                    $6,$7,'authorized_and_sent',1,$8)"#,
    )
    .bind(hop_id)
    .bind(receipt.id)
    .bind(receipt.execution_authority_id)
    .bind(&receipt.receipt_authority_hash)
    .bind(digest_v1('1'))
    .bind(destination_policy_id)
    .bind(destination_policy_hash)
    .bind(digest_v1('2'))
    .execute(db.pool())
    .await
    .expect("insert open network-hop census");
    sqlx::query(
        r#"INSERT INTO capability_execution_network_hop_addresses(
               id,network_hop_id,receipt_id,execution_authority_id,ordinal,
               address,address_class,selected_for_pin,member_hash
           ) VALUES($1,$2,$3,$4,0,'203.0.113.10','public',TRUE,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(hop_id)
    .bind(receipt.id)
    .bind(receipt.execution_authority_id)
    .bind(digest_v1('3'))
    .execute(db.pool())
    .await
    .expect("insert exact resolved address");
    let forged = sqlx::query(
        "UPDATE capability_execution_network_hops SET member_count=99,sealed_at=NOW() WHERE id=$1",
    )
    .bind(hop_id)
    .execute(db.pool())
    .await
    .expect_err("caller-authored DNS member count must be rejected");
    assert_database_rejection(&forged, "23514", "tool_truth_member_count_forged");
    sqlx::query("UPDATE capability_execution_network_hops SET sealed_at=NOW() WHERE id=$1")
        .bind(hop_id)
        .execute(db.pool())
        .await
        .expect("seal exact network-hop census");
    let late = sqlx::query(
        r#"INSERT INTO capability_execution_network_hop_addresses(
               id,network_hop_id,receipt_id,execution_authority_id,ordinal,
               address,address_class,selected_for_pin,member_hash
           ) VALUES($1,$2,$3,$4,1,'203.0.113.11','public',FALSE,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(hop_id)
    .bind(receipt.id)
    .bind(receipt.execution_authority_id)
    .bind(digest_v1('4'))
    .execute(db.pool())
    .await
    .expect_err("sealed network-hop census rejects late DNS members");
    assert_database_rejection(&late, "23514", "tool_truth_sealed_parent_immutable");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn operation_insert_defaults_tool_truth_contract_to_legacy_v1() {
    let (mut db, _data_dir) = fixture("operation_contract_default").await;
    let frozen = seed_frozen_execution(db.pool(), "operation-default").await;

    let contract: String =
        sqlx::query_scalar("SELECT tool_truth_contract FROM operation_state WHERE operation_id=$1")
            .bind(frozen.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("read operation-frozen Tool Truth contract");
    assert_eq!(contract, "legacy_v1");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn persisted_operation_tool_truth_contract_is_db_immutable() {
    let (mut db, _data_dir) = fixture("operation_contract_immutable").await;
    let frozen = seed_frozen_execution(db.pool(), "operation-immutable").await;

    let error = sqlx::query(
        "UPDATE operation_state SET tool_truth_contract='receipt_v1' WHERE operation_id=$1",
    )
    .bind(frozen.operation_id)
    .execute(db.pool())
    .await
    .expect_err("operation-frozen contract must reject direct SQL UPDATE");
    assert_database_rejection(&error, "23514", "operation_tool_truth_contract_immutable");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn rollout_rejects_direct_update_and_delete() {
    let (mut db, _data_dir) = fixture("rollout_guard").await;

    let update_error = sqlx::query(
        "UPDATE tool_truth_rollout SET new_operation_contract='shadow_v1' WHERE singleton=TRUE",
    )
    .execute(db.pool())
    .await
    .expect_err("frozen rollout must reject direct SQL UPDATE");
    assert_database_rejection(
        &update_error,
        "23514",
        "tool_truth_rollout_direct_mutation_forbidden",
    );

    let delete_error = sqlx::query("DELETE FROM tool_truth_rollout WHERE singleton=TRUE")
        .execute(db.pool())
        .await
        .expect_err("frozen rollout must reject direct SQL DELETE");
    assert_database_rejection(
        &delete_error,
        "23514",
        "tool_truth_rollout_direct_mutation_forbidden",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn task9_schema_has_dynamic_child_authority_set_and_gate_families() {
    let (mut db, _data_dir) = fixture("task9_schema_catalog").await;
    let tables: Vec<String> = sqlx::query_scalar(
        r#"SELECT table_name FROM information_schema.tables
            WHERE table_schema=current_schema() AND table_name = ANY($1)
            ORDER BY table_name"#,
    )
    .bind(vec![
        "capability_discovered_child_manifests",
        "capability_discovered_child_members",
        "capability_discovered_child_closures",
        "tool_truth_authority_set_seals",
        "tool_truth_authority_set_members",
        "tool_truth_gate_assessments",
    ])
    .fetch_all(db.pool())
    .await
    .expect("read Task 9 schema catalog");
    assert_eq!(tables.len(), 6);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn missing_denominator_gate_assessment_is_scoped_and_append_only() {
    let (mut db, _data_dir) = fixture("gate_assessment_missing").await;
    let frozen = seed_frozen_execution(db.pool(), "gate-assessment-missing").await;
    let (authority_id, authority_hash) = insert_host_authority(db.pool(), &frozen).await;
    let assessment_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tool_truth_gate_assessments(
               id,stable_gate_request_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_id,
               execution_authority_hash,assessment_basis_kind,denominator_id,
               authority_set_id,legacy_allowed,control_decision,coverage_grade,
               divergence,expected_item_count,terminal_item_count,degraded_item_count,
               residual,assessment_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
                    'missing_denominator',NULL,NULL,TRUE,'hold','incomplete',TRUE,
                    0,0,0,$12,$13)"#,
    )
    .bind(assessment_id)
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(authority_id)
    .bind(&authority_hash)
    .bind(serde_json::json!({"reason_code": "TOOL_TRUTH_DENOMINATOR_MISSING"}))
    .bind(digest_v1('d'))
    .execute(db.pool())
    .await
    .expect("persist explicit missing-denominator shadow assessment");

    let update_error =
        sqlx::query("UPDATE tool_truth_gate_assessments SET legacy_allowed=FALSE WHERE id=$1")
            .bind(assessment_id)
            .execute(db.pool())
            .await
            .expect_err("gate assessment is append-only");
    assert_database_rejection(&update_error, "23514", "tool_truth_append_only");

    let cross_org_error = sqlx::query(
        r#"INSERT INTO tool_truth_gate_assessments(
               id,stable_gate_request_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_id,
               execution_authority_hash,assessment_basis_kind,legacy_allowed,
               control_decision,coverage_grade,divergence,expected_item_count,
               terminal_item_count,degraded_item_count,residual,assessment_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
                    'missing_denominator',FALSE,'hold','incomplete',FALSE,0,0,0,'{}',$12)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.outside_organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(authority_id)
    .bind(&authority_hash)
    .bind(digest_v1('e'))
    .execute(db.pool())
    .await
    .expect_err("assessment cannot splice another organization into the authority tuple");
    assert_database_rejection(
        &cross_org_error,
        "23503",
        "tool_truth_gate_assessment_authority_fk",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn catalog_uses_stage_execution_uuid_and_bigint_worker_attempt_identity() {
    let (mut db, _data_dir) = fixture("identity_catalog").await;

    let forbidden_attempt_epoch_columns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM information_schema.columns
            WHERE table_schema=current_schema()
              AND table_name IN (
                  'coverage_denominators',
                  'capability_execution_destination_policies',
                  'capability_execution_receipts'
              )
              AND column_name='attempt_epoch'"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect stage-owned Tool Truth identity columns");
    assert_eq!(
        forbidden_attempt_epoch_columns, 0,
        "stage attempt identity must only be stage_execution_id"
    );

    let worker_epoch_type: Option<String> = sqlx::query_scalar(
        r#"SELECT data_type
             FROM information_schema.columns
            WHERE table_schema=current_schema()
              AND table_name='tool_truth_execution_authorities'
              AND column_name='worker_attempt_epoch'"#,
    )
    .fetch_optional(db.pool())
    .await
    .expect("inspect worker attempt identity type");
    assert_eq!(worker_epoch_type.as_deref(), Some("bigint"));

    for table_name in ["stage_worker_runs", "tool_calls"] {
        let data_type: Option<String> = sqlx::query_scalar(
            r#"SELECT data_type
                 FROM information_schema.columns
                WHERE table_schema=current_schema()
                  AND table_name=$1
                  AND column_name='attempt_epoch'"#,
        )
        .bind(table_name)
        .fetch_optional(db.pool())
        .await
        .expect("inspect existing worker fence epoch type");
        assert_eq!(data_type.as_deref(), Some("bigint"), "table={table_name}");
    }

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn execution_authority_rejects_cross_organization_scope_member() {
    let (mut db, _data_dir) = fixture("authority_cross_org").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-cross-org").await;

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.outside_organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(digest_v1('5'))
    .execute(db.pool())
    .await
    .expect_err("outside organization must not join a frozen scope authority");
    assert_database_rejection(&error, "23514", "tool_truth_authority_scope_org_mismatch");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn execution_authority_rejects_cross_scope_snapshot() {
    let (mut db, _data_dir) = fixture("authority_cross_scope").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-cross-scope-a").await;
    let foreign = seed_frozen_execution(db.pool(), "authority-cross-scope-b").await;

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(foreign.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(digest_v1('6'))
    .execute(db.pool())
    .await
    .expect_err("foreign snapshot must not join an operation authority");
    assert_database_rejection(
        &error,
        "23514",
        "tool_truth_authority_scope_snapshot_mismatch",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn execution_authority_rejects_cross_stage_execution() {
    let (mut db, _data_dir) = fixture("authority_cross_stage").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-cross-stage").await;

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,execution_owner_kind,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_execution','host_stage',$10
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.other_stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(digest_v1('7'))
    .execute(db.pool())
    .await
    .expect_err("stage execution and stage kind must be the same frozen parent");
    assert_database_rejection(&error, "23514", "tool_truth_authority_stage_mismatch");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn worker_execution_rejects_old_epoch_with_new_lease() {
    let (mut db, _data_dir) = fixture("authority_worker_fence").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-worker-fence").await;
    let forged_lease = Uuid::new_v4();
    assert_ne!(forged_lease, frozen.lease_token);

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_source_kind,stage_run_unit_id,execution_owner_kind,
               worker_run_id,worker_attempt_epoch,lease_token,source_tool_call_id,
               authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,
               'stage_unit',$10,'worker_tool',$11,$12,$13,$14,$15
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(frozen.stage_run_unit_id)
    .bind(frozen.worker_run_id)
    .bind(frozen.worker_attempt_epoch)
    .bind(forged_lease)
    .bind(frozen.source_tool_call_id)
    .bind(digest_v1('8'))
    .execute(db.pool())
    .await
    .expect_err("old worker epoch cannot be paired with a forged new lease");
    assert_database_rejection(&error, "23514", "tool_truth_worker_fence_mismatch");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn worker_execution_rejects_cross_worker_same_epoch_splice() {
    let (mut db, _data_dir) = fixture("authority_cross_worker_epoch").await;
    let frozen = seed_frozen_execution(db.pool(), "authority-cross-worker-epoch").await;
    let other_worker_id = Uuid::new_v4();
    let other_lease = Uuid::new_v4();
    let other_tool_call_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES(
               $1,$2,$3,$4,$5,1,'tool_truth_fixture_2','stage_unit','fixture-2',
               'main>enumeration>second','running',$6,'tool-truth-fixture-2',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),$7
           )"#,
    )
    .bind(other_worker_id)
    .bind(frozen.operation_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_run_unit_id)
    .bind(frozen.organization_id)
    .bind(other_lease)
    .bind(frozen.worker_attempt_epoch)
    .execute(db.pool())
    .await
    .expect("insert second worker at same epoch");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,'tool-truth-source-2',$2,$3,'primary','tool_truth_fixture','{}','running',
               $3,$4,$5,$6,$7,$8,$9
           )"#,
    )
    .bind(other_tool_call_id)
    .bind(frozen.session_id)
    .bind(frozen.operation_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_run_unit_id)
    .bind(other_worker_id)
    .bind(frozen.organization_id)
    .bind(frozen.worker_attempt_epoch)
    .bind(other_lease)
    .execute(db.pool())
    .await
    .expect("insert second worker tool call");

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_source_kind,stage_run_unit_id,
               execution_owner_kind,worker_run_id,worker_attempt_epoch,lease_token,
               source_tool_call_id,authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_unit',$10,'worker_tool',
               $11,$12,$13,$14,$15
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(frozen.stage_run_unit_id)
    .bind(frozen.worker_run_id)
    .bind(frozen.worker_attempt_epoch)
    .bind(frozen.lease_token)
    .bind(other_tool_call_id)
    .bind(digest_v1('8'))
    .execute(db.pool())
    .await
    .expect_err("same epoch from another worker cannot be spliced into authority");
    assert_database_rejection(&error, "23514", "tool_truth_worker_fence_mismatch");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn evidence_adapter_rejects_non_evidence_audit_role() {
    let (mut db, _data_dir) = fixture("evidence_role").await;
    let frozen = seed_frozen_execution(db.pool(), "evidence-role").await;
    let (execution_authority_id, execution_authority_hash) =
        insert_host_authority(db.pool(), &frozen).await;

    let audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,run_id,audit_role,detail,status
           ) VALUES(
               'tool_truth_fixture','test','not evidence',$1,$2,'action',$3,'completed'
           ) RETURNING id"#,
    )
    .bind(&frozen.project_path)
    .bind(frozen.operation_id)
    .bind(serde_json::json!({"organization_id": frozen.organization_id}))
    .fetch_one(db.pool())
    .await
    .expect("insert non-evidence audit row");
    let classification_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications(
               evidence_audit_id,classification,scope_version,reason,
               classified_by_session,producing_stage_run_id
           ) VALUES($1,'in_scope',1,'fixture',$2,$3)
           RETURNING id"#,
    )
    .bind(audit_id)
    .bind(frozen.session_id.to_string())
    .bind(frozen.stage_execution_id)
    .fetch_one(db.pool())
    .await
    .expect("insert current classification for non-evidence row");

    let error = sqlx::query(
        r#"INSERT INTO tool_truth_evidence_production_bindings(
               id,execution_authority_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,production_binding_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(execution_authority_id)
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(execution_authority_hash)
    .bind(audit_id)
    .bind(classification_id)
    .bind(digest_v1('9'))
    .execute(db.pool())
    .await
    .expect_err("action audit row must not be normalized as Evidence");
    assert_database_rejection(&error, "23514", "tool_truth_evidence_role_invalid");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn business_ref_adapter_rejects_typed_id_shape_confusion() {
    let (mut db, _data_dir) = fixture("business_ref_shape").await;

    let dns_uuid_error = sqlx::query(
        r#"INSERT INTO tool_truth_business_ref_authorities(
               id,execution_authority_id,evidence_authority_id,ref_kind,
               ref_uuid,ref_bigint,source_hash,authority_hash
           ) VALUES($1,$2,$3,'dns_record',$4,NULL,$5,$6)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest_v1('d'))
    .bind(digest_v1('e'))
    .execute(db.pool())
    .await
    .expect_err("DNS reference must use the BIGINT typed identity column");
    assert_database_rejection(
        &dns_uuid_error,
        "23514",
        "tool_truth_business_ref_id_shape_invalid",
    );

    let target_bigint_error = sqlx::query(
        r#"INSERT INTO tool_truth_business_ref_authorities(
               id,execution_authority_id,evidence_authority_id,ref_kind,
               ref_uuid,ref_bigint,source_hash,authority_hash
           ) VALUES($1,$2,$3,'target_asset',NULL,42,$4,$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(digest_v1('f'))
    .bind(digest_v1('0'))
    .execute(db.pool())
    .await
    .expect_err("UUID reference kind must not use the BIGINT identity column");
    assert_database_rejection(
        &target_bigint_error,
        "23514",
        "tool_truth_business_ref_id_shape_invalid",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn raw_witness_rejects_missing_reciprocal_receipt_binding() {
    let (mut db, _data_dir) = fixture("raw_reciprocal").await;
    let frozen = seed_frozen_execution(db.pool(), "raw-reciprocal").await;
    let (execution_authority_id, _) = insert_host_authority(db.pool(), &frozen).await;

    let raw_to_receipt_fk_defs: Vec<String> = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conrelid='capability_raw_witness_artifacts'::regclass
              AND contype='f'"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect raw witness foreign keys");
    assert!(
        raw_to_receipt_fk_defs.iter().any(|definition| {
            definition.contains(
                "FOREIGN KEY (receipt_id, execution_authority_id, receipt_authority_hash)",
            ) && definition.contains(
                "capability_execution_receipts(id, execution_authority_id, receipt_authority_hash)",
            )
        }),
        "raw witness must bind the exact receipt authority tuple: {raw_to_receipt_fk_defs:?}"
    );

    let receipt_to_raw_fk_defs: Vec<String> = sqlx::query_scalar(
        r#"SELECT pg_get_constraintdef(oid)
             FROM pg_constraint
            WHERE conrelid='capability_execution_receipts'::regclass
              AND contype='f'"#,
    )
    .fetch_all(db.pool())
    .await
    .expect("inspect receipt raw-witness back-reference");
    assert!(
        receipt_to_raw_fk_defs.iter().any(|definition| {
            definition.contains("FOREIGN KEY (raw_witness_artifact_id, id, execution_authority_id)")
                && definition.contains(
                    "capability_raw_witness_artifacts(id, receipt_id, execution_authority_id)",
                )
        }),
        "receipt must point back to its own exact raw witness: {receipt_to_raw_fk_defs:?}"
    );

    let error = sqlx::query(
        r#"INSERT INTO capability_raw_witness_artifacts(
               id,receipt_id,execution_authority_id,receipt_authority_hash,
               content_key,vault_object_ref_token,vault_object_ref_token_hash,
               sha256,ciphertext_sha256,encryption_contract_version,
               operation_key_ref_hash,key_generation,retention_policy_id,
               retention_policy_hash,sensitivity_disposition,
               original_byte_count,stored_byte_count,truncated
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,'raw_witness_envelope.v1',
               $10,1,$11,$12,'typed_derivative_ready',1,1,FALSE
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(execution_authority_id)
    .bind(digest_v1('1'))
    .bind(digest_v1('2'))
    .bind(vec![0_u8; 32])
    .bind(digest_v1('3'))
    .bind(digest_v1('4'))
    .bind(digest_v1('5'))
    .bind(digest_v1('6'))
    .bind(Uuid::new_v4())
    .bind(digest_v1('7'))
    .execute(db.pool())
    .await
    .expect_err("raw witness without an exact reciprocal receipt must be rejected");
    assert_database_rejection(
        &error,
        "23503",
        "capability_raw_witness_receipt_authority_fk",
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn sealed_header_rejects_late_denominator_member_insert() {
    let (mut db, _data_dir) = fixture("denominator_late_member").await;
    let frozen = seed_frozen_execution(db.pool(), "denominator-late-member").await;
    let (execution_authority_id, execution_authority_hash) =
        insert_host_authority(db.pool(), &frozen).await;
    let denominator_id = Uuid::new_v4();

    let denominator_hash: String = sqlx::query_scalar(
        r#"INSERT INTO coverage_denominators(
               id,stable_seal_request_id,execution_authority_id,
               operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,stage_execution_id,stage_kind,
               execution_authority_hash,denominator_kind,contract,
               denominator_hash,input_manifest_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
               'root','shadow_v1',$12,$13
           ) RETURNING denominator_hash"#,
    )
    .bind(denominator_id)
    .bind(Uuid::new_v4())
    .bind(execution_authority_id)
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(execution_authority_hash)
    .bind(digest_v1('8'))
    .bind(digest_v1('9'))
    .fetch_one(db.pool())
    .await
    .expect("insert open denominator header");
    sqlx::query(
        r#"INSERT INTO coverage_denominator_items(
               id,denominator_id,execution_authority_id,denominator_hash,
               ordinal,input_key,exact_asset,technique,expected_capability,member_hash
           ) VALUES($1,$2,$3,$4,0,'root','example.test','enumerate','dns',$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(denominator_id)
    .bind(execution_authority_id)
    .bind(&denominator_hash)
    .bind(digest_v1('b'))
    .execute(db.pool())
    .await
    .expect("insert denominator member before seal");
    let forged_count = sqlx::query(
        "UPDATE coverage_denominators SET sealed_at=statement_timestamp(),member_count=99 WHERE id=$1",
    )
    .bind(denominator_id)
    .execute(db.pool())
    .await
    .expect_err("caller-forged member count must be rejected, not overwritten");
    assert_database_rejection(&forged_count, "23514", "tool_truth_member_count_forged");
    let forged_hash = sqlx::query(
        "UPDATE coverage_denominators SET sealed_at=statement_timestamp(),member_set_hash=$2 WHERE id=$1",
    )
    .bind(denominator_id)
    .bind(digest_v1('a'))
    .execute(db.pool())
    .await
    .expect_err("caller-forged member hash must be rejected, not overwritten");
    assert_database_rejection(&forged_hash, "23514", "tool_truth_member_set_hash_forged");
    sqlx::query("UPDATE coverage_denominators SET sealed_at=statement_timestamp() WHERE id=$1")
        .bind(denominator_id)
        .execute(db.pool())
        .await
        .expect("seal denominator from its exact member set");

    let error = sqlx::query(
        r#"INSERT INTO coverage_denominator_items(
               id,denominator_id,execution_authority_id,denominator_hash,
               ordinal,input_key,exact_asset,technique,expected_capability,member_hash
           ) VALUES($1,$2,$3,$4,1,'late','late.example','enumerate','dns',$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(denominator_id)
    .bind(execution_authority_id)
    .bind(denominator_hash)
    .bind(digest_v1('c'))
    .execute(db.pool())
    .await
    .expect_err("sealed denominator must reject late direct-SQL members");
    assert_database_rejection(&error, "23514", "tool_truth_sealed_parent_immutable");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn seal_wave_denominator_derives_locked_wave_exactly_and_replays() {
    let (mut db, _data_dir) = fixture("repo_denominator_replay").await;
    let wave = seed_wave_denominator_fixture(
        db.pool(),
        "repo-denominator-replay",
        &["a.example", "b.example"],
    )
    .await;
    let command = seal_wave_command(&wave, Uuid::new_v4());

    let first = capability_execution_receipts::seal_wave_denominator(db.pool(), &command)
        .await
        .expect("seal server-derived denominator");
    assert_eq!(first.member_count, Some(2));
    assert!(first.sealed_at.is_some());
    assert!(first.input_manifest_hash.starts_with("sha256:"));

    let members: Vec<(i32, String, Uuid)> = sqlx::query_as(
        "SELECT ordinal,exact_asset,target_id FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY ordinal",
    )
    .bind(first.id)
    .fetch_all(db.pool())
    .await
    .expect("read exact denominator members");
    assert_eq!(members.len(), 2);
    assert_eq!(
        members
            .iter()
            .map(|(_, asset, _)| asset.as_str())
            .collect::<Vec<_>>(),
        vec!["a.example", "b.example"]
    );

    let replay = capability_execution_receipts::seal_wave_denominator(db.pool(), &command)
        .await
        .expect("response-loss replay returns exact denominator");
    assert_eq!(replay, first);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn seal_wave_denominator_rejects_stable_request_source_drift() {
    let (mut db, _data_dir) = fixture("repo_denominator_drift").await;
    let mut wave =
        seed_wave_denominator_fixture(db.pool(), "repo-denominator-drift", &["a.example"]).await;
    let stable_request = Uuid::new_v4();
    capability_execution_receipts::seal_wave_denominator(
        db.pool(),
        &seal_wave_command(&wave, stable_request),
    )
    .await
    .expect("seal first wave");

    stage_asset_waves::complete(db.pool(), wave.wave_id)
        .await
        .expect("complete first wave");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source
           ) VALUES($1,'later.example','domain','later.example','in',$2,$3,'tool_truth_fixture')"#,
    )
    .bind(Uuid::new_v4())
    .bind(&wave.frozen.project_path)
    .bind(wave.frozen.organization_id)
    .execute(db.pool())
    .await
    .expect("insert later target");
    let next = stage_asset_waves::create_next(
        db.pool(),
        wave.frozen.operation_id,
        wave.frozen.organization_id,
        wave.frozen.stage_kind,
        Some(wave.wave_id),
        100,
    )
    .await
    .expect("create next exact wave")
    .expect("later target creates next wave");
    wave.wave_id = next.wave.id;

    let error = capability_execution_receipts::seal_wave_denominator(
        db.pool(),
        &seal_wave_command(&wave, stable_request),
    )
    .await
    .expect_err("stable request cannot be rebound to another wave");
    assert!(error.to_string().contains("TOOL_TRUTH_MANIFEST_DRIFT"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn denominator_source_compound_derives_exact_members_and_replays() {
    let (mut db, _data_dir) = fixture("source_compound_replay").await;
    let wave = seed_wave_denominator_fixture(
        db.pool(),
        "source-compound-replay",
        &["a.example", "b.example"],
    )
    .await;
    let command = capability_execution_receipts::SealSourceDenominator {
        stable_seal_request_id: Uuid::new_v4(),
        stage_execution_id: wave.frozen.stage_execution_id,
        source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(wave.wave_id),
    };
    let first = capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &command,
        compile_test_denominator,
    )
    .await
    .expect("seal exact asset-times-technique denominator");
    assert_eq!(first.member_count, Some(4));

    let members: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT exact_asset,technique,expected_capability FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY ordinal",
    )
    .bind(first.id)
    .fetch_all(db.pool())
    .await
    .expect("read compiled denominator members");
    assert_eq!(members.len(), 4);
    let mut assets = members
        .iter()
        .map(|member| member.0.as_str())
        .collect::<Vec<_>>();
    assets.sort_unstable();
    assert_eq!(
        assets,
        vec!["a.example", "a.example", "b.example", "b.example"]
    );

    let replay = capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &command,
        compile_test_denominator,
    )
    .await
    .expect("same source request replays exactly");
    assert_eq!(replay, first);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn denominator_stable_request_cannot_rebind_wave_to_unit() {
    let (mut db, _data_dir) = fixture("source_compound_rebind").await;
    let wave =
        seed_wave_denominator_fixture(db.pool(), "source-compound-rebind", &["a.example"]).await;
    sqlx::query(
        "UPDATE stage_runs SET started_at=statement_timestamp()+INTERVAL '1 second' WHERE id=$1",
    )
    .bind(wave.frozen.stage_execution_id)
    .execute(db.pool())
    .await
    .expect("make fixture target part of the frozen unit cutoff");
    let stable = Uuid::new_v4();
    capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: stable,
            stage_execution_id: wave.frozen.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                wave.wave_id,
            ),
        },
        compile_test_denominator,
    )
    .await
    .expect("seal wave source first");
    let error = capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: stable,
            stage_execution_id: wave.frozen.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageTeamUnit(
                wave.frozen.stage_run_unit_id,
            ),
        },
        compile_test_denominator,
    )
    .await
    .expect_err("stable request cannot rebind from wave to unit");
    assert!(error.to_string().contains("TOOL_TRUTH_MANIFEST_DRIFT"));
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn bound_wave_rejects_late_source_members() {
    let (mut db, _data_dir) = fixture("bound_wave_immutable").await;
    let wave = seed_wave_denominator_fixture(db.pool(), "bound-wave", &["a.example"]).await;
    capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            stage_execution_id: wave.frozen.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                wave.wave_id,
            ),
        },
        compile_test_denominator,
    )
    .await
    .expect("bind and seal wave source");
    let late_target_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source
           ) VALUES($1,'late.example','domain','late.example','in',$2,$3,'tool_truth_fixture')"#,
    )
    .bind(late_target_id)
    .bind(&wave.frozen.project_path)
    .bind(wave.frozen.organization_id)
    .execute(db.pool())
    .await
    .expect("insert later business target outside frozen wave");
    let error = sqlx::query(
        r#"INSERT INTO stage_asset_wave_items(
               wave_id,target_id,asset_value,asset_type,source
           ) VALUES($1,$2,'late.example','domain','forged')"#,
    )
    .bind(wave.wave_id)
    .bind(late_target_id)
    .execute(db.pool())
    .await
    .expect_err("bound wave source cannot accept late members");
    assert_database_rejection(&error, "23514", "tool_truth_bound_wave_source_immutable");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn stage_team_unit_denominator_excludes_targets_created_after_stage_start() {
    let (mut db, _data_dir) = fixture("unit_cutoff").await;
    let wave = seed_wave_denominator_fixture(db.pool(), "unit-cutoff", &["before.example"]).await;
    sqlx::query(
        "UPDATE stage_runs SET started_at=statement_timestamp()+INTERVAL '1 second' WHERE id=$1",
    )
    .bind(wave.frozen.stage_execution_id)
    .execute(db.pool())
    .await
    .expect("move fixture stage start after the first target");
    sqlx::query(
        r#"INSERT INTO targets(
               id,name,target_type,value,scope,project_path,organization_id,source,created_at
           ) VALUES($1,'late.example','domain','late.example','in',$2,$3,'tool_truth_fixture',
                    (SELECT started_at+INTERVAL '1 second' FROM stage_runs WHERE id=$4))"#,
    )
    .bind(Uuid::new_v4())
    .bind(&wave.frozen.project_path)
    .bind(wave.frozen.organization_id)
    .bind(wave.frozen.stage_execution_id)
    .execute(db.pool())
    .await
    .expect("insert target after stage cutoff");
    let denominator = capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            stage_execution_id: wave.frozen.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageTeamUnit(
                wave.frozen.stage_run_unit_id,
            ),
        },
        compile_test_denominator,
    )
    .await
    .expect("seal unit denominator from stage-start census");
    let assets: Vec<String> = sqlx::query_scalar(
        "SELECT exact_asset FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY ordinal",
    )
    .bind(denominator.id)
    .fetch_all(db.pool())
    .await
    .expect("read unit denominator assets");
    assert_eq!(assets, vec!["before.example", "before.example"]);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn dynamic_child_manifest_distinguishes_sealed_empty_from_missing_and_rejects_late_members() {
    let (mut db, _data_dir) = fixture("child_manifest_empty").await;
    let wave = seed_wave_denominator_fixture(db.pool(), "child-manifest", &["a.example"]).await;
    let denominator = capability_execution_receipts::seal_source_denominator(
        db.pool(),
        &capability_execution_receipts::SealSourceDenominator {
            stable_seal_request_id: Uuid::new_v4(),
            stage_execution_id: wave.frozen.stage_execution_id,
            source: capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                wave.wave_id,
            ),
        },
        compile_test_denominator,
    )
    .await
    .expect("seal root denominator");
    let receipt = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "enum.directory".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin parent receipt for one capability subset");
    let parent_item_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM coverage_denominator_items WHERE denominator_id=$1 AND expected_capability='enum.directory'",
    )
    .bind(denominator.id)
    .fetch_one(db.pool())
    .await
    .expect("read exact parent denominator item");
    let manifest_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO capability_discovered_child_manifests(
               id,execution_authority_id,parent_receipt_id,parent_receipt_authority_hash,
               parent_denominator_id,parent_denominator_item_id,child_kind,
               capability_contract_version,capability_contract_hash,
               expected_downstream_technique,expected_downstream_capability,manifest_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'script','enumeration.directory.v1',$7,
                    'GOLISH-ENUM-JS','enum.javascript',$8)"#,
    )
    .bind(manifest_id)
    .bind(denominator.execution_authority_id)
    .bind(receipt.id)
    .bind(&receipt.receipt_authority_hash)
    .bind(denominator.id)
    .bind(parent_item_id)
    .bind(digest_v1('4'))
    .bind(digest_v1('5'))
    .execute(db.pool())
    .await
    .expect("open explicit child manifest");
    sqlx::query(
        "UPDATE capability_discovered_child_manifests SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(manifest_id)
    .execute(db.pool())
    .await
    .expect("server seals explicit empty child manifest");
    let sealed: (i64, bool) = sqlx::query_as(
        "SELECT member_count,sealed_empty FROM capability_discovered_child_manifests WHERE id=$1",
    )
    .bind(manifest_id)
    .fetch_one(db.pool())
    .await
    .expect("read sealed-empty distinction");
    assert_eq!(sealed, (0, true));

    let error = sqlx::query(
        r#"INSERT INTO capability_discovered_child_members(
               id,manifest_id,execution_authority_id,ordinal,child_key,exact_child_asset,
               canonical_child_identity_hash,scope_classification,
               expected_downstream_technique,expected_downstream_capability,member_hash
           ) VALUES($1,$2,$3,0,'script:late','https://a.example/late.js',$4,'in_scope',
                    'GOLISH-ENUM-JS','enum.javascript',$5)"#,
    )
    .bind(Uuid::new_v4())
    .bind(manifest_id)
    .bind(denominator.execution_authority_id)
    .bind(digest_v1('6'))
    .bind(digest_v1('7'))
    .execute(db.pool())
    .await
    .expect_err("sealed empty is immutable and cannot gain a late child");
    assert_database_rejection(&error, "23514", "tool_truth_sealed_parent_immutable");
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn begin_receipt_is_idempotent_and_attempt_identity_is_denominator_scoped() {
    let (mut db, _data_dir) = fixture("repo_receipt_begin").await;
    let wave = seed_wave_denominator_fixture(db.pool(), "repo-receipt-begin", &["a.example"]).await;
    let denominator = capability_execution_receipts::seal_wave_denominator(
        db.pool(),
        &seal_wave_command(&wave, Uuid::new_v4()),
    )
    .await
    .expect("seal receipt denominator");
    let command = capability_execution_receipts::BeginCapabilityReceipt {
        id: Uuid::new_v4(),
        denominator_id: denominator.id,
        capability: "dns_enumeration".to_string(),
        attempt_ordinal: 1,
    };
    let first = capability_execution_receipts::begin(db.pool(), &command)
        .await
        .expect("begin first receipt");
    let replay = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            ..command.clone()
        },
    )
    .await
    .expect("execution-key replay returns existing receipt");
    assert_eq!(replay.id, first.id);
    assert_eq!(replay.input_manifest_hash, denominator.input_manifest_hash);
    assert_eq!(replay.coverage_gap_reason, "policy_blocked");

    let second_attempt = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            attempt_ordinal: 2,
            ..command
        },
    )
    .await
    .expect("second attempt has distinct receipt identity");
    assert_ne!(second_attempt.id, first.id);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn begin_rejects_unsealed_denominator() {
    let (mut db, _data_dir) = fixture("repo_unsealed_begin").await;
    let frozen = seed_frozen_execution(db.pool(), "repo-unsealed-begin").await;
    let (execution_authority_id, execution_authority_hash) =
        insert_host_authority(db.pool(), &frozen).await;
    let denominator_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO coverage_denominators(
               id,stable_seal_request_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,stage_execution_id,stage_kind,execution_authority_hash,
               denominator_kind,contract,input_manifest_hash,denominator_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'root','shadow_v1',$12,$13)"#,
    )
    .bind(denominator_id)
    .bind(Uuid::new_v4())
    .bind(execution_authority_id)
    .bind(frozen.operation_id)
    .bind(frozen.project_scope_id)
    .bind(&frozen.project_path)
    .bind(frozen.scope_snapshot_id)
    .bind(frozen.organization_id)
    .bind(frozen.stage_execution_id)
    .bind(frozen.stage_kind)
    .bind(execution_authority_hash)
    .bind(digest_v1('d'))
    .bind(digest_v1('e'))
    .execute(db.pool())
    .await
    .expect("insert open denominator");

    let error = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id,
            capability: "dns_enumeration".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect_err("unsealed denominator cannot be consumed");
    assert!(error
        .to_string()
        .contains("TOOL_TRUTH_DENOMINATOR_UNSEALED"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn reconciliation_failure_seals_append_only_truth_and_replays() {
    let (mut db, _data_dir) = fixture("repo_reconciliation").await;
    let wave =
        seed_wave_denominator_fixture(db.pool(), "repo-reconciliation", &["a.example"]).await;
    let denominator = capability_execution_receipts::seal_wave_denominator(
        db.pool(),
        &seal_wave_command(&wave, Uuid::new_v4()),
    )
    .await
    .expect("seal reconciliation denominator");
    let receipt = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "dns_enumeration".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin reconciliation receipt");
    let command = capability_execution_receipts::AppendReconciliationFailure {
        id: Uuid::new_v4(),
        receipt_id: receipt.id,
        expected_row_version: receipt.row_version,
        state: capability_execution_receipts::ReconciliationFailureState::Orphaned,
        reason_code: "TOOL_TRUTH_ARTIFACT_MISSING".to_string(),
    };
    let first = capability_execution_receipts::append_reconciliation_failure(db.pool(), &command)
        .await
        .expect("append and seal orphan reconciliation");
    assert_eq!(first.semantic_authority_version, 1);
    assert_eq!(first.member_count, Some(0));
    assert!(first.semantic_reconciliation_hash.is_some());
    assert!(first.sealed_at.is_some());

    let replay = capability_execution_receipts::append_reconciliation_failure(db.pool(), &command)
        .await
        .expect("response-loss replay returns same reconciliation");
    assert_eq!(replay, first);
    let current = capability_execution_receipts::get(db.pool(), receipt.id)
        .await
        .expect("read current receipt")
        .expect("receipt exists");
    assert_eq!(current.reconciliation_state, "orphaned");
    assert_eq!(current.current_semantic_authority_version, 1);

    let error = sqlx::query(
        "UPDATE capability_execution_reconciliations SET reason_code='forged' WHERE id=$1",
    )
    .bind(first.id)
    .execute(db.pool())
    .await
    .expect_err("sealed reconciliation is append-only");
    assert_database_rejection(&error, "23514", "tool_truth_sealed_parent_immutable");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn consistent_reconciliation_requires_canonical_lineage_member() {
    let (mut db, _data_dir) = fixture("consistent_reconciliation_lineage").await;
    let wave = seed_wave_denominator_fixture(
        db.pool(),
        "consistent-reconciliation-lineage",
        &["a.example"],
    )
    .await;
    let denominator = capability_execution_receipts::seal_wave_denominator(
        db.pool(),
        &seal_wave_command(&wave, Uuid::new_v4()),
    )
    .await
    .expect("seal denominator");
    let receipt = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "dns_enumeration".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin receipt");
    let reconciliation_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO capability_execution_reconciliations(
               id,receipt_id,execution_authority_id,semantic_authority_version,
               reconciliation_state
           ) VALUES($1,$2,$3,1,'pending')"#,
    )
    .bind(reconciliation_id)
    .bind(receipt.id)
    .bind(receipt.execution_authority_id)
    .execute(db.pool())
    .await
    .expect("open reconciliation");
    let error = sqlx::query(
        r#"UPDATE capability_execution_reconciliations
              SET reconciliation_state='consistent',sealed_at=statement_timestamp()
            WHERE id=$1"#,
    )
    .bind(reconciliation_id)
    .execute(db.pool())
    .await
    .expect_err("consistent state without canonical lineage must be rejected");
    assert_database_rejection(
        &error,
        "23514",
        "tool_truth_consistent_reconciliation_requires_lineage",
    );
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn direct_sql_rejects_receipt_rewrite_late_lineage_and_budget_mutation() {
    let (mut db, _data_dir) = fixture("repo_direct_guards").await;
    let wave = seed_wave_denominator_fixture(db.pool(), "repo-direct-guards", &["a.example"]).await;
    let denominator = capability_execution_receipts::seal_wave_denominator(
        db.pool(),
        &seal_wave_command(&wave, Uuid::new_v4()),
    )
    .await
    .expect("seal guarded denominator");
    let receipt = capability_execution_receipts::begin(
        db.pool(),
        &capability_execution_receipts::BeginCapabilityReceipt {
            id: Uuid::new_v4(),
            denominator_id: denominator.id,
            capability: "dns_enumeration".to_string(),
            attempt_ordinal: 1,
        },
    )
    .await
    .expect("begin guarded receipt");

    let authority_error = sqlx::query(
        "UPDATE capability_execution_receipts SET receipt_authority_hash=$2,row_version=row_version+1 WHERE id=$1",
    )
    .bind(receipt.id)
    .bind(digest_v1('f'))
    .execute(db.pool())
    .await
    .expect_err("receipt authority fields are immutable");
    assert_database_rejection(
        &authority_error,
        "23514",
        "tool_truth_receipt_authority_immutable",
    );

    let cas_error =
        sqlx::query("UPDATE capability_execution_receipts SET typed_landing=$2 WHERE id=$1")
            .bind(receipt.id)
            .bind(serde_json::json!({"forged": true}))
            .execute(db.pool())
            .await
            .expect_err("receipt lifecycle mutation requires row-version CAS");
    assert_database_rejection(&cas_error, "23514", "tool_truth_receipt_cas_required");

    let (denominator_item_id, input_key): (Uuid, String) = sqlx::query_as(
        "SELECT id,input_key FROM coverage_denominator_items WHERE denominator_id=$1",
    )
    .bind(denominator.id)
    .fetch_one(db.pool())
    .await
    .expect("load exact denominator input");
    let input_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO capability_execution_receipt_inputs(
               id,receipt_id,denominator_id,denominator_item_id,execution_authority_id,input_key,
               attempt_state,landing_state,observation_state,coverage_extent,coverage_gap_reason
           ) VALUES($1,$2,$3,$4,$5,$6,'outcome_unknown','failed','indeterminate','none','source_unavailable')"#,
    )
    .bind(input_id)
    .bind(receipt.id)
    .bind(denominator.id)
    .bind(denominator_item_id)
    .bind(receipt.execution_authority_id)
    .bind(input_key)
    .execute(db.pool())
    .await
    .expect("insert open input closeout");
    sqlx::query(
        "UPDATE capability_execution_receipt_inputs SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(input_id)
    .execute(db.pool())
    .await
    .expect("seal exact empty input lineage");
    let late_lineage_error = sqlx::query(
        r#"INSERT INTO capability_execution_input_evidence_members(
               id,input_id,receipt_id,denominator_item_id,execution_authority_id,
               evidence_authority_id,ordinal,member_hash
           ) VALUES($1,$2,$3,$4,$5,$6,0,$7)"#,
    )
    .bind(Uuid::new_v4())
    .bind(input_id)
    .bind(receipt.id)
    .bind(denominator_item_id)
    .bind(receipt.execution_authority_id)
    .bind(Uuid::new_v4())
    .bind(digest_v1('1'))
    .execute(db.pool())
    .await
    .expect_err("sealed input rejects late lineage before foreign-key resolution");
    assert_database_rejection(
        &late_lineage_error,
        "23514",
        "tool_truth_sealed_parent_immutable",
    );

    sqlx::query(
        r#"INSERT INTO capability_execution_budget_contract_axes(
               receipt_id,execution_authority_id,axis,required_for_complete,
               planned_limit,required_observation_source
           ) VALUES($1,$2,'requests',TRUE,10,'host_governor')"#,
    )
    .bind(receipt.id)
    .bind(receipt.execution_authority_id)
    .execute(db.pool())
    .await
    .expect("insert immutable budget contract axis");
    sqlx::query(
        r#"INSERT INTO capability_execution_budget_observations(
               receipt_id,execution_authority_id,axis,actual_value,observed,observation_source
           ) VALUES($1,$2,'requests',1,TRUE,'host_governor')"#,
    )
    .bind(receipt.id)
    .bind(receipt.execution_authority_id)
    .execute(db.pool())
    .await
    .expect("insert immutable budget observation");
    let budget_error = sqlx::query(
        "UPDATE capability_execution_budget_observations SET actual_value=2 WHERE receipt_id=$1 AND axis='requests'",
    )
    .bind(receipt.id)
    .execute(db.pool())
    .await
    .expect_err("budget actual truth is append-only");
    assert_database_rejection(&budget_error, "23514", "tool_truth_append_only");

    db.stop().await;
}
