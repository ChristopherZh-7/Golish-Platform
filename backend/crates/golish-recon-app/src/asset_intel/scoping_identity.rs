//! Host-owned Scoping company resolver and immutable Company Identity freeze.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::{AssetIntelLookupResult, LookupCompanyMatch};

pub(crate) async fn freeze_company_lookup_result(
    pool: &PgPool,
    workspace: &Path,
    keyword: &str,
    selected_candidate_id: Option<&str>,
    result: &AssetIntelLookupResult,
) -> anyhow::Result<Value> {
    let tool = golish_core::current_agent_tool_context()
        .ok_or_else(|| anyhow::anyhow!("SCOPING_TRUSTED_TOOL_CONTEXT_MISSING"))?;
    let operation_id = tool
        .operation_id
        .ok_or_else(|| anyhow::anyhow!("SCOPING_OPERATION_CONTEXT_MISSING"))?;
    let stage_execution_id = tool
        .stage_execution_id
        .ok_or_else(|| anyhow::anyhow!("SCOPING_STAGE_EXECUTION_CONTEXT_MISSING"))?;
    anyhow::ensure!(
        tool.tool_name == "recon_lookup_company",
        "SCOPING_TOOL_CONTEXT_MISMATCH"
    );
    let operation_project_scope: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT project_scope_id FROM operation_state
            WHERE operation_id=$1 AND current_stage='scoping' AND superseded_by IS NULL"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    anyhow::ensure!(
        operation_project_scope.is_some(),
        "SCOPING_ACTIVE_OPERATION_MISSING"
    );
    if let Some(receipt) =
        golish_db::repo::scoping_company_identities::get_confirmed_for_operation(pool, operation_id)
            .await?
    {
        anyhow::ensure!(
            receipt.stage_execution_id == stage_execution_id && receipt.organization_id.is_some(),
            "SCOPING_CONFIRMED_COMPANY_IDENTITY_CONTEXT_MISMATCH"
        );
        return Ok(json!({
            "receipt_id": receipt.id,
            "resolution_status": "confirmed",
            "confirmation_method": receipt.confirmation_method,
            "organization_id": receipt.organization_id,
            "canonical_legal_name": receipt.canonical_legal_name,
            "identity_sha256": receipt.identity_sha256,
            "scope_policy_sha256": receipt.scope_policy_sha256,
            "candidate_set": [],
            "requires_human_choice": false,
            "reused": true,
        }));
    }
    let prior_attempt: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(resolution_attempt),-1) FROM scoping_company_identity_receipts WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await?;
    let resolution_attempt = prior_attempt.saturating_add(1);
    let prior_receipt_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM scoping_company_identity_receipts
            WHERE operation_id=$1 ORDER BY resolution_attempt DESC LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    let candidates = result
        .matches
        .iter()
        .map(|candidate| {
            let candidate_id = candidate_id(candidate);
            json!({
                "candidate_id": candidate_id,
                "provider_id": candidate.provider_id,
                "name": candidate.name,
                "credit_code": candidate.credit_code,
                "industry": candidate.industry,
                "legal_representative": candidate.legal_representative,
                "address": candidate.address,
                "registered_at": candidate.registered_at,
                "confidence": candidate.confidence,
                "evidence": redact(&candidate.evidence),
            })
        })
        .collect::<Vec<_>>();
    let selected = if let Some(selected_candidate_id) = selected_candidate_id {
        result
            .matches
            .iter()
            .find(|candidate| candidate_id(candidate) == selected_candidate_id)
            .cloned()
    } else {
        unique_host_confirmation(keyword, &result.matches)
    };
    let ambiguous = selected.is_none() && !result.matches.is_empty();
    let status = if selected.is_some() {
        "confirmed"
    } else if ambiguous {
        "needs_human"
    } else {
        "unresolved"
    };
    let confirmation_method = if selected.is_some() {
        if selected_candidate_id.is_some() {
            "human_selected"
        } else {
            "provider_corroborated"
        }
    } else {
        "none"
    };
    let session_id =
        golish_core::current_agent_session().unwrap_or_else(|| operation_id.to_string());
    let selected_payload = selected.as_ref().map(|candidate| {
        json!({
            "candidate_id": candidate_id(candidate),
            "provider_id": candidate.provider_id,
            "name": candidate.name,
            "credit_code": candidate.credit_code,
            "industry": candidate.industry,
            "legal_representative": candidate.legal_representative,
            "address": candidate.address,
            "registered_at": candidate.registered_at,
            "confidence": candidate.confidence,
            "evidence": redact(&candidate.evidence),
        })
    });
    let evidence = golish_db::repo::audit::log_evidence(
        pool,
        "scoping_company_resolution",
        "scoping",
        "scoping.company_resolution.v1",
        workspace.to_str(),
        "harness",
        None,
        Some(&session_id),
        Some("recon_lookup_company"),
        &json!({
            "kind": "scoping.company_resolution",
            "operation_id": operation_id,
            "keyword_sha256": sha256_hex(keyword.as_bytes()),
            "resolution_status": status,
            "selected": selected_payload,
            "candidate_set_sha256": prefixed_sha256(&json!(candidates)),
            "provider_status": result.provider_status,
        }),
        Some(operation_id),
        None,
        Some(keyword),
        Some(status),
    )
    .await?;

    let (organization_id, legal_name, registration_identifiers, identity_payload, scope_policy) =
        if let Some(candidate) = selected.as_ref() {
            let organization_id =
                find_or_create_organization(pool, workspace, candidate.name.trim()).await?;
            let trusted_roots = collect_domain_roots(&candidate.evidence);
            (
                Some(organization_id),
                Some(candidate.name.trim().to_string()),
                json!({
                    "credit_code": candidate.credit_code,
                    "provider_id": candidate.provider_id,
                }),
                json!({
                    "canonical_legal_name": candidate.name,
                    "credit_code": candidate.credit_code,
                    "industry": candidate.industry,
                    "legal_representative": candidate.legal_representative,
                    "address": candidate.address,
                    "registered_at": candidate.registered_at,
                    "trusted_roots": trusted_roots,
                }),
                json!({
                    "owned_only": true,
                    "reachable_only": true,
                    "trusted_roots": trusted_roots,
                    "third_party_default": "exclude",
                }),
            )
        } else {
            (
                None,
                None,
                json!({}),
                json!({
                    "keyword_sha256": sha256_hex(keyword.as_bytes()),
                    "candidates": candidates,
                }),
                json!({"owned_only": true, "reachable_only": true}),
            )
        };
    let identity_sha256 = prefixed_sha256(&identity_payload);
    let scope_policy_sha256 = prefixed_sha256(&scope_policy);
    let receipt_id = Uuid::new_v5(
        &operation_id,
        format!("company-identity:{resolution_attempt}:{identity_sha256}").as_bytes(),
    );
    let receipt = golish_db::repo::scoping_company_identities::ScopingCompanyIdentityReceiptRow {
        id: receipt_id,
        operation_id,
        stage_execution_id,
        resolution_attempt,
        supersedes_receipt_id: prior_receipt_id,
        organization_id,
        subject_hint: keyword.to_string(),
        canonical_legal_name: legal_name,
        aliases: json!([]),
        brands: json!([]),
        registration_identifiers,
        disambiguation_fields: json!({
            "candidate_count": result.matches.len(),
            "candidate_set_sha256": prefixed_sha256(&json!(candidates)),
        }),
        confirmation_method: confirmation_method.to_string(),
        resolution_status: status.to_string(),
        scope_policy: scope_policy.clone(),
        source_receipt_refs: if selected.is_some() {
            json!([format!("audit:{}", evidence.id)])
        } else {
            json!([])
        },
        artifact_refs: json!([format!("audit:{}", evidence.id)]),
        evidence_refs: json!([format!("audit:{}", evidence.id)]),
        identity_payload,
        identity_sha256: identity_sha256.clone(),
        scope_policy_sha256: scope_policy_sha256.clone(),
    };
    golish_db::repo::scoping_company_identities::insert_terminal_receipt(pool, &receipt).await?;
    Ok(json!({
        "receipt_id": receipt_id,
        "resolution_status": status,
        "confirmation_method": confirmation_method,
        "organization_id": organization_id,
        "canonical_legal_name": receipt.canonical_legal_name,
        "identity_sha256": identity_sha256,
        "scope_policy_sha256": scope_policy_sha256,
        "candidate_set": candidates,
        "requires_human_choice": status == "needs_human",
    }))
}

fn unique_host_confirmation(
    keyword: &str,
    matches: &[LookupCompanyMatch],
) -> Option<LookupCompanyMatch> {
    let strong = matches
        .iter()
        .filter(|candidate| candidate.confidence >= 0.8)
        .collect::<Vec<_>>();
    if strong.len() == 1 {
        return Some(strong[0].clone());
    }
    let exact = matches
        .iter()
        .filter(|candidate| candidate.name.trim().eq_ignore_ascii_case(keyword.trim()))
        .collect::<Vec<_>>();
    (exact.len() == 1).then(|| exact[0].clone())
}

fn candidate_id(candidate: &LookupCompanyMatch) -> String {
    format!(
        "company-candidate:v1:{}:{}",
        candidate.provider_id,
        sha256_hex(
            format!(
                "{}\u{1f}{}",
                candidate.name.trim().to_ascii_lowercase(),
                candidate.credit_code.as_deref().unwrap_or_default()
            )
            .as_bytes()
        )
    )
}

async fn find_or_create_organization(
    pool: &PgPool,
    workspace: &Path,
    legal_name: &str,
) -> anyhow::Result<Uuid> {
    let project_path = workspace
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("SCOPING_WORKSPACE_PATH_INVALID"))?;
    if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM organizations
            WHERE project_path=$1 AND lower(btrim(name))=lower(btrim($2))
            ORDER BY created_at LIMIT 1"#,
    )
    .bind(project_path)
    .bind(legal_name)
    .fetch_optional(pool)
    .await?
    {
        return Ok(id);
    }
    Ok(golish_db::repo::organizations::create(
        pool,
        project_path,
        legal_name,
        None,
        "Confirmed by immutable Scoping Company Identity receipt",
        "scoping_company_identity",
    )
    .await?
    .id)
}

fn collect_domain_roots(value: &Value) -> Vec<String> {
    fn visit(key: Option<&str>, value: &Value, roots: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (child_key, child) in map {
                    visit(Some(child_key), child, roots);
                }
            }
            Value::Array(values) => {
                for child in values {
                    visit(key, child, roots);
                }
            }
            Value::String(text)
                if key.is_some_and(|key| {
                    key.contains("domain") || key == "host" || key == "hostname" || key == "website"
                }) =>
            {
                let host = text
                    .trim()
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                if !host.is_empty() && host.contains('.') {
                    roots.insert(host);
                }
            }
            _ => {}
        }
    }
    let mut roots = BTreeSet::new();
    visit(None, value, &mut roots);
    roots.into_iter().collect()
}

fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    (
                        key.clone(),
                        if normalized.contains("token")
                            || normalized.contains("secret")
                            || normalized.contains("password")
                            || normalized.ends_with("_key")
                        {
                            Value::String("[REDACTED]".to_string())
                        } else {
                            redact(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact).collect()),
        other => other.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn prefixed_sha256(value: &Value) -> String {
    format!(
        "sha256:{}",
        sha256_hex(&serde_json::to_vec(value).unwrap_or_default())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn company(name: &str, confidence: f64) -> LookupCompanyMatch {
        LookupCompanyMatch {
            provider_id: "enterprise".to_string(),
            name: name.to_string(),
            credit_code: Some("9133".to_string()),
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence,
            evidence: json!({}),
        }
    }

    #[test]
    fn host_only_confirms_one_strong_or_one_exact_candidate() {
        assert_eq!(
            unique_host_confirmation("默安科技", &[company("杭州默安科技有限公司", 0.95)])
                .unwrap()
                .name,
            "杭州默安科技有限公司"
        );
        assert!(unique_host_confirmation(
            "默安科技",
            &[
                company("杭州默安科技有限公司", 0.95),
                company("默安科技集团", 0.9)
            ]
        )
        .is_none());
    }

    #[test]
    fn domain_roots_are_extracted_only_from_domain_shaped_fields() {
        assert_eq!(
            collect_domain_roots(&json!({
                "website": "https://moresec.cn/about",
                "description": "ignore.example"
            })),
            vec!["moresec.cn"]
        );
    }
}
