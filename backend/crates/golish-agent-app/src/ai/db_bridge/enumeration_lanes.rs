use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use golish_agent_kit::db_traits::{
    CloseEnumerationResolutionV2, CommitEnumerationBrowserProducerV2,
    CommitEnumerationJsApiProducerV2, EnumerationBrowserOccurrenceV2,
    EnumerationLaneClosureReceiptV2, EnumerationLaneKindV2, EnumerationProducerLineageV2,
    EnumerationProducerOccurrenceV2, EnumerationProducerParameterFactV2,
    EnumerationProducerScriptV2, EnumerationResolutionTerminalStateV2,
    ReduceEnumerationParameterV2, ReviewEnumerationCoverageV2,
};
use golish_db::repo::{
    capability_execution_receipts as receipts, enumeration_endpoint_occurrences as enumeration,
};

use super::{tool_truth::stable_denominator_seal_request, GolishDbRepoProvider};

#[derive(Debug, FromRow)]
struct ParameterOccurrenceRow {
    id: Uuid,
    source_url: String,
    canonical_request_url: Option<String>,
    resolution_status: String,
    scope_decision: String,
    request_schema: serde_json::Value,
}

#[derive(Debug, FromRow)]
struct ResolutionOccurrenceRow {
    id: Uuid,
    candidate_input_id: Uuid,
    candidate_denominator_id: Uuid,
}

fn stable_child(namespace: Uuid, label: impl AsRef<[u8]>) -> Uuid {
    Uuid::new_v5(&namespace, label.as_ref())
}

fn sha256_prefixed(value: &serde_json::Value) -> Result<String> {
    let digest = Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

fn canonical_origin(value: &str) -> Result<String> {
    let canonical = golish_pentest_domain::canonical_web_origin(value)
        .map(|origin| origin.key)
        .context("ENUMERATION_LANE_EXACT_ORIGIN_INVALID")?;
    ensure!(
        canonical == value,
        "ENUMERATION_LANE_EXACT_ORIGIN_NOT_CANONICAL"
    );
    Ok(canonical)
}

fn validate_identity(
    stable_request_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    target_id: Uuid,
    worker_run_id: Uuid,
    worker_attempt_epoch: i64,
    lease_token: Uuid,
    source_tool_call_id: Uuid,
) -> Result<()> {
    ensure!(
        !stable_request_id.is_nil()
            && !operation_id.is_nil()
            && !organization_id.is_nil()
            && !stage_execution_id.is_nil()
            && !stage_run_unit_id.is_nil()
            && !target_id.is_nil()
            && !worker_run_id.is_nil()
            && worker_attempt_epoch >= 0
            && !lease_token.is_nil()
            && !source_tool_call_id.is_nil(),
        "ENUMERATION_LANE_IDENTITY_INVALID"
    );
    Ok(())
}

fn validate_lineage(
    lineage: &EnumerationProducerLineageV2,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Uuid,
    worker_attempt_epoch: i64,
    source_tool_call_id: Uuid,
) -> Result<()> {
    ensure!(
        lineage.operation_id == operation_id
            && lineage.stage_execution_id == stage_execution_id
            && lineage.stage_run_unit_id == stage_run_unit_id
            && lineage.worker_run_id == worker_run_id
            && lineage.worker_attempt_epoch == worker_attempt_epoch
            && lineage.tool_call_record_id == source_tool_call_id,
        "ENUMERATION_LANE_LINEAGE_MISMATCH"
    );
    Ok(())
}

fn canonical_ids<T: Ord + Copy>(values: &[T]) -> Vec<T> {
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    values
}

pub(super) fn lane_receipt_view(
    row: enumeration::EnumerationLaneCommitReceiptRow,
    replayed: bool,
) -> Result<EnumerationLaneClosureReceiptV2> {
    let lane = match row.lane.as_str() {
        "browser" => EnumerationLaneKindV2::Browser,
        "js_api" => EnumerationLaneKindV2::JsApi,
        "parameter" => EnumerationLaneKindV2::Parameter,
        "resolution" => EnumerationLaneKindV2::Resolution,
        "coverage" => EnumerationLaneKindV2::Coverage,
        _ => anyhow::bail!("ENUMERATION_LANE_RECEIPT_KIND_INVALID"),
    };
    let view = EnumerationLaneClosureReceiptV2 {
        receipt_id: row.id,
        lane,
        execution_authority_id: row.execution_authority_id,
        artifact_sha256: row.artifact_sha256,
        receipt_set_sha256: row.receipt_set_sha256,
        closure_graph_sha256: row.closure_graph_sha256,
        dependency_receipt_ids: row.dependency_receipt_ids,
        evidence_audit_ids: row.evidence_audit_ids,
        script_denominator_id: row.script_denominator_id,
        candidate_denominator_ids: row.candidate_denominator_ids,
        parameter_denominator_ids: row.parameter_denominator_ids,
        resolution_occurrence_id: row.resolution_occurrence_id,
        resolution_terminal_receipt_id: row.resolution_terminal_receipt_id,
        resolution_terminal_receipt_input_id: row.resolution_terminal_receipt_input_id,
        terminal_disposition: row.terminal_disposition,
        entity_set_sha256: row.entity_set_sha256,
        denominator_set_sha256: row.denominator_set_sha256,
        script_count: row.script_count,
        candidate_count: row.candidate_count,
        occurrence_count: row.occurrence_count,
        parameter_assessment_count: row.parameter_assessment_count,
        parameter_fact_count: row.parameter_fact_count,
        unresolved_count: row.unresolved_count,
        group_count: row.group_count,
        occurrence_link_count: row.occurrence_link_count,
        api_link_count: row.api_link_count,
        missing: row.missing,
        replayed,
    };
    ensure!(view.is_terminal(), "ENUMERATION_LANE_RECEIPT_NONTERMINAL");
    Ok(view)
}

fn receipt_immutable_eq(
    left: &EnumerationLaneClosureReceiptV2,
    right: &EnumerationLaneClosureReceiptV2,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.replayed = false;
    right.replayed = false;
    left == right
}

async fn load_named_receipt(
    conn: &mut PgConnection,
    receipt: &EnumerationLaneClosureReceiptV2,
    expected_lane: EnumerationLaneKindV2,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    target_id: Uuid,
    exact_origin: &str,
) -> Result<enumeration::EnumerationLaneCommitReceiptRow> {
    ensure!(
        receipt.lane == expected_lane && receipt.is_terminal(),
        "ENUMERATION_LANE_DEPENDENCY_NONTERMINAL"
    );
    let lane = match expected_lane {
        EnumerationLaneKindV2::Browser => "browser",
        EnumerationLaneKindV2::JsApi => "js_api",
        EnumerationLaneKindV2::Parameter => "parameter",
        EnumerationLaneKindV2::Resolution => "resolution",
        EnumerationLaneKindV2::Coverage => "coverage",
    };
    let row = enumeration::load_enumeration_lane_commit_receipt(
        conn,
        receipt.receipt_id,
        operation_id,
        organization_id,
        stage_execution_id,
        stage_run_unit_id,
        target_id,
        exact_origin,
        lane,
        receipt.execution_authority_id,
        &receipt.receipt_set_sha256,
    )
    .await?;
    ensure!(
        receipt_immutable_eq(receipt, &lane_receipt_view(row.clone(), false)?),
        "ENUMERATION_LANE_DEPENDENCY_RECEIPT_DRIFT"
    );
    Ok(row)
}

async fn authority_for_lane_receipt(
    conn: &mut PgConnection,
    row: &enumeration::EnumerationLaneCommitReceiptRow,
) -> Result<receipts::ToolTruthExecutionAuthorityRef> {
    let authority = sqlx::query_as::<_, (Uuid, Uuid, String, Uuid, Uuid, Uuid, String)>(
        r#"SELECT id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
                  organization_id,stage_execution_id,authority_hash
             FROM tool_truth_execution_authorities
            WHERE id=$1 AND operation_id=$2 AND organization_id=$3
              AND stage_execution_id=$4 AND stage_run_unit_id=$5
              AND stage_kind='enumeration' AND execution_owner_kind='worker_tool'
            FOR SHARE"#,
    )
    .bind(row.execution_authority_id)
    .bind(row.operation_id)
    .bind(row.organization_id)
    .bind(row.stage_execution_id)
    .bind(row.stage_run_unit_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| anyhow::anyhow!("ENUMERATION_LANE_DEPENDENCY_AUTHORITY_MISSING"))?;
    Ok(receipts::ToolTruthExecutionAuthorityRef {
        id: authority.0,
        operation_id: row.operation_id,
        project_scope_id: authority.1,
        project_path_at_freeze: authority.2,
        scope_snapshot_id: authority.3,
        organization_id: authority.4,
        stage_execution_id: authority.5,
        authority_hash: authority.6,
    })
}

async fn response_loss_replay(
    conn: &mut PgConnection,
    stable_request_id: Uuid,
    lane: &str,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    target_id: Uuid,
    exact_origin: &str,
    artifact_sha256: &str,
    expected_dependencies: &[Uuid],
    expected_evidence: &[i64],
) -> Result<Option<EnumerationLaneClosureReceiptV2>> {
    let row = enumeration::load_enumeration_lane_commit_receipt_by_stable_request(
        conn,
        stable_request_id,
        lane,
        operation_id,
        organization_id,
        stage_execution_id,
        stage_run_unit_id,
        target_id,
        exact_origin,
        artifact_sha256,
    )
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    ensure!(
        row.dependency_receipt_ids == canonical_ids(expected_dependencies)
            && (expected_evidence.is_empty()
                || row.evidence_audit_ids == canonical_ids(expected_evidence)),
        "ENUMERATION_LANE_REPLAY_MANIFEST_DRIFT"
    );
    Ok(Some(lane_receipt_view(row, true)?))
}

async fn exact_root_subject(
    conn: &mut PgConnection,
    authority: &receipts::ToolTruthExecutionAuthorityRef,
    root_denominator_id: Uuid,
    target_id: Uuid,
    exact_origin: &str,
    technique: &str,
) -> Result<(Uuid, Uuid)> {
    let rows = sqlx::query_as::<_, (Uuid, Uuid)>(
        r#"SELECT item.id,origin.id
              FROM coverage_denominator_items item
              JOIN web_origins origin
                ON origin.organization_id=$5 AND origin.project_path=$6
               AND origin.origin=item.exact_asset
              JOIN targets target
                ON target.id=item.target_id AND target.organization_id=$5
               AND target.project_path=$6
             WHERE item.denominator_id=$1 AND item.execution_authority_id=$2
               AND item.target_id=$3 AND item.exact_asset=$4
               AND item.technique=$7
             ORDER BY item.id,origin.id
             FOR SHARE OF item,origin,target"#,
    )
    .bind(root_denominator_id)
    .bind(authority.id)
    .bind(target_id)
    .bind(exact_origin)
    .bind(authority.organization_id)
    .bind(&authority.project_path_at_freeze)
    .bind(technique)
    .fetch_all(&mut *conn)
    .await?;
    ensure!(
        rows.len() == 1,
        "ENUMERATION_LANE_EXACT_ROOT_SUBJECT_MISSING"
    );
    Ok(rows[0])
}

const LOCKED_FROZEN_ORIGIN_SUBJECT_SQL: &str = r#"SELECT item.target_id,origin.id
              FROM enumeration_worker_authority_roots root
              JOIN coverage_denominator_items item
                ON item.denominator_id=root.worker_root_denominator_id
               AND item.execution_authority_id=root.worker_execution_authority_id
              JOIN web_origins origin
                ON origin.organization_id=root.organization_id
               AND origin.project_path=root.project_path_at_freeze
               AND origin.origin=item.exact_asset
              JOIN targets target
                ON target.id=item.target_id AND target.organization_id=root.organization_id
               AND target.project_path=root.project_path_at_freeze
             WHERE root.worker_execution_authority_id=$1
               AND item.target_id=$2 AND origin.origin=$3
             ORDER BY item.target_id,origin.id
             FOR SHARE OF root,item,origin,target"#;

async fn exact_frozen_origin_subject(
    conn: &mut PgConnection,
    authority: &receipts::ToolTruthExecutionAuthorityRef,
    source_target_id: Uuid,
    canonical_url: &str,
) -> Result<(Uuid, Uuid)> {
    ensure!(
        !source_target_id.is_nil(),
        "ENUMERATION_LANE_RESOLVED_TARGET_MISSING"
    );
    let exact_origin = golish_pentest_domain::canonical_web_origin(canonical_url)
        .map(|origin| origin.key)
        .context("ENUMERATION_LANE_RESOLVED_ORIGIN_INVALID")?;
    let mut rows = sqlx::query_as::<_, (Uuid, Uuid)>(LOCKED_FROZEN_ORIGIN_SUBJECT_SQL)
        .bind(authority.id)
        .bind(source_target_id)
        .bind(exact_origin)
        .fetch_all(&mut *conn)
        .await?;
    // The same root subject may be represented by more than one technique
    // item. Keep the base rows locked above, then collapse those duplicate
    // projections in Rust: PostgreSQL rejects row locking on a DISTINCT query.
    rows.sort_unstable();
    rows.dedup();
    ensure!(
        rows.len() == 1,
        "ENUMERATION_LANE_RESOLVED_ORIGIN_OUTSIDE_FROZEN_ROOT"
    );
    Ok(rows[0])
}

#[cfg(test)]
mod exact_frozen_origin_subject_tests {
    use chrono::Utc;

    use super::{
        coverage_lane_command, receipts, resolution_lane_command,
        terminal_input_census_is_nonempty, LOCKED_FROZEN_ORIGIN_SUBJECT_SQL,
    };

    #[test]
    fn frozen_origin_lookup_pins_aliases_to_the_dispatched_target() {
        assert!(LOCKED_FROZEN_ORIGIN_SUBJECT_SQL.contains("item.target_id=$2"));
        assert!(LOCKED_FROZEN_ORIGIN_SUBJECT_SQL.contains("origin.origin=$3"));
    }

    fn denominator(member_count: i64) -> receipts::CoverageDenominatorRow {
        receipts::CoverageDenominatorRow {
            id: uuid::Uuid::new_v4(),
            stable_seal_request_id: uuid::Uuid::new_v4(),
            execution_authority_id: uuid::Uuid::new_v4(),
            contract: "receipt_v1".to_string(),
            input_manifest_hash: "sha256:test".to_string(),
            member_count: Some(member_count),
            member_set_hash: Some("sha256:test".to_string()),
            denominator_hash: "sha256:test".to_string(),
            sealed_at: Some(Utc::now()),
        }
    }

    #[test]
    fn locked_subject_query_deduplicates_after_locking_base_rows() {
        let source = include_str!("enumeration_lanes.rs");
        let query = source
            .split("const LOCKED_FROZEN_ORIGIN_SUBJECT_SQL")
            .nth(1)
            .expect("locked subject query")
            .split(";\n")
            .next()
            .expect("query terminator");

        assert!(query.contains("FOR SHARE OF root,item,origin,target"));
        assert!(!query.contains("SELECT DISTINCT"));
        assert!(source.contains("rows.sort_unstable();\n    rows.dedup();"));
    }

    #[test]
    fn sealed_empty_denominator_skips_impossible_generic_input_receipt() {
        assert!(!terminal_input_census_is_nonempty(&denominator(0), 0)
            .expect("sealed-empty census is valid"));
    }

    #[test]
    fn terminal_input_count_must_equal_the_sealed_denominator() {
        let error = terminal_input_census_is_nonempty(&denominator(0), 1)
            .expect_err("caller cannot add a sentinel to a sealed-empty census");
        assert!(error
            .to_string()
            .contains("ENUMERATION_TERMINAL_INPUT_DENOMINATOR_DRIFT"));
    }

    #[test]
    fn resolution_lane_receipt_binds_only_its_derived_evidence() {
        let stable_request_id = uuid::Uuid::new_v4();
        let target_id = uuid::Uuid::new_v4();
        let dependency_receipt_id = uuid::Uuid::new_v4();
        let occurrence_id = uuid::Uuid::new_v4();
        let terminal_receipt_id = uuid::Uuid::new_v4();
        let terminal_input_id = uuid::Uuid::new_v4();
        let derived_evidence_ids = vec![7001];
        let command = resolution_lane_command(
            stable_request_id,
            target_id,
            "https://example.test:443".to_string(),
            format!("sha256:{}", "a".repeat(64)),
            vec![dependency_receipt_id],
            derived_evidence_ids.clone(),
            occurrence_id,
            terminal_receipt_id,
            terminal_input_id,
        );

        assert_eq!(command.lane, "resolution");
        assert_eq!(command.evidence_audit_ids, derived_evidence_ids);
        assert_eq!(command.dependency_receipt_ids, vec![dependency_receipt_id]);
        assert_eq!(command.resolution_occurrence_id, Some(occurrence_id));
        assert_eq!(
            command.resolution_terminal_receipt_input_id,
            Some(terminal_input_id)
        );
    }

    #[test]
    fn coverage_lane_receipt_binds_only_its_derived_evidence() {
        let stable_request_id = uuid::Uuid::new_v4();
        let target_id = uuid::Uuid::new_v4();
        let dependency_receipt_ids = vec![uuid::Uuid::new_v4(), uuid::Uuid::new_v4()];
        let derived_evidence_ids = vec![8001];
        let command = coverage_lane_command(
            stable_request_id,
            target_id,
            "https://example.test:443".to_string(),
            format!("sha256:{}", "b".repeat(64)),
            dependency_receipt_ids.clone(),
            derived_evidence_ids.clone(),
        );

        assert_eq!(command.lane, "coverage");
        assert_eq!(command.dependency_receipt_ids, dependency_receipt_ids);
        assert_eq!(command.evidence_audit_ids, derived_evidence_ids);
    }
}

fn terminal_input_census_is_nonempty(
    denominator: &receipts::CoverageDenominatorRow,
    supplied_count: i64,
) -> Result<bool> {
    ensure!(
        denominator.sealed_at.is_some() && denominator.member_count == Some(supplied_count),
        "ENUMERATION_TERMINAL_INPUT_DENOMINATOR_DRIFT"
    );
    Ok(supplied_count > 0)
}

async fn begin_and_seal_inputs(
    conn: &mut PgConnection,
    authority: &receipts::ToolTruthExecutionAuthorityRef,
    namespace: Uuid,
    denominator: &receipts::CoverageDenominatorRow,
    capability: &str,
    inputs: Vec<enumeration::EnumerationTerminalReceiptInputWrite>,
) -> Result<Vec<receipts::CapabilityReceiptInputRef>> {
    let supplied_count = i64::try_from(inputs.len())?;
    let has_inputs = terminal_input_census_is_nonempty(denominator, supplied_count)?;
    // A sealed-empty derived denominator is itself the checked-empty census.
    // The generic Tool Truth receipt begin path deliberately requires a
    // matching denominator item/capability, which cannot exist for a true
    // zero-member set.  Enumeration lane receipts and the candidate
    // denominator closure seal bind that empty census; do not manufacture a
    // sentinel item (which would turn checked-empty into a fake observation).
    if !has_inputs {
        return Ok(Vec::new());
    }
    let receipt_id = stable_child(namespace, format!("receipt:{capability}"));
    receipts::begin_in_connection(
        conn,
        &receipts::BeginCapabilityReceipt {
            id: receipt_id,
            denominator_id: denominator.id,
            capability: capability.to_string(),
            attempt_ordinal: 1,
        },
    )
    .await?;
    Ok(
        enumeration::seal_enumeration_terminal_receipt_inputs_in_connection(
            conn,
            authority,
            &enumeration::SealEnumerationTerminalReceiptInputs {
                stable_seal_request_id: stable_child(
                    namespace,
                    format!("receipt-census:{capability}"),
                ),
                receipt_id,
                inputs,
            },
        )
        .await?,
    )
}

async fn persist_scripts(
    conn: &mut PgConnection,
    authority: &receipts::ToolTruthExecutionAuthorityRef,
    namespace: Uuid,
    exact_origin: &str,
    scripts: &[EnumerationProducerScriptV2],
    evidence: &[receipts::EvidenceAuthorityRef],
    root_denominator_id: Uuid,
    root_item_id: Uuid,
    derived_ordinal: i32,
    compatibility_version: &str,
) -> Result<(
    receipts::CoverageDenominatorRow,
    BTreeMap<String, Uuid>,
    BTreeMap<String, receipts::CapabilityReceiptInputRef>,
)> {
    let target_id: Uuid = sqlx::query_scalar(
        r#"SELECT item.target_id FROM coverage_denominator_items item
            WHERE item.denominator_id=$1 AND item.id=$2
              AND item.execution_authority_id=$3 FOR SHARE"#,
    )
    .bind(root_denominator_id)
    .bind(root_item_id)
    .bind(authority.id)
    .fetch_one(&mut *conn)
    .await?;
    let items = scripts
        .iter()
        .map(
            |script| enumeration::EnumerationDerivedDenominatorItemWrite {
                input_key: format!("script:{}:{}", script.source_file, script.content_sha256),
                target_id,
                exact_asset: script.manifest_url.clone(),
                technique: "analyze_script".to_string(),
                expected_capability: "enumeration.javascript".to_string(),
            },
        )
        .collect::<Vec<_>>();
    // The target id is server-read from the exact root rather than accepted in
    // each child. Resolve the futures-free placeholder above deterministically.
    let target_id: Uuid = sqlx::query_scalar(
        r#"SELECT item.target_id FROM coverage_denominator_items item
            WHERE item.denominator_id=$1 AND item.id=$2
              AND item.execution_authority_id=$3 FOR SHARE"#,
    )
    .bind(root_denominator_id)
    .bind(root_item_id)
    .bind(authority.id)
    .fetch_one(&mut *conn)
    .await?;
    let items = items
        .into_iter()
        .map(|mut item| {
            item.target_id = target_id;
            item
        })
        .collect::<Vec<_>>();
    let denominator = enumeration::seal_enumeration_derived_denominator_in_connection(
        conn,
        authority,
        &enumeration::SealEnumerationDerivedDenominator {
            stable_seal_request_id: namespace,
            parent_denominator_id: root_denominator_id,
            parent_denominator_item_id: root_item_id,
            derived_ordinal,
            items: items.clone(),
        },
    )
    .await?;
    let item_ids = items
        .iter()
        .map(|item| {
            (
                item.input_key.clone(),
                stable_child(denominator.id, item.input_key.as_bytes()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut descriptor_ids = BTreeMap::new();
    for (ordinal, script) in scripts.iter().enumerate() {
        let key = format!("script:{}:{}", script.source_file, script.content_sha256);
        let id = stable_child(namespace, format!("descriptor:{key}"));
        enumeration::persist_js_analysis_descriptor(
            conn,
            authority,
            &receipts::CapabilityReceiptInputRef {
                receipt_id: Uuid::nil(),
                receipt_input_id: Uuid::nil(),
                denominator_id: denominator.id,
                denominator_item_id: item_ids[&key],
                logical_input_key: key.clone(),
            },
            &enumeration::JsAnalysisDescriptorWrite {
                id,
                stable_descriptor_request_id: stable_child(namespace, format!("descriptor:{key}")),
                manifest_url: script.manifest_url.clone(),
                page_url: exact_origin.to_string(),
                document_url: script
                    .document_bases
                    .first()
                    .cloned()
                    .or_else(|| Some(exact_origin.to_string())),
                chunk_ordinal: i32::try_from(ordinal)?,
                source_map_url: None,
                script_sha256: Some(script.content_sha256.clone()),
                descriptor_metadata: serde_json::json!({
                    "source_urls": script.source_urls,
                    "discovered_from": script.discovered_from,
                    "document_bases": script.document_bases,
                    "capture_kind": "sealed_script_manifest",
                    "compatibility_version": compatibility_version,
                }),
            },
        )
        .await
        .context("ENUMERATION_SCRIPT_DESCRIPTOR")?;
        descriptor_ids.insert(script.source_file.clone(), id);
    }
    let inputs = begin_and_seal_inputs(
        conn,
        authority,
        namespace,
        &denominator,
        "enumeration.javascript",
        items
            .iter()
            .map(|item| enumeration::EnumerationTerminalReceiptInputWrite {
                denominator_item_id: item_ids[&item.input_key],
                outcome: enumeration::EnumerationTerminalInputOutcome::Found,
                evidence_authorities: evidence.to_vec(),
            })
            .collect(),
    )
    .await?
    .into_iter()
    .map(|input| (input.logical_input_key.clone(), input))
    .collect::<BTreeMap<_, _>>();
    for script in scripts {
        let key = format!("script:{}:{}", script.source_file, script.content_sha256);
        enumeration::bind_js_analysis_terminal_receipt(
            conn,
            authority,
            descriptor_ids[&script.source_file],
            &inputs[&key],
        )
        .await
        .context("ENUMERATION_BROWSER_SCRIPT_PROJECTION")?;
    }
    Ok((denominator, descriptor_ids, inputs))
}

// This is deliberately not async and is replaced before the derived set is
// written; it keeps the child item constructor free of caller-authored target
// ids while preserving a straightforward value type.
#[allow(dead_code)]
fn authority_target(
    _conn: &mut PgConnection,
    _authority: &receipts::ToolTruthExecutionAuthorityRef,
    _exact_origin: &str,
) -> Uuid {
    Uuid::nil()
}

fn occurrence_evidence(
    discovery: &[receipts::EvidenceAuthorityRef],
) -> Vec<receipts::EvidenceAuthorityRef> {
    let mut all = discovery.to_vec();
    all.extend(discovery.iter().cloned().map(|mut evidence| {
        evidence.role = "resolution".to_string();
        evidence
    }));
    all
}

fn parameter_fields(facts: &[EnumerationProducerParameterFactV2]) -> Vec<serde_json::Value> {
    facts
        .iter()
        .map(|fact| {
            serde_json::json!({
                "name": fact.name,
                "location": fact.location,
                "type": fact.value_type,
                "requirement": fact.requirement,
                "source_anchor_ids": fact.source_anchor_ids,
                "confidence": fact.confidence,
            })
        })
        .collect()
}

fn decode_parameter_fields(
    request_schema: &serde_json::Value,
) -> Result<Vec<EnumerationProducerParameterFactV2>> {
    let Some(fields) = request_schema
        .get("fields")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    let mut merged = BTreeMap::<(String, String), EnumerationProducerParameterFactV2>::new();
    for field in fields {
        let mut source_anchor_ids = field
            .get("source_anchor_ids")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        source_anchor_ids.sort();
        source_anchor_ids.dedup();
        let fact = EnumerationProducerParameterFactV2 {
            name: field
                .get("name")
                .and_then(serde_json::Value::as_str)
                .context("ENUMERATION_PARAMETER_FIELD_NAME_MISSING")?
                .to_string(),
            location: field
                .get("location")
                .and_then(serde_json::Value::as_str)
                .context("ENUMERATION_PARAMETER_FIELD_LOCATION_MISSING")?
                .to_string(),
            value_type: field
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            requirement: field
                .get("requirement")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            confidence: field
                .get("confidence")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(1.0) as f32,
            source_anchor_ids,
        };
        ensure!(
            !fact.name.trim().is_empty()
                && !fact.location.trim().is_empty()
                && !fact.source_anchor_ids.is_empty()
                && fact
                    .source_anchor_ids
                    .iter()
                    .all(|anchor| !anchor.trim().is_empty()),
            "ENUMERATION_PARAMETER_FIELD_INVALID"
        );
        let key = (fact.location.clone(), fact.name.clone());
        if let Some(existing) = merged.get_mut(&key) {
            if existing.value_type != fact.value_type {
                existing.value_type = "unknown".to_string();
            }
            if existing.requirement != fact.requirement {
                existing.requirement = "unknown".to_string();
            }
            existing.confidence = existing.confidence.min(fact.confidence);
            existing.source_anchor_ids.extend(fact.source_anchor_ids);
            existing.source_anchor_ids.sort();
            existing.source_anchor_ids.dedup();
        } else {
            merged.insert(key, fact);
        }
    }
    Ok(merged.into_values().collect())
}

fn browser_fingerprint(occurrence: &EnumerationBrowserOccurrenceV2) -> Result<String> {
    sha256_prefixed(&serde_json::json!({
        "capture_event_id": occurrence.capture_event_id,
        "canonical_request_url": occurrence.canonical_request_url,
        "logical_key": occurrence.logical_key,
        "method": occurrence.method,
    }))
}

fn persisted_browser_capture_event_id(candidate_id: Uuid, producer_capture_event_id: Uuid) -> Uuid {
    // Producer capture ids describe the browser-local event and can repeat
    // when the same deterministic fixture is observed in a later operation.
    // The provenance tables require a globally unique immutable event id, so
    // bind that producer identity to the operation-scoped candidate while
    // retaining the raw id in the source anchor and event fingerprint.
    stable_child(
        candidate_id,
        format!("browser-capture:{producer_capture_event_id}").as_bytes(),
    )
}

fn static_source_anchor(occurrence: &EnumerationProducerOccurrenceV2) -> String {
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

fn static_fingerprint(occurrence: &EnumerationProducerOccurrenceV2) -> Result<String> {
    sha256_prefixed(&serde_json::json!({
        "candidate_id": occurrence.candidate_id,
        "method": occurrence.method,
        "source_file": occurrence.source_file,
        "source_span": occurrence.source_span,
    }))
}

fn static_route_kind(occurrence: &EnumerationProducerOccurrenceV2) -> Result<&'static str> {
    if matches!(
        occurrence.resolution_status.as_str(),
        "ambiguous" | "unresolved"
    ) {
        return Ok("dynamic_unresolved");
    }
    if occurrence.resolution_status == "resolved" && occurrence.canonical_url.is_some() {
        return Ok(if occurrence.route_kind == "resolved_route_template" {
            "template"
        } else {
            "exact"
        });
    }
    match occurrence.route_kind.as_str() {
        "resolved_exact" => Ok("exact"),
        "resolved_route_template" => Ok("template"),
        "arbitrary_dynamic" => Ok("dynamic_unresolved"),
        _ => anyhow::bail!("ENUMERATION_JS_API_ROUTE_KIND_INVALID"),
    }
}

fn static_protocol(
    occurrence: &EnumerationProducerOccurrenceV2,
    exact_origin: &str,
) -> Result<&'static str> {
    match occurrence.protocol.as_str() {
        "http" => {
            let url =
                reqwest::Url::parse(occurrence.canonical_url.as_deref().unwrap_or(exact_origin))?;
            match url.scheme() {
                "http" => Ok("http"),
                "https" => Ok("https"),
                _ => anyhow::bail!("ENUMERATION_JS_API_PROTOCOL_INVALID"),
            }
        }
        "websocket" => Ok("websocket"),
        "graphql" => Ok("graphql"),
        _ => anyhow::bail!("ENUMERATION_JS_API_PROTOCOL_INVALID"),
    }
}

fn lane_command(
    stable_commit_request_id: Uuid,
    lane: &str,
    target_id: Uuid,
    exact_origin: String,
    artifact_sha256: String,
    dependency_receipt_ids: Vec<Uuid>,
    evidence_audit_ids: Vec<i64>,
    script_denominator_id: Option<Uuid>,
    candidate_denominator_ids: Vec<Uuid>,
    parameter_denominator_ids: Vec<Uuid>,
    resolution_occurrence_id: Option<Uuid>,
    resolution_terminal_receipt_id: Option<Uuid>,
    resolution_terminal_receipt_input_id: Option<Uuid>,
) -> enumeration::SealEnumerationLaneCommitReceipt {
    enumeration::SealEnumerationLaneCommitReceipt {
        stable_commit_request_id,
        lane: lane.to_string(),
        target_id,
        exact_origin,
        artifact_sha256,
        dependency_receipt_ids,
        evidence_audit_ids,
        script_denominator_id,
        candidate_denominator_ids,
        parameter_denominator_ids,
        resolution_occurrence_id,
        resolution_terminal_receipt_id,
        resolution_terminal_receipt_input_id,
    }
}

#[allow(clippy::too_many_arguments)]
fn resolution_lane_command(
    stable_commit_request_id: Uuid,
    target_id: Uuid,
    exact_origin: String,
    artifact_sha256: String,
    dependency_receipt_ids: Vec<Uuid>,
    lane_evidence_audit_ids: Vec<i64>,
    resolution_occurrence_id: Uuid,
    resolution_terminal_receipt_id: Uuid,
    resolution_terminal_receipt_input_id: Uuid,
) -> enumeration::SealEnumerationLaneCommitReceipt {
    lane_command(
        stable_commit_request_id,
        "resolution",
        target_id,
        exact_origin,
        artifact_sha256,
        dependency_receipt_ids,
        lane_evidence_audit_ids,
        None,
        vec![],
        vec![],
        Some(resolution_occurrence_id),
        Some(resolution_terminal_receipt_id),
        Some(resolution_terminal_receipt_input_id),
    )
}

fn coverage_lane_command(
    stable_commit_request_id: Uuid,
    target_id: Uuid,
    exact_origin: String,
    artifact_sha256: String,
    dependency_receipt_ids: Vec<Uuid>,
    lane_evidence_audit_ids: Vec<i64>,
) -> enumeration::SealEnumerationLaneCommitReceipt {
    lane_command(
        stable_commit_request_id,
        "coverage",
        target_id,
        exact_origin,
        artifact_sha256,
        dependency_receipt_ids,
        lane_evidence_audit_ids,
        None,
        vec![],
        vec![],
        None,
        None,
        None,
    )
}

async fn book_enumeration_lane_derived_evidence(
    conn: &mut PgConnection,
    authority: &receipts::ToolTruthExecutionAuthorityRef,
    stable_request_id: Uuid,
    lane: &str,
    target_id: Uuid,
    exact_origin: &str,
    artifact_sha256: &str,
    source_evidence_audit_ids: &[i64],
    worker_fence: &enumeration::EnumerationWorkerFence,
) -> Result<i64> {
    ensure!(
        matches!(lane, "parameter" | "resolution" | "coverage")
            && !stable_request_id.is_nil()
            && !target_id.is_nil()
            && artifact_sha256.starts_with("sha256:")
            && !source_evidence_audit_ids.is_empty()
            && source_evidence_audit_ids.iter().all(|id| *id > 0),
        "ENUMERATION_DERIVED_EVIDENCE_INPUT_INVALID"
    );
    let scope_version: i64 =
        sqlx::query_scalar("SELECT scope_rules_version FROM organizations WHERE id=$1 FOR SHARE")
            .bind(authority.organization_id)
            .fetch_one(&mut *conn)
            .await?;
    let producer = serde_json::json!({
        "artifact_sha256": artifact_sha256,
        "enumeration_lane": lane,
        "exact_origin": exact_origin,
        "lease_token": worker_fence.lease_token,
        "organization_id": authority.organization_id,
        "source_evidence_audit_ids": source_evidence_audit_ids,
        "source_tool_call_id": worker_fence.source_tool_call_id,
        "stable_request_id": stable_request_id,
        "stage_execution_id": authority.stage_execution_id,
        "target_id": target_id,
        "worker_attempt_epoch": worker_fence.worker_attempt_epoch,
        "worker_run_id": worker_fence.worker_run_id,
    });
    let audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,target_id,
               audit_role,evidence_technique,evidence_outcome)
           VALUES('enumeration_lane_receipt_observed','tool_truth',
                  'Canonical Enumeration lane exact-set reduction',$1,
                  'tool_truth_receipt','completed',$2,$3,$4,'evidence',$5,$6)
           RETURNING id"#,
    )
    .bind(&authority.project_path_at_freeze)
    .bind(serde_json::json!({"tool_truth_producer": producer}))
    .bind(authority.operation_id)
    .bind(target_id)
    .bind(match lane {
        "parameter" => "GOLISH-ENUM-PARAM",
        "resolution" => "GOLISH-ENUM-JSAPI-RESOLUTION",
        "coverage" => "GOLISH-ENUM-COVERAGE",
        _ => unreachable!("validated lane"),
    })
    .bind(if lane == "resolution" {
        "indeterminate"
    } else {
        "found"
    })
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        r#"INSERT INTO evidence_classifications(
               evidence_audit_id,classification,scope_version,reason,
               classified_by_session,producing_stage_run_id)
           VALUES($1,'in_scope',$2,'derived Enumeration exact-set evidence',
                  'tool_truth_receipt',$3)"#,
    )
    .bind(audit_id)
    .bind(scope_version)
    .bind(authority.stage_execution_id)
    .execute(&mut *conn)
    .await?;
    Ok(audit_id)
}

impl GolishDbRepoProvider {
    async fn enumeration_source_root(
        &self,
        stage_execution_id: Uuid,
        stage_run_unit_id: Uuid,
    ) -> Result<Uuid> {
        Ok(self
            .tool_truth_seal_denominator_impl(
                golish_agent_kit::db_traits::SealToolTruthDenominatorRequest {
                    stable_seal_request_id: stable_denominator_seal_request(
                        stage_execution_id,
                        stage_run_unit_id,
                    ),
                    stage_execution_id,
                    source:
                        golish_agent_kit::db_traits::ToolTruthDenominatorSourceRef::StageTeamUnit {
                            stage_run_unit_id,
                        },
                },
            )
            .await?
            .id)
    }

    pub(super) async fn enumeration_commit_browser_producer_v2_impl(
        &self,
        request: CommitEnumerationBrowserProducerV2,
    ) -> Result<EnumerationLaneClosureReceiptV2> {
        request
            .artifact
            .validate_census_and_hash()
            .context("ENUMERATION_BROWSER_ARTIFACT")?;
        validate_identity(
            request.stable_request_id,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            request.worker_run_id,
            request.worker_attempt_epoch,
            request.lease_token,
            request.source_tool_call_id,
        )
        .context("ENUMERATION_BROWSER_IDENTITY")?;
        validate_lineage(
            &request.artifact.lineage,
            request.operation_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.worker_run_id,
            request.worker_attempt_epoch,
            request.source_tool_call_id,
        )
        .context("ENUMERATION_BROWSER_LINEAGE")?;
        ensure!(
            request.stable_request_id
                == stable_child(
                    request.source_tool_call_id,
                    request.artifact.artifact_sha256.as_bytes()
                ),
            "ENUMERATION_BROWSER_STABLE_REQUEST_DRIFT"
        );
        let exact_origin = canonical_origin(&request.exact_origin)?;
        let evidence_ids = canonical_ids(&request.artifact.browser_evidence_audit_ids);
        ensure!(
            !evidence_ids.is_empty()
                && evidence_ids.iter().all(|id| *id > 0)
                && evidence_ids.len() == request.artifact.browser_evidence_audit_ids.len(),
            "ENUMERATION_BROWSER_EVIDENCE_MANIFEST_INVALID"
        );
        let mut replay_tx = self.pool.begin().await?;
        if let Some(receipt) = response_loss_replay(
            &mut replay_tx,
            request.stable_request_id,
            "browser",
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
            &request.artifact.artifact_sha256,
            &[],
            &evidence_ids,
        )
        .await
        .context("ENUMERATION_BROWSER_RESPONSE_LOSS_REPLAY")?
        {
            replay_tx.commit().await?;
            return Ok(receipt);
        }
        replay_tx.commit().await?;

        let source_root_denominator_id = self
            .enumeration_source_root(request.stage_execution_id, request.stage_run_unit_id)
            .await
            .context("ENUMERATION_BROWSER_SOURCE_ROOT")?;
        let mut tx = self.pool.begin().await?;
        enumeration::lock_enumeration_subject_identity(
            &mut tx,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await
        .context("ENUMERATION_BROWSER_SUBJECT_IDENTITY")?;
        let root = enumeration::seal_enumeration_worker_authority_root_in_connection(
            &mut tx,
            &enumeration::SealEnumerationWorkerAuthorityRoot {
                stable_authority_request_id: stable_child(
                    request.stable_request_id,
                    b"browser-authority",
                ),
                stable_root_request_id: stable_child(request.stable_request_id, b"browser-root"),
                source_root_denominator_id,
                worker_fence: enumeration::EnumerationWorkerFence {
                    worker_run_id: request.worker_run_id,
                    worker_attempt_epoch: request.worker_attempt_epoch,
                    lease_token: request.lease_token,
                    source_tool_call_id: request.source_tool_call_id,
                },
            },
        )
        .await
        .context("ENUMERATION_BROWSER_WORKER_AUTHORITY_ROOT")?;
        ensure!(
            root.authority.operation_id == request.operation_id
                && root.authority.organization_id == request.organization_id
                && root.authority.stage_execution_id == request.stage_execution_id,
            "ENUMERATION_BROWSER_ROOT_AUTHORITY_MISMATCH"
        );
        enumeration::lock_enumeration_lane_subject(
            &mut tx,
            &root.authority,
            request.target_id,
            &exact_origin,
        )
        .await
        .context("ENUMERATION_BROWSER_LANE_SUBJECT")?;
        let (root_item_id, source_web_origin_id) = exact_root_subject(
            &mut tx,
            &root.authority,
            root.root_denominator.id,
            request.target_id,
            &exact_origin,
            "GOLISH-ENUM-JS",
        )
        .await
        .context("ENUMERATION_BROWSER_ROOT_SUBJECT")?;
        let evidence = enumeration::bind_enumeration_evidence_authorities(
            &mut tx,
            &root.authority,
            &evidence_ids,
            "discovery",
        )
        .await
        .context("ENUMERATION_BROWSER_EVIDENCE_AUTHORITY")?;
        let script_namespace = stable_child(request.stable_request_id, b"browser-scripts");
        let (script_denominator, _, _) = persist_scripts(
            &mut tx,
            &root.authority,
            script_namespace,
            &exact_origin,
            &request.artifact.scripts,
            &evidence,
            root.root_denominator.id,
            root_item_id,
            1,
            "enumeration_browser_producer_artifact.v2",
        )
        .await?;

        let candidate_namespace =
            stable_child(request.stable_request_id, b"browser-runtime-candidates");
        let candidate_items = request
            .artifact
            .occurrences
            .iter()
            .map(
                |occurrence| enumeration::EnumerationDerivedDenominatorItemWrite {
                    input_key: format!("browser:{}", occurrence.logical_key),
                    target_id: request.target_id,
                    exact_asset: occurrence.canonical_request_url.clone(),
                    technique: "capture_runtime_request".to_string(),
                    expected_capability: "enumeration.candidate".to_string(),
                },
            )
            .collect::<Vec<_>>();
        ensure!(
            candidate_items
                .iter()
                .map(|item| &item.input_key)
                .collect::<BTreeSet<_>>()
                .len()
                == candidate_items.len(),
            "ENUMERATION_BROWSER_OCCURRENCE_KEY_DUPLICATE"
        );
        let candidate_denominator =
            enumeration::seal_enumeration_derived_denominator_in_connection(
                &mut tx,
                &root.authority,
                &enumeration::SealEnumerationDerivedDenominator {
                    stable_seal_request_id: candidate_namespace,
                    parent_denominator_id: root.root_denominator.id,
                    parent_denominator_item_id: root_item_id,
                    derived_ordinal: 2,
                    items: candidate_items.clone(),
                },
            )
            .await
            .context("ENUMERATION_BROWSER_CANDIDATE_DENOMINATOR")?;
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
            &mut tx,
            &root.authority,
            candidate_namespace,
            &candidate_denominator,
            "enumeration.candidate",
            candidate_items
                .iter()
                .map(|item| enumeration::EnumerationTerminalReceiptInputWrite {
                    denominator_item_id: candidate_item_ids[&item.input_key],
                    outcome: enumeration::EnumerationTerminalInputOutcome::Found,
                    evidence_authorities: evidence.clone(),
                })
                .collect(),
        )
        .await
        .context("ENUMERATION_BROWSER_CANDIDATE_INPUTS")?
        .into_iter()
        .map(|input| (input.logical_input_key.clone(), input))
        .collect::<BTreeMap<_, _>>();
        let occurrence_evidence = occurrence_evidence(&evidence);
        for occurrence in &request.artifact.occurrences {
            let key = format!("browser:{}", occurrence.logical_key);
            let input = &candidate_inputs[&key];
            let candidate_id = stable_child(
                root.authority.id,
                format!("browser-candidate:{}", occurrence.logical_key),
            );
            let capture_event_id =
                persisted_browser_capture_event_id(candidate_id, occurrence.capture_event_id);
            let fingerprint = browser_fingerprint(occurrence)?;
            enumeration::persist_candidate_descriptor(
                &mut tx,
                &root.authority,
                input,
                &enumeration::CandidateDescriptorWrite {
                    id: candidate_id,
                    stable_candidate_request_id: stable_child(
                        request.stable_request_id,
                        format!("browser-candidate:{}", occurrence.logical_key),
                    ),
                    js_analysis_item_id: None,
                    source_anchor: format!(
                        "runtime:{}:{}",
                        occurrence.capture_event_id, occurrence.logical_key
                    ),
                    callsite_fingerprint: fingerprint.clone(),
                    capture_event_id,
                    capture_attempt_ordinal: 1,
                    captured_at: request.artifact.captured_at,
                    event_fingerprint: fingerprint,
                    duplicate_ordinal: occurrence.duplicate_ordinal,
                    resolution_input: occurrence.canonical_request_url.clone(),
                },
            )
            .await
            .context("ENUMERATION_BROWSER_CANDIDATE_DESCRIPTOR")?;
            let (resolved_target_id, resolved_web_origin_id) =
                if occurrence.scope_decision == "in_scope" {
                    let subject = exact_frozen_origin_subject(
                        &mut tx,
                        &root.authority,
                        request.target_id,
                        &occurrence.canonical_request_url,
                    )
                    .await
                    .context("ENUMERATION_BROWSER_FROZEN_ORIGIN_SUBJECT")?;
                    (Some(subject.0), Some(subject.1))
                } else {
                    ensure!(
                        occurrence.scope_decision == "scope_excluded",
                        "ENUMERATION_BROWSER_SCOPE_DECISION_INVALID"
                    );
                    (None, None)
                };
            let observed_origin =
                golish_pentest_domain::canonical_web_origin(&occurrence.canonical_request_url)
                    .map(|origin| origin.key)
                    .context("ENUMERATION_BROWSER_OCCURRENCE_ORIGIN_INVALID")?;
            let expected_scope_decision = if observed_origin == exact_origin {
                "in_scope"
            } else {
                "scope_excluded"
            };
            ensure!(
                occurrence.scope_decision == expected_scope_decision,
                "ENUMERATION_BROWSER_SCOPE_DECISION_DRIFT"
            );
            let initiator_tuple_complete = occurrence.initiator_script_url.is_some()
                && occurrence.initiator_line.is_some()
                && occurrence.initiator_column.is_some()
                && occurrence.cdp_request_id_hash.is_some();
            let initiator_tuple_empty = occurrence.initiator_script_url.is_none()
                && occurrence.initiator_line.is_none()
                && occurrence.initiator_column.is_none()
                && occurrence.cdp_request_id_hash.is_none();
            ensure!(
                match occurrence.initiator_status.as_str() {
                    "matched" => initiator_tuple_complete,
                    "unmatched" | "unsupported_cdp" | "not_applicable" => {
                        initiator_tuple_empty
                    }
                    _ => false,
                },
                "ENUMERATION_BROWSER_INITIATOR_STATUS_DRIFT"
            );
            let initiator_matched = occurrence.initiator_status == "matched";
            let method = occurrence.method.to_ascii_uppercase();
            let url = reqwest::Url::parse(&occurrence.canonical_request_url)?;
            let protocol = match url.scheme() {
                "http" => "http",
                "https" => "https",
                _ => anyhow::bail!("ENUMERATION_BROWSER_PROTOCOL_INVALID"),
            };
            let observation_shape_valid = match occurrence.observation_kind.as_str() {
                "html_form" => {
                    !occurrence.request_sent
                        && occurrence.read_only_block_reason.as_deref()
                            == Some("html_form_observed_without_submission")
                        && occurrence.initiator_status == "not_applicable"
                }
                "runtime_request" if occurrence.request_sent => {
                    occurrence.read_only_block_reason.is_none()
                }
                "runtime_request" => {
                    occurrence
                        .read_only_block_reason
                        .as_deref()
                        .is_some_and(|reason| {
                            matches!(
                                reason,
                                "method_not_read_only"
                                    | "cross_origin_scope"
                                    | "dangerous_route"
                                    | "capture_send_status_unknown"
                            )
                        })
                }
                _ => false,
            };
            ensure!(
                observation_shape_valid
                    && (occurrence.read_only_block_reason.as_deref() != Some("cross_origin_scope")
                        || expected_scope_decision == "scope_excluded")
                    && (expected_scope_decision != "scope_excluded"
                        || occurrence.observation_kind == "html_form"
                        || occurrence.read_only_block_reason.as_deref()
                            == Some("cross_origin_scope")),
                "ENUMERATION_BROWSER_OBSERVATION_KIND_INVALID"
            );
            let in_scope = occurrence.scope_decision == "in_scope";
            let persisted_canonical_url =
                in_scope.then(|| occurrence.canonical_request_url.clone());
            let read_only_reason = occurrence.read_only_block_reason.as_deref().unwrap_or(
                if occurrence.observation_kind == "html_form" {
                    "html_form_observed_without_submission"
                } else {
                    "runtime_request_observed"
                },
            );
            let mut fields = parameter_fields(&occurrence.parameter_facts);
            let mut observed_fields = occurrence
                .parameter_facts
                .iter()
                .map(|field| {
                    (
                        field.location.to_ascii_lowercase(),
                        field.name.to_ascii_lowercase(),
                    )
                })
                .collect::<BTreeSet<_>>();
            let mut header_names = occurrence
                .request_header_names
                .iter()
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>();
            header_names.sort_unstable();
            header_names.dedup();
            for name in header_names {
                if observed_fields.insert(("header".to_string(), name.clone())) {
                    fields.push(serde_json::json!({
                        "name": name,
                        "location": "header",
                        "type": "string",
                        "requirement": "unknown",
                        "source_anchor_ids": [format!("runtime:{}", occurrence.capture_event_id)],
                        "confidence": 1.0,
                    }));
                }
            }
            enumeration::persist_endpoint_occurrence(
                &mut tx,
                &root.authority,
                input,
                &enumeration::EndpointOccurrenceWrite {
                    id: stable_child(candidate_id, b"browser-occurrence"),
                    stable_occurrence_request_id: stable_child(
                        request.stable_request_id,
                        format!("browser-occurrence:{}", occurrence.logical_key),
                    ),
                    candidate_input_id: candidate_id,
                    capture_event_id,
                    source_target_id: request.target_id,
                    source_web_origin_id,
                    resolved_target_id,
                    resolved_web_origin_id,
                    parent_occurrence_id: None,
                    source_url: occurrence.page_url.clone(),
                    document_url: occurrence.document_base.clone(),
                    script_url: occurrence.initiator_script_url.clone(),
                    script_sha256: None,
                    source_span: serde_json::json!({
                        "status": occurrence.observation_kind,
                        "reason_code": read_only_reason,
                    }),
                    initiator_url: initiator_matched
                        .then(|| occurrence.initiator_script_url.clone())
                        .flatten(),
                    initiator_status: if initiator_matched {
                        "matched".to_string()
                    } else {
                        occurrence.initiator_status.clone()
                    },
                    initiator_line: initiator_matched
                        .then_some(occurrence.initiator_line)
                        .flatten(),
                    initiator_column: initiator_matched
                        .then_some(occurrence.initiator_column)
                        .flatten(),
                    cdp_request_id_hash: initiator_matched
                        .then(|| occurrence.cdp_request_id_hash.clone())
                        .flatten(),
                    protocol: protocol.to_string(),
                    method,
                    graphql_operation_name: None,
                    websocket_subprotocol: None,
                    raw_expression: Some(occurrence.request_url.clone()),
                    receiver_kind: Some(if occurrence.observation_kind == "html_form" {
                        "browser_form".to_string()
                    } else {
                        "browser_runtime".to_string()
                    }),
                    observation_kind: occurrence.observation_kind.clone(),
                    inference_level: "observed".to_string(),
                    resolution_status: "resolved".to_string(),
                    scope_decision: occurrence.scope_decision.clone(),
                    candidate_classification: "endpoint".to_string(),
                    canonical_request_url: persisted_canonical_url.clone(),
                    display_url: persisted_canonical_url.clone(),
                    resolution_reason: if in_scope {
                        format!("browser_{}_exact_origin", occurrence.observation_kind)
                    } else {
                        format!("browser_{}_scope_excluded", occurrence.observation_kind)
                    },
                    resolution_base_facts: serde_json::json!({
                        "document_base": occurrence.document_base,
                        "selected_url": persisted_canonical_url,
                    }),
                    resolution_candidates: serde_json::json!([]),
                    resolution_chain: serde_json::json!([{
                        "step": if occurrence.observation_kind == "html_form" {
                            "browser_form"
                        } else {
                            "browser_runtime"
                        },
                        "selected": true,
                        "selected_url": occurrence.canonical_request_url,
                    }]),
                    route_kind: if in_scope {
                        "exact".to_string()
                    } else {
                        "dynamic_unresolved".to_string()
                    },
                    route_template: None,
                    request_sent: occurrence.request_sent,
                    request_schema: serde_json::json!({
                        "schema_version": 2,
                        "fields": fields,
                    }),
                    redaction_metadata: serde_json::json!({
                        "redacted": true,
                        "header_count": occurrence.request_header_names.len(),
                        "field_count": occurrence.parameter_facts.len(),
                        "policy_version": "value_free.v2",
                    }),
                    request_body_length: None,
                    runtime_sample_url: (in_scope
                        && occurrence.observation_kind == "runtime_request")
                        .then(|| occurrence.canonical_request_url.clone()),
                    observed_at: request.artifact.captured_at,
                },
                &occurrence_evidence,
            )
            .await
            .context("ENUMERATION_BROWSER_ENDPOINT_OCCURRENCE")?;
            enumeration::seal_enumeration_candidate_closure_in_connection(
                &mut tx,
                &root.authority,
                &enumeration::SealEnumerationCandidateClosure {
                    stable_closure_request_id: stable_child(
                        request.stable_request_id,
                        format!("browser-candidate-closure:{}", occurrence.logical_key),
                    ),
                    candidate_input_id: candidate_id,
                    resolution_terminal_input: None,
                },
            )
            .await
            .context("ENUMERATION_BROWSER_CANDIDATE_CLOSURE")?;
        }
        enumeration::seal_enumeration_candidate_denominator_closure_in_connection(
            &mut tx,
            &root.authority,
            &enumeration::SealEnumerationCandidateDenominatorClosure {
                stable_closure_request_id: stable_child(
                    request.stable_request_id,
                    b"browser-candidate-denominator-closure",
                ),
                denominator_id: candidate_denominator.id,
            },
        )
        .await
        .context("ENUMERATION_BROWSER_CANDIDATE_DENOMINATOR_CLOSURE")?;
        let (row, replayed) = enumeration::seal_enumeration_lane_commit_receipt(
            &mut tx,
            &root.authority,
            &lane_command(
                request.stable_request_id,
                "browser",
                request.target_id,
                exact_origin,
                request.artifact.artifact_sha256,
                vec![],
                evidence_ids,
                Some(script_denominator.id),
                vec![candidate_denominator.id],
                vec![],
                None,
                None,
                None,
            ),
        )
        .await
        .context("ENUMERATION_BROWSER_LANE_RECEIPT")?;
        tx.commit()
            .await
            .context("ENUMERATION_BROWSER_TRANSACTION_COMMIT")?;
        lane_receipt_view(row, replayed)
    }
}

#[cfg(test)]
mod tests {
    use super::persisted_browser_capture_event_id;
    use uuid::Uuid;

    #[test]
    fn browser_capture_event_identity_is_stable_within_candidate_and_distinct_across_operations() {
        let producer_capture_event_id =
            Uuid::parse_str("1cd8f95a-e84f-5826-86a4-2ebdcd20e68c").unwrap();
        let first_candidate = Uuid::parse_str("92edd931-b1bb-57a6-ada4-d4ccf38dd4c4").unwrap();
        let later_operation_candidate =
            Uuid::parse_str("f3dc577d-42cb-5fb4-af38-5e27b4a4fdc1").unwrap();

        let first = persisted_browser_capture_event_id(first_candidate, producer_capture_event_id);
        assert_eq!(
            first,
            persisted_browser_capture_event_id(first_candidate, producer_capture_event_id)
        );
        assert_ne!(
            first,
            persisted_browser_capture_event_id(
                later_operation_candidate,
                producer_capture_event_id
            )
        );
    }
}

impl GolishDbRepoProvider {
    pub(super) async fn enumeration_commit_js_api_producer_v2_impl(
        &self,
        request: CommitEnumerationJsApiProducerV2,
    ) -> Result<EnumerationLaneClosureReceiptV2> {
        request.artifact.validate_census_and_hash()?;
        validate_identity(
            request.stable_request_id,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            request.worker_run_id,
            request.worker_attempt_epoch,
            request.lease_token,
            request.source_tool_call_id,
        )?;
        validate_lineage(
            &request.artifact.lineage,
            request.operation_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.worker_run_id,
            request.worker_attempt_epoch,
            request.source_tool_call_id,
        )?;
        ensure!(
            request.stable_request_id
                == stable_child(
                    request.source_tool_call_id,
                    request.artifact.artifact_sha256.as_bytes()
                ),
            "ENUMERATION_JS_API_STABLE_REQUEST_DRIFT"
        );
        let exact_origin = canonical_origin(&request.exact_origin)?;
        let evidence_ids = canonical_ids(&request.artifact.jsapi_evidence_audit_ids);
        ensure!(
            !evidence_ids.is_empty()
                && evidence_ids.iter().all(|id| *id > 0)
                && evidence_ids.len() == request.artifact.jsapi_evidence_audit_ids.len(),
            "ENUMERATION_JS_API_EVIDENCE_MANIFEST_INVALID"
        );
        let dependency_ids = vec![request.browser_receipt.receipt_id];
        let mut replay_tx = self.pool.begin().await?;
        if let Some(receipt) = response_loss_replay(
            &mut replay_tx,
            request.stable_request_id,
            "js_api",
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
            &request.artifact.artifact_sha256,
            &dependency_ids,
            &evidence_ids,
        )
        .await?
        {
            let _browser_row = load_named_receipt(
                &mut replay_tx,
                &request.browser_receipt,
                EnumerationLaneKindV2::Browser,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            replay_tx.commit().await?;
            return Ok(receipt);
        }
        replay_tx.commit().await?;

        let source_root_denominator_id = self
            .enumeration_source_root(request.stage_execution_id, request.stage_run_unit_id)
            .await?;
        let mut tx = self.pool.begin().await?;
        enumeration::lock_enumeration_subject_identity(
            &mut tx,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await?;
        let browser_row = load_named_receipt(
            &mut tx,
            &request.browser_receipt,
            EnumerationLaneKindV2::Browser,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await?;
        let root = enumeration::seal_enumeration_worker_authority_root_in_connection(
            &mut tx,
            &enumeration::SealEnumerationWorkerAuthorityRoot {
                stable_authority_request_id: stable_child(
                    request.stable_request_id,
                    b"js-api-authority",
                ),
                stable_root_request_id: stable_child(request.stable_request_id, b"js-api-root"),
                source_root_denominator_id,
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
            "ENUMERATION_JS_API_ROOT_AUTHORITY_MISMATCH"
        );
        enumeration::lock_enumeration_lane_subject(
            &mut tx,
            &root.authority,
            request.target_id,
            &exact_origin,
        )
        .await?;
        let (root_item_id, source_web_origin_id) = exact_root_subject(
            &mut tx,
            &root.authority,
            root.root_denominator.id,
            request.target_id,
            &exact_origin,
            "GOLISH-ENUM-JSAPI",
        )
        .await?;

        let mut browser_scripts = sqlx::query_as::<_, (String, Option<String>)>(
            r#"SELECT descriptor.manifest_url,descriptor.script_sha256
                     FROM enumeration_js_analysis_items descriptor
                    WHERE descriptor.execution_authority_id=$1
                      AND descriptor.denominator_id=$2
                      AND descriptor.terminal_receipt_input_id IS NOT NULL
                    ORDER BY descriptor.manifest_url,descriptor.script_sha256"#,
        )
        .bind(browser_row.execution_authority_id)
        .bind(
            browser_row
                .script_denominator_id
                .context("ENUMERATION_JS_API_BROWSER_SCRIPT_DENOMINATOR_MISSING")?,
        )
        .fetch_all(&mut *tx)
        .await?;
        browser_scripts.sort();
        let mut artifact_scripts = request
            .artifact
            .scripts
            .iter()
            .map(|script| {
                (
                    script.manifest_url.clone(),
                    Some(script.content_sha256.clone()),
                )
            })
            .collect::<Vec<_>>();
        artifact_scripts.sort();
        ensure!(
            browser_scripts == artifact_scripts,
            "ENUMERATION_JS_API_BROWSER_SCRIPT_SET_DRIFT"
        );

        let evidence = enumeration::bind_enumeration_evidence_authorities(
            &mut tx,
            &root.authority,
            &evidence_ids,
            "discovery",
        )
        .await?;
        let script_namespace = stable_child(request.stable_request_id, b"js-api-scripts");
        let (script_denominator, descriptor_ids, script_inputs) = persist_scripts(
            &mut tx,
            &root.authority,
            script_namespace,
            &exact_origin,
            &request.artifact.scripts,
            &evidence,
            root.root_denominator.id,
            root_item_id,
            1,
            "enumeration_js_api_producer_artifact.v2",
        )
        .await?;
        let scripts_by_name = request
            .artifact
            .scripts
            .iter()
            .map(|script| (script.source_file.as_str(), script))
            .collect::<BTreeMap<_, _>>();
        ensure!(
            scripts_by_name.len() == request.artifact.scripts.len(),
            "ENUMERATION_JS_API_SCRIPT_NAME_DUPLICATE"
        );
        let mut occurrences_by_script =
            BTreeMap::<&str, Vec<&EnumerationProducerOccurrenceV2>>::new();
        for occurrence in &request.artifact.occurrences {
            ensure!(
                scripts_by_name.contains_key(occurrence.source_file.as_str()),
                "ENUMERATION_JS_API_OCCURRENCE_SCRIPT_MISSING"
            );
            occurrences_by_script
                .entry(occurrence.source_file.as_str())
                .or_default()
                .push(occurrence);
        }
        let mut candidate_denominator_ids = Vec::new();
        let occurrence_evidence = occurrence_evidence(&evidence);
        if request.artifact.scripts.is_empty() {
            let empty_namespace =
                stable_child(request.stable_request_id, b"js-api-empty-candidates");
            let empty = enumeration::seal_enumeration_derived_denominator_in_connection(
                &mut tx,
                &root.authority,
                &enumeration::SealEnumerationDerivedDenominator {
                    stable_seal_request_id: empty_namespace,
                    parent_denominator_id: root.root_denominator.id,
                    parent_denominator_item_id: root_item_id,
                    derived_ordinal: 2,
                    items: vec![],
                },
            )
            .await?;
            begin_and_seal_inputs(
                &mut tx,
                &root.authority,
                empty_namespace,
                &empty,
                "enumeration.candidate",
                vec![],
            )
            .await?;
            enumeration::seal_enumeration_candidate_denominator_closure_in_connection(
                &mut tx,
                &root.authority,
                &enumeration::SealEnumerationCandidateDenominatorClosure {
                    stable_closure_request_id: stable_child(
                        empty_namespace,
                        b"denominator-closure",
                    ),
                    denominator_id: empty.id,
                },
            )
            .await?;
            candidate_denominator_ids.push(empty.id);
        }

        for (script_ordinal, script) in request.artifact.scripts.iter().enumerate() {
            let occurrences = occurrences_by_script
                .get(script.source_file.as_str())
                .cloned()
                .unwrap_or_default();
            let script_key = format!("script:{}:{}", script.source_file, script.content_sha256);
            let script_input = &script_inputs[&script_key];
            let namespace = stable_child(
                request.stable_request_id,
                format!("js-api-candidates:{}", script.source_file),
            );
            let items = occurrences
                .iter()
                .map(
                    |occurrence| enumeration::EnumerationDerivedDenominatorItemWrite {
                        input_key: format!("candidate:{}", occurrence.candidate_id),
                        target_id: request.target_id,
                        exact_asset: if occurrence.scope_decision == "in_scope" {
                            occurrence
                                .canonical_url
                                .clone()
                                .unwrap_or_else(|| exact_origin.clone())
                        } else {
                            exact_origin.clone()
                        },
                        technique: "extract_endpoint_candidate".to_string(),
                        expected_capability: "enumeration.candidate".to_string(),
                    },
                )
                .collect::<Vec<_>>();
            ensure!(
                items
                    .iter()
                    .map(|item| &item.input_key)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == items.len(),
                "ENUMERATION_JS_API_CANDIDATE_ID_DUPLICATE"
            );
            let denominator = enumeration::seal_enumeration_derived_denominator_in_connection(
                &mut tx,
                &root.authority,
                &enumeration::SealEnumerationDerivedDenominator {
                    stable_seal_request_id: namespace,
                    parent_denominator_id: script_denominator.id,
                    parent_denominator_item_id: script_input.denominator_item_id,
                    derived_ordinal: i32::try_from(script_ordinal + 2)?,
                    items: items.clone(),
                },
            )
            .await?;
            candidate_denominator_ids.push(denominator.id);
            let item_ids = items
                .iter()
                .map(|item| {
                    (
                        item.input_key.clone(),
                        stable_child(denominator.id, item.input_key.as_bytes()),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let inputs = begin_and_seal_inputs(
                &mut tx,
                &root.authority,
                namespace,
                &denominator,
                "enumeration.candidate",
                items
                    .iter()
                    .map(|item| enumeration::EnumerationTerminalReceiptInputWrite {
                        denominator_item_id: item_ids[&item.input_key],
                        outcome: enumeration::EnumerationTerminalInputOutcome::Found,
                        evidence_authorities: evidence.clone(),
                    })
                    .collect(),
            )
            .await?
            .into_iter()
            .map(|input| (input.logical_input_key.clone(), input))
            .collect::<BTreeMap<_, _>>();
            let mut denominator_has_unresolved = false;
            for occurrence in occurrences {
                ensure!(
                    matches!(
                        occurrence.resolution_status.as_str(),
                        "resolved" | "ambiguous" | "unresolved" | "not_applicable"
                    ) && matches!(
                        occurrence.scope_decision.as_str(),
                        "in_scope" | "scope_excluded"
                    ),
                    "ENUMERATION_JS_API_OCCURRENCE_STATE_INVALID"
                );
                let unresolved = occurrence.scope_decision == "in_scope"
                    && matches!(
                        occurrence.resolution_status.as_str(),
                        "ambiguous" | "unresolved"
                    );
                denominator_has_unresolved |= unresolved;
                let key = format!("candidate:{}", occurrence.candidate_id);
                let input = &inputs[&key];
                let candidate_id = stable_child(
                    root.authority.id,
                    format!(
                        "js-api-candidate:{}:{}",
                        request.artifact.artifact_sha256, occurrence.candidate_id
                    ),
                );
                let capture_event_id = stable_child(candidate_id, b"static-capture:1");
                let fingerprint = static_fingerprint(occurrence)?;
                enumeration::persist_candidate_descriptor(
                    &mut tx,
                    &root.authority,
                    input,
                    &enumeration::CandidateDescriptorWrite {
                        id: candidate_id,
                        stable_candidate_request_id: stable_child(
                            request.stable_request_id,
                            format!("js-api-candidate:{}", occurrence.candidate_id),
                        ),
                        js_analysis_item_id: Some(descriptor_ids[&script.source_file]),
                        source_anchor: static_source_anchor(occurrence),
                        callsite_fingerprint: fingerprint.clone(),
                        capture_event_id,
                        capture_attempt_ordinal: 1,
                        captured_at: request.artifact.captured_at,
                        event_fingerprint: fingerprint,
                        duplicate_ordinal: 0,
                        resolution_input: occurrence
                            .canonical_url
                            .clone()
                            .unwrap_or_else(|| occurrence.raw_expression.clone()),
                    },
                )
                .await?;
                let (resolved_target_id, resolved_web_origin_id) = if occurrence.scope_decision
                    == "in_scope"
                    && occurrence.resolution_status == "resolved"
                {
                    let canonical_url = occurrence
                        .canonical_url
                        .as_deref()
                        .context("ENUMERATION_JS_API_CANONICAL_URL_MISSING")?;
                    let subject = exact_frozen_origin_subject(
                        &mut tx,
                        &root.authority,
                        request.target_id,
                        canonical_url,
                    )
                    .await?;
                    (Some(subject.0), Some(subject.1))
                } else {
                    (None, None)
                };
                ensure!(
                    !(occurrence.scope_decision == "scope_excluded"
                        && resolved_target_id.is_some()),
                    "ENUMERATION_JS_API_SCOPE_EXCLUDED_PROMOTION"
                );
                let route_kind = if occurrence.scope_decision == "scope_excluded" {
                    "dynamic_unresolved"
                } else {
                    static_route_kind(occurrence)?
                };
                let canonical_url = (occurrence.scope_decision == "in_scope")
                    .then(|| occurrence.canonical_url.clone())
                    .flatten();
                let route_template = (route_kind == "template")
                    .then(|| canonical_url.clone())
                    .flatten();
                enumeration::persist_endpoint_occurrence(
                    &mut tx,
                    &root.authority,
                    input,
                    &enumeration::EndpointOccurrenceWrite {
                        id: stable_child(candidate_id, b"js-api-occurrence"),
                        stable_occurrence_request_id: stable_child(
                            request.stable_request_id,
                            format!("js-api-occurrence:{}", occurrence.candidate_id),
                        ),
                        candidate_input_id: candidate_id,
                        capture_event_id,
                        source_target_id: request.target_id,
                        source_web_origin_id,
                        resolved_target_id,
                        resolved_web_origin_id,
                        parent_occurrence_id: None,
                        source_url: exact_origin.clone(),
                        document_url: script
                            .document_bases
                            .first()
                            .cloned()
                            .or_else(|| Some(exact_origin.clone())),
                        script_url: Some(script.manifest_url.clone()),
                        script_sha256: Some(script.content_sha256.clone()),
                        source_span: occurrence.source_span.clone(),
                        initiator_url: None,
                        initiator_status: "not_applicable".to_string(),
                        initiator_line: None,
                        initiator_column: None,
                        cdp_request_id_hash: None,
                        protocol: static_protocol(occurrence, &exact_origin)?.to_string(),
                        method: occurrence.method.to_ascii_uppercase(),
                        graphql_operation_name: occurrence.graphql_operation_name.clone(),
                        websocket_subprotocol: occurrence.websocket_subprotocol.clone(),
                        raw_expression: Some(occurrence.raw_expression.clone()),
                        receiver_kind: occurrence.receiver.clone(),
                        observation_kind: "static_ast".to_string(),
                        inference_level: "deterministic".to_string(),
                        resolution_status: occurrence.resolution_status.clone(),
                        scope_decision: occurrence.scope_decision.clone(),
                        candidate_classification: "endpoint".to_string(),
                        canonical_request_url: canonical_url.clone(),
                        display_url: canonical_url.clone(),
                        resolution_reason: occurrence.resolution_reason.clone(),
                        resolution_base_facts: serde_json::json!({
                            "selected_url": canonical_url,
                            "document_base": script.document_bases.first(),
                        }),
                        resolution_candidates: occurrence.resolution_chain.clone(),
                        resolution_chain: occurrence.resolution_chain.clone(),
                        route_kind: route_kind.to_string(),
                        route_template,
                        request_sent: occurrence.request_sent,
                        request_schema: serde_json::json!({
                            "schema_version": 2,
                            "fields": parameter_fields(&occurrence.parameter_facts),
                        }),
                        redaction_metadata: serde_json::json!({
                            "redacted": true,
                            "field_count": occurrence.parameter_facts.len(),
                            "policy_version": "value_free.v2",
                        }),
                        request_body_length: None,
                        runtime_sample_url: None,
                        observed_at: request.artifact.captured_at,
                    },
                    &occurrence_evidence,
                )
                .await?;
                if !unresolved {
                    enumeration::seal_enumeration_candidate_closure_in_connection(
                        &mut tx,
                        &root.authority,
                        &enumeration::SealEnumerationCandidateClosure {
                            stable_closure_request_id: stable_child(
                                request.stable_request_id,
                                format!("js-api-candidate-closure:{}", occurrence.candidate_id),
                            ),
                            candidate_input_id: candidate_id,
                            resolution_terminal_input: None,
                        },
                    )
                    .await?;
                }
            }
            if !denominator_has_unresolved {
                enumeration::seal_enumeration_candidate_denominator_closure_in_connection(
                    &mut tx,
                    &root.authority,
                    &enumeration::SealEnumerationCandidateDenominatorClosure {
                        stable_closure_request_id: stable_child(
                            namespace,
                            b"candidate-denominator-closure",
                        ),
                        denominator_id: denominator.id,
                    },
                )
                .await?;
            }
        }
        let (row, replayed) = enumeration::seal_enumeration_lane_commit_receipt(
            &mut tx,
            &root.authority,
            &lane_command(
                request.stable_request_id,
                "js_api",
                request.target_id,
                exact_origin,
                request.artifact.artifact_sha256,
                dependency_ids,
                evidence_ids,
                Some(script_denominator.id),
                candidate_denominator_ids,
                vec![],
                None,
                None,
                None,
            ),
        )
        .await?;
        tx.commit().await?;
        lane_receipt_view(row, replayed)
    }
}

impl GolishDbRepoProvider {
    pub(super) async fn enumeration_reduce_parameter_v2_impl(
        &self,
        request: ReduceEnumerationParameterV2,
    ) -> Result<EnumerationLaneClosureReceiptV2> {
        validate_identity(
            request.stable_request_id,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            request.worker_run_id,
            request.worker_attempt_epoch,
            request.lease_token,
            request.source_tool_call_id,
        )?;
        let exact_origin = canonical_origin(&request.exact_origin)?;
        let evidence_ids = canonical_ids(&request.evidence_audit_ids);
        ensure!(
            !evidence_ids.is_empty() && evidence_ids.len() == request.evidence_audit_ids.len(),
            "ENUMERATION_PARAMETER_EVIDENCE_INVALID"
        );
        let dependency_ids = canonical_ids(&[
            request.browser_receipt.receipt_id,
            request.js_api_receipt.receipt_id,
        ]);
        let artifact_sha256 = sha256_prefixed(&serde_json::json!({
            "browser_closure_graph_sha256": request.browser_receipt.closure_graph_sha256,
            "browser_receipt_id": request.browser_receipt.receipt_id,
            "browser_receipt_hash": request.browser_receipt.receipt_set_sha256,
            "evidence_audit_ids": evidence_ids,
            "exact_origin": exact_origin,
            "js_api_closure_graph_sha256": request.js_api_receipt.closure_graph_sha256,
            "js_api_receipt_id": request.js_api_receipt.receipt_id,
            "js_api_receipt_hash": request.js_api_receipt.receipt_set_sha256,
            "target_id": request.target_id,
        }))?;
        ensure!(
            request.stable_request_id
                == stable_child(request.source_tool_call_id, artifact_sha256.as_bytes()),
            "ENUMERATION_PARAMETER_STABLE_REQUEST_DRIFT"
        );
        let mut replay_tx = self.pool.begin().await?;
        if let Some(receipt) = response_loss_replay(
            &mut replay_tx,
            request.stable_request_id,
            "parameter",
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
            &artifact_sha256,
            &dependency_ids,
            &[],
        )
        .await?
        {
            let browser_row = load_named_receipt(
                &mut replay_tx,
                &request.browser_receipt,
                EnumerationLaneKindV2::Browser,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            let js_api_row = load_named_receipt(
                &mut replay_tx,
                &request.js_api_receipt,
                EnumerationLaneKindV2::JsApi,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            ensure!(
                js_api_row.dependency_receipt_ids == vec![browser_row.id],
                "ENUMERATION_PARAMETER_PRODUCER_DAG_DRIFT"
            );
            replay_tx.commit().await?;
            return Ok(receipt);
        }
        replay_tx.commit().await?;

        let source_root_denominator_id = self
            .enumeration_source_root(request.stage_execution_id, request.stage_run_unit_id)
            .await?;
        let mut tx = self.pool.begin().await?;
        let browser_row = load_named_receipt(
            &mut tx,
            &request.browser_receipt,
            EnumerationLaneKindV2::Browser,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await?;
        let js_api_row = load_named_receipt(
            &mut tx,
            &request.js_api_receipt,
            EnumerationLaneKindV2::JsApi,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await?;
        ensure!(
            js_api_row.dependency_receipt_ids == vec![browser_row.id],
            "ENUMERATION_PARAMETER_PRODUCER_DAG_DRIFT"
        );
        let root = enumeration::seal_enumeration_worker_authority_root_in_connection(
            &mut tx,
            &enumeration::SealEnumerationWorkerAuthorityRoot {
                stable_authority_request_id: stable_child(
                    request.stable_request_id,
                    b"parameter-authority",
                ),
                stable_root_request_id: stable_child(request.stable_request_id, b"parameter-root"),
                source_root_denominator_id,
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
            "ENUMERATION_PARAMETER_ROOT_AUTHORITY_MISMATCH"
        );
        enumeration::lock_enumeration_lane_subject(
            &mut tx,
            &root.authority,
            request.target_id,
            &exact_origin,
        )
        .await?;
        let (root_item_id, source_web_origin_id) = exact_root_subject(
            &mut tx,
            &root.authority,
            root.root_denominator.id,
            request.target_id,
            &exact_origin,
            "GOLISH-ENUM-PARAM",
        )
        .await?;
        let lane_evidence_ids = vec![
            book_enumeration_lane_derived_evidence(
                &mut tx,
                &root.authority,
                request.stable_request_id,
                "parameter",
                request.target_id,
                &exact_origin,
                &artifact_sha256,
                &evidence_ids,
                &enumeration::EnumerationWorkerFence {
                    worker_run_id: request.worker_run_id,
                    worker_attempt_epoch: request.worker_attempt_epoch,
                    lease_token: request.lease_token,
                    source_tool_call_id: request.source_tool_call_id,
                },
            )
            .await?,
        ];
        let evidence = enumeration::bind_enumeration_evidence_authorities(
            &mut tx,
            &root.authority,
            &lane_evidence_ids,
            "parameter",
        )
        .await?;
        let producer_authority_ids = canonical_ids(&[
            browser_row.execution_authority_id,
            js_api_row.execution_authority_id,
        ]);
        let occurrences = sqlx::query_as::<_, ParameterOccurrenceRow>(
            r#"SELECT occurrence.id,occurrence.source_url,occurrence.canonical_request_url,
                      occurrence.resolution_status,occurrence.scope_decision,
                      occurrence.request_schema
                 FROM enumeration_endpoint_occurrences occurrence
                WHERE occurrence.execution_authority_id=ANY($1)
                  AND occurrence.source_target_id=$2
                  AND occurrence.source_web_origin_id=$3
                ORDER BY occurrence.id"#,
        )
        .bind(&producer_authority_ids)
        .bind(request.target_id)
        .bind(source_web_origin_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut parameter_denominator_ids = Vec::with_capacity(occurrences.len());
        for (ordinal, occurrence) in occurrences.iter().enumerate() {
            let namespace = stable_child(
                request.stable_request_id,
                format!("parameter-denominator:{}", occurrence.id),
            );
            let input_key = format!("parameter:{}", occurrence.id);
            let denominator = enumeration::seal_enumeration_derived_denominator_in_connection(
                &mut tx,
                &root.authority,
                &enumeration::SealEnumerationDerivedDenominator {
                    stable_seal_request_id: namespace,
                    parent_denominator_id: root.root_denominator.id,
                    parent_denominator_item_id: root_item_id,
                    derived_ordinal: i32::try_from(ordinal + 1)?,
                    items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                        input_key: input_key.clone(),
                        target_id: request.target_id,
                        exact_asset: occurrence
                            .canonical_request_url
                            .clone()
                            .unwrap_or_else(|| occurrence.source_url.clone()),
                        technique: "reduce_parameter_facts".to_string(),
                        expected_capability: "enumeration.parameter".to_string(),
                    }],
                },
            )
            .await?;
            parameter_denominator_ids.push(denominator.id);
            let denominator_item_id = stable_child(denominator.id, input_key.as_bytes());
            let fields = decode_parameter_fields(&occurrence.request_schema)?;
            let (terminal_outcome, assessment_outcome, reason_code) =
                if occurrence.scope_decision == "scope_excluded" {
                    (
                        enumeration::EnumerationTerminalInputOutcome::NotApplicable,
                        "not_applicable",
                        "scope_excluded",
                    )
                } else if !fields.is_empty() {
                    (
                        enumeration::EnumerationTerminalInputOutcome::Found,
                        "found",
                        "parameter_facts_reduced",
                    )
                } else if matches!(
                    occurrence.resolution_status.as_str(),
                    "ambiguous" | "unresolved"
                ) {
                    (
                        enumeration::EnumerationTerminalInputOutcome::UnresolvedExhausted {
                            coverage_gap_reason: "source_unavailable".to_string(),
                        },
                        "unresolved",
                        "endpoint_resolution_pending",
                    )
                } else {
                    (
                        enumeration::EnumerationTerminalInputOutcome::CheckedEmpty,
                        "checked_empty",
                        "no_parameter_facts_observed",
                    )
                };
            let fields = if assessment_outcome == "found" {
                fields
            } else {
                Vec::new()
            };
            let inputs = begin_and_seal_inputs(
                &mut tx,
                &root.authority,
                namespace,
                &denominator,
                "enumeration.parameter",
                vec![enumeration::EnumerationTerminalReceiptInputWrite {
                    denominator_item_id,
                    outcome: terminal_outcome,
                    evidence_authorities: evidence.clone(),
                }],
            )
            .await?;
            ensure!(
                inputs.len() == 1,
                "ENUMERATION_PARAMETER_TERMINAL_INPUT_DRIFT"
            );
            let assessment_id = stable_child(
                root.authority.id,
                format!("parameter-assessment:{}", occurrence.id),
            );
            enumeration::persist_parameter_assessment(
                &mut tx,
                &root.authority,
                &inputs[0],
                &enumeration::ParameterAssessmentWrite {
                    id: assessment_id,
                    occurrence_id: occurrence.id,
                    outcome: assessment_outcome.to_string(),
                    reason_code: reason_code.to_string(),
                    parameters: fields
                        .iter()
                        .enumerate()
                        .map(|(field_ordinal, field)| {
                            Ok(enumeration::OccurrenceParameterWrite {
                                id: stable_child(
                                    assessment_id,
                                    format!(
                                        "parameter:{field_ordinal}:{}:{}",
                                        field.location, field.name
                                    ),
                                ),
                                name: field.name.clone(),
                                location: field.location.clone(),
                                value_type: field.value_type.clone(),
                                requirement: field.requirement.clone(),
                                confidence: field.confidence,
                                source_anchor_ids: field.source_anchor_ids.clone(),
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
                &evidence,
            )
            .await?;
        }
        enumeration::project_endpoint_groups(
            &mut tx,
            &root.authority,
            request.browser_receipt.receipt_id,
            request.js_api_receipt.receipt_id,
        )
        .await?;
        let (row, replayed) = enumeration::seal_enumeration_lane_commit_receipt(
            &mut tx,
            &root.authority,
            &lane_command(
                request.stable_request_id,
                "parameter",
                request.target_id,
                exact_origin,
                artifact_sha256,
                dependency_ids,
                lane_evidence_ids,
                None,
                vec![],
                parameter_denominator_ids,
                None,
                None,
                None,
            ),
        )
        .await?;
        tx.commit().await?;
        lane_receipt_view(row, replayed)
    }

    pub(super) async fn enumeration_close_resolution_v2_impl(
        &self,
        request: CloseEnumerationResolutionV2,
    ) -> Result<EnumerationLaneClosureReceiptV2> {
        validate_identity(
            request.stable_request_id,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            request.worker_run_id,
            request.worker_attempt_epoch,
            request.lease_token,
            request.source_tool_call_id,
        )?;
        ensure!(
            !request.resolution_work_item_id.is_nil() && !request.unresolved_occurrence_id.is_nil(),
            "ENUMERATION_RESOLUTION_ASSIGNMENT_INVALID"
        );
        let exact_origin = canonical_origin(&request.exact_origin)?;
        let evidence_ids = canonical_ids(&request.evidence_audit_ids);
        ensure!(
            !evidence_ids.is_empty() && evidence_ids.len() == request.evidence_audit_ids.len(),
            "ENUMERATION_RESOLUTION_EVIDENCE_INVALID"
        );
        let reason_code = request.reason_code.trim();
        let lower_reason = reason_code.to_ascii_lowercase();
        ensure!(
            !reason_code.is_empty()
                && reason_code.len() <= 256
                && !reason_code
                    .chars()
                    .any(|character| matches!(character, '\n' | '\r' | '\0'))
                && ![
                    "authorization:",
                    "cookie:",
                    "password=",
                    "secret=",
                    "token=",
                    "api_key=",
                    "api-key=",
                    "bearer ",
                ]
                .iter()
                .any(|needle| lower_reason.contains(needle)),
            "ENUMERATION_RESOLUTION_REASON_INVALID"
        );
        let terminal_state = match request.terminal_state {
            EnumerationResolutionTerminalStateV2::AdvisoryResidual => "advisory_residual",
            EnumerationResolutionTerminalStateV2::BudgetExhausted => "budget_exhausted",
            EnumerationResolutionTerminalStateV2::Unsupported => "unsupported",
        };
        ensure!(
            matches!(
                request.producer_receipt.lane,
                EnumerationLaneKindV2::Browser | EnumerationLaneKindV2::JsApi
            ),
            "ENUMERATION_RESOLUTION_PRODUCER_LANE_INVALID"
        );
        let dependency_ids = vec![request.producer_receipt.receipt_id];
        let artifact_sha256 = sha256_prefixed(&serde_json::json!({
            "evidence_audit_ids": evidence_ids,
            "exact_origin": exact_origin,
            "producer_closure_graph_sha256": request.producer_receipt.closure_graph_sha256,
            "producer_receipt_hash": request.producer_receipt.receipt_set_sha256,
            "producer_receipt_id": request.producer_receipt.receipt_id,
            "reason_code": reason_code,
            "resolution_work_item_id": request.resolution_work_item_id,
            "target_id": request.target_id,
            "terminal_state": terminal_state,
            "unresolved_occurrence_id": request.unresolved_occurrence_id,
        }))?;
        ensure!(
            request.stable_request_id
                == stable_child(request.source_tool_call_id, artifact_sha256.as_bytes()),
            "ENUMERATION_RESOLUTION_STABLE_REQUEST_DRIFT"
        );

        let mut replay_tx = self.pool.begin().await?;
        if let Some(receipt) = response_loss_replay(
            &mut replay_tx,
            request.stable_request_id,
            "resolution",
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
            &artifact_sha256,
            &dependency_ids,
            &[],
        )
        .await
        .context("ENUMERATION_RESOLUTION_RESPONSE_LOSS_REPLAY")?
        {
            load_named_receipt(
                &mut replay_tx,
                &request.producer_receipt,
                request.producer_receipt.lane,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            replay_tx.commit().await?;
            return Ok(receipt);
        }
        replay_tx.commit().await?;

        let source_root_denominator_id = self
            .enumeration_source_root(request.stage_execution_id, request.stage_run_unit_id)
            .await
            .context("ENUMERATION_RESOLUTION_SOURCE_ROOT")?;
        let mut tx = self.pool.begin().await?;
        enumeration::lock_enumeration_subject_identity(
            &mut tx,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await
        .context("ENUMERATION_RESOLUTION_SUBJECT_IDENTITY")?;
        let producer_row = load_named_receipt(
            &mut tx,
            &request.producer_receipt,
            request.producer_receipt.lane,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await
        .context("ENUMERATION_RESOLUTION_PRODUCER_RECEIPT")?;
        let occurrence = sqlx::query_as::<_, ResolutionOccurrenceRow>(
            r#"SELECT occurrence.id,occurrence.candidate_input_id,
                      candidate.denominator_id AS candidate_denominator_id
                 FROM enumeration_endpoint_occurrences occurrence
                 JOIN enumeration_endpoint_candidate_inputs candidate
                   ON candidate.id=occurrence.candidate_input_id
                  AND candidate.execution_authority_id=occurrence.execution_authority_id
                 JOIN web_origins origin
                   ON origin.id=occurrence.source_web_origin_id
                  AND origin.organization_id=occurrence.organization_id
                  AND origin.project_path=occurrence.project_path_at_freeze
                WHERE occurrence.id=$1
                  AND occurrence.execution_authority_id=$2
                  AND occurrence.operation_id=$3
                  AND occurrence.organization_id=$4
                  AND occurrence.stage_execution_id=$5
                  AND occurrence.source_target_id=$6
                  AND origin.origin=$7
                  AND occurrence.resolution_status IN ('ambiguous','unresolved')
                  AND occurrence.scope_decision='in_scope'
                  AND occurrence.candidate_classification='endpoint'
                FOR SHARE OF occurrence,candidate,origin"#,
        )
        .bind(request.unresolved_occurrence_id)
        .bind(producer_row.execution_authority_id)
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(request.stage_execution_id)
        .bind(request.target_id)
        .bind(&exact_origin)
        .fetch_optional(&mut *tx)
        .await?
        .context("ENUMERATION_RESOLUTION_OCCURRENCE_NOT_OWNED_BY_RECEIPT")?;
        ensure!(
            occurrence.id == request.unresolved_occurrence_id,
            "ENUMERATION_RESOLUTION_OCCURRENCE_DRIFT"
        );
        let root = enumeration::seal_enumeration_worker_authority_root_in_connection(
            &mut tx,
            &enumeration::SealEnumerationWorkerAuthorityRoot {
                stable_authority_request_id: stable_child(
                    request.stable_request_id,
                    b"resolution-authority",
                ),
                stable_root_request_id: stable_child(request.stable_request_id, b"resolution-root"),
                source_root_denominator_id,
                worker_fence: enumeration::EnumerationWorkerFence {
                    worker_run_id: request.worker_run_id,
                    worker_attempt_epoch: request.worker_attempt_epoch,
                    lease_token: request.lease_token,
                    source_tool_call_id: request.source_tool_call_id,
                },
            },
        )
        .await
        .context("ENUMERATION_RESOLUTION_WORKER_AUTHORITY_ROOT")?;
        ensure!(
            root.authority.operation_id == request.operation_id
                && root.authority.organization_id == request.organization_id
                && root.authority.stage_execution_id == request.stage_execution_id,
            "ENUMERATION_RESOLUTION_ROOT_AUTHORITY_MISMATCH"
        );
        enumeration::lock_enumeration_lane_subject(
            &mut tx,
            &root.authority,
            request.target_id,
            &exact_origin,
        )
        .await
        .context("ENUMERATION_RESOLUTION_LANE_SUBJECT")?;
        let (root_item_id, _) = exact_root_subject(
            &mut tx,
            &root.authority,
            root.root_denominator.id,
            request.target_id,
            &exact_origin,
            "GOLISH-ENUM-JSAPI",
        )
        .await
        .context("ENUMERATION_RESOLUTION_ROOT_SUBJECT")?;
        let lane_evidence_ids = vec![book_enumeration_lane_derived_evidence(
            &mut tx,
            &root.authority,
            request.stable_request_id,
            "resolution",
            request.target_id,
            &exact_origin,
            &artifact_sha256,
            &evidence_ids,
            &enumeration::EnumerationWorkerFence {
                worker_run_id: request.worker_run_id,
                worker_attempt_epoch: request.worker_attempt_epoch,
                lease_token: request.lease_token,
                source_tool_call_id: request.source_tool_call_id,
            },
        )
        .await
        .context("ENUMERATION_RESOLUTION_DERIVED_EVIDENCE")?];
        let evidence = enumeration::bind_enumeration_evidence_authorities(
            &mut tx,
            &root.authority,
            &lane_evidence_ids,
            "resolution",
        )
        .await
        .context("ENUMERATION_RESOLUTION_EVIDENCE_AUTHORITY")?;
        let denominator_namespace =
            stable_child(request.stable_request_id, b"resolution-denominator");
        let input_key = format!("resolution:{}", request.unresolved_occurrence_id);
        let denominator = enumeration::seal_enumeration_derived_denominator_in_connection(
            &mut tx,
            &root.authority,
            &enumeration::SealEnumerationDerivedDenominator {
                stable_seal_request_id: denominator_namespace,
                parent_denominator_id: root.root_denominator.id,
                parent_denominator_item_id: root_item_id,
                derived_ordinal: 1,
                items: vec![enumeration::EnumerationDerivedDenominatorItemWrite {
                    input_key: input_key.clone(),
                    target_id: request.target_id,
                    exact_asset: exact_origin.clone(),
                    technique: "resolve_unresolved_occurrence".to_string(),
                    expected_capability: "enumeration.js_api".to_string(),
                }],
            },
        )
        .await
        .context("ENUMERATION_RESOLUTION_DENOMINATOR")?;
        let gap_reason = match request.terminal_state {
            EnumerationResolutionTerminalStateV2::BudgetExhausted => "budget_exhausted",
            EnumerationResolutionTerminalStateV2::AdvisoryResidual
            | EnumerationResolutionTerminalStateV2::Unsupported => "unsupported",
        };
        let inputs = begin_and_seal_inputs(
            &mut tx,
            &root.authority,
            denominator_namespace,
            &denominator,
            "enumeration.js_api",
            vec![enumeration::EnumerationTerminalReceiptInputWrite {
                denominator_item_id: stable_child(denominator.id, input_key.as_bytes()),
                outcome: enumeration::EnumerationTerminalInputOutcome::UnresolvedExhausted {
                    coverage_gap_reason: gap_reason.to_string(),
                },
                evidence_authorities: evidence,
            }],
        )
        .await
        .context("ENUMERATION_RESOLUTION_TERMINAL_INPUT")?;
        ensure!(
            inputs.len() == 1,
            "ENUMERATION_RESOLUTION_TERMINAL_INPUT_DRIFT"
        );
        let suggestion_ids = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT suggestion.id
                 FROM enumeration_js_resolution_suggestions suggestion
                WHERE suggestion.assigned_work_item_id=$1
                  AND suggestion.assigned_cluster_id=$2
                  AND suggestion.parent_occurrence_id=$2
                  AND suggestion.worker_run_id=$3
                ORDER BY suggestion.id
                FOR SHARE"#,
        )
        .bind(request.resolution_work_item_id)
        .bind(request.unresolved_occurrence_id)
        .bind(request.worker_run_id)
        .fetch_all(&mut *tx)
        .await
        .context("ENUMERATION_RESOLUTION_TYPED_CLOSEOUT")?;
        ensure!(
            match request.terminal_state {
                EnumerationResolutionTerminalStateV2::AdvisoryResidual => {
                    !suggestion_ids.is_empty()
                }
                EnumerationResolutionTerminalStateV2::BudgetExhausted
                | EnumerationResolutionTerminalStateV2::Unsupported => {
                    suggestion_ids.is_empty()
                }
            },
            "ENUMERATION_RESOLUTION_SUGGESTION_TERMINAL_STATE_DRIFT"
        );
        enumeration::seal_enumeration_resolution_closeout(
            &mut tx,
            &root.authority,
            &enumeration::SealEnumerationResolutionCloseout {
                stable_closeout_request_id: stable_child(
                    request.stable_request_id,
                    b"resolution-closeout",
                ),
                assigned_work_item_id: request.resolution_work_item_id,
                worker_fence: enumeration::EnumerationWorkerFence {
                    worker_run_id: request.worker_run_id,
                    worker_attempt_epoch: request.worker_attempt_epoch,
                    lease_token: request.lease_token,
                    source_tool_call_id: request.source_tool_call_id,
                },
                parent_occurrence_id: request.unresolved_occurrence_id,
                producer_lane_receipt_id: request.producer_receipt.receipt_id,
                terminal_state: terminal_state.to_string(),
                reason_code: reason_code.to_string(),
                suggestion_ids,
                terminal_receipt_id: inputs[0].receipt_id,
                terminal_receipt_input_id: inputs[0].receipt_input_id,
            },
        )
        .await?;
        let producer_authority = authority_for_lane_receipt(&mut tx, &producer_row).await?;
        let unresolved_without_closeout: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)::BIGINT
                 FROM enumeration_endpoint_occurrences sibling
                WHERE sibling.candidate_input_id=$1
                  AND sibling.execution_authority_id=$2
                  AND sibling.resolution_status IN ('ambiguous','unresolved')
                  AND sibling.scope_decision='in_scope'
                  AND sibling.candidate_classification='endpoint'
                  AND NOT EXISTS (
                      SELECT 1 FROM enumeration_resolution_closeout_receipts closeout
                       WHERE closeout.parent_occurrence_id=sibling.id
                  )"#,
        )
        .bind(occurrence.candidate_input_id)
        .bind(producer_row.execution_authority_id)
        .fetch_one(&mut *tx)
        .await?;
        if unresolved_without_closeout == 0 {
            enumeration::seal_enumeration_candidate_closure_in_connection(
                &mut tx,
                &producer_authority,
                &enumeration::SealEnumerationCandidateClosure {
                    stable_closure_request_id: stable_child(
                        occurrence.candidate_input_id,
                        b"resolution-candidate-closure-v2",
                    ),
                    candidate_input_id: occurrence.candidate_input_id,
                    resolution_terminal_input: Some(inputs[0].clone()),
                },
            )
            .await?;
        }
        let open_candidate_count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)::BIGINT
                 FROM enumeration_endpoint_candidate_inputs candidate
                 LEFT JOIN enumeration_endpoint_candidate_closure_receipts closure
                   ON closure.candidate_input_id=candidate.id
                WHERE candidate.denominator_id=$1
                  AND candidate.execution_authority_id=$2
                  AND closure.id IS NULL"#,
        )
        .bind(occurrence.candidate_denominator_id)
        .bind(producer_row.execution_authority_id)
        .fetch_one(&mut *tx)
        .await?;
        if open_candidate_count == 0 {
            enumeration::seal_enumeration_candidate_denominator_closure_in_connection(
                &mut tx,
                &producer_authority,
                &enumeration::SealEnumerationCandidateDenominatorClosure {
                    stable_closure_request_id: stable_child(
                        occurrence.candidate_denominator_id,
                        b"resolution-denominator-closure-v2",
                    ),
                    denominator_id: occurrence.candidate_denominator_id,
                },
            )
            .await?;
        }
        let (row, replayed) = enumeration::seal_enumeration_lane_commit_receipt(
            &mut tx,
            &root.authority,
            &resolution_lane_command(
                request.stable_request_id,
                request.target_id,
                exact_origin,
                artifact_sha256,
                dependency_ids,
                lane_evidence_ids,
                request.unresolved_occurrence_id,
                inputs[0].receipt_id,
                inputs[0].receipt_input_id,
            ),
        )
        .await?;
        tx.commit().await?;
        lane_receipt_view(row, replayed)
    }

    pub(super) async fn enumeration_review_coverage_v2_impl(
        &self,
        request: ReviewEnumerationCoverageV2,
    ) -> Result<EnumerationLaneClosureReceiptV2> {
        validate_identity(
            request.stable_request_id,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            request.worker_run_id,
            request.worker_attempt_epoch,
            request.lease_token,
            request.source_tool_call_id,
        )?;
        let exact_origin = canonical_origin(&request.exact_origin)?;
        let evidence_ids = canonical_ids(&request.evidence_audit_ids);
        ensure!(
            !evidence_ids.is_empty() && evidence_ids.len() == request.evidence_audit_ids.len(),
            "ENUMERATION_COVERAGE_EVIDENCE_INVALID"
        );
        let mut resolution_receipts = request.resolution_receipts.clone();
        resolution_receipts.sort_by_key(|receipt| receipt.receipt_id);
        ensure!(
            resolution_receipts
                .windows(2)
                .all(|pair| pair[0].receipt_id < pair[1].receipt_id)
                && resolution_receipts
                    .iter()
                    .all(|receipt| receipt.lane == EnumerationLaneKindV2::Resolution),
            "ENUMERATION_COVERAGE_RESOLUTION_RECEIPT_SET_INVALID"
        );
        let dependency_ids = canonical_ids(
            &[
                request.browser_receipt.receipt_id,
                request.js_api_receipt.receipt_id,
                request.parameter_receipt.receipt_id,
            ]
            .into_iter()
            .chain(resolution_receipts.iter().map(|receipt| receipt.receipt_id))
            .collect::<Vec<_>>(),
        );
        let resolution_hashes = resolution_receipts
            .iter()
            .map(|receipt| {
                serde_json::json!({
                    "closure_graph_sha256": receipt.closure_graph_sha256,
                    "occurrence_id": receipt.resolution_occurrence_id,
                    "receipt_id": receipt.receipt_id,
                    "receipt_set_sha256": receipt.receipt_set_sha256,
                })
            })
            .collect::<Vec<_>>();
        let artifact_sha256 = sha256_prefixed(&serde_json::json!({
            "browser_closure_graph_sha256": request.browser_receipt.closure_graph_sha256,
            "browser_receipt_hash": request.browser_receipt.receipt_set_sha256,
            "browser_receipt_id": request.browser_receipt.receipt_id,
            "evidence_audit_ids": evidence_ids,
            "exact_origin": exact_origin,
            "js_api_closure_graph_sha256": request.js_api_receipt.closure_graph_sha256,
            "js_api_receipt_hash": request.js_api_receipt.receipt_set_sha256,
            "js_api_receipt_id": request.js_api_receipt.receipt_id,
            "parameter_closure_graph_sha256": request.parameter_receipt.closure_graph_sha256,
            "parameter_receipt_hash": request.parameter_receipt.receipt_set_sha256,
            "parameter_receipt_id": request.parameter_receipt.receipt_id,
            "resolution_receipts": resolution_hashes,
            "target_id": request.target_id,
        }))?;
        ensure!(
            request.stable_request_id
                == stable_child(request.source_tool_call_id, artifact_sha256.as_bytes()),
            "ENUMERATION_COVERAGE_STABLE_REQUEST_DRIFT"
        );

        let mut replay_tx = self.pool.begin().await?;
        if let Some(receipt) = response_loss_replay(
            &mut replay_tx,
            request.stable_request_id,
            "coverage",
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
            &artifact_sha256,
            &dependency_ids,
            &[],
        )
        .await?
        {
            load_named_receipt(
                &mut replay_tx,
                &request.browser_receipt,
                EnumerationLaneKindV2::Browser,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            load_named_receipt(
                &mut replay_tx,
                &request.js_api_receipt,
                EnumerationLaneKindV2::JsApi,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            load_named_receipt(
                &mut replay_tx,
                &request.parameter_receipt,
                EnumerationLaneKindV2::Parameter,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            for resolution in &resolution_receipts {
                load_named_receipt(
                    &mut replay_tx,
                    resolution,
                    EnumerationLaneKindV2::Resolution,
                    request.operation_id,
                    request.organization_id,
                    request.stage_execution_id,
                    request.stage_run_unit_id,
                    request.target_id,
                    &exact_origin,
                )
                .await?;
            }
            replay_tx.commit().await?;
            return Ok(receipt);
        }
        replay_tx.commit().await?;

        let source_root_denominator_id = self
            .enumeration_source_root(request.stage_execution_id, request.stage_run_unit_id)
            .await?;
        let mut tx = self.pool.begin().await?;
        let browser_row = load_named_receipt(
            &mut tx,
            &request.browser_receipt,
            EnumerationLaneKindV2::Browser,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await?;
        let js_api_row = load_named_receipt(
            &mut tx,
            &request.js_api_receipt,
            EnumerationLaneKindV2::JsApi,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await?;
        load_named_receipt(
            &mut tx,
            &request.parameter_receipt,
            EnumerationLaneKindV2::Parameter,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &exact_origin,
        )
        .await?;
        ensure!(
            request.js_api_receipt.dependency_receipt_ids
                == vec![request.browser_receipt.receipt_id]
                && request.parameter_receipt.dependency_receipt_ids
                    == canonical_ids(&[
                        request.browser_receipt.receipt_id,
                        request.js_api_receipt.receipt_id,
                    ]),
            "ENUMERATION_COVERAGE_PRODUCER_DAG_DRIFT"
        );
        for resolution in &resolution_receipts {
            load_named_receipt(
                &mut tx,
                resolution,
                EnumerationLaneKindV2::Resolution,
                request.operation_id,
                request.organization_id,
                request.stage_execution_id,
                request.stage_run_unit_id,
                request.target_id,
                &exact_origin,
            )
            .await?;
            ensure!(
                resolution.dependency_receipt_ids.len() == 1
                    && matches!(
                        resolution.dependency_receipt_ids[0],
                        id if id == request.browser_receipt.receipt_id
                            || id == request.js_api_receipt.receipt_id
                    ),
                "ENUMERATION_COVERAGE_RESOLUTION_DAG_DRIFT"
            );
        }
        let producer_authority_ids = canonical_ids(&[
            browser_row.execution_authority_id,
            js_api_row.execution_authority_id,
        ]);
        let unresolved_occurrence_ids = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT occurrence.id
                 FROM enumeration_endpoint_occurrences occurrence
                 JOIN web_origins origin
                   ON origin.id=occurrence.source_web_origin_id
                  AND origin.organization_id=occurrence.organization_id
                  AND origin.project_path=occurrence.project_path_at_freeze
                WHERE occurrence.execution_authority_id=ANY($1)
                  AND occurrence.operation_id=$2
                  AND occurrence.organization_id=$3
                  AND occurrence.stage_execution_id=$4
                  AND occurrence.source_target_id=$5
                  AND origin.origin=$6
                  AND occurrence.resolution_status IN ('ambiguous','unresolved')
                  AND occurrence.scope_decision='in_scope'
                  AND occurrence.candidate_classification='endpoint'
                ORDER BY occurrence.id
                FOR SHARE OF occurrence,origin"#,
        )
        .bind(&producer_authority_ids)
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(request.stage_execution_id)
        .bind(request.target_id)
        .bind(&exact_origin)
        .fetch_all(&mut *tx)
        .await?;
        let resolution_occurrence_ids = resolution_receipts
            .iter()
            .map(|receipt| {
                receipt
                    .resolution_occurrence_id
                    .context("ENUMERATION_COVERAGE_RESOLUTION_OCCURRENCE_MISSING")
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            unresolved_occurrence_ids == canonical_ids(&resolution_occurrence_ids),
            "ENUMERATION_COVERAGE_UNRESOLVED_EXACT_SET_DRIFT"
        );
        let root = enumeration::seal_enumeration_worker_authority_root_in_connection(
            &mut tx,
            &enumeration::SealEnumerationWorkerAuthorityRoot {
                stable_authority_request_id: stable_child(
                    request.stable_request_id,
                    b"coverage-authority",
                ),
                stable_root_request_id: stable_child(request.stable_request_id, b"coverage-root"),
                source_root_denominator_id,
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
            "ENUMERATION_COVERAGE_ROOT_AUTHORITY_MISMATCH"
        );
        enumeration::lock_enumeration_lane_subject(
            &mut tx,
            &root.authority,
            request.target_id,
            &exact_origin,
        )
        .await?;
        let lane_evidence_ids = vec![
            book_enumeration_lane_derived_evidence(
                &mut tx,
                &root.authority,
                request.stable_request_id,
                "coverage",
                request.target_id,
                &exact_origin,
                &artifact_sha256,
                &evidence_ids,
                &enumeration::EnumerationWorkerFence {
                    worker_run_id: request.worker_run_id,
                    worker_attempt_epoch: request.worker_attempt_epoch,
                    lease_token: request.lease_token,
                    source_tool_call_id: request.source_tool_call_id,
                },
            )
            .await?,
        ];
        enumeration::bind_enumeration_evidence_authorities(
            &mut tx,
            &root.authority,
            &lane_evidence_ids,
            "coverage",
        )
        .await?;
        let (row, replayed) = enumeration::seal_enumeration_lane_commit_receipt(
            &mut tx,
            &root.authority,
            &coverage_lane_command(
                request.stable_request_id,
                request.target_id,
                exact_origin,
                artifact_sha256,
                dependency_ids,
                lane_evidence_ids,
            ),
        )
        .await?;
        tx.commit().await?;
        lane_receipt_view(row, replayed)
    }
}
