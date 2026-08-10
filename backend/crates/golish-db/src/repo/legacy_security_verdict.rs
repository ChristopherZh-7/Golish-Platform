//! Grandfathered, append-only security-verdict authority for legacy Attempts.
//!
//! The adapter never invents a Campaign, oracle or coverage receipt.  It locks
//! and hashes the retained Candidate/Attempt/evidence/finding lineage itself,
//! then persists one deterministic receipt plus an exact ordered source set.

use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

const LEGACY_COVERAGE_UNAVAILABLE: &str = "legacy_coverage_unavailable";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn tagged_hash(value: &Value) -> String {
    format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(value)
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SealLegacyAttemptAuthorityV1 {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub attempt_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub adapter_version: String,
    pub adapter_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyAttemptAuthorityReceiptV1 {
    pub receipt_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub terminal_status: String,
    pub source_record_hash: String,
    pub source_member_count: i64,
    pub source_membership_hash: String,
    pub evidence_membership_hash: String,
    pub finding_id: Option<Uuid>,
    pub refutation_receipt_id: Option<Uuid>,
    pub limitation_membership_hash: String,
    pub adapter_version: String,
    pub adapter_digest: String,
    pub replayed: bool,
}

#[derive(sqlx::FromRow)]
struct LockedAttempt {
    candidate_id: Uuid,
    status: String,
    result_hash: String,
    row_version: i64,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    candidate_plan_hash: String,
    hypothesis_hash: String,
    finding_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct EvidenceRow {
    evidence_id: i64,
    role: String,
    action: String,
    category: String,
    details: String,
}

#[derive(Clone)]
struct SourceMember {
    kind: &'static str,
    reference_id: String,
    reference_hash: String,
    member_hash: String,
}

#[derive(sqlx::FromRow)]
struct PersistedSourceMember {
    ordinal: i32,
    source_kind: String,
    source_ref_id: String,
    source_ref_hash: String,
    member_hash: String,
}

fn source_member(kind: &'static str, reference_id: String, body: Value) -> SourceMember {
    let reference_hash = tagged_hash(&body);
    let member_hash = tagged_hash(&json!({
        "schema":"legacy_attempt_authority_source_member.v1",
        "kind":kind,
        "reference_id":reference_id,
        "reference_hash":reference_hash,
    }));
    SourceMember {
        kind,
        reference_id,
        reference_hash,
        member_hash,
    }
}

fn manifest_hash(members: &[SourceMember]) -> String {
    tagged_hash(&json!({
        "schema":"legacy_attempt_authority_source_set.v1",
        "members":members.iter().enumerate().map(|(ordinal, member)| json!({
            "ordinal":ordinal,
            "kind":member.kind,
            "reference_id":member.reference_id,
            "reference_hash":member.reference_hash,
            "member_hash":member.member_hash,
        })).collect::<Vec<_>>()
    }))
}

fn valid_tagged_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Seal one legacy terminal Attempt from locked database truth.
pub async fn seal_legacy_attempt_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    input: SealLegacyAttemptAuthorityV1,
) -> Result<LegacyAttemptAuthorityReceiptV1> {
    if input.adapter_version.trim().is_empty() || !valid_tagged_hash(&input.adapter_digest) {
        return Err(conflict("LEGACY_ATTEMPT_ADAPTER_IDENTITY_INVALID"));
    }
    let attempt = sqlx::query_as::<_, LockedAttempt>(
        r#"SELECT candidate.candidate_id,attempt.status,attempt.result_hash,
                  attempt.row_version,attempt.target_type_at_time,
                  attempt.target_value_at_time,attempt.target_identity_hash,
                  attempt.candidate_plan_hash,candidate.hypothesis_hash,
                  (SELECT lineage.finding_id
                     FROM finding_lineage lineage
                    WHERE lineage.candidate_attempt_id=attempt.id
                      AND lineage.candidate_id=attempt.candidate_id
                      AND lineage.operation_id=attempt.operation_id
                      AND lineage.organization_id=attempt.organization_id) AS finding_id
             FROM candidate_attempts attempt
             JOIN attack_candidates candidate
               ON candidate.candidate_id=attempt.candidate_id
              AND candidate.operation_uuid=attempt.operation_id
              AND candidate.organization_id=attempt.organization_id
              AND candidate.hypothesis_revision_id=$5
             JOIN operation_state operation
               ON operation.operation_id=attempt.operation_id
              AND operation.project_scope_id=$2
            WHERE attempt.operation_id=$1 AND attempt.organization_id=$3
              AND attempt.id=$4
            FOR SHARE OF attempt,candidate,operation"#,
    )
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.organization_id)
    .bind(input.attempt_id)
    .bind(input.hypothesis_revision_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("LEGACY_ATTEMPT_AUTHORITY_NOT_FOUND"))?;
    if !matches!(attempt.status.as_str(), "verified" | "refuted") {
        return Err(conflict("LEGACY_ATTEMPT_AUTHORITY_NOT_TERMINAL"));
    }
    if (attempt.status == "verified") != attempt.finding_id.is_some() {
        return Err(conflict("LEGACY_ATTEMPT_FINDING_LINEAGE_INCOMPLETE"));
    }

    let evidence = sqlx::query_as::<_, EvidenceRow>(
        r#"SELECT link.evidence_id,link.role,audit.action,audit.category,audit.details
             FROM candidate_attempt_evidence link
             JOIN audit_log audit ON audit.id=link.evidence_id
            WHERE link.attempt_id=$1
            ORDER BY link.evidence_id,link.role
            FOR SHARE OF link,audit"#,
    )
    .bind(input.attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if evidence.is_empty() {
        return Err(conflict("LEGACY_ATTEMPT_EVIDENCE_EMPTY"));
    }

    let candidate_body = json!({
        "candidate_id":attempt.candidate_id,
        "hypothesis_revision_id":input.hypothesis_revision_id,
        "hypothesis_hash":attempt.hypothesis_hash,
        "target_type_at_time":attempt.target_type_at_time,
        "target_value_at_time":attempt.target_value_at_time,
        "target_identity_hash":attempt.target_identity_hash,
        "candidate_plan_hash":attempt.candidate_plan_hash,
    });
    let terminal_body = json!({
        "attempt_id":input.attempt_id,
        "status":attempt.status,
        "result_hash":attempt.result_hash,
        "row_version":attempt.row_version,
    });
    let mut members = vec![
        source_member(
            "candidate_snapshot",
            attempt.candidate_id.to_string(),
            candidate_body.clone(),
        ),
        source_member(
            "attempt_terminal",
            input.attempt_id.to_string(),
            terminal_body.clone(),
        ),
    ];
    for row in &evidence {
        members.push(source_member(
            "evidence",
            row.evidence_id.to_string(),
            json!({
                "evidence_id":row.evidence_id,"role":row.role,
                "action":row.action,"category":row.category,"details":row.details,
            }),
        ));
    }
    let refutation_receipt_id = (attempt.status == "refuted").then(|| {
        Uuid::new_v5(
            &input.attempt_id,
            format!("legacy-refutation-authority.v1:{}", attempt.result_hash).as_bytes(),
        )
    });
    if let Some(finding_id) = attempt.finding_id {
        members.push(source_member(
            "finding_lineage",
            finding_id.to_string(),
            json!({"finding_id":finding_id,"attempt_id":input.attempt_id}),
        ));
    } else if let Some(refutation_id) = refutation_receipt_id {
        members.push(source_member(
            "refutation_lineage",
            refutation_id.to_string(),
            json!({
                "refutation_receipt_id":refutation_id,
                "attempt_id":input.attempt_id,
                "result_hash":attempt.result_hash,
            }),
        ));
    }
    members.push(source_member(
        "limitation",
        LEGACY_COVERAGE_UNAVAILABLE.to_owned(),
        json!({"code":LEGACY_COVERAGE_UNAVAILABLE}),
    ));
    let source_membership_hash = manifest_hash(&members);
    let evidence_members = members
        .iter()
        .filter(|member| member.kind == "evidence")
        .cloned()
        .collect::<Vec<_>>();
    let evidence_membership_hash = manifest_hash(&evidence_members);
    let limitation_membership_hash = tagged_hash(&json!([LEGACY_COVERAGE_UNAVAILABLE]));
    if let Some(refutation_receipt_id) = refutation_receipt_id {
        let candidate_snapshot_hash = tagged_hash(&candidate_body);
        let attempt_terminal_hash = tagged_hash(&terminal_body);
        let refutation_hash = tagged_hash(&json!({
            "schema":"legacy_attempt_refutation_authority.v1",
            "operation_id":input.operation_id,
            "project_scope_id":input.project_scope_id,
            "organization_id":input.organization_id,
            "candidate_id":attempt.candidate_id,
            "attempt_id":input.attempt_id,
            "hypothesis_revision_id":input.hypothesis_revision_id,
            "candidate_snapshot_hash":candidate_snapshot_hash,
            "attempt_terminal_hash":attempt_terminal_hash,
            "evidence_membership_hash":evidence_membership_hash,
            "adapter_version":input.adapter_version,
            "adapter_digest":input.adapter_digest,
        }));
        let inserted = sqlx::query(
            r#"INSERT INTO legacy_attempt_refutation_receipts(
                   receipt_id,operation_id,project_scope_id,organization_id,candidate_id,
                   attempt_id,hypothesis_revision_id,candidate_snapshot_hash,
                   attempt_terminal_hash,evidence_membership_hash,refutation_hash,
                   adapter_version,adapter_digest
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
               ON CONFLICT(operation_id,attempt_id,adapter_version) DO NOTHING"#,
        )
        .bind(refutation_receipt_id)
        .bind(input.operation_id)
        .bind(input.project_scope_id)
        .bind(input.organization_id)
        .bind(attempt.candidate_id)
        .bind(input.attempt_id)
        .bind(input.hypothesis_revision_id)
        .bind(&candidate_snapshot_hash)
        .bind(&attempt_terminal_hash)
        .bind(&evidence_membership_hash)
        .bind(&refutation_hash)
        .bind(&input.adapter_version)
        .bind(&input.adapter_digest)
        .execute(&mut **tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let exact: bool = sqlx::query_scalar(
                r#"SELECT receipt_id=$2 AND project_scope_id=$3 AND organization_id=$4
                          AND candidate_id=$5 AND hypothesis_revision_id=$6
                          AND candidate_snapshot_hash=$7 AND attempt_terminal_hash=$8
                          AND evidence_membership_hash=$9 AND refutation_hash=$10
                          AND adapter_digest=$11
                     FROM legacy_attempt_refutation_receipts
                    WHERE operation_id=$1 AND attempt_id=$12 AND adapter_version=$13
                    FOR SHARE"#,
            )
            .bind(input.operation_id)
            .bind(refutation_receipt_id)
            .bind(input.project_scope_id)
            .bind(input.organization_id)
            .bind(attempt.candidate_id)
            .bind(input.hypothesis_revision_id)
            .bind(&candidate_snapshot_hash)
            .bind(&attempt_terminal_hash)
            .bind(&evidence_membership_hash)
            .bind(&refutation_hash)
            .bind(&input.adapter_digest)
            .bind(input.attempt_id)
            .bind(&input.adapter_version)
            .fetch_one(&mut **tx)
            .await?;
            if !exact {
                return Err(conflict("LEGACY_REFUTATION_AUTHORITY_REPLAY_DRIFT"));
            }
        }
    }
    let source_record_hash = tagged_hash(&json!({
        "schema":"legacy_attempt_authority.v1",
        "operation_id":input.operation_id,
        "project_scope_id":input.project_scope_id,
        "organization_id":input.organization_id,
        "candidate":candidate_body,
        "attempt":terminal_body,
        "source_membership_hash":source_membership_hash,
        "evidence_membership_hash":evidence_membership_hash,
        "limitation_membership_hash":limitation_membership_hash,
        "adapter_version":input.adapter_version,
        "adapter_digest":input.adapter_digest,
    }));
    let receipt_id = Uuid::new_v5(
        &input.attempt_id,
        format!("legacy-attempt-authority.v1:{}", input.adapter_version).as_bytes(),
    );
    let inserted = sqlx::query(
        r#"INSERT INTO legacy_attempt_authority_receipts(
               receipt_id,operation_id,project_scope_id,organization_id,candidate_id,attempt_id,
               hypothesis_revision_id,terminal_status,source_record_hash,source_member_count,
               source_membership_hash,evidence_membership_hash,finding_id,refutation_receipt_id,
               limitation_membership_hash,adapter_version,adapter_digest
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
           ON CONFLICT(operation_id,attempt_id,adapter_version) DO NOTHING"#,
    )
    .bind(receipt_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.organization_id)
    .bind(attempt.candidate_id)
    .bind(input.attempt_id)
    .bind(input.hypothesis_revision_id)
    .bind(&attempt.status)
    .bind(&source_record_hash)
    .bind(
        i64::try_from(members.len())
            .map_err(|_| conflict("LEGACY_ATTEMPT_SOURCE_SET_TOO_LARGE"))?,
    )
    .bind(&source_membership_hash)
    .bind(&evidence_membership_hash)
    .bind(attempt.finding_id)
    .bind(refutation_receipt_id)
    .bind(&limitation_membership_hash)
    .bind(&input.adapter_version)
    .bind(&input.adapter_digest)
    .execute(&mut **tx)
    .await?;
    let replayed = inserted.rows_affected() == 0;
    if !replayed {
        for (ordinal, member) in members.iter().enumerate() {
            sqlx::query(
                r#"INSERT INTO legacy_attempt_authority_source_members(
                       receipt_id,ordinal,source_kind,source_ref_id,source_ref_hash,member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6)"#,
            )
            .bind(receipt_id)
            .bind(
                i32::try_from(ordinal)
                    .map_err(|_| conflict("LEGACY_ATTEMPT_SOURCE_SET_TOO_LARGE"))?,
            )
            .bind(member.kind)
            .bind(&member.reference_id)
            .bind(&member.reference_hash)
            .bind(&member.member_hash)
            .execute(&mut **tx)
            .await?;
        }
    } else {
        let exact: bool = sqlx::query_scalar(
            r#"SELECT receipt_id=$2 AND project_scope_id=$3 AND organization_id=$4
                      AND candidate_id=$5 AND hypothesis_revision_id=$6
                      AND terminal_status=$7 AND source_record_hash=$8
                      AND source_member_count=$9 AND source_membership_hash=$10
                      AND evidence_membership_hash=$11
                      AND finding_id IS NOT DISTINCT FROM $12
                      AND refutation_receipt_id IS NOT DISTINCT FROM $13
                      AND limitation_membership_hash=$14 AND adapter_digest=$15
                 FROM legacy_attempt_authority_receipts
                WHERE operation_id=$1 AND attempt_id=$16 AND adapter_version=$17
                FOR SHARE"#,
        )
        .bind(input.operation_id)
        .bind(receipt_id)
        .bind(input.project_scope_id)
        .bind(input.organization_id)
        .bind(attempt.candidate_id)
        .bind(input.hypothesis_revision_id)
        .bind(&attempt.status)
        .bind(&source_record_hash)
        .bind(
            i64::try_from(members.len())
                .map_err(|_| conflict("LEGACY_ATTEMPT_SOURCE_SET_TOO_LARGE"))?,
        )
        .bind(&source_membership_hash)
        .bind(&evidence_membership_hash)
        .bind(attempt.finding_id)
        .bind(refutation_receipt_id)
        .bind(&limitation_membership_hash)
        .bind(&input.adapter_digest)
        .bind(input.attempt_id)
        .bind(&input.adapter_version)
        .fetch_one(&mut **tx)
        .await?;
        let persisted_members = sqlx::query_as::<_, PersistedSourceMember>(
            r#"SELECT ordinal,source_kind,source_ref_id,source_ref_hash,member_hash
                 FROM legacy_attempt_authority_source_members
                WHERE receipt_id=$1 ORDER BY ordinal
                FOR SHARE"#,
        )
        .bind(receipt_id)
        .fetch_all(&mut **tx)
        .await?;
        let member_exact = persisted_members.len() == members.len()
            && persisted_members.iter().zip(&members).enumerate().all(
                |(ordinal, (persisted, expected))| {
                    i32::try_from(ordinal).ok() == Some(persisted.ordinal)
                        && persisted.source_kind == expected.kind
                        && persisted.source_ref_id == expected.reference_id
                        && persisted.source_ref_hash == expected.reference_hash
                        && persisted.member_hash == expected.member_hash
                },
            );
        if !exact || !member_exact {
            return Err(conflict("LEGACY_ATTEMPT_AUTHORITY_REPLAY_DRIFT"));
        }
    }

    Ok(LegacyAttemptAuthorityReceiptV1 {
        receipt_id,
        operation_id: input.operation_id,
        project_scope_id: input.project_scope_id,
        organization_id: input.organization_id,
        candidate_id: attempt.candidate_id,
        attempt_id: input.attempt_id,
        hypothesis_revision_id: input.hypothesis_revision_id,
        terminal_status: attempt.status,
        source_record_hash,
        source_member_count: i64::try_from(members.len()).unwrap_or(i64::MAX),
        source_membership_hash,
        evidence_membership_hash,
        finding_id: attempt.finding_id,
        refutation_receipt_id,
        limitation_membership_hash,
        adapter_version: input.adapter_version,
        adapter_digest: input.adapter_digest,
        replayed,
    })
}

#[cfg(test)]
mod tests {
    use super::{tagged_hash, valid_tagged_hash, LEGACY_COVERAGE_UNAVAILABLE};
    use serde_json::json;

    #[test]
    fn legacy_verdict_hashes_are_tagged_and_keep_mandatory_limitation() {
        let digest = tagged_hash(&json!({"limitation":LEGACY_COVERAGE_UNAVAILABLE}));
        assert!(valid_tagged_hash(&digest));
        assert_ne!(digest, tagged_hash(&json!({"limitation":"none"})));
    }
}
