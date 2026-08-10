-- Let a Candidate-only successor adopt an exact final Application Model from
-- its immutable Application Understanding stage-fork input. Existing local
-- AU -> Candidate authority rows retain their v1/v2 hashes and owner shape.

ALTER TABLE operation_stage_fork_inputs
    DROP CONSTRAINT operation_stage_fork_inputs_source_stage_kind_check;

ALTER TABLE operation_stage_fork_inputs
    ADD CONSTRAINT operation_stage_fork_inputs_source_stage_kind_check
        CHECK (operation_stage_fork_stage_rank(source_stage_kind) BETWEEN 1 AND 6);

ALTER TABLE attack_wave_application_model_authorities
    ADD COLUMN application_model_operation_id UUID,
    ADD COLUMN application_model_scope_snapshot_id UUID,
    ADD COLUMN application_model_stage_fork_input_id UUID
        REFERENCES operation_stage_fork_inputs(id) ON DELETE RESTRICT;

ALTER TABLE attack_wave_application_model_authorities
    DISABLE TRIGGER attack_wave_application_model_authority_immutable;

UPDATE attack_wave_application_model_authorities
   SET application_model_operation_id=operation_id,
       application_model_scope_snapshot_id=scope_snapshot_id
 WHERE application_model_operation_id IS NULL
    OR application_model_scope_snapshot_id IS NULL;

ALTER TABLE attack_wave_application_model_authorities
    ENABLE TRIGGER attack_wave_application_model_authority_immutable;

ALTER TABLE attack_wave_application_model_authorities
    ALTER COLUMN application_model_operation_id SET NOT NULL,
    ALTER COLUMN application_model_scope_snapshot_id SET NOT NULL;

DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    FOR constraint_name IN
        SELECT constraint_row.conname
          FROM pg_constraint AS constraint_row
         WHERE constraint_row.conrelid=
                   'attack_wave_application_model_authorities'::regclass
           AND constraint_row.contype='f'
           AND constraint_row.confrelid IN (
                 'application_model_manifests'::regclass,
                 'application_model_revisions'::regclass
           )
    LOOP
        EXECUTE format(
            'ALTER TABLE attack_wave_application_model_authorities DROP CONSTRAINT %I',
            constraint_name
        );
    END LOOP;
END;
$$;

ALTER TABLE attack_wave_application_model_authorities
    ADD CONSTRAINT attack_wave_application_model_authority_manifest_owner_fk
        FOREIGN KEY (
            application_model_manifest_id,
            application_model_operation_id,
            application_model_scope_snapshot_id,
            application_model_stage_execution_id,
            application_model_stage_run_unit_id,
            organization_id
        ) REFERENCES application_model_manifests(
            id,operation_id,scope_snapshot_id,stage_execution_id,
            stage_run_unit_id,organization_id
        ) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_wave_application_model_authority_revision_owner_fk
        FOREIGN KEY (
            application_model_revision_id,
            application_model_manifest_id,
            application_model_operation_id,
            application_model_scope_snapshot_id,
            application_model_stage_execution_id,
            application_model_stage_run_unit_id,
            organization_id
        ) REFERENCES application_model_revisions(
            id,manifest_id,operation_id,scope_snapshot_id,stage_execution_id,
            stage_run_unit_id,organization_id
        ) ON DELETE RESTRICT,
    ADD CONSTRAINT attack_wave_application_model_authority_owner_shape CHECK (
        (
            application_model_operation_id=operation_id
            AND application_model_scope_snapshot_id=scope_snapshot_id
            AND application_model_stage_fork_input_id IS NULL
        )
        OR (
            application_model_stage_fork_input_id IS NOT NULL
            AND (
                application_model_operation_id<>operation_id
                OR application_model_scope_snapshot_id<>scope_snapshot_id
            )
        )
    );

CREATE OR REPLACE FUNCTION validate_attack_wave_application_model_authority()
RETURNS trigger AS $$
DECLARE
    operation_contract TEXT;
    wave_generation INTEGER;
    wave_entry_submission UUID;
    wave_entry_consolidation UUID;
    wave_entry_stage_kind TEXT;
    wave_entry_stage_fork_input_id UUID;
    current_row application_model_current_revisions%ROWTYPE;
    revision_row application_model_revisions%ROWTYPE;
    handoff_row stage_handoffs%ROWTYPE;
    parent_authority attack_wave_application_model_authorities%ROWTYPE;
    model_fork_input operation_stage_fork_inputs%ROWTYPE;
    vuln_fork_input operation_stage_fork_inputs%ROWTYPE;
    fork_row operation_stage_forks%ROWTYPE;
    expected_hash TEXT;
BEGIN
    SELECT state.application_model_contract,run.generation,
           unit.entry_deliverable_submission_id,unit.entry_consolidation_id,
           unit.entry_stage_kind,unit.entry_stage_fork_input_id
      INTO STRICT operation_contract,wave_generation,wave_entry_submission,
                  wave_entry_consolidation,wave_entry_stage_kind,
                  wave_entry_stage_fork_input_id
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
       AND operation_id=NEW.application_model_operation_id
       AND scope_snapshot_id=NEW.application_model_scope_snapshot_id
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
       AND model_handoff.operation_id=NEW.application_model_operation_id
       AND model_handoff.scope_snapshot_id=NEW.application_model_scope_snapshot_id
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
           AND input.operation_id=NEW.application_model_operation_id
           AND input.scope_snapshot_id=NEW.application_model_scope_snapshot_id
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

        IF NEW.application_model_stage_fork_input_id IS NULL THEN
            IF NEW.application_model_operation_id<>NEW.operation_id
               OR NEW.application_model_scope_snapshot_id<>NEW.scope_snapshot_id
               OR wave_entry_stage_fork_input_id IS NOT NULL
            THEN
                RAISE EXCEPTION 'ATTACK_CANDIDATE_APPLICATION_MODEL_LOCAL_OWNER_MISMATCH';
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
            SELECT * INTO STRICT model_fork_input
              FROM operation_stage_fork_inputs AS input
             WHERE input.id=NEW.application_model_stage_fork_input_id
               AND input.operation_id=NEW.operation_id
               AND input.target_scope_snapshot_id=NEW.scope_snapshot_id
               AND input.source_operation_id=NEW.application_model_operation_id
               AND input.source_scope_snapshot_id=NEW.application_model_scope_snapshot_id
               AND input.organization_id=NEW.organization_id
               AND input.source_stage_kind='application_understanding'
               AND input.source_stage_execution_id=NEW.application_model_stage_execution_id
               AND input.source_stage_run_unit_id=NEW.application_model_stage_run_unit_id
               AND input.source_deliverable_submission_id=
                    NEW.application_model_deliverable_submission_id
               AND input.source_handoff_id=NEW.application_model_handoff_id;

            SELECT * INTO STRICT vuln_fork_input
              FROM operation_stage_fork_inputs AS input
             WHERE input.id=wave_entry_stage_fork_input_id
               AND input.operation_id=NEW.operation_id
               AND input.target_scope_snapshot_id=NEW.scope_snapshot_id
               AND input.source_operation_id=NEW.application_model_operation_id
               AND input.source_scope_snapshot_id=NEW.application_model_scope_snapshot_id
               AND input.organization_id=NEW.organization_id
               AND input.source_stage_kind='vuln_triage'
               AND input.source_deliverable_submission_id=wave_entry_submission
               AND input.source_handoff_id=NEW.source_vuln_handoff_id;

            SELECT fork.* INTO STRICT fork_row
              FROM operation_stage_forks AS fork
              JOIN operation_state AS source_operation
                ON source_operation.operation_id=fork.source_operation_id
               AND source_operation.superseded_by IS NULL
             WHERE fork.operation_id=NEW.operation_id
               AND fork.source_operation_id=NEW.application_model_operation_id
               AND fork.source_scope_snapshot_id=NEW.application_model_scope_snapshot_id
               AND fork.target_scope_snapshot_id=NEW.scope_snapshot_id
               AND 'application_understanding'=ANY(fork.adopted_stage_kinds);

            expected_hash := application_model_sha256_jsonb(jsonb_build_object(
                'schema_version','candidate_input_authority.v3',
                'generation',0,
                'operation_id',NEW.operation_id,
                'scope_snapshot_id',NEW.scope_snapshot_id,
                'wave_run_id',NEW.wave_run_id,
                'wave_unit_id',NEW.wave_unit_id,
                'organization_id',NEW.organization_id,
                'candidate_manifest_hash',NEW.candidate_manifest_hash,
                'source_vuln_handoff_id',NEW.source_vuln_handoff_id,
                'source_vuln_stage_fork_input_id',vuln_fork_input.id,
                'source_vuln_stage_fork_input_hash',vuln_fork_input.manifest_input_sha256,
                'application_model_operation_id',NEW.application_model_operation_id,
                'application_model_scope_snapshot_id',NEW.application_model_scope_snapshot_id,
                'application_model_stage_fork_input_id',model_fork_input.id,
                'application_model_stage_fork_input_hash',model_fork_input.manifest_input_sha256,
                'stage_fork_manifest_hash',fork_row.manifest_sha256,
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
           AND parent.application_model_operation_id=NEW.application_model_operation_id
           AND parent.application_model_scope_snapshot_id=NEW.application_model_scope_snapshot_id
           AND parent.application_model_stage_fork_input_id IS NOT DISTINCT FROM
                NEW.application_model_stage_fork_input_id
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
