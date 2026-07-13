use chrono::{DateTime, Utc};
use golish_memory_domain::event_catalog::{KnowledgeEventEnvelopeV1, KnowledgeEventNameV1};
use golish_memory_domain::scope::{OperationScope, ProjectScopeId};
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind};
use golish_memory_domain::{EpisodeVerdict, StageEpisode};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::knowledge_outbox::{
    append_event_with_catalog_deliveries_with_connection, KnowledgeOutboxError,
};

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct StageEpisodeRow {
    pub episode_id: Uuid,
    pub project_scope_id: Uuid,
    pub source_operation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub source_scope_snapshot_hash: String,
    pub stage_execution_id: Uuid,
    pub stage_kind: String,
    pub stage_run_unit_id_at_time: Option<Uuid>,
    pub worker_run_id_at_time: Option<Uuid>,
    pub candidate_attempt_id_at_time: Option<Uuid>,
    pub wave: Option<i32>,
    pub verdict: String,
    pub deliverable_submission_id_at_time: Option<Uuid>,
    pub handoff_id_at_time: Option<Uuid>,
    pub reason_codes: serde_json::Value,
    pub fact_refs: serde_json::Value,
    pub evidence_refs: Vec<i64>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

impl StageEpisodeRow {
    pub fn into_domain(self) -> Result<StageEpisode, StageEpisodeError> {
        let episode = StageEpisode {
            episode_id: self.episode_id,
            scope: OperationScope {
                project_scope_id: ProjectScopeId(self.project_scope_id),
                source_operation_id: self.source_operation_id,
                organization_id_at_time: self.organization_id_at_time,
                scope_snapshot_hash: self.source_scope_snapshot_hash,
            },
            stage_execution_id: self.stage_execution_id,
            stage_run_unit_id: self.stage_run_unit_id_at_time,
            worker_run_id: self.worker_run_id_at_time,
            candidate_attempt_id: self.candidate_attempt_id_at_time,
            stage_kind: self.stage_kind,
            wave: self.wave,
            verdict: parse_verdict(&self.verdict)?,
            deliverable_submission_id: self.deliverable_submission_id_at_time,
            handoff_id: self.handoff_id_at_time,
            reason_codes: serde_json::from_value(self.reason_codes)?,
            fact_refs: serde_json::from_value(self.fact_refs)?,
            evidence_ids: self.evidence_refs,
            started_at: self.started_at,
            ended_at: self.ended_at,
        };
        episode
            .validate()
            .map_err(StageEpisodeError::InvalidEpisode)?;
        Ok(episode)
    }
}

pub async fn close_episode_and_emit(
    pool: &PgPool,
    episode: &StageEpisode,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Uuid, StageEpisodeError> {
    let mut tx = pool.begin().await?;
    let event_id = close_episode_with_event(&mut tx, episode, event).await?;
    tx.commit().await?;
    Ok(event_id)
}

/// Caller-owned atomic seam for C2 canonical terminal transactions.
pub async fn close_episode_with_event(
    tx: &mut Transaction<'_, Postgres>,
    episode: &StageEpisode,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Uuid, StageEpisodeError> {
    close_episode_with_event_with_connection(tx, episode, event).await
}

pub async fn close_episode_with_event_with_connection(
    connection: &mut PgConnection,
    episode: &StageEpisode,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Uuid, StageEpisodeError> {
    episode
        .validate()
        .map_err(StageEpisodeError::InvalidEpisode)?;
    if event.project_scope_id != Some(episode.scope.project_scope_id)
        || event.source_operation_id != episode.scope.source_operation_id
        || event.organization_id_at_time != Some(episode.scope.organization_id_at_time)
        || event.event_name != KnowledgeEventNameV1::StageEpisodeClosed
        || event.payload.source.source_kind != CanonicalSourceKind::StageEpisode
        || event.payload.source.row_id != CanonicalRowId::Uuid(episode.episode_id)
    {
        return Err(StageEpisodeError::EventSourceMismatch);
    }
    let reason_codes = serde_json::to_value(&episode.reason_codes)?;
    let fact_refs = serde_json::to_value(&episode.fact_refs)?;
    let inserted = sqlx::query(
        r#"INSERT INTO stage_episodes (
               episode_id, project_scope_id, source_operation_id,
               organization_id_at_time, source_scope_snapshot_hash,
               stage_execution_id, stage_kind, stage_run_unit_id_at_time,
               worker_run_id_at_time, candidate_attempt_id_at_time, wave, verdict,
               deliverable_submission_id_at_time, handoff_id_at_time, reason_codes,
               fact_refs, evidence_refs, started_at, ended_at
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19
           )
           ON CONFLICT (episode_id) DO NOTHING"#,
    )
    .bind(episode.episode_id)
    .bind(episode.scope.project_scope_id.0)
    .bind(episode.scope.source_operation_id)
    .bind(episode.scope.organization_id_at_time)
    .bind(&episode.scope.scope_snapshot_hash)
    .bind(episode.stage_execution_id)
    .bind(&episode.stage_kind)
    .bind(episode.stage_run_unit_id)
    .bind(episode.worker_run_id)
    .bind(episode.candidate_attempt_id)
    .bind(episode.wave)
    .bind(episode.verdict.as_str())
    .bind(episode.deliverable_submission_id)
    .bind(episode.handoff_id)
    .bind(&reason_codes)
    .bind(&fact_refs)
    .bind(&episode.evidence_ids)
    .bind(episode.started_at)
    .bind(episode.ended_at)
    .execute(&mut *connection)
    .await?;
    if inserted.rows_affected() == 0 {
        let existing = fetch_with_connection(connection, episode.episode_id).await?;
        let exact_replay = existing.project_scope_id == episode.scope.project_scope_id.0
            && existing.source_operation_id == episode.scope.source_operation_id
            && existing.organization_id_at_time == episode.scope.organization_id_at_time
            && existing.source_scope_snapshot_hash == episode.scope.scope_snapshot_hash
            && existing.stage_execution_id == episode.stage_execution_id
            && existing.stage_kind == episode.stage_kind
            && existing.stage_run_unit_id_at_time == episode.stage_run_unit_id
            && existing.worker_run_id_at_time == episode.worker_run_id
            && existing.candidate_attempt_id_at_time == episode.candidate_attempt_id
            && existing.wave == episode.wave
            && existing.verdict == episode.verdict.as_str()
            && existing.deliverable_submission_id_at_time == episode.deliverable_submission_id
            && existing.handoff_id_at_time == episode.handoff_id
            && existing.reason_codes == reason_codes
            && existing.fact_refs == fact_refs
            && existing.evidence_refs == episode.evidence_ids
            && existing.started_at.timestamp_micros() == episode.started_at.timestamp_micros()
            && existing.ended_at.timestamp_micros() == episode.ended_at.timestamp_micros();
        if !exact_replay {
            return Err(StageEpisodeError::ReplayConflict);
        }
    }

    append_event_with_catalog_deliveries_with_connection(connection, event)
        .await
        .map_err(Into::into)
}

pub async fn get(pool: &PgPool, episode_id: Uuid) -> Result<StageEpisodeRow, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT episode_id, project_scope_id, source_operation_id,
                  organization_id_at_time, source_scope_snapshot_hash,
                  stage_execution_id, stage_kind, stage_run_unit_id_at_time,
                  worker_run_id_at_time, candidate_attempt_id_at_time, wave,
                  verdict, deliverable_submission_id_at_time, handoff_id_at_time,
                  reason_codes, fact_refs, evidence_refs, started_at, ended_at
           FROM stage_episodes WHERE episode_id = $1"#,
    )
    .bind(episode_id)
    .fetch_one(pool)
    .await
}

async fn fetch_with_connection(
    connection: &mut PgConnection,
    episode_id: Uuid,
) -> Result<StageEpisodeRow, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT episode_id, project_scope_id, source_operation_id,
                  organization_id_at_time, source_scope_snapshot_hash,
                  stage_execution_id, stage_kind, stage_run_unit_id_at_time,
                  worker_run_id_at_time, candidate_attempt_id_at_time, wave,
                  verdict, deliverable_submission_id_at_time, handoff_id_at_time,
                  reason_codes, fact_refs, evidence_refs, started_at, ended_at
           FROM stage_episodes WHERE episode_id = $1"#,
    )
    .bind(episode_id)
    .fetch_one(&mut *connection)
    .await
}

pub fn parse_verdict(value: &str) -> Result<EpisodeVerdict, StageEpisodeError> {
    match value {
        "passed" => Ok(EpisodeVerdict::Passed),
        "blocked" => Ok(EpisodeVerdict::Blocked),
        "exhausted" => Ok(EpisodeVerdict::Exhausted),
        "failed" => Ok(EpisodeVerdict::Failed),
        "superseded" => Ok(EpisodeVerdict::Superseded),
        _ => Err(StageEpisodeError::CorruptVerdict),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StageEpisodeError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Outbox(#[from] KnowledgeOutboxError),
    #[error("invalid stage episode: {0}")]
    InvalidEpisode(&'static str),
    #[error("stage episode event source does not match the episode")]
    EventSourceMismatch,
    #[error("stored episode verdict is corrupt")]
    CorruptVerdict,
    #[error("stage episode replay conflicts with the immutable stored row")]
    ReplayConflict,
}

impl StageEpisodeError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Sqlx(_) => "memory_episode_database_error",
            Self::Json(_) => "memory_episode_serialization_failed",
            Self::Outbox(error) => error.code(),
            Self::InvalidEpisode(code) => code,
            Self::EventSourceMismatch => "memory_episode_event_source_mismatch",
            Self::CorruptVerdict => "memory_episode_row_corrupt",
            Self::ReplayConflict => "memory_episode_replay_conflict",
        }
    }
}
