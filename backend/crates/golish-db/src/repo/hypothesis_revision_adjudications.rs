//! Revision-level adjudication persisted only under a live Plan A all-fresh guard.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{
    capability_execution_receipts::{
        with_all_fresh_tool_truth_authority_bundle, AllFreshToolTruthAuthorityBundle,
        CheckToolTruthAuthorityBundle, ToolTruthAuthorityBundleConsumerV1,
    },
    verification_campaigns::{
        conflict, exact_set_hash_on, json_hash_on, AUTHORITY_STALE, CONTRACT_INVALID,
    },
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct AdjudicateRevision {
    pub stable_request_id: Uuid,
    pub verification_plan_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub objective_outcome_set_seal_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub struct AdjudicateRevisionFromAuthority {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub generation_seal_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_plan_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRevisionAdjudication {
    pub revision_adjudication_id: Uuid,
    pub objective_outcome_set_seal_id: Uuid,
    pub tool_truth_authority_bundle_seal_id: Uuid,
    pub outcome: String,
    pub adjudication_hash: String,
    pub replayed: bool,
}

fn objective_outcome_set_replay_identity_matches(
    existing: &(Uuid, Uuid, Uuid, Uuid, Uuid, Uuid),
    request: &AdjudicateRevisionFromAuthority,
    project_scope_id: Uuid,
) -> bool {
    existing.1 == request.verification_plan_id
        && existing.2 == request.hypothesis_revision_id
        && existing.3 == request.operation_id
        && existing.4 == project_scope_id
        && existing.5 == request.organization_id
}

/// Server-selector compound for revision adjudication.  The latest objective
/// set is sealed with DB time, then revalidated against live heads inside the
/// all-fresh Tool Truth transaction before the adjudication is inserted.
pub async fn adjudicate_revision_from_current_authority(
    pool: &sqlx::PgPool,
    request: AdjudicateRevisionFromAuthority,
) -> Result<DurableRevisionAdjudication> {
    let project_scope_id: Uuid = sqlx::query_scalar(
        r#"SELECT operation.project_scope_id
             FROM operation_state operation
             JOIN operation_org_scope_snapshots scope
               ON scope.id=$2 AND scope.operation_id=operation.operation_id
              AND scope.project_scope_id=operation.project_scope_id
              AND scope.sealed_at IS NOT NULL
             JOIN operation_org_scope_units unit
               ON unit.snapshot_id=scope.id AND unit.organization_id=$3
             JOIN attack_hypothesis_verification_plans plan
               ON plan.plan_id=$6 AND plan.revision_id=$5 AND plan.sealed_at IS NOT NULL
             JOIN attack_hypothesis_revisions revision
               ON revision.revision_id=plan.revision_id
              AND revision.operation_id=operation.operation_id
              AND revision.organization_id=$3
             JOIN hypothesis_generation_members generation_member
               ON generation_member.revision_id=revision.revision_id
              AND generation_member.operation_id=operation.operation_id
              AND generation_member.organization_id=$3
             JOIN hypothesis_generation_seals generation_seal
               ON generation_seal.generation_id=generation_member.generation_id
              AND generation_seal.seal_id=$4
            WHERE operation.operation_id=$1"#,
    )
    .bind(request.operation_id)
    .bind(request.scope_snapshot_id)
    .bind(request.organization_id)
    .bind(request.generation_seal_id)
    .bind(request.hypothesis_revision_id)
    .bind(request.verification_plan_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let outcome_set_request_id = Uuid::new_v5(
        &request.stable_consumer_request_id,
        b"hypothesis-objective-outcome-set.v1",
    );
    let existing_outcome_set: Option<(Uuid, Uuid, Uuid, Uuid, Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT objective_outcome_set_seal_id,verification_plan_id,
                  hypothesis_revision_id,operation_id,project_scope_id,organization_id
             FROM hypothesis_objective_outcome_set_seals
            WHERE stable_request_id=$1 AND sealed_at IS NOT NULL"#,
    )
    .bind(outcome_set_request_id)
    .fetch_optional(pool)
    .await?;
    let objective_outcome_set_seal_id = if let Some(existing) = existing_outcome_set {
        if !objective_outcome_set_replay_identity_matches(&existing, &request, project_scope_id) {
            return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
        }
        existing.0
    } else {
        let cutoff_at: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT statement_timestamp()")
                .fetch_one(pool)
                .await?;
        super::hypothesis_objective_outcomes::seal_hypothesis_objective_outcome_set(
            pool,
            &super::hypothesis_objective_outcomes::SealObjectiveOutcomeSet {
                stable_request_id: outcome_set_request_id,
                verification_plan_id: request.verification_plan_id,
                hypothesis_revision_id: request.hypothesis_revision_id,
                operation_id: request.operation_id,
                project_scope_id,
                organization_id: request.organization_id,
                cutoff_at,
            },
        )
        .await?
    };
    let authority_request = CheckToolTruthAuthorityBundle {
        stable_consumer_request_id: request.stable_consumer_request_id,
        operation_id: request.operation_id,
        organization_id: request.organization_id,
        consumer_kind: ToolTruthAuthorityBundleConsumerV1::VerificationCampaign,
    };
    with_all_fresh_tool_truth_authority_bundle(pool, &authority_request, move |tx, authority| {
        Box::pin(async move {
            let stable_request_id = Uuid::new_v5(
                &request.stable_consumer_request_id,
                b"hypothesis-revision-adjudication.v1",
            );
            let replayed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM hypothesis_revision_adjudications WHERE stable_request_id=$1)",
            )
            .bind(stable_request_id)
            .fetch_one(&mut **tx)
            .await?;
            let revision_adjudication_id =
                adjudicate_hypothesis_revision_with_fresh_tool_truth(
                    tx,
                    authority,
                    &AdjudicateRevision {
                        stable_request_id,
                        verification_plan_id: request.verification_plan_id,
                        hypothesis_revision_id: request.hypothesis_revision_id,
                        objective_outcome_set_seal_id,
                        operation_id: request.operation_id,
                        project_scope_id,
                        organization_id: request.organization_id,
                    },
                )
                .await?;
            let (outcome, adjudication_hash): (String, String) = sqlx::query_as(
                "SELECT outcome,adjudication_hash FROM hypothesis_revision_adjudications WHERE revision_adjudication_id=$1",
            )
            .bind(revision_adjudication_id)
            .fetch_one(&mut **tx)
            .await?;
            Ok(DurableRevisionAdjudication {
                revision_adjudication_id,
                objective_outcome_set_seal_id,
                tool_truth_authority_bundle_seal_id: authority.bundle_seal_id(),
                outcome,
                adjudication_hash,
                replayed,
            })
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objective_outcome_set_replay_reuses_only_the_exact_authority_identity() {
        let request = AdjudicateRevisionFromAuthority {
            stable_consumer_request_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            generation_seal_id: Uuid::new_v4(),
            hypothesis_revision_id: Uuid::new_v4(),
            verification_plan_id: Uuid::new_v4(),
        };
        let project_scope_id = Uuid::new_v4();
        let existing = (
            Uuid::new_v4(),
            request.verification_plan_id,
            request.hypothesis_revision_id,
            request.operation_id,
            project_scope_id,
            request.organization_id,
        );
        assert!(objective_outcome_set_replay_identity_matches(
            &existing,
            &request,
            project_scope_id
        ));

        let mut foreign = existing;
        foreign.5 = Uuid::new_v4();
        assert!(!objective_outcome_set_replay_identity_matches(
            &foreign,
            &request,
            project_scope_id
        ));
    }
}

pub async fn adjudicate_hypothesis_revision_with_fresh_tool_truth(
    tx: &mut Transaction<'_, Postgres>,
    authority: &AllFreshToolTruthAuthorityBundle<'_>,
    command: &AdjudicateRevision,
) -> Result<Uuid> {
    if authority.checked().operation_id() != command.operation_id
        || authority.checked().organization_id() != command.organization_id
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let outcome_set_exact: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
              SELECT 1 FROM hypothesis_objective_outcome_set_seals outcome_set
               WHERE outcome_set.objective_outcome_set_seal_id=$1
                 AND outcome_set.verification_plan_id=$2
                 AND outcome_set.hypothesis_revision_id=$3
                 AND outcome_set.operation_id=$4 AND outcome_set.project_scope_id=$5
                 AND outcome_set.organization_id=$6 AND outcome_set.sealed_at IS NOT NULL
                 AND outcome_set.member_count=(
                     SELECT plan.objective_count FROM attack_hypothesis_verification_plans plan
                      WHERE plan.plan_id=$2 AND plan.revision_id=$3
                 )
                 AND NOT EXISTS(
                     SELECT 1
                       FROM hypothesis_objective_outcome_set_members member
                       LEFT JOIN hypothesis_objective_outcome_heads head
                         ON head.verification_plan_id=member.verification_plan_id
                        AND head.verification_objective_id=member.verification_objective_id
                      WHERE member.objective_outcome_set_seal_id=$1
                        AND (head.current_outcome_id IS DISTINCT FROM member.selected_current_outcome_id
                             OR head.current_ordinal IS DISTINCT FROM member.selected_current_ordinal
                             OR EXISTS(
                                 SELECT 1 FROM verification_authority_quarantine_events quarantine
                                  WHERE quarantine.objective_outcome_receipt_id=
                                        member.selected_current_outcome_id
                             ))
                 )
           )"#,
    )
    .bind(command.objective_outcome_set_seal_id)
    .bind(command.verification_plan_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    if !outcome_set_exact {
        return Err(conflict(AUTHORITY_STALE));
    }
    let path_authority: (bool, bool, i64) = sqlx::query_as(
        r#"WITH selected AS (
               SELECT set_member.verification_objective_id,receipt.outcome
                 FROM hypothesis_objective_outcome_set_members set_member
                 JOIN hypothesis_objective_outcome_receipts receipt
                   ON receipt.objective_outcome_receipt_id=set_member.selected_current_outcome_id
                WHERE set_member.objective_outcome_set_seal_id=$1
           ), path_stats AS (
               SELECT path.path_id,path.member_count,
                      COUNT(*) FILTER(WHERE selected.outcome='proof') AS proof_count,
                      COUNT(*) FILTER(
                          WHERE path_member.role='required_proof_and_path_falsifier'
                            AND selected.outcome='refutation'
                      ) AS falsifier_count
                 FROM attack_hypothesis_verification_plan_paths path
                 JOIN attack_hypothesis_verification_plan_path_members path_member
                   ON path_member.path_id=path.path_id AND path_member.plan_id=path.plan_id
                 JOIN attack_hypothesis_verification_plan_objectives objective
                   ON objective.plan_objective_id=path_member.plan_objective_id
                  AND objective.plan_id=path.plan_id
                 JOIN selected ON selected.verification_objective_id=objective.objective_id
                WHERE path.plan_id=$2
                GROUP BY path.path_id,path.member_count
           )
           SELECT COALESCE(BOOL_OR(proof_count=member_count),FALSE),
                  COALESCE(BOOL_AND(falsifier_count>0),FALSE),COUNT(*)
             FROM path_stats"#,
    )
    .bind(command.objective_outcome_set_seal_id)
    .bind(command.verification_plan_id)
    .fetch_one(&mut **tx)
    .await?;
    if path_authority.2 == 0 {
        return Err(conflict(AUTHORITY_STALE));
    }
    let outcome = if path_authority.0 {
        "verified"
    } else if path_authority.1 {
        "refuted"
    } else {
        "nonterminal"
    };
    let unresolved_set_hash = if outcome == "nonterminal" {
        let member_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash FROM hypothesis_objective_outcome_set_members
                WHERE objective_outcome_set_seal_id=$1 ORDER BY member_ordinal"#,
        )
        .bind(command.objective_outcome_set_seal_id)
        .fetch_all(&mut **tx)
        .await?;
        Some(
            exact_set_hash_on(
                tx,
                "hypothesis_revision_unresolved_objective_set.v1",
                &member_hashes,
            )
            .await?,
        )
    } else {
        None
    };
    #[derive(sqlx::FromRow)]
    struct BundleRow {
        relevant_root_set_hash: String,
        member_set_hash: String,
        semantic_authority_bundle_hash: String,
        freshness_attestation_bundle_hash: String,
        temporal_validity_bundle_hash: String,
        temporal_validity_policy_set_hash: String,
        target_state_epoch_set_hash: String,
        observation_window_started_at: chrono::DateTime<chrono::Utc>,
        observation_window_completed_at: chrono::DateTime<chrono::Utc>,
        effective_valid_until: chrono::DateTime<chrono::Utc>,
    }
    let bundle = sqlx::query_as::<_, BundleRow>(
        r#"SELECT relevant_root_set_hash,member_set_hash,semantic_authority_bundle_hash,
                  freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                  temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                  observation_window_started_at,observation_window_completed_at,
                  effective_valid_until
             FROM tool_truth_authority_bundle_seals
            WHERE id=$1 AND operation_id=$2 AND organization_id=$3
              AND consumer_kind='verification_campaign' AND sealed_at IS NOT NULL
              AND consistent_fresh_count=member_count AND stale_or_invalid_count=0
              AND effective_valid_until>statement_timestamp() FOR SHARE"#,
    )
    .bind(authority.bundle_seal_id())
    .bind(command.operation_id)
    .bind(command.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let temporal_census_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "bundle_seal_id": authority.bundle_seal_id(),
            "member_set_hash": bundle.member_set_hash,
            "temporal_validity_bundle_hash": bundle.temporal_validity_bundle_hash,
            "target_state_epoch_set_hash": bundle.target_state_epoch_set_hash,
        }),
    )
    .await?;
    let adjudication_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "verification_plan_id": command.verification_plan_id,
            "hypothesis_revision_id": command.hypothesis_revision_id,
            "objective_outcome_set_seal_id": command.objective_outcome_set_seal_id,
            "tool_truth_authority_bundle_seal_id": authority.bundle_seal_id(),
            "outcome": outcome,
            "unresolved_set_hash": unresolved_set_hash,
            "semantic_authority_bundle_hash": bundle.semantic_authority_bundle_hash,
            "freshness_attestation_bundle_hash": bundle.freshness_attestation_bundle_hash,
            "temporal_validity_bundle_hash": bundle.temporal_validity_bundle_hash,
            "temporal_census_hash": temporal_census_hash,
            "target_state_epoch_set_hash": bundle.target_state_epoch_set_hash,
        }),
    )
    .await?;
    let adjudication_id = Uuid::new_v5(
        &command.stable_request_id,
        b"hypothesis-revision-adjudication.v1",
    );
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        r#"SELECT revision_adjudication_id,adjudication_hash
             FROM hypothesis_revision_adjudications WHERE stable_request_id=$1 FOR SHARE"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((existing_id, existing_hash)) = existing {
        if existing_hash == adjudication_hash {
            return Ok(existing_id);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    sqlx::query(
        r#"INSERT INTO hypothesis_revision_adjudications(
               revision_adjudication_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,objective_outcome_set_seal_id,operation_id,
               project_scope_id,organization_id,tool_truth_authority_bundle_seal_id,
               relevant_root_set_hash,member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_census_hash,temporal_policy_hash,target_epoch_set_hash,
               observation_window_start,observation_window_end,effective_valid_until,
               outcome,unresolved_set_hash,adjudication_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                    $18,$19,$20,$21,$22,$23)"#,
    )
    .bind(adjudication_id)
    .bind(command.stable_request_id)
    .bind(command.verification_plan_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.objective_outcome_set_seal_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(authority.bundle_seal_id())
    .bind(&bundle.relevant_root_set_hash)
    .bind(&bundle.member_set_hash)
    .bind(&bundle.semantic_authority_bundle_hash)
    .bind(&bundle.freshness_attestation_bundle_hash)
    .bind(&bundle.temporal_validity_bundle_hash)
    .bind(&temporal_census_hash)
    .bind(&bundle.temporal_validity_policy_set_hash)
    .bind(&bundle.target_state_epoch_set_hash)
    .bind(bundle.observation_window_started_at)
    .bind(bundle.observation_window_completed_at)
    .bind(bundle.effective_valid_until)
    .bind(outcome)
    .bind(&unresolved_set_hash)
    .bind(&adjudication_hash)
    .execute(&mut **tx)
    .await?;
    if outcome != "nonterminal" {
        terminalize_hypothesis_revision_on(
            tx,
            command,
            adjudication_id,
            outcome,
            &adjudication_hash,
        )
        .await?;
    }
    Ok(adjudication_id)
}

async fn terminalize_hypothesis_revision_on(
    tx: &mut Transaction<'_, Postgres>,
    command: &AdjudicateRevision,
    adjudication_id: Uuid,
    outcome: &str,
    adjudication_hash: &str,
) -> Result<Uuid> {
    if !matches!(outcome, "verified" | "refuted") {
        return Err(conflict(CONTRACT_INVALID));
    }
    if let Some(existing) = sqlx::query_scalar::<_, Uuid>(
        "SELECT revision_terminal_decision_id FROM hypothesis_revision_terminal_decisions WHERE revision_adjudication_id=$1",
    )
    .bind(adjudication_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(existing);
    }
    #[derive(sqlx::FromRow)]
    struct RevisionAuthority {
        root_id: Uuid,
        revision_ordinal: i32,
        semantic_key: serde_json::Value,
        semantic_key_hash: String,
        subject_kind: String,
        subject_identity_hash: String,
        target_live_id: Option<Uuid>,
        target_type_at_time: String,
        target_value_at_time: String,
        predicate_schema: String,
        predicate_version: i32,
        normalized_arguments: serde_json::Value,
        trust_boundary: String,
        polarity: String,
        structured_claim: serde_json::Value,
        assumptions: serde_json::Value,
        missing_facts: serde_json::Value,
        priority: i32,
        risk_impact: serde_json::Value,
        head_version: i64,
    }
    let predecessor = sqlx::query_as::<_, RevisionAuthority>(
        r#"SELECT revision.root_id,revision.revision_ordinal,revision.semantic_key,
                  revision.semantic_key_hash,revision.subject_kind,
                  revision.subject_identity_hash,revision.target_live_id,
                  revision.target_type_at_time,revision.target_value_at_time,
                  revision.predicate_schema,revision.predicate_version,
                  revision.normalized_arguments,revision.trust_boundary,
                  revision.polarity,revision.structured_claim,revision.assumptions,
                  revision.missing_facts,revision.priority,revision.risk_impact,
                  head.head_version
             FROM attack_hypothesis_revisions revision
             JOIN attack_hypothesis_heads head
               ON head.head_revision_id=revision.revision_id
              AND head.root_id=revision.root_id
              AND head.operation_id=revision.operation_id
              AND head.organization_id=revision.organization_id
              AND head.head_lifecycle_state='current'
            WHERE revision.revision_id=$1 AND revision.operation_id=$2
              AND revision.organization_id=$3
            FOR UPDATE OF head"#,
    )
    .bind(command.hypothesis_revision_id)
    .bind(command.operation_id)
    .bind(command.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let terminal_decision_id = Uuid::new_v5(
        &command.stable_request_id,
        b"hypothesis-revision-terminal-decision.v1",
    );
    let successor_revision_id = Uuid::new_v5(
        &terminal_decision_id,
        b"hypothesis-terminal-successor-revision.v1",
    );
    let event_id = Uuid::new_v5(&terminal_decision_id, b"hypothesis-terminal-state-event.v1");
    let finding_id = (outcome == "verified")
        .then(|| Uuid::new_v5(&terminal_decision_id, b"hypothesis-terminal-finding.v1"));
    let refutation_lineage_id = (outcome == "refuted")
        .then(|| Uuid::new_v5(&terminal_decision_id, b"hypothesis-terminal-refutation.v1"));
    let decision_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "contract_version": "hypothesis-revision-terminal-decision.v1",
            "revision_adjudication_id": adjudication_id,
            "hypothesis_revision_id": command.hypothesis_revision_id,
            "terminal_successor_revision_id": successor_revision_id,
            "decision": outcome,
            "finding_id": finding_id,
            "refutation_lineage_id": refutation_lineage_id,
            "state_event_id": event_id,
            "adjudication_hash": adjudication_hash,
        }),
    )
    .await?;
    let revision_ingredients_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "predecessor_revision_id": command.hypothesis_revision_id,
            "revision_ordinal": predecessor.revision_ordinal + 1,
            "semantic_key_hash": predecessor.semantic_key_hash,
            "epistemic_state": outcome,
            "lifecycle_state": "closed",
            "origin_decision_hash": decision_hash,
        }),
    )
    .await?;
    let successor_revision_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "revision_id": successor_revision_id,
            "revision_ingredients_hash": revision_ingredients_hash,
            "semantic_key_hash": predecessor.semantic_key_hash,
            "structured_claim": predecessor.structured_claim,
            "decision": outcome,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,predecessor_revision_id,
               revision_ordinal,semantic_key,semantic_key_hash,subject_kind,
               subject_identity_hash,target_live_id,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,structured_claim,
               assumptions,missing_facts,priority,risk_impact,origin_decision_hash,
               revision_ingredients_hash,revision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                    $18,$19,'closed','deferred',$20,$21,$22,$23,$24,$25,$26,$27)"#,
    )
    .bind(successor_revision_id)
    .bind(predecessor.root_id)
    .bind(command.operation_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(predecessor.revision_ordinal + 1)
    .bind(&predecessor.semantic_key)
    .bind(&predecessor.semantic_key_hash)
    .bind(&predecessor.subject_kind)
    .bind(&predecessor.subject_identity_hash)
    .bind(predecessor.target_live_id)
    .bind(&predecessor.target_type_at_time)
    .bind(&predecessor.target_value_at_time)
    .bind(&predecessor.predicate_schema)
    .bind(predecessor.predicate_version)
    .bind(&predecessor.normalized_arguments)
    .bind(&predecessor.trust_boundary)
    .bind(&predecessor.polarity)
    .bind(outcome)
    .bind(&predecessor.structured_claim)
    .bind(&predecessor.assumptions)
    .bind(&predecessor.missing_facts)
    .bind(predecessor.priority)
    .bind(&predecessor.risk_impact)
    .bind(&decision_hash)
    .bind(&revision_ingredients_hash)
    .bind(&successor_revision_hash)
    .execute(&mut **tx)
    .await?;

    clone_terminal_revision_authorities_on(
        tx,
        command.hypothesis_revision_id,
        successor_revision_id,
        &successor_revision_hash,
        &revision_ingredients_hash,
    )
    .await?;

    if let Some(finding_id) = finding_id {
        super::findings::insert_verified_hypothesis_with_executor(
            &mut **tx,
            &super::findings::HypothesisVerifiedFindingWrite {
                id: finding_id,
                title: format!("Verified hypothesis: {}", predecessor.target_value_at_time),
                target_live_id: predecessor.target_live_id,
                target_value_at_time: predecessor.target_value_at_time.clone(),
                description: format!(
                    "The sealed verification plan reached a verified terminal outcome for hypothesis {}.",
                    command.hypothesis_revision_id
                ),
                evidence: serde_json::json!([{
                    "kind": "hypothesis_revision_adjudication",
                    "id": adjudication_id,
                    "hash": adjudication_hash,
                    "objective_outcome_set_seal_id": command.objective_outcome_set_seal_id,
                }]),
            },
        )
        .await?;
    }
    let event_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "event_id": event_id,
            "predecessor_revision_id": command.hypothesis_revision_id,
            "successor_revision_id": successor_revision_id,
            "event_kind": outcome,
            "revision_terminal_decision_id": terminal_decision_id,
            "decision_hash": decision_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_state_events(
               event_id,operation_id,organization_id,root_id,predecessor_revision_id,
               successor_revision_id,event_kind,origin_authority,successor_epistemic_state,
               authority_receipt_kind,authority_receipt_id,authority_receipt_hash,
               event_hash,server_decision_id,server_decision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,'hypothesis_revision_adjudication',$7,
                    'revision_transition_decision',$8,$9,$10,$8,$9)"#,
    )
    .bind(event_id)
    .bind(command.operation_id)
    .bind(command.organization_id)
    .bind(predecessor.root_id)
    .bind(command.hypothesis_revision_id)
    .bind(successor_revision_id)
    .bind(outcome)
    .bind(terminal_decision_id)
    .bind(&decision_hash)
    .bind(event_hash)
    .execute(&mut **tx)
    .await?;
    let advanced = sqlx::query(
        r#"UPDATE attack_hypothesis_heads
              SET head_revision_id=$1,head_revision_hash=$2,
                  head_semantic_key_hash=$3,head_epistemic_state=$4,
                  head_lifecycle_state='closed',head_version=head_version+1,
                  updated_at=statement_timestamp()
            WHERE root_id=$5 AND operation_id=$6 AND organization_id=$7
              AND head_revision_id=$8 AND head_version=$9
              AND head_lifecycle_state='current'"#,
    )
    .bind(successor_revision_id)
    .bind(&successor_revision_hash)
    .bind(&predecessor.semantic_key_hash)
    .bind(outcome)
    .bind(predecessor.root_id)
    .bind(command.operation_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(predecessor.head_version)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if advanced != 1 {
        return Err(conflict(AUTHORITY_STALE));
    }
    sqlx::query(
        r#"INSERT INTO hypothesis_revision_terminal_decisions(
               revision_terminal_decision_id,stable_request_id,revision_adjudication_id,
               hypothesis_revision_id,terminal_successor_revision_id,operation_id,
               project_scope_id,organization_id,decision,finding_id,
               refutation_lineage_id,state_event_id,decision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(terminal_decision_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"hypothesis-revision-terminal-request.v1",
    ))
    .bind(adjudication_id)
    .bind(command.hypothesis_revision_id)
    .bind(successor_revision_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(outcome)
    .bind(finding_id)
    .bind(refutation_lineage_id)
    .bind(event_id)
    .bind(&decision_hash)
    .execute(&mut **tx)
    .await?;
    Ok(terminal_decision_id)
}

async fn clone_terminal_revision_authorities_on(
    tx: &mut Transaction<'_, Postgres>,
    source_revision_id: Uuid,
    successor_revision_id: Uuid,
    successor_revision_hash: &str,
    successor_revision_ingredients_hash: &str,
) -> Result<()> {
    // UUIDv5(old-id, successor-revision) gives a stable, collision-free map
    // while semantic/member hashes stay unchanged: terminalization changes
    // epistemic state, not the plan that was adjudicated.
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revision_sources(
               source_id,revision_id,ordinal,source_role,source_kind,source_ref,
               source_hash,member_hash)
           SELECT uuid_generate_v5($2,source_id::TEXT),$2,ordinal,source_role,
                  source_kind,source_ref,source_hash,member_hash
             FROM attack_hypothesis_revision_sources WHERE revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_objectives(
               objective_id,revision_id,objective_ordinal,objective_intent,
               stopping_criteria,stopping_criteria_hash,objective_hash)
           SELECT uuid_generate_v5($2,objective_id::TEXT),$2,objective_ordinal,
                  objective_intent,stopping_criteria,stopping_criteria_hash,objective_hash
             FROM attack_hypothesis_verification_objectives WHERE revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_claim_components(
               component_id,revision_id,revision_hash,component_ordinal,component_key,
               kind,canonical_fragment_hash,canonical_condition_hash,required,
               derivation_contract_version,derivation_contract_digest,member_hash)
           SELECT uuid_generate_v5($2,component_id::TEXT),$2,$3,component_ordinal,
                  component_key,kind,canonical_fragment_hash,canonical_condition_hash,
                  required,derivation_contract_version,derivation_contract_digest,member_hash
             FROM attack_hypothesis_claim_components WHERE revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .bind(successor_revision_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_contracts(
               contract_id,revision_id,revision_hash,objective_id,contract_schema,
               contract_version,combinator,predicate_count,predicate_set_hash,
               required_control_count,required_control_set_hash,explicit_no_required_control,
               paired_differential_count,paired_differential_set_hash,ordered_step_count,
               ordered_step_set_hash,stopping_criteria_hash,compiler_digest,rule_digest,
               policy_snapshot_hash,contract_hash)
           SELECT uuid_generate_v5($2,contract_id::TEXT),$2,$3,
                  uuid_generate_v5($2,objective_id::TEXT),contract_schema,contract_version,
                  combinator,predicate_count,predicate_set_hash,required_control_count,
                  required_control_set_hash,explicit_no_required_control,
                  paired_differential_count,paired_differential_set_hash,ordered_step_count,
                  ordered_step_set_hash,stopping_criteria_hash,compiler_digest,rule_digest,
                  policy_snapshot_hash,contract_hash
             FROM attack_hypothesis_verification_contracts WHERE revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .bind(successor_revision_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_objective_claim_components(
               binding_id,contract_id,revision_id,objective_id,claim_component_id,
               ordinal,component_member_hash,binding_member_hash)
           SELECT uuid_generate_v5($2,binding.binding_id::TEXT),
                  uuid_generate_v5($2,binding.contract_id::TEXT),$2,
                  uuid_generate_v5($2,binding.objective_id::TEXT),
                  uuid_generate_v5($2,binding.claim_component_id::TEXT),binding.ordinal,
                  binding.component_member_hash,binding.binding_member_hash
             FROM attack_hypothesis_verification_objective_claim_components binding
            WHERE binding.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_predicate_components(
               predicate_component_id,contract_id,ordinal,semantic_key,predicate_schema,
               predicate_version,normalized_arguments,normalized_arguments_hash,
               expected_polarity,prerequisite_hash,member_hash)
           SELECT uuid_generate_v5($2,predicate.predicate_component_id::TEXT),
                  uuid_generate_v5($2,predicate.contract_id::TEXT),predicate.ordinal,
                  predicate.semantic_key,predicate.predicate_schema,predicate.predicate_version,
                  predicate.normalized_arguments,predicate.normalized_arguments_hash,
                  predicate.expected_polarity,predicate.prerequisite_hash,predicate.member_hash
             FROM attack_hypothesis_verification_predicate_components predicate
             JOIN attack_hypothesis_verification_contracts contract
               ON contract.contract_id=predicate.contract_id
            WHERE contract.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_required_controls(
               required_control_id,contract_id,ordinal,control_id,control_version,
               control_contract_hash,member_hash)
           SELECT uuid_generate_v5($2,control.required_control_id::TEXT),
                  uuid_generate_v5($2,control.contract_id::TEXT),control.ordinal,
                  control.control_id,control.control_version,control.control_contract_hash,
                  control.member_hash
             FROM attack_hypothesis_verification_required_controls control
             JOIN attack_hypothesis_verification_contracts contract
               ON contract.contract_id=control.contract_id
            WHERE contract.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_pair_bindings(
               pair_binding_id,contract_id,ordinal,pair_key,baseline_component_id,
               baseline_component_key,variant_component_id,variant_component_key,
               required_control_member_id,required_control_id,required_control_version,
               required_control_contract_hash,required_control_member_hash,
               comparator_rule_id,comparator_rule_version,comparator_rule_digest,member_hash)
           SELECT uuid_generate_v5($2,pair.pair_binding_id::TEXT),
                  uuid_generate_v5($2,pair.contract_id::TEXT),pair.ordinal,pair.pair_key,
                  uuid_generate_v5($2,pair.baseline_component_id::TEXT),
                  pair.baseline_component_key,
                  uuid_generate_v5($2,pair.variant_component_id::TEXT),
                  pair.variant_component_key,
                  uuid_generate_v5($2,pair.required_control_member_id::TEXT),
                  pair.required_control_id,pair.required_control_version,
                  pair.required_control_contract_hash,pair.required_control_member_hash,
                  pair.comparator_rule_id,pair.comparator_rule_version,
                  pair.comparator_rule_digest,pair.member_hash
             FROM attack_hypothesis_verification_pair_bindings pair
             JOIN attack_hypothesis_verification_contracts contract
               ON contract.contract_id=pair.contract_id
            WHERE contract.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_ordered_steps(
               ordered_step_id,contract_id,step_ordinal,predicate_component_id,
               component_key,predecessor_step_ordinal,session_binding_key_schema,
               session_binding_key_version,session_scope,interleaving_policy,reset_policy,step_hash)
           SELECT uuid_generate_v5($2,step.ordered_step_id::TEXT),
                  uuid_generate_v5($2,step.contract_id::TEXT),step.step_ordinal,
                  uuid_generate_v5($2,step.predicate_component_id::TEXT),step.component_key,
                  step.predecessor_step_ordinal,step.session_binding_key_schema,
                  step.session_binding_key_version,step.session_scope,
                  step.interleaving_policy,step.reset_policy,step.step_hash
             FROM attack_hypothesis_verification_ordered_steps step
             JOIN attack_hypothesis_verification_contracts contract
               ON contract.contract_id=step.contract_id
            WHERE contract.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plans(
               plan_id,revision_id,revision_hash,plan_schema,plan_version,
               revision_ingredients_hash,required_claim_component_count,
               required_claim_component_set_hash,objective_count,objective_set_hash,
               proof_path_count,proof_path_set_hash,outer_aggregation_policy_version,
               outer_aggregation_policy_digest,plan_hash,sealed_at)
           SELECT uuid_generate_v5($2,plan_id::TEXT),$2,$3,plan_schema,plan_version,$4,
                  required_claim_component_count,required_claim_component_set_hash,
                  objective_count,objective_set_hash,proof_path_count,proof_path_set_hash,
                  outer_aggregation_policy_version,outer_aggregation_policy_digest,
                  plan_hash,statement_timestamp()
             FROM attack_hypothesis_verification_plans WHERE revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .bind(successor_revision_hash)
    .bind(successor_revision_ingredients_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plan_objectives(
               plan_objective_id,plan_id,revision_id,objective_id,
               verification_contract_id,ordinal,objective_hash,
               verification_contract_version,verification_contract_hash,
               claim_component_count,claim_component_set_hash,stopping_criteria_hash,
               outcome_requirement,member_hash)
           SELECT uuid_generate_v5($2,item.plan_objective_id::TEXT),
                  uuid_generate_v5($2,item.plan_id::TEXT),$2,
                  uuid_generate_v5($2,item.objective_id::TEXT),
                  uuid_generate_v5($2,item.verification_contract_id::TEXT),item.ordinal,
                  item.objective_hash,item.verification_contract_version,
                  item.verification_contract_hash,item.claim_component_count,
                  item.claim_component_set_hash,item.stopping_criteria_hash,
                  item.outcome_requirement,item.member_hash
             FROM attack_hypothesis_verification_plan_objectives item
            WHERE item.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plan_paths(
               path_id,plan_id,path_ordinal,path_key,member_count,member_set_hash,path_hash)
           SELECT uuid_generate_v5($2,path.path_id::TEXT),
                  uuid_generate_v5($2,path.plan_id::TEXT),path.path_ordinal,path.path_key,
                  path.member_count,path.member_set_hash,path.path_hash
             FROM attack_hypothesis_verification_plan_paths path
             JOIN attack_hypothesis_verification_plans plan ON plan.plan_id=path.plan_id
            WHERE plan.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plan_path_members(
               path_member_id,path_id,plan_id,plan_objective_id,
               plan_objective_member_hash,revision_id,member_ordinal,
               verification_contract_hash,claim_component_set_hash,role,
               falsifier_claim_component_member_hashes,falsifier_claim_component_count,
               falsifier_claim_component_set_hash,member_hash)
           SELECT uuid_generate_v5($2,member.path_member_id::TEXT),
                  uuid_generate_v5($2,member.path_id::TEXT),
                  uuid_generate_v5($2,member.plan_id::TEXT),
                  uuid_generate_v5($2,member.plan_objective_id::TEXT),
                  member.plan_objective_member_hash,$2,member.member_ordinal,
                  member.verification_contract_hash,member.claim_component_set_hash,
                  member.role,member.falsifier_claim_component_member_hashes,
                  member.falsifier_claim_component_count,
                  member.falsifier_claim_component_set_hash,member.member_hash
             FROM attack_hypothesis_verification_plan_path_members member
             JOIN attack_hypothesis_verification_plans plan ON plan.plan_id=member.plan_id
            WHERE plan.revision_id=$1"#,
    )
    .bind(source_revision_id)
    .bind(successor_revision_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
