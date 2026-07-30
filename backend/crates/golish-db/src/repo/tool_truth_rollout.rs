use golish_pentest_domain::tool_truth::ToolTruthContract;
use sqlx::{Postgres, Transaction};

use crate::Result;

pub const TABLE_NAME: &str = "tool_truth_rollout";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ToolTruthRolloutRow {
    pub new_operation_contract: String,
}

/// Lock and strictly decode the Plan A deployment default.
///
/// Plan A deliberately exposes no setter or promotion seam. The database also
/// rejects direct singleton mutation, so every new operation freezes the same
/// legacy-safe contract until a separately authorized forward migration lands.
pub async fn get_for_share(tx: &mut Transaction<'_, Postgres>) -> Result<ToolTruthContract> {
    let row = sqlx::query_as::<_, ToolTruthRolloutRow>(
        "SELECT new_operation_contract FROM tool_truth_rollout WHERE singleton=TRUE FOR SHARE",
    )
    .fetch_one(&mut **tx)
    .await?;
    ToolTruthContract::try_from(row.new_operation_contract.as_str())
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))
}
