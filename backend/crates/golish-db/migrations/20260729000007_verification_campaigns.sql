-- Plan C: append-only Verification Campaign authority.
--
-- This migration is deliberately additive.  Plan B remains the only owner of
-- HypothesisVerificationPlanV1, residuals and the investigation outbox/catalog.

CREATE FUNCTION verification_reject_append_only()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'VERIFICATION_APPEND_ONLY_VIOLATION' USING ERRCODE='23514';
END;
$$;

CREATE FUNCTION verification_guard_sealable_header()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'VERIFICATION_SEALED_HEADER_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF OLD.sealed_at IS NOT NULL OR NEW.sealed_at IS NULL
       OR (to_jsonb(NEW)-'sealed_at') IS DISTINCT FROM (to_jsonb(OLD)-'sealed_at')
    THEN
        RAISE EXCEPTION 'VERIFICATION_SEALED_HEADER_MUTATION_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION verification_guard_set_member()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    header_table REGCLASS := TG_ARGV[0]::REGCLASS;
    header_id_column TEXT := TG_ARGV[1];
    member_header_column TEXT := TG_ARGV[2];
    owner_id UUID;
    sealed TIMESTAMPTZ;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'VERIFICATION_SET_MEMBER_APPEND_ONLY' USING ERRCODE='23514';
    END IF;
    owner_id := (to_jsonb(NEW)->>member_header_column)::UUID;
    EXECUTE format('SELECT sealed_at FROM %s WHERE %I=$1 FOR SHARE',header_table,header_id_column)
       INTO sealed USING owner_id;
    IF sealed IS NOT NULL THEN
        RAISE EXCEPTION 'VERIFICATION_SET_ALREADY_SEALED' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TABLE verification_campaign_safety_holds (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    campaign_dispatch_held BOOLEAN NOT NULL DEFAULT TRUE,
    operation_admission_held BOOLEAN NOT NULL DEFAULT FALSE,
    campaign_dispatch_generation BIGINT NOT NULL DEFAULT 0 CHECK (campaign_dispatch_generation>=0),
    operation_admission_generation BIGINT NOT NULL DEFAULT 0 CHECK (operation_admission_generation>=0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    reason_code TEXT NOT NULL DEFAULT 'initial_rollout_hold' CHECK (BTRIM(reason_code)<>''),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);
INSERT INTO verification_campaign_safety_holds(singleton) VALUES(TRUE);

CREATE FUNCTION verification_guard_safety_hold_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE' OR NEW.singleton IS DISTINCT FROM OLD.singleton
       OR NEW.row_version<>OLD.row_version+1
       OR NEW.campaign_dispatch_generation<OLD.campaign_dispatch_generation
       OR NEW.operation_admission_generation<OLD.operation_admission_generation
       OR NEW.campaign_dispatch_generation>OLD.campaign_dispatch_generation+1
       OR NEW.operation_admission_generation>OLD.operation_admission_generation+1
       OR ((NEW.campaign_dispatch_held IS DISTINCT FROM OLD.campaign_dispatch_held)
           <> (NEW.campaign_dispatch_generation=OLD.campaign_dispatch_generation+1))
       OR ((NEW.operation_admission_held IS DISTINCT FROM OLD.operation_admission_held)
           <> (NEW.operation_admission_generation=OLD.operation_admission_generation+1))
       OR BTRIM(NEW.reason_code)=''
    THEN
        RAISE EXCEPTION 'VERIFICATION_SAFETY_HOLD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_campaign_safety_hold_cas
BEFORE UPDATE OR DELETE ON verification_campaign_safety_holds
FOR EACH ROW EXECUTE FUNCTION verification_guard_safety_hold_cas();

-- Capability availability is durable input authority, never a transient
-- compiler boolean.  The Plan B objective/contract is referenced directly.
CREATE TABLE verification_capability_assessments (
    assessment_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    verification_contract_id UUID NOT NULL,
    verification_contract_hash TEXT NOT NULL CHECK (verification_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    capability_key TEXT NOT NULL CHECK (BTRIM(capability_key)<>''),
    capability_contract_version TEXT NOT NULL CHECK (BTRIM(capability_contract_version)<>''),
    capability_contract_hash TEXT NOT NULL CHECK (capability_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    policy_snapshot_id UUID NOT NULL,
    policy_snapshot_hash TEXT NOT NULL CHECK (policy_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    assessment_ordinal BIGINT NOT NULL CHECK (assessment_ordinal>=0),
    supersedes_assessment_id UUID,
    status TEXT NOT NULL CHECK (status IN (
        'unassessed','available','adapter_missing','policy_denied','prerequisite_missing'
    )),
    reason_code TEXT,
    residual_id UUID,
    adapter_contract_version TEXT,
    adapter_contract_digest TEXT CHECK (
        adapter_contract_digest IS NULL OR adapter_contract_digest ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    assessment_hash TEXT NOT NULL CHECK (assessment_hash ~ '^sha256:[0-9a-f]{64}$'),
    assessed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (status='available' AND adapter_contract_version IS NOT NULL
            AND adapter_contract_digest IS NOT NULL AND reason_code IS NULL AND residual_id IS NULL)
        OR (status<>'available' AND adapter_contract_version IS NULL
            AND adapter_contract_digest IS NULL AND BTRIM(reason_code)<>'' AND residual_id IS NOT NULL)
    ),
    UNIQUE(assessment_id,operation_id,project_scope_id,organization_id),
    UNIQUE(assessment_id,hypothesis_revision_id,verification_objective_id),
    UNIQUE(assessment_id,hypothesis_revision_id,verification_objective_id,verification_contract_hash),
    UNIQUE(
        hypothesis_revision_id,verification_objective_id,verification_contract_hash,
        capability_contract_hash,policy_snapshot_hash,assessment_ordinal
    ),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(verification_contract_id,hypothesis_revision_id,verification_objective_id,verification_contract_hash)
        REFERENCES attack_hypothesis_verification_contracts(
            contract_id,revision_id,objective_id,contract_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    FOREIGN KEY(supersedes_assessment_id)
        REFERENCES verification_capability_assessments(assessment_id) ON DELETE RESTRICT
);

CREATE TABLE verification_capability_assessment_set_seals (
    assessment_set_seal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    verification_contract_hash TEXT NOT NULL CHECK (verification_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    policy_snapshot_hash TEXT NOT NULL CHECK (policy_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    registry_contract_hash TEXT NOT NULL CHECK (registry_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>=0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    seal_hash TEXT NOT NULL CHECK (seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ,
    UNIQUE(assessment_set_seal_id,operation_id,project_scope_id,organization_id),
    UNIQUE(assessment_set_seal_id,hypothesis_revision_id,verification_objective_id,verification_contract_hash),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE verification_capability_assessment_set_members (
    assessment_set_seal_id UUID NOT NULL,
    assessment_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    capability_key TEXT NOT NULL CHECK (BTRIM(capability_key)<>''),
    assessment_ordinal BIGINT NOT NULL CHECK (assessment_ordinal>=0),
    assessment_hash TEXT NOT NULL CHECK (assessment_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(assessment_set_seal_id,member_ordinal),
    UNIQUE(assessment_set_seal_id,assessment_id),
    UNIQUE(assessment_set_seal_id,member_hash),
    FOREIGN KEY(assessment_set_seal_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_capability_assessment_set_seals(
            assessment_set_seal_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(assessment_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_capability_assessments(
            assessment_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

-- A generation seal is the Plan B Wave authority.  The denominator partitions
-- it before any Campaign is admitted.
CREATE TABLE verification_wave_coverage_denominators (
    wave_denominator_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_seal_id UUID NOT NULL,
    contract_version TEXT NOT NULL CHECK (BTRIM(contract_version)<>''),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    sealed_at TIMESTAMPTZ,
    UNIQUE(generation_seal_id),
    UNIQUE(wave_denominator_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(generation_seal_id)
        REFERENCES hypothesis_generation_seals(seal_id) ON DELETE RESTRICT
);

CREATE TABLE verification_wave_coverage_members (
    wave_coverage_member_id UUID PRIMARY KEY,
    wave_denominator_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    semantic_key TEXT NOT NULL CHECK (BTRIM(semantic_key)<>''),
    input_ref_kind TEXT NOT NULL CHECK (BTRIM(input_ref_kind)<>''),
    input_ref_id UUID NOT NULL,
    input_identity_hash TEXT NOT NULL CHECK (input_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    hypothesis_revision_id UUID NOT NULL,
    claim_component_id UUID NOT NULL,
    claim_component_hash TEXT NOT NULL CHECK (claim_component_hash ~ '^sha256:[0-9a-f]{64}$'),
    verification_objective_id UUID NOT NULL,
    predicate_component_id UUID NOT NULL,
    control_binding_kind TEXT NOT NULL CHECK (control_binding_kind IN ('required','explicit_no_control')),
    required_control_id UUID,
    required_control_hash TEXT CHECK (required_control_hash IS NULL OR required_control_hash ~ '^sha256:[0-9a-f]{64}$'),
    no_control_marker_hash TEXT CHECK (no_control_marker_hash IS NULL OR no_control_marker_hash ~ '^sha256:[0-9a-f]{64}$'),
    capability_assessment_id UUID NOT NULL,
    expected_capability_kind TEXT NOT NULL CHECK (BTRIM(expected_capability_kind)<>''),
    expected_action_kind TEXT NOT NULL CHECK (BTRIM(expected_action_kind)<>''),
    expected_oracle_kind TEXT NOT NULL CHECK (BTRIM(expected_oracle_kind)<>''),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (
        (control_binding_kind='required' AND required_control_id IS NOT NULL
            AND required_control_hash IS NOT NULL AND no_control_marker_hash IS NULL)
        OR (control_binding_kind='explicit_no_control' AND required_control_id IS NULL
            AND required_control_hash IS NULL AND no_control_marker_hash IS NOT NULL)
    ),
    UNIQUE(wave_denominator_id,member_ordinal),
    UNIQUE(wave_denominator_id,semantic_key),
    UNIQUE(wave_denominator_id,member_hash),
    UNIQUE(wave_coverage_member_id,wave_denominator_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(wave_denominator_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_wave_coverage_denominators(
            wave_denominator_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(claim_component_id,hypothesis_revision_id,claim_component_hash)
        REFERENCES attack_hypothesis_claim_components(
            component_id,revision_id,member_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(capability_assessment_id,hypothesis_revision_id,verification_objective_id)
        REFERENCES verification_capability_assessments(
            assessment_id,hypothesis_revision_id,verification_objective_id
        ) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX attack_hypothesis_verification_plan_objective_campaign_fk
ON attack_hypothesis_verification_plan_objectives(
    plan_objective_id,plan_id,revision_id,verification_contract_hash
);

CREATE UNIQUE INDEX attack_hypothesis_verification_plan_objective_assignment_fk
ON attack_hypothesis_verification_plan_objectives(
    plan_objective_id,plan_id,revision_id,objective_id
);

CREATE UNIQUE INDEX attack_hypothesis_verification_plan_objective_plan_fk
ON attack_hypothesis_verification_plan_objectives(
    plan_objective_id,plan_id,revision_id
);

CREATE TABLE verification_campaigns (
    campaign_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    verification_plan_hash TEXT NOT NULL CHECK (verification_plan_hash ~ '^sha256:[0-9a-f]{64}$'),
    plan_objective_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    verification_contract_id UUID NOT NULL,
    verification_contract_hash TEXT NOT NULL CHECK (verification_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    capability_assessment_set_seal_id UUID NOT NULL,
    wave_denominator_id UUID NOT NULL,
    tool_truth_authority_bundle_seal_id UUID NOT NULL,
    relevant_root_set_hash TEXT NOT NULL CHECK (relevant_root_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    authority_member_set_hash TEXT NOT NULL CHECK (authority_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    semantic_authority_bundle_hash TEXT NOT NULL CHECK (semantic_authority_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    freshness_attestation_bundle_hash TEXT NOT NULL CHECK (freshness_attestation_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    temporal_validity_bundle_hash TEXT NOT NULL CHECK (temporal_validity_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    effective_valid_until TIMESTAMPTZ NOT NULL,
    campaign_version BIGINT NOT NULL CHECK (campaign_version>0),
    state TEXT NOT NULL CHECK (state IN ('admitted','running','stopping','draining','terminal','superseded')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    admitted_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    terminal_at TIMESTAMPTZ,
    superseded_at TIMESTAMPTZ,
    CHECK ((terminal_at IS NULL OR superseded_at IS NULL) AND NOT (terminal_at IS NOT NULL AND superseded_at IS NOT NULL)),
    UNIQUE(campaign_id,operation_id,project_scope_id,organization_id),
    UNIQUE(campaign_id,hypothesis_revision_id,verification_objective_id,verification_contract_hash),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(verification_plan_id,hypothesis_revision_id,verification_plan_hash)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id,plan_hash) ON DELETE RESTRICT,
    FOREIGN KEY(plan_objective_id,verification_plan_id,hypothesis_revision_id,verification_contract_hash)
        REFERENCES attack_hypothesis_verification_plan_objectives(
            plan_objective_id,plan_id,revision_id,verification_contract_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(verification_contract_id,hypothesis_revision_id,verification_objective_id,verification_contract_hash)
        REFERENCES attack_hypothesis_verification_contracts(
            contract_id,revision_id,objective_id,contract_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(capability_assessment_set_seal_id,hypothesis_revision_id,verification_objective_id,verification_contract_hash)
        REFERENCES verification_capability_assessment_set_seals(
            assessment_set_seal_id,hypothesis_revision_id,verification_objective_id,verification_contract_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(wave_denominator_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_wave_coverage_denominators(
            wave_denominator_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(tool_truth_authority_bundle_seal_id,operation_id,organization_id)
        REFERENCES tool_truth_authority_bundle_seals(id,operation_id,organization_id)
        ON DELETE RESTRICT
);

CREATE UNIQUE INDEX verification_campaigns_one_active_contract
ON verification_campaigns(hypothesis_revision_id,verification_objective_id,verification_contract_hash)
WHERE terminal_at IS NULL AND superseded_at IS NULL;

CREATE TABLE verification_campaign_coverage_denominators (
    campaign_denominator_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    campaign_id UUID NOT NULL UNIQUE,
    hypothesis_revision_id UUID NOT NULL,
    wave_denominator_id UUID NOT NULL,
    contract_version TEXT NOT NULL CHECK (BTRIM(contract_version)<>''),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    sealed_at TIMESTAMPTZ,
    UNIQUE(campaign_denominator_id,campaign_id,operation_id,project_scope_id,organization_id),
    UNIQUE(campaign_denominator_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(wave_denominator_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_wave_coverage_denominators(
            wave_denominator_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_campaign_coverage_members (
    campaign_coverage_member_id UUID PRIMARY KEY,
    campaign_denominator_id UUID NOT NULL,
    wave_coverage_member_id UUID NOT NULL,
    wave_denominator_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    semantic_key TEXT NOT NULL CHECK (BTRIM(semantic_key)<>''),
    claim_component_id UUID NOT NULL,
    claim_component_hash TEXT NOT NULL CHECK (claim_component_hash ~ '^sha256:[0-9a-f]{64}$'),
    obligation_kind TEXT NOT NULL CHECK (BTRIM(obligation_kind)<>''),
    control_binding_kind TEXT NOT NULL CHECK (control_binding_kind IN ('required','explicit_no_control')),
    capability_assessment_id UUID NOT NULL,
    expected_capability_kind TEXT NOT NULL CHECK (BTRIM(expected_capability_kind)<>''),
    expected_action_kind TEXT NOT NULL CHECK (BTRIM(expected_action_kind)<>''),
    expected_oracle_kind TEXT NOT NULL CHECK (BTRIM(expected_oracle_kind)<>''),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(campaign_denominator_id,member_ordinal),
    UNIQUE(campaign_denominator_id,semantic_key),
    UNIQUE(campaign_denominator_id,wave_coverage_member_id),
    UNIQUE(campaign_denominator_id,member_hash),
    UNIQUE(campaign_coverage_member_id,campaign_denominator_id),
    FOREIGN KEY(campaign_denominator_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_coverage_denominators(
            campaign_denominator_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(wave_coverage_member_id,wave_denominator_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_wave_coverage_members(
            wave_coverage_member_id,wave_denominator_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(capability_assessment_id)
        REFERENCES verification_capability_assessments(assessment_id) ON DELETE RESTRICT
);

CREATE FUNCTION verification_derive_campaign_coverage_member_owner()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    SELECT wave_denominator_id INTO NEW.wave_denominator_id
      FROM verification_campaign_coverage_denominators
     WHERE campaign_denominator_id=NEW.campaign_denominator_id
       AND operation_id=NEW.operation_id AND project_scope_id=NEW.project_scope_id
       AND organization_id=NEW.organization_id FOR SHARE;
    IF NEW.wave_denominator_id IS NULL THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_DENOMINATOR_OWNER_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM verification_wave_coverage_members wave
         WHERE wave.wave_coverage_member_id=NEW.wave_coverage_member_id
           AND wave.wave_denominator_id=NEW.wave_denominator_id
           AND wave.operation_id=NEW.operation_id
           AND wave.project_scope_id=NEW.project_scope_id
           AND wave.organization_id=NEW.organization_id
           AND wave.semantic_key=NEW.semantic_key
           AND wave.claim_component_id=NEW.claim_component_id
           AND wave.claim_component_hash=NEW.claim_component_hash
           AND wave.control_binding_kind=NEW.control_binding_kind
           AND wave.capability_assessment_id=NEW.capability_assessment_id
           AND wave.expected_capability_kind=NEW.expected_capability_kind
           AND wave.expected_action_kind=NEW.expected_action_kind
           AND wave.expected_oracle_kind=NEW.expected_oracle_kind
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_MEMBER_WAVE_AUTHORITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_campaign_coverage_member_owner
BEFORE INSERT ON verification_campaign_coverage_members
FOR EACH ROW EXECUTE FUNCTION verification_derive_campaign_coverage_member_owner();

CREATE TABLE verification_campaign_rounds (
    round_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    round_ordinal INTEGER NOT NULL CHECK (round_ordinal>=0),
    expected_campaign_row_version BIGINT NOT NULL CHECK (expected_campaign_row_version>=0),
    round_input JSONB NOT NULL CHECK (jsonb_typeof(round_input)='object'),
    round_input_hash TEXT NOT NULL CHECK (round_input_hash ~ '^sha256:[0-9a-f]{64}$'),
    consult_member_count BIGINT NOT NULL CHECK (consult_member_count>=0),
    consult_member_set_hash TEXT NOT NULL CHECK (consult_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT CHECK (disposition IS NULL OR disposition IN (
        'action_compiled','no_action_compilable','denied','budget_stopped','superseded','terminal'
    )),
    disposition_reason_code TEXT,
    residual_id UUID,
    opened_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    closed_at TIMESTAMPTZ,
    CHECK ((closed_at IS NULL)=(disposition IS NULL)),
    CHECK (disposition IS NULL OR BTRIM(disposition_reason_code)<>''),
    UNIQUE(campaign_id,round_ordinal),
    UNIQUE(round_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE TABLE verification_consults (
    consult_id UUID PRIMARY KEY,
    round_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    consult_ordinal INTEGER NOT NULL CHECK (consult_ordinal>=0),
    role_kind TEXT NOT NULL CHECK (BTRIM(role_kind)<>''),
    request_packet JSONB NOT NULL CHECK (jsonb_typeof(request_packet)='object'),
    request_packet_hash TEXT NOT NULL CHECK (request_packet_hash ~ '^sha256:[0-9a-f]{64}$'),
    response_artifact JSONB,
    response_artifact_hash TEXT CHECK (response_artifact_hash IS NULL OR response_artifact_hash ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT NOT NULL CHECK (disposition IN ('pending','completed','failed','cancelled')),
    residual_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((response_artifact IS NULL)=(response_artifact_hash IS NULL)),
    CHECK ((disposition='completed')=(response_artifact IS NOT NULL)),
    UNIQUE(round_id,consult_ordinal),
    UNIQUE(round_id,role_kind),
    UNIQUE(consult_id,round_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(round_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_rounds(
            round_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

-- Provider execution happens only after the complete consult census above is
-- frozen.  Terminal outcomes are separate immutable facts so a queued census
-- member is never updated in place and failed/timeout calls cannot masquerade
-- as completed proposals.
CREATE TABLE verification_consult_terminals (
    consult_terminal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    consult_id UUID NOT NULL UNIQUE,
    round_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    role_kind TEXT NOT NULL CHECK (BTRIM(role_kind)<>''),
    input_projection_hash TEXT NOT NULL CHECK (input_projection_hash ~ '^sha256:[0-9a-f]{64}$'),
    terminal_state TEXT NOT NULL CHECK (terminal_state IN ('completed','failed','timed_out','cancelled')),
    response_artifact JSONB,
    response_artifact_hash TEXT CHECK (response_artifact_hash IS NULL OR response_artifact_hash ~ '^sha256:[0-9a-f]{64}$'),
    reason_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((response_artifact IS NULL)=(response_artifact_hash IS NULL)),
    CHECK ((terminal_state='completed')=(response_artifact IS NOT NULL)),
    CHECK ((terminal_state='completed')=(reason_code IS NULL)),
    CHECK (terminal_state='completed' OR BTRIM(COALESCE(reason_code,''))<>''),
    FOREIGN KEY(consult_id,round_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_consults(
            consult_id,round_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_strategy_artifacts (
    strategy_artifact_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    round_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    decision_kind TEXT NOT NULL CHECK (decision_kind IN ('compile_action','no_action_compilable','stop','refine')),
    typed_strategy JSONB NOT NULL CHECK (jsonb_typeof(typed_strategy)='object'),
    strategy_hash TEXT NOT NULL CHECK (strategy_hash ~ '^sha256:[0-9a-f]{64}$'),
    obligation_member_count BIGINT NOT NULL CHECK (obligation_member_count>=0),
    obligation_member_set_hash TEXT NOT NULL CHECK (obligation_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    residual_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(round_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_rounds(
            round_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE TABLE verification_strategy_obligations (
    strategy_artifact_id UUID NOT NULL REFERENCES verification_strategy_artifacts(strategy_artifact_id) ON DELETE RESTRICT,
    obligation_id UUID NOT NULL,
    obligation_ordinal INTEGER NOT NULL CHECK (obligation_ordinal>=0),
    obligation_kind TEXT NOT NULL CHECK (BTRIM(obligation_kind)<>''),
    semantic_key TEXT NOT NULL CHECK (BTRIM(semantic_key)<>''),
    disposition TEXT NOT NULL CHECK (disposition IN ('planned','blocked','unsupported','not_applicable')),
    residual_id UUID,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(strategy_artifact_id,obligation_id),
    UNIQUE(strategy_artifact_id,obligation_ordinal),
    UNIQUE(strategy_artifact_id,semantic_key),
    CHECK ((disposition='planned')=(residual_id IS NULL)),
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE TABLE verification_credential_authority_heads (
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    handle_id UUID NOT NULL REFERENCES vault_entries(id) ON DELETE RESTRICT,
    handle_version BIGINT NOT NULL CHECK (handle_version>0),
    revocation_generation BIGINT NOT NULL CHECK (revocation_generation>=0),
    revoked BOOLEAN NOT NULL DEFAULT FALSE,
    injection_origin TEXT NOT NULL CHECK (BTRIM(injection_origin)<>''),
    injection_contract_version TEXT NOT NULL CHECK (BTRIM(injection_contract_version)<>''),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(operation_id,handle_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

-- A prepared action freezes the exact credential version and revocation
-- generation that was reviewed. V1 deliberately has no in-place rotation
-- path: any vault mutation makes the frozen head stale and the action must be
-- recompiled/re-authorized. This prevents direct SQL from silently widening a
-- live send authority.
CREATE FUNCTION verification_reject_credential_authority_head_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'VERIFICATION_CREDENTIAL_AUTHORITY_IMMUTABLE'
        USING ERRCODE='23514';
END;
$$;

CREATE TRIGGER verification_credential_authority_heads_immutable
BEFORE UPDATE OR DELETE ON verification_credential_authority_heads
FOR EACH ROW EXECUTE FUNCTION verification_reject_credential_authority_head_mutation();

CREATE TABLE verification_prepared_actions (
    prepared_action_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL,
    round_id UUID NOT NULL,
    strategy_artifact_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    capability_assessment_id UUID NOT NULL,
    action_ordinal INTEGER NOT NULL CHECK (action_ordinal>=0),
    action_contract_kind TEXT NOT NULL CHECK (action_contract_kind IN ('single_action_v1','concurrent_action_group_v1')),
    action_kind TEXT NOT NULL CHECK (BTRIM(action_kind)<>''),
    canonical_request_hash TEXT NOT NULL CHECK (canonical_request_hash ~ '^sha256:[0-9a-f]{64}$'),
    display_projection JSONB NOT NULL CHECK (jsonb_typeof(display_projection)='object'),
    display_projection_hash TEXT NOT NULL CHECK (display_projection_hash ~ '^sha256:[0-9a-f]{64}$'),
    renderer_version TEXT NOT NULL CHECK (BTRIM(renderer_version)<>''),
    private_manifest JSONB NOT NULL CHECK (jsonb_typeof(private_manifest)='object'),
    private_manifest_hash TEXT NOT NULL CHECK (private_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    review_expires_at TIMESTAMPTZ NOT NULL,
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (BTRIM(target_type_at_time)<>''),
    target_value_at_time TEXT NOT NULL CHECK (BTRIM(target_value_at_time)<>''),
    target_identity_hash TEXT NOT NULL CHECK (target_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    credential_binding_hash TEXT CHECK (credential_binding_hash IS NULL OR credential_binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    policy_snapshot_hash TEXT NOT NULL CHECK (policy_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    upper_budget_set_hash TEXT NOT NULL CHECK (upper_budget_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    oracle_contract_hash TEXT NOT NULL CHECK (oracle_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    risk_tier TEXT NOT NULL CHECK (risk_tier IN ('T0','T1','T2','T3')),
    state TEXT NOT NULL CHECK (state IN (
        'pending_authorization','authorized','started','outcome_unknown',
        'compile_rejected','denied','expired','superseded','manually_blocked','succeeded','failed'
    )),
    reason_code TEXT,
    residual_id UUID,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    terminal_at TIMESTAMPTZ,
    CHECK (
        (state IN ('pending_authorization','authorized','started','outcome_unknown') AND terminal_at IS NULL)
        OR (state NOT IN ('pending_authorization','authorized','started','outcome_unknown')
            AND terminal_at IS NOT NULL AND BTRIM(reason_code)<>'')
    ),
    CHECK ((state IN ('compile_rejected','denied','expired','superseded','manually_blocked'))=(residual_id IS NOT NULL)),
    UNIQUE(campaign_id,action_ordinal),
    UNIQUE(prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(round_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_rounds(
            round_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(strategy_artifact_id) REFERENCES verification_strategy_artifacts(strategy_artifact_id) ON DELETE RESTRICT,
    FOREIGN KEY(capability_assessment_id) REFERENCES verification_capability_assessments(assessment_id) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX verification_prepared_actions_one_active_lane
ON verification_prepared_actions(campaign_id)
WHERE state IN ('pending_authorization','authorized','started','outcome_unknown');

CREATE FUNCTION verification_guard_prepared_action_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='UPDATE' AND OLD.target_live_id IS NOT NULL AND NEW.target_live_id IS NULL
       AND (to_jsonb(NEW)-'target_live_id') IS NOT DISTINCT FROM (to_jsonb(OLD)-'target_live_id')
    THEN
        RETURN NEW;
    END IF;
    IF TG_OP='DELETE' OR NEW.prepared_action_id<>OLD.prepared_action_id
       OR (to_jsonb(NEW)-ARRAY['state','reason_code','residual_id','row_version','terminal_at'])
          IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY['state','reason_code','residual_id','row_version','terminal_at'])
       OR NEW.row_version<>OLD.row_version+1
       OR NOT (
           (OLD.state='pending_authorization' AND NEW.state IN ('authorized','denied','expired','superseded','manually_blocked'))
           OR (OLD.state='authorized' AND NEW.state IN ('started','expired','superseded','manually_blocked'))
           OR (OLD.state='started' AND NEW.state IN ('succeeded','failed','outcome_unknown'))
           OR (OLD.state='outcome_unknown' AND NEW.state IN ('succeeded','failed','manually_blocked'))
       )
    THEN
        RAISE EXCEPTION 'VERIFICATION_PREPARED_ACTION_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_prepared_action_cas
BEFORE UPDATE OR DELETE ON verification_prepared_actions
FOR EACH ROW EXECUTE FUNCTION verification_guard_prepared_action_cas();

CREATE TABLE verification_prepared_action_group_members (
    group_member_id UUID PRIMARY KEY,
    prepared_action_id UUID NOT NULL REFERENCES verification_prepared_actions(prepared_action_id) ON DELETE RESTRICT,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    canonical_request_hash TEXT NOT NULL CHECK (canonical_request_hash ~ '^sha256:[0-9a-f]{64}$'),
    credential_session_binding_hash TEXT CHECK (
        credential_session_binding_hash IS NULL OR credential_session_binding_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    barrier_cohort_hash TEXT NOT NULL CHECK (barrier_cohort_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_start_window_ms BIGINT NOT NULL CHECK (expected_start_window_ms>0),
    upper_budget_hash TEXT NOT NULL CHECK (upper_budget_hash ~ '^sha256:[0-9a-f]{64}$'),
    oracle_role TEXT NOT NULL CHECK (BTRIM(oracle_role)<>''),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(group_member_id,prepared_action_id),
    UNIQUE(prepared_action_id,member_ordinal),
    UNIQUE(prepared_action_id,canonical_request_hash),
    UNIQUE(prepared_action_id,member_hash)
);

CREATE TABLE verification_action_conflict_sets (
    conflict_set_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    prepared_action_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ,
    UNIQUE(conflict_set_id,prepared_action_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_prepared_actions(
            prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_action_conflict_set_members (
    conflict_set_id UUID NOT NULL REFERENCES verification_action_conflict_sets(conflict_set_id) ON DELETE RESTRICT,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    key_kind TEXT NOT NULL CHECK (key_kind IN (
        'target_rate_limit','credential_session','resource','control_fixture'
    )),
    key_identity_hash TEXT NOT NULL CHECK (key_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    adapter_commutativity_authority_hash TEXT CHECK (
        adapter_commutativity_authority_hash IS NULL
        OR adapter_commutativity_authority_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(conflict_set_id,key_kind,key_identity_hash),
    UNIQUE(conflict_set_id,member_ordinal),
    UNIQUE(conflict_set_id,member_hash)
);

CREATE TABLE verification_conflict_key_heads (
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    key_kind TEXT NOT NULL CHECK (key_kind IN (
        'target_rate_limit','credential_session','resource','control_fixture'
    )),
    key_identity_hash TEXT NOT NULL CHECK (key_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    state TEXT NOT NULL CHECK (state IN ('free','active','recovery_hold')),
    owner_campaign_id UUID,
    owner_prepared_action_id UUID,
    fencing_token BIGINT NOT NULL DEFAULT 0 CHECK (fencing_token>=0),
    expires_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(operation_id,organization_id,key_kind,key_identity_hash),
    UNIQUE(operation_id,project_scope_id,organization_id,key_kind,key_identity_hash),
    CHECK (
        (state='free' AND owner_campaign_id IS NULL AND owner_prepared_action_id IS NULL AND expires_at IS NULL)
        OR (state<>'free' AND owner_campaign_id IS NOT NULL AND owner_prepared_action_id IS NOT NULL)
    ),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(owner_prepared_action_id,owner_campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_prepared_actions(
            prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE verification_conflict_key_events (
    conflict_event_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    key_kind TEXT NOT NULL,
    key_identity_hash TEXT NOT NULL,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal>0),
    event_kind TEXT NOT NULL CHECK (event_kind IN ('acquire','renew','recovery_hold','release')),
    expected_fencing_token BIGINT NOT NULL CHECK (expected_fencing_token>=0),
    new_fencing_token BIGINT NOT NULL CHECK (new_fencing_token>expected_fencing_token OR event_kind='release'),
    owner_campaign_id UUID NOT NULL,
    owner_prepared_action_id UUID NOT NULL,
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    residual_id UUID,
    event_hash TEXT NOT NULL CHECK (event_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,organization_id,key_kind,key_identity_hash,event_ordinal),
    FOREIGN KEY(operation_id,project_scope_id,organization_id,key_kind,key_identity_hash)
        REFERENCES verification_conflict_key_heads(
            operation_id,project_scope_id,organization_id,key_kind,key_identity_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(owner_prepared_action_id,owner_campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_prepared_actions(
            prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE FUNCTION verification_guard_conflict_key_head_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_event_kind TEXT;
BEGIN
    IF TG_OP='DELETE' OR ROW(NEW.operation_id,NEW.project_scope_id,NEW.organization_id,NEW.key_kind,NEW.key_identity_hash)
       IS DISTINCT FROM ROW(OLD.operation_id,OLD.project_scope_id,OLD.organization_id,OLD.key_kind,OLD.key_identity_hash)
       OR NEW.row_version<>OLD.row_version+1 OR NEW.fencing_token<OLD.fencing_token
    THEN
        RAISE EXCEPTION 'VERIFICATION_CONFLICT_KEY_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    expected_event_kind := CASE
        WHEN OLD.state='free' AND NEW.state='active' THEN 'acquire'
        WHEN OLD.state='active' AND NEW.state='active' THEN 'renew'
        WHEN OLD.state IN ('active','recovery_hold') AND NEW.state='recovery_hold' THEN 'recovery_hold'
        WHEN OLD.state IN ('active','recovery_hold') AND NEW.state='free' THEN 'release'
        ELSE NULL
    END;
    IF expected_event_kind IS NULL OR NOT EXISTS(
        SELECT 1 FROM verification_conflict_key_events event
         WHERE event.operation_id=NEW.operation_id AND event.organization_id=NEW.organization_id
           AND event.key_kind=NEW.key_kind AND event.key_identity_hash=NEW.key_identity_hash
           AND event.event_kind=expected_event_kind
           AND event.expected_fencing_token=OLD.fencing_token
           AND event.new_fencing_token=NEW.fencing_token
           AND event.owner_campaign_id=COALESCE(NEW.owner_campaign_id,OLD.owner_campaign_id)
           AND event.owner_prepared_action_id=COALESCE(NEW.owner_prepared_action_id,OLD.owner_prepared_action_id)
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_CONFLICT_KEY_EVENT_REQUIRED' USING ERRCODE='23514';
    END IF;
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_conflict_key_head_cas
AFTER UPDATE OR DELETE ON verification_conflict_key_heads
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_guard_conflict_key_head_cas();

CREATE TABLE verification_budget_contracts (
    budget_contract_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('operation','wave','campaign','action')),
    scope_id UUID NOT NULL,
    parent_contract_id UUID REFERENCES verification_budget_contracts(budget_contract_id) ON DELETE RESTRICT,
    contract_version TEXT NOT NULL CHECK (BTRIM(contract_version)<>''),
    contract_hash TEXT NOT NULL CHECK (contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ,
    CHECK ((scope_kind='operation')=(parent_contract_id IS NULL)),
    -- Operation ids span an engagement and may own more than one organization.
    -- Budget ancestry is organization-frozen, so the operation layer is one
    -- head per (operation, org), never one accidentally shared row.
    UNIQUE(scope_kind,scope_id,organization_id),
    UNIQUE(budget_contract_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE verification_budget_contract_axes (
    budget_contract_id UUID NOT NULL REFERENCES verification_budget_contracts(budget_contract_id) ON DELETE RESTRICT,
    axis_kind TEXT NOT NULL CHECK (axis_kind IN (
        'requests','response_bytes','wall_clock_ms','retries','browser_steps','oast_tokens'
    )),
    axis_ordinal INTEGER NOT NULL CHECK (axis_ordinal>=0),
    axis_limit BIGINT NOT NULL CHECK (axis_limit>=0),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(budget_contract_id,axis_kind),
    UNIQUE(budget_contract_id,axis_ordinal),
    UNIQUE(budget_contract_id,member_hash)
);

CREATE TABLE verification_budget_scope_heads (
    budget_contract_id UUID NOT NULL,
    axis_kind TEXT NOT NULL,
    consumed BIGINT NOT NULL DEFAULT 0 CHECK (consumed>=0),
    reserved BIGINT NOT NULL DEFAULT 0 CHECK (reserved>=0),
    unknown_held BIGINT NOT NULL DEFAULT 0 CHECK (unknown_held>=0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(budget_contract_id,axis_kind),
    FOREIGN KEY(budget_contract_id,axis_kind)
        REFERENCES verification_budget_contract_axes(budget_contract_id,axis_kind) ON DELETE RESTRICT
);

CREATE FUNCTION verification_validate_budget_contract_hierarchy()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    parent verification_budget_contracts%ROWTYPE;
    action verification_prepared_actions%ROWTYPE;
    campaign verification_campaigns%ROWTYPE;
    wave verification_wave_coverage_denominators%ROWTYPE;
    child_axis_count BIGINT;
    compatible_axis_count BIGINT;
BEGIN
    IF NEW.sealed_at IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT COUNT(*) INTO child_axis_count
      FROM verification_budget_contract_axes axis
     WHERE axis.budget_contract_id=NEW.budget_contract_id;
    IF NEW.scope_kind='operation' THEN
        IF NEW.parent_contract_id IS NOT NULL OR NEW.scope_id<>NEW.operation_id THEN
            RAISE EXCEPTION 'VERIFICATION_BUDGET_SCOPE_IDENTITY_INVALID' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    SELECT * INTO STRICT parent FROM verification_budget_contracts
     WHERE budget_contract_id=NEW.parent_contract_id FOR SHARE;
    IF parent.sealed_at IS NULL
       OR ROW(parent.operation_id,parent.project_scope_id,parent.organization_id)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.project_scope_id,NEW.organization_id)
    THEN
        RAISE EXCEPTION 'VERIFICATION_BUDGET_PARENT_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    IF NEW.scope_kind='wave' THEN
        SELECT * INTO STRICT wave FROM verification_wave_coverage_denominators
         WHERE wave_denominator_id=NEW.scope_id;
        IF parent.scope_kind<>'operation' OR parent.scope_id<>NEW.operation_id
           OR ROW(wave.operation_id,wave.project_scope_id,wave.organization_id)
              IS DISTINCT FROM ROW(NEW.operation_id,NEW.project_scope_id,NEW.organization_id)
        THEN
            RAISE EXCEPTION 'VERIFICATION_BUDGET_WAVE_PARENT_INVALID' USING ERRCODE='23514';
        END IF;
    ELSIF NEW.scope_kind='campaign' THEN
        SELECT * INTO STRICT campaign FROM verification_campaigns WHERE campaign_id=NEW.scope_id;
        IF parent.scope_kind<>'wave' OR parent.scope_id<>campaign.wave_denominator_id
           OR ROW(campaign.operation_id,campaign.project_scope_id,campaign.organization_id)
              IS DISTINCT FROM ROW(NEW.operation_id,NEW.project_scope_id,NEW.organization_id)
        THEN
            RAISE EXCEPTION 'VERIFICATION_BUDGET_CAMPAIGN_PARENT_INVALID' USING ERRCODE='23514';
        END IF;
    ELSIF NEW.scope_kind='action' THEN
        SELECT * INTO STRICT action FROM verification_prepared_actions
         WHERE prepared_action_id=NEW.scope_id;
        IF parent.scope_kind<>'campaign' OR parent.scope_id<>action.campaign_id
           OR ROW(action.operation_id,action.project_scope_id,action.organization_id)
              IS DISTINCT FROM ROW(NEW.operation_id,NEW.project_scope_id,NEW.organization_id)
        THEN
            RAISE EXCEPTION 'VERIFICATION_BUDGET_ACTION_PARENT_INVALID' USING ERRCODE='23514';
        END IF;
    END IF;
    SELECT COUNT(*) INTO compatible_axis_count
      FROM verification_budget_contract_axes child
      JOIN verification_budget_contract_axes ancestor
        ON ancestor.budget_contract_id=parent.budget_contract_id
       AND ancestor.axis_kind=child.axis_kind
       AND child.axis_limit<=ancestor.axis_limit
     WHERE child.budget_contract_id=NEW.budget_contract_id;
    IF compatible_axis_count<>child_axis_count OR child_axis_count<>parent.member_count THEN
        RAISE EXCEPTION 'VERIFICATION_BUDGET_AXIS_HIERARCHY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TABLE verification_prepared_action_authorizations (
    authorization_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    prepared_action_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('authorized','denied','expired','superseded','manually_blocked')),
    decision_reason_code TEXT NOT NULL CHECK (BTRIM(decision_reason_code)<>''),
    expected_action_row_version BIGINT NOT NULL CHECK (expected_action_row_version>=0),
    campaign_dispatch_generation BIGINT NOT NULL CHECK (campaign_dispatch_generation>=0),
    renderer_version TEXT NOT NULL CHECK (BTRIM(renderer_version)<>''),
    reviewed_action_hash TEXT NOT NULL CHECK (reviewed_action_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_display_projection_hash TEXT NOT NULL CHECK (expected_display_projection_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_private_manifest_hash TEXT NOT NULL CHECK (expected_private_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    authorization_hash TEXT NOT NULL CHECK (authorization_hash ~ '^sha256:[0-9a-f]{64}$'),
    decided_by UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    actor_kind TEXT NOT NULL DEFAULT 'local_operator' CHECK (actor_kind='local_operator'),
    operator_channel TEXT NOT NULL CHECK (operator_channel IN ('local_ui','local_cli','local_admin')),
    expires_at TIMESTAMPTZ,
    residual_id UUID,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((decision='authorized')=(residual_id IS NULL)),
    UNIQUE(authorization_receipt_id,prepared_action_id),
    FOREIGN KEY(prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_prepared_actions(
            prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX verification_prepared_action_one_authorization
ON verification_prepared_action_authorizations(prepared_action_id)
WHERE decision='authorized';

CREATE TABLE verification_budget_reservations (
    budget_reservation_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    prepared_action_id UUID NOT NULL,
    authorization_receipt_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    contract_set_hash TEXT NOT NULL CHECK (contract_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    upper_bound_membership_hash TEXT NOT NULL CHECK (upper_bound_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    state TEXT NOT NULL CHECK (state IN ('active','settled','unknown_held')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    settled_at TIMESTAMPTZ,
    CHECK ((state='active')=(settled_at IS NULL)),
    UNIQUE(prepared_action_id,authorization_receipt_id),
    UNIQUE(budget_reservation_id,prepared_action_id,authorization_receipt_id),
    FOREIGN KEY(prepared_action_id) REFERENCES verification_prepared_actions(prepared_action_id) ON DELETE RESTRICT,
    FOREIGN KEY(authorization_receipt_id,prepared_action_id)
        REFERENCES verification_prepared_action_authorizations(
            authorization_receipt_id,prepared_action_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE verification_action_executions (
    action_execution_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    prepared_action_id UUID NOT NULL,
    authorization_receipt_id UUID NOT NULL,
    budget_reservation_id UUID NOT NULL,
    conflict_set_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    execution_ordinal INTEGER NOT NULL CHECK (execution_ordinal>0),
    execution_kind TEXT NOT NULL CHECK (execution_kind IN ('single_action_v1','concurrent_action_group_v1')),
    state TEXT NOT NULL CHECK (state IN ('started','succeeded','failed','outcome_unknown')),
    campaign_dispatch_generation BIGINT NOT NULL CHECK (campaign_dispatch_generation>=0),
    durable_begin_hash TEXT NOT NULL CHECK (durable_begin_hash ~ '^sha256:[0-9a-f]{64}$'),
    capability_execution_receipt_id UUID,
    closeout_hash TEXT CHECK (closeout_hash IS NULL OR closeout_hash ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    started_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    completed_at TIMESTAMPTZ,
    CHECK ((state='started')=(completed_at IS NULL)),
    CHECK ((state='started')=(closeout_hash IS NULL)),
    CHECK (state='started' OR capability_execution_receipt_id IS NOT NULL),
    UNIQUE(prepared_action_id,authorization_receipt_id,execution_ordinal),
    UNIQUE(action_execution_id,prepared_action_id),
    UNIQUE(action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(prepared_action_id) REFERENCES verification_prepared_actions(prepared_action_id) ON DELETE RESTRICT,
    FOREIGN KEY(authorization_receipt_id,prepared_action_id)
        REFERENCES verification_prepared_action_authorizations(
            authorization_receipt_id,prepared_action_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(budget_reservation_id,prepared_action_id,authorization_receipt_id)
        REFERENCES verification_budget_reservations(
            budget_reservation_id,prepared_action_id,authorization_receipt_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(conflict_set_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_action_conflict_sets(
            conflict_set_id,prepared_action_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(capability_execution_receipt_id)
        REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX verification_action_executions_one_per_ordinal
ON verification_action_executions(prepared_action_id,authorization_receipt_id,execution_ordinal);

-- Plan C network work still produces a Plan A Tool Truth receipt.  This
-- append-only bridge binds the derived-child denominator/receipt to the exact
-- durable action execution and records whether the host preserved a complete
-- raw witness.  V1 deliberately marks metadata-only observations partial;
-- deterministic oracles must therefore return inconclusive rather than turn a
-- successful HTTP exchange into proof.
CREATE TABLE verification_action_capability_receipt_bindings (
    binding_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    action_execution_id UUID NOT NULL UNIQUE,
    prepared_action_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    capability_execution_receipt_id UUID NOT NULL UNIQUE,
    derived_denominator_id UUID NOT NULL UNIQUE,
    parent_denominator_id UUID NOT NULL,
    parent_denominator_item_id UUID NOT NULL,
    execution_authority_id UUID NOT NULL,
    binding_hash TEXT NOT NULL CHECK (binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(binding_id,action_execution_id,prepared_action_id),
    FOREIGN KEY(action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_action_executions(
            action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(capability_execution_receipt_id,derived_denominator_id,execution_authority_id)
        REFERENCES capability_execution_receipts(id,denominator_id,execution_authority_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(parent_denominator_item_id,parent_denominator_id,execution_authority_id)
        REFERENCES coverage_denominator_items(id,denominator_id,execution_authority_id)
        ON DELETE RESTRICT
);

CREATE TABLE verification_action_capability_receipt_finalizations (
    finalization_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    binding_id UUID NOT NULL UNIQUE,
    action_execution_id UUID NOT NULL UNIQUE,
    prepared_action_id UUID NOT NULL,
    capability_execution_receipt_id UUID NOT NULL UNIQUE,
    terminal_state TEXT NOT NULL CHECK (terminal_state IN ('succeeded','failed','outcome_unknown')),
    witness_completeness TEXT NOT NULL CHECK (witness_completeness IN ('complete_raw','metadata_only','unknown')),
    observation_hash TEXT NOT NULL CHECK (observation_hash ~ '^sha256:[0-9a-f]{64}$'),
    finalization_hash TEXT NOT NULL CHECK (finalization_hash ~ '^sha256:[0-9a-f]{64}$'),
    finalized_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(binding_id,action_execution_id,prepared_action_id)
        REFERENCES verification_action_capability_receipt_bindings(
            binding_id,action_execution_id,prepared_action_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(capability_execution_receipt_id)
        REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT
);

-- An outcome-unknown closeout is deliberately not the final word.  Recovery
-- is a second, immutable authority event: it records who/what reconciled the
-- durable execution, how the unknown budget hold was settled, and which
-- conflict leases were released.  The execution/action heads may only leave
-- outcome_unknown when a matching receipt already exists in the transaction.
CREATE TABLE verification_action_recovery_receipts (
    recovery_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    action_execution_id UUID NOT NULL,
    prepared_action_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    recovery_disposition TEXT NOT NULL CHECK (recovery_disposition IN (
        'outcome_unknown','reconciled_succeeded','reconciled_failed','manually_blocked'
    )),
    execution_result_state TEXT NOT NULL CHECK (execution_result_state IN (
        'outcome_unknown','succeeded','failed'
    )),
    budget_settlement_kind TEXT NOT NULL CHECK (budget_settlement_kind IN (
        'retain_unknown_hold','consume_unknown_hold','release_unknown_hold'
    )),
    prior_closeout_hash TEXT NOT NULL CHECK (prior_closeout_hash ~ '^sha256:[0-9a-f]{64}$'),
    recovery_hash TEXT NOT NULL CHECK (recovery_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_id UUID REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    recovered_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(action_execution_id,recovery_disposition),
    UNIQUE(recovery_receipt_id,action_execution_id,prepared_action_id),
    FOREIGN KEY(action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_action_executions(
            action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    CHECK ((recovery_disposition='manually_blocked')=(residual_id IS NOT NULL)),
    CHECK (
        (recovery_disposition='reconciled_succeeded' AND execution_result_state='succeeded')
        OR (recovery_disposition='reconciled_failed' AND execution_result_state='failed')
        OR (recovery_disposition IN ('outcome_unknown','manually_blocked')
            AND execution_result_state='outcome_unknown')
    )
);

CREATE TABLE verification_action_subexecutions (
    action_subexecution_id UUID PRIMARY KEY,
    action_execution_id UUID NOT NULL,
    prepared_action_id UUID NOT NULL,
    group_member_id UUID NOT NULL,
    subexecution_ordinal INTEGER NOT NULL CHECK (subexecution_ordinal>=0),
    state TEXT NOT NULL CHECK (state IN ('succeeded','failed','outcome_unknown')),
    capability_execution_receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    barrier_released_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ NOT NULL,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (started_at>=barrier_released_at),
    UNIQUE(action_execution_id,subexecution_ordinal),
    UNIQUE(action_execution_id,group_member_id),
    UNIQUE(action_execution_id,member_hash),
    FOREIGN KEY(action_execution_id,prepared_action_id)
        REFERENCES verification_action_executions(action_execution_id,prepared_action_id) ON DELETE RESTRICT,
    FOREIGN KEY(group_member_id,prepared_action_id)
        REFERENCES verification_prepared_action_group_members(group_member_id,prepared_action_id) ON DELETE RESTRICT
);

CREATE TABLE verification_budget_ledger_entries (
    budget_ledger_entry_id UUID PRIMARY KEY,
    budget_reservation_id UUID NOT NULL REFERENCES verification_budget_reservations(budget_reservation_id) ON DELETE RESTRICT,
    ancestor_contract_id UUID NOT NULL,
    axis_kind TEXT NOT NULL,
    entry_ordinal BIGINT NOT NULL CHECK (entry_ordinal>0),
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('reserve','consume','settle','hold_unknown','release')),
    delta BIGINT NOT NULL,
    resulting_consumed BIGINT NOT NULL CHECK (resulting_consumed>=0),
    resulting_reserved BIGINT NOT NULL CHECK (resulting_reserved>=0),
    resulting_unknown_held BIGINT NOT NULL CHECK (resulting_unknown_held>=0),
    expected_head_row_version BIGINT NOT NULL CHECK (expected_head_row_version>=0),
    resulting_head_hash TEXT NOT NULL CHECK (resulting_head_hash ~ '^sha256:[0-9a-f]{64}$'),
    fence BIGINT NOT NULL CHECK (fence>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(budget_reservation_id,ancestor_contract_id,axis_kind,entry_ordinal),
    FOREIGN KEY(ancestor_contract_id,axis_kind)
        REFERENCES verification_budget_scope_heads(budget_contract_id,axis_kind) ON DELETE RESTRICT
);

CREATE FUNCTION verification_guard_budget_head_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE' OR ROW(NEW.budget_contract_id,NEW.axis_kind)
       IS DISTINCT FROM ROW(OLD.budget_contract_id,OLD.axis_kind)
       OR NEW.row_version<>OLD.row_version+1
       OR NOT EXISTS(
           SELECT 1 FROM verification_budget_ledger_entries entry
            WHERE entry.ancestor_contract_id=NEW.budget_contract_id
              AND entry.axis_kind=NEW.axis_kind
              AND entry.expected_head_row_version=OLD.row_version
              AND ROW(entry.resulting_consumed,entry.resulting_reserved,entry.resulting_unknown_held)
                  = ROW(NEW.consumed,NEW.reserved,NEW.unknown_held)
       )
    THEN
        RAISE EXCEPTION 'VERIFICATION_BUDGET_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_budget_scope_head_cas
AFTER UPDATE OR DELETE ON verification_budget_scope_heads
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_guard_budget_head_cas();

CREATE TABLE verification_cleanup_obligations (
    cleanup_obligation_id UUID PRIMARY KEY,
    action_execution_id UUID NOT NULL REFERENCES verification_action_executions(action_execution_id) ON DELETE RESTRICT,
    obligation_ordinal INTEGER NOT NULL CHECK (obligation_ordinal>=0),
    obligation_kind TEXT NOT NULL CHECK (BTRIM(obligation_kind)<>''),
    status TEXT NOT NULL CHECK (status IN ('pending','completed','failed','outcome_unknown')),
    residual_id UUID REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    obligation_hash TEXT NOT NULL CHECK (obligation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(action_execution_id,obligation_ordinal),
    CHECK ((status='completed')=(residual_id IS NULL))
);

CREATE TABLE verification_oracle_assessments (
    oracle_assessment_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL,
    prepared_action_id UUID NOT NULL,
    action_execution_id UUID NOT NULL,
    campaign_coverage_member_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    oracle_revision_ordinal INTEGER NOT NULL CHECK (oracle_revision_ordinal>0),
    oracle_contract_version TEXT NOT NULL CHECK (BTRIM(oracle_contract_version)<>''),
    oracle_contract_hash TEXT NOT NULL CHECK (oracle_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    observation_receipt_hash TEXT NOT NULL CHECK (observation_receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    precondition_validity TEXT NOT NULL CHECK (precondition_validity IN ('valid','invalid','unknown')),
    control_validity TEXT NOT NULL CHECK (control_validity IN ('valid','invalid','not_assessed','not_required')),
    verdict TEXT NOT NULL CHECK (verdict IN ('proof','refutation','inconclusive')),
    assessment_body JSONB NOT NULL CHECK (jsonb_typeof(assessment_body)='object'),
    assessment_hash TEXT NOT NULL CHECK (assessment_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_id UUID,
    assessed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (verdict<>'inconclusive' OR residual_id IS NOT NULL),
    CHECK (verdict='inconclusive' OR (precondition_validity='valid' AND control_validity IN ('valid','not_required'))),
    UNIQUE(prepared_action_id,oracle_revision_ordinal),
    UNIQUE(oracle_assessment_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_prepared_actions(
            prepared_action_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_action_executions(
            action_execution_id,prepared_action_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_coverage_member_id)
        REFERENCES verification_campaign_coverage_members(campaign_coverage_member_id) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE FUNCTION verification_guard_oracle_assessment_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    wave_capability_assessment_id UUID;
    wave_control_binding_kind TEXT;
    action_assessment_id UUID;
BEGIN
    SELECT wave.capability_assessment_id,wave.control_binding_kind,
           action.capability_assessment_id
      INTO STRICT wave_capability_assessment_id,wave_control_binding_kind,
                  action_assessment_id
      FROM verification_prepared_actions action
      JOIN verification_campaign_coverage_denominators denominator
        ON denominator.campaign_id=action.campaign_id
      JOIN verification_campaign_coverage_members member
        ON member.campaign_denominator_id=denominator.campaign_denominator_id
       AND member.campaign_coverage_member_id=NEW.campaign_coverage_member_id
      JOIN verification_wave_coverage_members wave
        ON wave.wave_coverage_member_id=member.wave_coverage_member_id
       AND wave.wave_denominator_id=member.wave_denominator_id
     WHERE action.prepared_action_id=NEW.prepared_action_id
       AND action.campaign_id=NEW.campaign_id
       AND action.operation_id=NEW.operation_id
       AND action.project_scope_id=NEW.project_scope_id
       AND action.organization_id=NEW.organization_id;
    IF action_assessment_id<>wave_capability_assessment_id
       OR (wave_control_binding_kind='explicit_no_control'
           AND NEW.control_validity<>'not_required')
       OR (wave_control_binding_kind='required'
           AND NEW.control_validity='not_required')
       OR (NEW.residual_id IS NOT NULL AND NOT EXISTS(
           SELECT 1 FROM hypothesis_residual_risks residual
            WHERE residual.residual_id=NEW.residual_id
              AND residual.operation_id=NEW.operation_id
              AND residual.organization_id=NEW.organization_id
       ))
    THEN
        RAISE EXCEPTION 'VERIFICATION_ORACLE_ASSESSMENT_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_oracle_assessment_authority_guard
BEFORE INSERT ON verification_oracle_assessments
FOR EACH ROW EXECUTE FUNCTION verification_guard_oracle_assessment_authority();

CREATE TABLE verification_oracle_census_seals (
    oracle_census_seal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL,
    campaign_denominator_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    verification_contract_hash TEXT NOT NULL CHECK (verification_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    denominator_hash TEXT NOT NULL CHECK (denominator_hash ~ '^sha256:[0-9a-f]{64}$'),
    result_set_hash TEXT NOT NULL CHECK (result_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    census_hash TEXT NOT NULL CHECK (census_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ,
    UNIQUE(oracle_census_seal_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_denominator_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_coverage_denominators(
            campaign_denominator_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_oracle_census_members (
    oracle_census_member_id UUID PRIMARY KEY,
    oracle_census_seal_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    campaign_coverage_member_id UUID NOT NULL,
    predicate_component_id UUID NOT NULL,
    control_binding_kind TEXT NOT NULL CHECK (control_binding_kind IN ('required','explicit_no_control')),
    required_control_id UUID,
    required_control_hash TEXT CHECK (required_control_hash IS NULL OR required_control_hash ~ '^sha256:[0-9a-f]{64}$'),
    no_control_marker_hash TEXT CHECK (no_control_marker_hash IS NULL OR no_control_marker_hash ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT NOT NULL CHECK (disposition IN ('assessed','untested','blocked')),
    oracle_assessment_id UUID,
    residual_id UUID,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (
        (control_binding_kind='required' AND required_control_id IS NOT NULL
            AND required_control_hash IS NOT NULL AND no_control_marker_hash IS NULL)
        OR (control_binding_kind='explicit_no_control' AND required_control_id IS NULL
            AND required_control_hash IS NULL AND no_control_marker_hash IS NOT NULL)
    ),
    CHECK (
        (disposition='assessed' AND oracle_assessment_id IS NOT NULL AND residual_id IS NULL)
        OR (disposition IN ('untested','blocked') AND oracle_assessment_id IS NULL AND residual_id IS NOT NULL)
    ),
    UNIQUE(oracle_census_seal_id,member_ordinal),
    UNIQUE(oracle_census_seal_id,campaign_coverage_member_id),
    UNIQUE(oracle_census_seal_id,member_hash),
    FOREIGN KEY(oracle_census_seal_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_oracle_census_seals(
            oracle_census_seal_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_coverage_member_id)
        REFERENCES verification_campaign_coverage_members(campaign_coverage_member_id) ON DELETE RESTRICT,
    FOREIGN KEY(oracle_assessment_id)
        REFERENCES verification_oracle_assessments(oracle_assessment_id) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE FUNCTION verification_validate_oracle_census_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    denominator_count BIGINT;
    actual_count BIGINT;
    invalid_count BIGINT;
BEGIN
    IF NEW.sealed_at IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT member_count INTO STRICT denominator_count
      FROM verification_campaign_coverage_denominators
     WHERE campaign_denominator_id=NEW.campaign_denominator_id;
    SELECT COUNT(*),COUNT(*) FILTER(WHERE
               wave.predicate_component_id<>member.predicate_component_id
               OR wave.control_binding_kind<>member.control_binding_kind
               OR (wave.control_binding_kind='required' AND (
                    wave.required_control_id IS DISTINCT FROM member.required_control_id
                    OR wave.required_control_hash IS DISTINCT FROM member.required_control_hash
               ))
               OR (wave.control_binding_kind='explicit_no_control'
                    AND wave.no_control_marker_hash IS DISTINCT FROM member.no_control_marker_hash)
               OR (member.disposition='assessed' AND NOT EXISTS(
                    SELECT 1 FROM verification_oracle_assessments oracle
                     WHERE oracle.oracle_assessment_id=member.oracle_assessment_id
                       AND oracle.campaign_id=NEW.campaign_id
                       AND oracle.campaign_coverage_member_id=member.campaign_coverage_member_id
               ))
               OR (member.residual_id IS NOT NULL AND NOT EXISTS(
                    SELECT 1 FROM hypothesis_residual_risks residual
                     WHERE residual.residual_id=member.residual_id
                       AND residual.operation_id=NEW.operation_id
                       AND residual.organization_id=NEW.organization_id
               )))
      INTO actual_count,invalid_count
      FROM verification_oracle_census_members member
      JOIN verification_campaign_coverage_members campaign_member
        ON campaign_member.campaign_coverage_member_id=member.campaign_coverage_member_id
       AND campaign_member.campaign_denominator_id=NEW.campaign_denominator_id
      JOIN verification_wave_coverage_members wave
        ON wave.wave_coverage_member_id=campaign_member.wave_coverage_member_id
       AND wave.wave_denominator_id=campaign_member.wave_denominator_id
     WHERE member.oracle_census_seal_id=NEW.oracle_census_seal_id;
    IF actual_count<>denominator_count OR invalid_count<>0 THEN
        RAISE EXCEPTION 'VERIFICATION_ORACLE_CENSUS_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TABLE verification_campaign_adjudications (
    campaign_adjudication_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL UNIQUE,
    oracle_census_seal_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    verification_contract_hash TEXT NOT NULL CHECK (verification_contract_hash ~ '^sha256:[0-9a-f]{64}$'),
    oracle_census_hash TEXT NOT NULL CHECK (oracle_census_hash ~ '^sha256:[0-9a-f]{64}$'),
    outcome TEXT NOT NULL CHECK (outcome IN ('proof','refutation','inconclusive','blocked','exhausted_with_residuals')),
    unresolved_member_set_hash TEXT CHECK (
        unresolved_member_set_hash IS NULL OR unresolved_member_set_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    adjudication_hash TEXT NOT NULL CHECK (adjudication_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((outcome IN ('proof','refutation'))=(residual_id IS NULL)),
    UNIQUE(campaign_adjudication_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(oracle_census_seal_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_oracle_census_seals(
            oracle_census_seal_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE TABLE verification_campaign_terminal_decisions (
    campaign_terminal_decision_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL UNIQUE,
    campaign_adjudication_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    terminal_decision TEXT NOT NULL CHECK (terminal_decision IN (
        'proof','refutation','inconclusive','blocked','exhausted_with_residuals'
    )),
    terminal_hash TEXT NOT NULL CHECK (terminal_hash ~ '^sha256:[0-9a-f]{64}$'),
    terminal_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(campaign_terminal_decision_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_adjudication_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_adjudications(
            campaign_adjudication_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_campaign_coverage_receipts (
    campaign_coverage_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL UNIQUE,
    campaign_terminal_decision_id UUID NOT NULL UNIQUE,
    campaign_denominator_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    denominator_hash TEXT NOT NULL CHECK (denominator_hash ~ '^sha256:[0-9a-f]{64}$'),
    result_membership_hash TEXT NOT NULL CHECK (result_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_membership_hash TEXT NOT NULL CHECK (residual_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    tested_complete_count BIGINT NOT NULL CHECK (tested_complete_count>=0),
    tested_degraded_count BIGINT NOT NULL CHECK (tested_degraded_count>=0),
    untested_count BIGINT NOT NULL CHECK (untested_count>=0),
    blocked_count BIGINT NOT NULL CHECK (blocked_count>=0),
    coverage_status TEXT NOT NULL CHECK (coverage_status IN ('complete','partial','invalid')),
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(campaign_coverage_receipt_id,campaign_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_terminal_decision_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_terminal_decisions(
            campaign_terminal_decision_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_denominator_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_coverage_denominators(
            campaign_denominator_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_campaign_coverage_results (
    campaign_coverage_receipt_id UUID NOT NULL,
    campaign_coverage_member_id UUID NOT NULL,
    coverage_disposition TEXT NOT NULL CHECK (coverage_disposition IN (
        'tested_complete','tested_degraded','untested','blocked'
    )),
    epistemic_outcome TEXT NOT NULL CHECK (epistemic_outcome IN (
        'proof','refutation','inconclusive','not_assessed'
    )),
    control_binding_kind TEXT NOT NULL CHECK (control_binding_kind IN ('required','explicit_no_control')),
    control_validity TEXT NOT NULL CHECK (control_validity IN ('valid','invalid','not_assessed','not_required')),
    prepared_action_id UUID,
    capability_execution_receipt_id UUID,
    oracle_assessment_id UUID,
    residual_id UUID,
    result_hash TEXT NOT NULL CHECK (result_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(campaign_coverage_receipt_id,campaign_coverage_member_id),
    CONSTRAINT verification_campaign_coverage_result_shape_check CHECK (
        (
            (coverage_disposition='tested_complete'
                AND prepared_action_id IS NOT NULL AND capability_execution_receipt_id IS NOT NULL
                AND oracle_assessment_id IS NOT NULL AND residual_id IS NULL)
            OR (coverage_disposition='tested_degraded'
                AND prepared_action_id IS NOT NULL AND capability_execution_receipt_id IS NOT NULL
                AND oracle_assessment_id IS NOT NULL AND residual_id IS NOT NULL)
            OR (coverage_disposition IN ('untested','blocked')
                AND prepared_action_id IS NULL AND capability_execution_receipt_id IS NULL
                AND oracle_assessment_id IS NULL AND residual_id IS NOT NULL)
        )
        AND (
            (control_binding_kind='explicit_no_control' AND control_validity='not_required')
            OR (control_binding_kind='required' AND control_validity<>'not_required')
        )
    ),
    CONSTRAINT verification_campaign_coverage_control_shape_check CHECK (
        (control_binding_kind='explicit_no_control' AND control_validity='not_required')
        OR (control_binding_kind='required' AND control_validity<>'not_required')
    ),
    CHECK (
        (coverage_disposition IN ('tested_complete','tested_degraded') AND epistemic_outcome<>'not_assessed')
        OR (coverage_disposition IN ('untested','blocked') AND epistemic_outcome='not_assessed')
    ),
    FOREIGN KEY(campaign_coverage_receipt_id)
        REFERENCES verification_campaign_coverage_receipts(campaign_coverage_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_coverage_member_id)
        REFERENCES verification_campaign_coverage_members(campaign_coverage_member_id) ON DELETE RESTRICT,
    FOREIGN KEY(prepared_action_id) REFERENCES verification_prepared_actions(prepared_action_id) ON DELETE RESTRICT,
    FOREIGN KEY(capability_execution_receipt_id) REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    FOREIGN KEY(oracle_assessment_id) REFERENCES verification_oracle_assessments(oracle_assessment_id) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE FUNCTION verification_guard_campaign_coverage_result_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    receipt verification_campaign_coverage_receipts%ROWTYPE;
    wave_member verification_wave_coverage_members%ROWTYPE;
BEGIN
    SELECT * INTO STRICT receipt FROM verification_campaign_coverage_receipts
     WHERE campaign_coverage_receipt_id=NEW.campaign_coverage_receipt_id FOR SHARE;
    SELECT wave.* INTO STRICT wave_member
      FROM verification_campaign_coverage_members member
      JOIN verification_wave_coverage_members wave
        ON wave.wave_coverage_member_id=member.wave_coverage_member_id
       AND wave.wave_denominator_id=member.wave_denominator_id
     WHERE member.campaign_coverage_member_id=NEW.campaign_coverage_member_id
       AND member.campaign_denominator_id=receipt.campaign_denominator_id;
    IF NEW.control_binding_kind<>wave_member.control_binding_kind
       OR (NEW.residual_id IS NOT NULL AND NOT EXISTS(
           SELECT 1 FROM hypothesis_residual_risks residual
            WHERE residual.residual_id=NEW.residual_id
              AND residual.operation_id=receipt.operation_id
              AND residual.organization_id=receipt.organization_id
       ))
       OR (NEW.prepared_action_id IS NOT NULL AND NOT EXISTS(
           SELECT 1
             FROM verification_oracle_assessments oracle
             JOIN verification_action_executions execution
               ON execution.action_execution_id=oracle.action_execution_id
              AND execution.prepared_action_id=oracle.prepared_action_id
            WHERE oracle.oracle_assessment_id=NEW.oracle_assessment_id
              AND oracle.campaign_id=receipt.campaign_id
              AND oracle.campaign_coverage_member_id=NEW.campaign_coverage_member_id
              AND oracle.prepared_action_id=NEW.prepared_action_id
              AND oracle.verdict=NEW.epistemic_outcome
              AND oracle.control_validity=NEW.control_validity
              AND execution.capability_execution_receipt_id=NEW.capability_execution_receipt_id
       ))
    THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_COVERAGE_RESULT_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_campaign_coverage_result_authority_guard
BEFORE INSERT ON verification_campaign_coverage_results
FOR EACH ROW EXECUTE FUNCTION verification_guard_campaign_coverage_result_authority();

CREATE TABLE verification_wave_coverage_receipts (
    wave_coverage_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    wave_denominator_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    selected_campaign_receipt_set_hash TEXT NOT NULL CHECK (selected_campaign_receipt_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    unassigned_result_set_hash TEXT NOT NULL CHECK (unassigned_result_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    result_member_count BIGINT NOT NULL CHECK (result_member_count>0),
    result_member_set_hash TEXT NOT NULL CHECK (result_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    coverage_status TEXT NOT NULL CHECK (coverage_status IN ('complete','partial','invalid')),
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(wave_coverage_receipt_id,wave_denominator_id),
    FOREIGN KEY(wave_denominator_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_wave_coverage_denominators(
            wave_denominator_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_wave_unassigned_coverage_results (
    wave_coverage_receipt_id UUID NOT NULL,
    wave_coverage_member_id UUID NOT NULL,
    residual_id UUID NOT NULL REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN ('untested','blocked')),
    result_hash TEXT NOT NULL CHECK (result_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(wave_coverage_receipt_id,wave_coverage_member_id),
    FOREIGN KEY(wave_coverage_receipt_id)
        REFERENCES verification_wave_coverage_receipts(wave_coverage_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(wave_coverage_member_id)
        REFERENCES verification_wave_coverage_members(wave_coverage_member_id) ON DELETE RESTRICT
);

CREATE TABLE verification_fact_delta_bundles (
    fact_delta_bundle_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    campaign_id UUID NOT NULL UNIQUE,
    campaign_terminal_decision_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    delta_kind TEXT NOT NULL CHECK (delta_kind IN ('support','contradiction','inconclusive','no_change','retraction')),
    typed_delta JSONB NOT NULL CHECK (jsonb_typeof(typed_delta)='object'),
    evidence_ref_set_hash TEXT NOT NULL CHECK (evidence_ref_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_authority_hash TEXT NOT NULL CHECK (source_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    fact_delta_hash TEXT NOT NULL CHECK (fact_delta_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(fact_delta_bundle_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaigns(campaign_id,operation_id,project_scope_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_terminal_decision_id,campaign_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_terminal_decisions(
            campaign_terminal_decision_id,campaign_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX verification_fact_delta_one_per_terminal
ON verification_fact_delta_bundles(campaign_terminal_decision_id);

CREATE TABLE hypothesis_objective_claim_component_outcome_seals (
    claim_component_outcome_seal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    verification_plan_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    campaign_id UUID,
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    seal_hash TEXT NOT NULL CHECK (seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ,
    UNIQUE(claim_component_outcome_seal_id,verification_plan_id,verification_objective_id),
    FOREIGN KEY(verification_plan_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_id) REFERENCES verification_campaigns(campaign_id) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_objective_claim_component_outcome_members (
    claim_component_outcome_member_id UUID PRIMARY KEY,
    claim_component_outcome_seal_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    claim_component_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    claim_component_hash TEXT NOT NULL CHECK (claim_component_hash ~ '^sha256:[0-9a-f]{64}$'),
    predicate_component_id UUID NOT NULL,
    oracle_census_member_id UUID,
    campaign_coverage_member_id UUID,
    component_outcome TEXT NOT NULL CHECK (component_outcome IN (
        'proof','refutation','inconclusive','blocked','unassigned','invalidated'
    )),
    residual_id UUID,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (
        (component_outcome IN ('proof','refutation') AND oracle_census_member_id IS NOT NULL
            AND campaign_coverage_member_id IS NOT NULL AND residual_id IS NULL)
        OR (component_outcome IN ('inconclusive','blocked','unassigned','invalidated') AND residual_id IS NOT NULL)
    ),
    UNIQUE(claim_component_outcome_seal_id,member_ordinal),
    UNIQUE(claim_component_outcome_seal_id,claim_component_id),
    UNIQUE(claim_component_outcome_seal_id,member_hash),
    FOREIGN KEY(claim_component_outcome_seal_id,verification_plan_id,verification_objective_id)
        REFERENCES hypothesis_objective_claim_component_outcome_seals(
            claim_component_outcome_seal_id,verification_plan_id,verification_objective_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(claim_component_id,hypothesis_revision_id,claim_component_hash)
        REFERENCES attack_hypothesis_claim_components(
            component_id,revision_id,member_hash
        ) ON DELETE RESTRICT,
    FOREIGN KEY(oracle_census_member_id)
        REFERENCES verification_oracle_census_members(oracle_census_member_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_coverage_member_id)
        REFERENCES verification_campaign_coverage_members(campaign_coverage_member_id) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE FUNCTION verification_validate_claim_component_outcome_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    contract_id UUID;
    expected_count BIGINT;
    actual_count BIGINT;
    invalid_count BIGINT;
BEGIN
    IF NEW.sealed_at IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT objective.verification_contract_id,objective.claim_component_count
      INTO STRICT contract_id,expected_count
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
        ON binding.contract_id=contract_id
       AND binding.revision_id=member.hypothesis_revision_id
       AND binding.claim_component_id=member.claim_component_id
       AND binding.component_member_hash=member.claim_component_hash
      LEFT JOIN attack_hypothesis_verification_predicate_components predicate
        ON predicate.predicate_component_id=member.predicate_component_id
       AND predicate.contract_id=contract_id
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

CREATE TABLE hypothesis_objective_outcome_receipts (
    objective_outcome_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    verification_plan_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    outcome_ordinal BIGINT NOT NULL CHECK (outcome_ordinal>0),
    predecessor_outcome_id UUID,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'proof','refutation','inconclusive','blocked','exhausted_with_residuals','unassigned','invalidated'
    )),
    campaign_terminal_decision_id UUID,
    campaign_adjudication_id UUID,
    campaign_coverage_receipt_id UUID,
    oracle_census_seal_id UUID,
    claim_component_outcome_seal_id UUID NOT NULL,
    claim_component_outcome_seal_hash TEXT NOT NULL CHECK (claim_component_outcome_seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    fact_delta_bundle_id UUID,
    residual_id UUID,
    source_authority_hash TEXT NOT NULL CHECK (source_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    outcome_hash TEXT NOT NULL CHECK (outcome_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((outcome_ordinal=1)=(predecessor_outcome_id IS NULL)),
    CHECK (
        (outcome IN ('proof','refutation','inconclusive','blocked','exhausted_with_residuals')
            AND campaign_terminal_decision_id IS NOT NULL AND campaign_adjudication_id IS NOT NULL
            AND campaign_coverage_receipt_id IS NOT NULL AND oracle_census_seal_id IS NOT NULL
            AND fact_delta_bundle_id IS NOT NULL)
        OR (outcome='unassigned' AND campaign_terminal_decision_id IS NULL
            AND campaign_adjudication_id IS NULL AND campaign_coverage_receipt_id IS NULL
            AND oracle_census_seal_id IS NULL AND fact_delta_bundle_id IS NULL AND residual_id IS NOT NULL)
        OR (outcome='invalidated' AND predecessor_outcome_id IS NOT NULL AND residual_id IS NOT NULL)
    ),
    CHECK ((outcome IN ('proof','refutation'))=(residual_id IS NULL)),
    UNIQUE(verification_plan_id,verification_objective_id,outcome_ordinal),
    UNIQUE(objective_outcome_receipt_id,verification_plan_id,verification_objective_id,outcome_ordinal),
    UNIQUE(objective_outcome_receipt_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(verification_plan_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(verification_objective_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_objectives(objective_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_outcome_id)
        REFERENCES hypothesis_objective_outcome_receipts(objective_outcome_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_terminal_decision_id)
        REFERENCES verification_campaign_terminal_decisions(campaign_terminal_decision_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_adjudication_id)
        REFERENCES verification_campaign_adjudications(campaign_adjudication_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_coverage_receipt_id)
        REFERENCES verification_campaign_coverage_receipts(campaign_coverage_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(oracle_census_seal_id)
        REFERENCES verification_oracle_census_seals(oracle_census_seal_id) ON DELETE RESTRICT,
    FOREIGN KEY(claim_component_outcome_seal_id)
        REFERENCES hypothesis_objective_claim_component_outcome_seals(claim_component_outcome_seal_id) ON DELETE RESTRICT,
    FOREIGN KEY(fact_delta_bundle_id)
        REFERENCES verification_fact_delta_bundles(fact_delta_bundle_id) ON DELETE RESTRICT,
    FOREIGN KEY(residual_id) REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_objective_outcome_heads (
    verification_plan_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    current_outcome_id UUID NOT NULL,
    current_ordinal BIGINT NOT NULL CHECK (current_ordinal>0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY(verification_plan_id,verification_objective_id),
    FOREIGN KEY(current_outcome_id,verification_plan_id,verification_objective_id,current_ordinal)
        REFERENCES hypothesis_objective_outcome_receipts(
            objective_outcome_receipt_id,verification_plan_id,verification_objective_id,outcome_ordinal
        ) ON DELETE RESTRICT
);

CREATE FUNCTION verification_guard_objective_outcome_head_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    successor hypothesis_objective_outcome_receipts%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'VERIFICATION_OBJECTIVE_OUTCOME_HEAD_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT successor FROM hypothesis_objective_outcome_receipts
     WHERE objective_outcome_receipt_id=NEW.current_outcome_id
       AND verification_plan_id=NEW.verification_plan_id
       AND verification_objective_id=NEW.verification_objective_id FOR SHARE;
    IF TG_OP='INSERT' THEN
        IF NEW.row_version<>0 OR NEW.current_ordinal<>1 OR successor.predecessor_outcome_id IS NOT NULL THEN
            RAISE EXCEPTION 'VERIFICATION_OBJECTIVE_OUTCOME_HEAD_INITIAL_INVALID' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(NEW.verification_plan_id,NEW.verification_objective_id)
       IS DISTINCT FROM ROW(OLD.verification_plan_id,OLD.verification_objective_id)
       OR NEW.row_version<>OLD.row_version+1 OR NEW.current_ordinal<>OLD.current_ordinal+1
       OR successor.predecessor_outcome_id<>OLD.current_outcome_id
       OR (successor.campaign_terminal_decision_id IS NOT NULL AND NOT EXISTS(
           SELECT 1
             FROM verification_campaign_terminal_decisions terminal
             JOIN verification_campaigns campaign ON campaign.campaign_id=terminal.campaign_id
            WHERE terminal.campaign_terminal_decision_id=successor.campaign_terminal_decision_id
              AND campaign.superseded_at IS NULL AND campaign.terminal_at IS NOT NULL
       ))
    THEN
        RAISE EXCEPTION 'VERIFICATION_OBJECTIVE_OUTCOME_HEAD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER hypothesis_objective_outcome_head_cas
BEFORE INSERT OR UPDATE OR DELETE ON hypothesis_objective_outcome_heads
FOR EACH ROW EXECUTE FUNCTION verification_guard_objective_outcome_head_cas();

CREATE TABLE hypothesis_objective_outcome_set_seals (
    objective_outcome_set_seal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    verification_plan_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    cutoff_at TIMESTAMPTZ NOT NULL,
    head_set_hash TEXT NOT NULL CHECK (head_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    seal_hash TEXT NOT NULL CHECK (seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ,
    UNIQUE(objective_outcome_set_seal_id,verification_plan_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(verification_plan_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_objective_outcome_set_members (
    objective_outcome_set_seal_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    verification_objective_id UUID NOT NULL,
    selected_current_outcome_id UUID NOT NULL,
    selected_current_ordinal BIGINT NOT NULL CHECK (selected_current_ordinal>0),
    selected_current_outcome_hash TEXT NOT NULL CHECK (selected_current_outcome_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(objective_outcome_set_seal_id,verification_objective_id),
    UNIQUE(objective_outcome_set_seal_id,member_ordinal),
    UNIQUE(objective_outcome_set_seal_id,member_hash),
    FOREIGN KEY(objective_outcome_set_seal_id,verification_plan_id,operation_id,project_scope_id,organization_id)
        REFERENCES hypothesis_objective_outcome_set_seals(
            objective_outcome_set_seal_id,verification_plan_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(selected_current_outcome_id,verification_plan_id,verification_objective_id,selected_current_ordinal)
        REFERENCES hypothesis_objective_outcome_receipts(
            objective_outcome_receipt_id,verification_plan_id,verification_objective_id,outcome_ordinal
        ) ON DELETE RESTRICT
);

CREATE FUNCTION verification_validate_objective_outcome_set_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_count BIGINT;
    actual_count BIGINT;
    invalid_count BIGINT;
    actual_head_set_hash TEXT;
BEGIN
    IF NEW.sealed_at IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT objective_count INTO STRICT expected_count
      FROM attack_hypothesis_verification_plans
     WHERE plan_id=NEW.verification_plan_id
       AND revision_id=NEW.hypothesis_revision_id AND sealed_at IS NOT NULL;
    SELECT COUNT(*),COUNT(*) FILTER(WHERE
               objective.plan_objective_id IS NULL
               OR head.current_outcome_id IS DISTINCT FROM member.selected_current_outcome_id
               OR head.current_ordinal IS DISTINCT FROM member.selected_current_ordinal
               OR receipt.outcome_hash IS DISTINCT FROM member.selected_current_outcome_hash
               OR receipt.created_at>NEW.cutoff_at
               OR EXISTS(
                    SELECT 1 FROM verification_authority_quarantine_events quarantine
                     WHERE quarantine.objective_outcome_receipt_id=member.selected_current_outcome_id
               )),
           investigation_exact_member_set_hash(
               'hypothesis_objective_outcome_heads.v1',
               COALESCE(array_agg(member.selected_current_outcome_hash ORDER BY member.selected_current_outcome_hash),ARRAY[]::TEXT[])
           )
      INTO actual_count,invalid_count,actual_head_set_hash
      FROM hypothesis_objective_outcome_set_members member
      LEFT JOIN attack_hypothesis_verification_plan_objectives objective
        ON objective.plan_id=NEW.verification_plan_id
       AND objective.objective_id=member.verification_objective_id
       AND objective.ordinal=member.member_ordinal
      LEFT JOIN hypothesis_objective_outcome_heads head
        ON head.verification_plan_id=NEW.verification_plan_id
       AND head.verification_objective_id=member.verification_objective_id
      LEFT JOIN hypothesis_objective_outcome_receipts receipt
        ON receipt.objective_outcome_receipt_id=member.selected_current_outcome_id
       AND receipt.verification_plan_id=NEW.verification_plan_id
       AND receipt.verification_objective_id=member.verification_objective_id
       AND receipt.operation_id=NEW.operation_id
       AND receipt.project_scope_id=NEW.project_scope_id
       AND receipt.organization_id=NEW.organization_id
     WHERE member.objective_outcome_set_seal_id=NEW.objective_outcome_set_seal_id;
    IF actual_count<>expected_count OR invalid_count<>0 OR actual_head_set_hash<>NEW.head_set_hash THEN
        RAISE EXCEPTION 'VERIFICATION_OBJECTIVE_OUTCOME_SET_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TABLE hypothesis_revision_adjudications (
    revision_adjudication_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    verification_plan_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    objective_outcome_set_seal_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    tool_truth_authority_bundle_seal_id UUID NOT NULL,
    relevant_root_set_hash TEXT NOT NULL CHECK (relevant_root_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    semantic_authority_bundle_hash TEXT NOT NULL CHECK (semantic_authority_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    freshness_attestation_bundle_hash TEXT NOT NULL CHECK (freshness_attestation_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    temporal_validity_bundle_hash TEXT NOT NULL CHECK (temporal_validity_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    temporal_census_hash TEXT NOT NULL CHECK (temporal_census_hash ~ '^sha256:[0-9a-f]{64}$'),
    temporal_policy_hash TEXT NOT NULL CHECK (temporal_policy_hash ~ '^sha256:[0-9a-f]{64}$'),
    target_epoch_set_hash TEXT NOT NULL CHECK (target_epoch_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    observation_window_start TIMESTAMPTZ NOT NULL,
    observation_window_end TIMESTAMPTZ NOT NULL,
    effective_valid_until TIMESTAMPTZ NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('nonterminal','verified','refuted')),
    unresolved_set_hash TEXT CHECK (unresolved_set_hash IS NULL OR unresolved_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    adjudication_hash TEXT NOT NULL CHECK (adjudication_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (observation_window_end>=observation_window_start),
    CHECK (effective_valid_until>observation_window_end),
    CHECK ((outcome='nonterminal')=(unresolved_set_hash IS NOT NULL)),
    UNIQUE(revision_adjudication_id,hypothesis_revision_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(verification_plan_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(objective_outcome_set_seal_id,verification_plan_id,operation_id,project_scope_id,organization_id)
        REFERENCES hypothesis_objective_outcome_set_seals(
            objective_outcome_set_seal_id,verification_plan_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(tool_truth_authority_bundle_seal_id,operation_id,organization_id)
        REFERENCES tool_truth_authority_bundle_seals(id,operation_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_revision_terminal_decisions (
    revision_terminal_decision_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    revision_adjudication_id UUID NOT NULL UNIQUE,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    terminal_successor_revision_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('verified','refuted')),
    finding_id UUID REFERENCES findings(id) ON DELETE RESTRICT,
    refutation_lineage_id UUID,
    state_event_id UUID NOT NULL UNIQUE REFERENCES attack_hypothesis_state_events(event_id) ON DELETE RESTRICT,
    decision_hash TEXT NOT NULL CHECK (decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (decision='verified' AND finding_id IS NOT NULL AND refutation_lineage_id IS NULL)
        OR (decision='refuted' AND finding_id IS NULL AND refutation_lineage_id IS NOT NULL)
    ),
    UNIQUE(revision_terminal_decision_id,hypothesis_revision_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(revision_adjudication_id,hypothesis_revision_id,operation_id,project_scope_id,organization_id)
        REFERENCES hypothesis_revision_adjudications(
            revision_adjudication_id,hypothesis_revision_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(terminal_successor_revision_id)
        REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE fact_delta_consumptions (
    fact_delta_consumption_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    fact_delta_bundle_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_id UUID NOT NULL REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN (
        'applied','no_semantic_change','quarantined_invalid_authority'
    )),
    consumption_hash TEXT NOT NULL CHECK (consumption_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_id UUID REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    consumed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(fact_delta_bundle_id,generation_id),
    FOREIGN KEY(fact_delta_bundle_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_fact_delta_bundles(
            fact_delta_bundle_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    CHECK ((disposition='quarantined_invalid_authority')=(residual_id IS NOT NULL))
);

CREATE TABLE hypothesis_evolution_proposals (
    evolution_proposal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_generation_id UUID NOT NULL REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    source_fact_delta_bundle_id UUID NOT NULL REFERENCES verification_fact_delta_bundles(fact_delta_bundle_id) ON DELETE RESTRICT,
    proposal_kind TEXT NOT NULL CHECK (proposal_kind IN ('refine','split','merge','supersede','close_no_change')),
    proposal_body JSONB NOT NULL CHECK (jsonb_typeof(proposal_body)='object'),
    proposal_hash TEXT NOT NULL CHECK (proposal_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(source_generation_id,source_fact_delta_bundle_id)
);

CREATE TABLE hypothesis_evolution_decisions (
    evolution_decision_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    evolution_proposal_id UUID NOT NULL UNIQUE REFERENCES hypothesis_evolution_proposals(evolution_proposal_id) ON DELETE RESTRICT,
    decision TEXT NOT NULL CHECK (decision IN ('accepted','rejected','deferred')),
    decision_reason_code TEXT NOT NULL CHECK (BTRIM(decision_reason_code)<>''),
    successor_generation_id UUID REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    residual_id UUID REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    decision_hash TEXT NOT NULL CHECK (decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((decision='accepted')=(successor_generation_id IS NOT NULL)),
    CHECK ((decision='accepted')=(residual_id IS NULL))
);

CREATE TABLE hypothesis_consolidation_batches (
    consolidation_batch_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_id UUID NOT NULL UNIQUE REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    wave_coverage_receipt_id UUID NOT NULL UNIQUE REFERENCES verification_wave_coverage_receipts(wave_coverage_receipt_id) ON DELETE RESTRICT,
    fact_delta_member_count BIGINT NOT NULL CHECK (fact_delta_member_count>=0),
    fact_delta_member_set_hash TEXT NOT NULL CHECK (fact_delta_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    unassigned_residual_set_hash TEXT NOT NULL CHECK (unassigned_residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ,
    UNIQUE(consolidation_batch_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE hypothesis_consolidation_receipts (
    consolidation_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    consolidation_batch_id UUID NOT NULL UNIQUE REFERENCES hypothesis_consolidation_batches(consolidation_batch_id) ON DELETE RESTRICT,
    successor_generation_id UUID REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN ('advanced','fixed_point','blocked')),
    applied_fact_delta_set_hash TEXT NOT NULL CHECK (applied_fact_delta_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_set_hash TEXT NOT NULL CHECK (residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK ((disposition='advanced')=(successor_generation_id IS NOT NULL))
);

CREATE TABLE hypothesis_fixed_point_receipts (
    fixed_point_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    consolidation_receipt_id UUID NOT NULL UNIQUE REFERENCES hypothesis_consolidation_receipts(consolidation_receipt_id) ON DELETE RESTRICT,
    generation_id UUID NOT NULL UNIQUE REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    open_obligation_set_hash TEXT NOT NULL CHECK (open_obligation_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_set_hash TEXT NOT NULL CHECK (residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    fixed_point_hash TEXT NOT NULL CHECK (fixed_point_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE enrichment_obligations (
    enrichment_obligation_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_objective_id UUID,
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    status TEXT NOT NULL CHECK (status IN ('open','satisfied','superseded','blocked')),
    residual_id UUID NOT NULL REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    obligation_hash TEXT NOT NULL CHECK (obligation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE application_fact_refinement_obligations (
    refinement_obligation_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    source_fact_delta_bundle_id UUID NOT NULL REFERENCES verification_fact_delta_bundles(fact_delta_bundle_id) ON DELETE RESTRICT,
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    status TEXT NOT NULL CHECK (status IN ('open','satisfied','superseded','blocked')),
    residual_id UUID NOT NULL REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    obligation_hash TEXT NOT NULL CHECK (obligation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE verification_authority_quarantine_events (
    quarantine_event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    campaign_terminal_decision_id UUID NOT NULL,
    objective_outcome_receipt_id UUID NOT NULL,
    campaign_coverage_receipt_id UUID NOT NULL,
    oracle_census_seal_id UUID NOT NULL,
    fact_delta_bundle_id UUID NOT NULL,
    invalid_semantic_reconciliation_id UUID NOT NULL REFERENCES capability_execution_reconciliations(id) ON DELETE RESTRICT,
    invalid_semantic_reconciliation_hash TEXT NOT NULL CHECK (invalid_semantic_reconciliation_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_hash TEXT NOT NULL CHECK (member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    quarantine_hash TEXT NOT NULL CHECK (quarantine_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(campaign_terminal_decision_id,invalid_semantic_reconciliation_id),
    UNIQUE(quarantine_event_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(campaign_terminal_decision_id)
        REFERENCES verification_campaign_terminal_decisions(campaign_terminal_decision_id) ON DELETE RESTRICT,
    FOREIGN KEY(objective_outcome_receipt_id)
        REFERENCES hypothesis_objective_outcome_receipts(objective_outcome_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_coverage_receipt_id)
        REFERENCES verification_campaign_coverage_receipts(campaign_coverage_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(oracle_census_seal_id)
        REFERENCES verification_oracle_census_seals(oracle_census_seal_id) ON DELETE RESTRICT,
    FOREIGN KEY(fact_delta_bundle_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_fact_delta_bundles(
            fact_delta_bundle_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_authority_quarantine_members (
    quarantine_event_id UUID NOT NULL REFERENCES verification_authority_quarantine_events(quarantine_event_id) ON DELETE RESTRICT,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    authority_ref_kind TEXT NOT NULL CHECK (authority_ref_kind IN (
        'campaign_terminal','objective_outcome','campaign_coverage','oracle_census','fact_delta',
        'revision_adjudication','revision_terminal','finding','refutation','report_source'
    )),
    authority_ref_id UUID NOT NULL,
    authority_ref_hash TEXT NOT NULL CHECK (authority_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(quarantine_event_id,authority_ref_kind,authority_ref_id),
    UNIQUE(quarantine_event_id,member_ordinal),
    UNIQUE(quarantine_event_id,member_hash)
);

CREATE TABLE verification_authority_temporal_staleness_events (
    temporal_staleness_event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    authority_ref_kind TEXT NOT NULL CHECK (authority_ref_kind IN (
        'objective_outcome','revision_adjudication','revision_terminal','finding','refutation'
    )),
    authority_ref_id UUID NOT NULL,
    observed_as_of TIMESTAMPTZ NOT NULL,
    effective_valid_until TIMESTAMPTZ NOT NULL,
    revalidation_obligation_id UUID NOT NULL REFERENCES tool_truth_revalidation_obligations(id) ON DELETE RESTRICT,
    event_hash TEXT NOT NULL CHECK (event_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (observed_as_of>=effective_valid_until),
    UNIQUE(authority_ref_kind,authority_ref_id,effective_valid_until)
);

CREATE TABLE hypothesis_re_adjudication_obligations (
    re_adjudication_obligation_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    invalidated_authority_ref_kind TEXT NOT NULL CHECK (BTRIM(invalidated_authority_ref_kind)<>''),
    invalidated_authority_ref_id UUID NOT NULL,
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    status TEXT NOT NULL CHECK (status IN ('open','satisfied','superseded')),
    obligation_hash TEXT NOT NULL CHECK (obligation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(hypothesis_revision_id,invalidated_authority_ref_kind,invalidated_authority_ref_id),
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE verification_authority_correction_bundles (
    correction_bundle_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    quarantine_event_id UUID NOT NULL UNIQUE REFERENCES verification_authority_quarantine_events(quarantine_event_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    superseded_fact_delta_bundle_id UUID NOT NULL REFERENCES verification_fact_delta_bundles(fact_delta_bundle_id) ON DELETE RESTRICT,
    correction_kind TEXT NOT NULL CHECK (correction_kind IN ('retraction','supersession')),
    typed_correction_delta JSONB NOT NULL CHECK (jsonb_typeof(typed_correction_delta)='object'),
    correction_hash TEXT NOT NULL CHECK (correction_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(correction_bundle_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE verification_authority_correction_consumptions (
    correction_consumption_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    correction_bundle_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_id UUID NOT NULL REFERENCES hypothesis_generations(generation_id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN ('applied','superseded','rejected')),
    consumption_hash TEXT NOT NULL CHECK (consumption_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(correction_bundle_id,generation_id),
    FOREIGN KEY(correction_bundle_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_authority_correction_bundles(
            correction_bundle_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

-- Shadow evaluation is deliberately detached from authorization, credentials,
-- budget reservations, leases, Findings and FactDelta authority.
CREATE TABLE verification_campaign_shadow_evaluations (
    shadow_evaluation_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    verification_plan_id UUID NOT NULL,
    frozen_snapshot_id UUID NOT NULL,
    frozen_snapshot_hash TEXT NOT NULL CHECK (frozen_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    obligation_census_hash TEXT NOT NULL CHECK (obligation_census_hash ~ '^sha256:[0-9a-f]{64}$'),
    as_of_change_seq BIGINT NOT NULL CHECK (as_of_change_seq>=0),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    obligation_member_count BIGINT NOT NULL CHECK (obligation_member_count>0),
    obligation_member_set_hash TEXT NOT NULL CHECK (obligation_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    evaluation_hash TEXT NOT NULL CHECK (evaluation_hash ~ '^sha256:[0-9a-f]{64}$'),
    state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open','closed')),
    comparison_count BIGINT CHECK (comparison_count IS NULL OR comparison_count>=0),
    comparison_id_set_hash TEXT CHECK (comparison_id_set_hash IS NULL OR comparison_id_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    receipt_hash TEXT CHECK (receipt_hash IS NULL OR receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    closed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (state='open' AND comparison_count IS NULL AND comparison_id_set_hash IS NULL
            AND receipt_hash IS NULL AND closed_at IS NULL)
        OR (state='closed' AND comparison_count IS NOT NULL
            AND comparison_id_set_hash IS NOT NULL AND receipt_hash IS NOT NULL
            AND closed_at IS NOT NULL)
    ),
    UNIQUE(shadow_evaluation_id,operation_id,project_scope_id,organization_id),
    FOREIGN KEY(verification_plan_id,hypothesis_revision_id)
        REFERENCES attack_hypothesis_verification_plans(plan_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE verification_campaign_shadow_evaluation_obligations (
    shadow_evaluation_obligation_id UUID PRIMARY KEY,
    shadow_evaluation_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    obligation_ordinal INTEGER NOT NULL CHECK (obligation_ordinal>=0),
    plan_objective_id UUID NOT NULL REFERENCES attack_hypothesis_verification_plan_objectives(plan_objective_id) ON DELETE RESTRICT,
    plan_objective_member_hash TEXT NOT NULL CHECK (plan_objective_member_hash ~ '^sha256:[0-9a-f]{64}$'),
    frozen_target_hash TEXT NOT NULL CHECK (frozen_target_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(shadow_evaluation_id,obligation_ordinal),
    UNIQUE(shadow_evaluation_id,plan_objective_id),
    UNIQUE(shadow_evaluation_id,member_hash),
    UNIQUE(shadow_evaluation_obligation_id,shadow_evaluation_id),
    FOREIGN KEY(shadow_evaluation_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_shadow_evaluations(
            shadow_evaluation_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE verification_campaign_shadow_evaluation_items (
    shadow_evaluation_item_id UUID PRIMARY KEY,
    shadow_evaluation_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    item_ordinal INTEGER NOT NULL CHECK (item_ordinal>=0),
    shadow_evaluation_obligation_id UUID NOT NULL,
    plan_objective_id UUID NOT NULL REFERENCES attack_hypothesis_verification_plan_objectives(plan_objective_id) ON DELETE RESTRICT,
    compiled_semantic_signature_hash TEXT NOT NULL CHECK (compiled_semantic_signature_hash ~ '^sha256:[0-9a-f]{64}$'),
    legacy_capability_execution_receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    deterministic_oracle_replay_ref UUID NOT NULL,
    comparison_id UUID NOT NULL REFERENCES investigation_projection_compare_samples(comparison_id) ON DELETE RESTRICT,
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(shadow_evaluation_id,item_ordinal),
    UNIQUE(shadow_evaluation_id,shadow_evaluation_obligation_id),
    UNIQUE(shadow_evaluation_id,comparison_id),
    UNIQUE(shadow_evaluation_id,member_hash),
    FOREIGN KEY(shadow_evaluation_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_campaign_shadow_evaluations(
            shadow_evaluation_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(shadow_evaluation_obligation_id,shadow_evaluation_id)
        REFERENCES verification_campaign_shadow_evaluation_obligations(
            shadow_evaluation_obligation_id,shadow_evaluation_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION verification_validate_set_seal()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    member_table REGCLASS := TG_ARGV[0]::REGCLASS;
    member_parent_column TEXT := TG_ARGV[1];
    header_id_column TEXT := TG_ARGV[2];
    hash_domain TEXT := TG_ARGV[3];
    header_id UUID;
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'VERIFICATION_SEALED_HEADER_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF OLD.sealed_at IS NOT NULL OR NEW.sealed_at IS NULL
       OR (to_jsonb(NEW)-'sealed_at') IS DISTINCT FROM (to_jsonb(OLD)-'sealed_at')
    THEN
        RAISE EXCEPTION 'VERIFICATION_SEALED_HEADER_MUTATION_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    header_id := (to_jsonb(NEW)->>header_id_column)::UUID;
    EXECUTE format(
        'SELECT COUNT(*),investigation_exact_member_set_hash($2,COALESCE(array_agg(member_hash ORDER BY member_hash),ARRAY[]::TEXT[])) FROM %s WHERE %I=$1',
        member_table,member_parent_column
    ) INTO actual_count,actual_hash USING header_id,hash_domain;
    IF actual_count<>(to_jsonb(NEW)->>'member_count')::BIGINT
       OR actual_hash<>(to_jsonb(NEW)->>'member_set_hash')
    THEN
        RAISE EXCEPTION 'VERIFICATION_SET_SEAL_EXACT_SET_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION verification_validate_capability_assessment_set_latest()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.sealed_at IS NOT NULL AND OLD.sealed_at IS NULL AND EXISTS(
        SELECT 1
          FROM verification_capability_assessment_set_members member
          JOIN verification_capability_assessments assessment
            ON assessment.assessment_id=member.assessment_id
         WHERE member.assessment_set_seal_id=NEW.assessment_set_seal_id
           AND EXISTS(
               SELECT 1 FROM verification_capability_assessments newer
                WHERE newer.hypothesis_revision_id=assessment.hypothesis_revision_id
                  AND newer.verification_objective_id=assessment.verification_objective_id
                  AND newer.verification_contract_hash=assessment.verification_contract_hash
                  AND newer.capability_key=assessment.capability_key
                  AND newer.policy_snapshot_hash=assessment.policy_snapshot_hash
                  AND newer.assessment_ordinal>assessment.assessment_ordinal
           )
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_CAPABILITY_ASSESSMENT_SET_STALE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_capability_assessment_set_seal_guard
BEFORE UPDATE OR DELETE ON verification_capability_assessment_set_seals
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'verification_capability_assessment_set_members','assessment_set_seal_id',
    'assessment_set_seal_id','verification_capability_assessment_set.v1'
);
CREATE CONSTRAINT TRIGGER verification_capability_assessment_set_latest
AFTER UPDATE ON verification_capability_assessment_set_seals
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_validate_capability_assessment_set_latest();
CREATE TRIGGER verification_capability_assessment_set_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON verification_capability_assessment_set_members
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'verification_capability_assessment_set_seals','assessment_set_seal_id','assessment_set_seal_id'
);

CREATE TRIGGER verification_wave_coverage_denominator_seal_guard
BEFORE UPDATE OR DELETE ON verification_wave_coverage_denominators
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'verification_wave_coverage_members','wave_denominator_id',
    'wave_denominator_id','verification_wave_coverage_denominator.v1'
);
CREATE TRIGGER verification_wave_coverage_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON verification_wave_coverage_members
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'verification_wave_coverage_denominators','wave_denominator_id','wave_denominator_id'
);

CREATE TRIGGER verification_campaign_coverage_denominator_seal_guard
BEFORE UPDATE OR DELETE ON verification_campaign_coverage_denominators
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'verification_campaign_coverage_members','campaign_denominator_id',
    'campaign_denominator_id','verification_campaign_coverage_denominator.v1'
);
CREATE TRIGGER verification_campaign_coverage_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON verification_campaign_coverage_members
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'verification_campaign_coverage_denominators','campaign_denominator_id','campaign_denominator_id'
);

CREATE TRIGGER verification_action_conflict_set_seal_guard
BEFORE UPDATE OR DELETE ON verification_action_conflict_sets
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'verification_action_conflict_set_members','conflict_set_id',
    'conflict_set_id','verification_action_conflict_set.v1'
);
CREATE TRIGGER verification_action_conflict_set_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON verification_action_conflict_set_members
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'verification_action_conflict_sets','conflict_set_id','conflict_set_id'
);

CREATE TRIGGER verification_budget_contract_seal_guard
BEFORE UPDATE OR DELETE ON verification_budget_contracts
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'verification_budget_contract_axes','budget_contract_id',
    'budget_contract_id','verification_budget_contract.v1'
);
CREATE TRIGGER verification_budget_contract_hierarchy_guard
BEFORE UPDATE ON verification_budget_contracts
FOR EACH ROW EXECUTE FUNCTION verification_validate_budget_contract_hierarchy();
CREATE TRIGGER verification_budget_contract_axis_guard
BEFORE INSERT OR UPDATE OR DELETE ON verification_budget_contract_axes
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'verification_budget_contracts','budget_contract_id','budget_contract_id'
);

CREATE TRIGGER verification_oracle_census_seal_guard
BEFORE UPDATE OR DELETE ON verification_oracle_census_seals
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'verification_oracle_census_members','oracle_census_seal_id',
    'oracle_census_seal_id','verification_oracle_census.v1'
);
CREATE TRIGGER verification_oracle_census_authority_guard
BEFORE UPDATE ON verification_oracle_census_seals
FOR EACH ROW EXECUTE FUNCTION verification_validate_oracle_census_authority();
CREATE TRIGGER verification_oracle_census_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON verification_oracle_census_members
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'verification_oracle_census_seals','oracle_census_seal_id','oracle_census_seal_id'
);

CREATE TRIGGER verification_claim_component_outcome_seal_guard
BEFORE UPDATE OR DELETE ON hypothesis_objective_claim_component_outcome_seals
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'hypothesis_objective_claim_component_outcome_members','claim_component_outcome_seal_id',
    'claim_component_outcome_seal_id','hypothesis_objective_claim_component_outcomes.v1'
);
CREATE TRIGGER verification_claim_component_outcome_authority_guard
BEFORE UPDATE ON hypothesis_objective_claim_component_outcome_seals
FOR EACH ROW EXECUTE FUNCTION verification_validate_claim_component_outcome_authority();
CREATE TRIGGER verification_claim_component_outcome_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON hypothesis_objective_claim_component_outcome_members
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'hypothesis_objective_claim_component_outcome_seals','claim_component_outcome_seal_id','claim_component_outcome_seal_id'
);

CREATE TRIGGER verification_objective_outcome_set_seal_guard
BEFORE UPDATE OR DELETE ON hypothesis_objective_outcome_set_seals
FOR EACH ROW EXECUTE FUNCTION verification_validate_set_seal(
    'hypothesis_objective_outcome_set_members','objective_outcome_set_seal_id',
    'objective_outcome_set_seal_id','hypothesis_objective_outcome_set.v1'
);
CREATE TRIGGER verification_objective_outcome_set_authority_guard
BEFORE UPDATE ON hypothesis_objective_outcome_set_seals
FOR EACH ROW EXECUTE FUNCTION verification_validate_objective_outcome_set_authority();
CREATE TRIGGER verification_objective_outcome_set_member_guard
BEFORE INSERT OR UPDATE OR DELETE ON hypothesis_objective_outcome_set_members
FOR EACH ROW EXECUTE FUNCTION verification_guard_set_member(
    'hypothesis_objective_outcome_set_seals','objective_outcome_set_seal_id','objective_outcome_set_seal_id'
);

CREATE FUNCTION verification_guard_campaign_admission()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    dispatch_held BOOLEAN;
    assessment_sealed TIMESTAMPTZ;
    wave_sealed TIMESTAMPTZ;
    bundle tool_truth_authority_bundle_seals%ROWTYPE;
BEGIN
    SELECT campaign_dispatch_held INTO STRICT dispatch_held
      FROM verification_campaign_safety_holds WHERE singleton=TRUE FOR SHARE;
    SELECT sealed_at INTO assessment_sealed
      FROM verification_capability_assessment_set_seals
     WHERE assessment_set_seal_id=NEW.capability_assessment_set_seal_id FOR SHARE;
    SELECT sealed_at INTO wave_sealed
      FROM verification_wave_coverage_denominators
     WHERE wave_denominator_id=NEW.wave_denominator_id FOR SHARE;
    SELECT * INTO bundle FROM tool_truth_authority_bundle_seals
     WHERE id=NEW.tool_truth_authority_bundle_seal_id
       AND operation_id=NEW.operation_id AND organization_id=NEW.organization_id
       AND consumer_kind='verification_campaign' FOR SHARE;
    IF dispatch_held THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_DISPATCH_HELD' USING ERRCODE='23514';
    END IF;
    IF assessment_sealed IS NULL OR wave_sealed IS NULL OR bundle.sealed_at IS NULL
       OR bundle.consistent_fresh_count<>bundle.member_count
       OR bundle.stale_or_invalid_count<>0
       OR bundle.effective_valid_until IS NULL
       OR bundle.effective_valid_until<=statement_timestamp()
       OR bundle.relevant_root_set_hash<>NEW.relevant_root_set_hash
       OR bundle.member_set_hash<>NEW.authority_member_set_hash
       OR bundle.semantic_authority_bundle_hash<>NEW.semantic_authority_bundle_hash
       OR bundle.freshness_attestation_bundle_hash<>NEW.freshness_attestation_bundle_hash
       OR bundle.temporal_validity_bundle_hash<>NEW.temporal_validity_bundle_hash
       OR bundle.effective_valid_until<>NEW.effective_valid_until
    THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_ADMISSION_AUTHORITY_UNSEALED' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_campaign_admission_guard
BEFORE INSERT ON verification_campaigns
FOR EACH ROW EXECUTE FUNCTION verification_guard_campaign_admission();

CREATE FUNCTION verification_guard_action_authorization()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    hold verification_campaign_safety_holds%ROWTYPE;
    action verification_prepared_actions%ROWTYPE;
    denominator_sealed TIMESTAMPTZ;
    conflict_sealed TIMESTAMPTZ;
    capability_status TEXT;
BEGIN
    SELECT * INTO STRICT action FROM verification_prepared_actions
     WHERE prepared_action_id=NEW.prepared_action_id FOR UPDATE;
    IF action.state<>'pending_authorization'
       OR action.row_version<>NEW.expected_action_row_version
       OR action.risk_tier NOT IN ('T2','T3')
       OR action.display_projection_hash<>NEW.expected_display_projection_hash
       OR NEW.reviewed_action_hash<>action.display_projection_hash
       OR action.private_manifest_hash<>NEW.expected_private_manifest_hash
       OR action.renderer_version<>NEW.renderer_version
       OR action.review_expires_at<=statement_timestamp()
       OR (NEW.decision<>'authorized' AND NEW.expires_at IS NOT NULL)
       OR NOT EXISTS(
           SELECT 1 FROM operator_principals principal
            WHERE principal.id=NEW.decided_by
              AND principal.principal_kind='local_operator' AND principal.active
       )
    THEN
        RAISE EXCEPTION 'VERIFICATION_ACTION_REVIEW_AUTHORITY_STALE' USING ERRCODE='23514';
    END IF;
    IF NEW.decision<>'authorized' THEN
        RETURN NEW;
    END IF;
    SELECT * INTO STRICT hold FROM verification_campaign_safety_holds
     WHERE singleton=TRUE FOR SHARE;
    SELECT denominator.sealed_at INTO denominator_sealed
      FROM verification_campaign_coverage_denominators denominator
     WHERE denominator.campaign_id=action.campaign_id FOR SHARE;
    SELECT conflict_set.sealed_at INTO conflict_sealed
      FROM verification_action_conflict_sets conflict_set
     WHERE conflict_set.prepared_action_id=action.prepared_action_id FOR SHARE;
    SELECT status INTO capability_status FROM verification_capability_assessments
     WHERE assessment_id=action.capability_assessment_id FOR SHARE;
    IF hold.campaign_dispatch_held
       OR NEW.campaign_dispatch_generation<>hold.campaign_dispatch_generation
       OR action.target_live_id IS NULL OR denominator_sealed IS NULL OR conflict_sealed IS NULL
       OR capability_status<>'available'
       OR NEW.expires_at IS NULL OR NEW.expires_at<=statement_timestamp()
       OR NEW.expires_at>action.review_expires_at
    THEN
        RAISE EXCEPTION 'VERIFICATION_ACTION_AUTHORIZATION_AUTHORITY_STALE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_action_authorization_guard
BEFORE INSERT ON verification_prepared_action_authorizations
FOR EACH ROW EXECUTE FUNCTION verification_guard_action_authorization();

CREATE FUNCTION verification_guard_durable_action_begin()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    hold verification_campaign_safety_holds%ROWTYPE;
    action_state TEXT;
    action_contract_kind TEXT;
    action_target_live_id UUID;
    authorization_decision TEXT;
    authorization_expires_at TIMESTAMPTZ;
    reservation_state TEXT;
    missing_key_count BIGINT;
BEGIN
    SELECT * INTO STRICT hold FROM verification_campaign_safety_holds
     WHERE singleton=TRUE FOR SHARE;
    SELECT state,action_contract_kind,target_live_id
      INTO action_state,action_contract_kind,action_target_live_id
      FROM verification_prepared_actions
     WHERE prepared_action_id=NEW.prepared_action_id FOR UPDATE;
    SELECT decision,expires_at INTO authorization_decision,authorization_expires_at
      FROM verification_prepared_action_authorizations
     WHERE authorization_receipt_id=NEW.authorization_receipt_id FOR SHARE;
    SELECT state INTO reservation_state FROM verification_budget_reservations
     WHERE budget_reservation_id=NEW.budget_reservation_id FOR SHARE;
    SELECT COUNT(*) INTO missing_key_count
      FROM verification_action_conflict_set_members member
      JOIN verification_action_conflict_sets conflict_set
        ON conflict_set.conflict_set_id=member.conflict_set_id
      LEFT JOIN verification_conflict_key_heads head
        ON head.operation_id=NEW.operation_id AND head.organization_id=NEW.organization_id
       AND head.key_kind=member.key_kind AND head.key_identity_hash=member.key_identity_hash
       AND head.state='active' AND head.owner_prepared_action_id=NEW.prepared_action_id
     WHERE conflict_set.conflict_set_id=NEW.conflict_set_id AND head.operation_id IS NULL;
    IF hold.campaign_dispatch_held
       OR NEW.campaign_dispatch_generation<>hold.campaign_dispatch_generation
       OR action_state<>'authorized' OR authorization_decision<>'authorized'
       OR action_contract_kind<>NEW.execution_kind OR action_target_live_id IS NULL
       OR authorization_expires_at IS NULL OR authorization_expires_at<=statement_timestamp()
       OR reservation_state<>'active' OR missing_key_count<>0
    THEN
        RAISE EXCEPTION 'VERIFICATION_ACTION_DURABLE_BEGIN_AUTHORITY_STALE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_action_durable_begin_guard
BEFORE INSERT ON verification_action_executions
FOR EACH ROW EXECUTE FUNCTION verification_guard_durable_action_begin();

CREATE FUNCTION verification_enforce_campaign_coverage_exact_set()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    receipt verification_campaign_coverage_receipts%ROWTYPE;
    expected_count BIGINT;
    actual_count BIGINT;
    invalid_count BIGINT;
BEGIN
    SELECT * INTO STRICT receipt FROM verification_campaign_coverage_receipts
     WHERE campaign_coverage_receipt_id=NEW.campaign_coverage_receipt_id;
    SELECT member_count INTO STRICT expected_count
      FROM verification_campaign_coverage_denominators
     WHERE campaign_denominator_id=receipt.campaign_denominator_id;
    SELECT COUNT(*),COUNT(*) FILTER(WHERE member.campaign_denominator_id<>receipt.campaign_denominator_id)
      INTO actual_count,invalid_count
      FROM verification_campaign_coverage_results result
      JOIN verification_campaign_coverage_members member
        ON member.campaign_coverage_member_id=result.campaign_coverage_member_id
     WHERE result.campaign_coverage_receipt_id=receipt.campaign_coverage_receipt_id;
    IF actual_count<>expected_count OR invalid_count<>0
       OR actual_count<>(receipt.tested_complete_count+receipt.tested_degraded_count+receipt.untested_count+receipt.blocked_count)
    THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_COVERAGE_EXACT_SET_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_campaign_coverage_receipt_exact_set
AFTER INSERT ON verification_campaign_coverage_receipts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_campaign_coverage_exact_set();
CREATE CONSTRAINT TRIGGER verification_campaign_coverage_result_exact_set
AFTER INSERT ON verification_campaign_coverage_results
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_campaign_coverage_exact_set();

CREATE FUNCTION verification_enforce_campaign_terminal_compound()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    coverage_count BIGINT;
    fact_delta_count BIGINT;
    outcome_count BIGINT;
    active_action_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO coverage_count FROM verification_campaign_coverage_receipts
     WHERE campaign_terminal_decision_id=NEW.campaign_terminal_decision_id;
    SELECT COUNT(*) INTO fact_delta_count FROM verification_fact_delta_bundles
     WHERE campaign_terminal_decision_id=NEW.campaign_terminal_decision_id;
    SELECT COUNT(*) INTO outcome_count FROM hypothesis_objective_outcome_receipts
     WHERE campaign_terminal_decision_id=NEW.campaign_terminal_decision_id;
    SELECT COUNT(*) INTO active_action_count FROM verification_prepared_actions
     WHERE campaign_id=NEW.campaign_id
       AND state IN ('pending_authorization','authorized','started','outcome_unknown');
    IF coverage_count<>1 OR fact_delta_count<>1 OR outcome_count<>1 OR active_action_count<>0
       OR NOT EXISTS(
           SELECT 1 FROM verification_campaigns campaign
            WHERE campaign.campaign_id=NEW.campaign_id AND campaign.state='terminal'
              AND campaign.terminal_at IS NOT NULL
       )
    THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_TERMINAL_COMPOUND_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_campaign_terminal_compound
AFTER INSERT ON verification_campaign_terminal_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_campaign_terminal_compound();

CREATE FUNCTION verification_enforce_revision_terminal_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    adjudication_outcome TEXT;
    event attack_hypothesis_state_events%ROWTYPE;
BEGIN
    SELECT outcome INTO STRICT adjudication_outcome FROM hypothesis_revision_adjudications
     WHERE revision_adjudication_id=NEW.revision_adjudication_id;
    SELECT * INTO STRICT event FROM attack_hypothesis_state_events WHERE event_id=NEW.state_event_id;
    IF adjudication_outcome<>NEW.decision OR event.event_kind<>NEW.decision
       OR event.successor_revision_id<>NEW.terminal_successor_revision_id
       OR event.predecessor_revision_id<>NEW.hypothesis_revision_id
       OR event.origin_authority<>'hypothesis_revision_adjudication'
       OR event.authority_receipt_kind<>'revision_transition_decision'
       OR event.authority_receipt_id<>NEW.revision_terminal_decision_id
       OR event.authority_receipt_hash<>NEW.decision_hash
    THEN
        RAISE EXCEPTION 'VERIFICATION_REVISION_TERMINAL_AUTHORITY_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_revision_terminal_authority
AFTER INSERT ON hypothesis_revision_terminal_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_revision_terminal_authority();

CREATE FUNCTION verification_guard_campaign_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE'
       OR (to_jsonb(NEW)-ARRAY['state','row_version','terminal_at','superseded_at'])
          IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','row_version','terminal_at','superseded_at'])
       OR NEW.row_version<>OLD.row_version+1
       OR NOT (
           OLD.state=NEW.state
           OR
           (OLD.state='admitted' AND NEW.state IN ('running','superseded'))
           OR (OLD.state='running' AND NEW.state IN ('stopping','draining','terminal','superseded'))
           OR (OLD.state='stopping' AND NEW.state='draining')
           OR (OLD.state='draining' AND NEW.state='terminal')
       )
       OR (NEW.state='terminal' AND NEW.terminal_at IS NULL)
       OR (NEW.state='superseded' AND NEW.superseded_at IS NULL)
    THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_campaign_cas
BEFORE UPDATE OR DELETE ON verification_campaigns
FOR EACH ROW EXECUTE FUNCTION verification_guard_campaign_cas();

CREATE FUNCTION verification_guard_execution_closeout_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE'
       OR (to_jsonb(NEW)-ARRAY['state','capability_execution_receipt_id','closeout_hash','row_version','completed_at'])
          IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','capability_execution_receipt_id','closeout_hash','row_version','completed_at'])
       OR NOT (
           (OLD.state='started' AND NEW.state IN ('succeeded','failed','outcome_unknown'))
           OR
           (OLD.state='outcome_unknown' AND NEW.state IN ('succeeded','failed')
            AND EXISTS(
                SELECT 1 FROM verification_action_recovery_receipts receipt
                 WHERE receipt.action_execution_id=NEW.action_execution_id
                   AND receipt.prepared_action_id=NEW.prepared_action_id
                   AND receipt.execution_result_state=NEW.state
                   AND receipt.recovery_hash=NEW.closeout_hash
            ))
       )
       OR NEW.row_version<>OLD.row_version+1 OR NEW.capability_execution_receipt_id IS NULL
       OR NEW.closeout_hash IS NULL OR NEW.completed_at IS NULL
    THEN
        RAISE EXCEPTION 'VERIFICATION_ACTION_EXECUTION_CLOSEOUT_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_action_execution_closeout_cas
BEFORE UPDATE OR DELETE ON verification_action_executions
FOR EACH ROW EXECUTE FUNCTION verification_guard_execution_closeout_cas();

CREATE FUNCTION verification_guard_budget_reservation_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE'
       OR (to_jsonb(NEW)-ARRAY['state','row_version','settled_at'])
          IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['state','row_version','settled_at'])
       OR NOT (
           (OLD.state='active' AND NEW.state IN ('settled','unknown_held'))
           OR (OLD.state='unknown_held' AND NEW.state='settled')
       )
       OR NEW.row_version<>OLD.row_version+1 OR NEW.settled_at IS NULL
    THEN
        RAISE EXCEPTION 'VERIFICATION_BUDGET_RESERVATION_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_budget_reservation_cas
BEFORE UPDATE OR DELETE ON verification_budget_reservations
FOR EACH ROW EXECUTE FUNCTION verification_guard_budget_reservation_cas();

CREATE FUNCTION verification_enforce_wave_coverage_exact_union()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    denominator_count BIGINT;
    selected_count BIGINT;
    unassigned_count BIGINT;
    union_count BIGINT;
    duplicate_count BIGINT;
BEGIN
    SELECT member_count INTO STRICT denominator_count
      FROM verification_wave_coverage_denominators
     WHERE wave_denominator_id=NEW.wave_denominator_id;
    WITH selected AS (
        SELECT result.campaign_coverage_member_id,member.wave_coverage_member_id
          FROM verification_campaigns campaign
          JOIN verification_campaign_coverage_denominators denominator
            ON denominator.campaign_id=campaign.campaign_id
          JOIN verification_campaign_coverage_receipts receipt
            ON receipt.campaign_denominator_id=denominator.campaign_denominator_id
          JOIN verification_campaign_coverage_results result
            ON result.campaign_coverage_receipt_id=receipt.campaign_coverage_receipt_id
          JOIN verification_campaign_coverage_members member
            ON member.campaign_coverage_member_id=result.campaign_coverage_member_id
         WHERE denominator.wave_denominator_id=NEW.wave_denominator_id
           AND campaign.superseded_at IS NULL AND campaign.terminal_at IS NOT NULL
    ), unassigned AS (
        SELECT wave_coverage_member_id
          FROM verification_wave_unassigned_coverage_results
         WHERE wave_coverage_receipt_id=NEW.wave_coverage_receipt_id
    ), all_members AS (
        SELECT wave_coverage_member_id FROM selected
        UNION ALL
        SELECT wave_coverage_member_id FROM unassigned
    )
    SELECT (SELECT COUNT(*) FROM selected),(SELECT COUNT(*) FROM unassigned),
           COUNT(*),COUNT(*)-COUNT(DISTINCT wave_coverage_member_id)
      INTO selected_count,unassigned_count,union_count,duplicate_count
      FROM all_members;
    IF union_count<>denominator_count OR duplicate_count<>0
       OR union_count<>NEW.result_member_count OR selected_count+unassigned_count<>union_count
    THEN
        RAISE EXCEPTION 'VERIFICATION_WAVE_COVERAGE_EXACT_UNION_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_wave_coverage_exact_union
AFTER INSERT ON verification_wave_coverage_receipts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_wave_coverage_exact_union();
CREATE CONSTRAINT TRIGGER verification_wave_unassigned_exact_union
AFTER INSERT ON verification_wave_unassigned_coverage_results
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_wave_coverage_exact_union();

CREATE FUNCTION verification_enforce_objective_outcome_set_plan_exact()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_count BIGINT;
    actual_count BIGINT;
BEGIN
    IF NEW.sealed_at IS NULL THEN
        RETURN NULL;
    END IF;
    SELECT objective_count INTO STRICT expected_count
      FROM attack_hypothesis_verification_plans WHERE plan_id=NEW.verification_plan_id;
    SELECT COUNT(*) INTO actual_count FROM hypothesis_objective_outcome_set_members
     WHERE objective_outcome_set_seal_id=NEW.objective_outcome_set_seal_id;
    IF expected_count<>actual_count OR NEW.member_count<>actual_count THEN
        RAISE EXCEPTION 'VERIFICATION_OBJECTIVE_OUTCOME_SET_PLAN_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_objective_outcome_set_plan_exact
AFTER UPDATE ON hypothesis_objective_outcome_set_seals
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_objective_outcome_set_plan_exact();

CREATE FUNCTION verification_enforce_quarantine_exact_set()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    SELECT COUNT(*),investigation_exact_member_set_hash(
               'verification_authority_quarantine.v1',
               COALESCE(array_agg(member_hash ORDER BY member_hash),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM verification_authority_quarantine_members
     WHERE quarantine_event_id=NEW.quarantine_event_id;
    IF actual_count<>NEW.member_count OR actual_hash<>NEW.member_set_hash THEN
        RAISE EXCEPTION 'VERIFICATION_AUTHORITY_QUARANTINE_EXACT_SET_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_authority_quarantine_exact_set
AFTER INSERT ON verification_authority_quarantine_events
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_quarantine_exact_set();

CREATE FUNCTION verification_enforce_shadow_exact_set()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    SELECT COUNT(*),investigation_exact_member_set_hash(
               'verification_campaign_shadow_evaluation.v1',
               COALESCE(array_agg(member_hash ORDER BY member_hash),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM verification_campaign_shadow_evaluation_obligations
     WHERE shadow_evaluation_id=NEW.shadow_evaluation_id;
    IF actual_count<>NEW.obligation_member_count OR actual_hash<>NEW.obligation_member_set_hash THEN
        RAISE EXCEPTION 'VERIFICATION_SHADOW_EXACT_SET_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER verification_campaign_shadow_exact_set
AFTER INSERT ON verification_campaign_shadow_evaluations
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION verification_enforce_shadow_exact_set();

CREATE FUNCTION verification_guard_shadow_evaluation_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP='DELETE'
       OR (to_jsonb(NEW)-ARRAY['state','comparison_count','comparison_id_set_hash','receipt_hash','row_version','closed_at'])
          IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY['state','comparison_count','comparison_id_set_hash','receipt_hash','row_version','closed_at'])
       OR OLD.state<>'open' OR NEW.state<>'closed'
       OR NEW.row_version<>OLD.row_version+1
       OR NEW.comparison_count<>(SELECT COUNT(*) FROM verification_campaign_shadow_evaluation_items item WHERE item.shadow_evaluation_id=NEW.shadow_evaluation_id)
    THEN
        RAISE EXCEPTION 'VERIFICATION_SHADOW_EVALUATION_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_campaign_shadow_evaluation_cas
BEFORE UPDATE OR DELETE ON verification_campaign_shadow_evaluations
FOR EACH ROW EXECUTE FUNCTION verification_guard_shadow_evaluation_cas();

-- Immutable history.  The only mutable rows in this migration are the four
-- explicit CAS heads plus the narrowly guarded Campaign/action execution rows.
CREATE TRIGGER verification_capability_assessments_append_only BEFORE UPDATE OR DELETE ON verification_capability_assessments FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_capability_assessment_members_append_only BEFORE UPDATE OR DELETE ON verification_capability_assessment_set_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_wave_coverage_members_append_only BEFORE UPDATE OR DELETE ON verification_wave_coverage_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_campaign_coverage_members_append_only BEFORE UPDATE OR DELETE ON verification_campaign_coverage_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE FUNCTION verification_guard_campaign_round_close_cas()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE'
       OR OLD.closed_at IS NOT NULL
       OR NEW.closed_at IS NULL
       OR NEW.disposition IS NULL
       OR BTRIM(COALESCE(NEW.disposition_reason_code,''))=''
       OR (to_jsonb(NEW)-ARRAY['disposition','disposition_reason_code','residual_id','closed_at'])
          IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY['disposition','disposition_reason_code','residual_id','closed_at'])
    THEN
        RAISE EXCEPTION 'VERIFICATION_CAMPAIGN_ROUND_CLOSE_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER verification_campaign_round_close_cas
BEFORE UPDATE OR DELETE ON verification_campaign_rounds
FOR EACH ROW EXECUTE FUNCTION verification_guard_campaign_round_close_cas();
CREATE TRIGGER verification_consults_append_only BEFORE UPDATE OR DELETE ON verification_consults FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_consult_terminals_append_only BEFORE UPDATE OR DELETE ON verification_consult_terminals FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_strategy_artifacts_append_only BEFORE UPDATE OR DELETE ON verification_strategy_artifacts FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_strategy_obligations_append_only BEFORE UPDATE OR DELETE ON verification_strategy_obligations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_group_members_append_only BEFORE UPDATE OR DELETE ON verification_prepared_action_group_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_authorizations_append_only BEFORE UPDATE OR DELETE ON verification_prepared_action_authorizations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_conflict_set_members_append_only BEFORE UPDATE OR DELETE ON verification_action_conflict_set_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_conflict_events_append_only BEFORE UPDATE OR DELETE ON verification_conflict_key_events FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_budget_axes_append_only BEFORE UPDATE OR DELETE ON verification_budget_contract_axes FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_budget_ledger_append_only BEFORE UPDATE OR DELETE ON verification_budget_ledger_entries FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_subexecutions_append_only BEFORE UPDATE OR DELETE ON verification_action_subexecutions FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_action_recovery_receipts_append_only BEFORE UPDATE OR DELETE ON verification_action_recovery_receipts FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_action_receipt_bindings_append_only BEFORE UPDATE OR DELETE ON verification_action_capability_receipt_bindings FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_action_receipt_finalizations_append_only BEFORE UPDATE OR DELETE ON verification_action_capability_receipt_finalizations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_cleanup_obligations_append_only BEFORE UPDATE OR DELETE ON verification_cleanup_obligations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_oracle_assessments_append_only BEFORE UPDATE OR DELETE ON verification_oracle_assessments FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_oracle_census_members_append_only BEFORE UPDATE OR DELETE ON verification_oracle_census_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_campaign_adjudications_append_only BEFORE UPDATE OR DELETE ON verification_campaign_adjudications FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_campaign_terminals_append_only BEFORE UPDATE OR DELETE ON verification_campaign_terminal_decisions FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_campaign_coverage_receipts_append_only BEFORE UPDATE OR DELETE ON verification_campaign_coverage_receipts FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_campaign_coverage_results_append_only BEFORE UPDATE OR DELETE ON verification_campaign_coverage_results FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_wave_coverage_receipts_append_only BEFORE UPDATE OR DELETE ON verification_wave_coverage_receipts FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_wave_unassigned_results_append_only BEFORE UPDATE OR DELETE ON verification_wave_unassigned_coverage_results FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_fact_delta_bundles_append_only BEFORE UPDATE OR DELETE ON verification_fact_delta_bundles FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_claim_component_outcome_members_append_only BEFORE UPDATE OR DELETE ON hypothesis_objective_claim_component_outcome_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_objective_outcomes_append_only BEFORE UPDATE OR DELETE ON hypothesis_objective_outcome_receipts FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_objective_outcome_set_members_append_only BEFORE UPDATE OR DELETE ON hypothesis_objective_outcome_set_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_revision_adjudications_append_only BEFORE UPDATE OR DELETE ON hypothesis_revision_adjudications FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_shadow_obligations_append_only BEFORE UPDATE OR DELETE ON verification_campaign_shadow_evaluation_obligations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_shadow_items_append_only BEFORE UPDATE OR DELETE ON verification_campaign_shadow_evaluation_items FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_revision_terminals_append_only BEFORE UPDATE OR DELETE ON hypothesis_revision_terminal_decisions FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_fact_delta_consumptions_append_only BEFORE UPDATE OR DELETE ON fact_delta_consumptions FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_evolution_proposals_append_only BEFORE UPDATE OR DELETE ON hypothesis_evolution_proposals FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_evolution_decisions_append_only BEFORE UPDATE OR DELETE ON hypothesis_evolution_decisions FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_consolidation_batches_append_only BEFORE UPDATE OR DELETE ON hypothesis_consolidation_batches FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_consolidation_receipts_append_only BEFORE UPDATE OR DELETE ON hypothesis_consolidation_receipts FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_fixed_point_receipts_append_only BEFORE UPDATE OR DELETE ON hypothesis_fixed_point_receipts FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_enrichment_obligations_append_only BEFORE UPDATE OR DELETE ON enrichment_obligations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_refinement_obligations_append_only BEFORE UPDATE OR DELETE ON application_fact_refinement_obligations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_quarantine_events_append_only BEFORE UPDATE OR DELETE ON verification_authority_quarantine_events FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_quarantine_members_append_only BEFORE UPDATE OR DELETE ON verification_authority_quarantine_members FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_temporal_staleness_append_only BEFORE UPDATE OR DELETE ON verification_authority_temporal_staleness_events FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_re_adjudication_obligations_append_only BEFORE UPDATE OR DELETE ON hypothesis_re_adjudication_obligations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_correction_bundles_append_only BEFORE UPDATE OR DELETE ON verification_authority_correction_bundles FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_correction_consumptions_append_only BEFORE UPDATE OR DELETE ON verification_authority_correction_consumptions FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
CREATE TRIGGER verification_shadow_evaluations_append_only BEFORE UPDATE OR DELETE ON verification_campaign_shadow_evaluations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
-- Extend Plan B's single DEFERRABLE revision authority trigger in place.  The
-- candidate and server-validator branches are unchanged; only the branch that
-- Plan B intentionally left fail-closed for Plan C is installed here.
CREATE OR REPLACE FUNCTION enforce_hypothesis_revision_creating_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    creating attack_hypothesis_state_events%ROWTYPE;
    event_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO event_count FROM attack_hypothesis_state_events
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
       OR (NEW.predecessor_revision_id IS NOT NULL AND NOT EXISTS(
           SELECT 1 FROM attack_hypothesis_revisions predecessor
            WHERE predecessor.revision_id=NEW.predecessor_revision_id
              AND predecessor.root_id=NEW.root_id
              AND predecessor.operation_id=NEW.operation_id
              AND predecessor.organization_id=NEW.organization_id
              AND predecessor.revision_ordinal=NEW.revision_ordinal-1
       ))
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_PREDECESSOR_INVALID' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='candidate_analysis' AND (
        NEW.epistemic_state NOT IN ('proposed','supported','contested','inconclusive')
        OR ROW(creating.event_kind,NEW.epistemic_state) NOT IN (
            ROW('created','proposed'),ROW('supported','supported'),
            ROW('contested','contested'),ROW('inconclusive','inconclusive')
        )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_TERMINAL_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='candidate_analysis' AND (
        creating.authority_receipt_kind<>'candidate_gate_decision'
        OR NOT EXISTS(
            SELECT 1 FROM hypothesis_candidate_gate_decision_members mutation
             WHERE mutation.mutation_id=creating.server_decision_id
               AND mutation.operation_id=NEW.operation_id
               AND mutation.organization_id=NEW.organization_id
               AND mutation.root_id=NEW.root_id
               AND mutation.predecessor_revision_id IS NOT DISTINCT FROM NEW.predecessor_revision_id
               AND mutation.successor_revision_id=NEW.revision_id
               AND mutation.semantic_key_hash=NEW.semantic_key_hash
               AND mutation.successor_epistemic_state=NEW.epistemic_state
               AND mutation.origin_decision_hash=creating.server_decision_hash
               AND (mutation.route_kind IN ('reopen_historical','narrow_successor') OR EXISTS(
                   SELECT 1 FROM attack_hypotheses routed_root
                    WHERE routed_root.root_id=mutation.root_id
                      AND routed_root.root_kind=CASE mutation.route_kind
                          WHEN 'create_initial' THEN 'initial'
                          WHEN 'split' THEN 'split'
                          WHEN 'merge' THEN 'merge'
                          WHEN 'derive' THEN 'derive'
                      END
               ))
        )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_GATE_DECISION_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='server_validator' AND (
        NEW.epistemic_state<>'invalid'
        OR creating.authority_receipt_kind<>'server_validation'
        OR creating.authority_receipt_id IS NULL OR creating.authority_receipt_hash IS NULL
        OR creating.event_kind<>'invalidated'
        OR NOT EXISTS(
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
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_SERVER_VALIDATION_RECEIPT_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='hypothesis_revision_adjudication' AND (
        NEW.epistemic_state NOT IN ('verified','refuted') OR NEW.lifecycle_state<>'closed'
        OR creating.event_kind<>NEW.epistemic_state
        OR creating.authority_receipt_kind<>'revision_transition_decision'
        OR NOT EXISTS(
            SELECT 1
              FROM hypothesis_revision_terminal_decisions terminal
              JOIN hypothesis_revision_adjudications adjudication
                ON adjudication.revision_adjudication_id=terminal.revision_adjudication_id
              JOIN hypothesis_objective_outcome_set_seals outcome_set
                ON outcome_set.objective_outcome_set_seal_id=adjudication.objective_outcome_set_seal_id
             WHERE terminal.revision_terminal_decision_id=creating.authority_receipt_id
               AND terminal.decision_hash=creating.authority_receipt_hash
               AND terminal.state_event_id=creating.event_id
               AND terminal.terminal_successor_revision_id=NEW.revision_id
               AND terminal.hypothesis_revision_id=NEW.predecessor_revision_id
               AND terminal.operation_id=NEW.operation_id
               AND terminal.organization_id=NEW.organization_id
               AND terminal.decision=NEW.epistemic_state
               AND adjudication.outcome=terminal.decision
               AND adjudication.effective_valid_until>statement_timestamp()
               AND outcome_set.sealed_at IS NOT NULL
               AND NOT EXISTS(
                   SELECT 1
                     FROM hypothesis_objective_outcome_set_members outcome_member
                     JOIN verification_authority_quarantine_events quarantine
                       ON quarantine.objective_outcome_receipt_id=outcome_member.selected_current_outcome_id
                    WHERE outcome_member.objective_outcome_set_seal_id=outcome_set.objective_outcome_set_seal_id
               )
        )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_ADJUDICATION_AUTHORITY_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM attack_hypothesis_heads head
         WHERE head.root_id=NEW.root_id AND head.operation_id=NEW.operation_id
           AND head.organization_id=NEW.organization_id AND head.head_revision_id=NEW.revision_id
           AND head.head_revision_hash=NEW.revision_hash
           AND head.head_semantic_key_hash=NEW.semantic_key_hash
           AND head.head_epistemic_state=NEW.epistemic_state
           AND head.head_lifecycle_state=NEW.lifecycle_state
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_HEAD_CAS_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF (SELECT COUNT(*) FROM attack_hypothesis_verification_plans plan
         WHERE plan.revision_id=NEW.revision_id)<>1
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_VERIFICATION_PLAN_EXACT_ONE_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE TABLE verification_callback_obligations (
    callback_obligation_id UUID PRIMARY KEY,
    action_execution_id UUID NOT NULL REFERENCES verification_action_executions(action_execution_id) ON DELETE RESTRICT,
    obligation_ordinal INTEGER NOT NULL CHECK (obligation_ordinal>=0),
    callback_kind TEXT NOT NULL CHECK (BTRIM(callback_kind)<>''),
    status TEXT NOT NULL CHECK (status IN ('pending','observed','expired','failed')),
    residual_id UUID REFERENCES hypothesis_residual_risks(residual_id) ON DELETE RESTRICT,
    obligation_hash TEXT NOT NULL CHECK (obligation_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(action_execution_id,obligation_ordinal),
    CHECK ((status='observed')=(residual_id IS NULL))
);

CREATE TRIGGER verification_callback_obligations_append_only BEFORE UPDATE OR DELETE ON verification_callback_obligations FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();
