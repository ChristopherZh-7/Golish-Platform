use chrono::{Duration, Utc};
use golish_db::models::NewSession;
use golish_db::repo::{
    audit, operation_org_scope, operation_scope_decisions, organizations, project_scopes,
    runtime_memory_tx, scoping_company_identities, sessions, stage_deliverable_submissions,
    tool_calls,
};
use golish_db::{DbConfig, GolishDb};
use serial_test::serial;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::Uuid;

fn reserve_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("reserve local postgres port")
        .local_addr()
        .expect("read reserved postgres port")
        .port()
}

struct ScopeFixture {
    db: GolishDb,
    _data_dir: TempDir,
    project_scope_id: Uuid,
    project_path: String,
    session_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    root_id: Uuid,
    child_id: Uuid,
    foreign_operation_id: Uuid,
    foreign_stage_execution_id: Uuid,
    foreign_root_id: Uuid,
    foreign_child_id: Uuid,
}

impl ScopeFixture {
    async fn start(label: &str) -> Self {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let project_path = format!("/tmp/runtime-scope-{label}");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("runtime_scope_{label}_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start migrated embedded postgres");
        let scope = project_scopes::register_first_open(
            db.pool(),
            &project_path,
            &format!("scope-sha-{label}"),
        )
        .await
        .expect("register project scope");

        let session = sessions::create(
            db.pool(),
            NewSession {
                title: Some("exact scope decision".to_string()),
                workspace_path: Some(project_path.clone()),
                workspace_label: None,
                model: None,
                provider: None,
                project_path: Some(project_path.clone()),
            },
        )
        .await
        .expect("create primary session");
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: stage_execution_id,
                session_id: session.id,
                title: Some("primary scope".to_string()),
                input: "scope primary organization".to_string(),
                profile: "red_team".to_string(),
                entry_stage: "scoping".to_string(),
                project_scope_id: scope.project_scope_id,
                application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
                cli_scope: None,
            },
        )
        .await
        .expect("create primary operation");

        let foreign_session = sessions::create(
            db.pool(),
            NewSession {
                title: Some("foreign scope decision".to_string()),
                workspace_path: Some(project_path.clone()),
                workspace_label: None,
                model: None,
                provider: None,
                project_path: Some(project_path.clone()),
            },
        )
        .await
        .expect("create foreign session");
        let foreign_operation_id = Uuid::new_v4();
        let foreign_stage_execution_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id: foreign_operation_id,
                initial_stage_execution_id: foreign_stage_execution_id,
                session_id: foreign_session.id,
                title: Some("foreign scope".to_string()),
                input: "scope foreign organization".to_string(),
                profile: "red_team".to_string(),
                entry_stage: "scoping".to_string(),
                project_scope_id: scope.project_scope_id,
                application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
                cli_scope: None,
            },
        )
        .await
        .expect("create foreign operation");

        let root = organizations::create(
            db.pool(),
            &project_path,
            "Primary Root",
            None,
            "primary root",
            "fixture",
        )
        .await
        .expect("create primary root");
        let child = organizations::create(
            db.pool(),
            &project_path,
            "Canonical Child",
            Some(root.id),
            "primary child",
            "fixture",
        )
        .await
        .expect("create primary child");
        let foreign_root = organizations::create(
            db.pool(),
            &project_path,
            "Foreign Root",
            None,
            "foreign root",
            "fixture",
        )
        .await
        .expect("create foreign root");
        let foreign_child = organizations::create(
            db.pool(),
            &project_path,
            "Foreign Child",
            Some(foreign_root.id),
            "foreign child",
            "fixture",
        )
        .await
        .expect("create foreign child");
        organizations::update_profile(
            db.pool(),
            root.id,
            &organizations::ProfilePatch {
                intel: Some(serde_json::json!({
                    "engagement": {
                        "candidates": {
                            "organizations": [{
                                "id": "cand-child",
                                "kind": "organization",
                                "label": "Discovered Child",
                                "value": "Discovered Child",
                                "ownershipPercent": "51.00"
                            }],
                            "targets": []
                        }
                    }
                })),
                ..Default::default()
            },
        )
        .await
        .expect("persist primary candidate identity");
        let identity_payload = serde_json::json!({
            "canonical_legal_name": "Primary Root",
            "aliases": [],
            "brands": [],
            "registration_identifiers": {},
        });
        let scope_policy = serde_json::json!({"trusted_roots": ["primary.example"]});
        let evidence = audit::log_evidence(
            db.pool(),
            "scoping_company_identity_fixture",
            "scoping",
            "fixture.company_identity.v1",
            Some(&project_path),
            "fixture",
            None,
            Some(&session.id.to_string()),
            Some("recon_lookup_company"),
            &serde_json::json!({
                "operation_id": operation_id,
                "organization_id": root.id,
                "identity": identity_payload,
            }),
            Some(operation_id),
            None,
            None,
            Some("found"),
        )
        .await
        .expect("record company identity evidence");
        scoping_company_identities::insert_terminal_receipt(
            db.pool(),
            &scoping_company_identities::ScopingCompanyIdentityReceiptRow {
                id: Uuid::new_v4(),
                operation_id,
                stage_execution_id,
                resolution_attempt: 0,
                supersedes_receipt_id: None,
                organization_id: Some(root.id),
                subject_hint: "Primary Root".to_string(),
                canonical_legal_name: Some("Primary Root".to_string()),
                aliases: serde_json::json!([]),
                brands: serde_json::json!([]),
                registration_identifiers: serde_json::json!({}),
                disambiguation_fields: serde_json::json!({}),
                confirmation_method: "exact_reuse".to_string(),
                resolution_status: "confirmed".to_string(),
                scope_policy: scope_policy.clone(),
                source_receipt_refs: serde_json::json!(["fixture:exact_reuse"]),
                artifact_refs: serde_json::json!([]),
                evidence_refs: serde_json::json!([format!("audit:{}", evidence.id)]),
                identity_sha256: prefixed_json_sha256(&identity_payload),
                scope_policy_sha256: prefixed_json_sha256(&scope_policy),
                identity_payload,
            },
        )
        .await
        .expect("freeze confirmed company identity");

        Self {
            db,
            _data_dir: data_dir,
            project_scope_id: scope.project_scope_id,
            project_path,
            session_id: session.id,
            operation_id,
            stage_execution_id,
            root_id: root.id,
            child_id: child.id,
            foreign_operation_id,
            foreign_stage_execution_id,
            foreign_root_id: foreign_root.id,
            foreign_child_id: foreign_child.id,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_call(
        &self,
        operation_id: Uuid,
        stage_execution_id: Uuid,
        session_id: Uuid,
        sequence: i64,
        name: &str,
        args: serde_json::Value,
        result: serde_json::Value,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO tool_calls
               (id, call_id, session_id, task_id, name, args, result, status,
                created_at, operation_id, stage_execution_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'finished', $8, $4, $9)"#,
        )
        .bind(id)
        .bind(format!("scope-call-{sequence}"))
        .bind(session_id)
        .bind(operation_id)
        .bind(name)
        .bind(args)
        .bind(result.to_string())
        .bind(Utc::now() + Duration::milliseconds(sequence))
        .bind(stage_execution_id)
        .execute(self.db.pool())
        .await
        .expect("insert exact Scoping tool call");
        id
    }

    async fn install_included_lifecycle(
        &self,
        reviewed_candidate_id: &str,
        mapped_organization_id: Uuid,
    ) -> Vec<Uuid> {
        let choice = self
            .insert_call(
                self.operation_id,
                self.stage_execution_id,
                self.session_id,
                10,
                "ask_human",
                serde_json::json!({
                    "input_type": "choice",
                    "context": serde_json::json!({
                        "decision": "subsidiary_scope",
                        "organization_id": self.root_id
                    }).to_string()
                }),
                serde_json::json!({"response":"include subsidiaries","skipped":false}),
            )
            .await;
        let proposal = self
            .insert_call(
                self.operation_id,
                self.stage_execution_id,
                self.session_id,
                20,
                "manage_organizations",
                serde_json::json!({
                    "action": "propose_candidates",
                    "organization_id": self.root_id,
                    "candidates": [{"name":"Discovered Child"}]
                }),
                serde_json::json!({
                    "action":"propose_candidates",
                    "organization_id":self.root_id,
                    "recorded":1
                }),
            )
            .await;
        let review_row_id = "review-child";
        let review_response = serde_json::json!({
            "rows": [{
                "reviewRowId": review_row_id,
                "candidateId": reviewed_candidate_id,
                "organizationId": null,
                "name": "Human Edited Child Name",
                "aliases": ["Edited Alias"],
                "domains": ["edited.example"],
                "ownershipPercent": "51.00",
                "included": true
            }]
        });
        let review = self
            .insert_call(
                self.operation_id,
                self.stage_execution_id,
                self.session_id,
                30,
                "ask_human",
                serde_json::json!({
                    "input_type":"unit_review",
                    "context":serde_json::json!({"organization_id":self.root_id}).to_string()
                }),
                serde_json::json!({
                    "response": review_response.to_string(),
                    "skipped": false
                }),
            )
            .await;
        let create = self
            .insert_call(
                self.operation_id,
                self.stage_execution_id,
                self.session_id,
                40,
                "manage_organizations",
                serde_json::json!({
                    "action":"create_batch",
                    "parent_id":self.root_id,
                    "units":[{
                        "review_row_id":review_row_id,
                        "candidate_id":reviewed_candidate_id,
                        "name":"Human Edited Child Name"
                    }]
                }),
                serde_json::json!({
                    "action":"create_batch",
                    "created":[{
                        "review_row_id":review_row_id,
                        "candidate_id":reviewed_candidate_id,
                        "organization_id":mapped_organization_id,
                        "name":"Human Edited Child Name"
                    }],
                    "existing":[],
                    "failed":[]
                }),
            )
            .await;
        vec![choice, proposal, review, create]
    }

    async fn insert_trusted_scoping_submission(&self) -> Uuid {
        self.insert_trusted_scoping_submission_for(
            self.operation_id,
            self.stage_execution_id,
            self.session_id,
        )
        .await
    }

    async fn insert_trusted_scoping_submission_for(
        &self,
        operation_id: Uuid,
        stage_execution_id: Uuid,
        session_id: Uuid,
    ) -> Uuid {
        let request_id = format!("trusted-scoping-submit-{operation_id}");
        let tool_call_record_id = tool_calls::record_tracked_start(
            self.db.pool(),
            &request_id,
            session_id,
            Some(operation_id),
            None,
            "submit_stage_deliverable",
            &serde_json::json!({"stage_id":"scoping"}),
            Some(&tool_calls::RuntimeToolIdentity {
                operation_id,
                stage_execution_id,
                stage_run_unit_id: None,
                worker_run_id: None,
                organization_id: None,
                attempt_epoch: None,
                lease_token: None,
            }),
        )
        .await
        .expect("insert trusted Scoping submit tool call");
        let canonical_payload = format!(
            r#"{{"claims":[],"stage_id":"scoping","stage_run_id":"{}"}}"#,
            stage_execution_id
        );
        let payload_sha256 = Sha256::digest(canonical_payload.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let submission = stage_deliverable_submissions::insert(
            self.db.pool(),
            &stage_deliverable_submissions::NewStageDeliverableSubmission {
                operation_id,
                stage_execution_id,
                stage_run_unit_id: None,
                worker_run_id: None,
                organization_id: None,
                tool_call_record_id,
                tool_request_id: request_id,
                stage_kind: "scoping".to_string(),
                attempt_epoch: None,
                lease_token: None,
                canonical_payload_json: canonical_payload,
                payload_sha256,
            },
        )
        .await
        .expect("insert trusted Scoping submission");
        tool_calls::record_tracked_finish(
            self.db.pool(),
            tool_call_record_id,
            session_id,
            "finished",
            "accepted",
            1,
        )
        .await
        .expect("finish trusted Scoping submit tool call");
        submission.id
    }
}

fn prefixed_json_sha256(value: &serde_json::Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(value).expect("serialize fixture json"));
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn decision_input(fixture: &ScopeFixture) -> operation_scope_decisions::ExactScopeDecisionInput {
    operation_scope_decisions::ExactScopeDecisionInput {
        operation_id: fixture.operation_id,
        project_scope_id: fixture.project_scope_id,
        stage_execution_id: fixture.stage_execution_id,
        root_organization_id: fixture.root_id,
    }
}

#[tokio::test]
#[serial]
async fn trusted_company_intake_freezes_one_evidenced_root_identity_and_replays_exactly() {
    let mut fixture = ScopeFixture::start("trusted_identity").await;
    let input = scoping_company_identities::TrustedCompanyIdentityIntake {
        operation_id: fixture.foreign_operation_id,
        stage_execution_id: fixture.foreign_stage_execution_id,
        organization_id: fixture.foreign_root_id,
        canonical_legal_name: "Foreign Root".to_string(),
        session_id: None,
    };
    let frozen = scoping_company_identities::freeze_trusted_intake(fixture.db.pool(), &input)
        .await
        .expect("freeze trusted root identity");
    assert_eq!(frozen.organization_id, Some(fixture.foreign_root_id));
    assert_eq!(frozen.confirmation_method, "exact_reuse");
    assert_eq!(frozen.resolution_status, "confirmed");
    let evidence_ref = frozen.evidence_refs[0]
        .as_str()
        .expect("typed evidence reference");
    let evidence_id = evidence_ref
        .strip_prefix("audit:")
        .expect("audit evidence prefix")
        .parse::<i64>()
        .expect("audit evidence id");
    let evidence_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE id=$1 AND audit_role='evidence' AND run_id=$2",
    )
    .bind(evidence_id)
    .bind(fixture.foreign_operation_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("load trusted identity evidence");
    assert_eq!(evidence_count, 1);

    let replay = scoping_company_identities::freeze_trusted_intake(fixture.db.pool(), &input)
        .await
        .expect("replay trusted root identity");
    assert_eq!(replay, frozen);

    let child_error = scoping_company_identities::freeze_trusted_intake(
        fixture.db.pool(),
        &scoping_company_identities::TrustedCompanyIdentityIntake {
            organization_id: fixture.foreign_child_id,
            canonical_legal_name: "Foreign Child".to_string(),
            ..input
        },
    )
    .await
    .expect_err("ordinary child organization must not become root identity");
    assert!(child_error
        .to_string()
        .contains("SCOPING_TRUSTED_ORGANIZATION_MISMATCH"));
    fixture.db.stop().await;
}

#[tokio::test]
#[serial]
async fn human_selected_company_identity_is_frozen_before_scope_finalization() {
    let mut fixture = ScopeFixture::start("human_company_identity").await;
    let operation_id = fixture.foreign_operation_id;
    let stage_execution_id = fixture.foreign_stage_execution_id;
    let root_organization_id = fixture.foreign_root_id;
    let session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(operation_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("load foreign operation session");
    let candidate_id = "company-candidate:v1:fixture:foreign-root";
    let candidate = serde_json::json!({
        "candidate_id": candidate_id,
        "provider_id": "fixture",
        "name": "Foreign Root",
        "credit_code": "FOREIGN-CODE-001",
        "industry": "security",
        "legal_representative": "Fixture Owner",
        "address": "Fixture Address",
        "registered_at": "2020-01-01",
        "confidence": 0.68,
        "evidence": {"schema":"company_lookup_provenance.v1","provider_ids":["fixture"]}
    });
    let identity_payload = serde_json::json!({
        "keyword_sha256": "fixture-keyword",
        "candidates": [candidate.clone()]
    });
    let scope_policy = serde_json::json!({"owned_only":true,"reachable_only":true});
    let evidence = audit::log_evidence(
        fixture.db.pool(),
        "scoping_company_resolution_fixture",
        "scoping",
        "fixture.company_resolution.v1",
        Some(&fixture.project_path),
        "fixture",
        None,
        Some(&session_id.to_string()),
        Some("recon_lookup_company"),
        &serde_json::json!({
            "operation_id": operation_id,
            "resolution_status": "needs_human",
            "candidate_id": candidate_id,
        }),
        Some(operation_id),
        None,
        Some("Foreign Root"),
        Some("needs_human"),
    )
    .await
    .expect("record ambiguous company evidence");
    let pending_receipt_id = Uuid::new_v4();
    scoping_company_identities::insert_terminal_receipt(
        fixture.db.pool(),
        &scoping_company_identities::ScopingCompanyIdentityReceiptRow {
            id: pending_receipt_id,
            operation_id,
            stage_execution_id,
            resolution_attempt: 0,
            supersedes_receipt_id: None,
            organization_id: None,
            subject_hint: "Foreign Root".to_string(),
            canonical_legal_name: None,
            aliases: serde_json::json!([]),
            brands: serde_json::json!([]),
            registration_identifiers: serde_json::json!({}),
            disambiguation_fields: serde_json::json!({"candidate_count":1}),
            confirmation_method: "none".to_string(),
            resolution_status: "needs_human".to_string(),
            scope_policy: scope_policy.clone(),
            source_receipt_refs: serde_json::json!([]),
            artifact_refs: serde_json::json!([format!("audit:{}", evidence.id)]),
            evidence_refs: serde_json::json!([format!("audit:{}", evidence.id)]),
            identity_sha256: prefixed_json_sha256(&identity_payload),
            scope_policy_sha256: prefixed_json_sha256(&scope_policy),
            identity_payload,
        },
    )
    .await
    .expect("freeze needs-human company identity");

    let selected_option = "Foreign Root（统一社会信用代码 FOREIGN-CODE-001）";
    fixture
        .insert_call(
            operation_id,
            stage_execution_id,
            session_id,
            10,
            "ask_human",
            serde_json::json!({
                "input_type":"choice",
                "context":serde_json::json!({
                    "decision":"company_identity",
                    "candidates":[candidate]
                }).to_string(),
                "options":[selected_option,"不是这家，我来指定（Other）"]
            }),
            serde_json::json!({"response":selected_option,"skipped":false}),
        )
        .await;
    fixture
        .insert_call(
            operation_id,
            stage_execution_id,
            session_id,
            20,
            "manage_organizations",
            serde_json::json!({"action":"create","name":"Foreign Root"}),
            serde_json::json!({
                "action":"create",
                "id":root_organization_id,
                "name":"Foreign Root"
            }),
        )
        .await;
    fixture
        .insert_call(
            operation_id,
            stage_execution_id,
            session_id,
            30,
            "ask_human",
            serde_json::json!({
                "input_type":"choice",
                "context":serde_json::json!({
                    "decision":"subsidiary_scope",
                    "organization_id":root_organization_id
                }).to_string(),
                "options":["root_only","include_51","include_100"]
            }),
            serde_json::json!({"response":"root_only","skipped":false}),
        )
        .await;
    assert!(
        scoping_company_identities::exact_human_selection_root_is_ready(
            fixture.db.pool(),
            operation_id,
            stage_execution_id,
            root_organization_id,
        )
        .await
        .expect("validate pre-freeze Human selection authority"),
        "the exact Human choice and root create must be usable by Scoping resume"
    );
    assert!(
        !scoping_company_identities::exact_human_selection_root_is_ready(
            fixture.db.pool(),
            operation_id,
            stage_execution_id,
            fixture.root_id,
        )
        .await
        .expect("reject a foreign pre-freeze root"),
        "a root outside the operation's Human/Create witness must remain unauthorized"
    );
    let deliverable_submission_id = fixture
        .insert_trusted_scoping_submission_for(operation_id, stage_execution_id, session_id)
        .await;
    let input = runtime_memory_tx::FinalizeScopingScopeRow {
        operation_id,
        project_scope_id: fixture.project_scope_id,
        stage_execution_id,
        root_organization_id,
        deliverable_submission_id,
        scope_snapshot_id: Uuid::new_v4(),
        scoping_root_unit_id: Uuid::new_v4(),
    };

    let finalized = runtime_memory_tx::finalize_scoping_scope(fixture.db.pool(), &input)
        .await
        .expect("human-selected candidate should freeze before finalization");
    assert_eq!(finalized.scope.units.len(), 1);
    let confirmed =
        scoping_company_identities::get_confirmed_for_operation(fixture.db.pool(), operation_id)
            .await
            .expect("load confirmed company identity")
            .expect("confirmed company identity must exist");
    assert_eq!(confirmed.supersedes_receipt_id, Some(pending_receipt_id));
    assert_eq!(confirmed.organization_id, Some(root_organization_id));
    assert_eq!(
        confirmed.canonical_legal_name.as_deref(),
        Some("Foreign Root")
    );
    assert_eq!(confirmed.confirmation_method, "human_selected");

    fixture.db.stop().await;
}

#[tokio::test]
#[serial]
async fn exact_execution_decision_uses_ordered_stable_ids_and_ignores_cross_operation_rows() {
    let mut fixture = ScopeFixture::start("exact").await;
    let expected_calls = fixture
        .install_included_lifecycle("cand-child", fixture.child_id)
        .await;
    fixture
        .insert_call(
            fixture.foreign_operation_id,
            fixture.foreign_stage_execution_id,
            fixture.session_id,
            25,
            "manage_organizations",
            serde_json::json!({"action":"create_batch","parent_id":fixture.foreign_root_id}),
            serde_json::json!({
                "action":"create_batch",
                "created":[{
                    "review_row_id":"review-child",
                    "candidate_id":"cand-child",
                    "organization_id":fixture.foreign_child_id
                }]
            }),
        )
        .await;

    let lifecycle = tool_calls::scoping_lifecycle_for_execution(
        fixture.db.pool(),
        fixture.operation_id,
        fixture.stage_execution_id,
    )
    .await
    .expect("load exact Scoping lifecycle");
    assert_eq!(
        lifecycle.iter().map(|row| row.id).collect::<Vec<_>>(),
        expected_calls
    );

    let decision =
        operation_scope_decisions::derive_exact(fixture.db.pool(), &decision_input(&fixture))
            .await
            .expect("derive exact approved scope decision");
    assert_eq!(
        decision.mode,
        operation_scope_decisions::ScopeDecisionMode::Included
    );
    assert_eq!(
        decision
            .units
            .iter()
            .map(|unit| unit.organization_id)
            .collect::<Vec<_>>(),
        vec![fixture.root_id, fixture.child_id]
    );
    assert_eq!(decision.units[1].organization_name, "Canonical Child");
    assert_eq!(decision.units[1].ownership_percent.as_deref(), Some("51"));
    assert_eq!(decision.choice_tool_call_id, Some(expected_calls[0]));
    assert_eq!(decision.proposal_tool_call_id, Some(expected_calls[1]));
    assert_eq!(decision.review_tool_call_id, Some(expected_calls[2]));
    assert_eq!(decision.decision_hash.len(), 64);

    fixture.db.stop().await;
}

#[tokio::test]
#[serial]
async fn scoping_passive_recon_authorization_and_scope_derivation_follow_latest_same_root_choice() {
    let mut fixture = ScopeFixture::start("latest-choice").await;
    fixture
        .install_included_lifecycle("cand-child", fixture.child_id)
        .await;

    assert!(
        operation_scope_decisions::scoping_passive_recon_organization_authorized(
            fixture.db.pool(),
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.root_id,
        )
        .await
        .expect("query included-choice passive recon authorization"),
        "the exact included choice authorizes passive subsidiary evidence for its root"
    );
    assert!(
        !operation_scope_decisions::scoping_passive_recon_organization_authorized(
            fixture.db.pool(),
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.foreign_root_id,
        )
        .await
        .expect("query foreign-root passive recon authorization"),
        "a model-supplied foreign root must never inherit the exact choice"
    );

    let latest_choice = fixture
        .insert_call(
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.session_id,
            50,
            "ask_human",
            serde_json::json!({
                "input_type": "choice",
                "context": serde_json::json!({
                    "decision": "subsidiary_scope",
                    "organization_id": fixture.root_id
                }).to_string()
            }),
            serde_json::json!({"response":"root only","skipped":false}),
        )
        .await;

    assert!(
        !operation_scope_decisions::scoping_passive_recon_organization_authorized(
            fixture.db.pool(),
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.root_id,
        )
        .await
        .expect("query latest root-only passive recon authorization"),
        "the latest explicit root-only choice revokes passive subsidiary discovery authorization"
    );
    let decision =
        operation_scope_decisions::derive_exact(fixture.db.pool(), &decision_input(&fixture))
            .await
            .expect("latest same-root choice should deterministically derive scope");
    assert_eq!(
        decision.mode,
        operation_scope_decisions::ScopeDecisionMode::RootOnly
    );
    assert_eq!(decision.choice_tool_call_id, Some(latest_choice));
    assert_eq!(
        decision
            .units
            .iter()
            .map(|unit| unit.organization_id)
            .collect::<Vec<_>>(),
        vec![fixture.root_id]
    );

    fixture.db.stop().await;
}

#[tokio::test]
#[serial]
async fn included_choice_with_empty_current_protocol_review_derives_checked_empty_root_unit_set() {
    let mut fixture = ScopeFixture::start("empty-included").await;
    let choice = fixture
        .insert_call(
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.session_id,
            10,
            "ask_human",
            serde_json::json!({
                "input_type": "choice",
                "context": serde_json::json!({
                    "decision": "subsidiary_scope",
                    "organization_id": fixture.root_id
                }).to_string()
            }),
            serde_json::json!({"response":"include subsidiaries","skipped":false}),
        )
        .await;
    let proposal = fixture
        .insert_call(
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.session_id,
            20,
            "manage_organizations",
            serde_json::json!({
                "action": "propose_candidates",
                "organization_id": fixture.root_id,
                "candidates": []
            }),
            serde_json::json!({
                "action":"propose_candidates",
                "organization_id":fixture.root_id,
                "recorded":0
            }),
        )
        .await;
    let review = fixture
        .insert_call(
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.session_id,
            30,
            "ask_human",
            serde_json::json!({
                "input_type":"unit_review",
                "context":serde_json::json!({"organization_id":fixture.root_id}).to_string()
            }),
            serde_json::json!({
                "response": serde_json::json!({"rows":[]}).to_string(),
                "skipped": false
            }),
        )
        .await;

    assert!(
        operation_scope_decisions::scoping_passive_recon_organization_authorized(
            fixture.db.pool(),
            fixture.operation_id,
            fixture.stage_execution_id,
            fixture.root_id,
        )
        .await
        .expect("query exact empty subsidiary discovery authorization")
    );
    let decision =
        operation_scope_decisions::derive_exact(fixture.db.pool(), &decision_input(&fixture))
            .await
            .expect("derive an included-but-checked-empty scope decision");
    assert_eq!(
        decision.mode,
        operation_scope_decisions::ScopeDecisionMode::Included
    );
    assert_eq!(decision.choice_tool_call_id, Some(choice));
    assert_eq!(decision.proposal_tool_call_id, Some(proposal));
    assert_eq!(decision.review_tool_call_id, Some(review));
    assert_eq!(
        decision
            .units
            .iter()
            .map(|unit| unit.organization_id)
            .collect::<Vec<_>>(),
        vec![fixture.root_id],
        "zero qualifying subsidiaries is checked-empty, not an absent review"
    );

    fixture.db.stop().await;
}

#[tokio::test]
#[serial]
async fn candidate_and_foreign_organization_rebinding_fail_without_snapshot() {
    for (label, candidate_id, mapped_organization) in [
        ("foreign-candidate", "cand-from-another-root", None),
        ("foreign-org", "cand-child", Some(Uuid::nil())),
    ] {
        let mut fixture = ScopeFixture::start(label).await;
        let mapped = mapped_organization.unwrap_or(fixture.foreign_child_id);
        fixture
            .install_included_lifecycle(candidate_id, mapped)
            .await;
        let error =
            operation_scope_decisions::derive_exact(fixture.db.pool(), &decision_input(&fixture))
                .await
                .expect_err("hostile scope identity rebinding must fail closed");
        assert_eq!(error.code(), "scope_decision_row_mismatch");
        assert!(
            operation_org_scope::load_for_operation(fixture.db.pool(), fixture.operation_id,)
                .await
                .expect("load absent hostile snapshot")
                .is_none()
        );
        fixture.db.stop().await;
    }
}

#[tokio::test]
#[serial]
async fn canonical_scope_hash_is_order_independent_and_history_blocks_subtree_delete() {
    let mut fixture = ScopeFixture::start("freeze").await;
    fixture
        .install_included_lifecycle("cand-child", fixture.child_id)
        .await;
    let decision =
        operation_scope_decisions::derive_exact(fixture.db.pool(), &decision_input(&fixture))
            .await
            .expect("derive scope decision before freeze");
    let draft = operation_org_scope::NewOperationOrgScope::from_decision(
        Uuid::new_v4(),
        fixture.project_path.clone(),
        &decision,
    )
    .expect("build canonical scope draft");
    let mut reversed = draft.clone();
    reversed.units.reverse();
    assert_eq!(
        operation_org_scope::canonical_scope_hash(&draft).expect("hash ordered draft"),
        operation_org_scope::canonical_scope_hash(&reversed).expect("hash reversed draft")
    );

    let mut tx = fixture.db.pool().begin().await.expect("begin scope freeze");
    let frozen = operation_org_scope::freeze_with_connection(&mut tx, &draft)
        .await
        .expect("freeze immutable scope snapshot");
    tx.commit().await.expect("commit scope freeze");
    assert_eq!(frozen.units.len(), 2);
    assert_eq!(frozen.snapshot.scope_hash.len(), 64);
    assert!(operation_org_scope::history_exists_for_org_subtree(
        fixture.db.pool(),
        fixture.root_id,
    )
    .await
    .expect("query runtime scope history"));
    assert!(operation_org_scope::history_exists_for_org_subtree(
        fixture.db.pool(),
        fixture.child_id,
    )
    .await
    .expect("query child runtime scope history"));

    fixture.db.stop().await;
}

#[tokio::test]
#[serial]
async fn finalize_scoping_scope_atomically_binds_submission_and_replays_without_closing_execution()
{
    let mut fixture = ScopeFixture::start("finalize").await;
    fixture
        .install_included_lifecycle("cand-child", fixture.child_id)
        .await;
    let deliverable_submission_id = fixture.insert_trusted_scoping_submission().await;
    let input = runtime_memory_tx::FinalizeScopingScopeRow {
        operation_id: fixture.operation_id,
        project_scope_id: fixture.project_scope_id,
        stage_execution_id: fixture.stage_execution_id,
        root_organization_id: fixture.root_id,
        deliverable_submission_id,
        scope_snapshot_id: Uuid::new_v4(),
        scoping_root_unit_id: Uuid::new_v4(),
    };

    let finalized = runtime_memory_tx::finalize_scoping_scope(fixture.db.pool(), &input)
        .await
        .expect("atomically finalize Scoping scope");
    assert!(!finalized.replayed);
    assert_eq!(finalized.scope.snapshot.id, input.scope_snapshot_id);
    assert_eq!(finalized.root_unit.id, input.scoping_root_unit_id);
    assert_eq!(finalized.root_unit.status, "passed");
    assert_eq!(
        finalized.submission.stage_run_unit_id,
        Some(input.scoping_root_unit_id)
    );
    assert_eq!(finalized.submission.organization_id, Some(fixture.root_id));
    let stage_status: String = sqlx::query_scalar("SELECT status FROM stage_runs WHERE id=$1")
        .bind(fixture.stage_execution_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read Scoping execution status");
    assert_eq!(
        stage_status, "started",
        "Task3 closes/opens on the next stage entry; finalization must not create an active gap"
    );

    let replayed = runtime_memory_tx::finalize_scoping_scope(fixture.db.pool(), &input)
        .await
        .expect("idempotently replay finalized scope");
    assert!(replayed.replayed);
    assert_eq!(replayed.scope, finalized.scope);
    assert_eq!(replayed.root_unit, finalized.root_unit);
    fixture.db.stop().await;
}

#[tokio::test]
#[serial]
async fn freeze_failure_rolls_back_decision_snapshot_unit_and_submission_binding() {
    let mut fixture = ScopeFixture::start("rollback").await;
    fixture
        .install_included_lifecycle("cand-child", fixture.child_id)
        .await;
    let deliverable_submission_id = fixture.insert_trusted_scoping_submission().await;
    sqlx::raw_sql(
        r#"CREATE FUNCTION reject_scope_root_unit_fixture()
           RETURNS trigger AS $$
           BEGIN
               RAISE EXCEPTION 'injected root-unit failure';
           END;
           $$ LANGUAGE plpgsql;
           CREATE TRIGGER reject_scope_root_unit_fixture
           BEFORE INSERT ON stage_run_units
           FOR EACH ROW EXECUTE FUNCTION reject_scope_root_unit_fixture();"#,
    )
    .execute(fixture.db.pool())
    .await
    .expect("install root-unit failure trigger");
    let input = runtime_memory_tx::FinalizeScopingScopeRow {
        operation_id: fixture.operation_id,
        project_scope_id: fixture.project_scope_id,
        stage_execution_id: fixture.stage_execution_id,
        root_organization_id: fixture.root_id,
        deliverable_submission_id,
        scope_snapshot_id: Uuid::new_v4(),
        scoping_root_unit_id: Uuid::new_v4(),
    };

    assert!(
        runtime_memory_tx::finalize_scoping_scope(fixture.db.pool(), &input)
            .await
            .is_err()
    );
    let decision_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM operation_scope_decisions WHERE operation_id=$1")
            .bind(fixture.operation_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count rolled-back decisions");
    let snapshot_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM operation_org_scope_snapshots WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("count rolled-back snapshots");
    assert_eq!((decision_count, snapshot_count), (0, 0));
    let binding: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT stage_run_unit_id, organization_id FROM stage_deliverable_submissions WHERE id=$1",
    )
    .bind(deliverable_submission_id)
    .fetch_one(fixture.db.pool())
    .await
    .expect("read rolled-back submission binding");
    assert_eq!(binding, (None, None));
    let stage_status: String = sqlx::query_scalar("SELECT status FROM stage_runs WHERE id=$1")
        .bind(fixture.stage_execution_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("read Scoping execution after rollback");
    assert_eq!(stage_status, "started");
    fixture.db.stop().await;
}
