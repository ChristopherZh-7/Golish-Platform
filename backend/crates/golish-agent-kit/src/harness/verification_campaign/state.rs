//! Pure Campaign lifecycle and single-action lane enforcement.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::types::VerificationCampaignError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignPhaseV1 {
    Planning,
    ActionActive,
    Stopping,
    Draining,
    ReadyToTerminalize,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignStopReasonV1 {
    BudgetExhausted,
    DeadlineReached,
    NoProgress,
    ObjectiveDecided,
    PolicyStopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignStateV1 {
    campaign_id: Uuid,
    phase: CampaignPhaseV1,
    active_action_id: Option<Uuid>,
    stop_reason: Option<CampaignStopReasonV1>,
}

impl CampaignStateV1 {
    pub const fn new(campaign_id: Uuid) -> Self {
        Self {
            campaign_id,
            phase: CampaignPhaseV1::Planning,
            active_action_id: None,
            stop_reason: None,
        }
    }

    pub const fn campaign_id(&self) -> Uuid {
        self.campaign_id
    }

    pub const fn phase(&self) -> CampaignPhaseV1 {
        self.phase
    }

    pub const fn active_action_id(&self) -> Option<Uuid> {
        self.active_action_id
    }

    pub const fn stop_reason(&self) -> Option<CampaignStopReasonV1> {
        self.stop_reason
    }

    pub fn activate_action(
        &mut self,
        prepared_action_id: Uuid,
    ) -> Result<(), VerificationCampaignError> {
        if self.campaign_id.is_nil() || prepared_action_id.is_nil() {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_IDENTITY_INVALID",
                "Campaign and Prepared Action ids must be non-nil",
            ));
        }
        if self.active_action_id.is_some() || self.phase != CampaignPhaseV1::Planning {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_ACTIVE_ACTION_CONFLICT",
                "a Campaign can own only one active Prepared Action",
            ));
        }
        self.active_action_id = Some(prepared_action_id);
        self.phase = CampaignPhaseV1::ActionActive;
        Ok(())
    }

    pub fn complete_active_action(
        &mut self,
        prepared_action_id: Uuid,
    ) -> Result<(), VerificationCampaignError> {
        if !matches!(
            self.phase,
            CampaignPhaseV1::ActionActive | CampaignPhaseV1::Stopping
        ) || self.active_action_id != Some(prepared_action_id)
        {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_ACTIVE_ACTION_MISMATCH",
                "only the exact active Prepared Action can close its lane",
            ));
        }
        self.active_action_id = None;
        self.phase = if self.stop_reason.is_some() {
            CampaignPhaseV1::Stopping
        } else {
            CampaignPhaseV1::Planning
        };
        Ok(())
    }

    pub fn request_stop(
        &mut self,
        reason: CampaignStopReasonV1,
    ) -> Result<(), VerificationCampaignError> {
        if self.campaign_id.is_nil() {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_IDENTITY_INVALID",
                "Campaign id must be non-nil",
            ));
        }
        if matches!(
            self.phase,
            CampaignPhaseV1::Draining
                | CampaignPhaseV1::ReadyToTerminalize
                | CampaignPhaseV1::Terminal
        ) {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_INVALID_TRANSITION",
                "draining or terminal Campaign cannot restart stopping",
            ));
        }
        if self.phase == CampaignPhaseV1::Stopping {
            return if self.stop_reason == Some(reason) {
                Ok(())
            } else {
                Err(VerificationCampaignError::new(
                    "VERIFICATION_CAMPAIGN_STOP_REASON_CONFLICT",
                    "Campaign stop reason is immutable",
                ))
            };
        }
        self.stop_reason = Some(reason);
        self.phase = CampaignPhaseV1::Stopping;
        Ok(())
    }

    pub fn begin_draining(&mut self) -> Result<(), VerificationCampaignError> {
        if self.phase != CampaignPhaseV1::Stopping || self.active_action_id.is_some() {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_LOCAL_DRAIN_INCOMPLETE",
                "active Prepared Action must close before local drain",
            ));
        }
        self.phase = CampaignPhaseV1::Draining;
        Ok(())
    }

    pub fn complete_drain(&mut self) -> Result<(), VerificationCampaignError> {
        if self.phase != CampaignPhaseV1::Draining || self.active_action_id.is_some() {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_LOCAL_DRAIN_INCOMPLETE",
                "local action and recovery lane must drain before terminalization",
            ));
        }
        self.phase = CampaignPhaseV1::ReadyToTerminalize;
        Ok(())
    }

    /// Terminality is intentionally Campaign-local. FactDelta consumption and
    /// Registry consolidation are downstream Wave concerns and are not inputs.
    pub fn terminalize(&mut self) -> Result<(), VerificationCampaignError> {
        if self.phase != CampaignPhaseV1::ReadyToTerminalize {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_LOCAL_DRAIN_INCOMPLETE",
                "Campaign cannot terminalize before local drain completes",
            ));
        }
        self.phase = CampaignPhaseV1::Terminal;
        Ok(())
    }
}
