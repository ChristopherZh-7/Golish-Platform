//! Whole-record rollout selector for legacy and Candidate V2 attack truth.
//!
//! The selected semantic record is intentionally opaque and atomic. Callers
//! cannot combine legacy decisions with V2 review counts (or the inverse): a
//! selection always owns exactly one complete source record.

use golish_core::AttackExecutionContract;

use super::AttackExecutionError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttackDecisionSemanticKind {
    Candidate,
    NoCandidate,
}

/// One canonical semantic decision used only for rollout comparison.
///
/// `semantic_hash` must cover the complete decision payload at the adapter
/// boundary. The selector treats it as opaque and never interprets model prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackDecisionSemantic {
    work_item_key: String,
    kind: AttackDecisionSemanticKind,
    semantic_hash: String,
}

impl AttackDecisionSemantic {
    pub fn try_new(
        work_item_key: impl Into<String>,
        kind: AttackDecisionSemanticKind,
        semantic_hash: impl Into<String>,
    ) -> Result<Self, AttackExecutionError> {
        let work_item_key = work_item_key.into();
        let semantic_hash = semantic_hash.into();
        if work_item_key.trim().is_empty() || semantic_hash.trim().is_empty() {
            return Err(AttackExecutionError::new(
                "ATTACK_READ_DECISION_INVALID",
                "semantic decision requires a work-item key and canonical hash",
            ));
        }
        Ok(Self {
            work_item_key,
            kind,
            semantic_hash,
        })
    }

    pub fn work_item_key(&self) -> &str {
        &self.work_item_key
    }

    pub const fn kind(&self) -> AttackDecisionSemanticKind {
        self.kind
    }

    pub fn semantic_hash(&self) -> &str {
        &self.semantic_hash
    }
}

/// Aggregate counts independently loaded from the authoritative read source.
/// Candidate/no-Candidate counts are checked against the loaded child set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackReviewCounts {
    wave_unit_count: u32,
    review_closed_unit_count: u32,
    candidate_decision_count: u32,
    no_candidate_decision_count: u32,
}

impl AttackReviewCounts {
    pub const fn new(
        wave_unit_count: u32,
        review_closed_unit_count: u32,
        candidate_decision_count: u32,
        no_candidate_decision_count: u32,
    ) -> Self {
        Self {
            wave_unit_count,
            review_closed_unit_count,
            candidate_decision_count,
            no_candidate_decision_count,
        }
    }

    pub const fn wave_unit_count(self) -> u32 {
        self.wave_unit_count
    }

    pub const fn review_closed_unit_count(self) -> u32 {
        self.review_closed_unit_count
    }

    pub const fn candidate_decision_count(self) -> u32 {
        self.candidate_decision_count
    }

    pub const fn no_candidate_decision_count(self) -> u32 {
        self.no_candidate_decision_count
    }
}

/// A fully loaded, internally consistent semantic record.
///
/// Fields stay private so an adapter must construct the entire record before
/// selection. Equality is semantic: decision row order is canonicalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteAttackRead {
    decisions: Vec<AttackDecisionSemantic>,
    review_counts: AttackReviewCounts,
}

impl CompleteAttackRead {
    pub fn try_new(
        mut decisions: Vec<AttackDecisionSemantic>,
        review_counts: AttackReviewCounts,
    ) -> Result<Self, AttackExecutionError> {
        if review_counts.review_closed_unit_count > review_counts.wave_unit_count {
            return Err(AttackExecutionError::new(
                "ATTACK_READ_REVIEW_COUNTS_INVALID",
                "closed review-unit count exceeds the frozen wave-unit count",
            ));
        }

        decisions.sort_by(|left, right| {
            left.work_item_key
                .cmp(&right.work_item_key)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.semantic_hash.cmp(&right.semantic_hash))
        });
        if decisions
            .windows(2)
            .any(|pair| pair[0].work_item_key == pair[1].work_item_key)
        {
            return Err(AttackExecutionError::new(
                "ATTACK_READ_DECISION_DUPLICATE",
                "one work item must have exactly one semantic decision",
            ));
        }

        let candidate_count = decisions
            .iter()
            .filter(|decision| decision.kind == AttackDecisionSemanticKind::Candidate)
            .count();
        let no_candidate_count = decisions.len() - candidate_count;
        if usize::try_from(review_counts.candidate_decision_count).ok() != Some(candidate_count)
            || usize::try_from(review_counts.no_candidate_decision_count).ok()
                != Some(no_candidate_count)
        {
            return Err(AttackExecutionError::new(
                "ATTACK_READ_INCOMPLETE",
                "decision children do not match authoritative review counts",
            ));
        }

        Ok(Self {
            decisions,
            review_counts,
        })
    }

    pub fn decisions(&self) -> &[AttackDecisionSemantic] {
        &self.decisions
    }

    pub const fn review_counts(&self) -> AttackReviewCounts {
        self.review_counts
    }
}

/// State of the entire V2 semantic read. Missing and incomplete are distinct
/// for adapters/diagnostics, but both are one `V2Missing` shadow outcome and
/// both fail closed under `V2Only`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V2AttackRead {
    Complete(CompleteAttackRead),
    Missing,
    Incomplete,
}

impl V2AttackRead {
    pub fn from_parts(
        decisions: Vec<AttackDecisionSemantic>,
        review_counts: AttackReviewCounts,
    ) -> Self {
        match CompleteAttackRead::try_new(decisions, review_counts) {
            Ok(record) => Self::Complete(record),
            Err(_) => Self::Incomplete,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackReadSource {
    Legacy,
    V2,
    LegacyFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackShadowComparison {
    Match,
    Mismatch,
    V2Missing,
}

/// Atomic authoritative selection. The private `record` field is cloned or
/// moved only as a unit, never assembled field-by-field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackReadSelection {
    source: AttackReadSource,
    record: CompleteAttackRead,
    shadow_comparison: Option<AttackShadowComparison>,
    executes_v2_verifier: bool,
}

impl AttackReadSelection {
    pub const fn source(&self) -> AttackReadSource {
        self.source
    }

    pub const fn record(&self) -> &CompleteAttackRead {
        &self.record
    }

    pub const fn shadow_comparison(&self) -> Option<AttackShadowComparison> {
        self.shadow_comparison
    }

    pub const fn executes_v2_verifier(&self) -> bool {
        self.executes_v2_verifier
    }

    pub fn into_record(self) -> CompleteAttackRead {
        self.record
    }
}

fn require_legacy(
    legacy: Option<CompleteAttackRead>,
) -> Result<CompleteAttackRead, AttackExecutionError> {
    legacy.ok_or_else(|| {
        AttackExecutionError::new(
            "ATTACK_LEGACY_READ_REQUIRED",
            "the persisted attack contract requires a complete legacy read",
        )
    })
}

fn shadow_comparison(
    legacy: Option<&CompleteAttackRead>,
    v2: &V2AttackRead,
) -> AttackShadowComparison {
    match (legacy, v2) {
        (Some(legacy), V2AttackRead::Complete(v2)) if legacy == v2 => AttackShadowComparison::Match,
        (_, V2AttackRead::Complete(_)) => AttackShadowComparison::Mismatch,
        (_, V2AttackRead::Missing | V2AttackRead::Incomplete) => AttackShadowComparison::V2Missing,
    }
}

/// Select one complete authoritative attack record using the operation-frozen
/// rollout contract. Dual modes compare whole records; `V2Only` never falls
/// back to a legacy field or record.
pub fn select_attack_read(
    contract: AttackExecutionContract,
    legacy: Option<CompleteAttackRead>,
    v2: V2AttackRead,
) -> Result<AttackReadSelection, AttackExecutionError> {
    let executes_v2_verifier = contract.executes_v2_verifier();
    let (source, record, comparison) = match contract {
        AttackExecutionContract::Legacy => {
            (AttackReadSource::Legacy, require_legacy(legacy)?, None)
        }
        AttackExecutionContract::DualWriteReadLegacy => {
            let comparison = shadow_comparison(legacy.as_ref(), &v2);
            (
                AttackReadSource::Legacy,
                require_legacy(legacy)?,
                Some(comparison),
            )
        }
        AttackExecutionContract::DualWriteReadV2Fallback => {
            let comparison = shadow_comparison(legacy.as_ref(), &v2);
            match v2 {
                V2AttackRead::Complete(record) => (AttackReadSource::V2, record, Some(comparison)),
                V2AttackRead::Missing | V2AttackRead::Incomplete => (
                    AttackReadSource::LegacyFallback,
                    require_legacy(legacy)?,
                    Some(comparison),
                ),
            }
        }
        AttackExecutionContract::V2Only => match v2 {
            V2AttackRead::Complete(record) => (AttackReadSource::V2, record, None),
            V2AttackRead::Missing | V2AttackRead::Incomplete => {
                return Err(AttackExecutionError::new(
                    "ATTACK_V2_READ_REQUIRED",
                    "v2_only requires one complete V2 semantic record",
                ))
            }
        },
    };

    Ok(AttackReadSelection {
        source,
        record,
        shadow_comparison: comparison,
        executes_v2_verifier,
    })
}
