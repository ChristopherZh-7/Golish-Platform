//! Operation-wide seal over every grandfathered legacy Attempt authority.
//!
//! Reports cite the immutable per-Attempt receipts, while this seal proves
//! that the report adapter selected the complete canonical set for the
//! operation and retained the mandatory legacy coverage limitation.

use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn tagged_hash(value: &Value) -> String {
    format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(value)
    )
}

fn valid_tagged_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SealLegacyReportAuthorityV1 {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub adapter_version: String,
    pub adapter_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyReportAuthoritySealV1 {
    pub seal_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub source_member_count: i64,
    pub source_membership_hash: String,
    pub limitation_membership_hash: String,
    pub adapter_version: String,
    pub adapter_digest: String,
    pub seal_hash: String,
    pub replayed: bool,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct AttemptAuthorityRow {
    receipt_id: Uuid,
    organization_id: Uuid,
    attempt_id: Uuid,
    source_record_hash: String,
    limitation_membership_hash: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct PersistedMemberRow {
    ordinal: i32,
    legacy_attempt_authority_receipt_id: Uuid,
    receipt_hash: String,
    member_hash: String,
}

fn member_hash(row: &AttemptAuthorityRow) -> String {
    tagged_hash(&json!({
        "schema":"legacy_report_authority_member.v1",
        "receipt_id":row.receipt_id,
        "organization_id":row.organization_id,
        "attempt_id":row.attempt_id,
        "receipt_hash":row.source_record_hash,
        "limitation_membership_hash":row.limitation_membership_hash,
    }))
}

pub async fn seal_legacy_report_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    input: SealLegacyReportAuthorityV1,
) -> Result<LegacyReportAuthoritySealV1> {
    if input.adapter_version.trim().is_empty() || !valid_tagged_hash(&input.adapter_digest) {
        return Err(conflict("LEGACY_REPORT_ADAPTER_IDENTITY_INVALID"));
    }
    let scope_matches: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM operation_state
                WHERE operation_id=$1 AND project_scope_id=$2
           )"#,
    )
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .fetch_one(&mut **tx)
    .await?;
    if !scope_matches {
        return Err(conflict("LEGACY_REPORT_OPERATION_SCOPE_MISMATCH"));
    }
    let rows = sqlx::query_as::<_, AttemptAuthorityRow>(
        r#"SELECT receipt_id,organization_id,attempt_id,source_record_hash,
                  limitation_membership_hash
             FROM legacy_attempt_authority_receipts
            WHERE operation_id=$1 AND project_scope_id=$2 AND adapter_version=$3
            ORDER BY organization_id,attempt_id,receipt_id
            FOR SHARE"#,
    )
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(&input.adapter_version)
    .fetch_all(&mut **tx)
    .await?;
    if rows.is_empty() {
        return Err(conflict("LEGACY_REPORT_AUTHORITY_EMPTY"));
    }
    let member_hashes = rows.iter().map(member_hash).collect::<Vec<_>>();
    let source_membership_hash = tagged_hash(&json!({
        "schema":"legacy_report_authority_source_set.v1",
        "members":member_hashes,
    }));
    let limitation_membership_hash = tagged_hash(&json!({
        "schema":"legacy_report_authority_limitations.v1",
        "mandatory_code":"legacy_coverage_unavailable",
        "members":rows.iter().map(|row| &row.limitation_membership_hash).collect::<Vec<_>>(),
    }));
    let seal_hash = tagged_hash(&json!({
        "schema":"legacy_report_authority_seal.v1",
        "operation_id":input.operation_id,
        "project_scope_id":input.project_scope_id,
        "source_member_count":rows.len(),
        "source_membership_hash":source_membership_hash,
        "limitation_membership_hash":limitation_membership_hash,
        "adapter_version":input.adapter_version,
        "adapter_digest":input.adapter_digest,
    }));
    let seal_id = Uuid::new_v5(
        &input.operation_id,
        format!("legacy-report-authority.v1:{}", input.adapter_version).as_bytes(),
    );
    let source_member_count =
        i64::try_from(rows.len()).map_err(|_| conflict("LEGACY_REPORT_AUTHORITY_SET_TOO_LARGE"))?;
    let inserted = sqlx::query(
        r#"INSERT INTO legacy_report_authority_seals(
               seal_id,operation_id,project_scope_id,source_member_count,
               source_membership_hash,limitation_membership_hash,adapter_version,
               adapter_digest,seal_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
           ON CONFLICT(operation_id,adapter_version) DO NOTHING"#,
    )
    .bind(seal_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(source_member_count)
    .bind(&source_membership_hash)
    .bind(&limitation_membership_hash)
    .bind(&input.adapter_version)
    .bind(&input.adapter_digest)
    .bind(&seal_hash)
    .execute(&mut **tx)
    .await?;
    let replayed = inserted.rows_affected() == 0;
    if !replayed {
        for (ordinal, (row, member_hash)) in rows.iter().zip(&member_hashes).enumerate() {
            sqlx::query(
                r#"INSERT INTO legacy_report_authority_members(
                       seal_id,operation_id,ordinal,legacy_attempt_authority_receipt_id,
                       receipt_hash,member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6)"#,
            )
            .bind(seal_id)
            .bind(input.operation_id)
            .bind(
                i32::try_from(ordinal)
                    .map_err(|_| conflict("LEGACY_REPORT_AUTHORITY_SET_TOO_LARGE"))?,
            )
            .bind(row.receipt_id)
            .bind(&row.source_record_hash)
            .bind(member_hash)
            .execute(&mut **tx)
            .await?;
        }
    } else {
        let exact: bool = sqlx::query_scalar(
            r#"SELECT seal_id=$2 AND project_scope_id=$3 AND source_member_count=$4
                      AND source_membership_hash=$5 AND limitation_membership_hash=$6
                      AND adapter_digest=$7 AND seal_hash=$8
                 FROM legacy_report_authority_seals
                WHERE operation_id=$1 AND adapter_version=$9 FOR SHARE"#,
        )
        .bind(input.operation_id)
        .bind(seal_id)
        .bind(input.project_scope_id)
        .bind(source_member_count)
        .bind(&source_membership_hash)
        .bind(&limitation_membership_hash)
        .bind(&input.adapter_digest)
        .bind(&seal_hash)
        .bind(&input.adapter_version)
        .fetch_one(&mut **tx)
        .await?;
        let persisted = sqlx::query_as::<_, PersistedMemberRow>(
            r#"SELECT ordinal,legacy_attempt_authority_receipt_id,receipt_hash,member_hash
                 FROM legacy_report_authority_members
                WHERE seal_id=$1 ORDER BY ordinal FOR SHARE"#,
        )
        .bind(seal_id)
        .fetch_all(&mut **tx)
        .await?;
        let exact_members = persisted.len() == rows.len()
            && persisted
                .iter()
                .zip(&rows)
                .zip(&member_hashes)
                .enumerate()
                .all(|(ordinal, ((actual, expected), expected_hash))| {
                    i32::try_from(ordinal).ok() == Some(actual.ordinal)
                        && actual.legacy_attempt_authority_receipt_id == expected.receipt_id
                        && actual.receipt_hash == expected.source_record_hash
                        && &actual.member_hash == expected_hash
                });
        if !exact || !exact_members {
            return Err(conflict("LEGACY_REPORT_AUTHORITY_REPLAY_DRIFT"));
        }
    }
    Ok(LegacyReportAuthoritySealV1 {
        seal_id,
        operation_id: input.operation_id,
        project_scope_id: input.project_scope_id,
        source_member_count,
        source_membership_hash,
        limitation_membership_hash,
        adapter_version: input.adapter_version,
        adapter_digest: input.adapter_digest,
        seal_hash,
        replayed,
    })
}

#[cfg(test)]
mod tests {
    use super::{tagged_hash, valid_tagged_hash};
    use serde_json::json;

    #[test]
    fn report_authority_hash_is_domain_separated() {
        let hash = tagged_hash(&json!({"schema":"legacy_report_authority_seal.v1"}));
        assert!(valid_tagged_hash(&hash));
        assert_ne!(
            hash,
            tagged_hash(&json!({"schema":"legacy_attempt_authority.v1"}))
        );
    }
}
