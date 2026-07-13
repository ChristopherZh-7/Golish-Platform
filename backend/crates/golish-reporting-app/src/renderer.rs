use std::collections::{BTreeMap, BTreeSet};

use golish_reporting_domain::ReportReadModel;
use uuid::Uuid;

use crate::NarrativeRenderOutput;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum NarrativeError {
    #[error("renderer changed the canonical claim set")]
    ClaimSetMismatch,
    #[error("renderer returned a narrative for another revision")]
    RevisionMismatch,
}

fn canonical_claim_ids(model: &ReportReadModel) -> BTreeSet<Uuid> {
    model
        .organization_sections
        .iter()
        .flat_map(|section| section.section.claims.iter().map(|claim| claim.claim_id))
        .collect()
}

pub fn apply_narrative(
    model: &mut ReportReadModel,
    output: NarrativeRenderOutput,
) -> Result<(), NarrativeError> {
    if output.revision_id != model.revision_id {
        return Err(NarrativeError::RevisionMismatch);
    }
    let expected = canonical_claim_ids(model);
    let actual = output
        .narratives_by_claim
        .iter()
        .map(|(claim_id, _)| *claim_id)
        .collect::<BTreeSet<_>>();
    if expected != actual || actual.len() != output.narratives_by_claim.len() {
        return Err(NarrativeError::ClaimSetMismatch);
    }
    let narratives = output
        .narratives_by_claim
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    for section in &mut model.organization_sections {
        let lines = section
            .section
            .claims
            .iter()
            .filter_map(|claim| narratives.get(&claim.claim_id))
            .cloned()
            .collect::<Vec<_>>();
        section.section.rendered_content = Some(lines.join("\n\n"));
    }
    Ok(())
}

pub fn deterministic_narrative(model: &ReportReadModel) -> NarrativeRenderOutput {
    NarrativeRenderOutput {
        revision_id: model.revision_id,
        narratives_by_claim: model
            .organization_sections
            .iter()
            .flat_map(|section| section.section.claims.iter())
            .map(|claim| {
                (
                    claim.claim_id,
                    format!("{} {} {}", claim.subject_ref, claim.predicate, claim.value),
                )
            })
            .collect(),
    }
}
