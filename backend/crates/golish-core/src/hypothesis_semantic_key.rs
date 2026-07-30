//! Canonical semantic identity for Hypothesis Registry revisions.
//!
//! Presentation fields and live database identifiers intentionally never enter
//! this module.  The same primitives are consumed by Candidate (Plan B),
//! Verification (Plan C), and repository replay code.

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use uuid::Uuid;

const SEMANTIC_KEY_SCHEMA: &str = "hypothesis_semantic_key.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimPolarity {
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateMutationEpistemicState {
    Proposed,
    Supported,
    Contested,
    Inconclusive,
}

impl CandidateMutationEpistemicState {
    pub const ALL: [Self; 4] = [
        Self::Proposed,
        Self::Supported,
        Self::Contested,
        Self::Inconclusive,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Supported => "supported",
            Self::Contested => "contested",
            Self::Inconclusive => "inconclusive",
        }
    }
}

impl ClaimPolarity {
    pub const ALL: [Self; 2] = [Self::Positive, Self::Negative];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }
}

impl TryFrom<&str> for ClaimPolarity {
    type Error = HypothesisSemanticKeyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "positive" => Ok(Self::Positive),
            "negative" => Ok(Self::Negative),
            other => Err(HypothesisSemanticKeyError::UnknownPolarity(other.into())),
        }
    }
}

/// Canonical JSON object.  Construction from raw JSON detects duplicate keys;
/// construction from a typed `Value` is for data whose parser has already
/// established object-member uniqueness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CanonicalJsonObject(Value);

impl CanonicalJsonObject {
    pub fn try_from_value(value: Value) -> Result<Self, HypothesisSemanticKeyError> {
        if !value.is_object() {
            return Err(HypothesisSemanticKeyError::JsonObjectRequired);
        }
        Ok(Self(canonicalize_value(value)))
    }

    pub fn parse_raw(raw: &str) -> Result<Self, HypothesisSemanticKeyError> {
        let mut deserializer = serde_json::Deserializer::from_str(raw);
        let value = UniqueValueSeed
            .deserialize(&mut deserializer)
            .map_err(|error| HypothesisSemanticKeyError::InvalidJson(error.to_string()))?;
        deserializer
            .end()
            .map_err(|error| HypothesisSemanticKeyError::InvalidJson(error.to_string()))?;
        Self::try_from_value(value)
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.0).expect("canonical JSON values are serializable")
    }
}

impl<'de> Deserialize<'de> for CanonicalJsonObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = UniqueValueSeed.deserialize(deserializer)?;
        if !value.is_object() {
            return Err(de::Error::custom("canonical JSON value must be an object"));
        }
        Ok(Self(canonicalize_value(value)))
    }
}

struct UniqueValueSeed;

impl<'de> DeserializeSeed<'de> for UniqueValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.into()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(UniqueValueSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        let mut keys = BTreeSet::new();
        while let Some(key) = object.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
            }
            let value = object.next_value_seed(UniqueValueSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_value).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize_value(value));
            }
            Value::Object(canonical)
        }
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AtTimeSubjectIdentity {
    kind: String,
    identity_hash: String,
}

impl AtTimeSubjectIdentity {
    pub fn new(kind: String, identity_hash: String) -> Result<Self, HypothesisSemanticKeyError> {
        require_nonblank("subject.kind", &kind)?;
        validate_sha256(&identity_hash)?;
        Ok(Self {
            kind,
            identity_hash,
        })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn identity_hash(&self) -> &str {
        &self.identity_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PredicateIdentity {
    schema: String,
    version: u32,
    normalized_arguments: CanonicalJsonObject,
}

impl<'de> Deserialize<'de> for PredicateIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: String,
            version: u32,
            normalized_arguments: CanonicalJsonObject,
        }

        let wire = Wire::deserialize(deserializer)?;
        PredicateIdentity::from_canonical(wire.schema, wire.version, wire.normalized_arguments)
            .map_err(de::Error::custom)
    }
}

impl PredicateIdentity {
    pub fn new(
        schema: String,
        version: u32,
        normalized_arguments: Value,
    ) -> Result<Self, HypothesisSemanticKeyError> {
        require_nonblank("predicate.schema", &schema)?;
        if version == 0 {
            return Err(HypothesisSemanticKeyError::ZeroVersion("predicate.version"));
        }
        Ok(Self {
            schema,
            version,
            normalized_arguments: CanonicalJsonObject::try_from_value(normalized_arguments)?,
        })
    }

    pub fn from_canonical(
        schema: String,
        version: u32,
        normalized_arguments: CanonicalJsonObject,
    ) -> Result<Self, HypothesisSemanticKeyError> {
        require_nonblank("predicate.schema", &schema)?;
        if version == 0 {
            return Err(HypothesisSemanticKeyError::ZeroVersion("predicate.version"));
        }
        Ok(Self {
            schema,
            version,
            normalized_arguments,
        })
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub const fn version(&self) -> u32 {
        self.version
    }

    pub fn normalized_arguments(&self) -> &CanonicalJsonObject {
        &self.normalized_arguments
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisSemanticKeyV1 {
    schema: String,
    organization_id: Uuid,
    subject: AtTimeSubjectIdentity,
    predicate: PredicateIdentity,
    trust_boundary: String,
    polarity: ClaimPolarity,
}

/// Narrow adapter trait implemented by host-owned proposal DTOs.  It exposes
/// only identity fields, making presentation/evidence fields unobservable to
/// the key compiler.
pub trait SemanticClaimV1 {
    fn semantic_organization_id(&self) -> Uuid;
    fn semantic_subject_kind(&self) -> &str;
    fn semantic_subject_identity_hash(&self) -> &str;
    fn semantic_predicate(&self) -> &PredicateIdentity;
    fn semantic_trust_boundary(&self) -> &str;
    fn semantic_polarity(&self) -> ClaimPolarity;
}

impl HypothesisSemanticKeyV1 {
    pub fn from_claim<T: SemanticClaimV1 + ?Sized>(
        claim: &T,
    ) -> Result<Self, HypothesisSemanticKeyError> {
        Self::new(
            claim.semantic_organization_id(),
            AtTimeSubjectIdentity::new(
                claim.semantic_subject_kind().to_owned(),
                claim.semantic_subject_identity_hash().to_owned(),
            )?,
            claim.semantic_predicate().clone(),
            claim.semantic_trust_boundary().to_owned(),
            claim.semantic_polarity(),
        )
    }

    pub fn new(
        organization_id: Uuid,
        subject: AtTimeSubjectIdentity,
        predicate: PredicateIdentity,
        trust_boundary: String,
        polarity: ClaimPolarity,
    ) -> Result<Self, HypothesisSemanticKeyError> {
        if organization_id.is_nil() {
            return Err(HypothesisSemanticKeyError::NilUuid("organization_id"));
        }
        require_nonblank("trust_boundary", &trust_boundary)?;
        Ok(Self {
            schema: SEMANTIC_KEY_SCHEMA.into(),
            organization_id,
            subject,
            predicate,
            trust_boundary,
            polarity,
        })
    }

    pub fn hash(&self) -> Result<String, HypothesisSemanticKeyError> {
        let canonical = serde_json::to_vec(self)
            .map_err(|error| HypothesisSemanticKeyError::InvalidJson(error.to_string()))?;
        Ok(prefixed_sha256(&[
            SEMANTIC_KEY_SCHEMA.as_bytes(),
            b"\0",
            &canonical,
        ]))
    }

    pub const fn organization_id(&self) -> Uuid {
        self.organization_id
    }

    pub fn subject(&self) -> &AtTimeSubjectIdentity {
        &self.subject
    }

    pub fn predicate(&self) -> &PredicateIdentity {
        &self.predicate
    }

    pub fn trust_boundary(&self) -> &str {
        &self.trust_boundary
    }

    pub const fn polarity(&self) -> ClaimPolarity {
        self.polarity
    }
}

pub fn initial_root_id(
    operation_id: Uuid,
    semantic_key: &HypothesisSemanticKeyV1,
) -> Result<Uuid, HypothesisSemanticKeyError> {
    let namespace = operation_namespace(operation_id, semantic_key.organization_id)?;
    Ok(Uuid::new_v5(
        &namespace,
        format!("initial:{}", semantic_key.hash()?).as_bytes(),
    ))
}

pub fn split_root_id(
    operation_id: Uuid,
    semantic_key: &HypothesisSemanticKeyV1,
    parent_root_id: Uuid,
) -> Result<Uuid, HypothesisSemanticKeyError> {
    require_uuid("parent_root_id", parent_root_id)?;
    let namespace = operation_namespace(operation_id, semantic_key.organization_id)?;
    Ok(Uuid::new_v5(
        &namespace,
        format!("split:{parent_root_id}:{}", semantic_key.hash()?).as_bytes(),
    ))
}

pub fn merge_root_id(
    operation_id: Uuid,
    semantic_key: &HypothesisSemanticKeyV1,
    parent_root_ids: &[Uuid],
) -> Result<Uuid, HypothesisSemanticKeyError> {
    if parent_root_ids.len() < 2 {
        return Err(HypothesisSemanticKeyError::MergeParentsRequired);
    }
    let mut parents = parent_root_ids.to_vec();
    parents.sort_unstable();
    if parents.iter().any(Uuid::is_nil) {
        return Err(HypothesisSemanticKeyError::NilUuid("merge_parent_root_id"));
    }
    if parents.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(HypothesisSemanticKeyError::DuplicateMergeParent);
    }
    let joined = parents
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let namespace = operation_namespace(operation_id, semantic_key.organization_id)?;
    Ok(Uuid::new_v5(
        &namespace,
        format!("merge:{joined}:{}", semantic_key.hash()?).as_bytes(),
    ))
}

pub fn derive_root_id(
    operation_id: Uuid,
    semantic_key: &HypothesisSemanticKeyV1,
    source_root_id: Uuid,
    source_revision_id: Uuid,
    derivation_rule_hash: &str,
) -> Result<Uuid, HypothesisSemanticKeyError> {
    require_uuid("source_root_id", source_root_id)?;
    require_uuid("source_revision_id", source_revision_id)?;
    validate_sha256(derivation_rule_hash)?;
    let namespace = operation_namespace(operation_id, semantic_key.organization_id)?;
    Ok(Uuid::new_v5(
        &namespace,
        format!(
            "derive:{source_root_id}:{source_revision_id}:{derivation_rule_hash}:{}",
            semantic_key.hash()?
        )
        .as_bytes(),
    ))
}

pub fn candidate_revision_id(
    root_id: Uuid,
    ordinal: u32,
    semantic_key_hash: &str,
    origin_decision_hash: &str,
) -> Result<Uuid, HypothesisSemanticKeyError> {
    require_uuid("root_id", root_id)?;
    validate_sha256(semantic_key_hash)?;
    validate_sha256(origin_decision_hash)?;
    Ok(Uuid::new_v5(
        &root_id,
        format!("revision:{ordinal}:{semantic_key_hash}:{origin_decision_hash}").as_bytes(),
    ))
}

pub fn terminal_revision_id(
    root_id: Uuid,
    ordinal: u32,
    semantic_key_hash: &str,
    adjudication_hash: &str,
    transition_decision_hash: &str,
) -> Result<Uuid, HypothesisSemanticKeyError> {
    require_uuid("root_id", root_id)?;
    for hash in [
        semantic_key_hash,
        adjudication_hash,
        transition_decision_hash,
    ] {
        validate_sha256(hash)?;
    }
    Ok(Uuid::new_v5(
        &root_id,
        format!(
            "revision:{ordinal}:{semantic_key_hash}:{adjudication_hash}:{transition_decision_hash}"
        )
        .as_bytes(),
    ))
}

fn operation_namespace(
    operation_id: Uuid,
    organization_id: Uuid,
) -> Result<Uuid, HypothesisSemanticKeyError> {
    require_uuid("operation_id", operation_id)?;
    require_uuid("organization_id", organization_id)?;
    Ok(Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "golish:hypothesis-registry:v1:operation:{operation_id}:organization:{organization_id}"
        )
        .as_bytes(),
    ))
}

pub fn validate_sha256(value: &str) -> Result<(), HypothesisSemanticKeyError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(HypothesisSemanticKeyError::InvalidHash(value.into()));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HypothesisSemanticKeyError::InvalidHash(value.into()));
    }
    Ok(())
}

fn prefixed_sha256(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    format!("sha256:{}", hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn require_nonblank(field: &'static str, value: &str) -> Result<(), HypothesisSemanticKeyError> {
    if value.trim().is_empty() {
        Err(HypothesisSemanticKeyError::Blank(field))
    } else {
        Ok(())
    }
}

fn require_uuid(field: &'static str, value: Uuid) -> Result<(), HypothesisSemanticKeyError> {
    if value.is_nil() {
        Err(HypothesisSemanticKeyError::NilUuid(field))
    } else {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum HypothesisSemanticKeyError {
    #[error("{0} must not be blank")]
    Blank(&'static str),
    #[error("{0} must not be nil")]
    NilUuid(&'static str),
    #[error("{0} must be greater than zero")]
    ZeroVersion(&'static str),
    #[error("canonical JSON object required")]
    JsonObjectRequired,
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("invalid sha256 hash: {0}")]
    InvalidHash(String),
    #[error("unknown claim polarity: {0}")]
    UnknownPolarity(String),
    #[error("merge requires at least two distinct parent roots")]
    MergeParentsRequired,
    #[error("merge parent roots must be unique")]
    DuplicateMergeParent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_key() -> HypothesisSemanticKeyV1 {
        HypothesisSemanticKeyV1::new(
            Uuid::from_u128(2),
            AtTimeSubjectIdentity::new("service".into(), format!("sha256:{}", "1".repeat(64)))
                .unwrap(),
            PredicateIdentity::new(
                "http.authorization".into(),
                1,
                json!({"b": {"y": 2, "x": 1}, "a": [2, 1]}),
            )
            .unwrap(),
            "tenant".into(),
            ClaimPolarity::Positive,
        )
        .unwrap()
    }

    #[test]
    fn semantic_key_rejects_duplicate_raw_members() {
        let error = CanonicalJsonObject::parse_raw(r#"{"a":1,"a":2}"#).unwrap_err();
        assert!(error.to_string().contains("duplicate JSON key"));
    }

    #[test]
    fn semantic_key_hash_and_uuid_formulas_are_stable() {
        let key = fixture_key();
        let hash = key.hash().unwrap();
        assert!(hash.starts_with("sha256:"));
        assert_eq!(
            initial_root_id(Uuid::from_u128(1), &key).unwrap(),
            initial_root_id(Uuid::from_u128(1), &key).unwrap()
        );
        let a = Uuid::from_u128(11);
        let b = Uuid::from_u128(12);
        assert_eq!(
            merge_root_id(Uuid::from_u128(1), &key, &[a, b]).unwrap(),
            merge_root_id(Uuid::from_u128(1), &key, &[b, a]).unwrap()
        );
    }
}
