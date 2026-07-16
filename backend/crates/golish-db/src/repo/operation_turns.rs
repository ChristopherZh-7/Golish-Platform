use chrono::{DateTime, Utc};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

use super::runtime_memory_tx::RuntimeMemoryStoreResult;

pub const TABLE_NAME: &str = "operation_turns";

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct OperationTurnRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub ordinal: i64,
    pub trigger_input: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

const COLUMNS: &str = "id,operation_id,ordinal,trigger_input,status,started_at,terminal_at";

pub async fn insert_initial_with_executor<'e, E>(
    executor: E,
    operation_id: Uuid,
    trigger_input: &str,
) -> RuntimeMemoryStoreResult<OperationTurnRow>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "INSERT INTO operation_turns(id,operation_id,ordinal,trigger_input,status)
         VALUES($1,$2,1,$3,'running')
         RETURNING {COLUMNS}"
    );
    Ok(sqlx::query_as::<_, OperationTurnRow>(&sql)
        .bind(Uuid::new_v4())
        .bind(operation_id)
        .bind(trigger_input)
        .fetch_one(executor)
        .await?)
}

pub async fn list_for_operation<'e, E>(
    executor: E,
    operation_id: Uuid,
) -> RuntimeMemoryStoreResult<Vec<OperationTurnRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM operation_turns
          WHERE operation_id=$1 ORDER BY ordinal"
    );
    Ok(sqlx::query_as::<_, OperationTurnRow>(&sql)
        .bind(operation_id)
        .fetch_all(executor)
        .await?)
}

pub async fn get_open<'e, E>(
    executor: E,
    operation_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<OperationTurnRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM operation_turns
          WHERE operation_id=$1 AND status IN ('running','waiting')"
    );
    Ok(sqlx::query_as::<_, OperationTurnRow>(&sql)
        .bind(operation_id)
        .fetch_optional(executor)
        .await?)
}
