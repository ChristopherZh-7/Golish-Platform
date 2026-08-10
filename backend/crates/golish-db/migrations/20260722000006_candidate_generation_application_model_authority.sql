-- Extend strict Candidate Application Model provenance across FactDelta-opened
-- generations. Existing generation-zero rows remain valid and byte-compatible.

ALTER TABLE attack_wave_application_model_authorities
    ADD COLUMN source_consolidation_id UUID
        REFERENCES attack_wave_consolidations(id) ON DELETE RESTRICT,
    ADD COLUMN parent_wave_unit_id UUID
        REFERENCES attack_wave_units(id) ON DELETE RESTRICT,
    ADD COLUMN parent_input_authority_hash TEXT
        CHECK (
            parent_input_authority_hash IS NULL
            OR parent_input_authority_hash ~ '^sha256:[0-9a-f]{64}$'
        ),
    ADD CONSTRAINT attack_wave_application_model_authority_generation_shape CHECK (
        (
            source_consolidation_id IS NULL
            AND parent_wave_unit_id IS NULL
            AND parent_input_authority_hash IS NULL
        )
        OR (
            source_consolidation_id IS NOT NULL
            AND parent_wave_unit_id IS NOT NULL
            AND parent_input_authority_hash IS NOT NULL
        )
    ),
    ADD CONSTRAINT attack_wave_application_model_authority_parent_key UNIQUE (
        wave_unit_id,input_authority_hash
    ),
    ADD CONSTRAINT attack_wave_application_model_authority_parent_fk FOREIGN KEY (
        parent_wave_unit_id,parent_input_authority_hash
    ) REFERENCES attack_wave_application_model_authorities(
        wave_unit_id,input_authority_hash
    ) ON DELETE RESTRICT;

CREATE INDEX attack_wave_application_model_authorities_consolidation
    ON attack_wave_application_model_authorities(
        source_consolidation_id,parent_wave_unit_id
    ) WHERE source_consolidation_id IS NOT NULL;

CREATE OR REPLACE FUNCTION validate_attack_wave_application_model_authority()
RETURNS trigger AS $$
DECLARE
    operation_contract TEXT;
    wave_generation INTEGER;
    wave_entry_submission UUID;
    wave_entry_consolidation UUID;
    wave_entry_stage_kind TEXT;
    current_row application_model_current_revisions%ROWTYPE;
    revision_row application_model_revisions%ROWTYPE;
    handoff_row stage_handoffs%ROWTYPE;
    parent_authority attack_wave_application_model_authorities%ROWTYPE;
    expected_hash TEXT;
BEGIN
    SELECT state.application_model_contract,run.generation,
           unit.entry_deliverable_submission_id,unit.entry_consolidation_id,
           unit.entry_stage_kind
      INTO STRICT operation_contract,wave_generation,wave_entry_submission,
                  wave_entry_consolidation,wave_entry_stage_kind
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
       AND unit.manifest_hash=NEW.candidate_manifest_hash
       AND unit.manifest_frozen_at IS NOT NULL;

    IF operation_contract <> 'application_model_v1' THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_CONTRACT_MISMATCH';
    END IF;

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

    IF wave_generation=0 THEN
        IF wave_entry_stage_kind <> 'vuln_triage'
           OR wave_entry_submission IS NULL
           OR wave_entry_consolidation IS NOT NULL
           OR NEW.source_consolidation_id IS NOT NULL
           OR NEW.parent_wave_unit_id IS NOT NULL
           OR NEW.parent_input_authority_hash IS NOT NULL
        THEN
            RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_GENERATION_ZERO_ENTRY_MISMATCH';
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
    ELSE
        IF wave_generation < 1
           OR wave_entry_stage_kind IS NOT NULL
           OR wave_entry_submission IS NOT NULL
           OR wave_entry_consolidation IS DISTINCT FROM NEW.source_consolidation_id
           OR NEW.source_consolidation_id IS NULL
           OR NEW.parent_wave_unit_id IS NULL
           OR NEW.parent_input_authority_hash IS NULL
        THEN
            RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_NEXT_WAVE_ENTRY_MISMATCH';
        END IF;

        SELECT parent.* INTO STRICT parent_authority
          FROM attack_wave_application_model_authorities AS parent
          JOIN attack_wave_units AS parent_unit
            ON parent_unit.id=parent.wave_unit_id
           AND parent_unit.wave_run_id=parent.wave_run_id
           AND parent_unit.operation_id=parent.operation_id
           AND parent_unit.scope_snapshot_id=parent.scope_snapshot_id
           AND parent_unit.organization_id=parent.organization_id
           AND parent_unit.manifest_hash=parent.candidate_manifest_hash
           AND parent_unit.manifest_frozen_at IS NOT NULL
          JOIN attack_wave_runs AS parent_wave
            ON parent_wave.id=parent_unit.wave_run_id
           AND parent_wave.operation_id=parent_unit.operation_id
           AND parent_wave.scope_snapshot_id=parent_unit.scope_snapshot_id
           AND parent_wave.generation=wave_generation-1
          JOIN attack_wave_consolidations AS consolidation
            ON consolidation.id=NEW.source_consolidation_id
           AND consolidation.operation_id=NEW.operation_id
           AND consolidation.scope_snapshot_id=NEW.scope_snapshot_id
           AND consolidation.source_wave_run_id=parent_wave.id
           AND consolidation.source_generation=parent_wave.generation
           AND consolidation.decision_kind='opened_next_wave'
           AND consolidation.target_wave_run_id=NEW.wave_run_id
           AND consolidation.target_generation=wave_generation
          JOIN attack_wave_consolidation_members AS member
            ON member.consolidation_id=consolidation.id
           AND member.source_wave_run_id=parent_wave.id
           AND member.source_wave_unit_id=parent_unit.id
           AND member.organization_id=NEW.organization_id
           AND member.target_wave_run_id=NEW.wave_run_id
           AND member.target_wave_unit_id=NEW.wave_unit_id
           AND member.target_work_item_id IS NOT NULL
           AND member.route_kind='direct'
          JOIN attack_fact_deltas AS delta
            ON delta.id=member.fact_delta_id
           AND delta.operation_id=NEW.operation_id
           AND delta.scope_snapshot_id=NEW.scope_snapshot_id
           AND delta.wave_run_id=parent_wave.id
           AND delta.wave_unit_id=parent_unit.id
           AND delta.organization_id=NEW.organization_id
           AND delta.status='consumed'
           AND delta.consumed_by_wave_run_id=NEW.wave_run_id
         WHERE parent.wave_unit_id=NEW.parent_wave_unit_id
           AND parent.input_authority_hash=NEW.parent_input_authority_hash
           AND parent.operation_id=NEW.operation_id
           AND parent.scope_snapshot_id=NEW.scope_snapshot_id
           AND parent.organization_id=NEW.organization_id
           AND parent.source_vuln_handoff_id=NEW.source_vuln_handoff_id
           AND parent.application_model_manifest_id=NEW.application_model_manifest_id
           AND parent.application_model_revision_id=NEW.application_model_revision_id
           AND parent.application_model_stage_execution_id=NEW.application_model_stage_execution_id
           AND parent.application_model_stage_run_unit_id=NEW.application_model_stage_run_unit_id
           AND parent.application_model_deliverable_submission_id=NEW.application_model_deliverable_submission_id
           AND parent.application_model_handoff_id=NEW.application_model_handoff_id
           AND parent.application_model_manifest_hash=NEW.application_model_manifest_hash
           AND parent.application_model_model_hash=NEW.application_model_model_hash
           AND parent.application_model_replay_material_hash=NEW.application_model_replay_material_hash
           AND parent.application_model_gate_decision_hash=NEW.application_model_gate_decision_hash
         LIMIT 1;

        IF EXISTS (
            SELECT 1
              FROM attack_candidate_work_items AS work_item
             WHERE work_item.wave_unit_id=NEW.wave_unit_id
               AND work_item.operation_id=NEW.operation_id
               AND work_item.scope_snapshot_id=NEW.scope_snapshot_id
               AND work_item.organization_id=NEW.organization_id
               AND NOT EXISTS (
                    SELECT 1
                      FROM attack_wave_consolidation_members AS member
                     WHERE member.consolidation_id=NEW.source_consolidation_id
                       AND member.organization_id=NEW.organization_id
                       AND member.target_wave_run_id=NEW.wave_run_id
                       AND member.target_wave_unit_id=NEW.wave_unit_id
                       AND member.target_work_item_id=work_item.id
                       AND member.route_kind='direct'
               )
        ) THEN
            RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_NEXT_WAVE_DENOMINATOR_MISMATCH';
        END IF;

        expected_hash := application_model_sha256_jsonb(jsonb_build_object(
            'schema_version','candidate_input_authority.v2',
            'generation',wave_generation,
            'candidate_manifest_hash',NEW.candidate_manifest_hash,
            'source_consolidation_id',NEW.source_consolidation_id,
            'parent_wave_unit_id',NEW.parent_wave_unit_id,
            'parent_input_authority_hash',NEW.parent_input_authority_hash,
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
    END IF;

    IF NEW.input_authority_hash <> expected_hash THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_AUTHORITY_HASH_MISMATCH';
    END IF;
    RETURN NEW;
EXCEPTION
    WHEN NO_DATA_FOUND THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_AUTHORITY_INVALID';
END;
$$ LANGUAGE plpgsql;
