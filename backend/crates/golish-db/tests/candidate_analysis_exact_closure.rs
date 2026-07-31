use golish_db::{
    repo::candidate_analysis::validate_candidate_analysis_exact_closure_on, DbConfig, GolishDb,
};
use serde_json::{json, Value};
use serial_test::serial;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const ATTACK_CLASSES: [(&str, i32); 8] = [
    ("authentication", 1),
    ("authorization", 1),
    ("business_logic", 1),
    ("configuration", 1),
    ("data_exposure", 1),
    ("injection", 1),
    ("availability", 1),
    ("supply_chain", 1),
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
    let db = GolishDb::start(DbConfig {
        pg_data_dir: data_dir.path().join("pgdata"),
        port: reserve_local_port(),
        database: format!("candidate_exact_{}", Uuid::new_v4().simple()),
        ..DbConfig::default()
    })
    .await
    .expect("start isolated migrated postgres");
    (db, data_dir)
}

async fn hash_json(tx: &mut Transaction<'_, Postgres>, value: &Value) -> String {
    sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
        .bind(value)
        .fetch_one(&mut **tx)
        .await
        .expect("hash canonical JSON")
}

async fn hash_texts(tx: &mut Transaction<'_, Postgres>, values: &[String]) -> String {
    sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
        .bind(values)
        .fetch_one(&mut **tx)
        .await
        .expect("hash canonical text array")
}

async fn create_temp_authority_tables(tx: &mut Transaction<'_, Postgres>) {
    sqlx::raw_sql(
        r#"
        CREATE TEMP TABLE candidate_analysis_attempts(
            analysis_attempt_id UUID PRIMARY KEY,snapshot_id UUID NOT NULL,
            attack_class_checklist_version TEXT NOT NULL,attack_class_checklist_digest TEXT NOT NULL,
            trust_boundary_checklist_version TEXT NOT NULL,trust_boundary_checklist_digest TEXT NOT NULL,
            predecessor_attempt_id UUID,attempt_input_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_snapshot_inputs(
            snapshot_input_id UUID PRIMARY KEY,snapshot_id UUID NOT NULL,stable_input_key TEXT NOT NULL,
            source_ref TEXT NOT NULL,subject_kind_at_time TEXT NOT NULL,
            subject_identity_hash TEXT NOT NULL,source_content_hash TEXT NOT NULL,
            server_chunking_disposition TEXT NOT NULL,input_hash TEXT NOT NULL);
        CREATE TEMP TABLE hypothesis_proposals(
            proposal_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,proposal_ordinal INTEGER NOT NULL,
            proposal_hash TEXT NOT NULL,artifact_id UUID,structured_proposal JSONB);
        CREATE TEMP TABLE hypothesis_proposal_refs(
            proposal_ref_id UUID PRIMARY KEY,proposal_id UUID NOT NULL,analysis_attempt_id UUID NOT NULL,
            snapshot_input_id UUID NOT NULL,chunk_id UUID,source_role TEXT,
            source_hash TEXT,ref_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_proposal_censuses(
            proposal_census_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,
            proposal_count BIGINT NOT NULL,proposal_set_hash TEXT NOT NULL,census_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_proposal_census_members(
            census_member_id UUID PRIMARY KEY,proposal_census_id UUID NOT NULL,analysis_attempt_id UUID NOT NULL,
            proposal_id UUID NOT NULL,ordinal INTEGER NOT NULL,proposal_hash TEXT NOT NULL,member_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_input_proposal_dispositions(
            analysis_attempt_id UUID NOT NULL,snapshot_input_id UUID NOT NULL,proposal_ref_count BIGINT NOT NULL,
            proposal_ref_set_hash TEXT NOT NULL,disposition TEXT NOT NULL,blocker_code TEXT,disposition_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_enrichment_obligations(
            obligation_id UUID PRIMARY KEY,snapshot_id UUID NOT NULL,product_member_id UUID,
            feed_snapshot_member_id UUID,obligation_kind TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_feed_match_census_members(
            match_member_id UUID PRIMARY KEY,snapshot_id UUID NOT NULL,product_member_id UUID,
            feed_snapshot_member_id UUID,disposition TEXT NOT NULL,ordinal INTEGER NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_checklist_members(
            checklist_member_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,snapshot_input_id UUID NOT NULL,
            ordinal INTEGER NOT NULL,attack_class_contract_version TEXT NOT NULL,
            attack_class_contract_digest TEXT NOT NULL,trust_boundary_contract_version TEXT NOT NULL,
            trust_boundary_contract_digest TEXT NOT NULL,attack_class_id TEXT NOT NULL,
            attack_class_version INTEGER NOT NULL,trust_boundary_identity TEXT NOT NULL,
            trust_boundary_hash TEXT NOT NULL,applicability_basis JSONB NOT NULL,
            feed_match_member_refs UUID[] NOT NULL,applicability_disposition TEXT NOT NULL,
            enrichment_obligation_id UUID,member_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_input_chunk_censuses(
            chunk_census_id UUID PRIMARY KEY,snapshot_input_id UUID NOT NULL,snapshot_id UUID NOT NULL,
            disposition TEXT NOT NULL,
            chunk_count BIGINT NOT NULL,census_hash TEXT NOT NULL,source_byte_count BIGINT NOT NULL,
            chunking_contract_version TEXT NOT NULL,redaction_contract_version TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_input_chunk_census_members(
            chunk_id UUID PRIMARY KEY,chunk_census_id UUID NOT NULL,ordinal INTEGER NOT NULL,
            chunk_hash TEXT NOT NULL,source_range_start BIGINT NOT NULL,source_range_end BIGINT NOT NULL,
            snapshot_input_id UUID NOT NULL,snapshot_id UUID NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_chunk_partitions(
            chunk_partition_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,snapshot_input_id UUID NOT NULL,
            partition_ordinal INTEGER NOT NULL,first_chunk_ordinal INTEGER NOT NULL,last_chunk_ordinal INTEGER NOT NULL,
            chunk_count BIGINT NOT NULL,chunk_set_hash TEXT NOT NULL,bounded_context_budget BIGINT NOT NULL,
            partition_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_work_items(
            candidate_work_item_id UUID PRIMARY KEY,stage_work_item_id UUID NOT NULL UNIQUE,
            analysis_attempt_id UUID NOT NULL,phase TEXT NOT NULL,
            capability TEXT NOT NULL,microbatch_key TEXT,component_id UUID);
        CREATE TEMP TABLE stage_work_items(
            id UUID PRIMARY KEY,kind TEXT NOT NULL,role TEXT NOT NULL,status TEXT NOT NULL);
        CREATE TEMP TABLE stage_worker_runs(
            id UUID PRIMARY KEY,work_item_id UUID NOT NULL,specialist TEXT NOT NULL,
            work_item_kind TEXT NOT NULL,status TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_provider_attempts(
            provider_attempt_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,
            stage_work_item_id UUID NOT NULL,worker_run_id UUID NOT NULL,
            artifact_kind TEXT NOT NULL,artifact_id UUID,artifact_body JSONB NOT NULL,
            artifact_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_artifacts(
            artifact_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,
            candidate_work_item_id UUID NOT NULL,worker_run_id UUID NOT NULL,
            artifact_kind TEXT NOT NULL,stage_worker_output_id UUID,
            artifact_body JSONB NOT NULL,artifact_hash TEXT NOT NULL);
        CREATE TEMP TABLE stage_worker_outputs(
            id UUID PRIMARY KEY,work_item_id UUID NOT NULL,worker_run_id UUID NOT NULL,
            output_schema TEXT NOT NULL,output_version INTEGER NOT NULL,
            business_disposition TEXT NOT NULL,canonical_output JSONB NOT NULL,
            canonical_fact_refs JSONB NOT NULL,evidence_ids BIGINT[] NOT NULL,
            checked_empty_cells JSONB NOT NULL,blocker_codes TEXT[] NOT NULL,
            output_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_page_receipts(
            page_receipt_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,page_kind TEXT NOT NULL,
            stable_request_id UUID NOT NULL,snapshot_id UUID NOT NULL,snapshot_input_id UUID NOT NULL,
            chunk_census_id UUID NOT NULL,
            chunk_census_hash TEXT NOT NULL,source_size_bytes BIGINT NOT NULL,
            chunking_contract_version TEXT NOT NULL,redaction_contract_version TEXT NOT NULL,
            consumer_worker_run_id UUID NOT NULL,server_cursor TEXT NOT NULL,first_key TEXT,last_key TEXT,
            returned_count BIGINT NOT NULL,page_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_subreview_censuses(
            subreview_census_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,snapshot_input_id UUID NOT NULL,
            checklist_member_count BIGINT NOT NULL,checklist_member_set_hash TEXT NOT NULL,
            chunk_partition_count BIGINT NOT NULL,chunk_partition_set_hash TEXT NOT NULL,
            expected_member_count BIGINT NOT NULL,member_set_hash TEXT NOT NULL,census_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_subreview_census_members(
            subreview_census_member_id UUID PRIMARY KEY,subreview_census_id UUID NOT NULL,
            analysis_attempt_id UUID NOT NULL,snapshot_input_id UUID NOT NULL,checklist_member_id UUID NOT NULL,
            chunk_partition_id UUID NOT NULL,checklist_ordinal INTEGER NOT NULL,partition_ordinal INTEGER NOT NULL,
            designated_stage_work_item_id UUID NOT NULL,disposition TEXT NOT NULL,member_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_subreviews(
            subreview_id UUID PRIMARY KEY,subreview_census_member_id UUID NOT NULL,
            analysis_attempt_id UUID NOT NULL,snapshot_input_id UUID NOT NULL,
            subreview_hash TEXT NOT NULL,outcome TEXT NOT NULL,typed_missed_refs JSONB NOT NULL,
            blocker_codes TEXT[] NOT NULL,semantic_summary JSONB NOT NULL,
            semantic_observation_count BIGINT NOT NULL,semantic_summary_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_synthesis_census_members(
            synthesis_node_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,
            covered_input_count BIGINT NOT NULL,covered_input_set_hash TEXT NOT NULL,
            covered_checklist_count BIGINT NOT NULL,covered_checklist_set_hash TEXT NOT NULL,
            child_receipt_count BIGINT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_synthesis_node_children(
            child_member_id UUID PRIMARY KEY,synthesis_node_id UUID NOT NULL,
            ordinal INTEGER NOT NULL,child_subreview_id UUID,child_synthesis_node_id UUID);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_synthesis_reviews(
            synthesis_review_id UUID PRIMARY KEY,synthesis_node_id UUID NOT NULL,
            analysis_attempt_id UUID NOT NULL,review_hash TEXT NOT NULL,
            outcome TEXT NOT NULL,typed_missed_refs JSONB NOT NULL,blocker_codes TEXT[] NOT NULL,
            semantic_summary JSONB NOT NULL,semantic_observation_count BIGINT NOT NULL,
            semantic_summary_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_reviews(
            coverage_review_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,
            artifact_id UUID,snapshot_input_id UUID,outcome TEXT,review_mode TEXT,
            checklist_dispositions JSONB,typed_missed_refs JSONB,review_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_hypothesis_coverage_global_reviews(
            global_review_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,review_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_conflict_components(
            conflict_component_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,component_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_critic_censuses(
            critic_census_id UUID PRIMARY KEY,analysis_attempt_id UUID NOT NULL,member_count BIGINT NOT NULL,
            member_set_hash TEXT NOT NULL,census_hash TEXT NOT NULL);
        CREATE TEMP TABLE candidate_analysis_critic_census_members(
            critic_member_id UUID PRIMARY KEY,critic_census_id UUID NOT NULL,analysis_attempt_id UUID NOT NULL,
            ordinal INTEGER NOT NULL,member_kind TEXT NOT NULL,source_identity UUID NOT NULL,
            source_hash TEXT NOT NULL,member_hash TEXT NOT NULL);
        "#,
    )
    .execute(&mut **tx)
    .await
    .expect("create exact-closure temporary authority tables");
}

#[allow(clippy::too_many_arguments)]
async fn insert_chunk_page_receipt(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
    snapshot_id: Uuid,
    input_id: Uuid,
    chunk_census_id: Uuid,
    chunk_census_hash: &str,
    chunk_hash: &str,
    worker_id: Uuid,
) {
    let stable_request_id = Uuid::new_v4();
    let page_hash = hash_json(
        tx,
        &json!({
            "schema":"candidate_chunk_page_receipt.v1",
            "analysis_attempt_id":attempt_id,"snapshot_id":snapshot_id,
            "snapshot_input_id":input_id,"chunk_census_id":chunk_census_id,
            "chunk_census_hash":chunk_census_hash,"consumer_worker_run_id":worker_id,
            "first_ordinal":0,"last_ordinal":0,"returned_count":1,
            "ordered_chunk_hashes":[chunk_hash],"source_size_bytes":16,
            "chunking_contract_version":"chunking.v1",
            "redaction_contract_version":"redaction.v1",
        }),
    )
    .await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_page_receipts
           VALUES($1,$2,'chunk_page',$3,$4,$5,$6,$7,16,'chunking.v1','redaction.v1',
                  $8,'chunk:0:1','0','0',1,$9)"#,
    )
    .bind(Uuid::new_v5(
        &stable_request_id,
        b"candidate_page_receipt.v1",
    ))
    .bind(attempt_id)
    .bind(stable_request_id)
    .bind(snapshot_id)
    .bind(input_id)
    .bind(chunk_census_id)
    .bind(chunk_census_hash)
    .bind(worker_id)
    .bind(page_hash)
    .execute(&mut **tx)
    .await
    .expect("insert exact chunk-page receipt");
}

async fn seed_closed_candidate_analysis(tx: &mut Transaction<'_, Postgres>) -> (Uuid, Uuid, Uuid) {
    let attempt_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let input_id = Uuid::new_v4();
    let subject_hash = hash_json(tx, &json!({"subject":"root"})).await;
    let source_content_hash = hash_json(tx, &json!({"source":"root"})).await;
    let input_hash = hash_json(tx, &json!({"input":input_id})).await;
    let attack_manifest = json!({
        "contract":"attack_class.v1","version":1,
        "members":ATTACK_CLASSES.iter().map(|(id,version)|json!({
            "attack_class_id":id,"attack_class_version":version,
        })).collect::<Vec<_>>(),
    });
    let attack_digest = hash_json(tx, &attack_manifest).await;
    let boundary_digest = hash_json(
        tx,
        &json!({
            "contract":"trust_boundary.v1","version":1,
            "boundaries":[{"identity":"target","hash":subject_hash}],
        }),
    )
    .await;
    let attempt_input_hash = hash_json(tx, &json!({"attempt":attempt_id})).await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,attack_class_checklist_version,
               attack_class_checklist_digest,trust_boundary_checklist_version,
               trust_boundary_checklist_digest,predecessor_attempt_id,attempt_input_hash
           ) VALUES($1,$2,'attack_class.v1',$3,'trust_boundary.v1',$4,NULL,$5)"#,
    )
    .bind(attempt_id)
    .bind(snapshot_id)
    .bind(&attack_digest)
    .bind(&boundary_digest)
    .bind(attempt_input_hash)
    .execute(&mut **tx)
    .await
    .expect("insert attempt");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_inputs
            VALUES($1,$2,'input:0','source:test','target',$3,$4,'complete',$5)"#,
    )
    .bind(input_id)
    .bind(snapshot_id)
    .bind(&subject_hash)
    .bind(&source_content_hash)
    .bind(input_hash)
    .execute(&mut **tx)
    .await
    .expect("insert input");

    let empty_hash = hash_texts(tx, &[]).await;
    let proposal_census_id = Uuid::new_v4();
    let proposal_census_hash = hash_json(
        tx,
        &json!({"kind":"proposal","attempt":attempt_id,"count":0,"set":empty_hash}),
    )
    .await;
    sqlx::query("INSERT INTO candidate_analysis_proposal_censuses VALUES($1,$2,0,$3,$4)")
        .bind(proposal_census_id)
        .bind(attempt_id)
        .bind(&empty_hash)
        .bind(proposal_census_hash)
        .execute(&mut **tx)
        .await
        .expect("insert proposal census");
    let disposition_hash = hash_json(
        tx,
        &json!({
            "analysis_attempt_id":attempt_id,"snapshot_input_id":input_id,
            "proposal_ref_set_hash":empty_hash,"disposition":"zero_proposal",
            "blocker_code":Option::<String>::None,
        }),
    )
    .await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_proposal_dispositions
            VALUES($1,$2,0,$3,'zero_proposal',NULL,$4)"#,
    )
    .bind(attempt_id)
    .bind(input_id)
    .bind(&empty_hash)
    .bind(disposition_hash)
    .execute(&mut **tx)
    .await
    .expect("insert H1 disposition");

    let controller_stage_work_item_id = Uuid::new_v4();
    let controller_candidate_work_item_id =
        Uuid::new_v5(&attempt_id, controller_stage_work_item_id.as_bytes());
    let controller_worker_id = Uuid::new_v5(&controller_stage_work_item_id, b"candidate-worker.v1");
    let controller_provider_attempt_id = Uuid::new_v4();
    let controller_body = json!({
        "kind":"controller_dispatch.v1",
        "analysis_attempt_id":attempt_id,
    });
    let controller_hash = hash_json(tx, &controller_body).await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_work_items(
               candidate_work_item_id,stage_work_item_id,analysis_attempt_id,phase,
               capability,microbatch_key,component_id
           ) VALUES($1,$2,$3,'controller','candidate_controller_dispatch',NULL,NULL)"#,
    )
    .bind(controller_candidate_work_item_id)
    .bind(controller_stage_work_item_id)
    .bind(attempt_id)
    .execute(&mut **tx)
    .await
    .expect("insert exact controller Candidate work item");
    sqlx::query(
        "INSERT INTO stage_work_items VALUES($1,'candidate_controller_dispatch','controller','completed')",
    )
    .bind(controller_stage_work_item_id)
    .execute(&mut **tx)
    .await
    .expect("insert exact controller stage work item");
    sqlx::query(
        "INSERT INTO stage_worker_runs VALUES($1,$2,'controller','candidate_controller_dispatch','passed')",
    )
    .bind(controller_worker_id)
    .bind(controller_stage_work_item_id)
    .execute(&mut **tx)
    .await
    .expect("insert exact controller worker");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_provider_attempts(
               provider_attempt_id,analysis_attempt_id,stage_work_item_id,worker_run_id,
               artifact_kind,artifact_id,artifact_body,artifact_hash
           ) VALUES($1,$2,$3,$4,'controller_dispatch.v1',NULL,$5,$6)"#,
    )
    .bind(controller_provider_attempt_id)
    .bind(attempt_id)
    .bind(controller_stage_work_item_id)
    .bind(controller_worker_id)
    .bind(controller_body)
    .bind(controller_hash)
    .execute(&mut **tx)
    .await
    .expect("insert exact controller provider receipt");

    let chunk_id = Uuid::new_v4();
    let chunk_census_id = Uuid::new_v4();
    let chunk_hash = hash_json(tx, &json!({"chunk":0})).await;
    let chunk_census_hash = hash_json(tx, &json!({"chunk_census":chunk_census_id})).await;
    sqlx::query(
        "INSERT INTO candidate_analysis_input_chunk_censuses VALUES($1,$2,$3,'complete',1,$4,16,'chunking.v1','redaction.v1')",
    )
    .bind(chunk_census_id)
    .bind(input_id)
    .bind(snapshot_id)
    .bind(&chunk_census_hash)
    .execute(&mut **tx)
    .await
    .expect("insert chunk census");
    sqlx::query(
        "INSERT INTO candidate_analysis_input_chunk_census_members VALUES($1,$2,0,$3,0,16,$4,$5)",
    )
    .bind(chunk_id)
    .bind(chunk_census_id)
    .bind(&chunk_hash)
    .bind(input_id)
    .bind(snapshot_id)
    .execute(&mut **tx)
    .await
    .expect("insert chunk");
    let chunk_set_hash = hash_texts(tx, std::slice::from_ref(&chunk_hash)).await;
    let partition_id = Uuid::new_v4();
    let partition_hash = hash_json(
        tx,
        &json!({
            "analysis_attempt_id":attempt_id,"snapshot_input_id":input_id,
            "chunk_set_hash":chunk_set_hash,"first":0,"last":0,
        }),
    )
    .await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_chunk_partitions
            VALUES($1,$2,$3,0,0,0,1,$4,262144,$5)"#,
    )
    .bind(partition_id)
    .bind(attempt_id)
    .bind(input_id)
    .bind(&chunk_set_hash)
    .bind(&partition_hash)
    .execute(&mut **tx)
    .await
    .expect("insert partition");

    let analyst_item_id = Uuid::new_v4();
    let analyst_candidate_item_id = Uuid::new_v5(&attempt_id, analyst_item_id.as_bytes());
    let analyst_worker_id = Uuid::new_v5(&analyst_item_id, b"candidate-worker.v1");
    let analyst_provider_attempt_id = Uuid::new_v4();
    let analyst_artifact_id = Uuid::new_v5(&analyst_provider_attempt_id, b"hypothesis_proposal.v1");
    let analyst_output_id = Uuid::new_v5(&analyst_artifact_id, b"candidate_stage_worker_output.v1");
    let analyst_artifact_body = json!({"proposals": []});
    let analyst_artifact_hash = hash_json(tx, &analyst_artifact_body).await;
    let analyst_canonical_output = json!({
        "schema":"candidate_analysis_artifact_receipt.v1",
        "artifact_id":analyst_artifact_id,
        "artifact_hash":analyst_artifact_hash,
    });
    let analyst_output_hash = hash_json(tx, &analyst_canonical_output).await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_work_items(
               candidate_work_item_id,stage_work_item_id,analysis_attempt_id,phase,
               capability,microbatch_key,component_id
           ) VALUES($1,$2,$3,'proposal','hypothesis_proposal',$4,NULL)"#,
    )
    .bind(analyst_candidate_item_id)
    .bind(analyst_item_id)
    .bind(attempt_id)
    .bind(input_id.to_string())
    .execute(&mut **tx)
    .await
    .expect("insert analyst work item");
    sqlx::query(
        "INSERT INTO stage_work_items VALUES($1,'hypothesis_proposal','analyst','completed')",
    )
    .bind(analyst_item_id)
    .execute(&mut **tx)
    .await
    .expect("insert analyst stage work item");
    sqlx::query(
        "INSERT INTO stage_worker_runs VALUES($1,$2,'analyst','hypothesis_proposal','passed')",
    )
    .bind(analyst_worker_id)
    .bind(analyst_item_id)
    .execute(&mut **tx)
    .await
    .expect("insert analyst worker");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_provider_attempts(
               provider_attempt_id,analysis_attempt_id,stage_work_item_id,worker_run_id,
               artifact_kind,artifact_id,artifact_body,artifact_hash
           ) VALUES($1,$2,$3,$4,'hypothesis_proposal.v1',$5,$6,$7)"#,
    )
    .bind(analyst_provider_attempt_id)
    .bind(attempt_id)
    .bind(analyst_item_id)
    .bind(analyst_worker_id)
    .bind(analyst_artifact_id)
    .bind(&analyst_artifact_body)
    .bind(&analyst_artifact_hash)
    .execute(&mut **tx)
    .await
    .expect("insert exact analyst provider receipt");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_artifacts(
               artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,
               artifact_kind,stage_worker_output_id,artifact_body,artifact_hash
           ) VALUES($1,$2,$3,$4,'hypothesis_proposal.v1',$5,$6,$7)"#,
    )
    .bind(analyst_artifact_id)
    .bind(attempt_id)
    .bind(analyst_candidate_item_id)
    .bind(analyst_worker_id)
    .bind(analyst_output_id)
    .bind(&analyst_artifact_body)
    .bind(&analyst_artifact_hash)
    .execute(&mut **tx)
    .await
    .expect("insert exact analyst proposal artifact");
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,work_item_id,worker_run_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash
           ) VALUES($1,$2,$3,'candidate_analysis_artifact_receipt.v1',1,
                    'artifact_recorded',$4,'[]'::JSONB,$5,'[]'::JSONB,$6,$7)"#,
    )
    .bind(analyst_output_id)
    .bind(analyst_item_id)
    .bind(analyst_worker_id)
    .bind(&analyst_canonical_output)
    .bind(Vec::<i64>::new())
    .bind(Vec::<String>::new())
    .bind(analyst_output_hash)
    .execute(&mut **tx)
    .await
    .expect("insert exact analyst stage output");
    insert_chunk_page_receipt(
        tx,
        attempt_id,
        snapshot_id,
        input_id,
        chunk_census_id,
        &chunk_census_hash,
        &chunk_hash,
        analyst_worker_id,
    )
    .await;

    let mut checklist = Vec::new();
    for (ordinal, (attack_class_id, attack_class_version)) in ATTACK_CLASSES.iter().enumerate() {
        let ordinal = i32::try_from(ordinal).expect("bounded checklist ordinal");
        let checklist_id = Uuid::new_v5(
            &attempt_id,
            format!(
                "checklist:{input_id}:{attack_class_id}:{attack_class_version}:target:{subject_hash}"
            )
            .as_bytes(),
        );
        let member_hash = hash_json(
            tx,
            &json!({
                "analysis_attempt_id":attempt_id,"snapshot_input_id":input_id,"ordinal":ordinal,
                "attack_class_id":attack_class_id,"attack_class_version":attack_class_version,
                "trust_boundary_identity":"target","trust_boundary_hash":subject_hash,
                "attack_class_contract_digest":attack_digest,
                "trust_boundary_contract_digest":boundary_digest,
                "feed_match_member_refs":Vec::<Uuid>::new(),
                "applicability_disposition":"required",
                "enrichment_obligation_id":Option::<Uuid>::None,
            }),
        )
        .await;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_checklist_members
               VALUES($1,$2,$3,$4,'attack_class.v1',$5,'trust_boundary.v1',$6,$7,$8,
                      'target',$9,$10,$11,'required',NULL,$12)"#,
        )
        .bind(checklist_id)
        .bind(attempt_id)
        .bind(input_id)
        .bind(ordinal)
        .bind(&attack_digest)
        .bind(&boundary_digest)
        .bind(attack_class_id)
        .bind(attack_class_version)
        .bind(&subject_hash)
        .bind(json!({
            "source":"server_frozen_catalog_x_boundary",
            "input_subject_kind":"target","input_subject_identity_hash":subject_hash,
        }))
        .bind(Vec::<Uuid>::new())
        .bind(&member_hash)
        .execute(&mut **tx)
        .await
        .expect("insert checklist member");
        checklist.push((checklist_id, ordinal, member_hash));
    }

    let census_id = Uuid::new_v4();
    let mut subreview_member_hashes = Vec::new();
    let mut subreview_sources = Vec::new();
    for (checklist_id, ordinal, checklist_hash) in &checklist {
        let work_item_id = Uuid::new_v4();
        let candidate_work_item_id = Uuid::new_v5(&attempt_id, work_item_id.as_bytes());
        let critic_worker_id = Uuid::new_v5(&work_item_id, b"candidate-worker.v1");
        sqlx::query(
            r#"INSERT INTO candidate_analysis_work_items(
                   candidate_work_item_id,stage_work_item_id,analysis_attempt_id,phase,
                   capability,microbatch_key,component_id
               ) VALUES($1,$2,$3,'critic','hypothesis_coverage_subreview',$4,$5)"#,
        )
        .bind(candidate_work_item_id)
        .bind(work_item_id)
        .bind(attempt_id)
        .bind(partition_id.to_string())
        .bind(checklist_id)
        .execute(&mut **tx)
        .await
        .expect("insert designated work item");
        sqlx::query(
            "INSERT INTO stage_worker_runs VALUES($1,$2,'critic','hypothesis_coverage_subreview','passed')",
        )
        .bind(critic_worker_id)
        .bind(work_item_id)
        .execute(&mut **tx)
        .await
        .expect("insert critic worker");
        insert_chunk_page_receipt(
            tx,
            attempt_id,
            snapshot_id,
            input_id,
            chunk_census_id,
            &chunk_census_hash,
            &chunk_hash,
            critic_worker_id,
        )
        .await;
        let member_hash = hash_json(
            tx,
            &json!({
                "domain":"candidate_hypothesis_coverage_subreview_census_member.v1",
                "analysis_attempt_id":attempt_id,"snapshot_input_id":input_id,
                "checklist_member_id":checklist_id,"checklist_ordinal":ordinal,
                "checklist_member_hash":checklist_hash,"chunk_partition_id":partition_id,
                "partition_ordinal":0,"chunk_partition_hash":partition_hash,
                "designated_stage_work_item_id":work_item_id,"disposition":"required",
            }),
        )
        .await;
        let census_member_id = Uuid::new_v5(&census_id, member_hash.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreview_census_members
               VALUES($1,$2,$3,$4,$5,$6,$7,0,$8,'required',$9)"#,
        )
        .bind(census_member_id)
        .bind(census_id)
        .bind(attempt_id)
        .bind(input_id)
        .bind(checklist_id)
        .bind(partition_id)
        .bind(ordinal)
        .bind(work_item_id)
        .bind(&member_hash)
        .execute(&mut **tx)
        .await
        .expect("insert subreview census member");
        let subreview_id = Uuid::new_v4();
        let subreview_hash = hash_json(tx, &json!({"subreview":subreview_id})).await;
        let subreview_semantic_summary = json!({
            "covered_input_ids":[input_id],
            "covered_checklist_member_ids":[checklist_id],
            "observed_proposal_ids":[],
            "missed_checklist_member_ids":[],
            "blocker_codes":[],
            "semantic_observations":[],
        });
        let subreview_semantic_summary_hash = hash_json(tx, &subreview_semantic_summary).await;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreviews(
                   subreview_id,subreview_census_member_id,analysis_attempt_id,snapshot_input_id,
                   subreview_hash,outcome,typed_missed_refs,blocker_codes,semantic_summary,
                   semantic_observation_count,semantic_summary_hash
               ) VALUES($1,$2,$3,$4,$5,'no_local_miss','[]'::JSONB,$6,$7,0,$8)"#,
        )
        .bind(subreview_id)
        .bind(census_member_id)
        .bind(attempt_id)
        .bind(input_id)
        .bind(&subreview_hash)
        .bind(Vec::<String>::new())
        .bind(subreview_semantic_summary)
        .bind(subreview_semantic_summary_hash)
        .execute(&mut **tx)
        .await
        .expect("insert subreview");
        subreview_member_hashes.push(member_hash);
        subreview_sources.push((
            "hypothesis_coverage_subreview".to_owned(),
            subreview_id,
            subreview_hash,
        ));
    }
    let checklist_set_hash = hash_texts(
        tx,
        &checklist
            .iter()
            .map(|member| member.2.clone())
            .collect::<Vec<_>>(),
    )
    .await;
    let partition_set_hash = hash_texts(tx, std::slice::from_ref(&partition_hash)).await;
    let member_set_hash = hash_texts(tx, &subreview_member_hashes).await;
    let census_hash = hash_json(
        tx,
        &json!({
            "domain":"candidate_hypothesis_coverage_subreview_census.v1",
            "analysis_attempt_id":attempt_id,"snapshot_input_id":input_id,
            "checklist_member_count":8,"checklist_member_set_hash":checklist_set_hash,
            "chunk_partition_count":1,"chunk_partition_set_hash":partition_set_hash,
            "expected_member_count":8,"member_set_hash":member_set_hash,
        }),
    )
    .await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreview_censuses
           VALUES($1,$2,$3,8,$4,1,$5,8,$6,$7)"#,
    )
    .bind(census_id)
    .bind(attempt_id)
    .bind(input_id)
    .bind(checklist_set_hash)
    .bind(partition_set_hash)
    .bind(member_set_hash)
    .bind(census_hash)
    .execute(&mut **tx)
    .await
    .expect("insert subreview census");

    let synthesis_node_id = Uuid::new_v4();
    let synthesis_review_id = Uuid::new_v4();
    let synthesis_review_hash = hash_json(tx, &json!({"synthesis":synthesis_node_id})).await;
    let synthesis_input_set_hash = hash_texts(tx, &[input_id.to_string()]).await;
    let mut synthesis_checklist_ids = checklist.iter().map(|member| member.0).collect::<Vec<_>>();
    synthesis_checklist_ids.sort_unstable();
    let synthesis_checklist_set_hash = hash_texts(
        tx,
        &synthesis_checklist_ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>(),
    )
    .await;
    let synthesis_semantic_summary = json!({
        "covered_input_ids":[input_id],
        "covered_checklist_member_ids":synthesis_checklist_ids,
        "observed_proposal_ids":[],
        "missed_checklist_member_ids":[],
        "blocker_codes":[],
        "semantic_observations":[],
    });
    let synthesis_semantic_summary_hash = hash_json(tx, &synthesis_semantic_summary).await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_census_members(
               synthesis_node_id,analysis_attempt_id,covered_input_count,covered_input_set_hash,
               covered_checklist_count,covered_checklist_set_hash,child_receipt_count
           ) VALUES($1,$2,1,$3,8,$4,8)"#,
    )
    .bind(synthesis_node_id)
    .bind(attempt_id)
    .bind(synthesis_input_set_hash)
    .bind(synthesis_checklist_set_hash)
    .execute(&mut **tx)
    .await
    .expect("insert synthesis node");
    for (ordinal, (_, subreview_id, _)) in subreview_sources.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_node_children(
                   child_member_id,synthesis_node_id,ordinal,child_subreview_id,
                   child_synthesis_node_id
               ) VALUES($1,$2,$3,$4,NULL)"#,
        )
        .bind(Uuid::new_v5(
            &synthesis_node_id,
            format!("subreview:{subreview_id}").as_bytes(),
        ))
        .bind(synthesis_node_id)
        .bind(i32::try_from(ordinal).expect("bounded synthesis child ordinal"))
        .bind(subreview_id)
        .execute(&mut **tx)
        .await
        .expect("insert synthesis subreview child");
    }
    sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_reviews(
               synthesis_review_id,synthesis_node_id,analysis_attempt_id,review_hash,outcome,
               typed_missed_refs,blocker_codes,semantic_summary,semantic_observation_count,
               semantic_summary_hash
           ) VALUES($1,$2,$3,$4,'no_composite_miss','[]'::JSONB,$5,$6,0,$7)"#,
    )
    .bind(synthesis_review_id)
    .bind(synthesis_node_id)
    .bind(attempt_id)
    .bind(&synthesis_review_hash)
    .bind(Vec::<String>::new())
    .bind(synthesis_semantic_summary)
    .bind(synthesis_semantic_summary_hash)
    .execute(&mut **tx)
    .await
    .expect("insert synthesis review");
    subreview_sources.push((
        "hypothesis_coverage_synthesis".to_owned(),
        synthesis_review_id,
        synthesis_review_hash,
    ));
    let coverage_review_id = Uuid::new_v4();
    let coverage_review_hash = hash_json(tx, &json!({"coverage":input_id})).await;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_hypothesis_coverage_reviews(
               coverage_review_id,analysis_attempt_id,artifact_id,snapshot_input_id,
               outcome,review_mode,checklist_dispositions,typed_missed_refs,review_hash
           ) VALUES($1,$2,NULL,$3,'adequate','full','[]'::JSONB,'[]'::JSONB,$4)"#,
    )
    .bind(coverage_review_id)
    .bind(attempt_id)
    .bind(input_id)
    .bind(&coverage_review_hash)
    .execute(&mut **tx)
    .await
    .expect("insert input coverage review");
    subreview_sources.push((
        "hypothesis_coverage_input_review".to_owned(),
        coverage_review_id,
        coverage_review_hash,
    ));
    let global_review_id = Uuid::new_v4();
    let global_review_hash = hash_json(tx, &json!({"global":attempt_id})).await;
    sqlx::query(
        "INSERT INTO candidate_analysis_hypothesis_coverage_global_reviews VALUES($1,$2,$3)",
    )
    .bind(global_review_id)
    .bind(attempt_id)
    .bind(&global_review_hash)
    .execute(&mut **tx)
    .await
    .expect("insert global review");
    subreview_sources.push((
        "hypothesis_coverage_global_review".to_owned(),
        global_review_id,
        global_review_hash,
    ));
    subreview_sources
        .sort_by(|left, right| (left.0.as_str(), left.1).cmp(&(right.0.as_str(), right.1)));
    let critic_census_id = Uuid::new_v4();
    let mut critic_member_hashes = Vec::new();
    for (ordinal, (kind, source_id, source_hash)) in subreview_sources.iter().enumerate() {
        let member_hash = hash_json(
            tx,
            &json!({"member_kind":kind,"source_identity":source_id,"source_hash":source_hash}),
        )
        .await;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_critic_census_members
               VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(Uuid::new_v5(&critic_census_id, member_hash.as_bytes()))
        .bind(critic_census_id)
        .bind(attempt_id)
        .bind(i32::try_from(ordinal).expect("bounded critic ordinal"))
        .bind(kind)
        .bind(source_id)
        .bind(source_hash)
        .bind(&member_hash)
        .execute(&mut **tx)
        .await
        .expect("insert critic census member");
        critic_member_hashes.push(member_hash);
    }
    let critic_set_hash = hash_texts(tx, &critic_member_hashes).await;
    let critic_census_hash = hash_json(
        tx,
        &json!({
            "kind":"critic","attempt":attempt_id,
            "count":subreview_sources.len(),"set":critic_set_hash,
        }),
    )
    .await;
    sqlx::query("INSERT INTO candidate_analysis_critic_censuses VALUES($1,$2,$3,$4,$5)")
        .bind(critic_census_id)
        .bind(attempt_id)
        .bind(i64::try_from(subreview_sources.len()).expect("bounded critic members"))
        .bind(critic_set_hash)
        .bind(critic_census_hash)
        .execute(&mut **tx)
        .await
        .expect("insert critic census");
    (attempt_id, snapshot_id, input_id)
}

#[tokio::test]
#[serial]
async fn exact_closure_accepts_raw_closure_and_rejects_post_seal_proposal_drift() {
    let (db, _data_dir) = fixture().await;
    let pool: &PgPool = db.pool();
    let mut tx = pool.begin().await.expect("begin exact closure transaction");
    create_temp_authority_tables(&mut tx).await;
    let (attempt_id, snapshot_id, _) = seed_closed_candidate_analysis(&mut tx).await;

    let closure = validate_candidate_analysis_exact_closure_on(&mut tx, attempt_id, snapshot_id)
        .await
        .expect("accept independently closed authority rows");
    assert!(closure.gate_eligible);
    assert_eq!(closure.all_input_count, 1);
    assert_eq!(closure.complete_input_count, 1);
    assert!(closure.critic_census_hash.is_some());

    sqlx::query(
        "INSERT INTO hypothesis_proposals(proposal_id,analysis_attempt_id,proposal_ordinal,proposal_hash) VALUES($1,$2,0,$3)",
    )
        .bind(Uuid::new_v4())
        .bind(attempt_id)
        .bind(hash_json(&mut tx, &json!({"late":"proposal"})).await)
        .execute(&mut *tx)
        .await
        .expect("forge post-seal raw proposal drift");
    let error = validate_candidate_analysis_exact_closure_on(&mut tx, attempt_id, snapshot_id)
        .await
        .expect_err("post-seal raw proposal drift must be rejected");
    assert!(error
        .to_string()
        .contains("CANDIDATE_ANALYSIS_CENSUS_NOT_CLOSED"));
    tx.rollback().await.expect("rollback exact closure fixture");
}
