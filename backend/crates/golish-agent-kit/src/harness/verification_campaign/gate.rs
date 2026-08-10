//! Deterministic Campaign Gate checks with stable machine codes.

use std::collections::BTreeSet;

use super::state::CampaignPhaseV1;
use super::types::{
    require_hash, ArtifactAuthorityV1, CampaignCoverageDenominatorSealV1, CampaignGateSnapshotV1,
    CoverageResultStatusV1, CoverageResultV1, GateActionTruthV1, PreparedActionDisposition,
    RoundDispositionV1, VerificationCampaignError, VerificationWaveDenominatorSealV1,
    WaveMemberDispositionV1,
};

pub fn validate_gate_action(action: &GateActionTruthV1) -> Result<(), VerificationCampaignError> {
    if action.prepared_action_id.is_nil() {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_ACTION_ID_INVALID",
            "Prepared Action id must be non-nil",
        ));
    }
    if action.authority != ArtifactAuthorityV1::Canonical {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_SHADOW_AUTHORITY_FORBIDDEN",
            "shadow artifacts cannot enter Campaign authority",
        ));
    }
    for hash in [
        action.execution_receipt_hash.as_deref(),
        action.oracle_assessment_hash.as_deref(),
        action.residual_risk_hash.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        require_hash(hash).map_err(|_| {
            VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_ARTIFACT_HASH_INVALID",
                "Campaign artifact hash must be canonical",
            )
        })?;
    }

    if action.disposition.forbids_execution() {
        if action.reason_code.is_none() || action.residual_risk_hash.is_none() {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_TERMINAL_RESIDUAL_REQUIRED",
                "non-executed terminal disposition requires typed reason and residual",
            ));
        }
        if action.authorized
            || action.durable_started
            || action.execution_receipt_hash.is_some()
            || action.landed_reconciled
            || action.oracle_assessment_hash.is_some()
        {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_CONDITIONAL_ARTIFACT_FORBIDDEN",
                "non-executed disposition cannot fabricate execution or oracle artifacts",
            ));
        }
        return Ok(());
    }

    if action.durable_started && !action.authorized {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_AUTHORIZATION_REQUIRED",
            "durable begin requires authorization authority",
        ));
    }
    if action.execution_receipt_hash.is_some() && (!action.authorized || !action.durable_started) {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_DURABLE_BEGIN_REQUIRED",
            "execution receipt cannot exist before authorized durable begin",
        ));
    }
    if action.authorized && action.durable_started && action.execution_receipt_hash.is_none() {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_EXECUTION_RECEIPT_REQUIRED",
            "authorized durable begin requires an execution receipt",
        ));
    }
    if action.landed_reconciled && action.execution_receipt_hash.is_none() {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_EXECUTION_RECEIPT_REQUIRED",
            "reconciled execution requires its receipt",
        ));
    }
    if action.landed_reconciled && action.oracle_assessment_hash.is_none() {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_ACTION_ORACLE_REQUIRED",
            "landed and reconciled execution requires deterministic action oracle",
        ));
    }
    if action.oracle_assessment_hash.is_some() && !action.landed_reconciled {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_CONDITIONAL_ARTIFACT_FORBIDDEN",
            "action oracle cannot exist before landed reconciliation",
        ));
    }

    match action.disposition {
        PreparedActionDisposition::Succeeded | PreparedActionDisposition::Failed => {
            if !action.authorized || !action.durable_started || !action.landed_reconciled {
                return Err(VerificationCampaignError::new(
                    "VERIFICATION_CAMPAIGN_EXECUTION_CLOSEOUT_INCOMPLETE",
                    "executed terminal disposition requires reconciled closeout",
                ));
            }
        }
        PreparedActionDisposition::OutcomeUnknown => {
            if action.reason_code.is_none()
                || action.residual_risk_hash.is_none()
                || action.execution_receipt_hash.is_none()
                || action.oracle_assessment_hash.is_some()
                || action.landed_reconciled
            {
                return Err(VerificationCampaignError::new(
                    "VERIFICATION_CAMPAIGN_OUTCOME_UNKNOWN_WITNESS_INVALID",
                    "outcome_unknown requires execution witness and residual but no oracle",
                ));
            }
        }
        PreparedActionDisposition::ManuallyBlocked => {
            if action.reason_code.is_none()
                || action.residual_risk_hash.is_none()
                || !action.authorized
                || !action.durable_started
                || action.execution_receipt_hash.is_none()
                || action.landed_reconciled
                || action.oracle_assessment_hash.is_some()
            {
                return Err(VerificationCampaignError::new(
                    "VERIFICATION_CAMPAIGN_MANUAL_BLOCK_WITNESS_INVALID",
                    "manual block requires the unknown execution witness, receipt and residual",
                ));
            }
        }
        PreparedActionDisposition::CompileRejected
        | PreparedActionDisposition::Denied
        | PreparedActionDisposition::Expired
        | PreparedActionDisposition::Superseded => {}
    }
    Ok(())
}

pub fn validate_round_disposition(
    disposition: &RoundDispositionV1,
    execution_receipt_hash: Option<&String>,
    oracle_assessment_hash: Option<&String>,
) -> Result<(), VerificationCampaignError> {
    match disposition {
        RoundDispositionV1::Continue => Ok(()),
        RoundDispositionV1::NoActionCompilable {
            residual_risk_hash, ..
        } => {
            require_hash(residual_risk_hash).map_err(|_| {
                VerificationCampaignError::new(
                    "VERIFICATION_CAMPAIGN_TERMINAL_RESIDUAL_REQUIRED",
                    "no_action_compilable requires a canonical residual",
                )
            })?;
            if execution_receipt_hash.is_some() || oracle_assessment_hash.is_some() {
                Err(VerificationCampaignError::new(
                    "VERIFICATION_CAMPAIGN_CONDITIONAL_ARTIFACT_FORBIDDEN",
                    "no_action_compilable cannot fabricate execution or oracle",
                ))
            } else {
                Ok(())
            }
        }
        RoundDispositionV1::Stopping {
            residual_risk_hash, ..
        } => {
            require_hash(residual_risk_hash).map_err(|_| {
                VerificationCampaignError::new(
                    "VERIFICATION_CAMPAIGN_TERMINAL_RESIDUAL_REQUIRED",
                    "stopping disposition requires a canonical residual",
                )
            })?;
            Ok(())
        }
    }
}

pub fn validate_coverage_results(
    denominator: &CampaignCoverageDenominatorSealV1,
    results: &[CoverageResultV1],
) -> Result<(), VerificationCampaignError> {
    let expected = denominator
        .members
        .iter()
        .map(|member| member.member_hash.as_str())
        .collect::<BTreeSet<_>>();
    let actual = results
        .iter()
        .map(|result| result.denominator_member_hash.as_str())
        .collect::<BTreeSet<_>>();
    if expected != actual || actual.len() != results.len() {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_COVERAGE_CENSUS_MISMATCH",
            "terminal coverage results must exactly match denominator members",
        ));
    }
    for result in results {
        match &result.status {
            CoverageResultStatusV1::Tested {
                prepared_action_id,
                capability_receipt_hash,
                oracle_assessment_hash,
            } => {
                if prepared_action_id.is_nil()
                    || require_hash(capability_receipt_hash).is_err()
                    || require_hash(oracle_assessment_hash).is_err()
                {
                    return Err(VerificationCampaignError::new(
                        "VERIFICATION_CAMPAIGN_TESTED_COVERAGE_BINDING_INVALID",
                        "tested coverage requires action, capability receipt and oracle",
                    ));
                }
            }
            CoverageResultStatusV1::Untested { residual_risk_hash }
            | CoverageResultStatusV1::Degraded { residual_risk_hash }
            | CoverageResultStatusV1::Blocked { residual_risk_hash } => {
                if require_hash(residual_risk_hash).is_err() {
                    return Err(VerificationCampaignError::new(
                        "VERIFICATION_CAMPAIGN_RESIDUAL_BINDING_INVALID",
                        "non-tested coverage requires a canonical residual",
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn validate_wave_partition(
    wave: &VerificationWaveDenominatorSealV1,
    dispositions: &[WaveMemberDispositionV1],
) -> Result<(), VerificationCampaignError> {
    let expected = wave
        .members
        .iter()
        .map(|member| member.member_hash.as_str())
        .collect::<BTreeSet<_>>();
    let actual = dispositions
        .iter()
        .map(WaveMemberDispositionV1::wave_member_hash)
        .collect::<BTreeSet<_>>();
    if expected != actual || actual.len() != dispositions.len() {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_WAVE_PARTITION_MISMATCH",
            "each Wave member requires exactly one Campaign or unassigned residual",
        ));
    }
    for disposition in dispositions {
        let hash = match disposition {
            WaveMemberDispositionV1::Campaign {
                campaign_denominator_hash,
                ..
            } => campaign_denominator_hash,
            WaveMemberDispositionV1::Unassigned {
                residual_risk_hash, ..
            } => residual_risk_hash,
        };
        require_hash(hash).map_err(|_| {
            VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_WAVE_PARTITION_BINDING_INVALID",
                "Wave disposition binding must use a canonical hash",
            )
        })?;
    }
    Ok(())
}

pub fn validate_campaign_gate(
    snapshot: &CampaignGateSnapshotV1,
) -> Result<(), VerificationCampaignError> {
    if snapshot.authority != ArtifactAuthorityV1::Canonical {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_SHADOW_AUTHORITY_FORBIDDEN",
            "shadow evaluation cannot enter Campaign Gate",
        ));
    }
    let active_actions = snapshot
        .actions
        .iter()
        .filter(|action| {
            action.authorized
                && action.durable_started
                && !action.landed_reconciled
                && action.disposition != PreparedActionDisposition::OutcomeUnknown
                && action.disposition != PreparedActionDisposition::ManuallyBlocked
        })
        .count();
    if active_actions > 1 {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_ACTIVE_ACTION_CONFLICT",
            "Campaign Gate observed more than one active Prepared Action",
        ));
    }
    for action in &snapshot.actions {
        validate_gate_action(action)?;
    }
    if snapshot.phase == CampaignPhaseV1::Terminal {
        let denominator = snapshot.denominator.as_ref().ok_or_else(|| {
            VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_DENOMINATOR_REQUIRED",
                "terminal Campaign requires a sealed coverage denominator",
            )
        })?;
        validate_coverage_results(denominator, &snapshot.coverage_results)?;
        if snapshot.fact_delta_bundle_count != 1 {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_FACT_DELTA_EXACT_ONE_REQUIRED",
                "terminal Campaign writes exactly one immutable FactDelta bundle",
            ));
        }
        // `fact_delta_consumed` is deliberately ignored: consumption belongs
        // to Wave consolidation and cannot keep the Campaign lane open.
    }
    Ok(())
}
