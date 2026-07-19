-- Candidate generation-zero Wave entry backed by one exact immutable stage
-- fork input. Ordinary same-operation handoffs and FactDelta follow-on waves
-- retain their existing contracts.

ALTER TABLE operation_stage_fork_inputs
    ADD CONSTRAINT operation_stage_fork_inputs_wave_entry_unique UNIQUE (
        id,
        operation_id,
        target_scope_snapshot_id,
        organization_id
    );

ALTER TABLE attack_wave_units
    ADD COLUMN entry_stage_fork_input_id UUID,
    DROP CONSTRAINT attack_wave_units_entry_shape_check,
    ADD CONSTRAINT attack_wave_units_entry_shape_check CHECK (
        num_nonnulls(
            entry_consolidation_id,
            entry_stage_fork_input_id,
            entry_stage_execution_id
        ) = 1
        AND (
            entry_stage_execution_id IS NULL
            OR (
                entry_stage_run_unit_id IS NOT NULL
                AND entry_deliverable_submission_id IS NOT NULL
                AND entry_stage_kind = 'vuln_triage'
            )
        )
        AND (
            entry_stage_execution_id IS NOT NULL
            OR (
                entry_stage_run_unit_id IS NULL
                AND entry_deliverable_submission_id IS NULL
                AND entry_stage_kind IS NULL
            )
        )
    ),
    ADD CONSTRAINT attack_wave_units_stage_fork_input_fk FOREIGN KEY (
        entry_stage_fork_input_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES operation_stage_fork_inputs (
        id,
        operation_id,
        target_scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT;

DROP TRIGGER attack_wave_units_require_final_pass_entry ON attack_wave_units;

CREATE OR REPLACE FUNCTION enforce_attack_wave_entry_final_pass()
RETURNS trigger AS $$
BEGIN
    IF NEW.entry_stage_fork_input_id IS NOT NULL THEN
        IF NOT EXISTS (
            SELECT 1
              FROM operation_stage_fork_inputs AS vuln_input
              JOIN operation_stage_forks AS fork
                ON fork.operation_id=vuln_input.operation_id
               AND fork.source_operation_id=vuln_input.source_operation_id
               AND fork.entry_stage='attack_candidate'
              JOIN operation_stage_fork_inputs AS enumeration_input
                ON enumeration_input.operation_id=vuln_input.operation_id
               AND enumeration_input.source_operation_id=vuln_input.source_operation_id
               AND enumeration_input.organization_id=vuln_input.organization_id
               AND enumeration_input.source_stage_kind='enumeration'
              JOIN operation_state AS source_operation
                ON source_operation.operation_id=vuln_input.source_operation_id
               AND source_operation.superseded_by IS NULL
              JOIN stage_handoffs AS vuln_handoff
                ON vuln_handoff.id=vuln_input.source_handoff_id
               AND vuln_handoff.operation_id=vuln_input.source_operation_id
               AND vuln_handoff.organization_id=vuln_input.organization_id
               AND vuln_handoff.from_stage_kind='vuln_triage'
               AND vuln_handoff.invalidated_at IS NULL
              JOIN stage_handoffs AS enumeration_handoff
                ON enumeration_handoff.id=enumeration_input.source_handoff_id
               AND enumeration_handoff.operation_id=enumeration_input.source_operation_id
               AND enumeration_handoff.organization_id=enumeration_input.organization_id
               AND enumeration_handoff.from_stage_kind='enumeration'
               AND enumeration_handoff.invalidated_at IS NULL
             WHERE vuln_input.id=NEW.entry_stage_fork_input_id
               AND vuln_input.operation_id=NEW.operation_id
               AND vuln_input.target_scope_snapshot_id=NEW.scope_snapshot_id
               AND vuln_input.organization_id=NEW.organization_id
               AND vuln_input.source_stage_kind='vuln_triage'
        ) THEN
            RAISE EXCEPTION 'attack fork wave entry requires exact Enumeration/Vuln final-seal inputs';
        END IF;
    ELSIF NEW.entry_consolidation_id IS NULL THEN
        IF NOT EXISTS (
            SELECT 1
              FROM stage_run_units AS source_unit
              JOIN stage_handoffs AS handoff
                ON handoff.operation_id = source_unit.operation_id
               AND handoff.scope_snapshot_id = source_unit.scope_snapshot_id
               AND handoff.organization_id = source_unit.organization_id
               AND handoff.source_stage_run_unit_id = source_unit.id
               AND handoff.deliverable_submission_id = NEW.entry_deliverable_submission_id
               AND handoff.invalidated_at IS NULL
             WHERE source_unit.id = NEW.entry_stage_run_unit_id
               AND source_unit.operation_id = NEW.operation_id
               AND source_unit.stage_execution_id = NEW.entry_stage_execution_id
               AND source_unit.scope_snapshot_id = NEW.scope_snapshot_id
               AND source_unit.organization_id = NEW.organization_id
               AND source_unit.stage_kind = NEW.entry_stage_kind
               AND source_unit.status = 'passed'
               AND source_unit.terminal_at IS NOT NULL
        ) THEN
            RAISE EXCEPTION 'attack wave entry requires exact final-passed StageRunUnit and immutable handoff';
        END IF;
    ELSIF NOT EXISTS (
        SELECT 1
          FROM attack_wave_consolidations AS consolidation
          JOIN attack_wave_runs AS target_wave
            ON target_wave.id = consolidation.target_wave_run_id
           AND target_wave.operation_id = consolidation.operation_id
           AND target_wave.scope_snapshot_id = consolidation.scope_snapshot_id
         WHERE consolidation.id = NEW.entry_consolidation_id
           AND consolidation.decision_kind = 'opened_next_wave'
           AND consolidation.target_wave_run_id = NEW.wave_run_id
           AND consolidation.operation_id = NEW.operation_id
           AND consolidation.scope_snapshot_id = NEW.scope_snapshot_id
           AND consolidation.target_generation = target_wave.generation
    ) THEN
        RAISE EXCEPTION 'attack follow-on wave entry requires exact immutable FactDelta consolidation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_units_require_final_pass_entry
BEFORE INSERT OR UPDATE OF operation_id, scope_snapshot_id, organization_id,
    entry_stage_execution_id, entry_stage_run_unit_id,
    entry_deliverable_submission_id, entry_stage_kind, entry_consolidation_id,
    entry_stage_fork_input_id
ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION enforce_attack_wave_entry_final_pass();

-- Preserve the ordinary same-operation evidence rule. The only exception is
-- an exact generation-zero fork WaveUnit and only for its frozen
-- observation/support lineage. Attempt/FactDelta/Residual evidence tables are
-- intentionally not covered by this branch.
CREATE OR REPLACE FUNCTION enforce_attack_evidence_owner()
RETURNS trigger AS $$
DECLARE
    owner_id UUID;
    owner_operation_id UUID;
    owner_organization_id UUID;
    owner_target_live_id UUID;
    owner_wave_unit_id UUID;
    owner_work_item_id UUID;
    evidence_role TEXT;
    evidence_run_id UUID;
    evidence_organization_id UUID;
    evidence_target_id UUID;
    fork_allowed BOOLEAN := FALSE;
BEGIN
    owner_id := (to_jsonb(NEW) ->> TG_ARGV[3])::UUID;
    EXECUTE format(
        'SELECT %I, organization_id, target_live_id FROM %I WHERE %I = $1',
        TG_ARGV[2], TG_ARGV[0], TG_ARGV[1]
    ) INTO owner_operation_id, owner_organization_id, owner_target_live_id
      USING owner_id;
    IF owner_operation_id IS NULL THEN
        RAISE EXCEPTION 'attack evidence owner row is missing';
    END IF;

    SELECT audit_role,run_id,NULLIF(detail ->> 'organization_id', '')::UUID,target_id
      INTO evidence_role,evidence_run_id,evidence_organization_id,evidence_target_id
      FROM audit_log WHERE id=NEW.evidence_id;
    IF NOT FOUND OR evidence_role IS DISTINCT FROM 'evidence'
        OR evidence_organization_id IS DISTINCT FROM owner_organization_id
        OR (evidence_target_id IS NOT NULL
            AND evidence_target_id IS DISTINCT FROM owner_target_live_id)
    THEN
        RAISE EXCEPTION 'audit evidence does not match attack owner organization or target';
    END IF;
    IF evidence_run_id IS NOT DISTINCT FROM owner_operation_id THEN
        RETURN NEW;
    END IF;

    IF TG_ARGV[0] = 'attack_candidate_seeds' THEN
        SELECT wave_unit_id INTO owner_wave_unit_id
          FROM attack_candidate_seeds WHERE id=owner_id;
    ELSIF TG_ARGV[0] = 'attack_candidate_work_items' THEN
        SELECT wave_unit_id,id INTO owner_wave_unit_id,owner_work_item_id
          FROM attack_candidate_work_items WHERE id=owner_id;
    ELSIF TG_ARGV[0] = 'attack_candidates' THEN
        SELECT item.wave_unit_id,item.id
          INTO owner_wave_unit_id,owner_work_item_id
          FROM attack_candidates AS candidate
          JOIN attack_candidate_work_items AS item
            ON item.id=candidate.source_work_item_id
           AND item.operation_id=candidate.operation_uuid
         WHERE candidate.candidate_id=owner_id;
    ELSE
        RAISE EXCEPTION 'audit evidence does not match attack owner operation';
    END IF;

    SELECT EXISTS (
        SELECT 1
          FROM attack_wave_units AS wave_unit
          JOIN operation_stage_fork_inputs AS vuln_input
            ON vuln_input.id=wave_unit.entry_stage_fork_input_id
           AND vuln_input.operation_id=wave_unit.operation_id
           AND vuln_input.target_scope_snapshot_id=wave_unit.scope_snapshot_id
           AND vuln_input.organization_id=wave_unit.organization_id
           AND vuln_input.source_stage_kind='vuln_triage'
          JOIN operation_stage_fork_inputs AS enumeration_input
            ON enumeration_input.operation_id=vuln_input.operation_id
           AND enumeration_input.source_operation_id=vuln_input.source_operation_id
           AND enumeration_input.organization_id=vuln_input.organization_id
           AND enumeration_input.source_stage_kind='enumeration'
         WHERE wave_unit.id=owner_wave_unit_id
           AND wave_unit.operation_id=owner_operation_id
           AND wave_unit.organization_id=owner_organization_id
           AND evidence_run_id=vuln_input.source_operation_id
           AND (
                (
                    TG_ARGV[0] IN ('attack_candidate_seeds','attack_candidate_work_items')
                    AND NEW.role='observation'
                    AND NEW.evidence_id=ANY(vuln_input.source_evidence_ids)
                )
                OR (
                    TG_ARGV[0] IN ('attack_candidate_seeds','attack_candidate_work_items')
                    AND NEW.role='support'
                    AND NEW.evidence_id=ANY(enumeration_input.source_evidence_ids)
                )
                OR (
                    TG_ARGV[0]='attack_candidate_work_items'
                    AND NEW.role='decision'
                    AND EXISTS (
                        SELECT 1 FROM attack_candidate_work_item_evidence AS grounded
                         WHERE grounded.work_item_id=owner_work_item_id
                           AND grounded.evidence_id=NEW.evidence_id
                           AND grounded.role IN ('observation','support')
                    )
                )
                OR (
                    TG_ARGV[0]='attack_candidates'
                    AND NEW.role IN ('support','rationale')
                    AND EXISTS (
                        SELECT 1 FROM attack_candidate_work_item_evidence AS grounded
                         WHERE grounded.work_item_id=owner_work_item_id
                           AND grounded.evidence_id=NEW.evidence_id
                           AND grounded.role IN ('observation','support')
                    )
                )
           )
    ) INTO fork_allowed;
    IF NOT fork_allowed THEN
        RAISE EXCEPTION 'audit evidence does not match exact attack stage fork lineage';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
