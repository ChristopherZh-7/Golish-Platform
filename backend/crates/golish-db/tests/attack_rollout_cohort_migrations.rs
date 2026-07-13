use golish_core::AttackExecutionContract;
use golish_db::{repo, DbConfig, GolishDb};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read local postgres port")
        .port()
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => serde_json::to_string(value).unwrap(),
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
                        serde_json::to_string(key).unwrap(),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_json(value: &serde_json::Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct AdmittedWaveFixture {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    candidate_stage_execution_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    sibling_organization_id: Option<Uuid>,
    sibling_wave_unit_id: Option<Uuid>,
}

impl AdmittedWaveFixture {
    fn sibling(self) -> Self {
        Self {
            organization_id: self
                .sibling_organization_id
                .expect("fixture must include a sibling organization"),
            wave_unit_id: self
                .sibling_wave_unit_id
                .expect("fixture must include a sibling WaveUnit"),
            sibling_organization_id: None,
            sibling_wave_unit_id: None,
            ..self
        }
    }

    fn at_wave(
        self,
        wave_run_id: Uuid,
        wave_unit_id: Uuid,
        organization_id: Uuid,
        candidate_stage_execution_id: Uuid,
    ) -> Self {
        Self {
            wave_run_id,
            wave_unit_id,
            organization_id,
            candidate_stage_execution_id,
            sibling_organization_id: None,
            sibling_wave_unit_id: None,
            ..self
        }
    }
}

async fn seed_admitted_wave(pool: &PgPool) -> AdmittedWaveFixture {
    seed_admitted_wave_shape(pool, false, "dual_write_read_legacy").await
}

async fn seed_admitted_wave_with_sibling(pool: &PgPool) -> AdmittedWaveFixture {
    seed_admitted_wave_shape(pool, true, "dual_write_read_legacy").await
}

async fn seed_admitted_wave_shape(
    pool: &PgPool,
    include_sibling: bool,
    attack_contract: &str,
) -> AdmittedWaveFixture {
    let session_id = Uuid::new_v4();
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let scope_decision_id = Uuid::new_v4();
    let scope_snapshot_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let sibling_organization_id = include_sibling.then(Uuid::new_v4);
    let vuln_stage_execution_id = Uuid::new_v4();
    let candidate_stage_execution_id = Uuid::new_v4();
    let vuln_stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{operation_id}:candidate-wave:0").as_bytes(),
    );
    let wave_unit_id = Uuid::new_v4();
    let sibling_wave_unit_id = sibling_organization_id.map(|_| Uuid::new_v4());

    sqlx::query(
        "INSERT INTO sessions(id,title,status,project_path) \
         VALUES($1,'rollout cohort','running','/tmp/rollout-cohort')",
    )
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert cohort session");
    sqlx::query(
        "INSERT INTO tasks(id,session_id,title,input,status) \
         VALUES($1,$2,'rollout operation','candidate cohort','running')",
    )
    .bind(operation_id)
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert cohort task");
    sqlx::query(
        r#"INSERT INTO project_scopes(
               project_scope_id,canonical_project_path,path_sha256
           ) VALUES($1,'/tmp/rollout-cohort','sha256:rollout-cohort')"#,
    )
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("insert cohort project scope");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,project_scope_id
           ) VALUES(
               $1,'red_team','attack_candidate','v2_only',
               $2,$3
           )"#,
    )
    .bind(operation_id)
    .bind(attack_contract)
    .bind(project_scope_id)
    .execute(pool)
    .await
    .expect("freeze cohort operation contracts");
    sqlx::query(
        "INSERT INTO organizations(id,project_path,name) \
         VALUES($1,'/tmp/rollout-cohort','Cohort Org')",
    )
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert cohort organization");
    if let Some(sibling_organization_id) = sibling_organization_id {
        sqlx::query(
            "INSERT INTO organizations(id,project_path,name) \
             VALUES($1,'/tmp/rollout-cohort','Cohort Sibling Org')",
        )
        .bind(sibling_organization_id)
        .execute(pool)
        .await
        .expect("insert cohort sibling organization");
    }
    sqlx::query(
        r#"INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES
               ($1,$3,'vuln_triage','started'),
               ($2,$3,'attack_candidate','started')"#,
    )
    .bind(vuln_stage_execution_id)
    .bind(candidate_stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert cohort stage executions");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES(
               $1,$2,$3,$4,$5,'cli_flags',$6,'sha256:cohort-decision'
           )"#,
    )
    .bind(scope_decision_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(candidate_stage_execution_id)
    .bind(organization_id)
    .bind(
        if let Some(sibling_organization_id) = sibling_organization_id {
            serde_json::json!([
                {"organization_id": organization_id},
                {"organization_id": sibling_organization_id}
            ])
        } else {
            serde_json::json!([{"organization_id": organization_id}])
        },
    )
    .execute(pool)
    .await
    .expect("insert cohort scope decision");
    let mut scope_tx = pool.begin().await.expect("begin cohort scope freeze");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES(
               $1,$2,$3,$4,'/tmp/rollout-cohort',$5,
               'cli_flags','sha256:cohort-scope'
           )"#,
    )
    .bind(scope_snapshot_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(scope_decision_id)
    .bind(organization_id)
    .execute(&mut *scope_tx)
    .await
    .expect("insert cohort scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Cohort Org','root',0,0,'root',$3)"#,
    )
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *scope_tx)
    .await
    .expect("insert cohort scope unit");
    if let Some(sibling_organization_id) = sibling_organization_id {
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units(
                   snapshot_id,organization_id,parent_organization_id,
                   organization_name_at_freeze,role,depth,ordinal,
                   ownership_percent,decision_row_id,approval_source
               ) VALUES(
                   $1,$2,$3,'Cohort Sibling Org','subsidiary',1,1,
                   100,'sibling',$4
               )"#,
        )
        .bind(scope_snapshot_id)
        .bind(sibling_organization_id)
        .bind(organization_id)
        .bind(serde_json::json!({"source": "cli_flags"}))
        .execute(&mut *scope_tx)
        .await
        .expect("insert cohort sibling scope unit");
    }
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(scope_snapshot_id)
        .execute(&mut *scope_tx)
        .await
        .expect("seal cohort scope");
    scope_tx.commit().await.expect("commit cohort scope freeze");

    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               started_at,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,'vuln_triage',0,'formulaic_scanner',
               'passed',NOW(),NOW()
           )"#,
    )
    .bind(vuln_stage_run_unit_id)
    .bind(operation_id)
    .bind(vuln_stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .execute(pool)
    .await
    .expect("insert final-passed cohort predecessor Unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES(
               $1,$2,$3,$4,$5,0,'formulaic_scanner','stage_unit','cohort',
               'main>vuln_triage','running',$6,'cohort-fixture',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),0
           )"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(vuln_stage_execution_id)
    .bind(vuln_stage_run_unit_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert cohort predecessor WorkerRun");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,'cohort-submit',$2,$3,'primary','submit_stage_deliverable',
               '{}','{}','finished',$3,$4,$5,$6,$7,0,$8
           )"#,
    )
    .bind(tool_call_id)
    .bind(session_id)
    .bind(operation_id)
    .bind(vuln_stage_execution_id)
    .bind(vuln_stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert cohort predecessor tool call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               worker_run_id,organization_id,tool_call_record_id,
               tool_request_id,stage_kind,attempt_epoch,lease_token,
               payload,payload_sha256
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,'cohort-submit','vuln_triage',
               0,$8,'{}','sha256:cohort-submission'
           )"#,
    )
    .bind(submission_id)
    .bind(operation_id)
    .bind(vuln_stage_execution_id)
    .bind(vuln_stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(tool_call_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert cohort predecessor submission");
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,
               from_stage_kind,stage_execution_id,source_stage_run_unit_id,
               deliverable_submission_id,scope_hash,payload,payload_sha256,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES(
               $1,$2,$3,$4,'vuln_triage',$5,$6,$7,
               'sha256:cohort-scope','{}','sha256:cohort-handoff',
               'sha256:cohort-gate',NOW()
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(scope_snapshot_id)
    .bind(vuln_stage_execution_id)
    .bind(vuln_stage_run_unit_id)
    .bind(submission_id)
    .execute(pool)
    .await
    .expect("insert cohort predecessor handoff");
    let sibling_entry = if let Some(sibling_organization_id) = sibling_organization_id {
        Some(
            seed_additional_vuln_handoff(
                pool,
                session_id,
                operation_id,
                vuln_stage_execution_id,
                scope_snapshot_id,
                sibling_organization_id,
                0,
            )
            .await,
        )
    } else {
        None
    };
    sqlx::query(
        r#"INSERT INTO attack_wave_runs(
               id,operation_id,scope_snapshot_id,generation,status,
               policy_snapshot,policy_hash,max_waves,max_candidates_total,
               max_chain_depth,max_attempts_total
           ) VALUES(
               $1,$2,$3,0,'open',$4,
               'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326',
               3,100,3,200
           )"#,
    )
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(serde_json::json!({
        "max_waves": 3,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_attempts_total": 200
    }))
    .execute(pool)
    .await
    .expect("insert cohort generation-zero Wave");
    sqlx::query(
        r#"INSERT INTO attack_wave_units(
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_stage_execution_id,entry_stage_run_unit_id,
               entry_deliverable_submission_id,entry_stage_kind,ordinal,status
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,'open'
           )"#,
    )
    .bind(wave_unit_id)
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(vuln_stage_execution_id)
    .bind(vuln_stage_run_unit_id)
    .bind(submission_id)
    .execute(pool)
    .await
    .expect("insert first cohort WaveUnit");
    if let (
        Some(sibling_organization_id),
        Some(sibling_wave_unit_id),
        Some((sibling_stage_run_unit_id, sibling_submission_id)),
    ) = (sibling_organization_id, sibling_wave_unit_id, sibling_entry)
    {
        sqlx::query(
            r#"INSERT INTO attack_wave_units(
                   id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
                   entry_stage_execution_id,entry_stage_run_unit_id,
                   entry_deliverable_submission_id,entry_stage_kind,ordinal,status
               ) VALUES(
                   $1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',1,'open'
               )"#,
        )
        .bind(sibling_wave_unit_id)
        .bind(wave_run_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(sibling_organization_id)
        .bind(vuln_stage_execution_id)
        .bind(sibling_stage_run_unit_id)
        .bind(sibling_submission_id)
        .execute(pool)
        .await
        .expect("insert sibling cohort WaveUnit");
    }

    AdmittedWaveFixture {
        operation_id,
        scope_snapshot_id,
        organization_id,
        candidate_stage_execution_id,
        wave_run_id,
        wave_unit_id,
        sibling_organization_id,
        sibling_wave_unit_id,
    }
}

async fn seed_additional_vuln_handoff(
    pool: &PgPool,
    session_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    generation: i32,
) -> (Uuid, Uuid) {
    let stage_run_unit_id = Uuid::new_v4();
    let worker_run_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    let tool_call_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let request_id = format!("cohort-submit-{organization_id}-{generation}");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               started_at,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,'vuln_triage',$6,'formulaic_scanner',
               'passed',NOW(),NOW()
           )"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(generation)
    .execute(pool)
    .await
    .expect("insert additional cohort predecessor Unit");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES(
               $1,$2,$3,$4,$5,$6,'formulaic_scanner','stage_unit',$7,
               'main>vuln_triage','running',$8,'cohort-fixture',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),0
           )"#,
    )
    .bind(worker_run_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(organization_id)
    .bind(generation)
    .bind(&request_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert additional cohort predecessor WorkerRun");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,$2,$3,$4,'primary','submit_stage_deliverable',
               '{}','{}','finished',$4,$5,$6,$7,$8,0,$9
           )"#,
    )
    .bind(tool_call_id)
    .bind(&request_id)
    .bind(session_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert additional cohort predecessor tool call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               worker_run_id,organization_id,tool_call_record_id,
               tool_request_id,stage_kind,attempt_epoch,lease_token,
               payload,payload_sha256
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',
               0,$9,'{}','sha256:cohort-submission'
           )"#,
    )
    .bind(submission_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(worker_run_id)
    .bind(organization_id)
    .bind(tool_call_id)
    .bind(&request_id)
    .bind(lease_token)
    .execute(pool)
    .await
    .expect("insert additional cohort predecessor submission");
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,
               from_stage_kind,stage_execution_id,source_stage_run_unit_id,
               deliverable_submission_id,scope_hash,payload,payload_sha256,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES(
               $1,$2,$3,$4,'vuln_triage',$5,$6,$7,
               'sha256:cohort-scope','{}','sha256:cohort-handoff',
               'sha256:cohort-gate',NOW()
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(operation_id)
    .bind(organization_id)
    .bind(scope_snapshot_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(submission_id)
    .execute(pool)
    .await
    .expect("insert additional cohort predecessor handoff");
    (stage_run_unit_id, submission_id)
}

async fn seed_candidate_stage_execution(pool: &PgPool, operation_id: Uuid) -> Uuid {
    let stage_execution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'attack_candidate','started')",
    )
    .bind(stage_execution_id)
    .bind(operation_id)
    .execute(pool)
    .await
    .expect("insert follow-on Candidate stage execution");
    stage_execution_id
}

async fn seal_with_matching_no_candidate_shadow(
    pool: &PgPool,
    fixture: AdmittedWaveFixture,
    generation: i32,
) -> (Uuid, serde_json::Value, String) {
    let candidate_stage_run_unit_id = Uuid::new_v4();
    let candidate_worker_run_id = Uuid::new_v4();
    let candidate_lease_token = Uuid::new_v4();
    let candidate_tool_call_id = Uuid::new_v4();
    let candidate_submission_id = Uuid::new_v4();
    let seed_id = Uuid::new_v4();
    let work_item_id = Uuid::new_v4();
    let reason_code = "no_safe_candidate";
    let detail = "no authorized active action remains";
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status,
               started_at,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,'attack_candidate',$6,'attack_analyst',
               'passed',NOW(),NOW()
           )"#,
    )
    .bind(candidate_stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.candidate_stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(generation)
    .execute(pool)
    .await
    .expect("insert final-passed Candidate Unit");
    let session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(fixture.operation_id)
        .fetch_one(pool)
        .await
        .expect("load Candidate fixture session");
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status,lease_token,lease_owner,
               lease_acquired_at,lease_expires_at,heartbeat_at,attempt_epoch
           ) VALUES(
               $1,$2,$3,$4,$5,0,'attack_analyst','stage_unit','cohort-candidate',
               'main>attack_candidate','passed',$6,'cohort-fixture',NOW(),
               NOW()+INTERVAL '5 minutes',NOW(),0
           )"#,
    )
    .bind(candidate_worker_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.candidate_stage_execution_id)
    .bind(candidate_stage_run_unit_id)
    .bind(fixture.organization_id)
    .bind(candidate_lease_token)
    .execute(pool)
    .await
    .expect("insert final Candidate WorkerRun");
    let candidate_request_id = format!("cohort-candidate-submit:{candidate_stage_run_unit_id}");
    sqlx::query(
        r#"INSERT INTO tool_calls(
               id,call_id,session_id,task_id,agent,name,args,result,status,
               operation_id,stage_execution_id,stage_run_unit_id,worker_run_id,
               organization_id,attempt_epoch,lease_token
           ) VALUES(
               $1,$2,$3,$4,'primary','submit_stage_deliverable',
               '{}','{}','finished',$4,$5,$6,$7,$8,0,$9
           )"#,
    )
    .bind(candidate_tool_call_id)
    .bind(&candidate_request_id)
    .bind(session_id)
    .bind(fixture.operation_id)
    .bind(fixture.candidate_stage_execution_id)
    .bind(candidate_stage_run_unit_id)
    .bind(candidate_worker_run_id)
    .bind(fixture.organization_id)
    .bind(candidate_lease_token)
    .execute(pool)
    .await
    .expect("insert final Candidate submit tool call");
    sqlx::query(
        r#"INSERT INTO stage_deliverable_submissions(
               id,operation_id,stage_execution_id,stage_run_unit_id,
               worker_run_id,organization_id,tool_call_record_id,
               tool_request_id,stage_kind,attempt_epoch,lease_token,
               payload,payload_sha256
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,'attack_candidate',0,$9,
               '{}','sha256:cohort-candidate-submission'
           )"#,
    )
    .bind(candidate_submission_id)
    .bind(fixture.operation_id)
    .bind(fixture.candidate_stage_execution_id)
    .bind(candidate_stage_run_unit_id)
    .bind(candidate_worker_run_id)
    .bind(fixture.organization_id)
    .bind(candidate_tool_call_id)
    .bind(&candidate_request_id)
    .bind(candidate_lease_token)
    .execute(pool)
    .await
    .expect("insert exact Candidate submission");
    sqlx::query(
        r#"INSERT INTO stage_handoffs(
               id,operation_id,organization_id,scope_snapshot_id,
               from_stage_kind,stage_execution_id,source_stage_run_unit_id,
               deliverable_submission_id,scope_hash,payload,payload_sha256,
               unit_gate_decision_hash,gate_passed_at
           ) VALUES(
               $1,$2,$3,$4,'attack_candidate',$5,$6,$7,
               'sha256:cohort-scope','{}','sha256:cohort-candidate-handoff',
               'sha256:cohort-candidate-gate',NOW()
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(fixture.organization_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.candidate_stage_execution_id)
    .bind(candidate_stage_run_unit_id)
    .bind(candidate_submission_id)
    .execute(pool)
    .await
    .expect("insert exact Candidate final handoff");
    sqlx::query(
        r#"INSERT INTO attack_candidate_seeds(
               id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
               target_type_at_time,target_value_at_time,target_identity_hash,
               technique,observation,observation_hash
           ) VALUES(
               $1,$2,$3,$4,$5,'url','https://cohort.example.test',
               'sha256:cohort-target','manual-review','{}','sha256:observation'
           )"#,
    )
    .bind(seed_id)
    .bind(fixture.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .execute(pool)
    .await
    .expect("insert canonical Candidate seed");
    sqlx::query(
        r#"INSERT INTO attack_candidate_work_items(
               id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,
               organization_id,target_type_at_time,target_value_at_time,
               target_identity_hash,work_item_key,decision_kind,
               no_candidate_reason_code,no_candidate_detail,decided_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,'url','https://cohort.example.test',
               'sha256:cohort-target','cohort:no-candidate','no_candidate',
               $7,$8,NOW()
           )"#,
    )
    .bind(work_item_id)
    .bind(seed_id)
    .bind(fixture.wave_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(reason_code)
    .bind(detail)
    .execute(pool)
    .await
    .expect("insert canonical no-candidate decision");
    let project_path: String = sqlx::query_scalar(
        "SELECT project_path_at_freeze FROM operation_org_scope_snapshots WHERE id=$1",
    )
    .bind(fixture.scope_snapshot_id)
    .fetch_one(pool)
    .await
    .expect("load frozen project path for Candidate decision evidence");
    let decision_evidence_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,audit_role,run_id,target_id,detail
           ) VALUES(
               'cohort no-candidate decision','attack','',$1,'evidence',$2,NULL,$3
           ) RETURNING id"#,
    )
    .bind(project_path)
    .bind(fixture.operation_id)
    .bind(serde_json::json!({"organization_id": fixture.organization_id}))
    .fetch_one(pool)
    .await
    .expect("insert exact no-candidate decision evidence");
    sqlx::query(
        "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
         VALUES($1,$2,'decision')",
    )
    .bind(work_item_id)
    .bind(decision_evidence_id)
    .execute(pool)
    .await
    .expect("attach exact no-candidate decision evidence");
    let manifest_projection: serde_json::Value = sqlx::query_scalar(
        r#"SELECT COALESCE(jsonb_agg(
                   jsonb_build_object(
                       'evidence_ids',item_source.evidence_ids,
                       'target_identity_hash',item_source.target_identity_hash,
                       'technique',item_source.technique,
                       'work_item_id',item_source.work_item_id,
                       'work_item_key',item_source.work_item_key
                   ) ORDER BY item_source.work_item_key,item_source.work_item_id
               ),'[]'::jsonb)
             FROM (
                 SELECT item.id AS work_item_id,item.work_item_key,
                        item.target_identity_hash,seed.technique,
                        COALESCE((
                            SELECT jsonb_agg(source.evidence_id ORDER BY source.evidence_id)
                              FROM (
                                  SELECT evidence_id FROM attack_candidate_seed_evidence
                                   WHERE seed_id=item.seed_id
                                  UNION
                                  SELECT evidence_id FROM attack_candidate_work_item_evidence
                                   WHERE work_item_id=item.id
                                     AND role IN ('observation','support')
                              ) AS source
                        ),'[]'::jsonb) AS evidence_ids
                   FROM attack_candidate_work_items AS item
                   JOIN attack_candidate_seeds AS seed ON seed.id=item.seed_id
                  WHERE item.operation_id=$1 AND item.wave_unit_id=$2
                    AND item.organization_id=$3
             ) AS item_source"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.wave_unit_id)
    .bind(fixture.organization_id)
    .fetch_one(pool)
    .await
    .expect("rebuild exact cohort Candidate manifest");
    let manifest_hash = format!("sha256:{}", sha256_json(&manifest_projection));
    sqlx::query(
        r#"UPDATE attack_wave_units
              SET manifest_hash=$2,manifest_count=1,
                  manifest_frozen_at=NOW(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(fixture.wave_unit_id)
    .bind(manifest_hash)
    .execute(pool)
    .await
    .expect("freeze single-item Candidate manifest");

    let semantic_hash = sha256_json(&serde_json::json!({
        "work_item_id": work_item_id,
        "reason_code": reason_code,
        "detail": detail,
        "evidence_ids": [decision_evidence_id],
    }));
    let legacy_record = serde_json::json!({
        "decisions": [{
            "work_item_key": "cohort:no-candidate",
            "kind": "no_candidate",
            "semantic_hash": semantic_hash,
        }],
        "review_counts": {
            "wave_unit_count": 1,
            "review_closed_unit_count": 0,
            "candidate_decision_count": 0,
            "no_candidate_decision_count": 1,
        }
    });
    let record_hash = sha256_json(&legacy_record);
    sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads(
               stage_run_unit_id,operation_id,stage_execution_id,
               organization_id,attack_execution_contract,legacy_record,
               legacy_record_hash
           ) VALUES(
               $1,$2,$3,$4,'dual_write_read_legacy',$5,$6
           )"#,
    )
    .bind(candidate_stage_run_unit_id)
    .bind(fixture.operation_id)
    .bind(fixture.candidate_stage_execution_id)
    .bind(fixture.organization_id)
    .bind(&legacy_record)
    .bind(&record_hash)
    .execute(pool)
    .await
    .expect("insert and DB-seal matching legacy Candidate mirror");
    (candidate_stage_run_unit_id, legacy_record, record_hash)
}

async fn terminalize_wave_authority(pool: &PgPool, fixture: AdmittedWaveFixture) {
    sqlx::query(
        r#"UPDATE attack_wave_units
              SET status='terminal',review_closed=TRUE,verification_closed=TRUE,
                  consolidation_status='terminal',terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(fixture.wave_unit_id)
    .execute(pool)
    .await
    .expect("terminalize admitted WaveUnit");
    sqlx::query(
        r#"UPDATE attack_wave_runs
              SET status='terminal',terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(fixture.wave_run_id)
    .execute(pool)
    .await
    .expect("terminalize admitted Wave");
}

async fn close_with_matching_no_candidate_shadow(
    pool: &PgPool,
    fixture: AdmittedWaveFixture,
) -> (Uuid, serde_json::Value, String) {
    let result = seal_with_matching_no_candidate_shadow(pool, fixture, 0).await;
    terminalize_wave_authority(pool, fixture).await;
    result
}

#[test]
fn attack_v2_writers_require_a_runtime_contract_that_writes_v2() {
    for attack_contract in [
        AttackExecutionContract::DualWriteReadLegacy,
        AttackExecutionContract::DualWriteReadV2Fallback,
    ] {
        let error =
            repo::operation_state::validate_operation_contracts("legacy_v1", attack_contract)
                .expect_err("a dual attack writer cannot run on legacy-only runtime memory");
        assert_eq!(error.code(), "ATTACK_RUNTIME_MEMORY_V2_WRITER_REQUIRED");
    }

    repo::operation_state::validate_operation_contracts(
        "dual_write_legacy_read",
        AttackExecutionContract::DualWriteReadLegacy,
    )
    .expect("dual-write runtime memory can retain a dual attack semantic mirror");
    repo::operation_state::validate_operation_contracts("v2_only", AttackExecutionContract::V2Only)
        .expect("attack v2_only remains compatible with runtime v2_only");
}

#[tokio::test]
#[serial]
async fn rollout_schema_owns_candidate_admission_and_raw_transition_gate() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_cohort_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");

    let rollout: (String, i16, i64) = sqlx::query_as(
        "SELECT contract,rank,row_version FROM attack_execution_rollout WHERE singleton=TRUE",
    )
    .fetch_one(db.pool())
    .await
    .expect("read attack rollout");
    assert_eq!(
        rollout,
        ("dual_write_read_legacy".to_string(), 1, 1),
        "startup must enable sampling without bypassing the cohort gate"
    );

    let admission_sequence: String = sqlx::query_scalar(
        r#"SELECT pg_get_serial_sequence(
               'attack_execution_candidate_admissions',
               'admission_seq'
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("Candidate admission authority must own a monotonic sequence");
    assert!(
        admission_sequence.ends_with("attack_execution_candidate_admissions_admission_seq_seq"),
        "unexpected Candidate admission sequence: {admission_sequence}"
    );

    let admission_trigger: String = sqlx::query_scalar(
        r#"SELECT pg_get_functiondef(
               'admit_attack_execution_candidate_operation()'::regprocedure
           )"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect Candidate admission trigger");
    for required in [
        "FOR SHARE",
        "attack_execution_rollout",
        "attack_execution_candidate_admissions",
        "generation <> 0",
    ] {
        assert!(
            admission_trigger.contains(required),
            "Candidate admission trigger is missing {required}"
        );
    }

    let raw_transition = sqlx::query(
        r#"UPDATE attack_execution_rollout
              SET contract='dual_write_read_v2_fallback',rank=2,
                  row_version=row_version+1,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await;
    let error = raw_transition.expect_err("zero-sample raw rollout transition must be gated");
    let code = match &error {
        sqlx::Error::Database(database_error) => {
            database_error.code().map(|code| code.into_owned())
        }
        _ => None,
    };
    assert_eq!(code.as_deref(), Some("55000"));
    assert!(
        error
            .to_string()
            .contains("ATTACK_ROLLOUT_COHORT_NOT_READY"),
        "raw transition should expose the typed not-ready reason: {error}"
    );

    let rollout_gate: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('enforce_attack_execution_rollout_transition()'::regprocedure)",
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect DB-owned rollout gate");
    for required in [
        "MAX(admission.admission_seq)",
        "attack_execution_candidate_cohort_gate",
    ] {
        assert!(
            rollout_gate.contains(required),
            "rollout gate is missing {required}"
        );
    }
    let receipt_gate: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('reject_direct_attack_execution_rollout_promotion_receipt()'::regprocedure)",
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect promotion receipt recomputation");
    for required in [
        "attack_execution_candidate_cohort_gate",
        "NEW.from_rank := derived_from_rank",
        "NEW.admission_cutoff := current_cutoff",
        "rollout.rank NOT IN (2, 3)",
    ] {
        assert!(
            receipt_gate.contains(required),
            "promotion receipt authority is missing {required}"
        );
    }
    let receipt_owner: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('record_attack_execution_rollout_promotion_receipt()'::regprocedure)",
    )
    .fetch_one(db.pool())
    .await
    .expect("inspect promotion receipt owner trigger");
    assert!(receipt_owner.contains("attack_execution_rollout_promotions"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn v2_only_generation_zero_wave_unit_skips_dual_candidate_admission() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_v2_only_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    sqlx::raw_sql(
        r#"ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               DISABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
           UPDATE runtime_memory_rollout
              SET contract='v2_only',contract_rank=3,row_version=3,updated_at=NOW()
            WHERE singleton_id=1;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER runtime_memory_rollout_forward_only;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_attestation_gate;
           ALTER TABLE runtime_memory_rollout
               ENABLE TRIGGER zz_runtime_memory_rollout_promotion_receipt;
           ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER attack_execution_rollout_forward_only;
           ALTER TABLE attack_execution_rollout
               DISABLE TRIGGER zz_attack_execution_rollout_promotion_receipt;
           UPDATE attack_execution_rollout
              SET contract='v2_only',rank=3,row_version=3,updated_at=NOW()
            WHERE singleton=TRUE;
           ALTER TABLE attack_execution_rollout
               ENABLE TRIGGER attack_execution_rollout_forward_only;
           ALTER TABLE attack_execution_rollout
               ENABLE TRIGGER zz_attack_execution_rollout_promotion_receipt;"#,
    )
    .execute(db.pool())
    .await
    .expect("install exact V2-only rollout fixture");

    let fixture = seed_admitted_wave_shape(db.pool(), false, "v2_only").await;
    let admission_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_execution_candidate_admissions WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count dual Candidate admissions for V2-only operation");
    assert_eq!(admission_count, 0);
    db.stop().await;
}

#[tokio::test]
#[serial]
async fn first_wave_unit_admits_once_and_unsealed_manifest_blocks_promotion() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_admission_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave(db.pool()).await;

    let admission: (i64, Uuid, Uuid, Uuid, String, i16, i64) = sqlx::query_as(
        r#"SELECT admission_seq,operation_id,initial_wave_run_id,
                  first_wave_unit_id,attack_execution_contract,
                  rollout_rank,rollout_row_version
             FROM attack_execution_candidate_admissions
            WHERE operation_id=$1"#,
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load first Candidate admission");
    assert_eq!(admission.0, 1);
    assert_eq!(admission.1, fixture.operation_id);
    assert_eq!(admission.2, fixture.wave_run_id);
    assert_eq!(admission.3, fixture.wave_unit_id);
    assert_eq!(admission.4, "dual_write_read_legacy");
    assert_eq!((admission.5, admission.6), (1, 1));

    let gate: (i64, i64, i64, bool, String) = sqlx::query_as(
        "SELECT admission_count,candidate_unit_count,sample_count,ready,reason \
         FROM attack_execution_candidate_cohort_gate( \
             'dual_write_read_legacy',1::SMALLINT,$1 \
         )",
    )
    .bind(admission.0)
    .fetch_one(db.pool())
    .await
    .expect("evaluate admitted open cohort");
    assert_eq!(
        gate,
        (1, 0, 0, false, "candidate_manifest_not_sealed".to_string())
    );

    let raw_transition = sqlx::query(
        r#"UPDATE attack_execution_rollout
              SET contract='dual_write_read_v2_fallback',rank=2,
                  row_version=row_version+1,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await
    .expect_err("an admitted unsealed manifest must block raw promotion");
    assert!(raw_transition
        .to_string()
        .contains("candidate_manifest_not_sealed"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn nested_triggers_cannot_update_or_delete_candidate_admissions() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!(
            "attack_rollout_admission_hostile_{}",
            Uuid::new_v4().simple()
        ),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave(db.pool()).await;

    sqlx::query(
        r#"CREATE TABLE fixture_attack_admission_tamper_driver (
               operation_id UUID NOT NULL,
               action TEXT NOT NULL
           )"#,
    )
    .execute(db.pool())
    .await
    .expect("create hostile admission driver");
    sqlx::query(
        r#"CREATE FUNCTION fixture_nested_attack_admission_tamper()
           RETURNS trigger AS $$
           BEGIN
               IF NEW.action = 'update' THEN
                   UPDATE attack_execution_candidate_admissions
                      SET rollout_row_version = rollout_row_version
                    WHERE operation_id = NEW.operation_id;
               ELSIF NEW.action = 'delete' THEN
                   DELETE FROM attack_execution_candidate_admissions
                    WHERE operation_id = NEW.operation_id;
               END IF;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("create hostile nested admission trigger function");
    sqlx::query(
        r#"CREATE TRIGGER fixture_nested_attack_admission_tamper
           BEFORE INSERT ON fixture_attack_admission_tamper_driver
           FOR EACH ROW EXECUTE FUNCTION fixture_nested_attack_admission_tamper()"#,
    )
    .execute(db.pool())
    .await
    .expect("install hostile nested admission trigger");

    let nested_update = sqlx::query(
        "INSERT INTO fixture_attack_admission_tamper_driver(operation_id,action) \
         VALUES($1,'update')",
    )
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await;
    let nested_delete = sqlx::query(
        "INSERT INTO fixture_attack_admission_tamper_driver(operation_id,action) \
         VALUES($1,'delete')",
    )
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await;
    for (action, result) in [("update", nested_update), ("delete", nested_delete)] {
        let error = result.expect_err("nested admission mutation must be rejected");
        assert!(
            error
                .to_string()
                .contains("ATTACK_CANDIDATE_ADMISSION_INTERNAL_ONLY"),
            "unexpected nested admission {action} error: {error}"
        );
    }
    let admission_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_execution_candidate_admissions WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count retained Candidate admission");
    assert_eq!(admission_count, 1);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn nested_admission_insert_rebuilds_server_owned_fields_from_wave_unit() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!(
            "attack_rollout_admission_rebuild_{}",
            Uuid::new_v4().simple()
        ),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");

    // Isolate the admission-table authority: retain the normal Wave validation
    // trigger but suppress its owner insert for this fixture only.
    sqlx::query(
        "ALTER TABLE attack_wave_units DISABLE TRIGGER \
         aa_attack_execution_candidate_unit_admission",
    )
    .execute(db.pool())
    .await
    .expect("disable Candidate admission owner for hostile fixture");
    let fixture = seed_admitted_wave(db.pool()).await;
    sqlx::query(
        "ALTER TABLE attack_wave_units ENABLE TRIGGER \
         aa_attack_execution_candidate_unit_admission",
    )
    .execute(db.pool())
    .await
    .expect("restore Candidate admission owner");

    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_execution_candidate_admissions WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("count admissions before hostile nested insert");
    assert_eq!(before, 0);

    sqlx::query(
        r#"CREATE TABLE fixture_attack_admission_insert_driver(
               id INTEGER PRIMARY KEY,
               operation_id UUID NOT NULL,
               scope_snapshot_id UUID NOT NULL,
               wave_run_id UUID NOT NULL,
               wave_unit_id UUID NOT NULL,
               organization_id UUID NOT NULL
           )"#,
    )
    .execute(db.pool())
    .await
    .expect("create hostile admission insert driver");
    sqlx::query(
        r#"CREATE FUNCTION fixture_nested_attack_admission_insert()
           RETURNS trigger AS $$
           BEGIN
               INSERT INTO attack_execution_candidate_admissions(
                   admission_seq,operation_id,scope_snapshot_id,initial_wave_run_id,
                   first_wave_unit_id,first_organization_id,attack_execution_contract,
                   rollout_rank,rollout_row_version,admitted_at
               ) VALUES(
                   424242,NEW.operation_id,NEW.scope_snapshot_id,NEW.wave_run_id,
                   NEW.wave_unit_id,NEW.organization_id,'dual_write_read_legacy',1,999,
                   '2100-01-01T00:00:00Z'
               );
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("create hostile nested admission insert function");
    sqlx::query(
        r#"CREATE TRIGGER fixture_nested_attack_admission_insert
           BEFORE INSERT ON fixture_attack_admission_insert_driver
           FOR EACH ROW EXECUTE FUNCTION fixture_nested_attack_admission_insert()"#,
    )
    .execute(db.pool())
    .await
    .expect("install hostile nested admission insert trigger");
    sqlx::query(
        r#"INSERT INTO fixture_attack_admission_insert_driver(
               id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,organization_id
           ) VALUES(1,$1,$2,$3,$4,$5)"#,
    )
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(fixture.wave_unit_id)
    .bind(fixture.organization_id)
    .execute(db.pool())
    .await
    .expect("nested insert may only create a canonical admission");

    type CanonicalAdmissionRow = (
        i64,
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        String,
        i16,
        i64,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    );
    let admission: CanonicalAdmissionRow = sqlx::query_as(
        r#"SELECT admission_seq,operation_id,scope_snapshot_id,initial_wave_run_id,
                  first_wave_unit_id,attack_execution_contract,rollout_rank,
                  rollout_row_version,admitted_at,NOW()
             FROM attack_execution_candidate_admissions
            WHERE operation_id=$1"#,
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load canonicalized nested admission");
    assert_ne!(admission.0, 424242, "caller cannot choose admission_seq");
    assert_eq!(admission.1, fixture.operation_id);
    assert_eq!(admission.2, fixture.scope_snapshot_id);
    assert_eq!(admission.3, fixture.wave_run_id);
    assert_eq!(admission.4, fixture.wave_unit_id);
    assert_eq!(admission.5, "dual_write_read_legacy");
    assert_eq!(admission.6, 1);
    assert_eq!(admission.7, 1, "rollout row version is DB authority");
    assert!(
        admission.8 <= admission.9 && admission.8 >= admission.9 - chrono::Duration::minutes(1),
        "caller-supplied admission chronology escaped normalization: admitted_at={}, server_now={}",
        admission.8,
        admission.9
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn nested_trigger_cannot_delete_retained_shadow_sample() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_shadow_hostile_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave(db.pool()).await;
    let (candidate_stage_run_unit_id, _, _) =
        seal_with_matching_no_candidate_shadow(db.pool(), fixture, 0).await;

    sqlx::query(
        "CREATE TABLE fixture_attack_shadow_tamper_driver( \
             stage_run_unit_id UUID NOT NULL \
         )",
    )
    .execute(db.pool())
    .await
    .expect("create hostile shadow driver");
    sqlx::query(
        r#"CREATE FUNCTION fixture_nested_attack_shadow_delete()
           RETURNS trigger AS $$
           BEGIN
               DELETE FROM attack_execution_shadow_reads
                WHERE stage_run_unit_id=NEW.stage_run_unit_id;
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("create hostile nested shadow delete function");
    sqlx::query(
        r#"CREATE TRIGGER fixture_nested_attack_shadow_delete
           BEFORE INSERT ON fixture_attack_shadow_tamper_driver
           FOR EACH ROW EXECUTE FUNCTION fixture_nested_attack_shadow_delete()"#,
    )
    .execute(db.pool())
    .await
    .expect("install hostile nested shadow delete trigger");
    let delete = sqlx::query(
        "INSERT INTO fixture_attack_shadow_tamper_driver(stage_run_unit_id) VALUES($1)",
    )
    .bind(candidate_stage_run_unit_id)
    .execute(db.pool())
    .await
    .expect_err("nested trigger must not erase retained rollout evidence");
    assert!(
        delete
            .to_string()
            .contains("attack execution shadow samples cannot be deleted directly"),
        "unexpected retained shadow deletion error: {delete}"
    );
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM attack_execution_shadow_reads WHERE stage_run_unit_id=$1",
    )
    .bind(candidate_stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("count retained shadow sample");
    assert_eq!(retained, 1);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn shadow_close_timestamps_are_server_owned() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_shadow_server_time_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave(db.pool()).await;
    let (candidate_stage_run_unit_id, _, _) =
        seal_with_matching_no_candidate_shadow(db.pool(), fixture, 0).await;

    let (compared_at, updated_at, server_now): (
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        r#"SELECT compared_at,updated_at,NOW()
             FROM attack_execution_shadow_reads
            WHERE stage_run_unit_id=$1"#,
    )
    .bind(candidate_stage_run_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("read server-owned shadow close timestamps");
    assert_eq!(compared_at, updated_at);
    assert!(
        compared_at <= server_now && compared_at >= server_now - chrono::Duration::minutes(1),
        "caller-supplied future attestation time escaped normalization: compared_at={compared_at}, server_now={server_now}"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_final_seal_promotes_without_review_and_old_admission_can_continue() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_reachable_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave(db.pool()).await;
    seal_with_matching_no_candidate_shadow(db.pool(), fixture, 0).await;

    let authority: (
        String,
        bool,
        bool,
        String,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        r#"SELECT status,review_closed,verification_closed,
                      consolidation_status,terminal_at
                 FROM attack_wave_units WHERE id=$1"#,
    )
    .bind(fixture.wave_unit_id)
    .fetch_one(db.pool())
    .await
    .expect("read Candidate-final authority before review");
    assert_eq!(authority.0, "open");
    assert!(!authority.1 && !authority.2);
    assert_eq!(authority.3, "pending");
    assert!(authority.4.is_none());

    sqlx::query(
        r#"UPDATE attack_execution_rollout
              SET contract='dual_write_read_v2_fallback',rank=2,
                  row_version=row_version+1,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await
    .expect("Candidate final seal must be sufficient for one-rank promotion");

    let session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(fixture.operation_id)
        .fetch_one(db.pool())
        .await
        .expect("load immutable old-operation session authority");
    let vuln_stage_execution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'vuln_triage','started')",
    )
    .bind(vuln_stage_execution_id)
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect("insert follow-on vuln predecessor stage execution");
    let (entry_stage_run_unit_id, entry_submission_id) = seed_additional_vuln_handoff(
        db.pool(),
        session_id,
        fixture.operation_id,
        vuln_stage_execution_id,
        fixture.scope_snapshot_id,
        fixture.organization_id,
        1,
    )
    .await;
    let follow_on_candidate_stage_execution_id =
        seed_candidate_stage_execution(db.pool(), fixture.operation_id).await;
    let next_wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:candidate-wave:1", fixture.operation_id).as_bytes(),
    );
    let next_wave_unit_id = Uuid::new_v4();
    // Isolate the 00016 old-admission liveness trigger.  Full follow-on
    // FactDelta provenance is covered by the 00012 integration suite.
    sqlx::query("ALTER TABLE attack_wave_runs DISABLE TRIGGER attack_wave_follow_on_policy_exact")
        .execute(db.pool())
        .await
        .expect("disable unrelated follow-on provenance fixture trigger");
    sqlx::query(
        r#"INSERT INTO attack_wave_runs(
               id,operation_id,scope_snapshot_id,generation,status,
               policy_snapshot,policy_hash,max_waves,max_candidates_total,
               max_chain_depth,max_attempts_total
           ) VALUES(
               $1,$2,$3,1,'open',$4,
               'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326',
               3,100,3,200
           )"#,
    )
    .bind(next_wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(serde_json::json!({
        "max_waves": 3,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_attempts_total": 200
    }))
    .execute(db.pool())
    .await
    .expect("old admitted operation may materialize a later Wave");
    sqlx::query("ALTER TABLE attack_wave_runs ENABLE TRIGGER attack_wave_follow_on_policy_exact")
        .execute(db.pool())
        .await
        .expect("restore follow-on provenance trigger");
    sqlx::query(
        r#"INSERT INTO attack_wave_units(
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_stage_execution_id,entry_stage_run_unit_id,
               entry_deliverable_submission_id,entry_stage_kind,ordinal,status
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',0,'open')"#,
    )
    .bind(next_wave_unit_id)
    .bind(next_wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(vuln_stage_execution_id)
    .bind(entry_stage_run_unit_id)
    .bind(entry_submission_id)
    .execute(db.pool())
    .await
    .expect("old admitted operation may materialize a later WaveUnit");
    sqlx::query(
        r#"INSERT INTO stage_run_units(
               id,operation_id,stage_execution_id,scope_snapshot_id,
               organization_id,stage_kind,generation,specialist,status
           ) VALUES(
               $1,$2,$3,$4,$5,'attack_candidate',1,'attack_analyst','queued'
           )"#,
    )
    .bind(Uuid::new_v4())
    .bind(fixture.operation_id)
    .bind(follow_on_candidate_stage_execution_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .execute(db.pool())
    .await
    .expect("an admitted old-contract operation remains runnable after default promotion");

    let project_scope_id: Uuid =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(fixture.operation_id)
            .fetch_one(db.pool())
            .await
            .expect("load project scope for stale-operation fixture");
    let stale_operation_id = Uuid::new_v4();
    let stale_stage_execution_id = Uuid::new_v4();
    let stale_scope_decision_id = Uuid::new_v4();
    let stale_scope_snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tasks(id,session_id,title,input,status) \
         VALUES($1,$2,'stale rollout operation','stale admission','running')",
    )
    .bind(stale_operation_id)
    .bind(session_id)
    .execute(db.pool())
    .await
    .expect("insert stale rollout task");
    sqlx::query(
        r#"INSERT INTO operation_state(
               operation_id,profile,current_stage,runtime_memory_contract,
               attack_execution_contract,project_scope_id
           ) VALUES(
               $1,'red_team','attack_candidate','v2_only',
               'dual_write_read_legacy',$2
           )"#,
    )
    .bind(stale_operation_id)
    .bind(project_scope_id)
    .execute(db.pool())
    .await
    .expect("freeze stale old attack contract");
    sqlx::query(
        "INSERT INTO stage_runs(id,operation_id,stage_kind,status) \
         VALUES($1,$2,'attack_candidate','started')",
    )
    .bind(stale_stage_execution_id)
    .bind(stale_operation_id)
    .execute(db.pool())
    .await
    .expect("insert stale operation Candidate stage execution");
    sqlx::query(
        r#"INSERT INTO operation_scope_decisions(
               id,operation_id,project_scope_id,stage_execution_id,
               root_organization_id,mode,decision_rows,decision_hash
           ) VALUES(
               $1,$2,$3,$4,$5,'cli_flags',$6,'sha256:stale-decision'
           )"#,
    )
    .bind(stale_scope_decision_id)
    .bind(stale_operation_id)
    .bind(project_scope_id)
    .bind(stale_stage_execution_id)
    .bind(fixture.organization_id)
    .bind(serde_json::json!([{"organization_id": fixture.organization_id}]))
    .execute(db.pool())
    .await
    .expect("insert stale operation scope decision");
    let mut stale_scope_tx = db
        .pool()
        .begin()
        .await
        .expect("begin stale operation scope freeze");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots(
               id,operation_id,project_scope_id,scope_decision_id,
               project_path_at_freeze,root_organization_id,mode,scope_hash
           ) VALUES(
               $1,$2,$3,$4,'/tmp/rollout-cohort',$5,
               'cli_flags','sha256:stale-scope'
           )"#,
    )
    .bind(stale_scope_snapshot_id)
    .bind(stale_operation_id)
    .bind(project_scope_id)
    .bind(stale_scope_decision_id)
    .bind(fixture.organization_id)
    .execute(&mut *stale_scope_tx)
    .await
    .expect("insert stale operation scope snapshot");
    sqlx::query(
        r#"INSERT INTO operation_org_scope_units(
               snapshot_id,organization_id,organization_name_at_freeze,
               role,depth,ordinal,decision_row_id,approval_source
           ) VALUES($1,$2,'Cohort Org','root',0,0,'root',$3)"#,
    )
    .bind(stale_scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(serde_json::json!({"source": "cli_flags"}))
    .execute(&mut *stale_scope_tx)
    .await
    .expect("insert stale operation scope unit");
    sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
        .bind(stale_scope_snapshot_id)
        .execute(&mut *stale_scope_tx)
        .await
        .expect("seal stale operation scope");
    stale_scope_tx
        .commit()
        .await
        .expect("commit stale operation scope");
    let stale_wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{stale_operation_id}:candidate-wave:0").as_bytes(),
    );
    let stale_admission = sqlx::query(
        r#"INSERT INTO attack_wave_runs(
               id,operation_id,scope_snapshot_id,generation,status,
               policy_snapshot,policy_hash,max_waves,max_candidates_total,
               max_chain_depth,max_attempts_total
           ) VALUES(
               $1,$2,$3,0,'open',$4,
               'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326',
               3,100,3,200
           )"#,
    )
    .bind(stale_wave_run_id)
    .bind(stale_operation_id)
    .bind(stale_scope_snapshot_id)
    .bind(serde_json::json!({
        "max_waves": 3,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_attempts_total": 200
    }))
    .execute(db.pool())
    .await
    .expect_err("a first stale old-contract admission must remain rejected");
    assert!(
        stale_admission
            .to_string()
            .contains("ATTACK_CANDIDATE_ADMISSION_STALE_CONTRACT"),
        "unexpected stale first-admission error: {stale_admission}"
    );

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn raw_promotion_rebuilds_candidate_rows_and_writes_db_receipt() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_receipt_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave(db.pool()).await;
    let (_candidate_unit_id, _legacy_record, _record_hash) =
        close_with_matching_no_candidate_shadow(db.pool(), fixture).await;

    let source_tamper = sqlx::query(
        r#"UPDATE attack_candidate_work_items
              SET no_candidate_detail='changed after final seal'
            WHERE operation_id=$1"#,
    )
    .bind(fixture.operation_id)
    .execute(db.pool())
    .await
    .expect_err("closed shadow source rows must be immutable");
    assert!(source_tamper
        .to_string()
        .contains("ATTACK_CLOSED_SHADOW_SOURCE_IMMUTABLE"));

    sqlx::query(
        r#"UPDATE attack_execution_rollout
              SET contract='dual_write_read_v2_fallback',rank=2,
                  row_version=row_version+1,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await
    .expect("DB-recomputed exact Candidate cohort may promote one rank");

    let rollout: (String, i16, i64) = sqlx::query_as(
        "SELECT contract,rank,row_version FROM attack_execution_rollout WHERE singleton=TRUE",
    )
    .fetch_one(db.pool())
    .await
    .expect("load promoted rollout");
    assert_eq!(rollout, ("dual_write_read_v2_fallback".to_string(), 2, 2));
    let receipt: (i16, i16, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT from_rank,to_rank,admission_cutoff,admission_count,
                  candidate_unit_count,sample_count
             FROM attack_execution_rollout_promotions
            WHERE from_rank=1"#,
    )
    .fetch_one(db.pool())
    .await
    .expect("load DB-generated promotion receipt");
    assert_eq!(receipt, (1, 2, 1, 1, 1, 1));

    let direct_receipt = sqlx::query(
        r#"INSERT INTO attack_execution_rollout_promotions(
               from_rank,to_rank,from_contract,to_contract,
               from_row_version,to_row_version,admission_cutoff,
               admission_count,candidate_unit_count,sample_count
           ) VALUES(
               2,3,'dual_write_read_v2_fallback','v2_only',
               2,3,1,1,1,1
           )"#,
    )
    .execute(db.pool())
    .await
    .expect_err("callers cannot forge a promotion receipt");
    assert!(direct_receipt
        .to_string()
        .contains("ATTACK_ROLLOUT_PROMOTION_RECEIPT_INTERNAL_ONLY"));

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn nested_trigger_cannot_preoccupy_receipt_before_rollout_transition() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_receipt_hostile_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave(db.pool()).await;
    close_with_matching_no_candidate_shadow(db.pool(), fixture).await;

    sqlx::query("CREATE TABLE fixture_attack_receipt_insert_driver(id INTEGER PRIMARY KEY)")
        .execute(db.pool())
        .await
        .expect("create hostile receipt insert driver");
    sqlx::query(
        r#"CREATE FUNCTION fixture_nested_attack_receipt_insert()
           RETURNS trigger AS $$
           BEGIN
               INSERT INTO attack_execution_rollout_promotions(
                   from_rank,to_rank,from_contract,to_contract,
                   from_row_version,to_row_version,admission_cutoff,
                   admission_count,candidate_unit_count,sample_count
               ) VALUES(
                   1,2,'dual_write_read_legacy','dual_write_read_v2_fallback',
                   1,2,1,1,1,1
               );
               RETURN NEW;
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(db.pool())
    .await
    .expect("create hostile nested receipt insert function");
    sqlx::query(
        r#"CREATE TRIGGER fixture_nested_attack_receipt_insert
           BEFORE INSERT ON fixture_attack_receipt_insert_driver
           FOR EACH ROW EXECUTE FUNCTION fixture_nested_attack_receipt_insert()"#,
    )
    .execute(db.pool())
    .await
    .expect("install hostile nested receipt insert trigger");

    let preoccupy = sqlx::query("INSERT INTO fixture_attack_receipt_insert_driver(id) VALUES(1)")
        .execute(db.pool())
        .await
        .expect_err("a nested trigger cannot create a receipt before rollout advances");
    assert!(
        preoccupy
            .to_string()
            .contains("ATTACK_ROLLOUT_PROMOTION_RECEIPT_STATE_MISMATCH"),
        "unexpected pre-transition receipt rejection: {preoccupy}"
    );
    let receipt_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attack_execution_rollout_promotions")
            .fetch_one(db.pool())
            .await
            .expect("count receipts after hostile pre-transition insert");
    assert_eq!(receipt_count, 0);

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn exact_follow_on_zero_input_sibling_is_excluded_but_malformed_shape_blocks() {
    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_zero_input_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");
    let fixture = seed_admitted_wave_with_sibling(db.pool()).await;
    let sibling_fixture = fixture.sibling();
    seal_with_matching_no_candidate_shadow(db.pool(), fixture, 0).await;
    seal_with_matching_no_candidate_shadow(db.pool(), sibling_fixture, 0).await;
    sqlx::query(
        r#"UPDATE attack_wave_units
              SET status='terminal',review_closed=TRUE,verification_closed=TRUE,
                  consolidation_status='terminal',terminal_at=NOW(),updated_at=NOW()
            WHERE id=ANY($1)"#,
    )
    .bind(vec![fixture.wave_unit_id, sibling_fixture.wave_unit_id])
    .execute(db.pool())
    .await
    .expect("terminalize source WaveUnits before synthetic follow-on");
    sqlx::query(
        "UPDATE attack_wave_runs SET status='terminal',terminal_at=NOW(),updated_at=NOW() \
         WHERE id=$1",
    )
    .bind(fixture.wave_run_id)
    .execute(db.pool())
    .await
    .expect("terminalize source Wave before synthetic follow-on");

    let target_wave_run_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:candidate-wave:1", fixture.operation_id).as_bytes(),
    );
    let active_wave_unit_id = Uuid::new_v4();
    let zero_input_wave_unit_id = Uuid::new_v4();
    let consolidation_id = Uuid::new_v4();
    let mut follow_on_tx = db
        .pool()
        .begin()
        .await
        .expect("begin synthetic follow-on authority transaction");
    sqlx::query(
        r#"INSERT INTO attack_wave_runs(
               id,operation_id,scope_snapshot_id,generation,status,
               policy_snapshot,policy_hash,max_waves,max_candidates_total,
               max_chain_depth,max_attempts_total
           ) VALUES(
               $1,$2,$3,1,'open',$4,
               'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326',
               3,100,3,200
           )"#,
    )
    .bind(target_wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(serde_json::json!({
        "max_waves": 3,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_attempts_total": 200
    }))
    .execute(&mut *follow_on_tx)
    .await
    .expect("insert synthetic follow-on Wave");
    // This test isolates the 00016 cohort gate.  The separate 00012 integration
    // suite owns full FactDelta graph construction, so install the minimum
    // immutable opened-next-Wave authority while its deferred graph trigger is
    // disabled for this fixture only.
    sqlx::query(
        "ALTER TABLE attack_wave_consolidations DISABLE TRIGGER \
         attack_wave_consolidations_require_complete_graph",
    )
    .execute(&mut *follow_on_tx)
    .await
    .expect("disable deferred consolidation graph fixture trigger");
    sqlx::query(
        r#"INSERT INTO attack_wave_consolidations(
               id,operation_id,scope_snapshot_id,source_wave_run_id,
               source_generation,decision_kind,target_wave_run_id,target_generation,
               source_wave_version_before,source_wave_version_after,
               source_barrier_hash,policy_hash,fact_delta_set_hash,
               fact_delta_count,wave_count,candidate_count,chain_depth,
               attempt_count,reason_code,decision_hash
           ) VALUES(
               $1,$2,$3,$4,0,'opened_next_wave',$5,1,0,1,
               'sha256:source-barrier',
               'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326',
               'sha256:fact-delta-set',1,1,2,0,0,
               'accepted_fact_delta','sha256:synthetic-consolidation'
           )"#,
    )
    .bind(consolidation_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.wave_run_id)
    .bind(target_wave_run_id)
    .execute(&mut *follow_on_tx)
    .await
    .expect("insert immutable synthetic follow-on authority");
    sqlx::query(
        "ALTER TABLE attack_wave_consolidations ENABLE TRIGGER \
         attack_wave_consolidations_require_complete_graph",
    )
    .execute(&mut *follow_on_tx)
    .await
    .expect("restore deferred consolidation graph trigger");
    follow_on_tx
        .commit()
        .await
        .expect("commit synthetic follow-on authority transaction");
    sqlx::query(
        r#"INSERT INTO attack_wave_units(
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_consolidation_id,ordinal,status
           ) VALUES($1,$2,$3,$4,$5,$6,0,'open')"#,
    )
    .bind(active_wave_unit_id)
    .bind(target_wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(fixture.organization_id)
    .bind(consolidation_id)
    .execute(db.pool())
    .await
    .expect("insert active follow-on WaveUnit");
    let follow_on_candidate_stage_execution_id =
        seed_candidate_stage_execution(db.pool(), fixture.operation_id).await;
    let active_follow_on = fixture.at_wave(
        target_wave_run_id,
        active_wave_unit_id,
        fixture.organization_id,
        follow_on_candidate_stage_execution_id,
    );
    seal_with_matching_no_candidate_shadow(db.pool(), active_follow_on, 1).await;
    sqlx::query(
        r#"UPDATE attack_wave_units
              SET status='terminal',review_closed=TRUE,verification_closed=TRUE,
                  consolidation_status='terminal',terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1"#,
    )
    .bind(active_wave_unit_id)
    .execute(db.pool())
    .await
    .expect("terminalize active follow-on unit for zero-input gate isolation");
    sqlx::query(
        "UPDATE attack_wave_runs SET status='terminal',terminal_at=NOW(),updated_at=NOW() \
         WHERE id=$1",
    )
    .bind(target_wave_run_id)
    .execute(db.pool())
    .await
    .expect("terminalize follow-on Wave for zero-input gate isolation");

    let mut malformed_tx = db
        .pool()
        .begin()
        .await
        .expect("begin malformed zero-input shape probe");
    sqlx::query(
        r#"INSERT INTO attack_wave_units(
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_consolidation_id,ordinal,status,review_closed,
               verification_closed,consolidation_status,manifest_hash,
               manifest_count,manifest_frozen_at,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,1,'terminal',TRUE,TRUE,'terminal',
               'sha256:malformed-placeholder',1,NOW(),NOW()
           )"#,
    )
    .bind(zero_input_wave_unit_id)
    .bind(target_wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(sibling_fixture.organization_id)
    .bind(consolidation_id)
    .execute(&mut *malformed_tx)
    .await
    .expect("insert malformed terminal sibling authority");

    let cutoff: i64 = sqlx::query_scalar(
        "SELECT admission_seq FROM attack_execution_candidate_admissions WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(db.pool())
    .await
    .expect("load operation admission cutoff");
    let malformed_gate: (i64, i64, i64, bool, String) = sqlx::query_as(
        "SELECT admission_count,candidate_unit_count,sample_count,ready,reason \
         FROM attack_execution_candidate_cohort_gate( \
             'dual_write_read_legacy',1::SMALLINT,$1 \
         )",
    )
    .bind(cutoff)
    .fetch_one(&mut *malformed_tx)
    .await
    .expect("evaluate malformed zero-input sibling");
    assert!(!malformed_gate.3);
    assert_eq!(
        malformed_gate.4,
        "candidate_final_unit_missing_or_ambiguous"
    );
    malformed_tx
        .rollback()
        .await
        .expect("rollback malformed zero-input shape probe");

    sqlx::query(
        r#"INSERT INTO attack_wave_units(
               id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
               entry_consolidation_id,ordinal,status,review_closed,
               verification_closed,consolidation_status,terminal_at
           ) VALUES(
               $1,$2,$3,$4,$5,$6,1,'terminal',TRUE,TRUE,'terminal',NOW()
           )"#,
    )
    .bind(zero_input_wave_unit_id)
    .bind(target_wave_run_id)
    .bind(fixture.operation_id)
    .bind(fixture.scope_snapshot_id)
    .bind(sibling_fixture.organization_id)
    .bind(consolidation_id)
    .execute(db.pool())
    .await
    .expect("insert exact follow-on terminal zero-input shape");
    let exact_gate: (i64, i64, i64, bool, String) = sqlx::query_as(
        "SELECT admission_count,candidate_unit_count,sample_count,ready,reason \
         FROM attack_execution_candidate_cohort_gate( \
             'dual_write_read_legacy',1::SMALLINT,$1 \
         )",
    )
    .bind(cutoff)
    .fetch_one(db.pool())
    .await
    .expect("evaluate exact terminal zero-input sibling");
    assert_eq!(exact_gate, (1, 3, 3, true, "ready".to_string()));
    sqlx::query(
        r#"UPDATE attack_execution_rollout
              SET contract='dual_write_read_v2_fallback',rank=2,
                  row_version=row_version+1,updated_at=NOW()
            WHERE singleton=TRUE"#,
    )
    .execute(db.pool())
    .await
    .expect("exact zero-input sibling must not block Candidate-domain promotion");

    db.stop().await;
}

#[tokio::test]
#[serial]
async fn repository_reconcile_is_typed_noop_then_promotes_one_rank() {
    use repo::attack_execution_rollout::AttackExecutionRolloutReconcileOutcome;

    let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
    let mut db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("attack_rollout_reconcile_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start migrated embedded postgres");

    let not_ready = repo::attack_execution_rollout::reconcile_attack_execution_rollout(db.pool())
        .await
        .expect("empty cohort reconciliation is not an infrastructure error");
    assert!(matches!(
        not_ready,
        AttackExecutionRolloutReconcileOutcome::NotReady {
            rank: 1,
            row_version: 1,
            ref reason,
            ..
        } if reason == "candidate_cohort_empty"
    ));

    let fixture = seed_admitted_wave(db.pool()).await;
    close_with_matching_no_candidate_shadow(db.pool(), fixture).await;
    let promoted = repo::attack_execution_rollout::reconcile_attack_execution_rollout(db.pool())
        .await
        .expect("ready exact cohort reconciles");
    match promoted {
        AttackExecutionRolloutReconcileOutcome::Promoted(row) => {
            assert_eq!(row.contract, "dual_write_read_v2_fallback");
            assert_eq!((row.rank, row.row_version), (2, 2));
        }
        other => panic!("expected one adjacent promotion, got {other:?}"),
    }

    db.stop().await;
}
