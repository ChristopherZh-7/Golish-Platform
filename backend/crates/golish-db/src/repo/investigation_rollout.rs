use chrono::{DateTime, Utc};
use golish_core::{InvestigationContractVersion, InvestigationRolloutMode};
use sqlx::PgConnection;

use crate::Result;

pub const TABLE_NAME: &str = "investigation_rollout";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InvestigationRolloutRow {
    pub contract_version: String,
    pub rollout_mode: String,
    pub mode_rank: i16,
    pub row_version: i64,
    pub updated_at: DateTime<Utc>,
}

/// Read and share-lock the deployment default through a caller-owned
/// transaction. There is intentionally no mutation or promotion API in Plan B.
pub async fn get_for_share(connection: &mut PgConnection) -> Result<InvestigationRolloutRow> {
    Ok(sqlx::query_as::<_, InvestigationRolloutRow>(
        r#"SELECT contract_version,rollout_mode,mode_rank,row_version,updated_at
             FROM investigation_rollout
            WHERE singleton=TRUE
            FOR SHARE"#,
    )
    .fetch_one(connection)
    .await?)
}

/// Strictly decode one frozen operation pair. Unknown values and mismatched
/// contract/mode combinations fail closed; they never project legacy policy.
pub fn parse_frozen_pair(
    contract: &str,
    mode: &str,
) -> Result<(InvestigationContractVersion, InvestigationRolloutMode)> {
    let contract = InvestigationContractVersion::try_from(contract)
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))?;
    let mode = InvestigationRolloutMode::try_from(mode)
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))?;
    if !contract.allows(mode) {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "investigation contract/mode pair is not allowed: {}/{}",
            contract.as_str(),
            mode.as_str()
        )));
    }
    Ok((contract, mode))
}
