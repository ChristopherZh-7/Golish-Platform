use super::types::CandidateProposal;
use golish_core::hypothesis_semantic_key::{
    derive_root_id, initial_root_id, merge_root_id, split_root_id, validate_sha256,
    HypothesisSemanticKeyError, HypothesisSemanticKeyV1,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReducerDecision {
    AttachCurrent {
        root_id: Uuid,
        revision_id: Uuid,
    },
    CreateInitial {
        root_id: Uuid,
    },
    ReopenHistorical {
        root_id: Uuid,
        predecessor_revision_id: Uuid,
    },
    NoSemanticChange {
        root_id: Uuid,
        revision_id: Uuid,
    },
    ExplicitTransitionRequired {
        historical_root_id: Uuid,
    },
    Split {
        parent_root_id: Uuid,
        child_root_ids: Vec<Uuid>,
    },
    Merge {
        parent_root_ids: Vec<Uuid>,
        successor_root_id: Uuid,
    },
    Derive {
        source_root_id: Uuid,
        successor_root_id: Uuid,
    },
    NarrowSuccessor {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        successor_root_id: Uuid,
        covered_claim_component_set_hash: String,
    },
    RootIdCollision {
        computed_root_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerOperatorInputV1 {
    Semantic,
    Split {
        parent_root_id: Uuid,
    },
    Merge {
        parent_root_ids: Vec<Uuid>,
    },
    Derive {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        derivation_rule_hash: String,
    },
    NarrowSuccessor {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        covered_claim_component_set_hash: String,
    },
}

#[derive(Debug, Clone)]
pub struct ReducerProposal {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub semantic_key: HypothesisSemanticKeyV1,
    pub semantic_key_hash: String,
    pub initial_root_id: Uuid,
}

impl ReducerProposal {
    pub fn from_candidate(
        proposal: &CandidateProposal,
    ) -> Result<Self, HypothesisSemanticKeyError> {
        let key = HypothesisSemanticKeyV1::from_claim(proposal)?;
        Ok(Self {
            operation_id: proposal.operation_id,
            organization_id: proposal.organization_id,
            semantic_key_hash: key.hash()?,
            initial_root_id: initial_root_id(proposal.operation_id, &key)?,
            semantic_key: key,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReducerCatalog {
    operation_id: Uuid,
    organization_id: Uuid,
    current: BTreeMap<String, (Uuid, Uuid)>,
    historical: BTreeMap<String, HistoricalEntry>,
    root_ingredients: BTreeMap<Uuid, String>,
}

#[derive(Debug, Clone)]
struct HistoricalEntry {
    root_id: Uuid,
    revision_id: Uuid,
    root_has_current: bool,
    material_relation: bool,
}

impl ReducerCatalog {
    pub fn for_scope(operation_id: Uuid, organization_id: Uuid) -> Self {
        Self {
            operation_id,
            organization_id,
            current: BTreeMap::new(),
            historical: BTreeMap::new(),
            root_ingredients: BTreeMap::new(),
        }
    }

    pub fn with_current(
        mut self,
        semantic_key_hash: String,
        root_id: Uuid,
        revision_id: Uuid,
    ) -> Self {
        self.current
            .insert(semantic_key_hash, (root_id, revision_id));
        self
    }

    pub fn with_historical(
        mut self,
        semantic_key_hash: String,
        root_id: Uuid,
        revision_id: Uuid,
        root_has_current: bool,
        material_relation: bool,
    ) -> Self {
        self.historical.insert(
            semantic_key_hash,
            HistoricalEntry {
                root_id,
                revision_id,
                root_has_current,
                material_relation,
            },
        );
        self
    }

    pub fn with_root_ingredients(mut self, root_id: Uuid, ingredients_hash: String) -> Self {
        self.root_ingredients.insert(root_id, ingredients_hash);
        self
    }

    pub fn route(&self, proposal: &ReducerProposal) -> Result<ReducerDecision, ReducerError> {
        self.route_with_operator(proposal, &ReducerOperatorInputV1::Semantic)
    }

    pub fn route_with_operator(
        &self,
        proposal: &ReducerProposal,
        operator: &ReducerOperatorInputV1,
    ) -> Result<ReducerDecision, ReducerError> {
        if proposal.operation_id != self.operation_id
            || proposal.organization_id != self.organization_id
        {
            return Err(ReducerError::ScopeMismatch);
        }
        if let Some((root_id, revision_id)) = self.current.get(&proposal.semantic_key_hash) {
            return Ok(ReducerDecision::AttachCurrent {
                root_id: *root_id,
                revision_id: *revision_id,
            });
        }
        if let Some(historical) = self.historical.get(&proposal.semantic_key_hash) {
            if historical.root_has_current {
                return Ok(ReducerDecision::ExplicitTransitionRequired {
                    historical_root_id: historical.root_id,
                });
            }
            return Ok(if historical.material_relation {
                ReducerDecision::ReopenHistorical {
                    root_id: historical.root_id,
                    predecessor_revision_id: historical.revision_id,
                }
            } else {
                ReducerDecision::NoSemanticChange {
                    root_id: historical.root_id,
                    revision_id: historical.revision_id,
                }
            });
        }
        if let Some(existing_ingredients) = self.root_ingredients.get(&proposal.initial_root_id) {
            if existing_ingredients != &proposal.semantic_key_hash {
                return Ok(ReducerDecision::RootIdCollision {
                    computed_root_id: proposal.initial_root_id,
                });
            }
            return Ok(ReducerDecision::NoSemanticChange {
                root_id: proposal.initial_root_id,
                revision_id: Uuid::nil(),
            });
        }
        match operator {
            ReducerOperatorInputV1::Semantic => Ok(ReducerDecision::CreateInitial {
                root_id: proposal.initial_root_id,
            }),
            ReducerOperatorInputV1::Split { parent_root_id } => Ok(ReducerDecision::Split {
                parent_root_id: *parent_root_id,
                child_root_ids: vec![split_root_id(
                    proposal.operation_id,
                    &proposal.semantic_key,
                    *parent_root_id,
                )?],
            }),
            ReducerOperatorInputV1::Merge { parent_root_ids } => {
                let mut parent_root_ids = parent_root_ids.clone();
                parent_root_ids.sort_unstable();
                let successor_root_id = merge_root_id(
                    proposal.operation_id,
                    &proposal.semantic_key,
                    &parent_root_ids,
                )?;
                Ok(ReducerDecision::Merge {
                    parent_root_ids,
                    successor_root_id,
                })
            }
            ReducerOperatorInputV1::Derive {
                source_root_id,
                source_revision_id,
                derivation_rule_hash,
            } => Ok(ReducerDecision::Derive {
                source_root_id: *source_root_id,
                successor_root_id: derive_root_id(
                    proposal.operation_id,
                    &proposal.semantic_key,
                    *source_root_id,
                    *source_revision_id,
                    derivation_rule_hash,
                )?,
            }),
            ReducerOperatorInputV1::NarrowSuccessor {
                source_root_id,
                source_revision_id,
                covered_claim_component_set_hash,
            } => {
                validate_sha256(covered_claim_component_set_hash)?;
                Ok(ReducerDecision::NarrowSuccessor {
                    source_root_id: *source_root_id,
                    source_revision_id: *source_revision_id,
                    successor_root_id: derive_root_id(
                        proposal.operation_id,
                        &proposal.semantic_key,
                        *source_root_id,
                        *source_revision_id,
                        covered_claim_component_set_hash,
                    )?,
                    covered_claim_component_set_hash: covered_claim_component_set_hash.clone(),
                })
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReducerMutationSet {
    decisions: Vec<ReducerDecision>,
    mutation_set_hash: String,
}

impl ReducerMutationSet {
    pub fn decisions(&self) -> &[ReducerDecision] {
        &self.decisions
    }

    pub fn mutation_set_hash(&self) -> &str {
        &self.mutation_set_hash
    }
}

pub fn reduce_proposals(
    proposals: &[CandidateProposal],
    catalog: &ReducerCatalog,
) -> Result<ReducerMutationSet, ReducerError> {
    let mut canonical = BTreeMap::new();
    for proposal in proposals {
        let reduced = ReducerProposal::from_candidate(proposal)?;
        canonical
            .entry(reduced.semantic_key_hash.clone())
            .or_insert(reduced);
    }
    let decisions = canonical
        .values()
        .map(|proposal| catalog.route(proposal))
        .collect::<Result<Vec<_>, _>>()?;
    let mut hasher = Sha256::new();
    hasher.update(b"hypothesis_reducer_mutation_set.v1\0");
    for (semantic_hash, decision) in canonical.keys().zip(&decisions) {
        hasher.update((semantic_hash.len() as u64).to_be_bytes());
        hasher.update(semantic_hash.as_bytes());
        let decision = serde_json::to_vec(decision).map_err(ReducerError::Serialization)?;
        hasher.update((decision.len() as u64).to_be_bytes());
        hasher.update(&decision);
    }
    Ok(ReducerMutationSet {
        decisions,
        mutation_set_hash: format!("sha256:{}", hex_lower(&hasher.finalize())),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ReducerError {
    #[error(transparent)]
    Semantic(#[from] HypothesisSemanticKeyError),
    #[error("HYPOTHESIS_REDUCER_SCOPE_MISMATCH")]
    ScopeMismatch,
    #[error("failed to serialize canonical reducer decision: {0}")]
    Serialization(serde_json::Error),
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
