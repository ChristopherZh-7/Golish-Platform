-- PostgreSQL 17 treats an unqualified PL/pgSQL variable that shares a name
-- with joined table columns as ambiguous. Keep the existing trigger contract
-- and replace only the local variable name used by the exact authority join.

CREATE OR REPLACE FUNCTION verification_validate_claim_component_outcome_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_contract_id UUID;
    expected_count BIGINT;
    actual_count BIGINT;
    invalid_count BIGINT;
BEGIN
    IF NEW.sealed_at IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT objective.verification_contract_id,objective.claim_component_count
      INTO STRICT expected_contract_id,expected_count
      FROM attack_hypothesis_verification_plan_objectives objective
     WHERE objective.plan_id=NEW.verification_plan_id
       AND objective.revision_id=NEW.hypothesis_revision_id
       AND objective.objective_id=NEW.verification_objective_id;
    IF NEW.campaign_id IS NOT NULL AND NOT EXISTS(
        SELECT 1 FROM verification_campaigns campaign
         WHERE campaign.campaign_id=NEW.campaign_id
           AND campaign.verification_plan_id=NEW.verification_plan_id
           AND campaign.hypothesis_revision_id=NEW.hypothesis_revision_id
           AND campaign.verification_objective_id=NEW.verification_objective_id
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_CLAIM_OUTCOME_CAMPAIGN_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),COUNT(*) FILTER(WHERE
               binding.binding_id IS NULL
               OR predicate.predicate_component_id IS NULL
               OR (member.campaign_coverage_member_id IS NOT NULL AND wave.wave_coverage_member_id IS NULL)
               OR (member.oracle_census_member_id IS NOT NULL AND oracle_member.oracle_census_member_id IS NULL))
      INTO actual_count,invalid_count
      FROM hypothesis_objective_claim_component_outcome_members member
      LEFT JOIN attack_hypothesis_verification_objective_claim_components binding
        ON binding.contract_id=expected_contract_id
       AND binding.revision_id=member.hypothesis_revision_id
       AND binding.claim_component_id=member.claim_component_id
       AND binding.component_member_hash=member.claim_component_hash
      LEFT JOIN attack_hypothesis_verification_predicate_components predicate
        ON predicate.predicate_component_id=member.predicate_component_id
       AND predicate.contract_id=expected_contract_id
      LEFT JOIN verification_campaign_coverage_members campaign_member
        ON campaign_member.campaign_coverage_member_id=member.campaign_coverage_member_id
      LEFT JOIN verification_wave_coverage_members wave
        ON wave.wave_coverage_member_id=campaign_member.wave_coverage_member_id
       AND wave.claim_component_id=member.claim_component_id
       AND wave.claim_component_hash=member.claim_component_hash
       AND wave.predicate_component_id=member.predicate_component_id
      LEFT JOIN verification_oracle_census_members oracle_member
        ON oracle_member.oracle_census_member_id=member.oracle_census_member_id
       AND oracle_member.campaign_coverage_member_id=member.campaign_coverage_member_id
       AND oracle_member.predicate_component_id=member.predicate_component_id
     WHERE member.claim_component_outcome_seal_id=NEW.claim_component_outcome_seal_id;
    IF actual_count<>expected_count OR invalid_count<>0 THEN
        RAISE EXCEPTION 'VERIFICATION_CLAIM_COMPONENT_OUTCOME_EXACT_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
