use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use golish_agent_kit::db_traits::{
    CommitEnumerationJsApiProducerV2, EnumerationProducerClosureReceiptV2,
    EnumerationProducerOccurrenceV2, VerifyEnumerationProducerClosureV2,
};
use golish_db::repo::{
    capability_execution_receipts as receipts, enumeration_endpoint_occurrences as enumeration,
};

use super::{tool_truth::stable_denominator_seal_request, GolishDbRepoProvider};

fn stable_child(namespace: Uuid, label: impl AsRef<[u8]>) -> Uuid {
    Uuid::new_v5(&namespace, label.as_ref())
}

fn sha256_prefixed(value: &serde_json::Value) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

fn terminal_outcome(outcome: &str) -> Result<enumeration::EnumerationTerminalInputOutcome> {
    match outcome {
        "found" => Ok(enumeration::EnumerationTerminalInputOutcome::Found),
        "checked_empty" => Ok(enumeration::EnumerationTerminalInputOutcome::CheckedEmpty),
        _ => anyhow::bail!("ENUMERATION_PRODUCER_PARAMETER_OUTCOME_NONTERMINAL"),
    }
}

fn route_kind(value: &str) -> Result<&'static str> {
    match value {
        "resolved_exact" => Ok("exact"),
        "resolved_route_template" => Ok("template"),
        "arbitrary_dynamic" => Ok("dynamic_unresolved"),
        _ => anyhow::bail!("ENUMERATION_PRODUCER_ROUTE_KIND_INVALID"),
    }
}

fn protocol(value: &str, canonical_url: &str) -> Result<&'static str> {
    match value {
        "http" => match reqwest::Url::parse(canonical_url)?.scheme() {
            "http" => Ok("http"),
            "https" => Ok("https"),
            _ => anyhow::bail!("ENUMERATION_PRODUCER_PROTOCOL_INVALID"),
        },
        "websocket" => Ok("websocket"),
        "graphql" => Ok("graphql"),
        _ => anyhow::bail!("ENUMERATION_PRODUCER_PROTOCOL_INVALID"),
    }
}

fn source_anchor(occurrence: &EnumerationProducerOccurrenceV2) -> String {
    let line = occurrence
        .source_span
        .get("start_line")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let column = occurrence
        .source_span
        .get("start_column")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    format!(
        "{}:{line}:{column}:{}",
        occurrence.source_file, occurrence.candidate_id
    )
}

fn receipt_view(
    row: enumeration::EnumerationProducerCommitReceiptRow,
    replayed: bool,
) -> Result<EnumerationProducerClosureReceiptV2> {
    Ok(EnumerationProducerClosureReceiptV2 {
        producer_execution_authority_id: row.execution_authority_id,
        artifact_sha256: row.artifact_sha256,
        receipt_set_sha256: row.receipt_set_sha256,
        js_expected: row.js_expected,
        js_terminal: row.js_terminal,
        candidate_expected: row.candidate_expected,
        candidate_terminal: row.candidate_terminal,
        parameter_expected: row.parameter_expected,
        parameter_terminal: row.parameter_terminal,
        missing: row.missing,
        groups_created: u64::try_from(row.group_count)?,
        occurrence_links_created: u64::try_from(row.occurrence_link_count)?,
        api_links_created: u64::try_from(row.api_link_count)?,
        replayed,
    })
}

async fn exact_origin_subject(
    conn: &mut PgConnection,
    authority: &receipts::ToolTruthExecutionAuthorityRef,
    exact_origin: &str,
    preferred_target_id: Option<Uuid>,
) -> Result<(Uuid, Uuid)> {
    let mut rows = sqlx::query_as::<_, (Uuid, Uuid)>(
        r#"SELECT item.target_id,origin.id
              FROM enumeration_worker_authority_roots root
              JOIN coverage_denominator_items item
                ON item.denominator_id=root.worker_root_denominator_id
               AND item.execution_authority_id=root.worker_execution_authority_id
               AND item.technique='GOLISH-ENUM-JSAPI'
              JOIN targets target ON target.id=item.target_id
              JOIN web_origins origin
                ON origin.organization_id=root.organization_id
               AND origin.project_path=root.project_path_at_freeze
               AND origin.origin=item.exact_asset
             WHERE root.worker_execution_authority_id=$1
               AND root.organization_id=$2
               AND root.project_path_at_freeze=$3
               AND target.organization_id=root.organization_id
               AND target.project_path=root.project_path_at_freeze
               AND origin.origin=$4
               AND ($5::UUID IS NULL OR target.id=$5)
             ORDER BY item.target_id,origin.id
             FOR SHARE OF root,item,origin,target"#,
    )
    .bind(authority.id)
    .bind(authority.organization_id)
    .bind(&authority.project_path_at_freeze)
    .bind(exact_origin)
    .bind(preferred_target_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.sort_unstable();
    rows.dedup();
    ensure!(
        rows.len() == 1,
        "ENUMERATION_PRODUCER_EXACT_ORIGIN_AUTHORITY_AMBIGUOUS"
    );
    Ok(rows[0])
}

async fn begin_and_seal_inputs(
    pool: &sqlx::PgPool,
    authority: &receipts::ToolTruthExecutionAuthorityRef,
    namespace: Uuid,
    denominator: &receipts::CoverageDenominatorRow,
    capability: &str,
    inputs: Vec<enumeration::EnumerationTerminalReceiptInputWrite>,
) -> Result<Vec<receipts::CapabilityReceiptInputRef>> {
    let receipt = receipts::begin(
        pool,
        &receipts::BeginCapabilityReceipt {
            id: stable_child(namespace, format!("receipt:{capability}")),
            denominator_id: denominator.id,
            capability: capability.to_string(),
            attempt_ordinal: 1,
        },
    )
    .await?;
    enumeration::seal_enumeration_terminal_receipt_inputs(
        pool,
        authority,
        &enumeration::SealEnumerationTerminalReceiptInputs {
            stable_seal_request_id: stable_child(namespace, format!("receipt-census:{capability}")),
            receipt_id: receipt.id,
            inputs,
        },
    )
    .await
    .map_err(Into::into)
}

impl GolishDbRepoProvider {
    pub(super) async fn enumeration_commit_js_api_producer_v2_impl(
        &self,
        request: CommitEnumerationJsApiProducerV2,
    ) -> Result<EnumerationProducerClosureReceiptV2> {
        request.artifact.validate_census_and_hash()?;
        ensure!(
            !request.stable_request_id.is_nil()
                && !request.operation_id.is_nil()
                && !request.organization_id.is_nil()
                && !request.stage_execution_id.is_nil()
                && !request.stage_run_unit_id.is_nil()
                && !request.target_id.is_nil()
                && !request.worker_run_id.is_nil()
                && request.worker_attempt_epoch >= 0
                && !request.lease_token.is_nil()
                && !request.source_tool_call_id.is_nil(),
            "ENUMERATION_PRODUCER_COMMIT_IDENTITY_INVALID"
        );
        let lineage = &request.artifact.lineage;
        ensure!(
            lineage.operation_id == request.operation_id
                && lineage.stage_execution_id == request.stage_execution_id
                && lineage.stage_run_unit_id == request.stage_run_unit_id
                && lineage.worker_run_id == request.worker_run_id
                && lineage.worker_attempt_epoch == request.worker_attempt_epoch
                && lineage.tool_call_record_id == request.source_tool_call_id,
            "ENUMERATION_PRODUCER_COMMIT_LINEAGE_MISMATCH"
        );
        let expected_stable_request = stable_child(
            request.source_tool_call_id,
            request.artifact.artifact_sha256.as_bytes(),
        );
        ensure!(
            request.stable_request_id == expected_stable_request,
            "ENUMERATION_PRODUCER_COMMIT_REQUEST_ID_DRIFT"
        );
        let exact_origin = golish_pentest_domain::canonical_web_origin(&request.exact_origin)
            .map(|origin| origin.key)
            .context("ENUMERATION_PRODUCER_EXACT_ORIGIN_INVALID")?;
        ensure!(
            exact_origin == request.exact_origin,
            "ENUMERATION_PRODUCER_EXACT_ORIGIN_NOT_CANONICAL"
        );
        ensure!(
            request
                .artifact
                .occurrences
                .iter()
                .all(|occurrence| occurrence.resolution_status == "resolved"
                    && occurrence.scope_decision == "in_scope"
                    && matches!(
                        occurrence.parameter_assessment.outcome.as_str(),
                        "found" | "checked_empty"
                    )),
            "ENUMERATION_PRODUCER_ARTIFACT_NONTERMINAL"
        );

        let source_root = self
            .tool_truth_seal_denominator_impl(
                golish_agent_kit::db_traits::SealToolTruthDenominatorRequest {
                    stable_seal_request_id: stable_denominator_seal_request(
                        request.stage_execution_id,
                        request.stage_run_unit_id,
                    ),
                    stage_execution_id: request.stage_execution_id,
                    source:
                        golish_agent_kit::db_traits::ToolTruthDenominatorSourceRef::StageTeamUnit {
                            stage_run_unit_id: request.stage_run_unit_id,
                        },
                },
            )
            .await?;
        let root = enumeration::seal_enumeration_worker_authority_root(
            &self.pool,
            &enumeration::SealEnumerationWorkerAuthorityRoot {
                stable_authority_request_id: stable_child(
                    request.stable_request_id,
                    b"worker-authority",
                ),
                stable_root_request_id: stable_child(request.stable_request_id, b"worker-root"),
                source_root_denominator_id: source_root.id,
                worker_fence: enumeration::EnumerationWorkerFence {
                    worker_run_id: request.worker_run_id,
                    worker_attempt_epoch: request.worker_attempt_epoch,
                    lease_token: request.lease_token,
                    source_tool_call_id: request.source_tool_call_id,
                },
            },
        )
        .await?;
        ensure!(
            root.authority.operation_id == request.operation_id
                && root.authority.organization_id == request.organization_id
                && root.authority.stage_execution_id == request.stage_execution_id,
            "ENUMERATION_PRODUCER_ROOT_AUTHORITY_MISMATCH"
        );

        let (source_target_id, source_web_origin_id) = {
            let mut tx = self.pool.begin().await?;
            let subject = exact_origin_subject(
                &mut tx,
                &root.authority,
                &exact_origin,
                Some(request.target_id),
            )
            .await?;
            tx.commit().await?;
            subject
        };
        ensure!(
            source_target_id == request.target_id,
            "ENUMERATION_PRODUCER_SOURCE_TARGET_MISMATCH"
        );
        let root_parent_items = sqlx::query_as::<_, (Uuid,)>(
            r#"SELECT id FROM coverage_denominator_items
                WHERE denominator_id=$1 AND execution_authority_id=$2
                  AND target_id=$3 AND exact_asset=$4
                  AND technique='GOLISH-ENUM-JSAPI'
                ORDER BY id"#,
        )
        .bind(root.root_denominator.id)
        .bind(root.authority.id)
        .bind(request.target_id)
        .bind(&exact_origin)
        .fetch_all(self.pool.as_ref())
        .await?;
        ensure!(
            root_parent_items.len() == 1,
            "ENUMERATION_PRODUCER_ROOT_JSAPI_MEMBER_MISSING"
        );
        let root_parent_item_id = root_parent_items[0].0;

        let (jsapi_evidence, parameter_evidence) = {
            let mut tx = self.pool.begin().await?;
            let jsapi_evidence = enumeration::bind_enumeration_evidence_authorities(
                &mut tx,
                &root.authority,
                &request.artifact.jsapi_evidence_audit_ids,
                "discovery",
            )
            .await?;
            let parameter_evidence = enumeration::bind_enumeration_evidence_authorities(
                &mut tx,
                &root.authority,
                &request.artifact.parameter_evidence_audit_ids,
                "parameter",
            )
            .await?;
            tx.commit().await?;
            (jsapi_evidence, parameter_evidence)
        };

        let js_namespace = stable_child(request.stable_request_id, b"javascript-denominator");
        let js_items = request
            .artifact
            .scripts
            .iter()
            .map(
                |script| enumeration::EnumerationDerivedDenominatorItemWrite {
                    input_key: format!("script:{}:{}", script.source_file, script.content_sha256),
                    target_id: request.target_id,
                    exact_asset: script.manifest_url.clone(),
                    technique: "analyze_script".to_string(),
                    expected_capability: "enumeration.javascript".to_string(),
                },
            )
            .collect::<Vec<_>>();
        let js_denominator = enumeration::seal_enumeration_derived_denominator(
            &self.pool,
            &root.authority,
            &enumeration::SealEnumerationDerivedDenominator {
                stable_seal_request_id: js_namespace,
                parent_denominator_id: root.root_denominator.id,
                parent_denominator_item_id: root_parent_item_id,
                derived_ordinal: 1,
                items: js_items.clone(),
            },
        )
        .await?;
        let js_item_ids = js_items
            .iter()
            .map(|item| {
                (
                    item.input_key.clone(),
                    stable_child(js_denominator.id, item.input_key.as_bytes()),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let mut descriptor_ids = BTreeMap::new();
        {
            let mut tx = self.pool.begin().await?;
            for (ordinal, script) in request.artifact.scripts.iter().enumerate() {
                let input_key = format!("script:{}:{}", script.source_file, script.content_sha256);
                let descriptor_id = stable_child(
                    request.stable_request_id,
                    format!("js-descriptor:{input_key}"),
                );
                enumeration::persist_js_analysis_descriptor(
                    &mut tx,
                    &root.authority,
                    &receipts::CapabilityReceiptInputRef {
                        receipt_id: Uuid::nil(),
                        receipt_input_id: Uuid::nil(),
                        denominator_id: js_denominator.id,
                        denominator_item_id: *js_item_ids
                            .get(&input_key)
                            .context("ENUMERATION_PRODUCER_JS_MEMBER_MISSING")?,
                        logical_input_key: input_key,
                    },
                    &enumeration::JsAnalysisDescriptorWrite {
                        id: descriptor_id,
                        stable_descriptor_request_id: stable_child(
                            request.stable_request_id,
                            format!("js-descriptor-request:{}", script.source_file),
                        ),
                        manifest_url: script.manifest_url.clone(),
                        page_url: exact_origin.clone(),
                        document_url: script
                            .document_bases
                            .first()
                            .cloned()
                            .or_else(|| Some(exact_origin.clone())),
                        chunk_ordinal: i32::try_from(ordinal)?,
                        source_map_url: None,
                        script_sha256: Some(script.content_sha256.clone()),
                        descriptor_metadata: serde_json::json!({
                            "source_urls": script.source_urls,
                            "discovered_from": script.discovered_from,
                            "document_bases": script.document_bases,
                            "capture_kind": "saved_js_manifest",
                            "compatibility_version": "enumeration_js_api_producer_artifact.v2",
                        }),
                    },
                )
                .await?;
                descriptor_ids.insert(script.source_file.clone(), descriptor_id);
            }
            tx.commit().await?;
        }
        let js_receipt_inputs = begin_and_seal_inputs(
            &self.pool,
            &root.authority,
            js_namespace,
            &js_denominator,
            "enumeration.javascript",
            request
                .artifact
                .scripts
                .iter()
                .map(|script| {
                    let input_key =
                        format!("script:{}:{}", script.source_file, script.content_sha256);
                    enumeration::EnumerationTerminalReceiptInputWrite {
                        denominator_item_id: js_item_ids[&input_key],
                        outcome: if request
                            .artifact
                            .occurrences
                            .iter()
                            .any(|occurrence| occurrence.source_file == script.source_file)
                        {
                            enumeration::EnumerationTerminalInputOutcome::Found
                        } else {
                            enumeration::EnumerationTerminalInputOutcome::CheckedEmpty
                        },
                        evidence_authorities: jsapi_evidence.clone(),
                    }
                })
                .collect(),
        )
        .await?;
        let js_inputs_by_key = js_receipt_inputs
            .into_iter()
            .map(|input| (input.logical_input_key.clone(), input))
            .collect::<BTreeMap<_, _>>();
        {
            let mut tx = self.pool.begin().await?;
            for script in &request.artifact.scripts {
                let input_key = format!("script:{}:{}", script.source_file, script.content_sha256);
                enumeration::bind_js_analysis_terminal_receipt(
                    &mut tx,
                    &root.authority,
                    descriptor_ids[&script.source_file],
                    &js_inputs_by_key[&input_key],
                )
                .await?;
            }
            tx.commit().await?;
        }

        let scripts_by_name = request
            .artifact
            .scripts
            .iter()
            .map(|script| (script.source_file.as_str(), script))
            .collect::<BTreeMap<_, _>>();
        let occurrences_by_script = request.artifact.occurrences.iter().fold(
            BTreeMap::<&str, Vec<&EnumerationProducerOccurrenceV2>>::new(),
            |mut grouped, occurrence| {
                grouped
                    .entry(occurrence.source_file.as_str())
                    .or_default()
                    .push(occurrence);
                grouped
            },
        );
        let mut candidate_rows = BTreeMap::new();
        let mut occurrence_rows = BTreeMap::new();
        let mut candidate_denominators = Vec::new();
        for (script_name, occurrences) in occurrences_by_script {
            let script = scripts_by_name
                .get(script_name)
                .context("ENUMERATION_PRODUCER_OCCURRENCE_SCRIPT_MISSING")?;
            let js_input_key = format!("script:{}:{}", script.source_file, script.content_sha256);
            let js_parent_item_id = js_item_ids[&js_input_key];
            let candidate_namespace = stable_child(
                request.stable_request_id,
                format!("candidate-denominator:{script_name}"),
            );
            let candidate_items = occurrences
                .iter()
                .map(
                    |occurrence| enumeration::EnumerationDerivedDenominatorItemWrite {
                        input_key: format!("candidate:{}", occurrence.candidate_id),
                        target_id: request.target_id,
                        exact_asset: occurrence
                            .canonical_url
                            .clone()
                            .expect("producer artifact validation requires a canonical URL"),
                        technique: "extract_endpoint_candidate".to_string(),
                        expected_capability: "enumeration.candidate".to_string(),
                    },
                )
                .collect::<Vec<_>>();
            let candidate_denominator = enumeration::seal_enumeration_derived_denominator(
                &self.pool,
                &root.authority,
                &enumeration::SealEnumerationDerivedDenominator {
                    stable_seal_request_id: candidate_namespace,
                    parent_denominator_id: js_denominator.id,
                    parent_denominator_item_id: js_parent_item_id,
                    derived_ordinal: 2,
                    items: candidate_items.clone(),
                },
            )
            .await?;
            let candidate_item_ids = candidate_items
                .iter()
                .map(|item| {
                    (
                        item.input_key.clone(),
                        stable_child(candidate_denominator.id, item.input_key.as_bytes()),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let candidate_inputs = begin_and_seal_inputs(
                &self.pool,
                &root.authority,
                candidate_namespace,
                &candidate_denominator,
                "enumeration.candidate",
                candidate_items
                    .iter()
                    .map(|item| enumeration::EnumerationTerminalReceiptInputWrite {
                        denominator_item_id: candidate_item_ids[&item.input_key],
                        outcome: enumeration::EnumerationTerminalInputOutcome::Found,
                        evidence_authorities: jsapi_evidence.clone(),
                    })
                    .collect(),
            )
            .await?;
            let candidate_inputs = candidate_inputs
                .into_iter()
                .map(|input| (input.logical_input_key.clone(), input))
                .collect::<BTreeMap<_, _>>();
            {
                let mut tx = self.pool.begin().await?;
                for occurrence in &occurrences {
                    let input_key = format!("candidate:{}", occurrence.candidate_id);
                    let candidate_id = stable_child(
                        root.authority.id,
                        format!(
                            "candidate:{}:{}",
                            request.artifact.artifact_sha256, occurrence.candidate_id
                        ),
                    );
                    let capture_event_id = stable_child(candidate_id, b"capture-event:1");
                    let anchor = source_anchor(occurrence);
                    let fingerprint = sha256_prefixed(&serde_json::json!({
                        "candidate_id": occurrence.candidate_id,
                        "method": occurrence.method,
                        "source_file": occurrence.source_file,
                        "source_span": occurrence.source_span,
                    }))?;
                    enumeration::persist_candidate_descriptor(
                        &mut tx,
                        &root.authority,
                        &candidate_inputs[&input_key],
                        &enumeration::CandidateDescriptorWrite {
                            id: candidate_id,
                            stable_candidate_request_id: stable_child(
                                request.stable_request_id,
                                format!("candidate-request:{}", occurrence.candidate_id),
                            ),
                            js_analysis_item_id: Some(descriptor_ids[script_name]),
                            source_anchor: anchor,
                            callsite_fingerprint: fingerprint.clone(),
                            capture_event_id,
                            capture_attempt_ordinal: 1,
                            captured_at: request.artifact.captured_at,
                            event_fingerprint: fingerprint,
                            duplicate_ordinal: 0,
                            resolution_input: occurrence
                                .canonical_url
                                .clone()
                                .context("ENUMERATION_PRODUCER_CANONICAL_URL_MISSING")?,
                        },
                    )
                    .await?;
                    candidate_rows.insert(
                        occurrence.candidate_id.clone(),
                        (
                            candidate_id,
                            capture_event_id,
                            candidate_inputs[&input_key].clone(),
                        ),
                    );
                }
                tx.commit().await?;
            }
            {
                let mut tx = self.pool.begin().await?;
                for occurrence in &occurrences {
                    let canonical_url = occurrence
                        .canonical_url
                        .as_deref()
                        .context("ENUMERATION_PRODUCER_CANONICAL_URL_MISSING")?;
                    let resolved_origin =
                        golish_pentest_domain::canonical_web_origin(canonical_url)
                            .map(|origin| origin.key)
                            .context("ENUMERATION_PRODUCER_RESOLVED_ORIGIN_INVALID")?;
                    let (resolved_target_id, resolved_web_origin_id) =
                        exact_origin_subject(&mut tx, &root.authority, &resolved_origin, None)
                            .await?;
                    let (candidate_id, capture_event_id, candidate_input) =
                        &candidate_rows[&occurrence.candidate_id];
                    let occurrence_id = stable_child(
                        *candidate_id,
                        format!("occurrence:{}", request.artifact.artifact_sha256),
                    );
                    let mut occurrence_evidence = jsapi_evidence.clone();
                    occurrence_evidence.extend(jsapi_evidence.iter().cloned().map(
                        |mut evidence| {
                            evidence.role = "resolution".to_string();
                            evidence
                        },
                    ));
                    let route_kind = route_kind(&occurrence.route_kind)?;
                    let request_fields = occurrence
                        .parameter_assessment
                        .parameters
                        .iter()
                        .map(|parameter| {
                            serde_json::json!({
                                "name": parameter.name,
                                "location": parameter.location,
                                "type": parameter.value_type,
                                "requirement": parameter.requirement,
                                "source_anchor_ids": parameter.source_anchor_ids,
                            })
                        })
                        .collect::<Vec<_>>();
                    let persisted = enumeration::persist_endpoint_occurrence(
                        &mut tx,
                        &root.authority,
                        candidate_input,
                        &enumeration::EndpointOccurrenceWrite {
                            id: occurrence_id,
                            stable_occurrence_request_id: stable_child(
                                request.stable_request_id,
                                format!("occurrence-request:{}", occurrence.candidate_id),
                            ),
                            candidate_input_id: *candidate_id,
                            capture_event_id: *capture_event_id,
                            source_target_id,
                            source_web_origin_id,
                            resolved_target_id: Some(resolved_target_id),
                            resolved_web_origin_id: Some(resolved_web_origin_id),
                            parent_occurrence_id: None,
                            source_url: exact_origin.clone(),
                            document_url: Some(exact_origin.clone()),
                            script_url: Some(script.manifest_url.clone()),
                            script_sha256: Some(script.content_sha256.clone()),
                            source_span: occurrence.source_span.clone(),
                            initiator_url: None,
                            initiator_status: "not_applicable".to_string(),
                            initiator_line: None,
                            initiator_column: None,
                            cdp_request_id_hash: None,
                            protocol: protocol(&occurrence.protocol, canonical_url)?.to_string(),
                            method: occurrence.method.clone(),
                            graphql_operation_name: None,
                            websocket_subprotocol: None,
                            raw_expression: Some(occurrence.raw_expression.clone()),
                            receiver_kind: occurrence.receiver.clone(),
                            observation_kind: "static_ast".to_string(),
                            inference_level: "deterministic".to_string(),
                            resolution_status: "resolved".to_string(),
                            scope_decision: "in_scope".to_string(),
                            candidate_classification: "endpoint".to_string(),
                            canonical_request_url: Some(canonical_url.to_string()),
                            display_url: Some(canonical_url.to_string()),
                            resolution_reason: occurrence.resolution_reason.clone(),
                            resolution_base_facts: serde_json::json!({
                                "selected_url": canonical_url,
                            }),
                            resolution_candidates: serde_json::json!([]),
                            resolution_chain: occurrence.resolution_chain.clone(),
                            route_kind: route_kind.to_string(),
                            route_template: (route_kind == "template")
                                .then(|| canonical_url.to_string()),
                            request_sent: false,
                            request_schema: serde_json::json!({
                                "schema_version": 2,
                                "fields": request_fields,
                            }),
                            redaction_metadata: serde_json::json!({
                                "redacted": true,
                                "field_count": occurrence.parameter_assessment.parameters.len(),
                                "policy_version": "value_free.v2",
                            }),
                            request_body_length: None,
                            runtime_sample_url: None,
                            observed_at: request.artifact.captured_at,
                        },
                        &occurrence_evidence,
                    )
                    .await?;
                    occurrence_rows.insert(occurrence.candidate_id.clone(), persisted.id);
                }
                tx.commit().await?;
            }

            for occurrence in &occurrences {
                let input_key = format!("candidate:{}", occurrence.candidate_id);
                let candidate_item_id = candidate_item_ids[&input_key];
                let parameter_namespace = stable_child(
                    request.stable_request_id,
                    format!("parameter-denominator:{}", occurrence.candidate_id),
                );
                let parameter_input_key = format!("parameter:{}", occurrence.candidate_id);
                let parameter_denominator = enumeration::seal_enumeration_derived_denominator(
                    &self.pool,
                    &root.authority,
                    &enumeration::SealEnumerationDerivedDenominator {
                        stable_seal_request_id: parameter_namespace,
                        parent_denominator_id: candidate_denominator.id,
                        parent_denominator_item_id: candidate_item_id,
                        derived_ordinal: 3,
                        items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                            input_key: parameter_input_key.clone(),
                            target_id: request.target_id,
                            exact_asset: occurrence
                                .canonical_url
                                .clone()
                                .context("ENUMERATION_PRODUCER_CANONICAL_URL_MISSING")?,
                            technique: "reduce_parameter_facts".to_string(),
                            expected_capability: "enumeration.parameter".to_string(),
                        }],
                    },
                )
                .await?;
                let parameter_item_id =
                    stable_child(parameter_denominator.id, parameter_input_key.as_bytes());
                let mut parameter_refs = parameter_evidence.clone();
                for evidence in &mut parameter_refs {
                    evidence.role = "parameter".to_string();
                }
                let parameter_inputs = begin_and_seal_inputs(
                    &self.pool,
                    &root.authority,
                    parameter_namespace,
                    &parameter_denominator,
                    "enumeration.parameter",
                    vec![enumeration::EnumerationTerminalReceiptInputWrite {
                        denominator_item_id: parameter_item_id,
                        outcome: terminal_outcome(&occurrence.parameter_assessment.outcome)?,
                        evidence_authorities: parameter_refs.clone(),
                    }],
                )
                .await?;
                ensure!(
                    parameter_inputs.len() == 1,
                    "ENUMERATION_PRODUCER_PARAMETER_RECEIPT_DRIFT"
                );
                let assessment_id = stable_child(
                    root.authority.id,
                    format!("parameter-assessment:{}", occurrence.candidate_id),
                );
                let mut tx = self.pool.begin().await?;
                enumeration::persist_parameter_assessment(
                    &mut tx,
                    &root.authority,
                    &parameter_inputs[0],
                    &enumeration::ParameterAssessmentWrite {
                        id: assessment_id,
                        occurrence_id: occurrence_rows[&occurrence.candidate_id],
                        outcome: occurrence.parameter_assessment.outcome.clone(),
                        reason_code: occurrence.parameter_assessment.reason_code.clone(),
                        parameters: occurrence
                            .parameter_assessment
                            .parameters
                            .iter()
                            .enumerate()
                            .map(|(ordinal, parameter)| {
                                Ok(enumeration::OccurrenceParameterWrite {
                                    id: stable_child(
                                        assessment_id,
                                        format!(
                                            "parameter:{ordinal}:{}:{}",
                                            parameter.location, parameter.name
                                        ),
                                    ),
                                    name: parameter.name.clone(),
                                    location: parameter.location.clone(),
                                    value_type: parameter.value_type.clone(),
                                    requirement: parameter.requirement.clone(),
                                    confidence: parameter.confidence,
                                    source_anchor_ids: parameter.source_anchor_ids.clone(),
                                })
                            })
                            .collect::<Result<Vec<_>>>()?,
                    },
                )
                .await?;
                enumeration::bind_parameter_assessment_evidence(
                    &mut tx,
                    &root.authority,
                    assessment_id,
                    &parameter_refs,
                )
                .await?;
                tx.commit().await?;

                enumeration::seal_enumeration_candidate_closure(
                    &self.pool,
                    &root.authority,
                    &enumeration::SealEnumerationCandidateClosure {
                        stable_closure_request_id: stable_child(
                            request.stable_request_id,
                            format!("candidate-closure:{}", occurrence.candidate_id),
                        ),
                        candidate_input_id: candidate_rows[&occurrence.candidate_id].0,
                        resolution_terminal_input: None,
                    },
                )
                .await?;
            }
            enumeration::seal_enumeration_candidate_denominator_closure(
                &self.pool,
                &root.authority,
                &enumeration::SealEnumerationCandidateDenominatorClosure {
                    stable_closure_request_id: stable_child(
                        request.stable_request_id,
                        format!("candidate-denominator-closure:{script_name}"),
                    ),
                    denominator_id: candidate_denominator.id,
                },
            )
            .await?;
            candidate_denominators.push(candidate_denominator.id);
        }
        ensure!(
            !candidate_denominators.is_empty(),
            "ENUMERATION_PRODUCER_CANDIDATE_DENOMINATOR_VACUOUS"
        );

        let mut all_evidence_ids = request.artifact.jsapi_evidence_audit_ids.clone();
        all_evidence_ids.extend(&request.artifact.parameter_evidence_audit_ids);
        all_evidence_ids.sort_unstable();
        all_evidence_ids.dedup();
        let mut tx = self.pool.begin().await?;
        enumeration::project_endpoint_groups(&mut tx, &root.authority).await?;
        let (row, replayed) = enumeration::seal_enumeration_producer_commit_receipt(
            &mut tx,
            &root.authority,
            &enumeration::SealEnumerationProducerCommitReceipt {
                stable_commit_request_id: request.stable_request_id,
                target_id: request.target_id,
                exact_origin,
                artifact_sha256: request.artifact.artifact_sha256,
                evidence_audit_ids: all_evidence_ids,
            },
        )
        .await?;
        tx.commit().await?;
        receipt_view(row, replayed)
    }

    pub(super) async fn enumeration_verify_producer_closure_v2_impl(
        &self,
        request: VerifyEnumerationProducerClosureV2,
    ) -> Result<EnumerationProducerClosureReceiptV2> {
        ensure!(
            !request.operation_id.is_nil()
                && !request.organization_id.is_nil()
                && !request.stage_execution_id.is_nil()
                && !request.stage_run_unit_id.is_nil()
                && !request.target_id.is_nil()
                && !request.producer_execution_authority_id.is_nil(),
            "ENUMERATION_PRODUCER_VERIFIER_IDENTITY_INVALID"
        );
        let exact_origin = golish_pentest_domain::canonical_web_origin(&request.exact_origin)
            .map(|origin| origin.key)
            .context("ENUMERATION_PRODUCER_EXACT_ORIGIN_INVALID")?;
        ensure!(
            exact_origin == request.exact_origin,
            "ENUMERATION_PRODUCER_EXACT_ORIGIN_NOT_CANONICAL"
        );
        let rows = sqlx::query_as::<_, (Uuid, Uuid, String, Uuid, Uuid, Uuid, String)>(
            r#"SELECT id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
                      organization_id,stage_execution_id,authority_hash
                 FROM tool_truth_execution_authorities
                WHERE id=$1 AND operation_id=$2 AND organization_id=$3
                  AND stage_execution_id=$4 AND stage_run_unit_id=$5
                  AND stage_kind='enumeration' AND execution_owner_kind='worker_tool'
                ORDER BY id"#,
        )
        .bind(request.producer_execution_authority_id)
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(request.stage_execution_id)
        .bind(request.stage_run_unit_id)
        .fetch_all(self.pool.as_ref())
        .await?;
        ensure!(
            rows.len() == 1,
            "ENUMERATION_PRODUCER_VERIFIER_AUTHORITY_MISSING"
        );
        let row = &rows[0];
        let authority = receipts::ToolTruthExecutionAuthorityRef {
            id: row.0,
            operation_id: request.operation_id,
            project_scope_id: row.1,
            project_path_at_freeze: row.2.clone(),
            scope_snapshot_id: row.3,
            organization_id: row.4,
            stage_execution_id: row.5,
            authority_hash: row.6.clone(),
        };
        let mut tx = self.pool.begin().await?;
        let row = enumeration::verify_enumeration_producer_commit_receipt(
            &mut tx,
            &authority,
            request.target_id,
            &exact_origin,
            &request.artifact_sha256,
            &request.expected_receipt_set_sha256,
        )
        .await?;
        tx.commit().await?;
        receipt_view(row, true)
    }
}
