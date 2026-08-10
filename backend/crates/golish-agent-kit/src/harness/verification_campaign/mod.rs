//! Objective-local Verification Campaign domain contract.
//!
//! The Campaign reducer owns action/oracle/coverage truth for exactly one
//! Plan B objective. It intentionally re-exports Plan B's sealed outer reducer
//! instead of defining a second revision truth table.

pub mod gate;
pub mod oracle;
pub mod state;
pub mod types;

pub use gate::{
    validate_campaign_gate, validate_coverage_results, validate_gate_action,
    validate_round_disposition, validate_wave_partition,
};
pub use oracle::{adjudicate_campaign, reduce_action_oracle};
pub use state::{CampaignPhaseV1, CampaignStateV1, CampaignStopReasonV1};
pub use types::*;

pub use golish_core::hypothesis_verification::reduce_verification_plan_v1 as adjudicate_hypothesis_revision;
pub type HypothesisRevisionOutcome =
    golish_core::hypothesis_verification::HypothesisRevisionAdjudicationVerdictV1;

pub type ActionOracleContract = ActionOracleContractV1;
pub type ReconciledExecutionReceipt = ReconciledExecutionReceiptV1;
pub type ActionOracleAssessment = ActionOracleAssessmentV1;
pub type OracleCensus = OracleCensusV1;
pub type ObligationDispositionSet = ObligationDispositionSetV1;
pub type CampaignAdjudication = CampaignAdjudicationV1;

#[cfg(test)]
mod tests;
