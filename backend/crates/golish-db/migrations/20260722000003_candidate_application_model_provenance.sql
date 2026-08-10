-- Dormant Candidate -> Application Model provenance foundation.
--
-- Existing operations stay byte-compatible.  Only explicitly-created
-- application_model_v1 operations may use the strict compound Candidate entry;
-- this migration does not register a stage or alter the operation graph.

ALTER TABLE operation_state
    ADD COLUMN application_model_contract TEXT NOT NULL DEFAULT 'legacy_no_model'
        CHECK (application_model_contract IN ('legacy_no_model', 'application_model_v1'));

CREATE FUNCTION reject_application_model_contract_change()
RETURNS trigger AS $$
BEGIN
    IF NEW.application_model_contract IS DISTINCT FROM OLD.application_model_contract THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_CONTRACT_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_state_application_model_contract_immutable
BEFORE UPDATE OF application_model_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION reject_application_model_contract_change();

CREATE TABLE attack_wave_application_model_authorities (
    wave_unit_id UUID PRIMARY KEY,
    wave_run_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_vuln_handoff_id UUID NOT NULL REFERENCES stage_handoffs(id) ON DELETE RESTRICT,
    application_model_manifest_id UUID NOT NULL,
    application_model_revision_id UUID NOT NULL,
    application_model_stage_execution_id UUID NOT NULL,
    application_model_stage_run_unit_id UUID NOT NULL,
    application_model_deliverable_submission_id UUID NOT NULL,
    application_model_handoff_id UUID NOT NULL REFERENCES stage_handoffs(id) ON DELETE RESTRICT,
    application_model_manifest_hash TEXT NOT NULL
        CHECK (application_model_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    application_model_model_hash TEXT NOT NULL
        CHECK (application_model_model_hash ~ '^sha256:[0-9a-f]{64}$'),
    application_model_replay_material_hash TEXT NOT NULL
        CHECK (application_model_replay_material_hash ~ '^sha256:[0-9a-f]{64}$'),
    application_model_gate_decision_hash TEXT NOT NULL
        CHECK (application_model_gate_decision_hash ~ '^sha256:[0-9a-f]{64}$'),
    candidate_manifest_hash TEXT NOT NULL
        CHECK (candidate_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    input_authority_hash TEXT NOT NULL
        CHECK (input_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    bound_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (wave_unit_id, application_model_revision_id, input_authority_hash),
    FOREIGN KEY (
        wave_unit_id,wave_run_id,operation_id,scope_snapshot_id,organization_id
    ) REFERENCES attack_wave_units(
        id,wave_run_id,operation_id,scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        application_model_manifest_id,operation_id,scope_snapshot_id,
        application_model_stage_execution_id,application_model_stage_run_unit_id,
        organization_id
    ) REFERENCES application_model_manifests(
        id,operation_id,scope_snapshot_id,stage_execution_id,stage_run_unit_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        application_model_revision_id,application_model_manifest_id,operation_id,
        scope_snapshot_id,application_model_stage_execution_id,
        application_model_stage_run_unit_id,organization_id
    ) REFERENCES application_model_revisions(
        id,manifest_id,operation_id,scope_snapshot_id,stage_execution_id,
        stage_run_unit_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (application_model_deliverable_submission_id)
        REFERENCES stage_deliverable_submissions(id) ON DELETE RESTRICT
);

CREATE INDEX attack_wave_application_model_authorities_owner
    ON attack_wave_application_model_authorities(
        operation_id,scope_snapshot_id,organization_id,wave_run_id
    );

CREATE FUNCTION validate_attack_wave_application_model_authority()
RETURNS trigger AS $$
DECLARE
    operation_contract TEXT;
    wave_generation INTEGER;
    wave_entry_submission UUID;
    current_row application_model_current_revisions%ROWTYPE;
    revision_row application_model_revisions%ROWTYPE;
    handoff_row stage_handoffs%ROWTYPE;
    expected_hash TEXT;
BEGIN
    SELECT state.application_model_contract,run.generation,unit.entry_deliverable_submission_id
      INTO STRICT operation_contract,wave_generation,wave_entry_submission
      FROM attack_wave_units AS unit
      JOIN attack_wave_runs AS run
        ON run.id=unit.wave_run_id
       AND run.operation_id=unit.operation_id
       AND run.scope_snapshot_id=unit.scope_snapshot_id
      JOIN operation_state AS state ON state.operation_id=unit.operation_id
     WHERE unit.id=NEW.wave_unit_id
       AND unit.wave_run_id=NEW.wave_run_id
       AND unit.operation_id=NEW.operation_id
       AND unit.scope_snapshot_id=NEW.scope_snapshot_id
       AND unit.organization_id=NEW.organization_id
       AND unit.entry_stage_kind='vuln_triage'
       AND unit.manifest_hash=NEW.candidate_manifest_hash
       AND unit.manifest_frozen_at IS NOT NULL;

    IF operation_contract <> 'application_model_v1' OR wave_generation <> 0 THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_CONTRACT_MISMATCH';
    END IF;

    SELECT * INTO STRICT handoff_row
      FROM stage_handoffs
     WHERE id=NEW.source_vuln_handoff_id
       AND operation_id=NEW.operation_id
       AND scope_snapshot_id=NEW.scope_snapshot_id
       AND organization_id=NEW.organization_id
       AND from_stage_kind='vuln_triage'
       AND deliverable_submission_id=wave_entry_submission
       AND invalidated_at IS NULL;

    SELECT * INTO STRICT current_row
      FROM application_model_current_revisions
     WHERE manifest_id=NEW.application_model_manifest_id
       AND revision_id=NEW.application_model_revision_id
       AND authority_kind='model'
       AND stage_handoff_id=NEW.application_model_handoff_id
       AND deliverable_submission_id=NEW.application_model_deliverable_submission_id
       AND manifest_hash=NEW.application_model_manifest_hash
       AND model_hash=NEW.application_model_model_hash
       AND replay_material_hash=NEW.application_model_replay_material_hash
       AND gate_decision_hash=NEW.application_model_gate_decision_hash;

    SELECT * INTO STRICT revision_row
      FROM application_model_revisions
     WHERE id=NEW.application_model_revision_id
       AND manifest_id=NEW.application_model_manifest_id
       AND operation_id=NEW.operation_id
       AND scope_snapshot_id=NEW.scope_snapshot_id
       AND stage_execution_id=NEW.application_model_stage_execution_id
       AND stage_run_unit_id=NEW.application_model_stage_run_unit_id
       AND organization_id=NEW.organization_id
       AND source_submission_id=NEW.application_model_deliverable_submission_id
       AND status='final'
       AND row_version=1
       AND finalized_at IS NOT NULL
       AND model_hash=NEW.application_model_model_hash
       AND replay_material_hash=NEW.application_model_replay_material_hash;

    PERFORM 1
      FROM stage_handoffs AS model_handoff
      JOIN stage_run_units AS model_unit
        ON model_unit.id=model_handoff.source_stage_run_unit_id
       AND model_unit.operation_id=model_handoff.operation_id
       AND model_unit.stage_execution_id=model_handoff.stage_execution_id
       AND model_unit.organization_id=model_handoff.organization_id
       AND model_unit.stage_kind=model_handoff.from_stage_kind
     WHERE model_handoff.id=NEW.application_model_handoff_id
       AND model_handoff.operation_id=NEW.operation_id
       AND model_handoff.scope_snapshot_id=NEW.scope_snapshot_id
       AND model_handoff.organization_id=NEW.organization_id
       AND model_handoff.from_stage_kind='application_understanding'
       AND model_handoff.stage_execution_id=NEW.application_model_stage_execution_id
       AND model_handoff.source_stage_run_unit_id=NEW.application_model_stage_run_unit_id
       AND model_handoff.deliverable_submission_id=NEW.application_model_deliverable_submission_id
       AND ('sha256:' || model_handoff.unit_gate_decision_hash)=
            NEW.application_model_gate_decision_hash
       AND model_handoff.invalidated_at IS NULL
       AND model_unit.status='passed';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_HANDOFF_INVALID';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM application_model_manifest_inputs AS input
         WHERE input.manifest_id=NEW.application_model_manifest_id
           AND input.operation_id=NEW.operation_id
           AND input.scope_snapshot_id=NEW.scope_snapshot_id
           AND input.organization_id=NEW.organization_id
           AND input.source_handoff_id=NEW.source_vuln_handoff_id
    ) THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_DENOMINATOR_MISMATCH';
    END IF;

    expected_hash := application_model_sha256_jsonb(jsonb_build_object(
        'schema_version','candidate_input_authority.v1',
        'candidate_manifest_hash',NEW.candidate_manifest_hash,
        'source_vuln_handoff_id',NEW.source_vuln_handoff_id,
        'application_model_manifest_id',NEW.application_model_manifest_id,
        'application_model_revision_id',NEW.application_model_revision_id,
        'application_model_stage_execution_id',NEW.application_model_stage_execution_id,
        'application_model_stage_run_unit_id',NEW.application_model_stage_run_unit_id,
        'application_model_deliverable_submission_id',NEW.application_model_deliverable_submission_id,
        'application_model_handoff_id',NEW.application_model_handoff_id,
        'application_model_manifest_hash',NEW.application_model_manifest_hash,
        'application_model_model_hash',NEW.application_model_model_hash,
        'application_model_replay_material_hash',NEW.application_model_replay_material_hash,
        'application_model_gate_decision_hash',NEW.application_model_gate_decision_hash
    ));
    IF NEW.input_authority_hash <> expected_hash THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_AUTHORITY_HASH_MISMATCH';
    END IF;
    RETURN NEW;
EXCEPTION
    WHEN NO_DATA_FOUND THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_AUTHORITY_INVALID';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_application_model_authority_valid
BEFORE INSERT ON attack_wave_application_model_authorities
FOR EACH ROW EXECUTE FUNCTION validate_attack_wave_application_model_authority();

CREATE TRIGGER attack_wave_application_model_authority_immutable
BEFORE UPDATE OR DELETE ON attack_wave_application_model_authorities
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();

CREATE TABLE attack_candidate_application_model_refs (
    candidate_id UUID PRIMARY KEY REFERENCES attack_candidates(candidate_id) ON DELETE RESTRICT,
    wave_unit_id UUID NOT NULL,
    application_model_revision_id UUID NOT NULL,
    input_authority_hash TEXT NOT NULL
        CHECK (input_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (wave_unit_id,application_model_revision_id,input_authority_hash)
        REFERENCES attack_wave_application_model_authorities(
            wave_unit_id,application_model_revision_id,input_authority_hash
        ) ON DELETE RESTRICT
);

CREATE FUNCTION validate_attack_candidate_application_model_ref()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM attack_candidates AS candidate
         WHERE candidate.candidate_id=NEW.candidate_id
           AND candidate.wave_unit_id=NEW.wave_unit_id
           AND candidate.operation_uuid IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_REF_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_candidate_application_model_ref_valid
AFTER INSERT ON attack_candidate_application_model_refs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION validate_attack_candidate_application_model_ref();

CREATE TRIGGER attack_candidate_application_model_ref_immutable
BEFORE UPDATE OR DELETE ON attack_candidate_application_model_refs
FOR EACH ROW EXECUTE FUNCTION application_model_reject_immutable_change();
