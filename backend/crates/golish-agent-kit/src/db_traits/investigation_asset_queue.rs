//! SQLx-free durable company/asset queue boundary for Investigation.
//!
//! The repository freezes its members from the sealed operation scope and the
//! live in-scope target catalog. Callers can only name the expected next
//! server-owned member and advance a compare-and-swap head; they cannot submit
//! company or asset census rows.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::UnifiedInvestigationStageIdentity;

pub type InvestigationAssetQueueResult<T> = Result<T, InvestigationAssetQueueRepositoryError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvestigationAssetQueueRepositoryError {
    #[error("investigation_asset_queue_repository_unavailable: {operation}")]
    Unavailable { operation: &'static str },
    #[error("investigation_asset_queue_repository_invalid_request: {detail}")]
    InvalidRequest { detail: String },
    #[error("investigation_asset_queue_repository_not_found: {detail}")]
    NotFound { detail: String },
    #[error("investigation_asset_queue_repository_conflict: {detail}")]
    Conflict { detail: String },
    #[error("investigation_asset_queue_repository_authority_mismatch: {detail}")]
    AuthorityMismatch { detail: String },
    #[error("investigation_asset_queue_repository_evolution_fuel_exhausted: {asset_lane_id}")]
    EvolutionFuelExhausted { asset_lane_id: Uuid },
    #[error("investigation_asset_queue_repository_infrastructure: {detail}")]
    Infrastructure { detail: String },
}

impl InvestigationAssetQueueRepositoryError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "investigation_asset_queue_repository_unavailable",
            Self::InvalidRequest { .. } => "investigation_asset_queue_repository_invalid_request",
            Self::NotFound { .. } => "investigation_asset_queue_repository_not_found",
            Self::Conflict { .. } => "investigation_asset_queue_repository_conflict",
            Self::AuthorityMismatch { .. } => {
                "investigation_asset_queue_repository_authority_mismatch"
            }
            Self::EvolutionFuelExhausted { .. } => {
                "investigation_asset_queue_repository_evolution_fuel_exhausted"
            }
            Self::Infrastructure { .. } => "investigation_asset_queue_repository_infrastructure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationCompanyLaneState {
    Queued,
    Active,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationAssetLaneState {
    Queued,
    Analyzing,
    Verifying,
    Consolidating,
    Evolving,
    FixedPoint,
    Blocked,
    Residual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCompanyQueueMemberView {
    pub company_member_id: Uuid,
    pub organization_id: Uuid,
    pub depth: u32,
    pub ordinal: u32,
    pub state: InvestigationCompanyLaneState,
    /// CAS head of this member's owning company queue.
    pub company_queue_head_version: i64,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAssetLaneView {
    pub asset_lane_id: Uuid,
    pub asset_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub organization_id: Uuid,
    pub target_id: Uuid,
    pub target_type: String,
    pub target_value: String,
    pub target_source: String,
    pub target_identity_sha256: String,
    pub ordinal: u32,
    pub state: InvestigationAssetLaneState,
    pub evolution_epoch: u32,
    pub max_evolution_epochs: u32,
    /// CAS head of this lane's owning asset queue.
    pub asset_queue_head_version: i64,
    pub row_version: i64,
}

/// Load the one server-owned pending evolution authority for the current
/// Analysis epoch.  The caller supplies only the already-frozen lane owner and
/// expected epoch; it cannot nominate the authority id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadCurrentInvestigationAssetEvolutionAuthority {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub expected_evolution_epoch: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvestigationAssetEvolutionAuthorityView {
    pub asset_lane_id: Uuid,
    pub evolution_epoch: u32,
    pub pending_evolution_authority_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCompanyAssetQueueView {
    pub company_queue_id: Uuid,
    pub stage: UnifiedInvestigationStageIdentity,
    pub company_member_count: u32,
    pub company_member_set_sha256: String,
    pub company_head_version: i64,
    pub companies: Vec<InvestigationCompanyQueueMemberView>,
    pub assets: Vec<InvestigationAssetLaneView>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreezeInvestigationCompanyAssetQueue {
    pub stable_request_id: Uuid,
    pub stage: UnifiedInvestigationStageIdentity,
    pub max_evolution_epochs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimNextInvestigationCompany {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub expected_company_member_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_member_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimNextInvestigationAsset {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_asset_lane_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_lane_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionInvestigationAssetLane {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_lane_row_version: i64,
    pub from_state: InvestigationAssetLaneState,
    pub to_state: InvestigationAssetLaneState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompleteInvestigationCompany {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_member_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealZeroHypothesisAssetFixedPoint {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_lane_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAssetFixedPointReceiptView {
    pub fixed_point_receipt_id: Uuid,
    pub asset_lane: InvestigationAssetLaneView,
    pub receipt_sha256: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAssetBacklogView {
    pub asset_lane: InvestigationAssetLaneView,
    pub latest_generation_id: Option<Uuid>,
    pub latest_generation_seal_id: Option<Uuid>,
    pub generation_count: u32,
    pub hypothesis_root_count: u32,
    /// Current roots whose terminal head is backed by the exact dynamic-v2
    /// round -> resolution -> terminal-transition authority chain.
    pub dynamically_resolved_root_count: u32,
    pub revision_count: u32,
    pub verification_task_count: u32,
    pub open_verification_task_count: u32,
    pub campaign_count: u32,
    pub open_campaign_count: u32,
    pub prepared_action_count: u32,
    pub open_prepared_action_count: u32,
    pub action_execution_count: u32,
    pub open_action_execution_count: u32,
    pub oracle_count: u32,
    pub fact_delta_count: u32,
    pub wave_count: u32,
    pub advanced_wave_count: u32,
    pub fixed_point_wave_count: u32,
    pub pending_evolution_count: u32,
    pub pending_hypothesis_discovery_count: u32,
    pub backlog_member_count: u32,
    pub backlog_set_sha256: String,
    pub obligation_set_sha256: String,
    pub residual_set_sha256: String,
    pub zero_hypothesis_fixed_point_receipt_id: Option<Uuid>,
}

impl InvestigationAssetBacklogView {
    pub const fn is_drained(&self) -> bool {
        self.backlog_member_count == 0
            && self.pending_hypothesis_discovery_count == 0
            && ((self.hypothesis_root_count > 0
                && self.revision_count > 0
                && self.dynamically_resolved_root_count == self.hypothesis_root_count)
                || (self.hypothesis_root_count == 0
                    && self.revision_count == 0
                    && self.dynamically_resolved_root_count == 0
                    && self.zero_hypothesis_fixed_point_receipt_id.is_some()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadInvestigationAssetBacklog {
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseInvestigationAssetBacklogAndAdvance {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_company_queue_head_version: i64,
    pub expected_company_member_row_version: i64,
    pub expected_asset_queue_head_version: i64,
    pub expected_asset_lane_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationAssetProgressionDisposition {
    NextAsset,
    NextCompany,
    InvestigationComplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationResolutionClosureMemberView {
    pub organization_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub passed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationResolutionClosurePublicationView {
    pub publication_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub member_set_sha256: String,
    pub members: Vec<InvestigationResolutionClosureMemberView>,
}

impl InvestigationResolutionClosurePublicationView {
    /// Project the immutable per-company completion authority used by the
    /// outer Investigation pass-token gate. The persistence adapter verifies
    /// the publication/member hashes and live unit/plan/completion rows; this
    /// SQL-free boundary additionally refuses incomplete, foreign, or
    /// duplicate projections before they become a stage completion
    /// denominator.
    pub fn exact_completion_authority(
        &self,
        expected_operation_id: Uuid,
    ) -> InvestigationAssetQueueResult<Vec<(Uuid, DateTime<Utc>)>> {
        let mismatch =
            |detail: &'static str| InvestigationAssetQueueRepositoryError::AuthorityMismatch {
                detail: detail.to_string(),
            };
        if expected_operation_id.is_nil()
            || self.publication_id.is_nil()
            || self.operation_id != expected_operation_id
            || self.stage_execution_id.is_nil()
            || self.scope_snapshot_id.is_nil()
            || self.member_set_sha256.trim().is_empty()
            || self.members.is_empty()
        {
            return Err(mismatch("resolution_closure_publication_identity_mismatch"));
        }

        let mut organization_ids = std::collections::BTreeSet::new();
        let mut unit_ids = std::collections::BTreeSet::new();
        let mut plan_ids = std::collections::BTreeSet::new();
        let mut authority = Vec::with_capacity(self.members.len());
        for member in &self.members {
            if member.organization_id.is_nil()
                || member.stage_run_unit_id.is_nil()
                || member.stage_team_plan_id.is_nil()
                || !organization_ids.insert(member.organization_id)
                || !unit_ids.insert(member.stage_run_unit_id)
                || !plan_ids.insert(member.stage_team_plan_id)
            {
                return Err(mismatch("resolution_closure_member_authority_mismatch"));
            }
            authority.push((member.organization_id, member.passed_at));
        }
        authority.sort_by_key(|(organization_id, _)| *organization_id);
        Ok(authority)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAssetProgressionView {
    pub progression_receipt_id: Uuid,
    pub fixed_asset_lane_id: Uuid,
    pub disposition: InvestigationAssetProgressionDisposition,
    pub next_company_member_id: Option<Uuid>,
    pub next_asset_lane: Option<InvestigationAssetLaneView>,
    pub company_queue_head_version: i64,
    pub stage_closure: Option<InvestigationResolutionClosurePublicationView>,
    pub replayed: bool,
}

#[async_trait]
pub trait InvestigationAssetQueueRepository: Send + Sync {
    async fn freeze(
        &self,
        request: FreezeInvestigationCompanyAssetQueue,
    ) -> InvestigationAssetQueueResult<InvestigationCompanyAssetQueueView>;

    async fn claim_next_company(
        &self,
        request: ClaimNextInvestigationCompany,
    ) -> InvestigationAssetQueueResult<InvestigationCompanyQueueMemberView>;

    async fn claim_next_asset(
        &self,
        request: ClaimNextInvestigationAsset,
    ) -> InvestigationAssetQueueResult<InvestigationAssetLaneView>;

    async fn transition_asset(
        &self,
        request: TransitionInvestigationAssetLane,
    ) -> InvestigationAssetQueueResult<InvestigationAssetLaneView>;

    async fn load_current_evolution_authority(
        &self,
        request: LoadCurrentInvestigationAssetEvolutionAuthority,
    ) -> InvestigationAssetQueueResult<InvestigationAssetEvolutionAuthorityView>;

    async fn seal_zero_hypothesis_fixed_point(
        &self,
        request: SealZeroHypothesisAssetFixedPoint,
    ) -> InvestigationAssetQueueResult<InvestigationAssetFixedPointReceiptView>;

    async fn complete_company(
        &self,
        request: CompleteInvestigationCompany,
    ) -> InvestigationAssetQueueResult<InvestigationCompanyQueueMemberView>;

    async fn load_backlog(
        &self,
        _request: LoadInvestigationAssetBacklog,
    ) -> InvestigationAssetQueueResult<InvestigationAssetBacklogView> {
        Err(InvestigationAssetQueueRepositoryError::Unavailable {
            operation: "load_investigation_asset_backlog",
        })
    }

    async fn close_backlog_and_advance(
        &self,
        _request: CloseInvestigationAssetBacklogAndAdvance,
    ) -> InvestigationAssetQueueResult<InvestigationAssetProgressionView> {
        Err(InvestigationAssetQueueRepositoryError::Unavailable {
            operation: "close_investigation_asset_backlog_and_advance",
        })
    }

    async fn load_resolution_closure(
        &self,
        _operation_id: Uuid,
    ) -> InvestigationAssetQueueResult<Option<InvestigationResolutionClosurePublicationView>> {
        Err(InvestigationAssetQueueRepositoryError::Unavailable {
            operation: "load_investigation_resolution_closure",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolution_closure() -> InvestigationResolutionClosurePublicationView {
        InvestigationResolutionClosurePublicationView {
            publication_id: Uuid::from_u128(1),
            operation_id: Uuid::from_u128(2),
            stage_execution_id: Uuid::from_u128(3),
            scope_snapshot_id: Uuid::from_u128(4),
            member_set_sha256: format!("sha256:{}", "a".repeat(64)),
            members: vec![InvestigationResolutionClosureMemberView {
                organization_id: Uuid::from_u128(5),
                stage_run_unit_id: Uuid::from_u128(6),
                stage_team_plan_id: Uuid::from_u128(7),
                passed_at: Utc::now(),
            }],
        }
    }

    #[test]
    fn resolution_closure_projects_exact_completion_authority() {
        let publication = resolution_closure();
        assert_eq!(
            publication
                .exact_completion_authority(publication.operation_id)
                .expect("valid queue closure authority"),
            vec![(
                publication.members[0].organization_id,
                publication.members[0].passed_at,
            )]
        );
    }

    #[test]
    fn resolution_closure_rejects_foreign_and_duplicate_members() {
        let publication = resolution_closure();
        assert!(matches!(
            publication.exact_completion_authority(Uuid::from_u128(8)),
            Err(InvestigationAssetQueueRepositoryError::AuthorityMismatch { .. })
        ));

        let mut duplicate = publication;
        let mut member = duplicate.members[0].clone();
        member.stage_run_unit_id = Uuid::from_u128(9);
        member.stage_team_plan_id = Uuid::from_u128(10);
        duplicate.members.push(member);
        assert!(matches!(
            duplicate.exact_completion_authority(duplicate.operation_id),
            Err(InvestigationAssetQueueRepositoryError::AuthorityMismatch { .. })
        ));
    }
}
