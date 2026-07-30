//! Canonical, host-compiled verification contract.
//!
//! Request and model DTOs must never carry the sealed values in this module.
//! Callers provide only typed source ingredients through
//! [`VerificationContractBuildInputV1`]. Ordinals, member hashes, exact-set
//! hashes, the contract hash, and the deterministic contract id are derived
//! here. Persisted rows are accepted only through
//! [`VerificationContractV1::try_from_persisted`], which recompiles the whole
//! contract and compares every derived field.

use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use crate::hypothesis_semantic_key::ClaimPolarity;

const CONTRACT_SCHEMA: &str = "verification_contract.v1";
const CONTRACT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VerificationContractError {
    #[error("invalid verification contract field: {0}")]
    InvalidField(&'static str),
    #[error("invalid verification contract hash field: {0}")]
    InvalidHash(&'static str),
    #[error("duplicate verification contract identity: {0}")]
    DuplicateIdentity(String),
    #[error("verification contract combinator shape is invalid: {0}")]
    InvalidCombinatorShape(&'static str),
    #[error("verification contract reference is missing or stale: {0}")]
    InvalidReference(&'static str),
    #[error("canonical JSON object is invalid: {0}")]
    InvalidCanonicalJson(String),
    #[error("persisted verification contract field drifted: {0}")]
    PersistedMismatch(&'static str),
    #[error("unknown {kind}: {value}")]
    UnknownClosedValue { kind: &'static str, value: String },
}

impl VerificationContractError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidField(_) => "VERIFICATION_CONTRACT_INVALID_FIELD",
            Self::InvalidHash(_) => "VERIFICATION_CONTRACT_INVALID_HASH",
            Self::DuplicateIdentity(_) => "VERIFICATION_CONTRACT_DUPLICATE_IDENTITY",
            Self::InvalidCombinatorShape(_) => "VERIFICATION_CONTRACT_COMBINATOR_SHAPE_INVALID",
            Self::InvalidReference(_) => "VERIFICATION_CONTRACT_REFERENCE_INVALID",
            Self::InvalidCanonicalJson(_) => "VERIFICATION_CONTRACT_CANONICAL_JSON_INVALID",
            Self::PersistedMismatch(_) => "VERIFICATION_CONTRACT_PERSISTED_MISMATCH",
            Self::UnknownClosedValue { .. } => "VERIFICATION_CONTRACT_UNKNOWN_CLOSED_VALUE",
        }
    }
}

macro_rules! closed_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident { $($variant:ident => $wire:literal),+ $(,)? }
        kind = $kind:literal
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = VerificationContractError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $($wire => Ok(Self::$variant),)+
                    other => Err(VerificationContractError::UnknownClosedValue {
                        kind: $kind,
                        value: other.to_owned(),
                    }),
                }
            }
        }
    };
}

closed_enum! {
    pub enum ContractCombinatorV1 {
        AllOf => "all_of",
        AnyOf => "any_of",
        PairedDifferential => "paired_differential",
        OrderedSequence => "ordered_sequence",
    }
    kind = "verification contract combinator"
}

closed_enum! {
    pub enum OrderedSessionScopeV1 {
        SameExecutionSession => "same_execution_session",
    }
    kind = "ordered session scope"
}

closed_enum! {
    pub enum OrderedInterleavingPolicyV1 {
        Forbid => "forbid",
    }
    kind = "ordered interleaving policy"
}

closed_enum! {
    pub enum OrderedResetPolicyV1 {
        RestartAtStepZero => "restart_at_step_zero",
    }
    kind = "ordered reset policy"
}

/// An object whose recursive JSON object keys have one deterministic ordering.
///
/// `parse` rejects duplicate keys before they can be collapsed into a
/// `serde_json::Map`. `try_from_value` is suitable only for already-typed JSON;
/// a `Value` cannot retain evidence that its original source had duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalJsonObject {
    value: Value,
    canonical_json: String,
}

impl CanonicalJsonObject {
    pub fn parse(raw: &str) -> Result<Self, VerificationContractError> {
        let mut deserializer = serde_json::Deserializer::from_str(raw);
        let value = NoDuplicateValue::deserialize(&mut deserializer)
            .map_err(|error| VerificationContractError::InvalidCanonicalJson(error.to_string()))?
            .0;
        deserializer
            .end()
            .map_err(|error| VerificationContractError::InvalidCanonicalJson(error.to_string()))?;
        Self::try_from_value(value)
    }

    pub fn try_from_value(value: Value) -> Result<Self, VerificationContractError> {
        if !value.is_object() {
            return Err(VerificationContractError::InvalidCanonicalJson(
                "normalized arguments must be a JSON object".to_owned(),
            ));
        }
        let mut canonical_json = String::new();
        write_canonical_json(&value, &mut canonical_json);
        Ok(Self {
            value,
            canonical_json,
        })
    }

    pub fn as_value(&self) -> &Value {
        &self.value
    }

    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical_json.as_bytes()
    }
}

impl Serialize for CanonicalJsonObject {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.value.serialize(serializer)
    }
}

fn write_canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output.push_str(
            &serde_json::to_string(value).expect("serializing a JSON string cannot fail"),
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("serializing a JSON key cannot fail"),
                );
                output.push(':');
                write_canonical_json(&values[key], output);
            }
            output.push('}');
        }
    }
}

struct NoDuplicateValue(Value);

impl<'de> Deserialize<'de> for NoDuplicateValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateValueVisitor)
    }
}

struct NoDuplicateValueVisitor;

impl<'de> Visitor<'de> for NoDuplicateValueVisitor {
    type Value = NoDuplicateValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(NoDuplicateValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        NoDuplicateValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<NoDuplicateValue>()? {
            values.push(value.0);
        }
        Ok(NoDuplicateValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
            }
            let value = map.next_value::<NoDuplicateValue>()?;
            values.insert(key, value.0);
        }
        Ok(NoDuplicateValue(Value::Object(values)))
    }
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn require_hash(value: &str, field: &'static str) -> Result<(), VerificationContractError> {
    if valid_sha256(value) {
        Ok(())
    } else {
        Err(VerificationContractError::InvalidHash(field))
    }
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), VerificationContractError> {
    if value.trim().is_empty() {
        Err(VerificationContractError::InvalidField(field))
    } else {
        Ok(())
    }
}

struct DomainHashWriter(Sha256);

impl DomainHashWriter {
    fn new(domain: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(domain.as_bytes());
        digest.update([0]);
        Self(digest)
    }

    fn field(&mut self, bytes: &[u8]) {
        self.0.update((bytes.len() as u64).to_be_bytes());
        self.0.update(bytes);
    }

    fn text(&mut self, value: &str) {
        self.field(value.as_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.field(&value.to_be_bytes());
    }

    fn bool(&mut self, value: bool) {
        self.field(&[u8::from(value)]);
    }

    fn uuid(&mut self, value: Uuid) {
        self.field(value.as_bytes());
    }

    fn finish(self) -> String {
        let digest = self.0.finalize();
        let mut output = String::with_capacity(71);
        output.push_str("sha256:");
        for byte in digest {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing into String cannot fail");
        }
        output
    }
}

fn exact_set_hash(domain: &str, member_hashes: &[String]) -> String {
    let mut hash = DomainHashWriter::new(domain);
    hash.u32(u32::try_from(member_hashes.len()).unwrap_or(u32::MAX));
    for member_hash in member_hashes {
        hash.text(member_hash);
    }
    hash.finish()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredicateComponentInputV1 {
    pub semantic_key: String,
    pub predicate_schema: String,
    pub predicate_version: u32,
    pub normalized_arguments: CanonicalJsonObject,
    pub expected_polarity: ClaimPolarity,
    pub prerequisite_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationControlInputV1 {
    pub control_id: String,
    pub control_version: u32,
    pub control_contract_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDifferentialBindingInputV1 {
    pub pair_key: String,
    pub baseline_component_key: String,
    pub variant_component_key: String,
    pub required_control_id: String,
    pub required_control_version: u32,
    pub required_control_contract_hash: String,
    pub comparator_rule_id: String,
    pub comparator_rule_version: u32,
    pub comparator_rule_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedSequenceStepInputV1 {
    pub step_ordinal: u32,
    pub component_key: String,
    pub predecessor_step_ordinal: Option<u32>,
    pub session_binding_key_schema: String,
    pub session_binding_key_version: u32,
    pub session_scope: OrderedSessionScopeV1,
    pub interleaving_policy: OrderedInterleavingPolicyV1,
    pub reset_policy: OrderedResetPolicyV1,
}

/// Source-authority-only input to the host compiler.
///
/// This type intentionally contains no member hash, exact-set hash, count,
/// final contract hash, or contract id supplied by an untrusted caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationContractBuildInputV1 {
    pub revision_id: Uuid,
    pub revision_hash: String,
    pub objective_id: Uuid,
    pub combinator: ContractCombinatorV1,
    pub predicate_components: Vec<PredicateComponentInputV1>,
    pub required_controls: Vec<VerificationControlInputV1>,
    pub paired_differential_bindings: Vec<PairedDifferentialBindingInputV1>,
    pub ordered_steps: Vec<OrderedSequenceStepInputV1>,
    pub stopping_criteria_hash: String,
    pub compiler_digest: String,
    pub rule_digest: String,
    pub policy_snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PredicateComponentV1 {
    ordinal: u32,
    semantic_key: String,
    predicate_schema: String,
    predicate_version: u32,
    normalized_arguments: CanonicalJsonObject,
    expected_polarity: ClaimPolarity,
    prerequisite_hash: String,
    member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationControlV1 {
    ordinal: u32,
    control_id: String,
    control_version: u32,
    control_contract_hash: String,
    member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairedDifferentialBindingV1 {
    ordinal: u32,
    pair_key: String,
    baseline_component_key: String,
    variant_component_key: String,
    required_control_id: String,
    required_control_version: u32,
    required_control_contract_hash: String,
    required_control_member_hash: String,
    comparator_rule_id: String,
    comparator_rule_version: u32,
    comparator_rule_digest: String,
    member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrderedSequenceStepV1 {
    step_ordinal: u32,
    component_key: String,
    predecessor_step_ordinal: Option<u32>,
    session_binding_key_schema: String,
    session_binding_key_version: u32,
    session_scope: OrderedSessionScopeV1,
    interleaving_policy: OrderedInterleavingPolicyV1,
    reset_policy: OrderedResetPolicyV1,
    step_hash: String,
}

/// Host-sealed contract. No public constructor and no public fields exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationContractV1 {
    contract_id: Uuid,
    contract_schema: String,
    contract_version: u32,
    revision_id: Uuid,
    revision_hash: String,
    objective_id: Uuid,
    combinator: ContractCombinatorV1,
    predicate_components: Vec<PredicateComponentV1>,
    predicate_count: u32,
    predicate_set_hash: String,
    required_controls: Vec<VerificationControlV1>,
    required_control_count: u32,
    required_control_set_hash: String,
    explicit_no_required_control: bool,
    paired_differential_bindings: Vec<PairedDifferentialBindingV1>,
    paired_differential_count: u32,
    paired_differential_set_hash: String,
    ordered_steps: Vec<OrderedSequenceStepV1>,
    ordered_step_count: u32,
    ordered_step_set_hash: String,
    stopping_criteria_hash: String,
    compiler_digest: String,
    rule_digest: String,
    policy_snapshot_hash: String,
    contract_hash: String,
}

impl PredicateComponentV1 {
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub fn semantic_key(&self) -> &str {
        &self.semantic_key
    }

    pub fn predicate_schema(&self) -> &str {
        &self.predicate_schema
    }

    pub const fn predicate_version(&self) -> u32 {
        self.predicate_version
    }

    pub const fn normalized_arguments(&self) -> &CanonicalJsonObject {
        &self.normalized_arguments
    }

    pub const fn expected_polarity(&self) -> ClaimPolarity {
        self.expected_polarity
    }

    pub fn prerequisite_hash(&self) -> &str {
        &self.prerequisite_hash
    }

    pub fn member_hash(&self) -> &str {
        &self.member_hash
    }
}

impl VerificationControlV1 {
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub fn control_id(&self) -> &str {
        &self.control_id
    }

    pub const fn control_version(&self) -> u32 {
        self.control_version
    }

    pub fn control_contract_hash(&self) -> &str {
        &self.control_contract_hash
    }

    pub fn member_hash(&self) -> &str {
        &self.member_hash
    }
}

impl PairedDifferentialBindingV1 {
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub fn pair_key(&self) -> &str {
        &self.pair_key
    }

    pub fn baseline_component_key(&self) -> &str {
        &self.baseline_component_key
    }

    pub fn variant_component_key(&self) -> &str {
        &self.variant_component_key
    }

    pub fn required_control_id(&self) -> &str {
        &self.required_control_id
    }

    pub const fn required_control_version(&self) -> u32 {
        self.required_control_version
    }

    pub fn required_control_contract_hash(&self) -> &str {
        &self.required_control_contract_hash
    }

    pub fn required_control_member_hash(&self) -> &str {
        &self.required_control_member_hash
    }

    pub fn comparator_rule_id(&self) -> &str {
        &self.comparator_rule_id
    }

    pub const fn comparator_rule_version(&self) -> u32 {
        self.comparator_rule_version
    }

    pub fn comparator_rule_digest(&self) -> &str {
        &self.comparator_rule_digest
    }

    pub fn member_hash(&self) -> &str {
        &self.member_hash
    }
}

impl OrderedSequenceStepV1 {
    pub const fn step_ordinal(&self) -> u32 {
        self.step_ordinal
    }

    pub fn component_key(&self) -> &str {
        &self.component_key
    }

    pub const fn predecessor_step_ordinal(&self) -> Option<u32> {
        self.predecessor_step_ordinal
    }

    pub fn session_binding_key_schema(&self) -> &str {
        &self.session_binding_key_schema
    }

    pub const fn session_binding_key_version(&self) -> u32 {
        self.session_binding_key_version
    }

    pub const fn session_scope(&self) -> OrderedSessionScopeV1 {
        self.session_scope
    }

    pub const fn interleaving_policy(&self) -> OrderedInterleavingPolicyV1 {
        self.interleaving_policy
    }

    pub const fn reset_policy(&self) -> OrderedResetPolicyV1 {
        self.reset_policy
    }

    pub fn step_hash(&self) -> &str {
        &self.step_hash
    }
}

impl VerificationContractV1 {
    pub const fn contract_id(&self) -> Uuid {
        self.contract_id
    }

    pub fn contract_schema(&self) -> &str {
        &self.contract_schema
    }

    pub const fn contract_version(&self) -> u32 {
        self.contract_version
    }

    pub const fn revision_id(&self) -> Uuid {
        self.revision_id
    }

    pub fn revision_hash(&self) -> &str {
        &self.revision_hash
    }

    pub const fn objective_id(&self) -> Uuid {
        self.objective_id
    }

    pub const fn combinator(&self) -> ContractCombinatorV1 {
        self.combinator
    }

    pub fn predicate_components(&self) -> &[PredicateComponentV1] {
        &self.predicate_components
    }

    pub const fn predicate_count(&self) -> u32 {
        self.predicate_count
    }

    pub fn predicate_set_hash(&self) -> &str {
        &self.predicate_set_hash
    }

    pub fn required_controls(&self) -> &[VerificationControlV1] {
        &self.required_controls
    }

    pub const fn required_control_count(&self) -> u32 {
        self.required_control_count
    }

    pub fn required_control_set_hash(&self) -> &str {
        &self.required_control_set_hash
    }

    pub const fn explicit_no_required_control(&self) -> bool {
        self.explicit_no_required_control
    }

    pub fn paired_differential_bindings(&self) -> &[PairedDifferentialBindingV1] {
        &self.paired_differential_bindings
    }

    pub const fn paired_differential_count(&self) -> u32 {
        self.paired_differential_count
    }

    pub fn paired_differential_set_hash(&self) -> &str {
        &self.paired_differential_set_hash
    }

    pub fn ordered_steps(&self) -> &[OrderedSequenceStepV1] {
        &self.ordered_steps
    }

    pub const fn ordered_step_count(&self) -> u32 {
        self.ordered_step_count
    }

    pub fn ordered_step_set_hash(&self) -> &str {
        &self.ordered_step_set_hash
    }

    pub fn stopping_criteria_hash(&self) -> &str {
        &self.stopping_criteria_hash
    }

    pub fn compiler_digest(&self) -> &str {
        &self.compiler_digest
    }

    pub fn rule_digest(&self) -> &str {
        &self.rule_digest
    }

    pub fn policy_snapshot_hash(&self) -> &str {
        &self.policy_snapshot_hash
    }

    pub fn contract_hash(&self) -> &str {
        &self.contract_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(nibble: char) -> String {
        assert!(nibble.is_ascii_hexdigit() && !nibble.is_ascii_uppercase());
        format!("sha256:{}", nibble.to_string().repeat(64))
    }

    fn arguments(raw: &str) -> CanonicalJsonObject {
        CanonicalJsonObject::parse(raw).expect("valid fixture arguments")
    }

    fn predicate(key: &str, nibble: char) -> PredicateComponentInputV1 {
        PredicateComponentInputV1 {
            semantic_key: key.to_owned(),
            predicate_schema: "dns_observation.v1".to_owned(),
            predicate_version: 1,
            normalized_arguments: arguments(&format!(r#"{{"name":"{key}","port":443}}"#)),
            expected_polarity: ClaimPolarity::Positive,
            prerequisite_hash: digest(nibble),
        }
    }

    fn base_input(
        combinator: ContractCombinatorV1,
        predicate_components: Vec<PredicateComponentInputV1>,
    ) -> VerificationContractBuildInputV1 {
        VerificationContractBuildInputV1 {
            revision_id: Uuid::from_u128(0x1111),
            revision_hash: digest('1'),
            objective_id: Uuid::from_u128(0x2222),
            combinator,
            predicate_components,
            required_controls: Vec::new(),
            paired_differential_bindings: Vec::new(),
            ordered_steps: Vec::new(),
            stopping_criteria_hash: digest('2'),
            compiler_digest: digest('3'),
            rule_digest: digest('4'),
            policy_snapshot_hash: digest('5'),
        }
    }

    fn paired_input() -> VerificationContractBuildInputV1 {
        let mut input = base_input(
            ContractCombinatorV1::PairedDifferential,
            vec![predicate("variant", 'b'), predicate("baseline", 'a')],
        );
        input.required_controls = vec![VerificationControlInputV1 {
            control_id: "negative-control".to_owned(),
            control_version: 2,
            control_contract_hash: digest('6'),
        }];
        input.paired_differential_bindings = vec![PairedDifferentialBindingInputV1 {
            pair_key: "tls-delta".to_owned(),
            baseline_component_key: "baseline".to_owned(),
            variant_component_key: "variant".to_owned(),
            required_control_id: "negative-control".to_owned(),
            required_control_version: 2,
            required_control_contract_hash: digest('6'),
            comparator_rule_id: "strict-difference".to_owned(),
            comparator_rule_version: 3,
            comparator_rule_digest: digest('7'),
        }];
        input
    }

    fn ordered_input() -> VerificationContractBuildInputV1 {
        let mut input = base_input(
            ContractCombinatorV1::OrderedSequence,
            vec![predicate("second", 'b'), predicate("first", 'a')],
        );
        input.ordered_steps = vec![
            OrderedSequenceStepInputV1 {
                step_ordinal: 1,
                component_key: "second".to_owned(),
                predecessor_step_ordinal: Some(0),
                session_binding_key_schema: "execution_session.v1".to_owned(),
                session_binding_key_version: 1,
                session_scope: OrderedSessionScopeV1::SameExecutionSession,
                interleaving_policy: OrderedInterleavingPolicyV1::Forbid,
                reset_policy: OrderedResetPolicyV1::RestartAtStepZero,
            },
            OrderedSequenceStepInputV1 {
                step_ordinal: 0,
                component_key: "first".to_owned(),
                predecessor_step_ordinal: None,
                session_binding_key_schema: "execution_session.v1".to_owned(),
                session_binding_key_version: 1,
                session_scope: OrderedSessionScopeV1::SameExecutionSession,
                interleaving_policy: OrderedInterleavingPolicyV1::Forbid,
                reset_policy: OrderedResetPolicyV1::RestartAtStepZero,
            },
        ];
        input
    }

    #[test]
    fn verification_contract_closed_enums_reject_unknown_values() {
        assert_eq!(
            ContractCombinatorV1::ALL,
            &[
                ContractCombinatorV1::AllOf,
                ContractCombinatorV1::AnyOf,
                ContractCombinatorV1::PairedDifferential,
                ContractCombinatorV1::OrderedSequence,
            ]
        );
        assert!(matches!(
            ContractCombinatorV1::try_from("threshold"),
            Err(VerificationContractError::UnknownClosedValue { .. })
        ));
        assert!(serde_json::from_str::<ContractCombinatorV1>(r#""threshold""#).is_err());
    }

    #[test]
    fn verification_contract_canonical_json_sorts_recursively_and_rejects_duplicates() {
        let left =
            CanonicalJsonObject::parse(r#"{"z":0,"a":{"y":2,"x":1}}"#).expect("left JSON is valid");
        let right = CanonicalJsonObject::parse(r#"{"a":{"x":1,"y":2},"z":0}"#)
            .expect("right JSON is valid");
        assert_eq!(left, right);
        assert_eq!(left.canonical_json(), r#"{"a":{"x":1,"y":2},"z":0}"#);
        assert!(matches!(
            CanonicalJsonObject::parse(r#"{"a":{"x":1,"x":2}}"#),
            Err(VerificationContractError::InvalidCanonicalJson(_))
        ));
        assert!(CanonicalJsonObject::parse("[]").is_err());
    }

    #[test]
    fn verification_contract_permutations_have_one_canonical_identity() {
        let components = [
            predicate("alpha", 'a'),
            predicate("bravo", 'b'),
            predicate("charlie", 'c'),
        ];
        let permutations = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        let contracts = permutations.map(|permutation| {
            VerificationContractV1::compile(base_input(
                ContractCombinatorV1::AllOf,
                permutation
                    .into_iter()
                    .map(|index| components[index].clone())
                    .collect(),
            ))
            .expect("permutation compiles")
        });
        for contract in &contracts[1..] {
            assert_eq!(contract.contract_hash(), contracts[0].contract_hash());
            assert_eq!(contract.contract_id(), contracts[0].contract_id());
            assert_eq!(
                contract.predicate_set_hash(),
                contracts[0].predicate_set_hash()
            );
            assert_eq!(
                contract
                    .predicate_components()
                    .iter()
                    .map(PredicateComponentV1::member_hash)
                    .collect::<Vec<_>>(),
                contracts[0]
                    .predicate_components()
                    .iter()
                    .map(PredicateComponentV1::member_hash)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn verification_contract_source_mutation_changes_sealed_identity() {
        let input = base_input(ContractCombinatorV1::AllOf, vec![predicate("alpha", 'a')]);
        let original = VerificationContractV1::compile(input.clone()).expect("input compiles");
        let mut changed = input;
        changed.compiler_digest = digest('f');
        let changed = VerificationContractV1::compile(changed).expect("changed input compiles");
        assert_ne!(original.contract_hash(), changed.contract_hash());
        assert_ne!(original.contract_id(), changed.contract_id());
    }

    #[test]
    fn verification_contract_no_control_state_is_explicit_and_hashed() {
        let no_control = VerificationContractV1::compile(base_input(
            ContractCombinatorV1::AnyOf,
            vec![predicate("alpha", 'a')],
        ))
        .expect("no-control contract compiles");
        assert!(no_control.explicit_no_required_control());
        assert_eq!(no_control.required_control_count(), 0);
        assert!(valid_sha256(no_control.required_control_set_hash()));

        let mut with_control =
            base_input(ContractCombinatorV1::AnyOf, vec![predicate("alpha", 'a')]);
        with_control.required_controls = vec![VerificationControlInputV1 {
            control_id: "control".to_owned(),
            control_version: 1,
            control_contract_hash: digest('d'),
        }];
        let with_control = VerificationContractV1::compile(with_control)
            .expect("contract with a control compiles");
        assert!(!with_control.explicit_no_required_control());
        assert_ne!(no_control.contract_hash(), with_control.contract_hash());
    }

    #[test]
    fn verification_contract_all_and_any_reject_foreign_structure() {
        let mut input = base_input(ContractCombinatorV1::AllOf, vec![predicate("alpha", 'a')]);
        input.ordered_steps.push(OrderedSequenceStepInputV1 {
            step_ordinal: 0,
            component_key: "alpha".to_owned(),
            predecessor_step_ordinal: None,
            session_binding_key_schema: "execution_session.v1".to_owned(),
            session_binding_key_version: 1,
            session_scope: OrderedSessionScopeV1::SameExecutionSession,
            interleaving_policy: OrderedInterleavingPolicyV1::Forbid,
            reset_policy: OrderedResetPolicyV1::RestartAtStepZero,
        });
        assert!(matches!(
            VerificationContractV1::compile(input),
            Err(VerificationContractError::InvalidCombinatorShape(_))
        ));
    }

    #[test]
    fn verification_contract_paired_differential_seals_full_control_identity() {
        let contract = VerificationContractV1::compile(paired_input()).expect("pair compiles");
        let pair = &contract.paired_differential_bindings()[0];
        assert_eq!(
            pair.required_control_member_hash(),
            contract.required_controls()[0].member_hash()
        );
        assert!(valid_sha256(pair.member_hash()));

        let mut stale = paired_input();
        stale.paired_differential_bindings[0].required_control_version = 1;
        assert!(matches!(
            VerificationContractV1::compile(stale),
            Err(VerificationContractError::InvalidReference(_))
        ));

        let mut unpaired_component = paired_input();
        unpaired_component
            .predicate_components
            .push(predicate("orphan", 'c'));
        assert!(matches!(
            VerificationContractV1::compile(unpaired_component),
            Err(VerificationContractError::InvalidCombinatorShape(_))
        ));

        let mut unused_control = paired_input();
        unused_control
            .required_controls
            .push(VerificationControlInputV1 {
                control_id: "unused".to_owned(),
                control_version: 1,
                control_contract_hash: digest('e'),
            });
        assert!(matches!(
            VerificationContractV1::compile(unused_control),
            Err(VerificationContractError::InvalidCombinatorShape(_))
        ));
    }

    #[test]
    fn verification_contract_ordered_sequence_is_contiguous_and_session_bound() {
        let contract = VerificationContractV1::compile(ordered_input()).expect("order compiles");
        assert_eq!(contract.ordered_step_count(), 2);
        assert_eq!(contract.ordered_steps()[0].step_ordinal(), 0);
        assert_eq!(
            contract.ordered_steps()[1].predecessor_step_ordinal(),
            Some(0)
        );

        let mut gap = ordered_input();
        gap.ordered_steps[0].step_ordinal = 2;
        assert!(matches!(
            VerificationContractV1::compile(gap),
            Err(VerificationContractError::InvalidCombinatorShape(_))
        ));

        let mut wrong_predecessor = ordered_input();
        wrong_predecessor.ordered_steps[0].predecessor_step_ordinal = None;
        assert!(matches!(
            VerificationContractV1::compile(wrong_predecessor),
            Err(VerificationContractError::InvalidCombinatorShape(_))
        ));

        let mut mixed_session = ordered_input();
        mixed_session.ordered_steps[0].session_binding_key_version = 2;
        assert!(matches!(
            VerificationContractV1::compile(mixed_session),
            Err(VerificationContractError::InvalidCombinatorShape(_))
        ));
    }

    #[test]
    fn verification_contract_persisted_replay_recompiles_and_rejects_drift() {
        let contract = VerificationContractV1::compile(paired_input()).expect("pair compiles");
        let snapshot = contract.persisted_snapshot();
        let replayed = VerificationContractV1::try_from_persisted(snapshot.clone())
            .expect("untampered snapshot replays");
        assert_eq!(replayed, contract);

        let mut member_drift = snapshot.clone();
        member_drift.predicate_components[0].member_hash = digest('0');
        assert!(matches!(
            VerificationContractV1::try_from_persisted(member_drift),
            Err(VerificationContractError::PersistedMismatch(_))
        ));

        let mut count_drift = snapshot.clone();
        count_drift.predicate_count += 1;
        assert!(matches!(
            VerificationContractV1::try_from_persisted(count_drift),
            Err(VerificationContractError::PersistedMismatch(_))
        ));

        let mut set_drift = snapshot.clone();
        set_drift.required_control_set_hash = digest('0');
        assert!(matches!(
            VerificationContractV1::try_from_persisted(set_drift),
            Err(VerificationContractError::PersistedMismatch(_))
        ));

        let mut pair_binding_drift = snapshot.clone();
        pair_binding_drift.paired_differential_bindings[0].required_control_member_hash =
            digest('0');
        assert!(matches!(
            VerificationContractV1::try_from_persisted(pair_binding_drift),
            Err(VerificationContractError::PersistedMismatch(_))
        ));

        let mut final_hash_drift = snapshot;
        final_hash_drift.contract_hash = digest('0');
        assert!(matches!(
            VerificationContractV1::try_from_persisted(final_hash_drift),
            Err(VerificationContractError::PersistedMismatch(_))
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPredicateComponentV1 {
    pub ordinal: u32,
    pub semantic_key: String,
    pub predicate_schema: String,
    pub predicate_version: u32,
    pub normalized_arguments: Value,
    pub expected_polarity: ClaimPolarity,
    pub prerequisite_hash: String,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedVerificationControlV1 {
    pub ordinal: u32,
    pub control_id: String,
    pub control_version: u32,
    pub control_contract_hash: String,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPairedDifferentialBindingV1 {
    pub ordinal: u32,
    pub pair_key: String,
    pub baseline_component_key: String,
    pub variant_component_key: String,
    pub required_control_id: String,
    pub required_control_version: u32,
    pub required_control_contract_hash: String,
    pub required_control_member_hash: String,
    pub comparator_rule_id: String,
    pub comparator_rule_version: u32,
    pub comparator_rule_digest: String,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedOrderedSequenceStepV1 {
    pub step_ordinal: u32,
    pub component_key: String,
    pub predecessor_step_ordinal: Option<u32>,
    pub session_binding_key_schema: String,
    pub session_binding_key_version: u32,
    pub session_scope: OrderedSessionScopeV1,
    pub interleaving_policy: OrderedInterleavingPolicyV1,
    pub reset_policy: OrderedResetPolicyV1,
    pub step_hash: String,
}

/// Database/repository transport form. It is deliberately not authoritative:
/// [`VerificationContractV1::try_from_persisted`] is the only validating path
/// back to a sealed contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedVerificationContractV1 {
    pub contract_id: Uuid,
    pub contract_schema: String,
    pub contract_version: u32,
    pub revision_id: Uuid,
    pub revision_hash: String,
    pub objective_id: Uuid,
    pub combinator: ContractCombinatorV1,
    pub predicate_components: Vec<PersistedPredicateComponentV1>,
    pub predicate_count: u32,
    pub predicate_set_hash: String,
    pub required_controls: Vec<PersistedVerificationControlV1>,
    pub required_control_count: u32,
    pub required_control_set_hash: String,
    pub explicit_no_required_control: bool,
    pub paired_differential_bindings: Vec<PersistedPairedDifferentialBindingV1>,
    pub paired_differential_count: u32,
    pub paired_differential_set_hash: String,
    pub ordered_steps: Vec<PersistedOrderedSequenceStepV1>,
    pub ordered_step_count: u32,
    pub ordered_step_set_hash: String,
    pub stopping_criteria_hash: String,
    pub compiler_digest: String,
    pub rule_digest: String,
    pub policy_snapshot_hash: String,
    pub contract_hash: String,
}

impl VerificationContractV1 {
    pub fn persisted_snapshot(&self) -> PersistedVerificationContractV1 {
        PersistedVerificationContractV1 {
            contract_id: self.contract_id,
            contract_schema: self.contract_schema.clone(),
            contract_version: self.contract_version,
            revision_id: self.revision_id,
            revision_hash: self.revision_hash.clone(),
            objective_id: self.objective_id,
            combinator: self.combinator,
            predicate_components: self
                .predicate_components
                .iter()
                .map(|component| PersistedPredicateComponentV1 {
                    ordinal: component.ordinal,
                    semantic_key: component.semantic_key.clone(),
                    predicate_schema: component.predicate_schema.clone(),
                    predicate_version: component.predicate_version,
                    normalized_arguments: component.normalized_arguments.as_value().clone(),
                    expected_polarity: component.expected_polarity,
                    prerequisite_hash: component.prerequisite_hash.clone(),
                    member_hash: component.member_hash.clone(),
                })
                .collect(),
            predicate_count: self.predicate_count,
            predicate_set_hash: self.predicate_set_hash.clone(),
            required_controls: self
                .required_controls
                .iter()
                .map(|control| PersistedVerificationControlV1 {
                    ordinal: control.ordinal,
                    control_id: control.control_id.clone(),
                    control_version: control.control_version,
                    control_contract_hash: control.control_contract_hash.clone(),
                    member_hash: control.member_hash.clone(),
                })
                .collect(),
            required_control_count: self.required_control_count,
            required_control_set_hash: self.required_control_set_hash.clone(),
            explicit_no_required_control: self.explicit_no_required_control,
            paired_differential_bindings: self
                .paired_differential_bindings
                .iter()
                .map(|binding| PersistedPairedDifferentialBindingV1 {
                    ordinal: binding.ordinal,
                    pair_key: binding.pair_key.clone(),
                    baseline_component_key: binding.baseline_component_key.clone(),
                    variant_component_key: binding.variant_component_key.clone(),
                    required_control_id: binding.required_control_id.clone(),
                    required_control_version: binding.required_control_version,
                    required_control_contract_hash: binding.required_control_contract_hash.clone(),
                    required_control_member_hash: binding.required_control_member_hash.clone(),
                    comparator_rule_id: binding.comparator_rule_id.clone(),
                    comparator_rule_version: binding.comparator_rule_version,
                    comparator_rule_digest: binding.comparator_rule_digest.clone(),
                    member_hash: binding.member_hash.clone(),
                })
                .collect(),
            paired_differential_count: self.paired_differential_count,
            paired_differential_set_hash: self.paired_differential_set_hash.clone(),
            ordered_steps: self
                .ordered_steps
                .iter()
                .map(|step| PersistedOrderedSequenceStepV1 {
                    step_ordinal: step.step_ordinal,
                    component_key: step.component_key.clone(),
                    predecessor_step_ordinal: step.predecessor_step_ordinal,
                    session_binding_key_schema: step.session_binding_key_schema.clone(),
                    session_binding_key_version: step.session_binding_key_version,
                    session_scope: step.session_scope,
                    interleaving_policy: step.interleaving_policy,
                    reset_policy: step.reset_policy,
                    step_hash: step.step_hash.clone(),
                })
                .collect(),
            ordered_step_count: self.ordered_step_count,
            ordered_step_set_hash: self.ordered_step_set_hash.clone(),
            stopping_criteria_hash: self.stopping_criteria_hash.clone(),
            compiler_digest: self.compiler_digest.clone(),
            rule_digest: self.rule_digest.clone(),
            policy_snapshot_hash: self.policy_snapshot_hash.clone(),
            contract_hash: self.contract_hash.clone(),
        }
    }

    pub fn try_from_persisted(
        mut persisted: PersistedVerificationContractV1,
    ) -> Result<Self, VerificationContractError> {
        persisted
            .predicate_components
            .sort_by_key(|component| component.ordinal);
        persisted
            .required_controls
            .sort_by_key(|control| control.ordinal);
        persisted
            .paired_differential_bindings
            .sort_by_key(|binding| binding.ordinal);
        persisted
            .ordered_steps
            .sort_by_key(|step| step.step_ordinal);

        for (index, component) in persisted.predicate_components.iter().enumerate() {
            if component.ordinal != checked_count(index, "persisted predicate ordinal")? {
                return Err(VerificationContractError::PersistedMismatch(
                    "predicate_components.ordinal",
                ));
            }
        }
        for (index, control) in persisted.required_controls.iter().enumerate() {
            if control.ordinal != checked_count(index, "persisted control ordinal")? {
                return Err(VerificationContractError::PersistedMismatch(
                    "required_controls.ordinal",
                ));
            }
        }
        for (index, binding) in persisted.paired_differential_bindings.iter().enumerate() {
            if binding.ordinal != checked_count(index, "persisted pair ordinal")? {
                return Err(VerificationContractError::PersistedMismatch(
                    "paired_differential_bindings.ordinal",
                ));
            }
        }
        for (index, step) in persisted.ordered_steps.iter().enumerate() {
            if step.step_ordinal != checked_count(index, "persisted step ordinal")? {
                return Err(VerificationContractError::PersistedMismatch(
                    "ordered_steps.step_ordinal",
                ));
            }
        }

        let build_input = VerificationContractBuildInputV1 {
            revision_id: persisted.revision_id,
            revision_hash: persisted.revision_hash.clone(),
            objective_id: persisted.objective_id,
            combinator: persisted.combinator,
            predicate_components: persisted
                .predicate_components
                .iter()
                .map(|component| {
                    Ok(PredicateComponentInputV1 {
                        semantic_key: component.semantic_key.clone(),
                        predicate_schema: component.predicate_schema.clone(),
                        predicate_version: component.predicate_version,
                        normalized_arguments: CanonicalJsonObject::try_from_value(
                            component.normalized_arguments.clone(),
                        )?,
                        expected_polarity: component.expected_polarity,
                        prerequisite_hash: component.prerequisite_hash.clone(),
                    })
                })
                .collect::<Result<Vec<_>, VerificationContractError>>()?,
            required_controls: persisted
                .required_controls
                .iter()
                .map(|control| VerificationControlInputV1 {
                    control_id: control.control_id.clone(),
                    control_version: control.control_version,
                    control_contract_hash: control.control_contract_hash.clone(),
                })
                .collect(),
            paired_differential_bindings: persisted
                .paired_differential_bindings
                .iter()
                .map(|binding| PairedDifferentialBindingInputV1 {
                    pair_key: binding.pair_key.clone(),
                    baseline_component_key: binding.baseline_component_key.clone(),
                    variant_component_key: binding.variant_component_key.clone(),
                    required_control_id: binding.required_control_id.clone(),
                    required_control_version: binding.required_control_version,
                    required_control_contract_hash: binding.required_control_contract_hash.clone(),
                    comparator_rule_id: binding.comparator_rule_id.clone(),
                    comparator_rule_version: binding.comparator_rule_version,
                    comparator_rule_digest: binding.comparator_rule_digest.clone(),
                })
                .collect(),
            ordered_steps: persisted
                .ordered_steps
                .iter()
                .map(|step| OrderedSequenceStepInputV1 {
                    step_ordinal: step.step_ordinal,
                    component_key: step.component_key.clone(),
                    predecessor_step_ordinal: step.predecessor_step_ordinal,
                    session_binding_key_schema: step.session_binding_key_schema.clone(),
                    session_binding_key_version: step.session_binding_key_version,
                    session_scope: step.session_scope,
                    interleaving_policy: step.interleaving_policy,
                    reset_policy: step.reset_policy,
                })
                .collect(),
            stopping_criteria_hash: persisted.stopping_criteria_hash.clone(),
            compiler_digest: persisted.compiler_digest.clone(),
            rule_digest: persisted.rule_digest.clone(),
            policy_snapshot_hash: persisted.policy_snapshot_hash.clone(),
        };
        let sealed = Self::compile(build_input)?;
        if sealed.persisted_snapshot() != persisted {
            return Err(VerificationContractError::PersistedMismatch(
                "sealed contract",
            ));
        }
        Ok(sealed)
    }
}

fn checked_count(value: usize, field: &'static str) -> Result<u32, VerificationContractError> {
    u32::try_from(value).map_err(|_| VerificationContractError::InvalidField(field))
}

fn compile_predicate_components(
    mut inputs: Vec<PredicateComponentInputV1>,
) -> Result<Vec<PredicateComponentV1>, VerificationContractError> {
    if inputs.is_empty() {
        return Err(VerificationContractError::InvalidCombinatorShape(
            "predicate_components must not be empty",
        ));
    }
    inputs.sort_by(|left, right| {
        left.semantic_key
            .as_bytes()
            .cmp(right.semantic_key.as_bytes())
    });

    let mut seen = BTreeSet::new();
    let mut components = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.into_iter().enumerate() {
        require_nonempty(&input.semantic_key, "predicate.semantic_key")?;
        require_nonempty(&input.predicate_schema, "predicate.predicate_schema")?;
        if input.predicate_version == 0 {
            return Err(VerificationContractError::InvalidField(
                "predicate.predicate_version",
            ));
        }
        require_hash(&input.prerequisite_hash, "predicate.prerequisite_hash")?;
        if !seen.insert(input.semantic_key.clone()) {
            return Err(VerificationContractError::DuplicateIdentity(format!(
                "predicate:{}",
                input.semantic_key
            )));
        }

        let ordinal = checked_count(index, "predicate.ordinal")?;
        let mut member_hash = DomainHashWriter::new("verification_predicate_component.v1");
        member_hash.u32(ordinal);
        member_hash.text(&input.semantic_key);
        member_hash.text(&input.predicate_schema);
        member_hash.u32(input.predicate_version);
        member_hash.field(input.normalized_arguments.canonical_bytes());
        member_hash.text(input.expected_polarity.as_str());
        member_hash.text(&input.prerequisite_hash);
        components.push(PredicateComponentV1 {
            ordinal,
            semantic_key: input.semantic_key,
            predicate_schema: input.predicate_schema,
            predicate_version: input.predicate_version,
            normalized_arguments: input.normalized_arguments,
            expected_polarity: input.expected_polarity,
            prerequisite_hash: input.prerequisite_hash,
            member_hash: member_hash.finish(),
        });
    }
    Ok(components)
}

fn compile_required_controls(
    mut inputs: Vec<VerificationControlInputV1>,
) -> Result<Vec<VerificationControlV1>, VerificationContractError> {
    inputs.sort_by(|left, right| {
        (
            left.control_id.as_bytes(),
            left.control_version,
            left.control_contract_hash.as_bytes(),
        )
            .cmp(&(
                right.control_id.as_bytes(),
                right.control_version,
                right.control_contract_hash.as_bytes(),
            ))
    });

    let mut seen_identity = BTreeSet::new();
    let mut controls = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.into_iter().enumerate() {
        require_nonempty(&input.control_id, "control.control_id")?;
        if input.control_version == 0 {
            return Err(VerificationContractError::InvalidField(
                "control.control_version",
            ));
        }
        require_hash(
            &input.control_contract_hash,
            "control.control_contract_hash",
        )?;
        if !seen_identity.insert((input.control_id.clone(), input.control_version)) {
            return Err(VerificationContractError::DuplicateIdentity(format!(
                "control:{}@{}",
                input.control_id, input.control_version
            )));
        }

        let ordinal = checked_count(index, "control.ordinal")?;
        let mut member_hash = DomainHashWriter::new("verification_control.v1");
        member_hash.u32(ordinal);
        member_hash.text(&input.control_id);
        member_hash.u32(input.control_version);
        member_hash.text(&input.control_contract_hash);
        controls.push(VerificationControlV1 {
            ordinal,
            control_id: input.control_id,
            control_version: input.control_version,
            control_contract_hash: input.control_contract_hash,
            member_hash: member_hash.finish(),
        });
    }
    Ok(controls)
}

fn compile_paired_bindings(
    mut inputs: Vec<PairedDifferentialBindingInputV1>,
    predicate_components: &[PredicateComponentV1],
    required_controls: &[VerificationControlV1],
) -> Result<Vec<PairedDifferentialBindingV1>, VerificationContractError> {
    if inputs.is_empty() {
        return Err(VerificationContractError::InvalidCombinatorShape(
            "paired_differential requires at least one pair",
        ));
    }
    inputs.sort_by(|left, right| left.pair_key.as_bytes().cmp(right.pair_key.as_bytes()));

    let predicate_keys = predicate_components
        .iter()
        .map(|component| component.semantic_key.as_str())
        .collect::<BTreeSet<_>>();
    let mut used_predicates = BTreeSet::new();
    let mut used_controls = BTreeSet::new();
    let mut seen_pairs = BTreeSet::new();
    let mut bindings = Vec::with_capacity(inputs.len());

    for (index, input) in inputs.into_iter().enumerate() {
        require_nonempty(&input.pair_key, "pair.pair_key")?;
        if !seen_pairs.insert(input.pair_key.clone()) {
            return Err(VerificationContractError::DuplicateIdentity(format!(
                "pair:{}",
                input.pair_key
            )));
        }
        if input.baseline_component_key == input.variant_component_key {
            return Err(VerificationContractError::InvalidCombinatorShape(
                "pair baseline and variant must be distinct",
            ));
        }
        for component_key in [
            input.baseline_component_key.as_str(),
            input.variant_component_key.as_str(),
        ] {
            if !predicate_keys.contains(component_key) {
                return Err(VerificationContractError::InvalidReference(
                    "pair component",
                ));
            }
            if !used_predicates.insert(component_key.to_owned()) {
                return Err(VerificationContractError::InvalidCombinatorShape(
                    "a predicate component occurs in more than one pair role",
                ));
            }
        }

        require_nonempty(&input.required_control_id, "pair.required_control_id")?;
        if input.required_control_version == 0 {
            return Err(VerificationContractError::InvalidField(
                "pair.required_control_version",
            ));
        }
        require_hash(
            &input.required_control_contract_hash,
            "pair.required_control_contract_hash",
        )?;
        let control = required_controls
            .iter()
            .find(|control| {
                control.control_id == input.required_control_id
                    && control.control_version == input.required_control_version
                    && control.control_contract_hash == input.required_control_contract_hash
            })
            .ok_or(VerificationContractError::InvalidReference(
                "pair required control identity/version/hash",
            ))?;
        used_controls.insert(control.member_hash.clone());

        require_nonempty(&input.comparator_rule_id, "pair.comparator_rule_id")?;
        if input.comparator_rule_version == 0 {
            return Err(VerificationContractError::InvalidField(
                "pair.comparator_rule_version",
            ));
        }
        require_hash(&input.comparator_rule_digest, "pair.comparator_rule_digest")?;

        let ordinal = checked_count(index, "pair.ordinal")?;
        let mut member_hash = DomainHashWriter::new("verification_paired_differential.v1");
        member_hash.u32(ordinal);
        member_hash.text(&input.pair_key);
        member_hash.text(&input.baseline_component_key);
        member_hash.text(&input.variant_component_key);
        member_hash.text(&input.required_control_id);
        member_hash.u32(input.required_control_version);
        member_hash.text(&input.required_control_contract_hash);
        member_hash.text(&control.member_hash);
        member_hash.text(&input.comparator_rule_id);
        member_hash.u32(input.comparator_rule_version);
        member_hash.text(&input.comparator_rule_digest);
        bindings.push(PairedDifferentialBindingV1 {
            ordinal,
            pair_key: input.pair_key,
            baseline_component_key: input.baseline_component_key,
            variant_component_key: input.variant_component_key,
            required_control_id: input.required_control_id,
            required_control_version: input.required_control_version,
            required_control_contract_hash: input.required_control_contract_hash,
            required_control_member_hash: control.member_hash.clone(),
            comparator_rule_id: input.comparator_rule_id,
            comparator_rule_version: input.comparator_rule_version,
            comparator_rule_digest: input.comparator_rule_digest,
            member_hash: member_hash.finish(),
        });
    }

    if used_predicates.len() != predicate_keys.len() {
        return Err(VerificationContractError::InvalidCombinatorShape(
            "paired_differential must pair every predicate exactly once",
        ));
    }
    if used_controls.len() != required_controls.len() {
        return Err(VerificationContractError::InvalidCombinatorShape(
            "paired_differential contains an unreferenced required control",
        ));
    }
    Ok(bindings)
}

fn compile_ordered_steps(
    mut inputs: Vec<OrderedSequenceStepInputV1>,
    predicate_components: &[PredicateComponentV1],
) -> Result<Vec<OrderedSequenceStepV1>, VerificationContractError> {
    if inputs.len() < 2 {
        return Err(VerificationContractError::InvalidCombinatorShape(
            "ordered_sequence requires at least two steps",
        ));
    }
    inputs.sort_by_key(|step| step.step_ordinal);

    let predicate_keys = predicate_components
        .iter()
        .map(|component| component.semantic_key.as_str())
        .collect::<BTreeSet<_>>();
    let mut used_predicates = BTreeSet::new();
    let first = inputs
        .first()
        .expect("the length check guarantees an ordered step");
    require_nonempty(
        &first.session_binding_key_schema,
        "ordered.session_binding_key_schema",
    )?;
    if first.session_binding_key_version == 0 {
        return Err(VerificationContractError::InvalidField(
            "ordered.session_binding_key_version",
        ));
    }
    let session_binding_key_schema = first.session_binding_key_schema.clone();
    let session_binding_key_version = first.session_binding_key_version;
    let session_scope = first.session_scope;
    let interleaving_policy = first.interleaving_policy;
    let reset_policy = first.reset_policy;

    let mut steps = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.into_iter().enumerate() {
        let expected_ordinal = checked_count(index, "ordered.step_ordinal")?;
        if input.step_ordinal != expected_ordinal {
            return Err(VerificationContractError::InvalidCombinatorShape(
                "ordered step ordinals must be contiguous from zero",
            ));
        }
        let expected_predecessor = expected_ordinal.checked_sub(1);
        if input.predecessor_step_ordinal != expected_predecessor {
            return Err(VerificationContractError::InvalidCombinatorShape(
                "ordered predecessor must name the immediately preceding step",
            ));
        }
        if !predicate_keys.contains(input.component_key.as_str()) {
            return Err(VerificationContractError::InvalidReference(
                "ordered component",
            ));
        }
        if !used_predicates.insert(input.component_key.clone()) {
            return Err(VerificationContractError::InvalidCombinatorShape(
                "an ordered predicate component occurs more than once",
            ));
        }
        if input.session_binding_key_schema != session_binding_key_schema
            || input.session_binding_key_version != session_binding_key_version
            || input.session_scope != session_scope
            || input.interleaving_policy != interleaving_policy
            || input.reset_policy != reset_policy
        {
            return Err(VerificationContractError::InvalidCombinatorShape(
                "ordered steps must share one exact session binding and policy",
            ));
        }

        let mut step_hash = DomainHashWriter::new("verification_ordered_step.v1");
        step_hash.u32(input.step_ordinal);
        step_hash.text(&input.component_key);
        step_hash.bool(input.predecessor_step_ordinal.is_some());
        if let Some(predecessor) = input.predecessor_step_ordinal {
            step_hash.u32(predecessor);
        }
        step_hash.text(&input.session_binding_key_schema);
        step_hash.u32(input.session_binding_key_version);
        step_hash.text(input.session_scope.as_str());
        step_hash.text(input.interleaving_policy.as_str());
        step_hash.text(input.reset_policy.as_str());
        steps.push(OrderedSequenceStepV1 {
            step_ordinal: input.step_ordinal,
            component_key: input.component_key,
            predecessor_step_ordinal: input.predecessor_step_ordinal,
            session_binding_key_schema: input.session_binding_key_schema,
            session_binding_key_version: input.session_binding_key_version,
            session_scope: input.session_scope,
            interleaving_policy: input.interleaving_policy,
            reset_policy: input.reset_policy,
            step_hash: step_hash.finish(),
        });
    }

    if used_predicates.len() != predicate_keys.len() {
        return Err(VerificationContractError::InvalidCombinatorShape(
            "ordered_sequence must include every predicate exactly once",
        ));
    }
    Ok(steps)
}

impl VerificationContractV1 {
    pub fn compile(
        input: VerificationContractBuildInputV1,
    ) -> Result<Self, VerificationContractError> {
        if input.revision_id.is_nil() {
            return Err(VerificationContractError::InvalidField("revision_id"));
        }
        if input.objective_id.is_nil() {
            return Err(VerificationContractError::InvalidField("objective_id"));
        }
        require_hash(&input.revision_hash, "revision_hash")?;
        require_hash(&input.stopping_criteria_hash, "stopping_criteria_hash")?;
        require_hash(&input.compiler_digest, "compiler_digest")?;
        require_hash(&input.rule_digest, "rule_digest")?;
        require_hash(&input.policy_snapshot_hash, "policy_snapshot_hash")?;

        let VerificationContractBuildInputV1 {
            revision_id,
            revision_hash,
            objective_id,
            combinator,
            predicate_components,
            required_controls,
            paired_differential_bindings,
            ordered_steps,
            stopping_criteria_hash,
            compiler_digest,
            rule_digest,
            policy_snapshot_hash,
        } = input;

        let predicate_components = compile_predicate_components(predicate_components)?;
        let required_controls = compile_required_controls(required_controls)?;
        let (paired_differential_bindings, ordered_steps) = match combinator {
            ContractCombinatorV1::AllOf | ContractCombinatorV1::AnyOf => {
                if !paired_differential_bindings.is_empty() || !ordered_steps.is_empty() {
                    return Err(VerificationContractError::InvalidCombinatorShape(
                        "all_of/any_of cannot carry pair or ordered structure",
                    ));
                }
                (Vec::new(), Vec::new())
            }
            ContractCombinatorV1::PairedDifferential => {
                if !ordered_steps.is_empty() {
                    return Err(VerificationContractError::InvalidCombinatorShape(
                        "paired_differential cannot carry ordered steps",
                    ));
                }
                (
                    compile_paired_bindings(
                        paired_differential_bindings,
                        &predicate_components,
                        &required_controls,
                    )?,
                    Vec::new(),
                )
            }
            ContractCombinatorV1::OrderedSequence => {
                if !paired_differential_bindings.is_empty() {
                    return Err(VerificationContractError::InvalidCombinatorShape(
                        "ordered_sequence cannot carry differential pairs",
                    ));
                }
                (
                    Vec::new(),
                    compile_ordered_steps(ordered_steps, &predicate_components)?,
                )
            }
        };

        let predicate_count = checked_count(predicate_components.len(), "predicate_count")?;
        let predicate_set_hash = exact_set_hash(
            "verification_predicate_set.v1",
            &predicate_components
                .iter()
                .map(|value| value.member_hash.clone())
                .collect::<Vec<_>>(),
        );
        let required_control_count =
            checked_count(required_controls.len(), "required_control_count")?;
        let required_control_set_hash = exact_set_hash(
            "verification_control_set.v1",
            &required_controls
                .iter()
                .map(|value| value.member_hash.clone())
                .collect::<Vec<_>>(),
        );
        let explicit_no_required_control = required_controls.is_empty();
        let paired_differential_count = checked_count(
            paired_differential_bindings.len(),
            "paired_differential_count",
        )?;
        let paired_differential_set_hash = exact_set_hash(
            "verification_paired_differential_set.v1",
            &paired_differential_bindings
                .iter()
                .map(|value| value.member_hash.clone())
                .collect::<Vec<_>>(),
        );
        let ordered_step_count = checked_count(ordered_steps.len(), "ordered_step_count")?;
        let ordered_step_set_hash = exact_set_hash(
            "verification_ordered_step_set.v1",
            &ordered_steps
                .iter()
                .map(|value| value.step_hash.clone())
                .collect::<Vec<_>>(),
        );

        let mut contract_hash = DomainHashWriter::new(CONTRACT_SCHEMA);
        contract_hash.text(CONTRACT_SCHEMA);
        contract_hash.u32(CONTRACT_VERSION);
        contract_hash.uuid(revision_id);
        contract_hash.text(&revision_hash);
        contract_hash.uuid(objective_id);
        contract_hash.text(combinator.as_str());
        contract_hash.u32(predicate_count);
        contract_hash.text(&predicate_set_hash);
        contract_hash.u32(required_control_count);
        contract_hash.text(&required_control_set_hash);
        contract_hash.bool(explicit_no_required_control);
        contract_hash.u32(paired_differential_count);
        contract_hash.text(&paired_differential_set_hash);
        contract_hash.u32(ordered_step_count);
        contract_hash.text(&ordered_step_set_hash);
        contract_hash.text(&stopping_criteria_hash);
        contract_hash.text(&compiler_digest);
        contract_hash.text(&rule_digest);
        contract_hash.text(&policy_snapshot_hash);
        let contract_hash = contract_hash.finish();
        let contract_id_name =
            format!("{CONTRACT_SCHEMA}:{objective_id}:{policy_snapshot_hash}:{contract_hash}");
        let contract_id = Uuid::new_v5(&revision_id, contract_id_name.as_bytes());

        Ok(Self {
            contract_id,
            contract_schema: CONTRACT_SCHEMA.to_owned(),
            contract_version: CONTRACT_VERSION,
            revision_id,
            revision_hash,
            objective_id,
            combinator,
            predicate_components,
            predicate_count,
            predicate_set_hash,
            required_controls,
            required_control_count,
            required_control_set_hash,
            explicit_no_required_control,
            paired_differential_bindings,
            paired_differential_count,
            paired_differential_set_hash,
            ordered_steps,
            ordered_step_count,
            ordered_step_set_hash,
            stopping_criteria_hash,
            compiler_digest,
            rule_digest,
            policy_snapshot_hash,
            contract_hash,
        })
    }
}
