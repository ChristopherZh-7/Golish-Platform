//! Server-owned operator identity repository.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "operator_principals";

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct OperatorPrincipalRow {
    pub id: Uuid,
    pub principal_kind: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Load the one active local operator identity. There is deliberately no ID
/// argument: request/model input cannot select or impersonate an actor.
pub async fn current_local(pool: &PgPool) -> Result<OperatorPrincipalRow> {
    Ok(sqlx::query_as::<_, OperatorPrincipalRow>(
        r#"SELECT id, principal_kind, active, created_at, updated_at
             FROM operator_principals
            WHERE principal_kind = 'local_operator' AND active
            FOR SHARE"#,
    )
    .fetch_one(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_operator_query_has_no_request_selected_identity() {
        let source = include_str!("operator_principals.rs");
        assert!(source.contains("WHERE principal_kind = 'local_operator' AND active"));
        let request_selected_signature =
            ["pub async fn current_local(pool: &PgPool", ", id"].concat();
        assert!(!source.contains(&request_selected_signature));
        assert_eq!(TABLE_NAME, "operator_principals");
    }
}
