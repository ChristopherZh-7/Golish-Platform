use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "attack_paths";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct AttackPathRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub path_hash: String,
    pub status: String,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct AttackPathEdgeRow {
    pub id: Uuid,
    pub attack_path_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub ordinal: i32,
    pub from_identity_hash: String,
    pub to_identity_hash: String,
    pub technique: String,
    pub properties: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewAttackPathEdge {
    pub id: Uuid,
    pub ordinal: i32,
    pub from_identity_hash: String,
    pub to_identity_hash: String,
    pub technique: String,
    pub properties: Value,
    pub evidence: Vec<(i64, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewAttackPath {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub path_hash: String,
    pub edges: Vec<NewAttackPathEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttackPathWithEdges {
    pub path: AttackPathRow,
    pub edges: Vec<AttackPathEdgeRow>,
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate(input: &NewAttackPath) -> Result<()> {
    if !is_hash(&input.path_hash) || input.edges.is_empty() || input.edges.len() > 256 {
        return Err(anyhow::anyhow!("post_exploit_attack_path_invalid").into());
    }
    let mut ordinals = input
        .edges
        .iter()
        .map(|edge| edge.ordinal)
        .collect::<Vec<_>>();
    ordinals.sort_unstable();
    if ordinals != (0..input.edges.len() as i32).collect::<Vec<_>>() {
        return Err(anyhow::anyhow!("post_exploit_attack_path_ordinal_gap").into());
    }
    for edge in &input.edges {
        if !is_hash(&edge.from_identity_hash)
            || !is_hash(&edge.to_identity_hash)
            || edge.from_identity_hash == edge.to_identity_hash
            || edge.technique.trim().is_empty()
            || edge.technique.len() > 128
            || !edge.properties.is_object()
            || edge.evidence.is_empty()
            || edge.evidence.len() > 1024
            || edge
                .evidence
                .iter()
                .any(|(id, role)| *id <= 0 || !matches!(role.as_str(), "observation" | "support"))
        {
            return Err(anyhow::anyhow!("post_exploit_attack_path_edge_invalid").into());
        }
        let mut evidence = edge.evidence.clone();
        evidence.sort_unstable();
        evidence.dedup();
        if evidence.len() != edge.evidence.len() {
            return Err(anyhow::anyhow!("post_exploit_attack_path_edge_evidence_duplicate").into());
        }
    }
    Ok(())
}

async fn load_with_connection(
    connection: &mut PgConnection,
    path_id: Uuid,
) -> Result<AttackPathWithEdges> {
    let path =
        sqlx::query_as::<_, AttackPathRow>("SELECT * FROM attack_paths WHERE id=$1 FOR SHARE")
            .bind(path_id)
            .fetch_one(&mut *connection)
            .await?;
    let edges = sqlx::query_as::<_, AttackPathEdgeRow>(
        "SELECT * FROM attack_path_edges WHERE attack_path_id=$1 ORDER BY ordinal",
    )
    .bind(path_id)
    .fetch_all(&mut *connection)
    .await?;
    Ok(AttackPathWithEdges { path, edges })
}

async fn ensure_exact_replay(
    connection: &mut PgConnection,
    stored: &AttackPathWithEdges,
    input: &NewAttackPath,
) -> Result<()> {
    if stored.edges.len() != input.edges.len()
        || stored
            .edges
            .iter()
            .zip(&input.edges)
            .any(|(stored, expected)| {
                stored.id != expected.id
                    || stored.ordinal != expected.ordinal
                    || stored.from_identity_hash != expected.from_identity_hash
                    || stored.to_identity_hash != expected.to_identity_hash
                    || stored.technique != expected.technique
                    || stored.properties != expected.properties
            })
    {
        return Err(anyhow::anyhow!("post_exploit_attack_path_edge_replay_conflict").into());
    }
    for edge in &input.edges {
        let stored_evidence = sqlx::query_as::<_, (i64, String)>(
            r#"SELECT evidence_id,role FROM attack_path_edge_evidence
                WHERE attack_path_edge_id=$1 ORDER BY evidence_id,role"#,
        )
        .bind(edge.id)
        .fetch_all(&mut *connection)
        .await?;
        let mut expected_evidence = edge.evidence.clone();
        expected_evidence.sort_unstable();
        if stored_evidence != expected_evidence {
            return Err(
                anyhow::anyhow!("post_exploit_attack_path_edge_evidence_replay_conflict").into(),
            );
        }
    }
    Ok(())
}

pub async fn insert_with_connection(
    connection: &mut PgConnection,
    input: &NewAttackPath,
) -> Result<AttackPathWithEdges> {
    validate(input)?;
    let mut evidence_ids = input
        .edges
        .iter()
        .flat_map(|edge| edge.evidence.iter().map(|(id, _)| *id))
        .collect::<Vec<_>>();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    super::foothold_candidates::validate_evidence_authority(
        connection,
        input.operation_id,
        input.scope_snapshot_id,
        input.organization_id_at_time,
        &evidence_ids,
    )
    .await?;
    let inserted = sqlx::query_as::<_, AttackPathRow>(
        r#"INSERT INTO attack_paths(
               id,operation_id,project_scope_id,scope_snapshot_id,
               organization_id_at_time,path_hash
           ) VALUES($1,$2,$3,$4,$5,$6)
           ON CONFLICT(operation_id,organization_id_at_time,path_hash) DO NOTHING
           RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id_at_time)
    .bind(&input.path_hash)
    .fetch_optional(&mut *connection)
    .await?;
    let (path, inserted_new) = match inserted {
        Some(row) => (row, true),
        None => {
            let row = sqlx::query_as::<_, AttackPathRow>(
                r#"SELECT * FROM attack_paths
                    WHERE operation_id=$1 AND organization_id_at_time=$2
                      AND path_hash=$3 FOR SHARE"#,
            )
            .bind(input.operation_id)
            .bind(input.organization_id_at_time)
            .bind(&input.path_hash)
            .fetch_one(&mut *connection)
            .await?;
            if row.id != input.id
                || row.project_scope_id != input.project_scope_id
                || row.scope_snapshot_id != input.scope_snapshot_id
            {
                return Err(anyhow::anyhow!("post_exploit_attack_path_replay_conflict").into());
            }
            (row, false)
        }
    };
    if !inserted_new {
        let stored = load_with_connection(connection, path.id).await?;
        ensure_exact_replay(connection, &stored, input).await?;
        return Ok(stored);
    }
    for edge in &input.edges {
        sqlx::query(
            r#"INSERT INTO attack_path_edges(
                   id,attack_path_id,operation_id,project_scope_id,scope_snapshot_id,
                   organization_id_at_time,ordinal,from_identity_hash,to_identity_hash,
                   technique,properties
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
               ON CONFLICT(attack_path_id,ordinal) DO NOTHING"#,
        )
        .bind(edge.id)
        .bind(path.id)
        .bind(input.operation_id)
        .bind(input.project_scope_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id_at_time)
        .bind(edge.ordinal)
        .bind(&edge.from_identity_hash)
        .bind(&edge.to_identity_hash)
        .bind(&edge.technique)
        .bind(&edge.properties)
        .execute(&mut *connection)
        .await?;
        for (evidence_id, role) in &edge.evidence {
            sqlx::query(
                r#"INSERT INTO attack_path_edge_evidence(
                       attack_path_edge_id,evidence_id,role
                   ) VALUES($1,$2,$3) ON CONFLICT DO NOTHING"#,
            )
            .bind(edge.id)
            .bind(evidence_id)
            .bind(role)
            .execute(&mut *connection)
            .await?;
        }
    }
    let stored = load_with_connection(connection, path.id).await?;
    ensure_exact_replay(connection, &stored, input).await?;
    Ok(stored)
}

pub async fn create(pool: &PgPool, input: &NewAttackPath) -> Result<AttackPathWithEdges> {
    let mut tx = pool.begin().await?;
    let rows = insert_with_connection(&mut tx, input).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<AttackPathWithEdges>> {
    let mut connection = pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attack_paths WHERE id=$1)")
        .bind(id)
        .fetch_one(&mut *connection)
        .await?;
    if !exists {
        return Ok(None);
    }
    Ok(Some(load_with_connection(&mut connection, id).await?))
}
