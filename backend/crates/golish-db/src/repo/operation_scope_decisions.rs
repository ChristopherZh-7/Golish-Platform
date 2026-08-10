//! Trusted Scoping decision schema contract.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::repo::tool_calls::{self, ExactScopingLifecycleRow};

pub const TABLE_NAME: &str = "operation_scope_decisions";
pub const MODE_CHECK_SQL: &str =
    "CHECK (mode IN ('root_only','included','reuse_reconfirmed','cli_flags'))";
pub const OPERATION_EXECUTION_UNIQUE_SQL: &str = "UNIQUE(operation_id, stage_execution_id)";
pub const STAGE_EXECUTION_OWNER_FK_SQL: &str =
    "FOREIGN KEY(stage_execution_id, operation_id) REFERENCES stage_runs(id, operation_id)";

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationScopeDecisionRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
    pub mode: String,
    pub choice_tool_call_id: Option<Uuid>,
    pub proposal_tool_call_id: Option<Uuid>,
    pub review_tool_call_id: Option<Uuid>,
    pub decision_rows: Value,
    pub decision_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScopeDecisionError {
    #[error("scope decision identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("scope decision conflict: {code}")]
    Conflict { code: &'static str },
    #[error("scope decision row missing: {entity}")]
    Missing { entity: &'static str },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Repository(#[from] crate::DbError),
}

impl ScopeDecisionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IdentityMismatch { code } | Self::Conflict { code } => code,
            Self::Missing { entity } => entity,
            Self::Sqlx(_) | Self::Repository(_) => "scope_decision_storage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactScopeDecisionInput {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovedOrgUnit {
    pub decision_row_id: String,
    pub candidate_id: String,
    pub organization_id: Uuid,
    pub parent_organization_id: Option<Uuid>,
    pub organization_name: String,
    pub depth: i32,
    pub ordinal: i32,
    pub ownership_percent: Option<String>,
    pub aliases: Vec<String>,
    pub domains: Vec<String>,
    pub approval_source: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovedOrgScopeDecision {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
    pub mode: ScopeDecisionMode,
    pub units: Vec<ApprovedOrgUnit>,
    pub choice_tool_call_id: Option<Uuid>,
    pub proposal_tool_call_id: Option<Uuid>,
    pub review_tool_call_id: Option<Uuid>,
    pub decision_rows: Value,
    pub decision_hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScopeDecisionMode {
    RootOnly,
    Included,
    ReuseReconfirmed,
    CliFlags,
}

impl ScopeDecisionMode {
    pub const ALL: [Self; 4] = [
        Self::RootOnly,
        Self::Included,
        Self::ReuseReconfirmed,
        Self::CliFlags,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RootOnly => "root_only",
            Self::Included => "included",
            Self::ReuseReconfirmed => "reuse_reconfirmed",
            Self::CliFlags => "cli_flags",
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct LockedScopeRoot {
    organization_name: String,
    intel: Value,
}

#[derive(Debug, sqlx::FromRow)]
struct DescendantOrganization {
    id: Uuid,
    name: String,
    parent_id: Option<Uuid>,
    depth: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubsidiaryChoice {
    RootOnly,
    Included,
}

#[derive(Debug, Clone)]
struct ParsedReviewRow {
    review_row_id: String,
    candidate_id: String,
    organization_id: Option<Uuid>,
    name: String,
    aliases: Vec<String>,
    domains: Vec<String>,
    ownership_percent: Option<String>,
    included: bool,
    raw: Value,
}

pub async fn derive_exact(
    pool: &PgPool,
    input: &ExactScopeDecisionInput,
) -> Result<ApprovedOrgScopeDecision, ScopeDecisionError> {
    let mut connection = pool.acquire().await?;
    derive_exact_with_connection(&mut connection, input).await
}

/// Authorize the one pre-freeze passive recon action needed by Scoping.
///
/// The requested organization id is untrusted model input until this query
/// binds it to the exact operation, Scoping execution, project root and latest
/// successful human subsidiary choice. Identity mismatches are ordinary
/// denials so callers can fail closed without leaking cross-operation detail.
pub async fn scoping_passive_recon_organization_authorized(
    pool: &PgPool,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    root_organization_id: Uuid,
) -> Result<bool, ScopeDecisionError> {
    let mut connection = pool.acquire().await?;
    let exact_root_exists = sqlx::query_scalar::<_, bool>(
        r#"SELECT TRUE
             FROM operation_state AS operation
             JOIN project_scopes AS project
               ON project.project_scope_id = operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN stage_runs AS execution
               ON execution.id = $2
              AND execution.operation_id = operation.operation_id
              AND execution.stage_kind = 'scoping'
              AND execution.status = 'started'
             JOIN organizations AS organization
               ON organization.id = $3
              AND organization.project_path = project.canonical_project_path
              AND organization.parent_id IS NULL
            WHERE operation.operation_id = $1
              AND operation.current_stage = 'scoping'
              AND operation.superseded_by IS NULL"#,
    )
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(root_organization_id)
    .fetch_optional(&mut *connection)
    .await?
    .is_some();
    if !exact_root_exists {
        return Ok(false);
    }

    let lifecycle = tool_calls::scoping_lifecycle_for_execution_with_connection(
        &mut connection,
        operation_id,
        stage_execution_id,
    )
    .await?;
    Ok(matches!(
        latest_subsidiary_choice(&lifecycle, root_organization_id, None),
        Some((_, SubsidiaryChoice::Included))
    ))
}

pub async fn derive_exact_with_connection(
    connection: &mut PgConnection,
    input: &ExactScopeDecisionInput,
) -> Result<ApprovedOrgScopeDecision, ScopeDecisionError> {
    let root = sqlx::query_as::<_, LockedScopeRoot>(
        r#"SELECT organization.name AS organization_name, organization.intel
             FROM operation_state AS operation
             JOIN project_scopes AS project
               ON project.project_scope_id = operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN stage_runs AS execution
               ON execution.id = $3
              AND execution.operation_id = operation.operation_id
              AND execution.stage_kind = 'scoping'
              AND execution.status = 'started'
             JOIN organizations AS organization
               ON organization.id = $4
              AND organization.project_path = project.canonical_project_path
              AND organization.parent_id IS NULL
            WHERE operation.operation_id = $1
              AND operation.project_scope_id = $2
              AND operation.current_stage = 'scoping'
              AND operation.superseded_by IS NULL
            FOR SHARE OF operation, project, execution, organization"#,
    )
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.stage_execution_id)
    .bind(input.root_organization_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(ScopeDecisionError::IdentityMismatch {
        code: "scope_root_identity_mismatch",
    })?;

    let lifecycle = tool_calls::scoping_lifecycle_for_execution_with_connection(
        connection,
        input.operation_id,
        input.stage_execution_id,
    )
    .await?;
    let (choice_row, choice) = latest_subsidiary_choice(
        &lifecycle,
        input.root_organization_id,
        Some(&root.organization_name),
    )
    .ok_or(ScopeDecisionError::Missing {
        entity: "scope_decision_choice",
    })?;

    let root_unit = ApprovedOrgUnit {
        decision_row_id: format!("root:{}", input.root_organization_id),
        candidate_id: String::new(),
        organization_id: input.root_organization_id,
        parent_organization_id: None,
        organization_name: root.organization_name,
        depth: 0,
        ordinal: 0,
        ownership_percent: None,
        aliases: Vec::new(),
        domains: Vec::new(),
        approval_source: serde_json::json!({
            "kind": "subsidiary_scope_choice",
            "tool_call_id": choice_row.id,
        }),
    };

    let (mode, proposal_tool_call_id, review_tool_call_id, mut units, review_rows) = match choice {
        SubsidiaryChoice::RootOnly => (
            ScopeDecisionMode::RootOnly,
            None,
            None,
            vec![root_unit],
            Vec::new(),
        ),
        SubsidiaryChoice::Included => {
            let choice_index = lifecycle
                .iter()
                .position(|row| row.id == choice_row.id)
                .ok_or(ScopeDecisionError::IdentityMismatch {
                    code: "scope_decision_row_mismatch",
                })?;
            let proposal = lifecycle
                .iter()
                .skip(choice_index + 1)
                .find(|row| successful_proposal(row, input.root_organization_id))
                .ok_or(ScopeDecisionError::Missing {
                    entity: "scope_candidate_proposal",
                })?;
            let proposal_index = lifecycle
                .iter()
                .position(|row| row.id == proposal.id)
                .ok_or(ScopeDecisionError::IdentityMismatch {
                    code: "scope_decision_row_mismatch",
                })?;
            let review = lifecycle
                .iter()
                .skip(proposal_index + 1)
                .find_map(|row| {
                    parse_unit_review(row, input.root_organization_id).map(|rows| (row, rows))
                })
                .ok_or(ScopeDecisionError::Missing {
                    entity: "scope_unit_review",
                })?;
            let review_index = lifecycle
                .iter()
                .position(|row| row.id == review.0.id)
                .ok_or(ScopeDecisionError::IdentityMismatch {
                    code: "scope_decision_row_mismatch",
                })?;
            let mappings = create_mappings(&lifecycle[review_index + 1..])?;
            let candidates = candidate_map(&root.intel);
            let mut all_reused = true;
            let mut approved = vec![root_unit];
            let mut seen_review_rows = HashSet::new();
            let mut seen_organizations = HashSet::from([input.root_organization_id]);
            for row in review.1.iter().filter(|row| row.included) {
                if row.review_row_id.is_empty() || !seen_review_rows.insert(&row.review_row_id) {
                    return Err(ScopeDecisionError::IdentityMismatch {
                        code: "scope_decision_row_mismatch",
                    });
                }
                let candidate = if row.candidate_id.is_empty() {
                    None
                } else if let Some(existing_id) = row
                    .candidate_id
                    .strip_prefix("existing-org:")
                    .and_then(|value| value.parse::<Uuid>().ok())
                {
                    if row.organization_id != Some(existing_id) {
                        return Err(ScopeDecisionError::IdentityMismatch {
                            code: "scope_decision_row_mismatch",
                        });
                    }
                    None
                } else {
                    Some(candidates.get(row.candidate_id.as_str()).ok_or(
                        ScopeDecisionError::IdentityMismatch {
                            code: "scope_decision_row_mismatch",
                        },
                    )?)
                };
                if let (Some(candidate), Some(review_org)) = (candidate, row.organization_id) {
                    let candidate_org = candidate
                        .get("organizationId")
                        .or_else(|| candidate.get("organization_id"))
                        .and_then(Value::as_str)
                        .and_then(|value| value.parse::<Uuid>().ok());
                    if candidate_org.is_some() && candidate_org != Some(review_org) {
                        return Err(ScopeDecisionError::IdentityMismatch {
                            code: "scope_decision_row_mismatch",
                        });
                    }
                }

                let organization_id = match row.organization_id {
                    Some(id) => id,
                    None => {
                        all_reused = false;
                        mappings
                            .get(&(row.review_row_id.clone(), row.candidate_id.clone()))
                            .copied()
                            .ok_or(ScopeDecisionError::IdentityMismatch {
                                code: "scope_decision_row_mismatch",
                            })?
                    }
                };
                if !seen_organizations.insert(organization_id) {
                    return Err(ScopeDecisionError::IdentityMismatch {
                        code: "scope_decision_row_mismatch",
                    });
                }
                let organization = lock_descendant(
                    connection,
                    input.root_organization_id,
                    organization_id,
                    input.project_scope_id,
                )
                .await?
                .ok_or(ScopeDecisionError::IdentityMismatch {
                    code: "scope_decision_row_mismatch",
                })?;
                let ownership_percent =
                    normalize_ownership_percent(row.ownership_percent.as_deref().or_else(|| {
                        candidate.and_then(|candidate| {
                            candidate
                                .get("ownershipPercent")
                                .or_else(|| candidate.get("ownership_percent"))
                                .and_then(Value::as_str)
                        })
                    }))?;
                approved.push(ApprovedOrgUnit {
                    decision_row_id: row.review_row_id.clone(),
                    candidate_id: row.candidate_id.clone(),
                    organization_id: organization.id,
                    parent_organization_id: organization.parent_id,
                    organization_name: organization.name,
                    depth: organization.depth,
                    ordinal: approved.len() as i32,
                    ownership_percent,
                    aliases: row.aliases.clone(),
                    domains: row.domains.clone(),
                    approval_source: serde_json::json!({
                        "kind": "unit_review",
                        "review_tool_call_id": review.0.id,
                        "review_name": row.name,
                    }),
                });
            }
            let mode = if approved.len() > 1 && all_reused {
                ScopeDecisionMode::ReuseReconfirmed
            } else {
                ScopeDecisionMode::Included
            };
            (
                mode,
                Some(proposal.id),
                Some(review.0.id),
                approved,
                review.1.iter().map(|row| row.raw.clone()).collect(),
            )
        }
    };
    units.sort_by_key(|unit| (unit.depth, unit.ordinal, unit.organization_id));
    for (ordinal, unit) in units.iter_mut().enumerate() {
        unit.ordinal = ordinal as i32;
    }
    let decision_rows = serde_json::json!({
        "schema_version": 1,
        "review_rows": review_rows,
        "approved_units": units,
    });
    let hash_payload = serde_json::json!({
        "schema_version": 1,
        "operation_id": input.operation_id,
        "project_scope_id": input.project_scope_id,
        "stage_execution_id": input.stage_execution_id,
        "root_organization_id": input.root_organization_id,
        "mode": mode.as_str(),
        "choice_tool_call_id": choice_row.id,
        "proposal_tool_call_id": proposal_tool_call_id,
        "review_tool_call_id": review_tool_call_id,
        "decision_rows": decision_rows,
    });
    let decision_hash = sha256_json(&hash_payload);
    Ok(ApprovedOrgScopeDecision {
        id: Uuid::new_v4(),
        operation_id: input.operation_id,
        project_scope_id: input.project_scope_id,
        stage_execution_id: input.stage_execution_id,
        root_organization_id: input.root_organization_id,
        mode,
        units,
        choice_tool_call_id: Some(choice_row.id),
        proposal_tool_call_id,
        review_tool_call_id,
        decision_rows,
        decision_hash,
    })
}

pub async fn insert_with_connection(
    connection: &mut PgConnection,
    decision: &ApprovedOrgScopeDecision,
) -> Result<OperationScopeDecisionRow, ScopeDecisionError> {
    Ok(sqlx::query_as::<_, OperationScopeDecisionRow>(
        r#"INSERT INTO operation_scope_decisions
           (id, operation_id, project_scope_id, stage_execution_id,
            root_organization_id, mode, choice_tool_call_id,
            proposal_tool_call_id, review_tool_call_id, decision_rows,
            decision_hash)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           RETURNING *"#,
    )
    .bind(decision.id)
    .bind(decision.operation_id)
    .bind(decision.project_scope_id)
    .bind(decision.stage_execution_id)
    .bind(decision.root_organization_id)
    .bind(decision.mode.as_str())
    .bind(decision.choice_tool_call_id)
    .bind(decision.proposal_tool_call_id)
    .bind(decision.review_tool_call_id)
    .bind(&decision.decision_rows)
    .bind(&decision.decision_hash)
    .fetch_one(connection)
    .await?)
}

pub async fn load_for_execution_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> Result<Option<OperationScopeDecisionRow>, ScopeDecisionError> {
    Ok(sqlx::query_as::<_, OperationScopeDecisionRow>(
        r#"SELECT id, operation_id, project_scope_id, stage_execution_id,
                  root_organization_id, mode, choice_tool_call_id,
                  proposal_tool_call_id, review_tool_call_id, decision_rows,
                  decision_hash, created_at
             FROM operation_scope_decisions
            WHERE operation_id=$1 AND stage_execution_id=$2"#,
    )
    .bind(operation_id)
    .bind(stage_execution_id)
    .fetch_optional(connection)
    .await?)
}

fn latest_subsidiary_choice<'a>(
    lifecycle: &'a [ExactScopingLifecycleRow],
    root_organization_id: Uuid,
    root_organization_name: Option<&str>,
) -> Option<(&'a ExactScopingLifecycleRow, SubsidiaryChoice)> {
    lifecycle
        .iter()
        .filter_map(|row| {
            parse_subsidiary_choice(row, root_organization_id, root_organization_name)
                .map(|choice| (row, choice))
        })
        .next_back()
}

fn parse_subsidiary_choice(
    row: &ExactScopingLifecycleRow,
    root_organization_id: Uuid,
    root_organization_name: Option<&str>,
) -> Option<SubsidiaryChoice> {
    if row.name != "ask_human" {
        return None;
    }
    match tool_calls::subsidiary_scope_decision(
        &row.args,
        row.result.as_deref(),
        root_organization_id,
        root_organization_name,
    )? {
        true => Some(SubsidiaryChoice::RootOnly),
        false => Some(SubsidiaryChoice::Included),
    }
}

fn successful_human_result(result: Option<&str>) -> Option<String> {
    let result = serde_json::from_str::<Value>(result?).ok()?;
    if result.get("error").is_some() || result.get("skipped").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    result.get("response")?.as_str().map(str::to_string)
}

fn successful_proposal(row: &ExactScopingLifecycleRow, root_organization_id: Uuid) -> bool {
    if row.name != "manage_organizations"
        || row.args.get("action").and_then(Value::as_str) != Some("propose_candidates")
        || row
            .args
            .get("organization_id")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<Uuid>().ok())
            != Some(root_organization_id)
    {
        return false;
    }
    let Some(result) = row
        .result
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
    else {
        return false;
    };
    result.get("error").is_none()
        && result.get("action").and_then(Value::as_str) == Some("propose_candidates")
        && result
            .get("organization_id")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<Uuid>().ok())
            == Some(root_organization_id)
        && result.get("recorded").and_then(Value::as_i64).is_some()
}

fn parse_unit_review(
    row: &ExactScopingLifecycleRow,
    root_organization_id: Uuid,
) -> Option<Vec<ParsedReviewRow>> {
    if row.name != "ask_human" || row.args.get("input_type")?.as_str()? != "unit_review" {
        return None;
    }
    let context = serde_json::from_str::<Value>(row.args.get("context")?.as_str()?).ok()?;
    if context
        .get("organization_id")?
        .as_str()?
        .parse::<Uuid>()
        .ok()?
        != root_organization_id
    {
        return None;
    }
    let response = successful_human_result(row.result.as_deref())?;
    let response = serde_json::from_str::<Value>(&response).ok()?;
    let rows = response.get("rows")?.as_array()?;
    rows.iter().map(parse_review_row).collect()
}

fn parse_review_row(value: &Value) -> Option<ParsedReviewRow> {
    let review_row_id = value.get("reviewRowId")?.as_str()?.trim().to_string();
    let candidate_id = value
        .get("candidateId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let organization_id = value
        .get("organizationId")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok());
    let name = value.get("name")?.as_str()?.trim().to_string();
    if review_row_id.is_empty() || name.is_empty() {
        return None;
    }
    Some(ParsedReviewRow {
        review_row_id,
        candidate_id,
        organization_id,
        name,
        aliases: string_array(value.get("aliases")),
        domains: string_array(value.get("domains")),
        ownership_percent: value
            .get("ownershipPercent")
            .and_then(Value::as_str)
            .map(str::to_string),
        included: value.get("included").and_then(Value::as_bool)?,
        raw: value.clone(),
    })
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn create_mappings(
    lifecycle: &[ExactScopingLifecycleRow],
) -> Result<HashMap<(String, String), Uuid>, ScopeDecisionError> {
    let mut mappings = HashMap::new();
    for row in lifecycle {
        if row.name != "manage_organizations"
            || row.args.get("action").and_then(Value::as_str) != Some("create_batch")
        {
            continue;
        }
        let Some(result) = row
            .result
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .filter(|result| result.get("error").is_none())
        else {
            continue;
        };
        for item in ["created", "existing"]
            .into_iter()
            .filter_map(|key| result.get(key).and_then(Value::as_array))
            .flatten()
        {
            let Some(review_row_id) = item.get("review_row_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(candidate_id) = item.get("candidate_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(organization_id) = item
                .get("organization_id")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<Uuid>().ok())
            else {
                continue;
            };
            let key = (review_row_id.to_string(), candidate_id.to_string());
            if mappings.insert(key, organization_id).is_some() {
                return Err(ScopeDecisionError::Conflict {
                    code: "scope_create_mapping_ambiguous",
                });
            }
        }
    }
    Ok(mappings)
}

fn candidate_map(intel: &Value) -> HashMap<&str, &Value> {
    intel
        .pointer("/engagement/candidates/organizations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|candidate| Some((candidate.get("id")?.as_str()?, candidate)))
        .collect()
}

async fn lock_descendant(
    connection: &mut PgConnection,
    root_organization_id: Uuid,
    organization_id: Uuid,
    project_scope_id: Uuid,
) -> Result<Option<DescendantOrganization>, ScopeDecisionError> {
    let descendant = sqlx::query_as::<_, DescendantOrganization>(
        r#"WITH RECURSIVE tree AS (
               SELECT organization.id, organization.name, organization.parent_id, 0 AS depth
                 FROM organizations AS organization
                 JOIN project_scopes AS project
                   ON project.project_scope_id = $3
                  AND project.retired_at IS NULL
                  AND organization.project_path = project.canonical_project_path
                WHERE organization.id = $1
               UNION ALL
               SELECT child.id, child.name, child.parent_id, tree.depth + 1
                 FROM organizations AS child
                 JOIN tree ON child.parent_id = tree.id
           )
           SELECT id, name, parent_id, depth
             FROM tree
            WHERE id = $2 AND depth > 0"#,
    )
    .bind(root_organization_id)
    .bind(organization_id)
    .bind(project_scope_id)
    .fetch_optional(&mut *connection)
    .await?;
    if descendant.is_some() {
        // The recursive CTE proves ancestry, but PostgreSQL does not allow
        // row-lock clauses over recursive queries. Lock the proven live row in
        // the same transaction so its canonical name/parent cannot change
        // between decision derivation and immutable snapshot insertion.
        sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR SHARE")
            .bind(organization_id)
            .fetch_one(connection)
            .await?;
    }
    Ok(descendant)
}

fn normalize_ownership_percent(value: Option<&str>) -> Result<Option<String>, ScopeDecisionError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if !value
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.')
        || value.matches('.').count() > 1
    {
        return Err(ScopeDecisionError::IdentityMismatch {
            code: "scope_decision_row_mismatch",
        });
    }
    let numeric = value
        .parse::<f64>()
        .map_err(|_| ScopeDecisionError::IdentityMismatch {
            code: "scope_decision_row_mismatch",
        })?;
    if !(0.0..=100.0).contains(&numeric) {
        return Err(ScopeDecisionError::IdentityMismatch {
            code: "scope_decision_row_mismatch",
        });
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    Ok(Some(if fraction.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    }))
}

pub(crate) fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize JSON string"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize JSON object key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

pub(crate) fn sha256_json(value: &Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_scope_decision_binds_exact_execution() {
        assert_eq!(TABLE_NAME, "operation_scope_decisions");
        assert!(OPERATION_EXECUTION_UNIQUE_SQL.contains("operation_id, stage_execution_id"));
        assert!(STAGE_EXECUTION_OWNER_FK_SQL.contains("stage_runs(id, operation_id)"));
        assert_eq!(ScopeDecisionMode::CliFlags.as_str(), "cli_flags");
        assert!(ScopeDecisionMode::ALL
            .iter()
            .all(|mode| MODE_CHECK_SQL.contains(mode.as_str())));
    }

    #[test]
    fn exact_scope_freeze_reuses_canonical_typed_subsidiary_choice_parser() {
        let root_organization_id = Uuid::new_v4();
        let row = |response: &str| ExactScopingLifecycleRow {
            id: Uuid::new_v4(),
            call_id: Uuid::new_v4().to_string(),
            session_id: Uuid::new_v4(),
            name: "ask_human".to_string(),
            args: serde_json::json!({
                "input_type": "choice",
                "context": serde_json::json!({
                    "decision": "subsidiary_scope",
                    "organization_id": root_organization_id,
                }).to_string(),
                "question": "Choose subsidiary scope",
                "options": ["root_only", "include_51", "include_100"],
            }),
            result: Some(
                serde_json::json!({
                    "response": response,
                    "skipped": false,
                })
                .to_string(),
            ),
            created_at: Utc::now(),
        };

        assert_eq!(
            parse_subsidiary_choice(&row("root_only"), root_organization_id, None),
            Some(SubsidiaryChoice::RootOnly)
        );
        assert_eq!(
            parse_subsidiary_choice(&row("include_100"), root_organization_id, None),
            Some(SubsidiaryChoice::Included)
        );

        let legacy = ExactScopingLifecycleRow {
            args: serde_json::json!({
                "input_type": "choice",
                "context": "Confirmed enterprise: Golish Fixture Corporation",
                "question": "Confirm subsidiary scope for Golish Fixture Corporation",
                "options": [
                    "Root-only: only Golish Fixture Corporation is in scope",
                    "Include subsidiaries",
                ],
            }),
            result: Some(
                serde_json::json!({
                    "response": "Root-only: only Golish Fixture Corporation is in scope",
                    "skipped": false,
                })
                .to_string(),
            ),
            ..row("root_only")
        };
        assert_eq!(
            parse_subsidiary_choice(
                &legacy,
                root_organization_id,
                Some("Golish Fixture Corporation"),
            ),
            Some(SubsidiaryChoice::RootOnly),
            "an exact-operation legacy request is accepted only when it names the exact root"
        );
    }
}
