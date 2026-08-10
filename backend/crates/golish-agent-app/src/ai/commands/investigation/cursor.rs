use std::collections::BTreeSet;
use std::fmt::Write as _;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const INVESTIGATION_CURSOR_VERSION: u8 = 2;
pub(crate) const INVESTIGATION_PROJECTION_SCHEMA_VERSION: u32 = 1;
pub(crate) const INVESTIGATION_CURSOR_INVALID: &str = "INVESTIGATION_CURSOR_INVALID";
pub(crate) const INVESTIGATION_PROJECTION_STALE: &str = "INVESTIGATION_PROJECTION_STALE";

const MAX_CURSOR_TOKEN_BYTES: usize = 16 * 1024;
const MAX_CURSOR_PAYLOAD_BYTES: usize = 12 * 1024;
const HMAC_SHA256_BYTES: usize = 32;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvestigationCursorV2 {
    pub(crate) version: u8,
    pub(crate) resource_kind: String,
    pub(crate) operation_id: Uuid,
    pub(crate) projection_schema_version: u32,
    pub(crate) as_of_change_seq: i64,
    pub(crate) as_of_temporal_cutoff: DateTime<Utc>,
    pub(crate) authority_epoch_set_hash: String,
    pub(crate) earliest_effective_valid_until: DateTime<Utc>,
    pub(crate) tool_truth_contract: String,
    pub(crate) investigation_contract_version: String,
    pub(crate) investigation_rollout_mode: String,
    pub(crate) filter_digest: String,
    pub(crate) page_size: u32,
    pub(crate) stable_sort_key: InvestigationStableSortKeyV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvestigationCursorV1Legacy {
    pub(crate) version: u8,
    pub(crate) resource_kind: String,
    pub(crate) operation_id: Uuid,
    pub(crate) projection_schema_version: u32,
    pub(crate) as_of_change_seq: i64,
    pub(crate) tool_truth_contract: String,
    pub(crate) investigation_contract_version: String,
    pub(crate) investigation_rollout_mode: String,
    pub(crate) filter_digest: String,
    pub(crate) page_size: u32,
    pub(crate) stable_sort_key: InvestigationStableSortKeyV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum InvestigationStableSortKeyV1 {
    Hypothesis {
        organization_ordinal: i32,
        group_key: String,
        readiness_rank: i16,
        epistemic_rank: i16,
        root_id: Uuid,
        revision_ordinal: i32,
    },
    Campaign {
        wave_ordinal: i64,
        campaign_ordinal: i64,
        campaign_id: Uuid,
    },
    Timeline {
        change_seq: i64,
        event_id: Uuid,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InvestigationCursorTemporalBinding {
    pub(crate) as_of_change_seq: i64,
    pub(crate) as_of_temporal_cutoff: DateTime<Utc>,
    pub(crate) authority_epoch_set_hash: String,
    pub(crate) earliest_effective_valid_until: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub(crate) struct InvestigationCursorBinding<'a> {
    pub(crate) resource_kind: &'a str,
    pub(crate) operation_id: Uuid,
    pub(crate) tool_truth_contract: &'a str,
    pub(crate) investigation_contract_version: &'a str,
    pub(crate) investigation_rollout_mode: &'a str,
    pub(crate) filter_digest: &'a str,
    pub(crate) page_size: u32,
    /// When an outer trusted envelope already supplied the frozen first-page
    /// snapshot, every field must match the signed cursor exactly.
    pub(crate) expected_temporal: Option<&'a InvestigationCursorTemporalBinding>,
}

#[derive(Clone, Debug)]
pub(crate) struct InvestigationCursorCurrentAuthority<'a> {
    /// Captured from the same read-only DB transaction as `db_now` and epoch.
    pub(crate) current_change_seq: i64,
    pub(crate) db_now: DateTime<Utc>,
    pub(crate) current_authority_epoch_set_hash: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InvestigationCursorFailure {
    Invalid,
    Stale,
}

impl InvestigationCursorFailure {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Invalid => INVESTIGATION_CURSOR_INVALID,
            Self::Stale => INVESTIGATION_PROJECTION_STALE,
        }
    }

    pub(crate) const fn restart_required(self) -> bool {
        matches!(self, Self::Stale)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VerifiedInvestigationCursor {
    Current(InvestigationCursorV2),
    Historical(InvestigationCursorV1Legacy),
}

impl InvestigationCursorV2 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        resource_kind: impl Into<String>,
        operation_id: Uuid,
        temporal: InvestigationCursorTemporalBinding,
        tool_truth_contract: impl Into<String>,
        investigation_contract_version: impl Into<String>,
        investigation_rollout_mode: impl Into<String>,
        filter_digest: impl Into<String>,
        requested_page_size: u32,
        stable_sort_key: InvestigationStableSortKeyV1,
    ) -> Result<Self, InvestigationCursorFailure> {
        let cursor = Self {
            version: INVESTIGATION_CURSOR_VERSION,
            resource_kind: resource_kind.into(),
            operation_id,
            projection_schema_version: INVESTIGATION_PROJECTION_SCHEMA_VERSION,
            as_of_change_seq: temporal.as_of_change_seq,
            as_of_temporal_cutoff: temporal.as_of_temporal_cutoff,
            authority_epoch_set_hash: temporal.authority_epoch_set_hash,
            earliest_effective_valid_until: temporal.earliest_effective_valid_until,
            tool_truth_contract: tool_truth_contract.into(),
            investigation_contract_version: investigation_contract_version.into(),
            investigation_rollout_mode: investigation_rollout_mode.into(),
            filter_digest: filter_digest.into(),
            page_size: clamp_investigation_page_size(requested_page_size),
            stable_sort_key,
        };
        validate_v2_shape(&cursor)?;
        Ok(cursor)
    }

    #[cfg(test)]
    pub(crate) fn temporal_binding(&self) -> InvestigationCursorTemporalBinding {
        InvestigationCursorTemporalBinding {
            as_of_change_seq: self.as_of_change_seq,
            as_of_temporal_cutoff: self.as_of_temporal_cutoff,
            authority_epoch_set_hash: self.authority_epoch_set_hash.clone(),
            earliest_effective_valid_until: self.earliest_effective_valid_until,
        }
    }
}

pub(crate) fn clamp_investigation_page_size(requested: u32) -> u32 {
    requested.clamp(1, 100)
}

pub(crate) fn canonical_filter_digest<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("canonical filter value is serializable");
    let digest = Sha256::digest(bytes);
    let mut result = String::with_capacity("sha256:".len() + digest.len() * 2);
    result.push_str("sha256:");
    for byte in digest {
        write!(result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

pub(crate) fn issue_current_cursor(
    cursor: &InvestigationCursorV2,
    cursor_salt: &[u8; HMAC_SHA256_BYTES],
) -> Result<String, InvestigationCursorFailure> {
    validate_v2_shape(cursor)?;
    canonical_sign(cursor, cursor_salt)
}

/// Verifies and decodes either cursor generation. This is the only entrypoint
/// for historical readers; there is intentionally no V1 issuing API.
pub(crate) fn decode_historical_cursor(
    token: &str,
    cursor_salt: &[u8; HMAC_SHA256_BYTES],
    binding: &InvestigationCursorBinding<'_>,
) -> Result<VerifiedInvestigationCursor, InvestigationCursorFailure> {
    let payload = verify_and_decode_payload(token, cursor_salt)?;
    let version = serde_json::from_slice::<CursorVersion>(&payload)
        .map_err(|_| InvestigationCursorFailure::Invalid)?
        .version;

    match version {
        INVESTIGATION_CURSOR_VERSION => {
            let cursor = decode_canonical::<InvestigationCursorV2>(&payload)?;
            validate_v2_shape(&cursor)?;
            validate_common_binding(&cursor.common(), binding)?;
            validate_expected_temporal(&cursor, binding.expected_temporal)?;
            Ok(VerifiedInvestigationCursor::Current(cursor))
        }
        1 => {
            let cursor = decode_canonical::<InvestigationCursorV1Legacy>(&payload)?;
            validate_v1_shape(&cursor)?;
            validate_common_binding(&cursor.common(), binding)?;
            if binding.expected_temporal.is_some() {
                return Err(InvestigationCursorFailure::Invalid);
            }
            Ok(VerifiedInvestigationCursor::Historical(cursor))
        }
        _ => Err(InvestigationCursorFailure::Invalid),
    }
}

/// Current Registry pagination may consume only V2. A valid V1 token is a
/// typed restart, never an upgrade or a newly signed continuation.
pub(crate) fn continue_current_cursor(
    token: &str,
    cursor_salt: &[u8; HMAC_SHA256_BYTES],
    binding: &InvestigationCursorBinding<'_>,
    authority: &InvestigationCursorCurrentAuthority<'_>,
) -> Result<InvestigationCursorV2, InvestigationCursorFailure> {
    let cursor = match decode_historical_cursor(token, cursor_salt, binding)? {
        VerifiedInvestigationCursor::Current(cursor) => cursor,
        VerifiedInvestigationCursor::Historical(_) => {
            return Err(InvestigationCursorFailure::Stale)
        }
    };

    // A signed cursor from the future or one whose frozen temporal interval is
    // internally impossible is invalid, rather than a normal stale snapshot.
    if authority.db_now < cursor.as_of_temporal_cutoff {
        return Err(InvestigationCursorFailure::Invalid);
    }
    if authority.current_change_seq != cursor.as_of_change_seq
        || authority.current_authority_epoch_set_hash != cursor.authority_epoch_set_hash
        || authority.db_now > cursor.earliest_effective_valid_until
    {
        return Err(InvestigationCursorFailure::Stale);
    }

    Ok(cursor)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InvestigationFilterField {
    EpistemicState,
    ReadinessState,
    CapabilityState,
    SourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvestigationFilterConflict<'a> {
    pub(crate) left_field: InvestigationFilterField,
    pub(crate) left_value: &'a str,
    pub(crate) right_field: InvestigationFilterField,
    pub(crate) right_value: &'a str,
}

/// The read-model owner supplies its closed catalogs here. This keeps String
/// IPC fields from becoming an open enum and gives future Plan D filters one
/// shared mutual-exclusion seam without defining mirror wire enums.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InvestigationFilterPolicy<'a> {
    pub(crate) epistemic_states: &'a [&'a str],
    pub(crate) readiness_states: &'a [&'a str],
    pub(crate) capability_states: &'a [&'a str],
    pub(crate) source_kinds: &'a [&'a str],
    pub(crate) conflicts: &'a [InvestigationFilterConflict<'a>],
}

#[derive(Clone, Debug)]
pub(crate) struct InvestigationFilterInput<'a> {
    pub(crate) organization_ids: &'a [Uuid],
    pub(crate) epistemic_states: &'a [String],
    pub(crate) readiness_states: &'a [String],
    pub(crate) capability_states: &'a [String],
    pub(crate) source_kinds: &'a [String],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CanonicalInvestigationFilters {
    organization_ids: Vec<Uuid>,
    epistemic_states: Vec<String>,
    readiness_states: Vec<String>,
    capability_states: Vec<String>,
    source_kinds: Vec<String>,
}

impl CanonicalInvestigationFilters {
    pub(crate) fn digest(&self) -> String {
        canonical_filter_digest(self)
    }

    pub(crate) fn organization_ids(&self) -> &[Uuid] {
        &self.organization_ids
    }

    pub(crate) fn epistemic_states(&self) -> &[String] {
        &self.epistemic_states
    }

    pub(crate) fn readiness_states(&self) -> &[String] {
        &self.readiness_states
    }

    pub(crate) fn capability_states(&self) -> &[String] {
        &self.capability_states
    }

    pub(crate) fn source_kinds(&self) -> &[String] {
        &self.source_kinds
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InvestigationFilterFailure {
    UnknownValue {
        field: InvestigationFilterField,
        value: String,
    },
    MutuallyExclusive {
        left_field: InvestigationFilterField,
        left_value: String,
        right_field: InvestigationFilterField,
        right_value: String,
    },
}

pub(crate) fn canonicalize_investigation_filters(
    input: InvestigationFilterInput<'_>,
    policy: InvestigationFilterPolicy<'_>,
) -> Result<CanonicalInvestigationFilters, InvestigationFilterFailure> {
    let organization_ids = input
        .organization_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let epistemic_states = canonical_filter_values(
        InvestigationFilterField::EpistemicState,
        input.epistemic_states,
        policy.epistemic_states,
    )?;
    let readiness_states = canonical_filter_values(
        InvestigationFilterField::ReadinessState,
        input.readiness_states,
        policy.readiness_states,
    )?;
    let capability_states = canonical_filter_values(
        InvestigationFilterField::CapabilityState,
        input.capability_states,
        policy.capability_states,
    )?;
    let source_kinds = canonical_filter_values(
        InvestigationFilterField::SourceKind,
        input.source_kinds,
        policy.source_kinds,
    )?;

    let canonical = CanonicalInvestigationFilters {
        organization_ids,
        epistemic_states,
        readiness_states,
        capability_states,
        source_kinds,
    };
    for conflict in policy.conflicts {
        if canonical.contains(conflict.left_field, conflict.left_value)
            && canonical.contains(conflict.right_field, conflict.right_value)
        {
            return Err(InvestigationFilterFailure::MutuallyExclusive {
                left_field: conflict.left_field,
                left_value: conflict.left_value.to_string(),
                right_field: conflict.right_field,
                right_value: conflict.right_value.to_string(),
            });
        }
    }
    Ok(canonical)
}

impl CanonicalInvestigationFilters {
    fn contains(&self, field: InvestigationFilterField, value: &str) -> bool {
        let values = match field {
            InvestigationFilterField::EpistemicState => &self.epistemic_states,
            InvestigationFilterField::ReadinessState => &self.readiness_states,
            InvestigationFilterField::CapabilityState => &self.capability_states,
            InvestigationFilterField::SourceKind => &self.source_kinds,
        };
        values
            .binary_search_by(|candidate| candidate.as_str().cmp(value))
            .is_ok()
    }
}

fn canonical_filter_values(
    field: InvestigationFilterField,
    values: &[String],
    allowed: &[&str],
) -> Result<Vec<String>, InvestigationFilterFailure> {
    let mut canonical = BTreeSet::new();
    for value in values {
        if !allowed.contains(&value.as_str()) {
            return Err(InvestigationFilterFailure::UnknownValue {
                field,
                value: value.clone(),
            });
        }
        canonical.insert(value.clone());
    }
    Ok(canonical.into_iter().collect())
}

#[derive(Deserialize)]
struct CursorVersion {
    version: u8,
}

#[derive(Clone, Copy)]
struct CursorCommon<'a> {
    resource_kind: &'a str,
    operation_id: Uuid,
    projection_schema_version: u32,
    tool_truth_contract: &'a str,
    investigation_contract_version: &'a str,
    investigation_rollout_mode: &'a str,
    filter_digest: &'a str,
    page_size: u32,
    stable_sort_key: &'a InvestigationStableSortKeyV1,
}

impl InvestigationCursorV2 {
    fn common(&self) -> CursorCommon<'_> {
        CursorCommon {
            resource_kind: &self.resource_kind,
            operation_id: self.operation_id,
            projection_schema_version: self.projection_schema_version,
            tool_truth_contract: &self.tool_truth_contract,
            investigation_contract_version: &self.investigation_contract_version,
            investigation_rollout_mode: &self.investigation_rollout_mode,
            filter_digest: &self.filter_digest,
            page_size: self.page_size,
            stable_sort_key: &self.stable_sort_key,
        }
    }
}

impl InvestigationCursorV1Legacy {
    fn common(&self) -> CursorCommon<'_> {
        CursorCommon {
            resource_kind: &self.resource_kind,
            operation_id: self.operation_id,
            projection_schema_version: self.projection_schema_version,
            tool_truth_contract: &self.tool_truth_contract,
            investigation_contract_version: &self.investigation_contract_version,
            investigation_rollout_mode: &self.investigation_rollout_mode,
            filter_digest: &self.filter_digest,
            page_size: self.page_size,
            stable_sort_key: &self.stable_sort_key,
        }
    }
}

fn validate_v2_shape(cursor: &InvestigationCursorV2) -> Result<(), InvestigationCursorFailure> {
    if cursor.version != INVESTIGATION_CURSOR_VERSION
        || cursor.as_of_change_seq < 0
        || cursor.as_of_temporal_cutoff > cursor.earliest_effective_valid_until
    {
        return Err(InvestigationCursorFailure::Invalid);
    }
    validate_common_shape(&cursor.common())
}

fn validate_v1_shape(
    cursor: &InvestigationCursorV1Legacy,
) -> Result<(), InvestigationCursorFailure> {
    if cursor.version != 1 || cursor.as_of_change_seq < 0 {
        return Err(InvestigationCursorFailure::Invalid);
    }
    validate_common_shape(&cursor.common())
}

fn validate_common_shape(common: &CursorCommon<'_>) -> Result<(), InvestigationCursorFailure> {
    if common.projection_schema_version != INVESTIGATION_PROJECTION_SCHEMA_VERSION
        || !(1..=100).contains(&common.page_size)
        || common.resource_kind.is_empty()
        || common.tool_truth_contract.is_empty()
        || common.investigation_contract_version.is_empty()
        || common.investigation_rollout_mode.is_empty()
        || common.filter_digest.is_empty()
        || !stable_key_matches_resource(common.resource_kind, common.stable_sort_key)
    {
        return Err(InvestigationCursorFailure::Invalid);
    }
    Ok(())
}

fn stable_key_matches_resource(resource_kind: &str, key: &InvestigationStableSortKeyV1) -> bool {
    matches!(
        (resource_kind, key),
        (
            "hypotheses",
            InvestigationStableSortKeyV1::Hypothesis { .. }
        ) | ("campaigns", InvestigationStableSortKeyV1::Campaign { .. })
            | ("timeline", InvestigationStableSortKeyV1::Timeline { .. })
    )
}

fn validate_common_binding(
    common: &CursorCommon<'_>,
    binding: &InvestigationCursorBinding<'_>,
) -> Result<(), InvestigationCursorFailure> {
    if common.resource_kind != binding.resource_kind
        || common.operation_id != binding.operation_id
        || common.tool_truth_contract != binding.tool_truth_contract
        || common.investigation_contract_version != binding.investigation_contract_version
        || common.investigation_rollout_mode != binding.investigation_rollout_mode
        || common.filter_digest != binding.filter_digest
        || common.page_size != clamp_investigation_page_size(binding.page_size)
    {
        return Err(InvestigationCursorFailure::Invalid);
    }
    Ok(())
}

fn validate_expected_temporal(
    cursor: &InvestigationCursorV2,
    expected: Option<&InvestigationCursorTemporalBinding>,
) -> Result<(), InvestigationCursorFailure> {
    if let Some(expected) = expected {
        if cursor.as_of_change_seq != expected.as_of_change_seq
            || cursor.as_of_temporal_cutoff != expected.as_of_temporal_cutoff
            || cursor.authority_epoch_set_hash != expected.authority_epoch_set_hash
            || cursor.earliest_effective_valid_until != expected.earliest_effective_valid_until
        {
            return Err(InvestigationCursorFailure::Invalid);
        }
    }
    Ok(())
}

fn canonical_sign<T: Serialize>(
    payload: &T,
    cursor_salt: &[u8; HMAC_SHA256_BYTES],
) -> Result<String, InvestigationCursorFailure> {
    let payload = serde_json::to_vec(payload).map_err(|_| InvestigationCursorFailure::Invalid)?;
    if payload.len() > MAX_CURSOR_PAYLOAD_BYTES {
        return Err(InvestigationCursorFailure::Invalid);
    }
    let mut mac =
        HmacSha256::new_from_slice(cursor_salt).map_err(|_| InvestigationCursorFailure::Invalid)?;
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn verify_and_decode_payload(
    token: &str,
    cursor_salt: &[u8; HMAC_SHA256_BYTES],
) -> Result<Vec<u8>, InvestigationCursorFailure> {
    if token.is_empty() || token.len() > MAX_CURSOR_TOKEN_BYTES {
        return Err(InvestigationCursorFailure::Invalid);
    }
    let mut parts = token.split('.');
    let encoded_payload = parts.next().ok_or(InvestigationCursorFailure::Invalid)?;
    let encoded_signature = parts.next().ok_or(InvestigationCursorFailure::Invalid)?;
    if encoded_payload.is_empty() || encoded_signature.is_empty() || parts.next().is_some() {
        return Err(InvestigationCursorFailure::Invalid);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(encoded_payload)
        .map_err(|_| InvestigationCursorFailure::Invalid)?;
    let signature = URL_SAFE_NO_PAD
        .decode(encoded_signature)
        .map_err(|_| InvestigationCursorFailure::Invalid)?;
    if payload.len() > MAX_CURSOR_PAYLOAD_BYTES || signature.len() != HMAC_SHA256_BYTES {
        return Err(InvestigationCursorFailure::Invalid);
    }

    let mut mac =
        HmacSha256::new_from_slice(cursor_salt).map_err(|_| InvestigationCursorFailure::Invalid)?;
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| InvestigationCursorFailure::Invalid)?;
    Ok(payload)
}

fn decode_canonical<T>(payload: &[u8]) -> Result<T, InvestigationCursorFailure>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let decoded =
        serde_json::from_slice::<T>(payload).map_err(|_| InvestigationCursorFailure::Invalid)?;
    let canonical =
        serde_json::to_vec(&decoded).map_err(|_| InvestigationCursorFailure::Invalid)?;
    if canonical != payload {
        return Err(InvestigationCursorFailure::Invalid);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    const SALT: [u8; 32] = [0x5a; 32];

    fn time(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 31, 1, 2, second)
            .single()
            .expect("fixture timestamp")
    }

    fn operation_id() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000042").expect("fixture UUID")
    }

    fn cursor() -> InvestigationCursorV2 {
        InvestigationCursorV2::new(
            "hypotheses",
            operation_id(),
            InvestigationCursorTemporalBinding {
                as_of_change_seq: 17,
                as_of_temporal_cutoff: time(10),
                authority_epoch_set_hash: format!("sha256:{}", "a".repeat(64)),
                earliest_effective_valid_until: time(50),
            },
            "tool_truth_receipt_v1",
            "hypothesis_registry_v1",
            "new_only",
            format!("sha256:{}", "b".repeat(64)),
            250,
            InvestigationStableSortKeyV1::Hypothesis {
                organization_ordinal: 0,
                group_key: "group-a".to_string(),
                readiness_rank: 1,
                epistemic_rank: 2,
                root_id: Uuid::parse_str("00000000-0000-0000-0000-000000000043")
                    .expect("fixture UUID"),
                revision_ordinal: 3,
            },
        )
        .expect("valid cursor")
    }

    fn binding<'a>(cursor: &'a InvestigationCursorV2) -> InvestigationCursorBinding<'a> {
        InvestigationCursorBinding {
            resource_kind: &cursor.resource_kind,
            operation_id: cursor.operation_id,
            tool_truth_contract: &cursor.tool_truth_contract,
            investigation_contract_version: &cursor.investigation_contract_version,
            investigation_rollout_mode: &cursor.investigation_rollout_mode,
            filter_digest: &cursor.filter_digest,
            page_size: cursor.page_size,
            expected_temporal: None,
        }
    }

    fn authority(cursor: &InvestigationCursorV2) -> InvestigationCursorCurrentAuthority<'_> {
        InvestigationCursorCurrentAuthority {
            current_change_seq: cursor.as_of_change_seq,
            db_now: time(20),
            current_authority_epoch_set_hash: &cursor.authority_epoch_set_hash,
        }
    }

    fn tamper_signed_field(token: &str, field: &str, replacement: serde_json::Value) -> String {
        let (payload, signature) = token.split_once('.').expect("signed token");
        let decoded = URL_SAFE_NO_PAD.decode(payload).expect("base64 payload");
        let mut value: serde_json::Value =
            serde_json::from_slice(&decoded).expect("JSON cursor payload");
        value
            .as_object_mut()
            .expect("cursor object")
            .insert(field.to_owned(), replacement);
        let tampered = serde_json::to_vec(&value).expect("tampered JSON");
        format!("{}.{}", URL_SAFE_NO_PAD.encode(tampered), signature)
    }

    #[test]
    fn investigation_cursor_v2_round_trip_is_canonical_and_clamps_page_size() {
        let cursor = cursor();
        assert_eq!(cursor.page_size, 100);
        let token = issue_current_cursor(&cursor, &SALT).expect("issue current V2");
        assert!(!token.contains(['+', '/', '=']));

        let decoded =
            continue_current_cursor(&token, &SALT, &binding(&cursor), &authority(&cursor))
                .expect("valid continuation");
        assert_eq!(decoded, cursor);
        assert_eq!(
            issue_current_cursor(&decoded, &SALT).expect("canonical replay"),
            token
        );
    }

    #[test]
    fn investigation_cursor_rejects_tamper_and_all_binding_mismatches() {
        let cursor = cursor();
        let token = issue_current_cursor(&cursor, &SALT).expect("issue current V2");
        let mut tampered = token.clone().into_bytes();
        tampered[2] = if tampered[2] == b'A' { b'B' } else { b'A' };
        assert_eq!(
            continue_current_cursor(
                std::str::from_utf8(&tampered).expect("ASCII token"),
                &SALT,
                &binding(&cursor),
                &authority(&cursor)
            ),
            Err(InvestigationCursorFailure::Invalid)
        );

        let mut wrong = binding(&cursor);
        wrong.resource_kind = "campaigns";
        assert_eq!(
            continue_current_cursor(&token, &SALT, &wrong, &authority(&cursor)),
            Err(InvestigationCursorFailure::Invalid)
        );
        let mut wrong = binding(&cursor);
        wrong.operation_id = Uuid::nil();
        assert_eq!(
            continue_current_cursor(&token, &SALT, &wrong, &authority(&cursor)),
            Err(InvestigationCursorFailure::Invalid)
        );
        let mut wrong = binding(&cursor);
        wrong.filter_digest = "sha256:wrong";
        assert_eq!(
            continue_current_cursor(&token, &SALT, &wrong, &authority(&cursor)),
            Err(InvestigationCursorFailure::Invalid)
        );
    }

    #[test]
    fn investigation_cursor_rejects_each_temporal_binding_mismatch_as_invalid() {
        let cursor = cursor();
        let token = issue_current_cursor(&cursor, &SALT).expect("issue current V2");
        let base = cursor.temporal_binding();
        let mismatches = [
            InvestigationCursorTemporalBinding {
                as_of_change_seq: base.as_of_change_seq + 1,
                ..base.clone()
            },
            InvestigationCursorTemporalBinding {
                as_of_temporal_cutoff: time(11),
                ..base.clone()
            },
            InvestigationCursorTemporalBinding {
                authority_epoch_set_hash: "sha256:different".to_string(),
                ..base.clone()
            },
            InvestigationCursorTemporalBinding {
                earliest_effective_valid_until: time(49),
                ..base
            },
        ];

        for expected in &mismatches {
            let mut binding = binding(&cursor);
            binding.expected_temporal = Some(expected);
            assert_eq!(
                continue_current_cursor(&token, &SALT, &binding, &authority(&cursor)),
                Err(InvestigationCursorFailure::Invalid)
            );
        }
    }

    #[test]
    fn investigation_cursor_v2_each_temporal_field_tamper_is_invalid() {
        let cursor = cursor();
        let token = issue_current_cursor(&cursor, &SALT).expect("issue current V2");
        let tampered = [
            tamper_signed_field(
                &token,
                "as_of_change_seq",
                serde_json::Value::from(cursor.as_of_change_seq + 1),
            ),
            tamper_signed_field(
                &token,
                "as_of_temporal_cutoff",
                serde_json::Value::from(time(11).to_rfc3339()),
            ),
            tamper_signed_field(
                &token,
                "authority_epoch_set_hash",
                serde_json::Value::from("sha256:tampered"),
            ),
            tamper_signed_field(
                &token,
                "earliest_effective_valid_until",
                serde_json::Value::from(time(49).to_rfc3339()),
            ),
        ];
        for token in tampered {
            assert_eq!(
                continue_current_cursor(&token, &SALT, &binding(&cursor), &authority(&cursor),),
                Err(InvestigationCursorFailure::Invalid)
            );
        }
    }

    #[test]
    fn investigation_cursor_valid_signature_drift_is_stale_and_requires_restart() {
        let cursor = cursor();
        let token = issue_current_cursor(&cursor, &SALT).expect("issue current V2");
        let stale_cases = [
            InvestigationCursorCurrentAuthority {
                current_change_seq: cursor.as_of_change_seq + 1,
                db_now: time(20),
                current_authority_epoch_set_hash: &cursor.authority_epoch_set_hash,
            },
            InvestigationCursorCurrentAuthority {
                current_change_seq: cursor.as_of_change_seq,
                db_now: time(20),
                current_authority_epoch_set_hash: "sha256:new-epoch",
            },
            InvestigationCursorCurrentAuthority {
                current_change_seq: cursor.as_of_change_seq,
                db_now: time(51),
                current_authority_epoch_set_hash: &cursor.authority_epoch_set_hash,
            },
        ];
        for authority in &stale_cases {
            let error = continue_current_cursor(&token, &SALT, &binding(&cursor), authority)
                .expect_err("drift must be stale");
            assert_eq!(error.code(), INVESTIGATION_PROJECTION_STALE);
            assert!(error.restart_required());
        }
    }

    #[test]
    fn investigation_cursor_v1_is_verified_for_history_but_current_requires_restart() {
        let current = cursor();
        let legacy = InvestigationCursorV1Legacy {
            version: 1,
            resource_kind: current.resource_kind.clone(),
            operation_id: current.operation_id,
            projection_schema_version: current.projection_schema_version,
            as_of_change_seq: current.as_of_change_seq,
            tool_truth_contract: current.tool_truth_contract.clone(),
            investigation_contract_version: current.investigation_contract_version.clone(),
            investigation_rollout_mode: current.investigation_rollout_mode.clone(),
            filter_digest: current.filter_digest.clone(),
            page_size: current.page_size,
            stable_sort_key: current.stable_sort_key.clone(),
        };
        // Test-only construction proves the decoder without exposing a V1 writer.
        let token = canonical_sign(&legacy, &SALT).expect("fixture V1");
        assert_eq!(
            decode_historical_cursor(&token, &SALT, &binding(&current)),
            Ok(VerifiedInvestigationCursor::Historical(legacy))
        );
        let error =
            continue_current_cursor(&token, &SALT, &binding(&current), &authority(&current))
                .expect_err("V1 cannot continue current pagination");
        assert_eq!(error, InvestigationCursorFailure::Stale);
        assert!(error.restart_required());
    }

    #[test]
    fn investigation_filter_digest_deduplicates_and_sorts_and_rejects_bad_filters() {
        const EPISTEMIC: &[&str] = &["proposed", "supported", "contested"];
        const READINESS: &[&str] = &["ready_for_strategy", "unsafe"];
        const CAPABILITY: &[&str] = &["available", "unavailable"];
        const SOURCE: &[&str] = &["tool_truth", "knowledge_feed"];
        const CONFLICTS: &[InvestigationFilterConflict<'_>] = &[InvestigationFilterConflict {
            left_field: InvestigationFilterField::ReadinessState,
            left_value: "ready_for_strategy",
            right_field: InvestigationFilterField::CapabilityState,
            right_value: "unavailable",
        }];
        let policy = InvestigationFilterPolicy {
            epistemic_states: EPISTEMIC,
            readiness_states: READINESS,
            capability_states: CAPABILITY,
            source_kinds: SOURCE,
            conflicts: CONFLICTS,
        };
        let organization = operation_id();
        let canonical = canonicalize_investigation_filters(
            InvestigationFilterInput {
                organization_ids: &[organization, organization],
                epistemic_states: &[
                    "supported".to_string(),
                    "proposed".to_string(),
                    "supported".to_string(),
                ],
                readiness_states: &[],
                capability_states: &[],
                source_kinds: &["tool_truth".to_string(), "knowledge_feed".to_string()],
            },
            policy,
        )
        .expect("canonical filters");
        assert_eq!(canonical.organization_ids(), &[organization]);
        assert_eq!(canonical.epistemic_states(), &["proposed", "supported"]);
        assert!(canonical.readiness_states().is_empty());
        assert!(canonical.capability_states().is_empty());
        assert_eq!(canonical.source_kinds(), &["knowledge_feed", "tool_truth"]);
        assert_eq!(canonical.digest(), canonical.digest());

        let unknown = canonicalize_investigation_filters(
            InvestigationFilterInput {
                organization_ids: &[],
                epistemic_states: &["model_invented".to_string()],
                readiness_states: &[],
                capability_states: &[],
                source_kinds: &[],
            },
            policy,
        );
        assert!(matches!(
            unknown,
            Err(InvestigationFilterFailure::UnknownValue { .. })
        ));

        let conflict = canonicalize_investigation_filters(
            InvestigationFilterInput {
                organization_ids: &[],
                epistemic_states: &[],
                readiness_states: &["ready_for_strategy".to_string()],
                capability_states: &["unavailable".to_string()],
                source_kinds: &[],
            },
            policy,
        );
        assert!(matches!(
            conflict,
            Err(InvestigationFilterFailure::MutuallyExclusive { .. })
        ));
        assert_eq!(clamp_investigation_page_size(0), 1);
        assert_eq!(clamp_investigation_page_size(101), 100);
    }

    #[test]
    fn investigation_cursor_preserves_all_three_tagged_stable_keys() {
        let keys = [
            cursor().stable_sort_key,
            InvestigationStableSortKeyV1::Campaign {
                wave_ordinal: 1,
                campaign_ordinal: 2,
                campaign_id: operation_id(),
            },
            InvestigationStableSortKeyV1::Timeline {
                change_seq: 3,
                event_id: operation_id(),
            },
        ];
        let serialized = keys
            .iter()
            .map(|key| serde_json::to_value(key).expect("serialize stable key"))
            .collect::<Vec<_>>();
        assert_eq!(serialized[0]["kind"], "hypothesis");
        assert_eq!(serialized[1]["kind"], "campaign");
        assert_eq!(serialized[2]["kind"], "timeline");
    }
}
