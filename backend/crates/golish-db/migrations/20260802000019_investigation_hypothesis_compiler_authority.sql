-- Install a first-class unified Investigation origin for canonical Hypothesis
-- Registry generations. The model-facing payload is never stored here as a
-- registry mutation: the host compiler must bind every decision to the exact
-- Investigation analysis work/binding and to durable frozen proof members.

ALTER TABLE investigation_pentagi_delegation_census_seals
    ADD CONSTRAINT investigation_pentagi_delegation_census_compiler_authority_unique
    UNIQUE(census_seal_id,task_plan_id,primary_worker_run_id,seal_sha256);

CREATE TABLE investigation_hypothesis_compilation_decisions (
    decision_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    binding_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    work_id UUID NOT NULL,
    candidate_snapshot_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    task_plan_id UUID NOT NULL UNIQUE,
    delegation_census_seal_id UUID NOT NULL UNIQUE,
    primary_worker_run_id UUID NOT NULL,
    delegation_census_sha256 TEXT NOT NULL CHECK(delegation_census_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    cognitive_output_schema TEXT NOT NULL
        CHECK(cognitive_output_schema='investigation_cognitive_output.v1'),
    proposal_count BIGINT NOT NULL CHECK(proposal_count>0),
    proposal_set_sha256 TEXT NOT NULL CHECK(proposal_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    action_intent_count BIGINT NOT NULL CHECK(action_intent_count>=0),
    action_intent_set_sha256 TEXT NOT NULL CHECK(action_intent_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    proof_member_count BIGINT NOT NULL CHECK(proof_member_count>0),
    proof_member_set_sha256 TEXT NOT NULL CHECK(proof_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    mutation_count BIGINT NOT NULL CHECK(mutation_count>0),
    mutation_set_sha256 TEXT NOT NULL CHECK(mutation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    claim_component_set_sha256 TEXT NOT NULL CHECK(claim_component_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    verification_contract_set_sha256 TEXT NOT NULL CHECK(verification_contract_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    verification_plan_set_sha256 TEXT NOT NULL CHECK(verification_plan_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    generation_transition_set_sha256 TEXT NOT NULL CHECK(generation_transition_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    decision_sha256 TEXT NOT NULL CHECK(decision_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(decision_id,operation_id,organization_id),
    UNIQUE(decision_id,operation_id,organization_id,decision_sha256),
    FOREIGN KEY(
        binding_id,authority_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id,work_id,candidate_snapshot_id,
        analysis_attempt_id
    ) REFERENCES investigation_analysis_attempt_bindings(
        binding_id,authority_id,operation_id,stage_execution_id,
        stage_run_unit_id,organization_id,work_id,candidate_snapshot_id,
        analysis_attempt_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        delegation_census_seal_id,task_plan_id,primary_worker_run_id,
        delegation_census_sha256
    ) REFERENCES investigation_pentagi_delegation_census_seals(
        census_seal_id,task_plan_id,primary_worker_run_id,seal_sha256
    ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_validate_hypothesis_compilation_decision()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS(
        SELECT 1
          FROM investigation_pentagi_task_plans plan
         WHERE plan.task_plan_id=NEW.task_plan_id
           AND plan.authority_id=NEW.authority_id
           AND plan.operation_id=NEW.operation_id
           AND plan.stage_execution_id=NEW.stage_execution_id
           AND plan.stage_run_unit_id=NEW.stage_run_unit_id
           AND plan.organization_id=NEW.organization_id
           AND plan.subject_kind='analysis_attempt'
           AND plan.subject_id=NEW.analysis_attempt_id
           AND plan.status='sealed'
           AND EXISTS(
               SELECT 1 FROM investigation_pentagi_pipeline_events event
                WHERE event.task_plan_id=plan.task_plan_id
                  AND event.event_kind='primary_synthesis'
                  AND event.actor_worker_run_id=NEW.primary_worker_run_id
           )
         FOR SHARE OF plan
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPILER_REQUIRES_SEALED_PRIMARY_SYNTHESIS'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_hypothesis_compilation_decision_validate
BEFORE INSERT ON investigation_hypothesis_compilation_decisions
FOR EACH ROW EXECUTE FUNCTION investigation_validate_hypothesis_compilation_decision();

CREATE TABLE investigation_hypothesis_compilation_members (
    compilation_member_id UUID PRIMARY KEY,
    decision_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    proposal_id UUID NOT NULL,
    canonical_proposal JSONB NOT NULL CHECK(jsonb_typeof(canonical_proposal)='object'),
    proposal_sha256 TEXT NOT NULL CHECK(proposal_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    route_kind TEXT NOT NULL CHECK(route_kind IN('create_initial','attach_current')),
    root_id UUID NOT NULL,
    predecessor_revision_id UUID,
    successor_revision_id UUID NOT NULL,
    semantic_key_sha256 TEXT NOT NULL CHECK(semantic_key_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    successor_epistemic_state TEXT NOT NULL CHECK(successor_epistemic_state IN(
        'proposed','supported','contested','inconclusive'
    )),
    origin_decision_sha256 TEXT NOT NULL CHECK(origin_decision_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    generation_transition_sha256 TEXT NOT NULL CHECK(generation_transition_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK(
        (route_kind='create_initial' AND predecessor_revision_id IS NULL)
        OR route_kind='attach_current'
    ),
    UNIQUE(decision_id,ordinal),
    UNIQUE(decision_id,proposal_id),
    UNIQUE(decision_id,root_id),
    UNIQUE(compilation_member_id,origin_decision_sha256),
    UNIQUE(compilation_member_id,decision_id,operation_id,organization_id,successor_revision_id),
    FOREIGN KEY(decision_id,operation_id,organization_id)
        REFERENCES investigation_hypothesis_compilation_decisions(
            decision_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(successor_revision_id,root_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id
        ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE investigation_hypothesis_compilation_proof_members (
    proof_member_id UUID PRIMARY KEY,
    decision_id UUID NOT NULL,
    compilation_member_id UUID NOT NULL,
    -- Repeated solely to make the compound ownership FK exact without a
    -- latest-row lookup.
    successor_revision_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    candidate_snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    snapshot_input_id UUID NOT NULL,
    chunk_id UUID NOT NULL,
    source_role TEXT NOT NULL CHECK(source_role IN('support','contradiction','authorization_use')),
    source_sha256 TEXT NOT NULL CHECK(source_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(decision_id,ordinal),
    UNIQUE(decision_id,compilation_member_id,snapshot_input_id,chunk_id,source_role),
    FOREIGN KEY(decision_id,operation_id,organization_id)
        REFERENCES investigation_hypothesis_compilation_decisions(
            decision_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(compilation_member_id,decision_id,operation_id,organization_id,successor_revision_id)
        REFERENCES investigation_hypothesis_compilation_members(
            compilation_member_id,decision_id,operation_id,organization_id,successor_revision_id
        ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(snapshot_input_id,candidate_snapshot_id)
        REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id,snapshot_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(chunk_id)
        REFERENCES candidate_analysis_input_chunk_census_members(chunk_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION investigation_validate_hypothesis_compilation_proof()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_snapshot_id UUID;
    expected_source_sha256 TEXT;
BEGIN
    SELECT decision.candidate_snapshot_id
      INTO STRICT expected_snapshot_id
      FROM investigation_hypothesis_compilation_decisions decision
     WHERE decision.decision_id=NEW.decision_id
       AND decision.operation_id=NEW.operation_id
       AND decision.organization_id=NEW.organization_id
     FOR SHARE;
    IF expected_snapshot_id<>NEW.candidate_snapshot_id THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPILER_PROOF_SNAPSHOT_MISMATCH' USING ERRCODE='23514';
    END IF;
    SELECT input.source_content_hash
      INTO STRICT expected_source_sha256
      FROM candidate_analysis_snapshot_inputs input
      JOIN candidate_analysis_input_chunk_census_members chunk
        ON chunk.chunk_id=NEW.chunk_id
       AND chunk.snapshot_input_id=input.snapshot_input_id
       AND chunk.snapshot_id=input.snapshot_id
     WHERE input.snapshot_input_id=NEW.snapshot_input_id
       AND input.snapshot_id=NEW.candidate_snapshot_id
     FOR SHARE OF input,chunk;
    IF expected_source_sha256<>NEW.source_sha256 THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPILER_PROOF_SOURCE_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_hypothesis_compilation_proof_validate
BEFORE INSERT ON investigation_hypothesis_compilation_proof_members
FOR EACH ROW EXECUTE FUNCTION investigation_validate_hypothesis_compilation_proof();

CREATE FUNCTION investigation_enforce_hypothesis_compilation_exact_sets()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    decision investigation_hypothesis_compilation_decisions%ROWTYPE;
    actual_mutation_count BIGINT;
    actual_mutation_set TEXT;
    actual_proposal_count BIGINT;
    actual_proposal_set TEXT;
    actual_proof_count BIGINT;
    actual_proof_set TEXT;
BEGIN
    SELECT * INTO STRICT decision
      FROM investigation_hypothesis_compilation_decisions
     WHERE decision_id=NEW.decision_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_hypothesis_compilation_members.v1',
               COALESCE(array_agg(member_sha256 ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO actual_mutation_count,actual_mutation_set
      FROM investigation_hypothesis_compilation_members
     WHERE decision_id=decision.decision_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_candidate_proposals.v1',
               COALESCE(array_agg(proposal_sha256 ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO actual_proposal_count,actual_proposal_set
      FROM investigation_hypothesis_compilation_members
     WHERE decision_id=decision.decision_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_hypothesis_compilation_proofs.v1',
               COALESCE(array_agg(member_sha256 ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO actual_proof_count,actual_proof_set
      FROM investigation_hypothesis_compilation_proof_members
     WHERE decision_id=decision.decision_id;
    IF actual_proposal_count<>decision.proposal_count
       OR actual_proposal_set<>decision.proposal_set_sha256
       OR actual_mutation_count<>decision.mutation_count
       OR actual_mutation_set<>decision.mutation_set_sha256
       OR actual_proof_count<>decision.proof_member_count
       OR actual_proof_set<>decision.proof_member_set_sha256
    THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPILER_EXACT_SET_REQUIRED' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_hypothesis_compilation_decision_exact_set
AFTER INSERT ON investigation_hypothesis_compilation_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_enforce_hypothesis_compilation_exact_sets();

CREATE CONSTRAINT TRIGGER investigation_hypothesis_compilation_member_exact_set
AFTER INSERT ON investigation_hypothesis_compilation_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_enforce_hypothesis_compilation_exact_sets();

CREATE CONSTRAINT TRIGGER investigation_hypothesis_compilation_proof_exact_set
AFTER INSERT ON investigation_hypothesis_compilation_proof_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_enforce_hypothesis_compilation_exact_sets();

-- Existing generations remain Candidate-origin. New rows must name exactly one
-- origin, so old readers remain valid while unified Investigation gets a
-- first-class compiler authority instead of manufacturing a Candidate fence.
ALTER TABLE hypothesis_generations
    ADD COLUMN investigation_compilation_decision_id UUID UNIQUE;

ALTER TABLE hypothesis_generations
    ALTER COLUMN candidate_gate_decision_id DROP NOT NULL;

ALTER TABLE hypothesis_generations
    ADD CONSTRAINT hypothesis_generations_exact_one_compiler_origin CHECK(
        (candidate_gate_decision_id IS NULL)<>(investigation_compilation_decision_id IS NULL)
    ),
    ADD CONSTRAINT hypothesis_generations_investigation_compiler_fk FOREIGN KEY(
        investigation_compilation_decision_id,operation_id,organization_id
    ) REFERENCES investigation_hypothesis_compilation_decisions(
        decision_id,operation_id,organization_id
    ) ON DELETE RESTRICT;

ALTER TABLE attack_hypothesis_state_events
    DROP CONSTRAINT attack_hypothesis_state_events_origin_authority_check,
    DROP CONSTRAINT attack_hypothesis_state_events_authority_receipt_kind_check,
    ADD CONSTRAINT attack_hypothesis_state_events_origin_authority_check CHECK(
        origin_authority IN(
            'candidate_analysis','investigation_compiler','server_validator',
            'hypothesis_revision_adjudication'
        )
    ),
    ADD CONSTRAINT attack_hypothesis_state_events_authority_receipt_kind_check CHECK(
        authority_receipt_kind IN(
            'candidate_gate_decision','investigation_compilation_decision',
            'server_validation','revision_transition_decision'
        )
    );

CREATE FUNCTION investigation_enforce_hypothesis_compiler_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.origin_authority='investigation_compiler'
       AND (
           NEW.authority_receipt_kind<>'investigation_compilation_decision'
           OR NOT EXISTS(
               SELECT 1
                 FROM investigation_hypothesis_compilation_members member
                WHERE member.compilation_member_id=NEW.authority_receipt_id
                  AND member.operation_id=NEW.operation_id
                  AND member.organization_id=NEW.organization_id
                  AND member.root_id=NEW.root_id
                  AND member.predecessor_revision_id IS NOT DISTINCT FROM NEW.predecessor_revision_id
                  AND member.successor_revision_id=NEW.successor_revision_id
                  AND member.successor_epistemic_state=NEW.successor_epistemic_state
                  AND member.origin_decision_sha256=NEW.authority_receipt_hash
           )
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPILER_DECISION_REQUIRED' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER attack_hypothesis_state_events_investigation_compiler_authority
AFTER INSERT ON attack_hypothesis_state_events
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_enforce_hypothesis_compiler_event();

CREATE TABLE investigation_hypothesis_canonical_apply_receipts (
    apply_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    decision_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_id UUID NOT NULL UNIQUE,
    generation_seal_id UUID NOT NULL UNIQUE,
    projection_outbox_batch_id UUID NOT NULL UNIQUE,
    revision_count BIGINT NOT NULL CHECK(revision_count>=0),
    revision_set_sha256 TEXT NOT NULL CHECK(revision_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(apply_receipt_id,operation_id,organization_id),
    FOREIGN KEY(decision_id,operation_id,organization_id)
        REFERENCES investigation_hypothesis_compilation_decisions(
            decision_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(generation_seal_id,generation_id)
        REFERENCES hypothesis_generation_seals(seal_id,generation_id) ON DELETE RESTRICT,
    FOREIGN KEY(projection_outbox_batch_id,operation_id)
        REFERENCES investigation_projection_outbox_batches(batch_id,operation_id) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_hypothesis_compilation_decisions_append_only
BEFORE UPDATE OR DELETE ON investigation_hypothesis_compilation_decisions
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TRIGGER investigation_hypothesis_compilation_members_append_only
BEFORE UPDATE OR DELETE ON investigation_hypothesis_compilation_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TRIGGER investigation_hypothesis_compilation_proof_members_append_only
BEFORE UPDATE OR DELETE ON investigation_hypothesis_compilation_proof_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TRIGGER investigation_hypothesis_canonical_apply_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_hypothesis_canonical_apply_receipts
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
