-- Plan B: Hypothesis Registry and Candidate Analysis.
--
-- This is the sole Plan B migration. It is additive: immutable history is
-- corrected only by later forward migrations, never by changing Plan A.

-- ---------------------------------------------------------------------------
-- Shared immutable-history helpers
-- ---------------------------------------------------------------------------

CREATE FUNCTION investigation_reject_append_only()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'investigation_append_only' USING ERRCODE='23514';
END;
$$;

-- HypothesisVerificationPlanV1 hashes a canonical, lexically sorted JSON
-- array with the same domain/NUL/u64-length envelope as golish-core.
CREATE FUNCTION investigation_exact_member_set_hash(
    domain_tag TEXT,
    member_hashes TEXT[]
) RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    WITH canonical AS (
        SELECT '[' || COALESCE(string_agg(
            '"' || member_hash || '"', ',' ORDER BY member_hash
        ), '') || ']' AS json_text
        FROM unnest(member_hashes) AS member(member_hash)
    )
    SELECT 'sha256:' || encode(digest(
        convert_to(domain_tag,'UTF8')
        || decode('00','hex')
        || int8send(octet_length(json_text)::BIGINT)
        || convert_to(json_text,'UTF8'),
        'sha256'
    ),'hex')
    FROM canonical
$$;

-- Candidate Gate uses a length-prefixed binary writer rather than the core
-- JSON-array envelope above. Keep the SQL constraint byte-identical to the
-- pure Rust Gate so the canonical writer cannot silently reseal a mutation.
CREATE FUNCTION candidate_gate_exact_member_set_hash(
    domain_tag TEXT,
    member_hashes TEXT[]
) RETURNS TEXT
LANGUAGE plpgsql
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
DECLARE
    payload BYTEA;
    member_hash TEXT;
BEGIN
    payload := int8send(octet_length(domain_tag)::BIGINT)
        || convert_to(domain_tag,'UTF8');
    FOR member_hash IN
        SELECT value FROM unnest(member_hashes) AS member(value) ORDER BY value
    LOOP
        payload := payload
            || int8send(octet_length(member_hash)::BIGINT)
            || convert_to(member_hash,'UTF8');
    END LOOP;
    RETURN 'sha256:' || encode(digest(payload,'sha256'),'hex');
END;
$$;

-- VerificationContractV1 uses its dedicated binary DomainHashWriter: a
-- u32 count field followed by each already-canonical member hash field.
CREATE FUNCTION verification_contract_exact_member_set_hash(
    domain_tag TEXT,
    ordered_member_hashes TEXT[]
) RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
STRICT
PARALLEL SAFE
AS $$
    SELECT 'sha256:' || encode(digest(
        convert_to(domain_tag,'UTF8')
        || decode('00','hex')
        || int8send(4::BIGINT)
        || int4send(cardinality(ordered_member_hashes))
        || COALESCE((
            SELECT string_agg(
                int8send(octet_length(member_hash)::BIGINT)
                || convert_to(member_hash,'UTF8'),
                ''::BYTEA ORDER BY ordinal
            )
            FROM unnest(ordered_member_hashes) WITH ORDINALITY
                AS member(member_hash,ordinal)
        ),''::BYTEA),
        'sha256'
    ),'hex')
$$;

-- Plan A deliberately exposes opaque guards rather than every SQL identity.
-- These additive candidate keys let Plan B prove that copied authority fields
-- identify one immutable, sealed Plan A bundle/member without a mirror reducer.
ALTER TABLE tool_truth_authority_bundle_seals
    ADD CONSTRAINT tool_truth_authority_bundle_plan_b_identity_unique UNIQUE (
        id,operation_id,organization_id,stable_consumer_request_id,
        relevant_root_set_hash,member_set_hash,
        semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
        target_state_epoch_set_hash,sealed_at
    );

ALTER TABLE tool_truth_authority_bundle_members
    ADD CONSTRAINT tool_truth_authority_bundle_member_plan_b_identity_unique UNIQUE (
        id,bundle_seal_id,operation_id,organization_id,ordinal,root_family,
        root_execution_authority_id,root_denominator_id,root_denominator_hash,
        authority_set_seal_id,authority_set_semantic_hash,
        authority_set_graph_hash,authority_set_freshness_hash,
        temporal_validity_policy_set_hash,target_state_epoch_set_hash,
        semantic_status,temporal_validity_status,member_status,member_hash
    );

-- ---------------------------------------------------------------------------
-- Deployment default and operation-frozen joint contract
-- ---------------------------------------------------------------------------

CREATE TABLE investigation_rollout (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    contract_version TEXT NOT NULL CHECK (
        contract_version IN ('legacy_candidate_v1','hypothesis_registry_v1')
    ),
    rollout_mode TEXT NOT NULL CHECK (
        rollout_mode IN (
            'legacy_only','shadow_registry','dual_read_compare',
            'registry_authoritative_legacy_projection','new_only'
        )
    ),
    mode_rank SMALLINT NOT NULL CHECK (mode_rank BETWEEN 0 AND 4),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT investigation_rollout_pair_check CHECK (
        (contract_version='legacy_candidate_v1' AND rollout_mode='legacy_only' AND mode_rank=0)
        OR (contract_version='hypothesis_registry_v1' AND rollout_mode='shadow_registry' AND mode_rank=1)
        OR (contract_version='hypothesis_registry_v1' AND rollout_mode='dual_read_compare' AND mode_rank=2)
        OR (contract_version='hypothesis_registry_v1' AND rollout_mode='registry_authoritative_legacy_projection' AND mode_rank=3)
        OR (contract_version='hypothesis_registry_v1' AND rollout_mode='new_only' AND mode_rank=4)
    )
);

INSERT INTO investigation_rollout(singleton,contract_version,rollout_mode,mode_rank)
VALUES(TRUE,'legacy_candidate_v1','legacy_only',0);

CREATE FUNCTION investigation_reject_rollout_direct_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'investigation_rollout_direct_mutation_forbidden' USING ERRCODE='23514';
END;
$$;

CREATE TRIGGER investigation_rollout_direct_mutation_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_rollout
FOR EACH ROW EXECUTE FUNCTION investigation_reject_rollout_direct_mutation();

CREATE FUNCTION operation_joint_contract_rank(
    tool_truth TEXT,
    investigation_contract TEXT,
    investigation_mode TEXT
) RETURNS SMALLINT
LANGUAGE SQL
IMMUTABLE
STRICT
AS $$
    SELECT CASE
        WHEN tool_truth='legacy_v1' AND investigation_contract='legacy_candidate_v1' AND investigation_mode='legacy_only' THEN 0
        WHEN tool_truth='shadow_v1' AND investigation_contract='legacy_candidate_v1' AND investigation_mode='legacy_only' THEN 1
        WHEN tool_truth='shadow_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='shadow_registry' THEN 2
        WHEN tool_truth='shadow_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='dual_read_compare' THEN 3
        WHEN tool_truth='receipt_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='dual_read_compare' THEN 4
        WHEN tool_truth='receipt_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='registry_authoritative_legacy_projection' THEN 5
        WHEN tool_truth='receipt_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='new_only' THEN 6
        ELSE NULL
    END
$$;

ALTER TABLE operation_state
    ADD COLUMN investigation_contract_version TEXT NOT NULL DEFAULT 'legacy_candidate_v1',
    ADD COLUMN investigation_rollout_mode TEXT NOT NULL DEFAULT 'legacy_only',
    ADD CONSTRAINT operation_state_investigation_contract_check CHECK (
        investigation_contract_version IN ('legacy_candidate_v1','hypothesis_registry_v1')
    ),
    ADD CONSTRAINT operation_state_investigation_rollout_mode_check CHECK (
        investigation_rollout_mode IN (
            'legacy_only','shadow_registry','dual_read_compare',
            'registry_authoritative_legacy_projection','new_only'
        )
    ),
    ADD CONSTRAINT operation_state_joint_contract_pair_check CHECK (
        operation_joint_contract_rank(
            tool_truth_contract,
            investigation_contract_version,
            investigation_rollout_mode
        ) IS NOT NULL
    );

CREATE FUNCTION enforce_operation_investigation_contract_immutable()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF ROW(NEW.investigation_contract_version,NEW.investigation_rollout_mode)
       IS DISTINCT FROM
       ROW(OLD.investigation_contract_version,OLD.investigation_rollout_mode)
    THEN
        RAISE EXCEPTION 'OPERATION_INVESTIGATION_CONTRACT_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_state_investigation_contract_immutable
BEFORE UPDATE OF investigation_contract_version,investigation_rollout_mode ON operation_state
FOR EACH ROW EXECUTE FUNCTION enforce_operation_investigation_contract_immutable();

CREATE TABLE operation_contract_adoptions (
    adoption_id UUID PRIMARY KEY,
    source_operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    target_operation_id UUID NOT NULL UNIQUE,
    source_tool_truth_contract TEXT NOT NULL,
    source_investigation_contract_version TEXT NOT NULL,
    source_investigation_rollout_mode TEXT NOT NULL,
    source_joint_rank SMALLINT NOT NULL CHECK (source_joint_rank BETWEEN 0 AND 6),
    target_tool_truth_contract TEXT NOT NULL,
    target_investigation_contract_version TEXT NOT NULL,
    target_investigation_rollout_mode TEXT NOT NULL,
    target_joint_rank SMALLINT NOT NULL CHECK (target_joint_rank BETWEEN 0 AND 6),
    source_final_seal_hash TEXT NOT NULL CHECK (source_final_seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    adoption_set_hash TEXT NOT NULL CHECK (adoption_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    stable_request_id UUID NOT NULL UNIQUE,
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT operation_contract_adoption_source_rank_check CHECK (
        operation_joint_contract_rank(
            source_tool_truth_contract,
            source_investigation_contract_version,
            source_investigation_rollout_mode
        )=source_joint_rank
    ),
    CONSTRAINT operation_contract_adoption_target_rank_check CHECK (
        operation_joint_contract_rank(
            target_tool_truth_contract,
            target_investigation_contract_version,
            target_investigation_rollout_mode
        )=target_joint_rank
    ),
    CONSTRAINT operation_contract_adoption_adjacent_check CHECK (
        target_joint_rank=source_joint_rank+1
    ),
    FOREIGN KEY(target_operation_id)
        REFERENCES operation_state(operation_id) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TRIGGER operation_contract_adoptions_append_only
BEFORE UPDATE OR DELETE ON operation_contract_adoptions
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- The current Plan B deployment remains the legacy pair. A future Plan D
-- migration owns promotion and may replace this insert coordinator. An
-- adjacent fork adoption may preinstall its deferred receipt before inserting
-- the target operation.
DROP TRIGGER operation_state_tool_truth_contract_insert_guard ON operation_state;

CREATE FUNCTION validate_operation_joint_contract_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    deployed_tool TEXT;
    deployed_contract TEXT;
    deployed_mode TEXT;
BEGIN
    IF EXISTS (
        SELECT 1
          FROM operation_contract_adoptions adoption
          JOIN operation_state source ON source.operation_id=adoption.source_operation_id
          JOIN operation_stage_forks fork
            ON fork.operation_id=NEW.operation_id
           AND fork.source_operation_id=source.operation_id
         WHERE adoption.target_operation_id=NEW.operation_id
           AND adoption.source_tool_truth_contract=source.tool_truth_contract
           AND adoption.source_investigation_contract_version=source.investigation_contract_version
           AND adoption.source_investigation_rollout_mode=source.investigation_rollout_mode
           AND adoption.target_tool_truth_contract=NEW.tool_truth_contract
           AND adoption.target_investigation_contract_version=NEW.investigation_contract_version
           AND adoption.target_investigation_rollout_mode=NEW.investigation_rollout_mode
    ) THEN
        RETURN NEW;
    END IF;
    IF EXISTS (
        SELECT 1
          FROM operation_stage_forks fork
          JOIN operation_state source ON source.operation_id=fork.source_operation_id
         WHERE fork.operation_id=NEW.operation_id
           AND source.tool_truth_contract=NEW.tool_truth_contract
           AND source.investigation_contract_version=NEW.investigation_contract_version
           AND source.investigation_rollout_mode=NEW.investigation_rollout_mode
           AND NOT EXISTS (
               SELECT 1 FROM operation_contract_adoptions adoption
                WHERE adoption.target_operation_id=NEW.operation_id
           )
    ) THEN
        RETURN NEW;
    END IF;
    SELECT new_operation_contract INTO deployed_tool
      FROM tool_truth_rollout WHERE singleton FOR SHARE;
    SELECT contract_version,rollout_mode INTO deployed_contract,deployed_mode
      FROM investigation_rollout WHERE singleton FOR SHARE;
    IF ROW(NEW.tool_truth_contract,NEW.investigation_contract_version,NEW.investigation_rollout_mode)
       = ROW(deployed_tool,deployed_contract,deployed_mode)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'operation_joint_contract_not_deployed_or_adopted' USING ERRCODE='23514';
END;
$$;

CREATE CONSTRAINT TRIGGER operation_state_joint_contract_insert_guard
AFTER INSERT ON operation_state
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION validate_operation_joint_contract_insert();

-- ---------------------------------------------------------------------------
-- Stage Team vocabulary extension
-- ---------------------------------------------------------------------------

ALTER TABLE stage_work_items
    DROP CONSTRAINT stage_work_items_created_by_check,
    ADD CONSTRAINT stage_work_items_created_by_check CHECK (
        created_by IN (
            'server_seed','accepted_worker_request','gate_repair','server_phase_transition'
        )
    );

ALTER TABLE stage_worker_outputs
    DROP CONSTRAINT stage_worker_outputs_business_disposition_check,
    ADD CONSTRAINT stage_worker_outputs_business_disposition_check CHECK (
        business_disposition IN ('found','checked_empty','blocked','artifact_recorded')
    );

-- ---------------------------------------------------------------------------
-- Canonical Hypothesis Registry
-- ---------------------------------------------------------------------------

CREATE TABLE attack_hypotheses (
    root_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    root_kind TEXT NOT NULL CHECK (root_kind IN ('initial','split','merge','derive')),
    identity_ingredients JSONB NOT NULL CHECK (jsonb_typeof(identity_ingredients)='object'),
    identity_ingredients_hash TEXT NOT NULL CHECK (identity_ingredients_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(root_id,operation_id,organization_id),
    UNIQUE(operation_id,organization_id,identity_ingredients_hash)
);

CREATE TABLE attack_hypothesis_revisions (
    revision_id UUID PRIMARY KEY,
    root_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    predecessor_revision_id UUID,
    revision_ordinal INTEGER NOT NULL CHECK (revision_ordinal>=0),
    semantic_key JSONB NOT NULL CHECK (jsonb_typeof(semantic_key)='object'),
    semantic_key_hash TEXT NOT NULL CHECK (semantic_key_hash ~ '^sha256:[0-9a-f]{64}$'),
    subject_kind TEXT NOT NULL CHECK (btrim(subject_kind)<>''),
    subject_identity_hash TEXT NOT NULL CHECK (subject_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (btrim(target_type_at_time)<>''),
    target_value_at_time TEXT NOT NULL CHECK (btrim(target_value_at_time)<>''),
    predicate_schema TEXT NOT NULL CHECK (btrim(predicate_schema)<>''),
    predicate_version INTEGER NOT NULL CHECK (predicate_version>0),
    normalized_arguments JSONB NOT NULL CHECK (jsonb_typeof(normalized_arguments)='object'),
    trust_boundary TEXT NOT NULL CHECK (btrim(trust_boundary)<>''),
    polarity TEXT NOT NULL CHECK (polarity IN ('positive','negative')),
    epistemic_state TEXT NOT NULL CHECK (epistemic_state IN (
        'proposed','supported','contested','verified','refuted','inconclusive','invalid'
    )),
    lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN ('current','superseded','closed')),
    planning_readiness TEXT NOT NULL CHECK (planning_readiness IN (
        'ready_for_strategy','needs_enrichment','deferred','out_of_scope','unsafe'
    )),
    structured_claim JSONB NOT NULL CHECK (jsonb_typeof(structured_claim)='object'),
    assumptions JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(assumptions)='array'),
    missing_facts JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(missing_facts)='array'),
    priority INTEGER NOT NULL,
    risk_impact JSONB NOT NULL CHECK (jsonb_typeof(risk_impact)='object'),
    origin_decision_hash TEXT NOT NULL CHECK (origin_decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    revision_ingredients_hash TEXT NOT NULL CHECK (revision_ingredients_hash ~ '^sha256:[0-9a-f]{64}$'),
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(revision_id,root_id,operation_id,organization_id,epistemic_state),
    UNIQUE(revision_id,root_id,operation_id,organization_id),
    UNIQUE(revision_id,root_id,operation_id,organization_id,semantic_key_hash,revision_hash),
    UNIQUE(revision_id,root_id,operation_id,organization_id,revision_ingredients_hash),
    UNIQUE(revision_id,root_id,operation_id,organization_id,revision_hash),
    UNIQUE(revision_id,operation_id,organization_id),
    UNIQUE(revision_id,revision_hash),
    UNIQUE(revision_id,revision_hash,revision_ingredients_hash),
    UNIQUE(root_id,revision_ordinal),
    FOREIGN KEY(root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_revision_id,root_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    CHECK (
        (epistemic_state IN ('verified','refuted','invalid') AND lifecycle_state='closed')
        OR epistemic_state NOT IN ('verified','refuted','invalid')
    ),
    CHECK (lifecycle_state<>'superseded' OR planning_readiness<>'ready_for_strategy')
);

-- Revisions remain immutable.  Current/superseded/closed authority therefore
-- lives in one narrow mutable CAS row per root instead of rewriting history.
CREATE TABLE attack_hypothesis_heads (
    root_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    head_revision_id UUID NOT NULL,
    head_revision_hash TEXT NOT NULL CHECK (head_revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    head_semantic_key_hash TEXT NOT NULL CHECK (head_semantic_key_hash ~ '^sha256:[0-9a-f]{64}$'),
    head_epistemic_state TEXT NOT NULL CHECK (head_epistemic_state IN (
        'proposed','supported','contested','verified','refuted','inconclusive','invalid'
    )),
    head_lifecycle_state TEXT NOT NULL CHECK (
        head_lifecycle_state IN ('current','superseded','closed')
    ),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK (head_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(root_id,operation_id,organization_id),
    FOREIGN KEY(root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(
        head_revision_id,root_id,operation_id,organization_id,
        head_semantic_key_hash,head_revision_hash
    ) REFERENCES attack_hypothesis_revisions(
        revision_id,root_id,operation_id,organization_id,
        semantic_key_hash,revision_hash
    ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        (head_epistemic_state IN ('verified','refuted','invalid')
            AND head_lifecycle_state='closed')
        OR head_epistemic_state NOT IN ('verified','refuted','invalid')
    )
);

CREATE UNIQUE INDEX attack_hypothesis_one_current_semantic_key
ON attack_hypothesis_heads(operation_id,organization_id,head_semantic_key_hash)
WHERE head_lifecycle_state='current';

CREATE FUNCTION enforce_attack_hypothesis_head_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    successor attack_hypothesis_revisions%ROWTYPE;
    predecessor attack_hypothesis_revisions%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'HYPOTHESIS_HEAD_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT successor
      FROM attack_hypothesis_revisions
     WHERE revision_id=NEW.head_revision_id
       AND root_id=NEW.root_id
       AND operation_id=NEW.operation_id
       AND organization_id=NEW.organization_id
     FOR SHARE;
    IF ROW(successor.revision_hash,successor.semantic_key_hash,successor.epistemic_state)
       IS DISTINCT FROM
       ROW(NEW.head_revision_hash,NEW.head_semantic_key_hash,NEW.head_epistemic_state)
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_HEAD_REVISION_IDENTITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.head_version<>0 OR successor.lifecycle_state<>NEW.head_lifecycle_state THEN
            RAISE EXCEPTION 'HYPOTHESIS_HEAD_INITIAL_STATE_INVALID' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(NEW.root_id,NEW.operation_id,NEW.organization_id)
       IS DISTINCT FROM ROW(OLD.root_id,OLD.operation_id,OLD.organization_id)
       OR NEW.head_version<>OLD.head_version+1
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    IF NEW.head_revision_id=OLD.head_revision_id THEN
        IF ROW(NEW.head_revision_hash,NEW.head_semantic_key_hash,NEW.head_epistemic_state)
           IS DISTINCT FROM
           ROW(OLD.head_revision_hash,OLD.head_semantic_key_hash,OLD.head_epistemic_state)
           OR OLD.head_lifecycle_state<>'current'
           OR NEW.head_lifecycle_state<>'superseded'
        THEN
            RAISE EXCEPTION 'HYPOTHESIS_HEAD_LIFECYCLE_TRANSITION_INVALID' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    SELECT * INTO STRICT predecessor
      FROM attack_hypothesis_revisions
     WHERE revision_id=OLD.head_revision_id
       AND root_id=OLD.root_id
       AND operation_id=OLD.operation_id
       AND organization_id=OLD.organization_id
     FOR SHARE;
    IF successor.predecessor_revision_id IS DISTINCT FROM OLD.head_revision_id
       OR successor.revision_ordinal<>predecessor.revision_ordinal+1
       OR OLD.head_lifecycle_state<>'current'
       OR successor.lifecycle_state<>NEW.head_lifecycle_state
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_HEAD_SUCCESSOR_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER attack_hypothesis_heads_cas_guard
BEFORE INSERT OR UPDATE OR DELETE ON attack_hypothesis_heads
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_head_cas();

CREATE FUNCTION enforce_attack_hypothesis_root_head_required()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF (SELECT COUNT(*) FROM attack_hypothesis_heads head
         WHERE head.root_id=NEW.root_id)<>1
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_ROOT_HEAD_EXACT_ONE_REQUIRED' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER attack_hypothesis_root_head_required
AFTER INSERT ON attack_hypotheses
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_root_head_required();

CREATE TABLE attack_hypothesis_revision_sources (
    source_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    source_role TEXT NOT NULL CHECK (source_role IN (
        'support','contradiction','application_context','knowledge_signal','gap'
    )),
    source_kind TEXT NOT NULL CHECK (btrim(source_kind)<>''),
    source_ref TEXT NOT NULL CHECK (btrim(source_ref)<>''),
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(revision_id,ordinal),
    UNIQUE(revision_id,source_kind,source_ref)
);

CREATE TABLE attack_hypothesis_verification_objectives (
    objective_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    objective_ordinal INTEGER NOT NULL CHECK (objective_ordinal>=0),
    objective_intent JSONB NOT NULL CHECK (jsonb_typeof(objective_intent)='object'),
    stopping_criteria JSONB NOT NULL CHECK (jsonb_typeof(stopping_criteria)='object'),
    stopping_criteria_hash TEXT NOT NULL CHECK (stopping_criteria_hash ~ '^sha256:[0-9a-f]{64}$'),
    objective_hash TEXT NOT NULL CHECK (objective_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(revision_id,objective_ordinal),
    UNIQUE(revision_id,objective_hash),
    UNIQUE(objective_id,revision_id),
    UNIQUE(objective_id,revision_id,objective_hash),
    UNIQUE(objective_id,revision_id,stopping_criteria_hash)
);

CREATE TABLE attack_hypothesis_claim_components (
    component_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL,
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    component_ordinal INTEGER NOT NULL CHECK (component_ordinal>=0),
    component_key TEXT NOT NULL CHECK (btrim(component_key)<>''),
    kind TEXT NOT NULL CHECK (kind IN (
        'claim_clause','impact_qualifier','trust_boundary_condition','identity_condition'
    )),
    canonical_fragment_hash TEXT NOT NULL CHECK (canonical_fragment_hash ~ '^sha256:[0-9a-f]{64}$'),
    canonical_condition_hash TEXT NOT NULL CHECK (canonical_condition_hash ~ '^sha256:[0-9a-f]{64}$'),
    required BOOLEAN NOT NULL,
    derivation_contract_version INTEGER NOT NULL CHECK (derivation_contract_version>0),
    derivation_contract_digest TEXT NOT NULL CHECK (derivation_contract_digest ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(revision_id,component_ordinal),
    UNIQUE(revision_id,component_key),
    UNIQUE(revision_id,member_hash),
    UNIQUE(component_id,revision_id,member_hash),
    FOREIGN KEY(revision_id,revision_hash)
        REFERENCES attack_hypothesis_revisions(revision_id,revision_hash)
        ON DELETE RESTRICT
);

CREATE TABLE attack_hypothesis_verification_contracts (
    contract_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL,
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    objective_id UUID NOT NULL REFERENCES attack_hypothesis_verification_objectives(objective_id) ON DELETE RESTRICT,
    contract_schema TEXT NOT NULL DEFAULT 'verification_contract.v1'
        CHECK (contract_schema='verification_contract.v1'),
    contract_version INTEGER NOT NULL DEFAULT 1 CHECK (contract_version=1),
    combinator TEXT NOT NULL CHECK (combinator IN (
        'all_of','any_of','paired_differential','ordered_sequence'
    )),
    predicate_count BIGINT NOT NULL CHECK (predicate_count>0),
    predicate_set_hash TEXT NOT NULL CHECK (predicate_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    required_control_count BIGINT NOT NULL CHECK (required_control_count>=0),
    required_control_set_hash TEXT NOT NULL CHECK (required_control_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    explicit_no_required_control BOOLEAN NOT NULL,
    paired_differential_count BIGINT NOT NULL CHECK (paired_differential_count>=0),
    paired_differential_set_hash TEXT NOT NULL CHECK (paired_differential_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    ordered_step_count BIGINT NOT NULL CHECK (ordered_step_count>=0),
    ordered_step_set_hash TEXT NOT NULL CHECK (ordered_step_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    stopping_criteria_hash TEXT NOT NULL CHECK (stopping_criteria_hash ~ '^sha256:[0-9a-f]{64}$'),
    compiler_digest TEXT NOT NULL CHECK (compiler_digest ~ '^sha256:[0-9a-f]{64}$'),
    rule_digest TEXT NOT NULL CHECK (rule_digest ~ '^sha256:[0-9a-f]{64}$'),
    policy_snapshot_hash TEXT NOT NULL CHECK (policy_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    contract_hash TEXT NOT NULL CHECK (contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((required_control_count=0)=explicit_no_required_control),
    CHECK (
        (combinator IN ('all_of','any_of')
            AND paired_differential_count=0 AND ordered_step_count=0)
        OR (combinator='paired_differential'
            AND paired_differential_count>0 AND ordered_step_count=0)
        OR (combinator='ordered_sequence'
            AND paired_differential_count=0 AND ordered_step_count>=2)
    ),
    UNIQUE(revision_id,objective_id,contract_version,policy_snapshot_hash),
    UNIQUE(contract_id,revision_id,objective_id),
    UNIQUE(contract_id,revision_id,objective_id,contract_hash),
    UNIQUE(
        contract_id,revision_id,objective_id,contract_version,contract_hash
    ),
    FOREIGN KEY(revision_id,revision_hash)
        REFERENCES attack_hypothesis_revisions(revision_id,revision_hash)
        ON DELETE RESTRICT,
    FOREIGN KEY(objective_id,revision_id)
        REFERENCES attack_hypothesis_verification_objectives(objective_id,revision_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(objective_id,revision_id,stopping_criteria_hash)
        REFERENCES attack_hypothesis_verification_objectives(
            objective_id,revision_id,stopping_criteria_hash
        )
        ON DELETE RESTRICT
);

CREATE TABLE attack_hypothesis_verification_plans (
    plan_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL UNIQUE,
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    plan_schema TEXT NOT NULL DEFAULT 'hypothesis_verification_plan.v1'
        CHECK (plan_schema='hypothesis_verification_plan.v1'),
    plan_version INTEGER NOT NULL DEFAULT 1 CHECK (plan_version=1),
    revision_ingredients_hash TEXT NOT NULL CHECK (revision_ingredients_hash ~ '^sha256:[0-9a-f]{64}$'),
    required_claim_component_count BIGINT NOT NULL CHECK (required_claim_component_count>0),
    required_claim_component_set_hash TEXT NOT NULL CHECK (required_claim_component_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    objective_count BIGINT NOT NULL CHECK (objective_count>0),
    objective_set_hash TEXT NOT NULL CHECK (objective_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    proof_path_count BIGINT NOT NULL CHECK (proof_path_count>0),
    proof_path_set_hash TEXT NOT NULL CHECK (proof_path_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    outer_aggregation_policy_version INTEGER NOT NULL CHECK (outer_aggregation_policy_version>0),
    outer_aggregation_policy_digest TEXT NOT NULL CHECK (outer_aggregation_policy_digest ~ '^sha256:[0-9a-f]{64}$'),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL,
    UNIQUE(plan_id,revision_id),
    UNIQUE(plan_id,revision_id,plan_hash),
    FOREIGN KEY(revision_id,revision_hash,revision_ingredients_hash)
        REFERENCES attack_hypothesis_revisions(
            revision_id,revision_hash,revision_ingredients_hash
        )
        ON DELETE RESTRICT
);

CREATE TABLE hypothesis_server_validation_receipts (
    receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    root_id UUID NOT NULL,
    predecessor_revision_id UUID,
    predecessor_revision_hash TEXT CHECK (
        predecessor_revision_hash IS NULL OR predecessor_revision_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    validated_revision_id UUID NOT NULL,
    validated_revision_ingredients_hash TEXT NOT NULL CHECK (validated_revision_ingredients_hash ~ '^sha256:[0-9a-f]{64}$'),
    validator_contract_version INTEGER NOT NULL CHECK (validator_contract_version>0),
    validator_contract_digest TEXT NOT NULL CHECK (validator_contract_digest ~ '^sha256:[0-9a-f]{64}$'),
    invalid_reason_code TEXT NOT NULL CHECK (btrim(invalid_reason_code)<>''),
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((predecessor_revision_id IS NULL)=(predecessor_revision_hash IS NULL)),
    UNIQUE(receipt_id,operation_id,organization_id,root_id,receipt_hash),
    UNIQUE(
        receipt_id,operation_id,organization_id,root_id,validated_revision_id,
        validated_revision_ingredients_hash,receipt_hash
    ),
    FOREIGN KEY(root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_revision_id,root_id,operation_id,organization_id,predecessor_revision_hash)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id,revision_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(
        validated_revision_id,root_id,operation_id,organization_id,
        validated_revision_ingredients_hash
    ) REFERENCES attack_hypothesis_revisions(
        revision_id,root_id,operation_id,organization_id,revision_ingredients_hash
    ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

-- Candidate state events are authorized by a host-reduced, append-only Gate
-- decision exact set.  The per-mutation origin hash deliberately excludes the
-- successor revision/event ids, so revision identity remains acyclic.
CREATE TABLE hypothesis_candidate_gate_decisions (
    decision_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    candidate_snapshot_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    mutation_count BIGINT NOT NULL CHECK (mutation_count>0),
    mutation_set_hash TEXT NOT NULL CHECK (mutation_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    generation_transition_count BIGINT NOT NULL CHECK (generation_transition_count>0),
    generation_transition_set_hash TEXT NOT NULL CHECK (
        generation_transition_set_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    gate_authority_hash TEXT NOT NULL CHECK (gate_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    decision_hash TEXT NOT NULL CHECK (decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(decision_id,operation_id,organization_id),
    UNIQUE(decision_id,operation_id,organization_id,decision_hash)
);

CREATE TABLE hypothesis_candidate_gate_decision_members (
    mutation_id UUID PRIMARY KEY,
    decision_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    route_kind TEXT NOT NULL CHECK (route_kind IN (
        'attach_current','no_semantic_change','create_initial','reopen_historical',
        'split','merge','derive','narrow_successor'
    )),
    root_id UUID NOT NULL,
    predecessor_revision_id UUID,
    successor_revision_id UUID NOT NULL,
    semantic_key_hash TEXT NOT NULL CHECK (semantic_key_hash ~ '^sha256:[0-9a-f]{64}$'),
    successor_epistemic_state TEXT NOT NULL CHECK (successor_epistemic_state IN (
        'proposed','supported','contested','inconclusive'
    )),
    origin_decision_hash TEXT NOT NULL CHECK (origin_decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    generation_transition_hash TEXT NOT NULL CHECK (
        generation_transition_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (route_kind IN (
            'attach_current','no_semantic_change','create_initial',
            'split','merge','derive','narrow_successor'
        )
            AND predecessor_revision_id IS NULL)
        OR (route_kind='reopen_historical'
            AND predecessor_revision_id IS NOT NULL)
    ),
    UNIQUE(decision_id,ordinal),
    UNIQUE(decision_id,mutation_id),
    UNIQUE(decision_id,root_id),
    UNIQUE(
        mutation_id,operation_id,organization_id,root_id,predecessor_revision_id,
        successor_revision_id,semantic_key_hash,successor_epistemic_state,
        origin_decision_hash
    ),
    FOREIGN KEY(decision_id,operation_id,organization_id)
        REFERENCES hypothesis_candidate_gate_decisions(
            decision_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_revision_id,root_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(successor_revision_id,root_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id
        ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE UNIQUE INDEX hypothesis_candidate_gate_writing_successor_unique
    ON hypothesis_candidate_gate_decision_members(successor_revision_id)
    WHERE route_kind NOT IN ('attach_current','no_semantic_change');

CREATE FUNCTION enforce_hypothesis_candidate_gate_decision_exact_set()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    decision hypothesis_candidate_gate_decisions%ROWTYPE;
    actual_count BIGINT;
    ordinal_count BIGINT;
    minimum_ordinal INTEGER;
    maximum_ordinal INTEGER;
    actual_set_hash TEXT;
    actual_transition_set_hash TEXT;
BEGIN
    SELECT * INTO STRICT decision
      FROM hypothesis_candidate_gate_decisions
     WHERE decision_id=NEW.decision_id;
    SELECT COUNT(*),COUNT(DISTINCT ordinal),MIN(ordinal),MAX(ordinal),
           candidate_gate_exact_member_set_hash(
               'candidate_mutations.v1',
               COALESCE(array_agg(member_hash ORDER BY ordinal),ARRAY[]::TEXT[])
           ),
           candidate_gate_exact_member_set_hash(
               'candidate_generation_transitions.v1',
               COALESCE(array_agg(generation_transition_hash ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO actual_count,ordinal_count,minimum_ordinal,maximum_ordinal,
           actual_set_hash,actual_transition_set_hash
      FROM hypothesis_candidate_gate_decision_members
     WHERE decision_id=decision.decision_id;
    IF actual_count<>decision.mutation_count
       OR ordinal_count<>actual_count
       OR minimum_ordinal<>0
       OR maximum_ordinal<>actual_count-1
       OR actual_set_hash<>decision.mutation_set_hash
       OR actual_count<>decision.generation_transition_count
       OR actual_transition_set_hash<>decision.generation_transition_set_hash
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_GATE_MUTATION_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER hypothesis_candidate_gate_decision_exact_set
AFTER INSERT ON hypothesis_candidate_gate_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_hypothesis_candidate_gate_decision_exact_set();

CREATE CONSTRAINT TRIGGER hypothesis_candidate_gate_decision_member_exact_set
AFTER INSERT ON hypothesis_candidate_gate_decision_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_hypothesis_candidate_gate_decision_exact_set();

CREATE TABLE attack_hypothesis_state_events (
    event_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    root_id UUID NOT NULL REFERENCES attack_hypotheses(root_id) ON DELETE RESTRICT,
    predecessor_revision_id UUID,
    successor_revision_id UUID NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN (
        'created','supported','contested','inconclusive','invalidated','verified','refuted'
    )),
    origin_authority TEXT NOT NULL CHECK (origin_authority IN (
        'candidate_analysis','server_validator','hypothesis_revision_adjudication'
    )),
    successor_epistemic_state TEXT NOT NULL CHECK (successor_epistemic_state IN (
        'proposed','supported','contested','verified','refuted','inconclusive','invalid'
    )),
    authority_receipt_kind TEXT NOT NULL CHECK (authority_receipt_kind IN (
        'candidate_gate_decision','server_validation','revision_transition_decision'
    )),
    authority_receipt_id UUID NOT NULL,
    authority_receipt_hash TEXT NOT NULL CHECK (authority_receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    event_hash TEXT NOT NULL CHECK (event_hash ~ '^sha256:[0-9a-f]{64}$'),
    server_decision_id UUID NOT NULL,
    server_decision_hash TEXT NOT NULL CHECK (server_decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (server_decision_id=authority_receipt_id),
    CHECK (server_decision_hash=authority_receipt_hash),
    UNIQUE(successor_revision_id),
    FOREIGN KEY(root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_revision_id,root_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(successor_revision_id,root_id,operation_id,organization_id,successor_epistemic_state)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id,epistemic_state
        ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE FUNCTION enforce_hypothesis_revision_creating_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    creating attack_hypothesis_state_events%ROWTYPE;
    event_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO event_count
      FROM attack_hypothesis_state_events
     WHERE successor_revision_id=NEW.revision_id;
    IF event_count<>1 THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_REQUIRED' USING ERRCODE='23514';
    END IF;
    SELECT * INTO creating FROM attack_hypothesis_state_events
     WHERE successor_revision_id=NEW.revision_id;
    IF ROW(creating.root_id,creating.operation_id,creating.organization_id,creating.successor_epistemic_state)
       IS DISTINCT FROM ROW(NEW.root_id,NEW.operation_id,NEW.organization_id,NEW.epistemic_state)
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_SCOPE_STATE_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF creating.predecessor_revision_id IS DISTINCT FROM NEW.predecessor_revision_id THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_PREDECESSOR_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF creating.server_decision_hash<>NEW.origin_decision_hash THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_DECISION_HASH_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF (NEW.revision_ordinal=0) IS DISTINCT FROM (NEW.predecessor_revision_id IS NULL)
       OR (
           NEW.predecessor_revision_id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM attack_hypothesis_revisions predecessor
                WHERE predecessor.revision_id=NEW.predecessor_revision_id
                  AND predecessor.root_id=NEW.root_id
                  AND predecessor.operation_id=NEW.operation_id
                  AND predecessor.organization_id=NEW.organization_id
                  AND predecessor.revision_ordinal=NEW.revision_ordinal-1
           )
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_PREDECESSOR_INVALID' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='candidate_analysis'
       AND (
           NEW.epistemic_state NOT IN ('proposed','supported','contested','inconclusive')
           OR ROW(creating.event_kind,NEW.epistemic_state) NOT IN (
               ROW('created','proposed'),ROW('supported','supported'),
               ROW('contested','contested'),ROW('inconclusive','inconclusive')
           )
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_TERMINAL_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='candidate_analysis'
       AND (
           creating.authority_receipt_kind<>'candidate_gate_decision'
           OR NOT EXISTS (
               SELECT 1
                 FROM hypothesis_candidate_gate_decision_members mutation
                WHERE mutation.mutation_id=creating.server_decision_id
                  AND mutation.operation_id=NEW.operation_id
                  AND mutation.organization_id=NEW.organization_id
                  AND mutation.root_id=NEW.root_id
                  AND mutation.predecessor_revision_id IS NOT DISTINCT FROM NEW.predecessor_revision_id
                  AND mutation.successor_revision_id=NEW.revision_id
                  AND mutation.semantic_key_hash=NEW.semantic_key_hash
                  AND mutation.successor_epistemic_state=NEW.epistemic_state
                  AND mutation.origin_decision_hash=creating.server_decision_hash
                  AND (
                      mutation.route_kind IN ('reopen_historical','narrow_successor')
                      OR EXISTS (
                          SELECT 1 FROM attack_hypotheses routed_root
                           WHERE routed_root.root_id=mutation.root_id
                             AND routed_root.root_kind=CASE mutation.route_kind
                                 WHEN 'create_initial' THEN 'initial'
                                 WHEN 'split' THEN 'split'
                                 WHEN 'merge' THEN 'merge'
                                 WHEN 'derive' THEN 'derive'
                             END
                      )
                  )
           )
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_GATE_DECISION_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='server_validator'
       AND (
           NEW.epistemic_state<>'invalid'
           OR creating.authority_receipt_kind<>'server_validation'
           OR creating.authority_receipt_id IS NULL
           OR creating.authority_receipt_hash IS NULL
           OR creating.event_kind<>'invalidated'
           OR NOT EXISTS (
               SELECT 1 FROM hypothesis_server_validation_receipts receipt
                WHERE receipt.receipt_id=creating.authority_receipt_id
                  AND receipt.operation_id=NEW.operation_id
                  AND receipt.organization_id=NEW.organization_id
                  AND receipt.root_id=NEW.root_id
                  AND receipt.predecessor_revision_id IS NOT DISTINCT FROM NEW.predecessor_revision_id
                  AND receipt.validated_revision_id=NEW.revision_id
                  AND receipt.validated_revision_ingredients_hash=NEW.revision_ingredients_hash
                  AND receipt.receipt_hash=creating.authority_receipt_hash
           )
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_SERVER_VALIDATION_RECEIPT_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='hypothesis_revision_adjudication' THEN
        RAISE EXCEPTION 'PLAN_C_REVISION_ADJUDICATION_AUTHORITY_NOT_INSTALLED' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM attack_hypothesis_heads head
         WHERE head.root_id=NEW.root_id
           AND head.operation_id=NEW.operation_id
           AND head.organization_id=NEW.organization_id
           AND head.head_revision_id=NEW.revision_id
           AND head.head_revision_hash=NEW.revision_hash
           AND head.head_semantic_key_hash=NEW.semantic_key_hash
           AND head.head_epistemic_state=NEW.epistemic_state
           AND head.head_lifecycle_state=NEW.lifecycle_state
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_HEAD_CAS_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF (SELECT COUNT(*)
          FROM attack_hypothesis_verification_plans plan
         WHERE plan.revision_id=NEW.revision_id)<>1
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_VERIFICATION_PLAN_EXACT_ONE_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER attack_hypothesis_revision_creating_event_required
AFTER INSERT ON attack_hypothesis_revisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_hypothesis_revision_creating_event();

CREATE TRIGGER attack_hypotheses_append_only
BEFORE UPDATE OR DELETE ON attack_hypotheses
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_revisions_append_only
BEFORE UPDATE OR DELETE ON attack_hypothesis_revisions
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_revision_sources_append_only
BEFORE UPDATE OR DELETE ON attack_hypothesis_revision_sources
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_objectives_append_only
BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_objectives
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_claim_components_append_only
BEFORE UPDATE OR DELETE ON attack_hypothesis_claim_components
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_contracts_append_only
BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_contracts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_plans_append_only
BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_plans
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_server_validation_receipts_append_only
BEFORE UPDATE OR DELETE ON hypothesis_server_validation_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_candidate_gate_decisions_append_only
BEFORE UPDATE OR DELETE ON hypothesis_candidate_gate_decisions
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_candidate_gate_decision_members_append_only
BEFORE UPDATE OR DELETE ON hypothesis_candidate_gate_decision_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_state_events_append_only
BEFORE UPDATE OR DELETE ON attack_hypothesis_state_events
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- ---------------------------------------------------------------------------
-- Candidate Analysis authority snapshot and attempt spine
-- ---------------------------------------------------------------------------

CREATE TABLE candidate_analysis_snapshots (
    snapshot_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    wave_ordinal INTEGER NOT NULL CHECK (wave_ordinal>=0),
    scope_snapshot_id UUID,
    genesis BOOLEAN NOT NULL,
    previous_generation_seal_id UUID,
    source_set_hash TEXT NOT NULL CHECK (source_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    fact_delta_watermark BIGINT NOT NULL DEFAULT 0 CHECK (fact_delta_watermark>=0),
    capability_revision_hash TEXT NOT NULL CHECK (capability_revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    policy_revision_hash TEXT NOT NULL CHECK (policy_revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    credential_revision_hash TEXT NOT NULL CHECK (credential_revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    snapshot_status TEXT NOT NULL CHECK (snapshot_status IN (
        'sealed_ready','blocked_authority_bundle'
    )),
    tool_truth_authority_bundle_seal_id UUID NOT NULL,
    stable_consumer_request_id UUID NOT NULL,
    relevant_root_count BIGINT NOT NULL CHECK (relevant_root_count=4),
    relevant_root_set_hash TEXT NOT NULL CHECK (relevant_root_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    bundle_member_count BIGINT NOT NULL CHECK (bundle_member_count=4),
    bundle_member_set_hash TEXT NOT NULL CHECK (bundle_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    semantic_authority_bundle_hash TEXT NOT NULL CHECK (semantic_authority_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    freshness_attestation_bundle_hash TEXT NOT NULL CHECK (freshness_attestation_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    temporal_validity_bundle_hash TEXT NOT NULL CHECK (temporal_validity_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    temporal_validity_policy_set_hash TEXT NOT NULL CHECK (temporal_validity_policy_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    target_state_epoch_set_hash TEXT NOT NULL CHECK (target_state_epoch_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    observation_window_hash TEXT NOT NULL CHECK (observation_window_hash ~ '^sha256:[0-9a-f]{64}$'),
    bundle_sealed_at TIMESTAMPTZ NOT NULL,
    candidate_snapshot_authority_hash TEXT NOT NULL CHECK (candidate_snapshot_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(snapshot_id,operation_id,organization_id),
    UNIQUE(snapshot_id,operation_id,organization_id,tool_truth_authority_bundle_seal_id),
    UNIQUE(operation_id,organization_id,wave_ordinal),
    FOREIGN KEY(
        tool_truth_authority_bundle_seal_id,operation_id,organization_id,
        stable_consumer_request_id,relevant_root_set_hash,bundle_member_set_hash,
        semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
        target_state_epoch_set_hash,bundle_sealed_at
    ) REFERENCES tool_truth_authority_bundle_seals(
        id,operation_id,organization_id,stable_consumer_request_id,
        relevant_root_set_hash,member_set_hash,
        semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
        target_state_epoch_set_hash,sealed_at
    ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_snapshot_authority_bundle_members (
    snapshot_member_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    bundle_seal_id UUID NOT NULL,
    tool_truth_authority_bundle_member_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 3),
    root_family TEXT NOT NULL CHECK (root_family IN ('ti','eas','enum','vuln')),
    root_execution_authority_id UUID NOT NULL,
    root_denominator_id UUID NOT NULL,
    root_denominator_hash TEXT NOT NULL,
    authority_set_seal_id UUID NOT NULL,
    authority_set_semantic_hash TEXT NOT NULL,
    authority_set_graph_hash TEXT NOT NULL,
    authority_set_freshness_hash TEXT NOT NULL,
    temporal_validity_policy_set_hash TEXT NOT NULL,
    target_state_epoch_set_hash TEXT NOT NULL,
    semantic_status TEXT NOT NULL,
    temporal_validity_status TEXT NOT NULL,
    member_status TEXT NOT NULL CHECK (member_status IN (
        'consistent_fresh','semantic_invalid','expired','mixed_epoch','skew_exceeded'
    )),
    member_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(snapshot_id,ordinal),
    UNIQUE(snapshot_id,root_family),
    FOREIGN KEY(snapshot_id,operation_id,organization_id,bundle_seal_id)
        REFERENCES candidate_analysis_snapshots(
            snapshot_id,operation_id,organization_id,tool_truth_authority_bundle_seal_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(
        tool_truth_authority_bundle_member_id,bundle_seal_id,operation_id,organization_id,
        ordinal,root_family,root_execution_authority_id,root_denominator_id,
        root_denominator_hash,authority_set_seal_id,authority_set_semantic_hash,
        authority_set_graph_hash,authority_set_freshness_hash,
        temporal_validity_policy_set_hash,target_state_epoch_set_hash,
        semantic_status,temporal_validity_status,member_status,member_hash
    ) REFERENCES tool_truth_authority_bundle_members(
        id,bundle_seal_id,operation_id,organization_id,ordinal,root_family,
        root_execution_authority_id,root_denominator_id,root_denominator_hash,
        authority_set_seal_id,authority_set_semantic_hash,
        authority_set_graph_hash,authority_set_freshness_hash,
        temporal_validity_policy_set_hash,target_state_epoch_set_hash,
        semantic_status,temporal_validity_status,member_status,member_hash
    ) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_candidate_snapshot_exact_authority_bundle()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    ordinal_count BIGINT;
    root_family_count BIGINT;
    fresh_count BIGINT;
BEGIN
    SELECT COUNT(*),COUNT(DISTINCT ordinal),COUNT(DISTINCT root_family),
           COUNT(*) FILTER (WHERE member_status='consistent_fresh')
      INTO actual_count,ordinal_count,root_family_count,fresh_count
      FROM candidate_analysis_snapshot_authority_bundle_members
     WHERE snapshot_id=NEW.snapshot_id;
    IF actual_count<>4 OR ordinal_count<>4 OR root_family_count<>4 THEN
        RAISE EXCEPTION 'CANDIDATE_SNAPSHOT_AUTHORITY_BUNDLE_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    IF NEW.snapshot_status='sealed_ready' AND fresh_count<>4 THEN
        RAISE EXCEPTION 'CANDIDATE_SNAPSHOT_ALL_FRESH_AUTHORITY_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER candidate_analysis_snapshot_exact_authority_bundle
AFTER INSERT ON candidate_analysis_snapshots
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_snapshot_exact_authority_bundle();

CREATE TABLE candidate_analysis_attempts (
    analysis_attempt_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal>=0),
    predecessor_attempt_id UUID REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    attempt_input_hash TEXT NOT NULL CHECK (attempt_input_hash ~ '^sha256:[0-9a-f]{64}$'),
    attack_class_checklist_version TEXT NOT NULL,
    attack_class_checklist_digest TEXT NOT NULL CHECK (attack_class_checklist_digest ~ '^sha256:[0-9a-f]{64}$'),
    trust_boundary_checklist_version TEXT NOT NULL,
    trust_boundary_checklist_digest TEXT NOT NULL CHECK (trust_boundary_checklist_digest ~ '^sha256:[0-9a-f]{64}$'),
    coverage_sampling_contract_version TEXT NOT NULL,
    coverage_sampling_contract_digest TEXT NOT NULL CHECK (coverage_sampling_contract_digest ~ '^sha256:[0-9a-f]{64}$'),
    retry_limit INTEGER NOT NULL CHECK (retry_limit BETWEEN 0 AND 8),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(snapshot_id,attempt_ordinal),
    UNIQUE(analysis_attempt_id,snapshot_id,operation_id,organization_id),
    FOREIGN KEY(snapshot_id,operation_id,organization_id)
        REFERENCES candidate_analysis_snapshots(snapshot_id,operation_id,organization_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION candidate_attempt_requires_ready_snapshot()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM candidate_analysis_snapshots snapshot
         WHERE snapshot.snapshot_id=NEW.snapshot_id
           AND snapshot.snapshot_status='sealed_ready'
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ANALYSIS_SNAPSHOT_NOT_READY' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER candidate_analysis_attempt_requires_ready_snapshot
BEFORE INSERT ON candidate_analysis_attempts
FOR EACH ROW EXECUTE FUNCTION candidate_attempt_requires_ready_snapshot();

CREATE TRIGGER candidate_analysis_snapshots_append_only
BEFORE UPDATE OR DELETE ON candidate_analysis_snapshots
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_snapshot_members_append_only
BEFORE UPDATE OR DELETE ON candidate_analysis_snapshot_authority_bundle_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_attempts_append_only
BEFORE UPDATE OR DELETE ON candidate_analysis_attempts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

ALTER TABLE hypothesis_candidate_gate_decisions
    ADD CONSTRAINT hypothesis_candidate_gate_decision_snapshot_fk
        FOREIGN KEY(candidate_snapshot_id,operation_id,organization_id)
        REFERENCES candidate_analysis_snapshots(snapshot_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    ADD CONSTRAINT hypothesis_candidate_gate_decision_attempt_fk
        FOREIGN KEY(
            analysis_attempt_id,candidate_snapshot_id,operation_id,organization_id
        ) REFERENCES candidate_analysis_attempts(
            analysis_attempt_id,snapshot_id,operation_id,organization_id
        ) ON DELETE RESTRICT;

-- ---------------------------------------------------------------------------
-- Projection heads and immutable batch spine
-- ---------------------------------------------------------------------------

CREATE TABLE investigation_projection_source_heads (
    operation_id UUID PRIMARY KEY REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    last_source_batch_seq BIGINT NOT NULL DEFAULT 0 CHECK (last_source_batch_seq>=0),
    last_source_batch_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE investigation_projection_heads (
    operation_id UUID PRIMARY KEY REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    projection_schema_version INTEGER NOT NULL DEFAULT 1 CHECK (projection_schema_version=1),
    change_seq BIGINT NOT NULL DEFAULT 0 CHECK (change_seq>=0),
    last_projected_batch_id UUID,
    cursor_salt BYTEA NOT NULL DEFAULT gen_random_bytes(32) CHECK (octet_length(cursor_salt)=32),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE investigation_projection_outbox_batches (
    batch_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID,
    source_batch_seq BIGINT NOT NULL CHECK (source_batch_seq>0),
    predecessor_batch_id UUID,
    stable_request_id UUID NOT NULL,
    source_transaction_id UUID NOT NULL,
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_occurred_at TIMESTAMPTZ,
    source_time_status TEXT NOT NULL CHECK (source_time_status IN ('known','historical_unknown')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((source_time_status='known')=(source_occurred_at IS NOT NULL)),
    UNIQUE(operation_id,source_batch_seq),
    UNIQUE(operation_id,stable_request_id),
    UNIQUE(batch_id,operation_id),
    UNIQUE(batch_id,operation_id,source_batch_seq),
    FOREIGN KEY(predecessor_batch_id) REFERENCES investigation_projection_outbox_batches(batch_id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE investigation_projection_entity_versions (
    operation_id UUID NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id UUID NOT NULL,
    entity_version BIGINT NOT NULL CHECK (entity_version>0),
    batch_id UUID NOT NULL,
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    projection_hash TEXT NOT NULL CHECK (projection_hash ~ '^sha256:[0-9a-f]{64}$'),
    projection_body JSONB NOT NULL CHECK (jsonb_typeof(projection_body)='object'),
    predecessor_absent BOOLEAN NOT NULL,
    predecessor_entity_version BIGINT,
    predecessor_projection_hash TEXT,
    change_seq BIGINT NOT NULL CHECK (change_seq>0),
    source_occurred_at TIMESTAMPTZ,
    source_time_status TEXT NOT NULL CHECK (source_time_status IN ('known','historical_unknown')),
    projected_at TIMESTAMPTZ NOT NULL,
    invalidation_reason TEXT,
    PRIMARY KEY(operation_id,entity_kind,entity_id,entity_version),
    UNIQUE(operation_id,change_seq),
    UNIQUE(operation_id,entity_kind,entity_id,entity_version,projection_hash),
    CHECK ((source_time_status='known')=(source_occurred_at IS NOT NULL)),
    CHECK (
        (entity_version=1 AND predecessor_absent AND predecessor_entity_version IS NULL
            AND predecessor_projection_hash IS NULL)
        OR (entity_version>1 AND NOT predecessor_absent
            AND predecessor_entity_version=entity_version-1
            AND predecessor_projection_hash ~ '^sha256:[0-9a-f]{64}$')
    ),
    FOREIGN KEY(batch_id,operation_id)
        REFERENCES investigation_projection_outbox_batches(batch_id,operation_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,entity_kind,entity_id,predecessor_entity_version,predecessor_projection_hash)
        REFERENCES investigation_projection_entity_versions(
            operation_id,entity_kind,entity_id,entity_version,projection_hash
        ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE investigation_projection_batch_receipts (
    receipt_id UUID PRIMARY KEY,
    batch_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    source_batch_seq BIGINT NOT NULL,
    predecessor_batch_id UUID,
    first_change_seq BIGINT NOT NULL CHECK (first_change_seq>0),
    last_change_seq BIGINT NOT NULL CHECK (last_change_seq>=first_change_seq),
    entity_version_manifest_hash TEXT NOT NULL CHECK (entity_version_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    change_manifest_hash TEXT NOT NULL CHECK (change_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    timeline_manifest_hash TEXT NOT NULL CHECK (timeline_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    projected_at TIMESTAMPTZ NOT NULL,
    UNIQUE(operation_id,source_batch_seq),
    FOREIGN KEY(batch_id,operation_id,source_batch_seq)
        REFERENCES investigation_projection_outbox_batches(batch_id,operation_id,source_batch_seq)
        ON DELETE RESTRICT
);

INSERT INTO investigation_projection_source_heads(operation_id)
SELECT operation_id FROM operation_state
ON CONFLICT(operation_id) DO NOTHING;

INSERT INTO investigation_projection_heads(operation_id)
SELECT operation_id FROM operation_state
ON CONFLICT(operation_id) DO NOTHING;

CREATE FUNCTION initialize_investigation_projection_heads()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    INSERT INTO investigation_projection_source_heads(operation_id)
    VALUES(NEW.operation_id) ON CONFLICT(operation_id) DO NOTHING;
    INSERT INTO investigation_projection_heads(operation_id)
    VALUES(NEW.operation_id) ON CONFLICT(operation_id) DO NOTHING;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_state_initialize_investigation_projection_heads
AFTER INSERT ON operation_state
FOR EACH ROW EXECUTE FUNCTION initialize_investigation_projection_heads();

CREATE FUNCTION enforce_investigation_projection_head_identity_immutable()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF ROW(NEW.projection_schema_version,NEW.cursor_salt)
       IS DISTINCT FROM ROW(OLD.projection_schema_version,OLD.cursor_salt)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PROJECTION_HEAD_IDENTITY_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_projection_head_identity_immutable
BEFORE UPDATE OF projection_schema_version,cursor_salt ON investigation_projection_heads
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_projection_head_identity_immutable();

CREATE TRIGGER investigation_projection_source_heads_delete_forbidden
BEFORE DELETE ON investigation_projection_source_heads
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER investigation_projection_heads_delete_forbidden
BEFORE DELETE ON investigation_projection_heads
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

CREATE TRIGGER investigation_projection_batches_append_only
BEFORE UPDATE OR DELETE ON investigation_projection_outbox_batches
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER investigation_projection_entity_versions_append_only
BEFORE UPDATE OR DELETE ON investigation_projection_entity_versions
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER investigation_projection_batch_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_projection_batch_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- ---------------------------------------------------------------------------
-- VerificationContract and HypothesisVerificationPlan exact sets
-- ---------------------------------------------------------------------------

ALTER TABLE attack_hypothesis_verification_contracts
    ADD CONSTRAINT attack_hypothesis_verification_contract_owner_unique
    UNIQUE(contract_id,revision_id,objective_id);

ALTER TABLE attack_hypothesis_verification_plans
    ADD CONSTRAINT attack_hypothesis_verification_plan_owner_unique
    UNIQUE(plan_id,revision_id);

CREATE TABLE attack_hypothesis_verification_objective_claim_components (
    binding_id UUID PRIMARY KEY,
    contract_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    objective_id UUID NOT NULL,
    claim_component_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    component_member_hash TEXT NOT NULL CHECK (component_member_hash ~ '^sha256:[0-9a-f]{64}$'),
    binding_member_hash TEXT NOT NULL CHECK (binding_member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(contract_id,ordinal),
    UNIQUE(contract_id,claim_component_id),
    FOREIGN KEY(contract_id,revision_id,objective_id)
        REFERENCES attack_hypothesis_verification_contracts(
            contract_id,revision_id,objective_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(claim_component_id,revision_id,component_member_hash)
        REFERENCES attack_hypothesis_claim_components(
            component_id,revision_id,member_hash
        ) ON DELETE RESTRICT
);

CREATE TABLE attack_hypothesis_verification_predicate_components (
    predicate_component_id UUID PRIMARY KEY,
    contract_id UUID NOT NULL REFERENCES attack_hypothesis_verification_contracts(contract_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    semantic_key TEXT NOT NULL CHECK (btrim(semantic_key)<>''),
    predicate_schema TEXT NOT NULL CHECK (btrim(predicate_schema)<>''),
    predicate_version INTEGER NOT NULL CHECK (predicate_version>0),
    normalized_arguments JSONB NOT NULL CHECK (jsonb_typeof(normalized_arguments)='object'),
    normalized_arguments_hash TEXT NOT NULL CHECK (normalized_arguments_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_polarity TEXT NOT NULL CHECK (expected_polarity IN ('positive','negative')),
    prerequisite_hash TEXT NOT NULL CHECK (prerequisite_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(contract_id,ordinal),
    UNIQUE(contract_id,semantic_key),
    UNIQUE(predicate_component_id,contract_id),
    UNIQUE(predicate_component_id,contract_id,semantic_key),
    UNIQUE(predicate_component_id,contract_id,semantic_key,member_hash)
);

CREATE TABLE attack_hypothesis_verification_required_controls (
    required_control_id UUID PRIMARY KEY,
    contract_id UUID NOT NULL REFERENCES attack_hypothesis_verification_contracts(contract_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    control_id TEXT NOT NULL CHECK (btrim(control_id)<>''),
    control_version INTEGER NOT NULL CHECK (control_version>0),
    control_contract_hash TEXT NOT NULL CHECK (control_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(contract_id,ordinal),
    UNIQUE(contract_id,control_id,control_version),
    UNIQUE(required_control_id,contract_id,control_id,control_version,control_contract_hash),
    UNIQUE(
        required_control_id,contract_id,control_id,control_version,
        control_contract_hash,member_hash
    )
);

CREATE TABLE attack_hypothesis_verification_pair_bindings (
    pair_binding_id UUID PRIMARY KEY,
    contract_id UUID NOT NULL REFERENCES attack_hypothesis_verification_contracts(contract_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    pair_key TEXT NOT NULL CHECK (btrim(pair_key)<>''),
    baseline_component_id UUID NOT NULL,
    baseline_component_key TEXT NOT NULL CHECK (btrim(baseline_component_key)<>''),
    variant_component_id UUID NOT NULL,
    variant_component_key TEXT NOT NULL CHECK (btrim(variant_component_key)<>''),
    required_control_member_id UUID NOT NULL,
    required_control_id TEXT NOT NULL CHECK (btrim(required_control_id)<>''),
    required_control_version INTEGER NOT NULL CHECK (required_control_version>0),
    required_control_contract_hash TEXT NOT NULL CHECK (required_control_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    required_control_member_hash TEXT NOT NULL CHECK (required_control_member_hash ~ '^sha256:[0-9a-f]{64}$'),
    comparator_rule_id TEXT NOT NULL CHECK (btrim(comparator_rule_id)<>''),
    comparator_rule_version INTEGER NOT NULL CHECK (comparator_rule_version>0),
    comparator_rule_digest TEXT NOT NULL CHECK (comparator_rule_digest ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (baseline_component_id<>variant_component_id),
    UNIQUE(contract_id,ordinal),
    UNIQUE(contract_id,pair_key),
    UNIQUE(contract_id,baseline_component_id,variant_component_id),
    FOREIGN KEY(baseline_component_id,contract_id,baseline_component_key)
        REFERENCES attack_hypothesis_verification_predicate_components(
            predicate_component_id,contract_id,semantic_key
        ) ON DELETE RESTRICT,
    FOREIGN KEY(variant_component_id,contract_id,variant_component_key)
        REFERENCES attack_hypothesis_verification_predicate_components(
            predicate_component_id,contract_id,semantic_key
        ) ON DELETE RESTRICT,
    FOREIGN KEY(
        required_control_member_id,contract_id,required_control_id,
        required_control_version,required_control_contract_hash,
        required_control_member_hash
    )
        REFERENCES attack_hypothesis_verification_required_controls(
            required_control_id,contract_id,control_id,control_version,
            control_contract_hash,member_hash
        ) ON DELETE RESTRICT
);

CREATE TABLE attack_hypothesis_verification_ordered_steps (
    ordered_step_id UUID PRIMARY KEY,
    contract_id UUID NOT NULL REFERENCES attack_hypothesis_verification_contracts(contract_id) ON DELETE RESTRICT,
    step_ordinal INTEGER NOT NULL CHECK (step_ordinal>=0),
    predicate_component_id UUID NOT NULL,
    component_key TEXT NOT NULL CHECK (btrim(component_key)<>''),
    predecessor_step_ordinal INTEGER CHECK (predecessor_step_ordinal>=0),
    session_binding_key_schema TEXT NOT NULL CHECK (btrim(session_binding_key_schema)<>''),
    session_binding_key_version INTEGER NOT NULL CHECK (session_binding_key_version>0),
    session_scope TEXT NOT NULL CHECK (session_scope='same_execution_session'),
    interleaving_policy TEXT NOT NULL CHECK (interleaving_policy='forbid'),
    reset_policy TEXT NOT NULL CHECK (reset_policy='restart_at_step_zero'),
    step_hash TEXT NOT NULL CHECK (step_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(contract_id,step_ordinal),
    UNIQUE(ordered_step_id,contract_id),
    FOREIGN KEY(predicate_component_id,contract_id,component_key)
        REFERENCES attack_hypothesis_verification_predicate_components(
            predicate_component_id,contract_id,semantic_key
        ) ON DELETE RESTRICT,
    FOREIGN KEY(contract_id,predecessor_step_ordinal)
        REFERENCES attack_hypothesis_verification_ordered_steps(contract_id,step_ordinal)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    CHECK ((step_ordinal=0)=(predecessor_step_ordinal IS NULL))
);

CREATE TABLE attack_hypothesis_verification_plan_objectives (
    plan_objective_id UUID PRIMARY KEY,
    plan_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    objective_id UUID NOT NULL,
    verification_contract_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    objective_hash TEXT NOT NULL CHECK (objective_hash ~ '^sha256:[0-9a-f]{64}$'),
    verification_contract_version INTEGER NOT NULL CHECK (verification_contract_version=1),
    verification_contract_hash TEXT NOT NULL CHECK (verification_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    claim_component_count BIGINT NOT NULL CHECK (claim_component_count>0),
    claim_component_set_hash TEXT NOT NULL CHECK (claim_component_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    stopping_criteria_hash TEXT NOT NULL CHECK (stopping_criteria_hash ~ '^sha256:[0-9a-f]{64}$'),
    outcome_requirement TEXT NOT NULL CHECK (outcome_requirement IN (
        'satisfy_bound_components','satisfy_or_falsify_bound_required_components'
    )),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(plan_id,ordinal),
    UNIQUE(plan_id,objective_id),
    UNIQUE(plan_objective_id,plan_id,revision_id,claim_component_set_hash),
    UNIQUE(
        plan_objective_id,plan_id,revision_id,
        claim_component_set_hash,verification_contract_hash
    ),
    UNIQUE(
        plan_objective_id,plan_id,revision_id,
        claim_component_set_hash,verification_contract_hash,member_hash
    ),
    FOREIGN KEY(plan_id,revision_id)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(
        verification_contract_id,revision_id,objective_id,
        verification_contract_version,verification_contract_hash
    )
        REFERENCES attack_hypothesis_verification_contracts(
            contract_id,revision_id,objective_id,contract_version,contract_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(objective_id,revision_id,objective_hash)
        REFERENCES attack_hypothesis_verification_objectives(
            objective_id,revision_id,objective_hash
        ) ON DELETE RESTRICT
);

CREATE TABLE attack_hypothesis_verification_plan_paths (
    path_id UUID PRIMARY KEY,
    plan_id UUID NOT NULL REFERENCES attack_hypothesis_verification_plans(plan_id) ON DELETE RESTRICT,
    path_ordinal INTEGER NOT NULL CHECK (path_ordinal>=0),
    path_key TEXT NOT NULL CHECK (btrim(path_key)<>''),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    path_hash TEXT NOT NULL CHECK (path_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(plan_id,path_ordinal),
    UNIQUE(plan_id,path_key),
    UNIQUE(path_id,plan_id)
);

CREATE TABLE attack_hypothesis_verification_plan_path_members (
    path_member_id UUID PRIMARY KEY,
    path_id UUID NOT NULL,
    plan_id UUID NOT NULL,
    plan_objective_id UUID NOT NULL,
    plan_objective_member_hash TEXT NOT NULL CHECK (plan_objective_member_hash ~ '^sha256:[0-9a-f]{64}$'),
    revision_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    verification_contract_hash TEXT NOT NULL CHECK (verification_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    claim_component_set_hash TEXT NOT NULL CHECK (claim_component_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    role TEXT NOT NULL CHECK (role IN (
        'required_proof','required_proof_and_path_falsifier'
    )),
    falsifier_claim_component_member_hashes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    falsifier_claim_component_count BIGINT NOT NULL CHECK (falsifier_claim_component_count>=0),
    falsifier_claim_component_set_hash TEXT NOT NULL CHECK (falsifier_claim_component_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (role='required_proof'
            AND cardinality(falsifier_claim_component_member_hashes)=0
            AND falsifier_claim_component_count=0)
        OR (role='required_proof_and_path_falsifier'
            AND cardinality(falsifier_claim_component_member_hashes)>0
            AND falsifier_claim_component_count=cardinality(falsifier_claim_component_member_hashes))
    ),
    UNIQUE(path_id,member_ordinal),
    UNIQUE(path_id,plan_objective_id),
    FOREIGN KEY(path_id,plan_id)
        REFERENCES attack_hypothesis_verification_plan_paths(path_id,plan_id) ON DELETE RESTRICT,
    FOREIGN KEY(
        plan_objective_id,plan_id,revision_id,claim_component_set_hash,
        verification_contract_hash,plan_objective_member_hash
    )
        REFERENCES attack_hypothesis_verification_plan_objectives(
            plan_objective_id,plan_id,revision_id,claim_component_set_hash,
            verification_contract_hash,member_hash
        ) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_attack_hypothesis_verification_contract_exact_sets()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    contract attack_hypothesis_verification_contracts%ROWTYPE;
    predicate_count BIGINT;
    predicate_ordinal_count BIGINT;
    predicate_min INTEGER;
    predicate_max INTEGER;
    predicate_set_hash TEXT;
    control_count BIGINT;
    control_ordinal_count BIGINT;
    control_min INTEGER;
    control_max INTEGER;
    control_set_hash TEXT;
    pair_count BIGINT;
    pair_ordinal_count BIGINT;
    pair_min INTEGER;
    pair_max INTEGER;
    pair_set_hash TEXT;
    pair_component_count BIGINT;
    pair_control_count BIGINT;
    ordered_count BIGINT;
    ordered_ordinal_count BIGINT;
    ordered_min INTEGER;
    ordered_max INTEGER;
    ordered_set_hash TEXT;
BEGIN
    SELECT * INTO STRICT contract
      FROM attack_hypothesis_verification_contracts
     WHERE contract_id=NEW.contract_id;
    SELECT COUNT(*),COUNT(DISTINCT ordinal),MIN(ordinal),MAX(ordinal),
           verification_contract_exact_member_set_hash(
               'verification_predicate_set.v1',
               COALESCE(array_agg(member_hash ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO predicate_count,predicate_ordinal_count,predicate_min,predicate_max,predicate_set_hash
      FROM attack_hypothesis_verification_predicate_components
     WHERE contract_id=contract.contract_id;
    SELECT COUNT(*),COUNT(DISTINCT ordinal),MIN(ordinal),MAX(ordinal),
           verification_contract_exact_member_set_hash(
               'verification_control_set.v1',
               COALESCE(array_agg(member_hash ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO control_count,control_ordinal_count,control_min,control_max,control_set_hash
      FROM attack_hypothesis_verification_required_controls
     WHERE contract_id=contract.contract_id;
    SELECT COUNT(*),COUNT(DISTINCT ordinal),MIN(ordinal),MAX(ordinal),
           verification_contract_exact_member_set_hash(
               'verification_paired_differential_set.v1',
               COALESCE(array_agg(member_hash ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO pair_count,pair_ordinal_count,pair_min,pair_max,pair_set_hash
      FROM attack_hypothesis_verification_pair_bindings
     WHERE contract_id=contract.contract_id;
    SELECT COUNT(DISTINCT component_id)
      INTO pair_component_count
      FROM (
          SELECT baseline_component_id AS component_id
            FROM attack_hypothesis_verification_pair_bindings
           WHERE contract_id=contract.contract_id
          UNION ALL
          SELECT variant_component_id
            FROM attack_hypothesis_verification_pair_bindings
           WHERE contract_id=contract.contract_id
      ) AS pair_components;
    SELECT COUNT(DISTINCT required_control_member_id)
      INTO pair_control_count
      FROM attack_hypothesis_verification_pair_bindings
     WHERE contract_id=contract.contract_id;
    SELECT COUNT(*),COUNT(DISTINCT step_ordinal),MIN(step_ordinal),MAX(step_ordinal),
           verification_contract_exact_member_set_hash(
               'verification_ordered_step_set.v1',
               COALESCE(array_agg(step_hash ORDER BY step_ordinal),ARRAY[]::TEXT[])
           )
      INTO ordered_count,ordered_ordinal_count,ordered_min,ordered_max,ordered_set_hash
      FROM attack_hypothesis_verification_ordered_steps
     WHERE contract_id=contract.contract_id;

    IF predicate_count<>contract.predicate_count
       OR predicate_ordinal_count<>predicate_count
       OR predicate_min<>0 OR predicate_max<>predicate_count-1
       OR predicate_set_hash<>contract.predicate_set_hash
       OR control_count<>contract.required_control_count
       OR control_ordinal_count<>control_count
       OR (control_count>0 AND (control_min<>0 OR control_max<>control_count-1))
       OR control_set_hash<>contract.required_control_set_hash
       OR pair_count<>contract.paired_differential_count
       OR pair_ordinal_count<>pair_count
       OR (pair_count>0 AND (pair_min<>0 OR pair_max<>pair_count-1))
       OR pair_set_hash<>contract.paired_differential_set_hash
       OR ordered_count<>contract.ordered_step_count
       OR ordered_ordinal_count<>ordered_count
       OR (ordered_count>0 AND (ordered_min<>0 OR ordered_max<>ordered_count-1))
       OR ordered_set_hash<>contract.ordered_step_set_hash
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_VERIFICATION_CONTRACT_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    IF contract.combinator='paired_differential'
       AND (
           pair_component_count<>predicate_count
           OR pair_count*2<>predicate_count
           OR pair_control_count<>control_count
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_VERIFICATION_PAIR_COMPONENT_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    IF contract.combinator='ordered_sequence'
       AND (
           ordered_count<>predicate_count
           OR (SELECT COUNT(DISTINCT predicate_component_id)
                 FROM attack_hypothesis_verification_ordered_steps
                WHERE contract_id=contract.contract_id)<>predicate_count
           OR (SELECT COUNT(DISTINCT ROW(
                    session_binding_key_schema,session_binding_key_version,session_scope,
                    interleaving_policy,reset_policy
                ))
                 FROM attack_hypothesis_verification_ordered_steps
                WHERE contract_id=contract.contract_id)<>1
           OR EXISTS (
               SELECT 1
                 FROM attack_hypothesis_verification_ordered_steps step
            LEFT JOIN attack_hypothesis_verification_ordered_steps predecessor
                   ON predecessor.contract_id=step.contract_id
                  AND predecessor.step_ordinal=step.predecessor_step_ordinal
                WHERE step.contract_id=contract.contract_id
                  AND step.step_ordinal>0
                  AND predecessor.step_ordinal IS DISTINCT FROM step.step_ordinal-1
           )
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_VERIFICATION_ORDERED_STEP_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER attack_hypothesis_verification_contract_exact_sets
AFTER INSERT ON attack_hypothesis_verification_contracts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_contract_exact_sets();

CREATE CONSTRAINT TRIGGER attack_hypothesis_predicate_component_contract_exact_set
AFTER INSERT ON attack_hypothesis_verification_predicate_components
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_contract_exact_sets();

CREATE CONSTRAINT TRIGGER attack_hypothesis_required_control_contract_exact_set
AFTER INSERT ON attack_hypothesis_verification_required_controls
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_contract_exact_sets();

CREATE CONSTRAINT TRIGGER attack_hypothesis_pair_binding_contract_exact_set
AFTER INSERT ON attack_hypothesis_verification_pair_bindings
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_contract_exact_sets();

CREATE CONSTRAINT TRIGGER attack_hypothesis_ordered_step_contract_exact_set
AFTER INSERT ON attack_hypothesis_verification_ordered_steps
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_contract_exact_sets();

CREATE FUNCTION enforce_attack_hypothesis_verification_plan_exact_sets()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    sealed_plan attack_hypothesis_verification_plans%ROWTYPE;
    required_component_count BIGINT;
    required_component_set_hash TEXT;
    objective_count BIGINT;
    objective_ordinal_count BIGINT;
    objective_min INTEGER;
    objective_max INTEGER;
    objective_set_hash TEXT;
    path_count BIGINT;
    path_ordinal_count BIGINT;
    path_min INTEGER;
    path_max INTEGER;
    path_set_hash TEXT;
BEGIN
    SELECT * INTO STRICT sealed_plan
      FROM attack_hypothesis_verification_plans
     WHERE plan_id=NEW.plan_id;
    SELECT COUNT(*),investigation_exact_member_set_hash(
               'hypothesis_verification_plan_required_components.v1',
               COALESCE(array_agg(member_hash ORDER BY component_ordinal),ARRAY[]::TEXT[])
           )
      INTO required_component_count,required_component_set_hash
      FROM attack_hypothesis_claim_components
     WHERE revision_id=sealed_plan.revision_id AND required;
    SELECT COUNT(*),COUNT(DISTINCT ordinal),MIN(ordinal),MAX(ordinal),
           investigation_exact_member_set_hash(
               'hypothesis_verification_plan_objectives.v1',
               COALESCE(array_agg(member_hash ORDER BY ordinal),ARRAY[]::TEXT[])
           )
      INTO objective_count,objective_ordinal_count,objective_min,objective_max,objective_set_hash
      FROM attack_hypothesis_verification_plan_objectives
     WHERE plan_id=sealed_plan.plan_id;
    SELECT COUNT(*),COUNT(DISTINCT path_ordinal),MIN(path_ordinal),MAX(path_ordinal),
           investigation_exact_member_set_hash(
               'hypothesis_verification_plan_paths.v1',
               COALESCE(array_agg(path_hash ORDER BY path_ordinal),ARRAY[]::TEXT[])
           )
      INTO path_count,path_ordinal_count,path_min,path_max,path_set_hash
      FROM attack_hypothesis_verification_plan_paths
     WHERE plan_id=sealed_plan.plan_id;

    IF required_component_count<>sealed_plan.required_claim_component_count
       OR required_component_set_hash<>sealed_plan.required_claim_component_set_hash
       OR objective_count<>sealed_plan.objective_count
       OR objective_ordinal_count<>objective_count
       OR objective_min<>0 OR objective_max<>objective_count-1
       OR objective_set_hash<>sealed_plan.objective_set_hash
       OR path_count<>sealed_plan.proof_path_count
       OR path_ordinal_count<>path_count
       OR path_min<>0 OR path_max<>path_count-1
       OR path_set_hash<>sealed_plan.proof_path_set_hash
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_VERIFICATION_PLAN_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_hypothesis_verification_plan_objectives plan_objective
          JOIN attack_hypothesis_verification_contracts contract
            ON contract.contract_id=plan_objective.verification_contract_id
           AND contract.revision_id=plan_objective.revision_id
           AND contract.objective_id=plan_objective.objective_id
     LEFT JOIN LATERAL (
            SELECT COUNT(*) AS member_count,
                   investigation_exact_member_set_hash(
                       'hypothesis_verification_plan_objective_components.v1',
                       COALESCE(array_agg(component_member_hash ORDER BY ordinal),ARRAY[]::TEXT[])
                   ) AS member_set_hash
              FROM attack_hypothesis_verification_objective_claim_components binding
             WHERE binding.contract_id=plan_objective.verification_contract_id
        ) component_set ON TRUE
         WHERE plan_objective.plan_id=sealed_plan.plan_id
           AND (
               component_set.member_count<>plan_objective.claim_component_count
               OR component_set.member_set_hash<>plan_objective.claim_component_set_hash
               OR contract.stopping_criteria_hash<>plan_objective.stopping_criteria_hash
           )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_OBJECTIVE_COMPONENT_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_hypothesis_verification_objectives objective
         WHERE objective.revision_id=sealed_plan.revision_id
           AND NOT EXISTS (
               SELECT 1 FROM attack_hypothesis_verification_plan_objectives plan_objective
                WHERE plan_objective.plan_id=sealed_plan.plan_id
                  AND plan_objective.objective_id=objective.objective_id
           )
    ) OR EXISTS (
        SELECT 1
          FROM attack_hypothesis_verification_plan_objectives plan_objective
         WHERE plan_objective.plan_id=sealed_plan.plan_id
           AND NOT EXISTS (
               SELECT 1 FROM attack_hypothesis_verification_plan_path_members path_member
                WHERE path_member.plan_id=sealed_plan.plan_id
                  AND path_member.plan_objective_id=plan_objective.plan_objective_id
           )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_OBJECTIVE_COVERAGE_REQUIRED'
            USING ERRCODE='23514';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_hypothesis_verification_plan_paths path
     LEFT JOIN LATERAL (
            SELECT COUNT(*) AS member_count,COUNT(DISTINCT member_ordinal) AS ordinal_count,
                   MIN(member_ordinal) AS minimum_ordinal,MAX(member_ordinal) AS maximum_ordinal,
                   investigation_exact_member_set_hash(
                       'hypothesis_verification_plan_path_members.v1',
                       COALESCE(array_agg(member_hash ORDER BY member_ordinal),ARRAY[]::TEXT[])
                   ) AS member_set_hash,
                   COUNT(*) FILTER (
                       WHERE role='required_proof_and_path_falsifier'
                   ) AS falsifier_count
              FROM attack_hypothesis_verification_plan_path_members member
             WHERE member.path_id=path.path_id
        ) member_set ON TRUE
         WHERE path.plan_id=sealed_plan.plan_id
           AND (
               member_set.member_count<>path.member_count
               OR member_set.ordinal_count<>member_set.member_count
               OR member_set.minimum_ordinal<>0
               OR member_set.maximum_ordinal<>member_set.member_count-1
               OR member_set.member_set_hash<>path.member_set_hash
               OR member_set.falsifier_count=0
           )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_PATH_MEMBER_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_hypothesis_verification_plan_path_members member
         WHERE member.plan_id=sealed_plan.plan_id
           AND (
               member.falsifier_claim_component_member_hashes IS DISTINCT FROM
                   ARRAY(
                       SELECT DISTINCT falsifier.member_hash
                         FROM unnest(member.falsifier_claim_component_member_hashes)
                              AS falsifier(member_hash)
                        ORDER BY falsifier.member_hash
                   )
               OR member.falsifier_claim_component_set_hash<>
                   investigation_exact_member_set_hash(
                       'hypothesis_verification_plan_path_falsifiers.v1',
                       member.falsifier_claim_component_member_hashes
                   )
               OR EXISTS (
                   SELECT 1
                     FROM unnest(member.falsifier_claim_component_member_hashes)
                          AS falsifier(member_hash)
                    WHERE NOT EXISTS (
                        SELECT 1
                          FROM attack_hypothesis_verification_plan_objectives plan_objective
                          JOIN attack_hypothesis_verification_objective_claim_components binding
                            ON binding.contract_id=plan_objective.verification_contract_id
                           AND binding.component_member_hash=falsifier.member_hash
                          JOIN attack_hypothesis_claim_components component
                            ON component.component_id=binding.claim_component_id
                           AND component.revision_id=member.revision_id
                           AND component.required
                         WHERE plan_objective.plan_objective_id=member.plan_objective_id
                    )
               )
           )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_PATH_FALSIFIER_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_hypothesis_verification_plan_paths path
         WHERE path.plan_id=sealed_plan.plan_id
           AND (
               EXISTS (
                   SELECT component.member_hash
                     FROM attack_hypothesis_claim_components component
                    WHERE component.revision_id=sealed_plan.revision_id AND component.required
                   EXCEPT
                   SELECT binding.component_member_hash
                     FROM attack_hypothesis_verification_plan_path_members path_member
                     JOIN attack_hypothesis_verification_plan_objectives plan_objective
                       ON plan_objective.plan_objective_id=path_member.plan_objective_id
                     JOIN attack_hypothesis_verification_objective_claim_components binding
                       ON binding.contract_id=plan_objective.verification_contract_id
                    WHERE path_member.path_id=path.path_id
               )
               OR EXISTS (
                   SELECT binding.component_member_hash
                     FROM attack_hypothesis_verification_plan_path_members path_member
                     JOIN attack_hypothesis_verification_plan_objectives plan_objective
                       ON plan_objective.plan_objective_id=path_member.plan_objective_id
                     JOIN attack_hypothesis_verification_objective_claim_components binding
                       ON binding.contract_id=plan_objective.verification_contract_id
                    WHERE path_member.path_id=path.path_id
                   EXCEPT
                   SELECT component.member_hash
                     FROM attack_hypothesis_claim_components component
                    WHERE component.revision_id=sealed_plan.revision_id AND component.required
               )
           )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_PATH_REQUIRED_COMPONENT_COVERAGE_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER attack_hypothesis_verification_plan_exact_sets
AFTER INSERT ON attack_hypothesis_verification_plans
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_plan_exact_sets();

CREATE CONSTRAINT TRIGGER attack_hypothesis_plan_objective_exact_set
AFTER INSERT ON attack_hypothesis_verification_plan_objectives
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_plan_exact_sets();

CREATE CONSTRAINT TRIGGER attack_hypothesis_plan_path_exact_set
AFTER INSERT ON attack_hypothesis_verification_plan_paths
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_plan_exact_sets();

CREATE CONSTRAINT TRIGGER attack_hypothesis_plan_path_member_exact_set
AFTER INSERT ON attack_hypothesis_verification_plan_path_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_hypothesis_verification_plan_exact_sets();

-- These three inputs are compiled before the immutable plan header.  Once a
-- plan exists, appending another objective/component/binding would silently
-- change its denominator without touching any plan-owned child table.
CREATE FUNCTION reject_attack_hypothesis_sealed_plan_input_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_TABLE_NAME='attack_hypothesis_claim_components'
       AND EXISTS (
           SELECT 1 FROM attack_hypothesis_verification_plans plan
            WHERE plan.revision_id=(to_jsonb(NEW)->>'revision_id')::UUID
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_CLAIM_COMPONENT_SET_SEALED'
            USING ERRCODE='23514';
    END IF;
    IF TG_TABLE_NAME='attack_hypothesis_verification_objectives'
       AND EXISTS (
           SELECT 1 FROM attack_hypothesis_verification_plans plan
            WHERE plan.revision_id=(to_jsonb(NEW)->>'revision_id')::UUID
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_OBJECTIVE_SET_SEALED'
            USING ERRCODE='23514';
    END IF;
    IF TG_TABLE_NAME='attack_hypothesis_verification_objective_claim_components'
       AND EXISTS (
           SELECT 1
             FROM attack_hypothesis_verification_plan_objectives plan_objective
            WHERE plan_objective.verification_contract_id=
                  (to_jsonb(NEW)->>'contract_id')::UUID
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_PLAN_OBJECTIVE_COMPONENT_SET_SEALED'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER attack_hypothesis_claim_components_plan_seal_guard
BEFORE INSERT ON attack_hypothesis_claim_components
FOR EACH ROW EXECUTE FUNCTION reject_attack_hypothesis_sealed_plan_input_insert();

CREATE TRIGGER attack_hypothesis_verification_objectives_plan_seal_guard
BEFORE INSERT ON attack_hypothesis_verification_objectives
FOR EACH ROW EXECUTE FUNCTION reject_attack_hypothesis_sealed_plan_input_insert();

CREATE TRIGGER attack_hypothesis_objective_components_plan_seal_guard
BEFORE INSERT ON attack_hypothesis_verification_objective_claim_components
FOR EACH ROW EXECUTE FUNCTION reject_attack_hypothesis_sealed_plan_input_insert();

-- ---------------------------------------------------------------------------
-- Registry lineage, generation seals, and retained residual risk
-- ---------------------------------------------------------------------------

CREATE TABLE attack_hypothesis_relations (
    relation_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    source_root_id UUID NOT NULL,
    source_revision_id UUID NOT NULL,
    target_root_id UUID NOT NULL,
    target_revision_id UUID NOT NULL,
    relation_kind TEXT NOT NULL CHECK (relation_kind IN (
        'support','contradict','refine','split','merge','derive','duplicate','supersede'
    )),
    relation_hash TEXT NOT NULL CHECK (relation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (source_revision_id<>target_revision_id),
    UNIQUE(operation_id,organization_id,source_revision_id,target_revision_id,relation_kind),
    FOREIGN KEY(source_root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(target_root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(source_revision_id,source_root_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(target_revision_id,target_root_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,root_id,operation_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_generations (
    generation_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    generation_ordinal INTEGER NOT NULL CHECK (generation_ordinal>=0),
    candidate_snapshot_id UUID NOT NULL,
    candidate_gate_decision_id UUID NOT NULL UNIQUE,
    candidate_snapshot_authority_hash TEXT NOT NULL CHECK (candidate_snapshot_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    previous_generation_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,organization_id,generation_ordinal),
    UNIQUE(generation_id,operation_id,organization_id),
    UNIQUE(generation_id,previous_generation_id,operation_id,organization_id),
    FOREIGN KEY(candidate_snapshot_id,operation_id,organization_id)
        REFERENCES candidate_analysis_snapshots(snapshot_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(candidate_gate_decision_id,operation_id,organization_id)
        REFERENCES hypothesis_candidate_gate_decisions(decision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(previous_generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT
);

CREATE TABLE hypothesis_generation_members (
    generation_member_id UUID PRIMARY KEY,
    generation_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(generation_id,ordinal),
    UNIQUE(generation_id,revision_id),
    UNIQUE(generation_member_id,revision_id),
    UNIQUE(
        generation_member_id,generation_id,operation_id,organization_id,revision_id
    ),
    FOREIGN KEY(generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,operation_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_generation_transitions (
    transition_id UUID PRIMARY KEY,
    generation_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    previous_generation_id UUID NOT NULL,
    previous_generation_member_id UUID NOT NULL,
    previous_revision_id UUID NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('unchanged','terminal','successor')),
    transition_hash TEXT NOT NULL CHECK (transition_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(generation_id,previous_generation_member_id),
    UNIQUE(transition_id,operation_id,organization_id),
    FOREIGN KEY(
        generation_id,previous_generation_id,operation_id,organization_id
    ) REFERENCES hypothesis_generations(
        generation_id,previous_generation_id,operation_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        previous_generation_member_id,previous_generation_id,
        operation_id,organization_id,previous_revision_id
    ) REFERENCES hypothesis_generation_members(
        generation_member_id,generation_id,operation_id,organization_id,revision_id
    )
        ON DELETE RESTRICT
);

CREATE TABLE hypothesis_generation_transition_successors (
    successor_id UUID PRIMARY KEY,
    transition_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    successor_revision_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(transition_id,ordinal),
    UNIQUE(transition_id,successor_revision_id),
    FOREIGN KEY(transition_id,operation_id,organization_id)
        REFERENCES hypothesis_generation_transitions(
            transition_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(successor_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,operation_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_generation_seals (
    seal_id UUID PRIMARY KEY,
    generation_id UUID NOT NULL UNIQUE REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    member_count BIGINT NOT NULL CHECK (member_count>=0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    event_count BIGINT NOT NULL CHECK (event_count>=0),
    event_set_hash TEXT NOT NULL CHECK (event_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    open_obligation_set_hash TEXT NOT NULL CHECK (open_obligation_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    controller_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    generation_hash TEXT NOT NULL CHECK (generation_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(seal_id,generation_id,generation_hash),
    UNIQUE(seal_id,generation_id)
);

CREATE TABLE hypothesis_candidate_canonical_apply_receipts (
    apply_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    candidate_gate_decision_id UUID NOT NULL UNIQUE,
    generation_id UUID NOT NULL UNIQUE,
    generation_seal_id UUID NOT NULL UNIQUE,
    projection_outbox_batch_id UUID NOT NULL UNIQUE,
    revision_count BIGINT NOT NULL CHECK (revision_count>=0),
    revision_set_hash TEXT NOT NULL CHECK (revision_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(apply_receipt_id,operation_id,organization_id),
    FOREIGN KEY(candidate_gate_decision_id,operation_id,organization_id)
        REFERENCES hypothesis_candidate_gate_decisions(decision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(generation_seal_id,generation_id)
        REFERENCES hypothesis_generation_seals(seal_id,generation_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(projection_outbox_batch_id,operation_id)
        REFERENCES investigation_projection_outbox_batches(batch_id,operation_id)
        ON DELETE RESTRICT
);

CREATE TABLE hypothesis_candidate_canonical_apply_receipt_members (
    apply_receipt_member_id UUID PRIMARY KEY,
    apply_receipt_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(apply_receipt_id,ordinal),
    UNIQUE(apply_receipt_id,revision_id),
    UNIQUE(revision_id),
    FOREIGN KEY(apply_receipt_id,operation_id,organization_id)
        REFERENCES hypothesis_candidate_canonical_apply_receipts(
            apply_receipt_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(revision_id,revision_hash)
        REFERENCES attack_hypothesis_revisions(
            revision_id,revision_hash
        ) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_hypothesis_candidate_apply_receipt_exact_set()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    receipt hypothesis_candidate_canonical_apply_receipts%ROWTYPE;
    actual_count BIGINT;
    actual_set_hash TEXT;
    gate_writing_count BIGINT;
    gate_matched_count BIGINT;
BEGIN
    SELECT * INTO STRICT receipt
      FROM hypothesis_candidate_canonical_apply_receipts
     WHERE apply_receipt_id=NEW.apply_receipt_id;
    SELECT COUNT(*),candidate_gate_exact_member_set_hash(
               'candidate_apply_revisions.v1',
               COALESCE(array_agg(member_hash ORDER BY ordinal),ARRAY[]::TEXT[]))
      INTO actual_count,actual_set_hash
      FROM hypothesis_candidate_canonical_apply_receipt_members
     WHERE apply_receipt_id=receipt.apply_receipt_id;
    SELECT COUNT(*) INTO gate_writing_count
      FROM hypothesis_candidate_gate_decision_members gate_member
     WHERE gate_member.decision_id=receipt.candidate_gate_decision_id
       AND gate_member.route_kind NOT IN ('attach_current','no_semantic_change');
    SELECT COUNT(*) INTO gate_matched_count
      FROM hypothesis_candidate_gate_decision_members gate_member
     WHERE gate_member.decision_id=receipt.candidate_gate_decision_id
       AND gate_member.route_kind NOT IN ('attach_current','no_semantic_change')
       AND EXISTS(
           SELECT 1 FROM hypothesis_candidate_canonical_apply_receipt_members receipt_member
            WHERE receipt_member.apply_receipt_id=receipt.apply_receipt_id
              AND receipt_member.revision_id=gate_member.successor_revision_id
       );
    IF actual_count<>receipt.revision_count
       OR actual_set_hash<>receipt.revision_set_hash
       OR gate_writing_count<>receipt.revision_count
       OR gate_matched_count<>receipt.revision_count
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_APPLY_RECEIPT_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER hypothesis_candidate_apply_receipt_exact_set
AFTER INSERT ON hypothesis_candidate_canonical_apply_receipts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_hypothesis_candidate_apply_receipt_exact_set();
CREATE CONSTRAINT TRIGGER hypothesis_candidate_apply_receipt_member_exact_set
AFTER INSERT ON hypothesis_candidate_canonical_apply_receipt_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_hypothesis_candidate_apply_receipt_exact_set();

CREATE FUNCTION enforce_hypothesis_candidate_revision_apply_membership()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS(
        SELECT 1 FROM attack_hypothesis_state_events event
         WHERE event.successor_revision_id=NEW.revision_id
           AND event.origin_authority='candidate_analysis'
    ) AND NOT EXISTS(
        SELECT 1 FROM hypothesis_candidate_canonical_apply_receipt_members member
         WHERE member.revision_id=NEW.revision_id
           AND member.operation_id=NEW.operation_id
           AND member.organization_id=NEW.organization_id
           AND member.revision_hash=NEW.revision_hash
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_CANONICAL_APPLY_RECEIPT_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER attack_hypothesis_revisions_apply_receipt_required
AFTER INSERT ON attack_hypothesis_revisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_hypothesis_candidate_revision_apply_membership();

CREATE TABLE hypothesis_residual_risks (
    residual_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    revision_id UUID,
    snapshot_id UUID,
    reason_code TEXT NOT NULL CHECK (btrim(reason_code)<>''),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('candidate_analysis','reporting','operator','plan_c')),
    affected_inputs JSONB NOT NULL DEFAULT '[]'::JSONB CHECK (jsonb_typeof(affected_inputs)='array'),
    next_action JSONB NOT NULL CHECK (jsonb_typeof(next_action)='object'),
    residual_hash TEXT NOT NULL CHECK (residual_hash ~ '^sha256:[0-9a-f]{64}$'),
    closed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(
            revision_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(snapshot_id,operation_id,organization_id)
        REFERENCES candidate_analysis_snapshots(
            snapshot_id,operation_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TRIGGER attack_hypothesis_verification_objective_claim_components_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_objective_claim_components FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_predicate_components_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_predicate_components FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_required_controls_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_required_controls FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_pair_bindings_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_pair_bindings FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_ordered_steps_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_ordered_steps FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_plan_objectives_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_plan_objectives FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_plan_paths_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_plan_paths FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_verification_plan_path_members_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_verification_plan_path_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER attack_hypothesis_relations_append_only BEFORE UPDATE OR DELETE ON attack_hypothesis_relations FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_generations_append_only BEFORE UPDATE OR DELETE ON hypothesis_generations FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_generation_members_append_only BEFORE UPDATE OR DELETE ON hypothesis_generation_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_generation_transitions_append_only BEFORE UPDATE OR DELETE ON hypothesis_generation_transitions FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_generation_transition_successors_append_only BEFORE UPDATE OR DELETE ON hypothesis_generation_transition_successors FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_generation_seals_append_only BEFORE UPDATE OR DELETE ON hypothesis_generation_seals FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_residual_risks_append_only BEFORE UPDATE OR DELETE ON hypothesis_residual_risks FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- ---------------------------------------------------------------------------
-- Candidate snapshot source, temporal, and managed-feed exact authorities
-- ---------------------------------------------------------------------------

CREATE TABLE candidate_managed_feed_trust_stores (
    trust_store_version BIGINT PRIMARY KEY CHECK (trust_store_version>0),
    trust_store_hash TEXT NOT NULL UNIQUE CHECK (trust_store_hash ~ '^sha256:[0-9a-f]{64}$'),
    key_revocation_epoch BIGINT NOT NULL CHECK (key_revocation_epoch>=0),
    key_revocation_epoch_hash TEXT NOT NULL CHECK (
        key_revocation_epoch_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    installed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(trust_store_version,trust_store_hash,key_revocation_epoch,key_revocation_epoch_hash)
);

CREATE TABLE candidate_managed_feed_signer_keys (
    signer_key_member_id UUID PRIMARY KEY,
    trust_store_version BIGINT NOT NULL,
    trust_store_hash TEXT NOT NULL,
    key_revocation_epoch BIGINT NOT NULL,
    key_revocation_epoch_hash TEXT NOT NULL,
    signer_id TEXT NOT NULL CHECK (btrim(signer_id)<>''),
    signer_key_id TEXT NOT NULL CHECK (btrim(signer_key_id)<>''),
    signature_algorithm TEXT NOT NULL CHECK (
        signature_algorithm IN ('ed25519','ecdsa_p256_sha256')
    ),
    revoked BOOLEAN NOT NULL,
    key_member_hash TEXT NOT NULL CHECK (key_member_hash ~ '^sha256:[0-9a-f]{64}$'),
    installed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(trust_store_version,signer_key_id),
    UNIQUE(trust_store_version,signer_id,signer_key_id,signature_algorithm,
           revoked,key_member_hash),
    FOREIGN KEY(trust_store_version,trust_store_hash,key_revocation_epoch,
                key_revocation_epoch_hash)
        REFERENCES candidate_managed_feed_trust_stores(
            trust_store_version,trust_store_hash,key_revocation_epoch,
            key_revocation_epoch_hash
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_managed_feed_trust_store_head (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    trust_store_version BIGINT NOT NULL,
    trust_store_hash TEXT NOT NULL,
    key_revocation_epoch BIGINT NOT NULL,
    key_revocation_epoch_hash TEXT NOT NULL,
    head_version BIGINT NOT NULL DEFAULT 0 CHECK (head_version>=0),
    FOREIGN KEY(trust_store_version,trust_store_hash,key_revocation_epoch,
                key_revocation_epoch_hash)
        REFERENCES candidate_managed_feed_trust_stores(
            trust_store_version,trust_store_hash,key_revocation_epoch,
            key_revocation_epoch_hash
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_snapshot_source_sets (
    source_set_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    source_kind TEXT NOT NULL CHECK (source_kind IN (
        'tool_truth_bundle','previous_generation','state_events','relations',
        'open_obligations','expected_fact_deltas','unconsumed_fact_deltas',
        'consumed_fact_deltas','managed_knowledge_feed'
    )),
    member_count BIGINT NOT NULL CHECK (member_count>=0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_empty BOOLEAN NOT NULL,
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (sealed_empty=(member_count=0)),
    UNIQUE(snapshot_id,source_kind),
    UNIQUE(source_set_id,snapshot_id)
);

CREATE TABLE candidate_analysis_snapshot_source_set_members (
    source_member_id UUID PRIMARY KEY,
    source_set_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    source_identity TEXT NOT NULL CHECK (btrim(source_identity)<>''),
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(source_set_id,ordinal),
    UNIQUE(source_set_id,source_identity),
    FOREIGN KEY(source_set_id,snapshot_id)
        REFERENCES candidate_analysis_snapshot_source_sets(source_set_id,snapshot_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_temporal_validity_censuses (
    census_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    tool_truth_authority_bundle_seal_id UUID NOT NULL REFERENCES tool_truth_authority_bundle_seals(id) ON DELETE RESTRICT,
    temporal_validity_policy_set_hash TEXT NOT NULL CHECK (temporal_validity_policy_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    target_state_epoch_set_hash TEXT NOT NULL CHECK (target_state_epoch_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    decision_count BIGINT NOT NULL CHECK (decision_count>=0),
    decision_set_hash TEXT NOT NULL CHECK (decision_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(census_id,snapshot_id)
);

CREATE TABLE candidate_analysis_temporal_validity_census_members (
    census_member_id UUID PRIMARY KEY,
    census_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    root_family TEXT NOT NULL CHECK (root_family IN ('ti','eas','enum','vuln')),
    bundle_member_id UUID NOT NULL REFERENCES tool_truth_authority_bundle_members(id) ON DELETE RESTRICT,
    receipt_id UUID REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    temporal_census_id UUID REFERENCES capability_execution_temporal_censuses(id) ON DELETE RESTRICT,
    temporal_policy_id UUID NOT NULL REFERENCES evidence_temporal_validity_policies(id) ON DELETE RESTRICT,
    temporal_policy_hash TEXT NOT NULL CHECK (temporal_policy_hash ~ '^sha256:[0-9a-f]{64}$'),
    policy_member_id UUID REFERENCES evidence_temporal_validity_policy_members(id) ON DELETE RESTRICT,
    evidence_class TEXT NOT NULL CHECK (btrim(evidence_class)<>''),
    receipt_observed_at TIMESTAMPTZ,
    receipt_valid_until TIMESTAMPTZ,
    source_target_state_epoch BIGINT,
    current_target_state_epoch BIGINT,
    observation_window_started_at TIMESTAMPTZ,
    observation_window_completed_at TIMESTAMPTZ,
    max_cross_observation_skew_ms BIGINT NOT NULL CHECK (max_cross_observation_skew_ms>=0),
    temporal_status TEXT NOT NULL CHECK (temporal_status IN (
        'fresh','expired','mixed_epoch','skew_exceeded'
    )),
    semantic_status TEXT NOT NULL CHECK (semantic_status IN (
        'consistent','pending','orphaned','superseded'
    )),
    decision_hash TEXT NOT NULL CHECK (decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (observation_window_started_at IS NULL OR observation_window_completed_at>=observation_window_started_at),
    UNIQUE(census_id,ordinal),
    UNIQUE(census_id,bundle_member_id,receipt_id,evidence_class),
    FOREIGN KEY(census_id,snapshot_id)
        REFERENCES candidate_analysis_temporal_validity_censuses(census_id,snapshot_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_stale_evidence_residuals (
    residual_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    temporal_census_member_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_temporal_validity_census_members(census_member_id) ON DELETE RESTRICT,
    bundle_member_id UUID NOT NULL REFERENCES tool_truth_authority_bundle_members(id) ON DELETE RESTRICT,
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'authority_semantic_invalid','authority_expired','authority_mixed_epoch',
        'authority_skew_exceeded','required_root_missing'
    )),
    target_state_epoch_identity_hash TEXT NOT NULL CHECK (target_state_epoch_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    required_capability TEXT NOT NULL CHECK (btrim(required_capability)<>''),
    residual_hash TEXT NOT NULL CHECK (residual_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(snapshot_id,bundle_member_id)
);

CREATE TABLE candidate_analysis_revalidation_obligations (
    obligation_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    stale_residual_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_stale_evidence_residuals(residual_id) ON DELETE RESTRICT,
    tool_truth_revalidation_obligation_id UUID REFERENCES tool_truth_revalidation_obligations(id) ON DELETE RESTRICT,
    root_family TEXT NOT NULL CHECK (root_family IN ('ti','eas','enum','vuln')),
    evidence_identity_hash TEXT NOT NULL CHECK (evidence_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    target_state_epoch_identity_hash TEXT NOT NULL CHECK (target_state_epoch_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    required_capability TEXT NOT NULL CHECK (btrim(required_capability)<>''),
    reason_code TEXT NOT NULL CHECK (btrim(reason_code)<>''),
    obligation_hash TEXT NOT NULL CHECK (obligation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(snapshot_id,evidence_identity_hash,reason_code)
);

CREATE TABLE candidate_analysis_knowledge_feed_denominators (
    denominator_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    catalog_id UUID NOT NULL,
    catalog_version INTEGER NOT NULL CHECK (catalog_version>0),
    catalog_hash TEXT NOT NULL CHECK (catalog_hash ~ '^sha256:[0-9a-f]{64}$'),
    trust_policy_id UUID NOT NULL,
    trust_policy_version INTEGER NOT NULL CHECK (trust_policy_version>0),
    trust_policy_hash TEXT NOT NULL CHECK (trust_policy_hash ~ '^sha256:[0-9a-f]{64}$'),
    signature_algorithm_allowlist_hash TEXT NOT NULL CHECK (signature_algorithm_allowlist_hash ~ '^sha256:[0-9a-f]{64}$'),
    trust_store_version INTEGER NOT NULL CHECK (trust_store_version>0),
    trust_store_hash TEXT NOT NULL CHECK (trust_store_hash ~ '^sha256:[0-9a-f]{64}$'),
    key_revocation_epoch BIGINT NOT NULL CHECK (key_revocation_epoch>=0),
    key_revocation_epoch_hash TEXT NOT NULL CHECK (key_revocation_epoch_hash ~ '^sha256:[0-9a-f]{64}$'),
    required_source_count BIGINT NOT NULL CHECK (required_source_count=5),
    required_source_set_hash TEXT NOT NULL CHECK (required_source_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    required_member_count BIGINT NOT NULL CHECK (required_member_count>=5),
    required_member_set_hash TEXT NOT NULL CHECK (required_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    denominator_hash TEXT NOT NULL CHECK (denominator_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(denominator_id,snapshot_id)
);

CREATE TABLE candidate_analysis_knowledge_feed_denominator_members (
    expected_member_id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    source_kind TEXT NOT NULL CHECK (source_kind IN (
        'cve','cpe','kev','vendor_advisory','detection_rule'
    )),
    source_identity TEXT NOT NULL CHECK (btrim(source_identity)<>''),
    schema_name TEXT NOT NULL CHECK (btrim(schema_name)<>''),
    minimum_schema_version INTEGER NOT NULL CHECK (minimum_schema_version>0),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(denominator_id,ordinal),
    UNIQUE(denominator_id,source_kind,source_identity),
    UNIQUE(expected_member_id,denominator_id),
    FOREIGN KEY(denominator_id,snapshot_id)
        REFERENCES candidate_analysis_knowledge_feed_denominators(denominator_id,snapshot_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_knowledge_feed_snapshots (
    feed_snapshot_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    denominator_id UUID NOT NULL,
    trust_policy_hash TEXT NOT NULL CHECK (trust_policy_hash ~ '^sha256:[0-9a-f]{64}$'),
    trust_store_hash TEXT NOT NULL CHECK (trust_store_hash ~ '^sha256:[0-9a-f]{64}$'),
    key_revocation_epoch BIGINT NOT NULL CHECK (key_revocation_epoch>=0),
    member_count BIGINT NOT NULL CHECK (member_count>=5),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    feed_snapshot_hash TEXT NOT NULL CHECK (feed_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(feed_snapshot_id,snapshot_id,denominator_id),
    FOREIGN KEY(denominator_id,snapshot_id)
        REFERENCES candidate_analysis_knowledge_feed_denominators(denominator_id,snapshot_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_knowledge_feed_snapshot_members (
    feed_snapshot_member_id UUID PRIMARY KEY,
    feed_snapshot_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    denominator_id UUID NOT NULL,
    expected_member_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    feed_id TEXT,
    source_id TEXT,
    feed_schema TEXT NOT NULL CHECK (btrim(feed_schema)<>''),
    feed_version TEXT,
    published_at TIMESTAMPTZ,
    host_ingested_at TIMESTAMPTZ,
    content_hash TEXT,
    signed_manifest_hash TEXT,
    signer_id TEXT,
    signer_key_id TEXT,
    signature_algorithm TEXT,
    signature_verification_receipt_hash TEXT,
    signer_key_member_hash TEXT,
    provenance JSONB NOT NULL DEFAULT '{}'::JSONB CHECK (jsonb_typeof(provenance)='object'),
    age_policy_version TEXT NOT NULL CHECK (btrim(age_policy_version)<>''),
    age_policy_digest TEXT NOT NULL CHECK (age_policy_digest ~ '^sha256:[0-9a-f]{64}$'),
    computed_age_seconds BIGINT CHECK (computed_age_seconds IS NULL OR computed_age_seconds>=0),
    effective_valid_until TIMESTAMPTZ,
    disposition TEXT NOT NULL CHECK (disposition IN (
        'current','stale','signature_invalid','signer_revoked','unavailable'
    )),
    immutable_feed_body JSONB,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (disposition='unavailable' AND feed_id IS NULL AND content_hash IS NULL
            AND signed_manifest_hash IS NULL AND immutable_feed_body IS NULL
            AND signer_key_member_hash IS NULL AND effective_valid_until IS NULL)
        OR (disposition<>'unavailable' AND feed_id IS NOT NULL
            AND content_hash ~ '^sha256:[0-9a-f]{64}$'
            AND signed_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
            AND signature_verification_receipt_hash ~ '^sha256:[0-9a-f]{64}$'
            AND signer_key_member_hash ~ '^sha256:[0-9a-f]{64}$'
            AND effective_valid_until IS NOT NULL
            AND jsonb_typeof(immutable_feed_body)='object')
    ),
    UNIQUE(feed_snapshot_id,ordinal),
    UNIQUE(feed_snapshot_id,expected_member_id),
    UNIQUE(feed_snapshot_member_id,feed_snapshot_id),
    FOREIGN KEY(feed_snapshot_id,snapshot_id,denominator_id)
        REFERENCES candidate_analysis_knowledge_feed_snapshots(
            feed_snapshot_id,snapshot_id,denominator_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(expected_member_id,denominator_id)
        REFERENCES candidate_analysis_knowledge_feed_denominator_members(
            expected_member_id,denominator_id
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_product_version_censuses (
    product_census_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    application_model_authority_hash TEXT NOT NULL CHECK (application_model_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    product_count BIGINT NOT NULL CHECK (product_count>=0),
    product_set_hash TEXT NOT NULL CHECK (product_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(product_census_id,snapshot_id)
);

CREATE TABLE candidate_analysis_product_version_census_members (
    product_member_id UUID PRIMARY KEY,
    product_census_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    subject_kind TEXT NOT NULL CHECK (btrim(subject_kind)<>''),
    subject_identity_hash TEXT NOT NULL CHECK (subject_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    product_identity TEXT NOT NULL CHECK (btrim(product_identity)<>''),
    cpe_candidates JSONB NOT NULL CHECK (jsonb_typeof(cpe_candidates)='array'),
    observed_version TEXT,
    disposition TEXT NOT NULL CHECK (disposition IN ('known','unknown','conflicting')),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(product_census_id,ordinal),
    UNIQUE(product_census_id,subject_identity_hash,product_identity),
    UNIQUE(product_member_id,product_census_id),
    FOREIGN KEY(product_census_id,snapshot_id)
        REFERENCES candidate_analysis_product_version_censuses(product_census_id,snapshot_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_feed_match_censuses (
    match_census_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    product_census_id UUID NOT NULL REFERENCES candidate_analysis_product_version_censuses(product_census_id) ON DELETE RESTRICT,
    feed_snapshot_id UUID NOT NULL REFERENCES candidate_analysis_knowledge_feed_snapshots(feed_snapshot_id) ON DELETE RESTRICT,
    matcher_contract_version TEXT NOT NULL CHECK (btrim(matcher_contract_version)<>''),
    matcher_contract_digest TEXT NOT NULL CHECK (matcher_contract_digest ~ '^sha256:[0-9a-f]{64}$'),
    input_product_count BIGINT NOT NULL CHECK (input_product_count>=0),
    input_product_set_hash TEXT NOT NULL CHECK (input_product_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    input_feed_count BIGINT NOT NULL CHECK (input_feed_count>=0),
    input_feed_set_hash TEXT NOT NULL CHECK (input_feed_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>=0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(match_census_id,snapshot_id)
);

CREATE TABLE candidate_analysis_feed_match_census_members (
    match_member_id UUID PRIMARY KEY,
    match_census_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    product_member_id UUID NOT NULL REFERENCES candidate_analysis_product_version_census_members(product_member_id) ON DELETE RESTRICT,
    feed_snapshot_member_id UUID NOT NULL REFERENCES candidate_analysis_knowledge_feed_snapshot_members(feed_snapshot_member_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    disposition TEXT NOT NULL CHECK (disposition IN (
        'matched','no_match','unknown_product_version','feed_stale','feed_invalid'
    )),
    matched_entry_kind TEXT,
    matched_entry_id TEXT,
    matched_entry_version TEXT,
    matched_range TEXT,
    matched_entry_hash TEXT,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (disposition='matched' AND matched_entry_id IS NOT NULL
            AND matched_entry_hash ~ '^sha256:[0-9a-f]{64}$')
        OR (disposition<>'matched' AND matched_entry_id IS NULL
            AND matched_entry_hash IS NULL)
    ),
    UNIQUE(match_census_id,ordinal),
    UNIQUE(match_census_id,product_member_id,feed_snapshot_member_id,matched_entry_id),
    FOREIGN KEY(match_census_id,snapshot_id)
        REFERENCES candidate_analysis_feed_match_censuses(match_census_id,snapshot_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_enrichment_obligations (
    obligation_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    obligation_kind TEXT NOT NULL CHECK (obligation_kind IN (
        'feed_refresh','product_version_enrichment','feed_matcher_upgrade'
    )),
    product_member_id UUID REFERENCES candidate_analysis_product_version_census_members(product_member_id) ON DELETE RESTRICT,
    feed_snapshot_member_id UUID REFERENCES candidate_analysis_knowledge_feed_snapshot_members(feed_snapshot_member_id) ON DELETE RESTRICT,
    reason_code TEXT NOT NULL CHECK (btrim(reason_code)<>''),
    affected_checklist_member_key TEXT NOT NULL CHECK (btrim(affected_checklist_member_key)<>''),
    obligation_hash TEXT NOT NULL CHECK (obligation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (product_member_id IS NOT NULL OR feed_snapshot_member_id IS NOT NULL),
    UNIQUE(snapshot_id,obligation_kind,affected_checklist_member_key)
);

CREATE TRIGGER candidate_analysis_snapshot_source_sets_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_snapshot_source_sets FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_snapshot_source_set_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_snapshot_source_set_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_temporal_validity_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_temporal_validity_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_temporal_validity_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_temporal_validity_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_stale_evidence_residuals_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_stale_evidence_residuals FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_revalidation_obligations_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_revalidation_obligations FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_knowledge_feed_denominators_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_knowledge_feed_denominators FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_candidate_canonical_apply_receipts_append_only BEFORE UPDATE OR DELETE ON hypothesis_candidate_canonical_apply_receipts FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_candidate_canonical_apply_receipt_members_append_only BEFORE UPDATE OR DELETE ON hypothesis_candidate_canonical_apply_receipt_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_managed_feed_trust_stores_append_only BEFORE UPDATE OR DELETE ON candidate_managed_feed_trust_stores FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_managed_feed_signer_keys_append_only BEFORE UPDATE OR DELETE ON candidate_managed_feed_signer_keys FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_knowledge_feed_denominator_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_knowledge_feed_denominator_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_knowledge_feed_snapshots_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_knowledge_feed_snapshots FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_knowledge_feed_snapshot_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_knowledge_feed_snapshot_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_product_version_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_product_version_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_product_version_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_product_version_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_feed_match_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_feed_match_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_feed_match_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_feed_match_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_enrichment_obligations_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_enrichment_obligations FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- ---------------------------------------------------------------------------
-- Candidate immutable input delivery, H1, conflict, and coverage partitions
-- ---------------------------------------------------------------------------

CREATE TABLE candidate_analysis_snapshot_inputs (
    snapshot_input_id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    stable_input_key TEXT NOT NULL CHECK (btrim(stable_input_key)<>''),
    source_kind TEXT NOT NULL CHECK (btrim(source_kind)<>''),
    source_ref TEXT NOT NULL CHECK (btrim(source_ref)<>''),
    source_ref_hash TEXT NOT NULL CHECK (source_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_content_hash TEXT NOT NULL CHECK (source_content_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_byte_count BIGINT NOT NULL CHECK (source_byte_count>=0),
    subject_kind_at_time TEXT NOT NULL CHECK (btrim(subject_kind_at_time)<>''),
    subject_identity_hash TEXT NOT NULL CHECK (subject_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    server_chunking_disposition TEXT NOT NULL CHECK (server_chunking_disposition IN (
        'complete','source_empty','blocked_oversize','blocked_unrepresentable'
    )),
    instruction_authority BOOLEAN NOT NULL DEFAULT FALSE CHECK (NOT instruction_authority),
    input_hash TEXT NOT NULL CHECK (input_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(snapshot_id,stable_input_key),
    UNIQUE(snapshot_input_id,snapshot_id)
);

CREATE TABLE candidate_analysis_input_chunk_censuses (
    chunk_census_id UUID PRIMARY KEY,
    snapshot_input_id UUID NOT NULL UNIQUE,
    snapshot_id UUID NOT NULL,
    chunking_contract_version TEXT NOT NULL CHECK (btrim(chunking_contract_version)<>''),
    redaction_contract_version TEXT NOT NULL CHECK (btrim(redaction_contract_version)<>''),
    source_content_hash TEXT NOT NULL CHECK (source_content_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_byte_count BIGINT NOT NULL CHECK (source_byte_count>=0),
    disposition TEXT NOT NULL CHECK (disposition IN (
        'complete','source_empty','blocked_oversize','blocked_unrepresentable'
    )),
    chunk_count BIGINT NOT NULL CHECK (chunk_count>=0),
    chunk_member_set_hash TEXT NOT NULL CHECK (chunk_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (disposition='complete' AND chunk_count>0)
        OR (disposition IN ('source_empty','blocked_oversize','blocked_unrepresentable')
            AND chunk_count=0)
    ),
    UNIQUE(chunk_census_id,snapshot_input_id,snapshot_id),
    UNIQUE(chunk_census_id,snapshot_input_id,snapshot_id,census_hash,
           source_byte_count,chunking_contract_version,redaction_contract_version),
    FOREIGN KEY(snapshot_input_id,snapshot_id)
        REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id,snapshot_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_input_chunk_census_members (
    chunk_id UUID PRIMARY KEY,
    chunk_census_id UUID NOT NULL,
    snapshot_input_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    source_range_start BIGINT NOT NULL CHECK (source_range_start>=0),
    source_range_end BIGINT NOT NULL CHECK (source_range_end>source_range_start),
    envelope_schema TEXT NOT NULL CHECK (btrim(envelope_schema)<>''),
    immutable_redacted_body JSONB,
    content_blob_id UUID,
    body_or_blob_hash TEXT NOT NULL CHECK (body_or_blob_hash ~ '^sha256:[0-9a-f]{64}$'),
    chunking_contract_version TEXT NOT NULL,
    redaction_contract_version TEXT NOT NULL,
    chunk_hash TEXT NOT NULL CHECK (chunk_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((immutable_redacted_body IS NULL)<>(content_blob_id IS NULL)),
    CHECK (immutable_redacted_body IS NULL OR jsonb_typeof(immutable_redacted_body)='object'),
    UNIQUE(chunk_census_id,ordinal),
    UNIQUE(chunk_census_id,source_range_start,source_range_end),
    UNIQUE(chunk_id,chunk_census_id),
    FOREIGN KEY(chunk_census_id,snapshot_input_id,snapshot_id)
        REFERENCES candidate_analysis_input_chunk_censuses(
            chunk_census_id,snapshot_input_id,snapshot_id
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_attempt_state_events (
    attempt_event_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    event_ordinal INTEGER NOT NULL CHECK (event_ordinal>=0),
    event_kind TEXT NOT NULL CHECK (event_kind IN (
        'opened','superseded_missed_hypothesis','sealed','blocked'
    )),
    predecessor_event_id UUID,
    event_hash TEXT NOT NULL CHECK (event_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(analysis_attempt_id,event_ordinal),
    UNIQUE(attempt_event_id,analysis_attempt_id),
    FOREIGN KEY(predecessor_event_id)
        REFERENCES candidate_analysis_attempt_state_events(attempt_event_id) ON DELETE RESTRICT,
    CHECK ((event_ordinal=0)=(event_kind='opened'))
);

CREATE UNIQUE INDEX candidate_analysis_attempt_one_terminal_event
ON candidate_analysis_attempt_state_events(analysis_attempt_id)
WHERE event_kind IN ('superseded_missed_hypothesis','sealed','blocked');

CREATE TABLE candidate_analysis_page_receipts (
    page_receipt_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_id UUID NOT NULL REFERENCES candidate_analysis_snapshots(snapshot_id) ON DELETE RESTRICT,
    page_kind TEXT NOT NULL CHECK (page_kind IN ('input_page','chunk_page')),
    stable_request_id UUID NOT NULL,
    snapshot_input_id UUID REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    chunk_census_id UUID REFERENCES candidate_analysis_input_chunk_censuses(chunk_census_id) ON DELETE RESTRICT,
    chunk_census_hash TEXT CHECK (
        chunk_census_hash IS NULL OR chunk_census_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_size_bytes BIGINT CHECK (source_size_bytes IS NULL OR source_size_bytes>=0),
    chunking_contract_version TEXT,
    redaction_contract_version TEXT,
    consumer_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    server_cursor TEXT NOT NULL CHECK (btrim(server_cursor)<>''),
    first_key TEXT,
    last_key TEXT,
    returned_count BIGINT NOT NULL CHECK (returned_count>=0),
    page_hash TEXT NOT NULL CHECK (page_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((returned_count=0)=(first_key IS NULL AND last_key IS NULL)),
    CHECK (
        (page_kind='input_page' AND snapshot_input_id IS NULL
            AND chunk_census_id IS NULL AND chunk_census_hash IS NULL
            AND source_size_bytes IS NULL AND chunking_contract_version IS NULL
            AND redaction_contract_version IS NULL)
        OR
        (page_kind='chunk_page' AND snapshot_input_id IS NOT NULL
            AND chunk_census_id IS NOT NULL AND chunk_census_hash IS NOT NULL
            AND source_size_bytes IS NOT NULL AND btrim(chunking_contract_version)<>''
            AND btrim(redaction_contract_version)<>'')
    ),
    UNIQUE(analysis_attempt_id,consumer_worker_run_id,stable_request_id),
    FOREIGN KEY(snapshot_input_id,snapshot_id)
        REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id,snapshot_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(chunk_census_id,snapshot_input_id,snapshot_id,chunk_census_hash,
                source_size_bytes,chunking_contract_version,redaction_contract_version)
        REFERENCES candidate_analysis_input_chunk_censuses(
            chunk_census_id,snapshot_input_id,snapshot_id,census_hash,
            source_byte_count,chunking_contract_version,redaction_contract_version
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_work_items (
    candidate_work_item_id UUID PRIMARY KEY,
    stage_work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    phase TEXT NOT NULL CHECK (phase IN ('proposal','critic','controller')),
    capability TEXT NOT NULL CHECK (btrim(capability)<>''),
    microbatch_key TEXT,
    component_id UUID,
    page_authority_set_hash TEXT CHECK (page_authority_set_hash IS NULL OR page_authority_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    work_item_hash TEXT NOT NULL CHECK (work_item_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(candidate_work_item_id,analysis_attempt_id),
    UNIQUE(analysis_attempt_id,phase,capability,microbatch_key,component_id)
);

CREATE TABLE candidate_analysis_artifacts (
    artifact_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    candidate_work_item_id UUID NOT NULL,
    worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    stage_worker_output_id UUID UNIQUE REFERENCES stage_worker_outputs(id) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED,
    artifact_kind TEXT NOT NULL CHECK (artifact_kind IN (
        'hypothesis_proposal.v1','proposal_conflict_review.v1',
        'hypothesis_coverage_subreview.v1','hypothesis_coverage_synthesis.v1',
        'hypothesis_coverage_review.v1','controller_decision.v1'
    )),
    artifact_schema_version INTEGER NOT NULL DEFAULT 1 CHECK (artifact_schema_version=1),
    artifact_body JSONB NOT NULL CHECK (jsonb_typeof(artifact_body)='object'),
    artifact_hash TEXT NOT NULL CHECK (artifact_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(analysis_attempt_id,artifact_kind,artifact_hash),
    UNIQUE(artifact_id,analysis_attempt_id),
    FOREIGN KEY(candidate_work_item_id,analysis_attempt_id)
        REFERENCES candidate_analysis_work_items(candidate_work_item_id,analysis_attempt_id)
        ON DELETE RESTRICT
);

-- Server-owned compiler output authority. This is deliberately separate from
-- model-produced analysis artifacts: the final Controller cannot self-report
-- any of these exact-set hashes, and the Gate can load them before canonical
-- Registry rows exist.
CREATE TABLE candidate_analysis_host_compilation_seals (
    compilation_seal_id UUID PRIMARY KEY,
    stable_compilation_request_id UUID NOT NULL UNIQUE,
    analysis_attempt_id UUID NOT NULL,
    snapshot_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    final_submitter_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    mutation_set_hash TEXT NOT NULL CHECK (mutation_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    claim_component_set_hash TEXT NOT NULL CHECK (claim_component_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    verification_contract_set_hash TEXT NOT NULL CHECK (verification_contract_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    verification_plan_set_hash TEXT NOT NULL CHECK (verification_plan_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    generation_transition_set_hash TEXT NOT NULL CHECK (generation_transition_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    compiler_seal_hash TEXT NOT NULL CHECK (compiler_seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(analysis_attempt_id),
    UNIQUE(compilation_seal_id,analysis_attempt_id),
    FOREIGN KEY(analysis_attempt_id,snapshot_id,operation_id,organization_id)
        REFERENCES candidate_analysis_attempts(
            analysis_attempt_id,snapshot_id,operation_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION enforce_candidate_artifact_recorded_output()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE artifact_count BIGINT;
BEGIN
    IF NEW.business_disposition<>'artifact_recorded' THEN
        RETURN NULL;
    END IF;
    SELECT count(*) INTO artifact_count
      FROM candidate_analysis_artifacts artifact
      JOIN candidate_analysis_work_items candidate_item
        ON candidate_item.candidate_work_item_id=artifact.candidate_work_item_id
     WHERE artifact.stage_worker_output_id=NEW.id
       AND artifact.worker_run_id=NEW.worker_run_id
       AND candidate_item.stage_work_item_id=NEW.work_item_id;
    IF artifact_count<>1 THEN
        RAISE EXCEPTION 'CANDIDATE_ANALYSIS_ARTIFACT_RECORDED_EXACT_ARTIFACT_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER stage_worker_output_candidate_artifact_required
AFTER INSERT ON stage_worker_outputs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_artifact_recorded_output();

CREATE TABLE hypothesis_proposals (
    proposal_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    artifact_id UUID NOT NULL,
    proposal_ordinal INTEGER NOT NULL CHECK (proposal_ordinal>=0),
    structured_proposal JSONB NOT NULL CHECK (jsonb_typeof(structured_proposal)='object'),
    proposal_hash TEXT NOT NULL CHECK (proposal_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(analysis_attempt_id,proposal_ordinal),
    UNIQUE(analysis_attempt_id,proposal_hash),
    UNIQUE(proposal_id,analysis_attempt_id),
    FOREIGN KEY(artifact_id,analysis_attempt_id)
        REFERENCES candidate_analysis_artifacts(artifact_id,analysis_attempt_id)
        ON DELETE RESTRICT
);

CREATE TABLE hypothesis_proposal_refs (
    proposal_ref_id UUID PRIMARY KEY,
    proposal_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    chunk_id UUID REFERENCES candidate_analysis_input_chunk_census_members(chunk_id) ON DELETE RESTRICT,
    source_role TEXT NOT NULL CHECK (source_role IN (
        'support','contradiction','application_context','knowledge_signal','gap'
    )),
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    ref_hash TEXT NOT NULL CHECK (ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(proposal_id,snapshot_input_id,chunk_id,source_role,source_hash),
    FOREIGN KEY(proposal_id,analysis_attempt_id)
        REFERENCES hypothesis_proposals(proposal_id,analysis_attempt_id) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_proposal_censuses (
    proposal_census_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    proposal_count BIGINT NOT NULL CHECK (proposal_count>=0),
    proposal_set_hash TEXT NOT NULL CHECK (proposal_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(proposal_census_id,analysis_attempt_id)
);

CREATE TABLE candidate_analysis_proposal_census_members (
    census_member_id UUID PRIMARY KEY,
    proposal_census_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    proposal_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    proposal_hash TEXT NOT NULL CHECK (proposal_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(proposal_census_id,ordinal),
    UNIQUE(proposal_census_id,proposal_id),
    FOREIGN KEY(proposal_census_id,analysis_attempt_id)
        REFERENCES candidate_analysis_proposal_censuses(proposal_census_id,analysis_attempt_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(proposal_id,analysis_attempt_id)
        REFERENCES hypothesis_proposals(proposal_id,analysis_attempt_id) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_input_proposal_dispositions (
    disposition_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    proposal_ref_count BIGINT NOT NULL CHECK (proposal_ref_count>=0),
    proposal_ref_set_hash TEXT NOT NULL CHECK (proposal_ref_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT NOT NULL CHECK (disposition IN ('has_proposal','zero_proposal','blocked')),
    blocker_code TEXT,
    disposition_hash TEXT NOT NULL CHECK (disposition_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((disposition='has_proposal')=(proposal_ref_count>0)),
    CHECK ((disposition='blocked')=(blocker_code IS NOT NULL)),
    UNIQUE(analysis_attempt_id,snapshot_input_id)
);

CREATE TABLE candidate_analysis_conflict_components (
    conflict_component_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    proposal_count BIGINT NOT NULL CHECK (proposal_count>0),
    proposal_set_hash TEXT NOT NULL CHECK (proposal_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    component_hash TEXT NOT NULL CHECK (component_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(analysis_attempt_id,ordinal),
    UNIQUE(conflict_component_id,analysis_attempt_id)
);

CREATE TABLE candidate_analysis_conflict_component_members (
    conflict_member_id UUID PRIMARY KEY,
    conflict_component_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    proposal_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(conflict_component_id,ordinal),
    UNIQUE(analysis_attempt_id,proposal_id),
    FOREIGN KEY(conflict_component_id,analysis_attempt_id)
        REFERENCES candidate_analysis_conflict_components(
            conflict_component_id,analysis_attempt_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(proposal_id,analysis_attempt_id)
        REFERENCES hypothesis_proposals(proposal_id,analysis_attempt_id) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_hypothesis_coverage_checklist_members (
    checklist_member_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    attack_class_contract_version TEXT NOT NULL CHECK (btrim(attack_class_contract_version)<>''),
    attack_class_contract_digest TEXT NOT NULL CHECK (attack_class_contract_digest ~ '^sha256:[0-9a-f]{64}$'),
    trust_boundary_contract_version TEXT NOT NULL CHECK (btrim(trust_boundary_contract_version)<>''),
    trust_boundary_contract_digest TEXT NOT NULL CHECK (trust_boundary_contract_digest ~ '^sha256:[0-9a-f]{64}$'),
    attack_class_id TEXT NOT NULL CHECK (btrim(attack_class_id)<>''),
    attack_class_version INTEGER NOT NULL CHECK (attack_class_version>0),
    trust_boundary_identity TEXT NOT NULL CHECK (btrim(trust_boundary_identity)<>''),
    trust_boundary_hash TEXT NOT NULL CHECK (trust_boundary_hash ~ '^sha256:[0-9a-f]{64}$'),
    applicability_basis JSONB NOT NULL CHECK (jsonb_typeof(applicability_basis)='object'),
    feed_match_member_refs UUID[] NOT NULL DEFAULT ARRAY[]::UUID[],
    applicability_disposition TEXT NOT NULL CHECK (applicability_disposition IN (
        'required','not_applicable','blocked_feed_authority','blocked_product_version'
    )),
    enrichment_obligation_id UUID REFERENCES candidate_analysis_enrichment_obligations(obligation_id) ON DELETE RESTRICT,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((applicability_disposition LIKE 'blocked_%')=(enrichment_obligation_id IS NOT NULL)),
    UNIQUE(analysis_attempt_id,snapshot_input_id,ordinal),
    UNIQUE(analysis_attempt_id,snapshot_input_id,attack_class_id,attack_class_version,trust_boundary_hash),
    UNIQUE(checklist_member_id,analysis_attempt_id,snapshot_input_id)
);

CREATE TABLE candidate_analysis_hypothesis_coverage_chunk_partitions (
    chunk_partition_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    partition_ordinal INTEGER NOT NULL CHECK (partition_ordinal>=0),
    first_chunk_ordinal INTEGER NOT NULL CHECK (first_chunk_ordinal>=0),
    last_chunk_ordinal INTEGER NOT NULL CHECK (last_chunk_ordinal>=first_chunk_ordinal),
    chunk_count BIGINT NOT NULL CHECK (chunk_count>0),
    chunk_set_hash TEXT NOT NULL CHECK (chunk_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    bounded_context_budget BIGINT NOT NULL CHECK (bounded_context_budget>0),
    partition_hash TEXT NOT NULL CHECK (partition_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (chunk_count=last_chunk_ordinal-first_chunk_ordinal+1),
    UNIQUE(analysis_attempt_id,snapshot_input_id,partition_ordinal),
    UNIQUE(chunk_partition_id,analysis_attempt_id,snapshot_input_id)
);

CREATE TRIGGER candidate_analysis_snapshot_inputs_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_snapshot_inputs FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_input_chunk_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_input_chunk_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_input_chunk_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_input_chunk_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_attempt_state_events_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_attempt_state_events FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_page_receipts_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_page_receipts FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_work_items_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_work_items FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_artifacts_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_artifacts FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_host_compilation_seals_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_host_compilation_seals FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_proposals_append_only BEFORE UPDATE OR DELETE ON hypothesis_proposals FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_proposal_refs_append_only BEFORE UPDATE OR DELETE ON hypothesis_proposal_refs FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_proposal_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_proposal_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_proposal_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_proposal_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_input_proposal_dispositions_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_input_proposal_dispositions FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_conflict_components_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_conflict_components FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_conflict_component_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_conflict_component_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_checklist_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_checklist_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_chunk_partitions_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_chunk_partitions FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- ---------------------------------------------------------------------------
-- Candidate H2 map/reduce coverage authority and host-reduced reviews
-- ---------------------------------------------------------------------------

CREATE TABLE candidate_analysis_hypothesis_coverage_subreview_censuses (
    subreview_census_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    checklist_member_count BIGINT NOT NULL CHECK (checklist_member_count>0),
    checklist_member_set_hash TEXT NOT NULL CHECK (checklist_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    chunk_partition_count BIGINT NOT NULL CHECK (chunk_partition_count>0),
    chunk_partition_set_hash TEXT NOT NULL CHECK (chunk_partition_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_member_count BIGINT NOT NULL CHECK (expected_member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (expected_member_count=checklist_member_count*chunk_partition_count),
    UNIQUE(analysis_attempt_id,snapshot_input_id),
    UNIQUE(subreview_census_id,analysis_attempt_id,snapshot_input_id)
);

CREATE TABLE candidate_analysis_hypothesis_coverage_subreview_census_members (
    subreview_census_member_id UUID PRIMARY KEY,
    subreview_census_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    snapshot_input_id UUID NOT NULL,
    checklist_member_id UUID NOT NULL,
    chunk_partition_id UUID NOT NULL,
    checklist_ordinal INTEGER NOT NULL CHECK (checklist_ordinal>=0),
    partition_ordinal INTEGER NOT NULL CHECK (partition_ordinal>=0),
    designated_stage_work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN ('required','sampling_omitted')),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(subreview_census_id,checklist_member_id,chunk_partition_id),
    UNIQUE(subreview_census_member_id,subreview_census_id,analysis_attempt_id,snapshot_input_id),
    FOREIGN KEY(subreview_census_id,analysis_attempt_id,snapshot_input_id)
        REFERENCES candidate_analysis_hypothesis_coverage_subreview_censuses(
            subreview_census_id,analysis_attempt_id,snapshot_input_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(checklist_member_id,analysis_attempt_id,snapshot_input_id)
        REFERENCES candidate_analysis_hypothesis_coverage_checklist_members(
            checklist_member_id,analysis_attempt_id,snapshot_input_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(chunk_partition_id,analysis_attempt_id,snapshot_input_id)
        REFERENCES candidate_analysis_hypothesis_coverage_chunk_partitions(
            chunk_partition_id,analysis_attempt_id,snapshot_input_id
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_hypothesis_coverage_subreviews (
    subreview_id UUID PRIMARY KEY,
    subreview_census_member_id UUID NOT NULL UNIQUE,
    subreview_census_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    snapshot_input_id UUID NOT NULL,
    designated_chunk_count BIGINT NOT NULL CHECK (designated_chunk_count>0),
    designated_chunk_set_hash TEXT NOT NULL CHECK (designated_chunk_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    read_receipt_count BIGINT NOT NULL CHECK (read_receipt_count>0),
    read_receipt_set_hash TEXT NOT NULL CHECK (read_receipt_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    h1_proposal_ref_count BIGINT NOT NULL CHECK (h1_proposal_ref_count>=0),
    h1_proposal_ref_set_hash TEXT NOT NULL CHECK (h1_proposal_ref_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    primary_analyst_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    map_critic_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    context_budget BIGINT NOT NULL CHECK (context_budget>0),
    context_truncated BOOLEAN NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('no_local_miss','missed_hypothesis','blocked')),
    typed_missed_refs JSONB NOT NULL DEFAULT '[]'::JSONB CHECK (jsonb_typeof(typed_missed_refs)='array'),
    blocker_codes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    subreview_hash TEXT NOT NULL CHECK (subreview_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (primary_analyst_worker_run_id<>map_critic_worker_run_id),
    CHECK (NOT context_truncated OR outcome='blocked'),
    CHECK ((outcome='missed_hypothesis')=(jsonb_array_length(typed_missed_refs)>0)),
    FOREIGN KEY(subreview_census_member_id,subreview_census_id,analysis_attempt_id,snapshot_input_id)
        REFERENCES candidate_analysis_hypothesis_coverage_subreview_census_members(
            subreview_census_member_id,subreview_census_id,analysis_attempt_id,snapshot_input_id
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_hypothesis_coverage_synthesis_censuses (
    synthesis_census_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    relationship_cross_index_hash TEXT NOT NULL CHECK (relationship_cross_index_hash ~ '^sha256:[0-9a-f]{64}$'),
    fan_in_limit INTEGER NOT NULL CHECK (fan_in_limit BETWEEN 2 AND 64),
    node_count BIGINT NOT NULL CHECK (node_count>0),
    node_set_hash TEXT NOT NULL CHECK (node_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    dimension_root_count BIGINT NOT NULL CHECK (dimension_root_count>0),
    dimension_root_set_hash TEXT NOT NULL CHECK (dimension_root_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    global_root_node_id UUID NOT NULL,
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(synthesis_census_id,analysis_attempt_id)
);

CREATE TABLE candidate_analysis_hypothesis_coverage_synthesis_census_members (
    synthesis_node_id UUID PRIMARY KEY,
    synthesis_census_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    node_kind TEXT NOT NULL CHECK (node_kind IN (
        'cross_chunk','cross_input_partition','cross_input_reduce',
        'cross_dimension_reduce','global_semantic_root'
    )),
    level INTEGER NOT NULL CHECK (level>=0),
    partition_ordinal INTEGER NOT NULL CHECK (partition_ordinal>=0),
    attack_class_id TEXT,
    trust_boundary_hash TEXT,
    covered_input_count BIGINT NOT NULL CHECK (covered_input_count>0),
    covered_input_set_hash TEXT NOT NULL CHECK (covered_input_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    covered_checklist_count BIGINT NOT NULL CHECK (covered_checklist_count>0),
    covered_checklist_set_hash TEXT NOT NULL CHECK (covered_checklist_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    child_receipt_count BIGINT NOT NULL CHECK (child_receipt_count>0),
    child_receipt_set_hash TEXT NOT NULL CHECK (child_receipt_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    relationship_cross_index_hash TEXT NOT NULL CHECK (relationship_cross_index_hash ~ '^sha256:[0-9a-f]{64}$'),
    descendant_worker_count BIGINT NOT NULL CHECK (descendant_worker_count>0),
    descendant_worker_set_hash TEXT NOT NULL CHECK (descendant_worker_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    node_hash TEXT NOT NULL CHECK (node_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(synthesis_census_id,node_kind,level,partition_ordinal,attack_class_id,trust_boundary_hash),
    UNIQUE(synthesis_node_id,synthesis_census_id,analysis_attempt_id),
    FOREIGN KEY(synthesis_census_id,analysis_attempt_id)
        REFERENCES candidate_analysis_hypothesis_coverage_synthesis_censuses(
            synthesis_census_id,analysis_attempt_id
        ) ON DELETE RESTRICT
);

ALTER TABLE candidate_analysis_hypothesis_coverage_synthesis_censuses
    ADD CONSTRAINT candidate_synthesis_global_root_fk
    FOREIGN KEY(global_root_node_id,synthesis_census_id,analysis_attempt_id)
    REFERENCES candidate_analysis_hypothesis_coverage_synthesis_census_members(
        synthesis_node_id,synthesis_census_id,analysis_attempt_id
    ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE candidate_analysis_hypothesis_coverage_synthesis_reviews (
    synthesis_review_id UUID PRIMARY KEY,
    synthesis_node_id UUID NOT NULL UNIQUE,
    synthesis_census_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    synthesis_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    transitive_descendant_worker_set_hash TEXT NOT NULL CHECK (transitive_descendant_worker_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    worker_separation_valid BOOLEAN NOT NULL,
    context_truncated BOOLEAN NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('no_composite_miss','missed_hypothesis','blocked')),
    typed_missed_refs JSONB NOT NULL DEFAULT '[]'::JSONB CHECK (jsonb_typeof(typed_missed_refs)='array'),
    review_hash TEXT NOT NULL CHECK (review_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (worker_separation_valid OR outcome='blocked'),
    CHECK (NOT context_truncated OR outcome='blocked'),
    FOREIGN KEY(synthesis_node_id,synthesis_census_id,analysis_attempt_id)
        REFERENCES candidate_analysis_hypothesis_coverage_synthesis_census_members(
            synthesis_node_id,synthesis_census_id,analysis_attempt_id
        ) ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_hypothesis_coverage_global_reviews (
    global_review_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    synthesis_census_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_hypothesis_coverage_synthesis_censuses(synthesis_census_id) ON DELETE RESTRICT,
    global_root_review_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_hypothesis_coverage_synthesis_reviews(synthesis_review_id) ON DELETE RESTRICT,
    dimension_root_count BIGINT NOT NULL CHECK (dimension_root_count>0),
    dimension_root_set_hash TEXT NOT NULL CHECK (dimension_root_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    relationship_cross_index_hash TEXT NOT NULL CHECK (relationship_cross_index_hash ~ '^sha256:[0-9a-f]{64}$'),
    worker_separation_set_hash TEXT NOT NULL CHECK (worker_separation_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    outcome TEXT NOT NULL CHECK (outcome IN ('adequate','missed_hypothesis','blocked')),
    review_hash TEXT NOT NULL CHECK (review_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE candidate_analysis_critic_censuses (
    critic_census_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(critic_census_id,analysis_attempt_id)
);

CREATE TABLE candidate_analysis_critic_census_members (
    critic_member_id UUID PRIMARY KEY,
    critic_census_id UUID NOT NULL,
    analysis_attempt_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    member_kind TEXT NOT NULL CHECK (member_kind IN (
        'proposal_conflict_component','hypothesis_coverage_subreview',
        'hypothesis_coverage_synthesis','hypothesis_coverage_input_review',
        'hypothesis_coverage_global_review'
    )),
    source_identity UUID NOT NULL,
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(critic_census_id,ordinal),
    UNIQUE(critic_census_id,member_kind,source_identity),
    FOREIGN KEY(critic_census_id,analysis_attempt_id)
        REFERENCES candidate_analysis_critic_censuses(critic_census_id,analysis_attempt_id)
        ON DELETE RESTRICT
);

CREATE TABLE candidate_analysis_hypothesis_coverage_reviews (
    coverage_review_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal>=0),
    chunk_census_id UUID NOT NULL REFERENCES candidate_analysis_input_chunk_censuses(chunk_census_id) ON DELETE RESTRICT,
    chunk_partition_count BIGINT NOT NULL CHECK (chunk_partition_count>0),
    chunk_partition_set_hash TEXT NOT NULL CHECK (chunk_partition_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    subreview_census_id UUID NOT NULL REFERENCES candidate_analysis_hypothesis_coverage_subreview_censuses(subreview_census_id) ON DELETE RESTRICT,
    read_receipt_set_hash TEXT NOT NULL CHECK (read_receipt_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    h1_proposal_ref_count BIGINT NOT NULL CHECK (h1_proposal_ref_count>=0),
    h1_proposal_ref_set_hash TEXT NOT NULL CHECK (h1_proposal_ref_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    attack_class_checklist_version TEXT NOT NULL,
    attack_class_checklist_digest TEXT NOT NULL CHECK (attack_class_checklist_digest ~ '^sha256:[0-9a-f]{64}$'),
    trust_boundary_checklist_version TEXT NOT NULL,
    trust_boundary_checklist_digest TEXT NOT NULL CHECK (trust_boundary_checklist_digest ~ '^sha256:[0-9a-f]{64}$'),
    checklist_member_set_hash TEXT NOT NULL CHECK (checklist_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    synthesis_census_id UUID NOT NULL REFERENCES candidate_analysis_hypothesis_coverage_synthesis_censuses(synthesis_census_id) ON DELETE RESTRICT,
    global_review_id UUID NOT NULL REFERENCES candidate_analysis_hypothesis_coverage_global_reviews(global_review_id) ON DELETE RESTRICT,
    coverage_sampling_contract_version TEXT NOT NULL,
    coverage_sampling_contract_digest TEXT NOT NULL CHECK (coverage_sampling_contract_digest ~ '^sha256:[0-9a-f]{64}$'),
    worker_separation_set_hash TEXT NOT NULL CHECK (worker_separation_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    review_mode TEXT NOT NULL CHECK (review_mode IN ('full','deterministic_sample')),
    outcome TEXT NOT NULL CHECK (outcome IN ('adequate','missed_hypothesis','blocked')),
    checklist_dispositions JSONB NOT NULL CHECK (jsonb_typeof(checklist_dispositions)='array'),
    typed_missed_refs JSONB NOT NULL DEFAULT '[]'::JSONB CHECK (jsonb_typeof(typed_missed_refs)='array'),
    review_hash TEXT NOT NULL CHECK (review_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (review_mode='full' OR outcome='blocked'),
    UNIQUE(analysis_attempt_id,snapshot_input_id)
);

CREATE TABLE hypothesis_proposal_relations (
    proposal_relation_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    source_proposal_id UUID NOT NULL REFERENCES hypothesis_proposals(proposal_id) ON DELETE RESTRICT,
    target_proposal_id UUID NOT NULL REFERENCES hypothesis_proposals(proposal_id) ON DELETE RESTRICT,
    relation_kind TEXT NOT NULL CHECK (relation_kind IN (
        'support','contradict','refine','duplicate','merge_candidate'
    )),
    relation_hash TEXT NOT NULL CHECK (relation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (source_proposal_id<>target_proposal_id),
    UNIQUE(analysis_attempt_id,source_proposal_id,target_proposal_id,relation_kind)
);

CREATE TABLE hypothesis_merge_decisions (
    merge_decision_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    conflict_component_id UUID NOT NULL UNIQUE REFERENCES candidate_analysis_conflict_components(conflict_component_id) ON DELETE RESTRICT,
    decision_kind TEXT NOT NULL CHECK (decision_kind IN (
        'keep_distinct','merge','duplicate','split_required','blocked'
    )),
    source_proposal_set_hash TEXT NOT NULL CHECK (source_proposal_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    canonical_decision JSONB NOT NULL CHECK (jsonb_typeof(canonical_decision)='object'),
    decision_hash TEXT NOT NULL CHECK (decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE input_processing_dispositions (
    input_disposition_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN (
        'analyzed','informational','duplicate_input','not_security_relevant','gap','blocked'
    )),
    reason_code TEXT,
    disposition_hash TEXT NOT NULL CHECK (disposition_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(analysis_attempt_id,snapshot_input_id)
);

CREATE TABLE input_hypothesis_relations (
    input_hypothesis_relation_id UUID PRIMARY KEY,
    analysis_attempt_id UUID NOT NULL REFERENCES candidate_analysis_attempts(analysis_attempt_id) ON DELETE RESTRICT,
    snapshot_input_id UUID NOT NULL REFERENCES candidate_analysis_snapshot_inputs(snapshot_input_id) ON DELETE RESTRICT,
    revision_id UUID NOT NULL REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    relation_kind TEXT NOT NULL CHECK (relation_kind IN (
        'creates_hypothesis','supports_existing','contradicts_existing','qualifies_existing'
    )),
    relation_hash TEXT NOT NULL CHECK (relation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(analysis_attempt_id,snapshot_input_id,revision_id,relation_kind)
);

CREATE TRIGGER candidate_analysis_hypothesis_coverage_subreview_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_subreview_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_subreview_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_subreview_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_subreviews_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_subreviews FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_synthesis_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_synthesis_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_synthesis_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_synthesis_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_synthesis_reviews_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_synthesis_reviews FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_global_reviews_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_global_reviews FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_critic_censuses_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_critic_censuses FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_critic_census_members_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_critic_census_members FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER candidate_analysis_hypothesis_coverage_reviews_append_only BEFORE UPDATE OR DELETE ON candidate_analysis_hypothesis_coverage_reviews FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_proposal_relations_append_only BEFORE UPDATE OR DELETE ON hypothesis_proposal_relations FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_merge_decisions_append_only BEFORE UPDATE OR DELETE ON hypothesis_merge_decisions FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER input_processing_dispositions_append_only BEFORE UPDATE OR DELETE ON input_processing_dispositions FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER input_hypothesis_relations_append_only BEFORE UPDATE OR DELETE ON input_hypothesis_relations FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- ---------------------------------------------------------------------------
-- Closed projection catalog, frozen outbox source, and compatibility views
-- ---------------------------------------------------------------------------

CREATE FUNCTION projection_timeline_mapping_is_valid(
    p_entity_kind TEXT,
    p_change_kind TEXT,
    p_timeline_event_kind TEXT
) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT
AS $$
    SELECT EXISTS (
        SELECT 1 FROM (VALUES
            ('generation','insert','generation_sealed'),
            ('hypothesis','insert','hypothesis_inserted'),
            ('hypothesis','supersede','hypothesis_superseded'),
            ('hypothesis','close','hypothesis_closed'),
            ('hypothesis','invalidate','hypothesis_invalidated'),
            ('hypothesis_verification_plan','close','hypothesis_verification_plan_sealed'),
            ('hypothesis_verification_objective_outcome','close','hypothesis_verification_objective_outcome_closed'),
            ('hypothesis_verification_objective_outcome','invalidate','hypothesis_verification_objective_outcome_invalidated'),
            ('hypothesis_revision_adjudication','close','hypothesis_revision_adjudication_closed'),
            ('hypothesis_revision_adjudication','invalidate','hypothesis_revision_adjudication_invalidated'),
            ('hypothesis_revision_terminal_decision','close','hypothesis_revision_terminal_decision_closed'),
            ('hypothesis_revision_terminal_decision','invalidate','hypothesis_revision_terminal_decision_invalidated'),
            ('hypothesis_state_event','insert','hypothesis_state_event_inserted'),
            ('hypothesis_state_event','invalidate','hypothesis_state_event_invalidated'),
            ('finding','insert','finding_inserted'),
            ('finding','invalidate','finding_invalidated'),
            ('refutation','insert','refutation_inserted'),
            ('refutation','invalidate','refutation_invalidated'),
            ('relation','insert','relation_inserted'),
            ('relation','invalidate','relation_invalidated'),
            ('residual','insert','residual_inserted'),
            ('residual','close','residual_closed'),
            ('residual','invalidate','residual_invalidated'),
            ('capability_assessment','insert','capability_assessment_inserted'),
            ('capability_assessment','invalidate','capability_assessment_invalidated'),
            ('capability_assessment_set','close','capability_assessment_set_sealed'),
            ('legacy_candidate_projection','insert','legacy_candidate_projection_materialized'),
            ('legacy_candidate_projection','invalidate','legacy_candidate_projection_invalidated'),
            ('legacy_attempt_projection','insert','legacy_attempt_projection_materialized'),
            ('legacy_attempt_projection','invalidate','legacy_attempt_projection_invalidated'),
            ('shadow_comparison','compare','shadow_comparison_recorded'),
            ('campaign','insert','campaign_inserted'),
            ('campaign','supersede','campaign_superseded'),
            ('campaign','close','campaign_closed'),
            ('campaign_round','insert','campaign_round_inserted'),
            ('campaign_round','close','campaign_round_closed'),
            ('consult','insert','consult_inserted'),
            ('consult','close','consult_closed'),
            ('strategy','insert','strategy_inserted'),
            ('strategy_obligation','insert','strategy_obligation_inserted'),
            ('prepared_action','insert','prepared_action_inserted'),
            ('prepared_action','supersede','prepared_action_superseded'),
            ('authorization','insert','authorization_inserted'),
            ('action_execution','insert','action_execution_inserted'),
            ('action_execution','close','action_execution_closed'),
            ('conflict_lease','insert','conflict_lease_acquired'),
            ('conflict_lease','supersede','conflict_lease_recovery_held'),
            ('conflict_lease','close','conflict_lease_released'),
            ('budget_ledger_entry','insert','budget_ledger_entry_recorded'),
            ('cleanup_obligation','insert','cleanup_obligation_inserted'),
            ('cleanup_obligation','close','cleanup_obligation_closed'),
            ('callback_obligation','insert','callback_obligation_inserted'),
            ('callback_obligation','close','callback_obligation_closed'),
            ('oracle','insert','oracle_inserted'),
            ('oracle','invalidate','oracle_invalidated'),
            ('oracle_census','close','oracle_census_sealed'),
            ('adjudication','insert','adjudication_inserted'),
            ('campaign_terminal','close','campaign_terminal_closed'),
            ('campaign_terminal','invalidate','campaign_terminal_invalidated'),
            ('fact_delta','insert','fact_delta_inserted'),
            ('fact_delta','invalidate','fact_delta_invalidated'),
            ('fact_delta_consumption','insert','fact_delta_consumed'),
            ('fact_delta_consumption','close','fact_delta_consumption_closed'),
            ('hypothesis_evolution_proposal','insert','hypothesis_evolution_proposed'),
            ('hypothesis_evolution_decision','insert','hypothesis_evolution_decided'),
            ('consolidation','close','consolidation_closed'),
            ('fixed_point','close','fixed_point_closed'),
            ('enrichment_obligation','insert','enrichment_obligation_inserted'),
            ('enrichment_obligation','close','enrichment_obligation_closed'),
            ('application_fact_refinement_obligation','insert','application_fact_refinement_obligation_inserted'),
            ('application_fact_refinement_obligation','close','application_fact_refinement_obligation_closed'),
            ('coverage','insert','coverage_denominator_sealed'),
            ('coverage','supersede','coverage_result_recorded'),
            ('coverage','close','coverage_closed'),
            ('coverage','invalidate','coverage_invalidated'),
            ('report','insert','report_inserted'),
            ('report','close','report_closed'),
            ('report','supersede','report_superseded')
        ) AS allowed(entity_kind,change_kind,timeline_event_kind)
        WHERE allowed.entity_kind=p_entity_kind
          AND allowed.change_kind=p_change_kind
          AND allowed.timeline_event_kind=p_timeline_event_kind
    )
$$;

ALTER TABLE investigation_projection_entity_versions
    ADD CONSTRAINT investigation_projection_entity_kind_check CHECK (entity_kind IN (
        'generation','hypothesis','hypothesis_verification_plan',
        'hypothesis_verification_objective_outcome','hypothesis_revision_adjudication',
        'hypothesis_revision_terminal_decision','hypothesis_state_event','finding','refutation',
        'relation','residual','capability_assessment','capability_assessment_set',
        'legacy_candidate_projection','legacy_attempt_projection','shadow_comparison',
        'campaign','campaign_round','consult','strategy','strategy_obligation',
        'prepared_action','authorization','action_execution','conflict_lease',
        'budget_ledger_entry','cleanup_obligation','callback_obligation','oracle',
        'oracle_census','adjudication','campaign_terminal','fact_delta',
        'fact_delta_consumption','hypothesis_evolution_proposal',
        'hypothesis_evolution_decision','consolidation','fixed_point',
        'enrichment_obligation','application_fact_refinement_obligation','coverage','report'
    )),
    ADD CONSTRAINT investigation_projection_entity_invalidation_reason_check CHECK (
        invalidation_reason IS NULL OR invalidation_reason IN (
            'source_superseded','source_quarantined','authority_stale','source_deleted',
            'legacy_projection_unsupported','legacy_projection_derivation_failed',
            'legacy_projection_diverged','contract_unsupported'
        )
    );

CREATE TABLE investigation_projection_source_blobs (
    blob_id UUID PRIMARY KEY,
    payload_schema TEXT NOT NULL DEFAULT 'projection_source_snapshot.v1'
        CHECK (payload_schema='projection_source_snapshot.v1'),
    payload_schema_version INTEGER NOT NULL DEFAULT 1 CHECK (payload_schema_version=1),
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^sha256:[0-9a-f]{64}$'),
    byte_count BIGINT NOT NULL CHECK (byte_count BETWEEN 1 AND 16777216),
    immutable_redacted_bytes BYTEA NOT NULL,
    redaction_contract_version TEXT NOT NULL CHECK (btrim(redaction_contract_version)<>''),
    redaction_metadata JSONB NOT NULL CHECK (jsonb_typeof(redaction_metadata)='object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (octet_length(immutable_redacted_bytes)=byte_count),
    UNIQUE(payload_schema,payload_schema_version,content_hash),
    UNIQUE(blob_id,content_hash)
);

CREATE TABLE investigation_projection_outbox (
    outbox_member_id UUID PRIMARY KEY,
    batch_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    source_batch_seq BIGINT NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    entity_kind TEXT NOT NULL CHECK (entity_kind IN (
        'generation','hypothesis','hypothesis_verification_plan',
        'hypothesis_verification_objective_outcome','hypothesis_revision_adjudication',
        'hypothesis_revision_terminal_decision','hypothesis_state_event','finding','refutation',
        'relation','residual','capability_assessment','capability_assessment_set',
        'legacy_candidate_projection','legacy_attempt_projection','shadow_comparison',
        'campaign','campaign_round','consult','strategy','strategy_obligation',
        'prepared_action','authorization','action_execution','conflict_lease',
        'budget_ledger_entry','cleanup_obligation','callback_obligation','oracle',
        'oracle_census','adjudication','campaign_terminal','fact_delta',
        'fact_delta_consumption','hypothesis_evolution_proposal',
        'hypothesis_evolution_decision','consolidation','fixed_point',
        'enrichment_obligation','application_fact_refinement_obligation','coverage','report'
    )),
    change_kind TEXT NOT NULL CHECK (change_kind IN (
        'insert','supersede','close','compare','invalidate'
    )),
    source_entity_id UUID NOT NULL,
    source_entity_version BIGINT NOT NULL CHECK (source_entity_version>0),
    source_entity_hash TEXT NOT NULL CHECK (source_entity_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_occurred_at TIMESTAMPTZ,
    source_time_status TEXT NOT NULL CHECK (source_time_status IN ('known','historical_unknown')),
    source_snapshot_schema TEXT NOT NULL DEFAULT 'projection_source_snapshot.v1'
        CHECK (source_snapshot_schema='projection_source_snapshot.v1'),
    source_snapshot_version INTEGER NOT NULL DEFAULT 1 CHECK (source_snapshot_version=1),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    immutable_source_body JSONB,
    source_blob_id UUID,
    source_blob_hash TEXT,
    timeline_event_kind TEXT NOT NULL,
    invalidation_reason TEXT,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((source_time_status='known')=(source_occurred_at IS NOT NULL)),
    CHECK ((immutable_source_body IS NULL)<>(source_blob_id IS NULL)),
    CHECK (immutable_source_body IS NULL OR jsonb_typeof(immutable_source_body)='object'),
    CHECK ((source_blob_id IS NULL)=(source_blob_hash IS NULL)),
    CHECK (source_blob_hash IS NULL OR source_blob_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK ((change_kind='invalidate')=(invalidation_reason IS NOT NULL)),
    CHECK (invalidation_reason IS NULL OR invalidation_reason IN (
        'source_superseded','source_quarantined','authority_stale','source_deleted',
        'legacy_projection_unsupported','legacy_projection_derivation_failed',
        'legacy_projection_diverged','contract_unsupported'
    )),
    CHECK (projection_timeline_mapping_is_valid(entity_kind,change_kind,timeline_event_kind)),
    UNIQUE(batch_id,member_ordinal),
    UNIQUE(batch_id,entity_kind,source_entity_id,source_entity_version,change_kind),
    UNIQUE(outbox_member_id,batch_id,operation_id,source_batch_seq),
    FOREIGN KEY(batch_id,operation_id,source_batch_seq)
        REFERENCES investigation_projection_outbox_batches(
            batch_id,operation_id,source_batch_seq
        ) ON DELETE RESTRICT,
    FOREIGN KEY(source_blob_id,source_blob_hash)
        REFERENCES investigation_projection_source_blobs(blob_id,content_hash)
        ON DELETE RESTRICT
);

CREATE TABLE investigation_projection_changes (
    operation_id UUID NOT NULL,
    change_seq BIGINT NOT NULL CHECK (change_seq>0),
    event_id UUID NOT NULL UNIQUE,
    batch_id UUID NOT NULL,
    source_batch_seq BIGINT NOT NULL CHECK (source_batch_seq>0),
    outbox_member_id UUID NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id UUID NOT NULL,
    entity_version BIGINT NOT NULL CHECK (entity_version>0),
    change_kind TEXT NOT NULL CHECK (change_kind IN (
        'insert','supersede','close','compare','invalidate'
    )),
    timeline_event_kind TEXT NOT NULL,
    invalidation_reason TEXT,
    change_hash TEXT NOT NULL CHECK (change_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_occurred_at TIMESTAMPTZ,
    source_time_status TEXT NOT NULL CHECK (source_time_status IN ('known','historical_unknown')),
    projected_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY(operation_id,change_seq),
    CHECK ((source_time_status='known')=(source_occurred_at IS NOT NULL)),
    CHECK ((change_kind='invalidate')=(invalidation_reason IS NOT NULL)),
    CHECK (invalidation_reason IS NULL OR invalidation_reason IN (
        'source_superseded','source_quarantined','authority_stale','source_deleted',
        'legacy_projection_unsupported','legacy_projection_derivation_failed',
        'legacy_projection_diverged','contract_unsupported'
    )),
    CHECK (projection_timeline_mapping_is_valid(entity_kind,change_kind,timeline_event_kind)),
    FOREIGN KEY(operation_id) REFERENCES investigation_projection_heads(operation_id) ON DELETE RESTRICT,
    FOREIGN KEY(batch_id,operation_id,source_batch_seq)
        REFERENCES investigation_projection_outbox_batches(
            batch_id,operation_id,source_batch_seq
        ) ON DELETE RESTRICT,
    FOREIGN KEY(outbox_member_id,batch_id,operation_id,source_batch_seq)
        REFERENCES investigation_projection_outbox(
            outbox_member_id,batch_id,operation_id,source_batch_seq
        ) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,entity_kind,entity_id,entity_version)
        REFERENCES investigation_projection_entity_versions(
            operation_id,entity_kind,entity_id,entity_version
        ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE hypothesis_legacy_candidate_projection_versions (
    legacy_candidate_projection_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    entity_id UUID NOT NULL,
    entity_version BIGINT NOT NULL CHECK (entity_version>0),
    source_generation_id UUID NOT NULL REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    source_revision_id UUID NOT NULL REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    source_contract_hash TEXT NOT NULL CHECK (source_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    projection_status TEXT NOT NULL CHECK (projection_status IN ('ready','unsupported','invalidated')),
    projection_body JSONB CHECK (projection_body IS NULL OR jsonb_typeof(projection_body)='object'),
    projection_hash TEXT NOT NULL CHECK (projection_hash ~ '^sha256:[0-9a-f]{64}$'),
    batch_id UUID NOT NULL REFERENCES investigation_projection_outbox_batches(batch_id) ON DELETE RESTRICT,
    change_seq BIGINT NOT NULL CHECK (change_seq>0),
    invalidation_reason TEXT,
    projected_at TIMESTAMPTZ NOT NULL,
    CHECK ((projection_status='ready')=(projection_body IS NOT NULL)),
    UNIQUE(operation_id,entity_id,entity_version),
    UNIQUE(legacy_candidate_projection_id,operation_id)
);

CREATE TABLE hypothesis_legacy_attempt_projection_versions (
    legacy_attempt_projection_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    entity_id UUID NOT NULL,
    entity_version BIGINT NOT NULL CHECK (entity_version>0),
    source_generation_id UUID NOT NULL REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    source_revision_id UUID NOT NULL REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    source_contract_hash TEXT NOT NULL CHECK (source_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    projection_status TEXT NOT NULL CHECK (projection_status IN ('ready','unsupported','invalidated')),
    projection_body JSONB CHECK (projection_body IS NULL OR jsonb_typeof(projection_body)='object'),
    projection_hash TEXT NOT NULL CHECK (projection_hash ~ '^sha256:[0-9a-f]{64}$'),
    batch_id UUID NOT NULL REFERENCES investigation_projection_outbox_batches(batch_id) ON DELETE RESTRICT,
    change_seq BIGINT NOT NULL CHECK (change_seq>0),
    invalidation_reason TEXT,
    projected_at TIMESTAMPTZ NOT NULL,
    CHECK ((projection_status='ready')=(projection_body IS NOT NULL)),
    UNIQUE(operation_id,entity_id,entity_version),
    UNIQUE(legacy_attempt_projection_id,operation_id)
);

CREATE TABLE investigation_projection_compare_samples (
    comparison_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID,
    projection_schema_version INTEGER NOT NULL DEFAULT 1 CHECK (projection_schema_version=1),
    as_of_change_seq BIGINT NOT NULL CHECK (as_of_change_seq>=0),
    comparison_contract_version TEXT NOT NULL DEFAULT 'comparison_record.v1'
        CHECK (comparison_contract_version='comparison_record.v1'),
    tool_truth_contract TEXT NOT NULL CHECK (
        tool_truth_contract IN ('legacy_v1','shadow_v1','receipt_v1')
    ),
    investigation_contract_version TEXT NOT NULL CHECK (
        investigation_contract_version IN ('legacy_candidate_v1','hypothesis_registry_v1')
    ),
    investigation_rollout_mode TEXT NOT NULL CHECK (
        investigation_rollout_mode IN (
            'legacy_only','shadow_registry','dual_read_compare',
            'registry_authoritative_legacy_projection','new_only'
        )
    ),
    record_kind TEXT NOT NULL CHECK (btrim(record_kind)<>''),
    record_key TEXT NOT NULL CHECK (btrim(record_key)<>''),
    legacy_hash TEXT CHECK (legacy_hash IS NULL OR legacy_hash ~ '^sha256:[0-9a-f]{64}$'),
    registry_hash TEXT CHECK (registry_hash IS NULL OR registry_hash ~ '^sha256:[0-9a-f]{64}$'),
    comparison_state TEXT NOT NULL CHECK (comparison_state IN (
        'match','mismatch','registry_missing','legacy_projection_missing',
        'incomplete','authority_corrupt'
    )),
    diff_summary JSONB NOT NULL DEFAULT '{}'::JSONB CHECK (jsonb_typeof(diff_summary)='object'),
    compared_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (operation_joint_contract_rank(
        tool_truth_contract,investigation_contract_version,investigation_rollout_mode
    ) IS NOT NULL),
    UNIQUE(operation_id,as_of_change_seq,record_kind,record_key),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

ALTER TABLE attack_candidate_work_items
    ADD COLUMN hypothesis_revision_id UUID REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    ADD COLUMN legacy_projection_id UUID REFERENCES hypothesis_legacy_candidate_projection_versions(legacy_candidate_projection_id) ON DELETE RESTRICT;

ALTER TABLE attack_candidates
    ADD COLUMN hypothesis_revision_id UUID REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    ADD COLUMN legacy_projection_id UUID REFERENCES hypothesis_legacy_candidate_projection_versions(legacy_candidate_projection_id) ON DELETE RESTRICT;

CREATE TRIGGER investigation_projection_source_blobs_append_only BEFORE UPDATE OR DELETE ON investigation_projection_source_blobs FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER investigation_projection_outbox_append_only BEFORE UPDATE OR DELETE ON investigation_projection_outbox FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER investigation_projection_changes_append_only BEFORE UPDATE OR DELETE ON investigation_projection_changes FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_legacy_candidate_projection_versions_append_only BEFORE UPDATE OR DELETE ON hypothesis_legacy_candidate_projection_versions FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER hypothesis_legacy_attempt_projection_versions_append_only BEFORE UPDATE OR DELETE ON hypothesis_legacy_attempt_projection_versions FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER investigation_projection_compare_samples_append_only BEFORE UPDATE OR DELETE ON investigation_projection_compare_samples FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();

-- Source and projection heads are the only mutable projection rows. Their
-- updates are guarded by complete immutable batch/receipt truth, not by
-- per-outbox processed flags.
CREATE FUNCTION enforce_investigation_source_head_advance()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    batch investigation_projection_outbox_batches%ROWTYPE;
    actual_count BIGINT;
    min_ordinal BIGINT;
    max_ordinal BIGINT;
    actual_hash TEXT;
BEGIN
    IF NEW.operation_id IS DISTINCT FROM OLD.operation_id
       OR NEW.last_source_batch_seq<>OLD.last_source_batch_seq+1
       OR NEW.last_source_batch_id IS NULL
    THEN
        RAISE EXCEPTION 'INVESTIGATION_SOURCE_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT * INTO batch FROM investigation_projection_outbox_batches
     WHERE batch_id=NEW.last_source_batch_id
       AND operation_id=NEW.operation_id
       AND source_batch_seq=NEW.last_source_batch_seq;
    IF NOT FOUND OR batch.predecessor_batch_id IS DISTINCT FROM OLD.last_source_batch_id THEN
        RAISE EXCEPTION 'INVESTIGATION_SOURCE_BATCH_PREDECESSOR_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*),COALESCE(MIN(member_ordinal),0),COALESCE(MAX(member_ordinal),-1),
           tool_truth_sha256(COALESCE(jsonb_agg(member_hash ORDER BY member_ordinal),'[]'::jsonb)::text)
      INTO actual_count,min_ordinal,max_ordinal,actual_hash
      FROM investigation_projection_outbox WHERE batch_id=batch.batch_id;
    IF actual_count<>batch.member_count
       OR min_ordinal<>0 OR max_ordinal<>actual_count-1
       OR actual_hash<>batch.member_set_hash
    THEN
        RAISE EXCEPTION 'INVESTIGATION_SOURCE_BATCH_EXACT_SET_INVALID' USING ERRCODE='23514';
    END IF;
    NEW.updated_at:=statement_timestamp();
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_projection_source_head_advance_guard
BEFORE UPDATE ON investigation_projection_source_heads
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_source_head_advance();

CREATE FUNCTION enforce_investigation_projection_head_advance()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    receipt investigation_projection_batch_receipts%ROWTYPE;
    entity_count BIGINT;
    change_count BIGINT;
BEGIN
    IF NEW.operation_id IS DISTINCT FROM OLD.operation_id
       OR NEW.change_seq<=OLD.change_seq
       OR NEW.last_projected_batch_id IS NULL
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PROJECTION_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT * INTO receipt FROM investigation_projection_batch_receipts
     WHERE batch_id=NEW.last_projected_batch_id AND operation_id=NEW.operation_id;
    IF NOT FOUND
       OR receipt.predecessor_batch_id IS DISTINCT FROM OLD.last_projected_batch_id
       OR receipt.first_change_seq<>OLD.change_seq+1
       OR receipt.last_change_seq<>NEW.change_seq
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PROJECTION_RECEIPT_PREDECESSOR_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*) INTO entity_count
      FROM investigation_projection_entity_versions WHERE batch_id=receipt.batch_id;
    SELECT COUNT(*) INTO change_count
      FROM investigation_projection_changes WHERE batch_id=receipt.batch_id;
    IF entity_count<>change_count
       OR change_count<>receipt.last_change_seq-receipt.first_change_seq+1
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PROJECTION_BATCH_EXACT_SET_INVALID' USING ERRCODE='23514';
    END IF;
    NEW.updated_at:=statement_timestamp();
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_projection_head_advance_guard
BEFORE UPDATE OF change_seq,last_projected_batch_id ON investigation_projection_heads
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_projection_head_advance();

CREATE FUNCTION enforce_candidate_server_phase_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    candidate_item candidate_analysis_work_items%ROWTYPE;
    attempt candidate_analysis_attempts%ROWTYPE;
    snapshot candidate_analysis_snapshots%ROWTYPE;
    input_count BIGINT;
    adequate_review_count BIGINT;
BEGIN
    IF NEW.created_by<>'server_phase_transition' THEN
        RETURN NULL;
    END IF;
    SELECT * INTO candidate_item FROM candidate_analysis_work_items
     WHERE stage_work_item_id=NEW.id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'CANDIDATE_PHASE_TRANSITION_BINDING_REQUIRED' USING ERRCODE='23514';
    END IF;
    SELECT * INTO attempt FROM candidate_analysis_attempts
     WHERE analysis_attempt_id=candidate_item.analysis_attempt_id;
    SELECT * INTO snapshot FROM candidate_analysis_snapshots
     WHERE snapshot_id=attempt.snapshot_id;
    IF ROW(attempt.operation_id,attempt.organization_id)
       IS DISTINCT FROM ROW(NEW.operation_id,NEW.organization_id)
       OR snapshot.snapshot_status<>'sealed_ready'
    THEN
        RAISE EXCEPTION 'CANDIDATE_PHASE_TRANSITION_AUTHORITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF candidate_item.phase='proposal' THEN
        RAISE EXCEPTION 'CANDIDATE_PROPOSAL_REQUIRES_SERVER_SEED' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM candidate_analysis_proposal_censuses census
         WHERE census.analysis_attempt_id=attempt.analysis_attempt_id
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_H1_CENSUS_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF candidate_item.phase='controller' THEN
        IF NOT EXISTS (
            SELECT 1 FROM candidate_analysis_critic_censuses census
             WHERE census.analysis_attempt_id=attempt.analysis_attempt_id
        ) OR NOT EXISTS (
            SELECT 1 FROM candidate_analysis_hypothesis_coverage_global_reviews review
             WHERE review.analysis_attempt_id=attempt.analysis_attempt_id
               AND review.outcome='adequate'
        ) THEN
            RAISE EXCEPTION 'CANDIDATE_H2_GLOBAL_REVIEW_REQUIRED' USING ERRCODE='23514';
        END IF;
        SELECT COUNT(*) INTO input_count FROM candidate_analysis_snapshot_inputs
         WHERE snapshot_id=attempt.snapshot_id;
        SELECT COUNT(*) INTO adequate_review_count
          FROM candidate_analysis_hypothesis_coverage_reviews review
         WHERE review.analysis_attempt_id=attempt.analysis_attempt_id
           AND review.outcome='adequate';
        IF input_count<>adequate_review_count THEN
            RAISE EXCEPTION 'CANDIDATE_INPUT_COVERAGE_REVIEW_EXACT_SET_REQUIRED'
                USING ERRCODE='23514';
        END IF;
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER stage_work_item_candidate_phase_transition_guard
AFTER INSERT ON stage_work_items
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_server_phase_transition();

CREATE FUNCTION enforce_candidate_attempt_event_chain()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    opened_count BIGINT;
    active_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO opened_count
      FROM candidate_analysis_attempt_state_events
     WHERE analysis_attempt_id=NEW.analysis_attempt_id AND event_kind='opened';
    IF opened_count<>1 THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_OPENED_EVENT_EXACT_ONE_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    IF (NEW.attempt_ordinal=0) IS DISTINCT FROM (NEW.predecessor_attempt_id IS NULL)
       OR (
           NEW.predecessor_attempt_id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM candidate_analysis_attempts predecessor
                WHERE predecessor.analysis_attempt_id=NEW.predecessor_attempt_id
                  AND predecessor.snapshot_id=NEW.snapshot_id
                  AND predecessor.attempt_ordinal=NEW.attempt_ordinal-1
           )
       )
       OR (
           NEW.predecessor_attempt_id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM candidate_analysis_attempt_state_events predecessor_event
                WHERE predecessor_event.analysis_attempt_id=NEW.predecessor_attempt_id
                  AND predecessor_event.event_kind='superseded_missed_hypothesis'
           )
       )
    THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_PREDECESSOR_INVALID' USING ERRCODE='23514';
    END IF;
    SELECT COUNT(*) INTO active_count
      FROM candidate_analysis_attempts attempt
     WHERE attempt.snapshot_id=NEW.snapshot_id
       AND NOT EXISTS (
           SELECT 1 FROM candidate_analysis_attempt_state_events terminal
            WHERE terminal.analysis_attempt_id=attempt.analysis_attempt_id
              AND terminal.event_kind IN ('superseded_missed_hypothesis','sealed','blocked')
       );
    IF active_count<>1 THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_ACTIVE_EXACT_ONE_REQUIRED' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER candidate_analysis_attempt_event_chain_guard
AFTER INSERT ON candidate_analysis_attempts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_attempt_event_chain();
