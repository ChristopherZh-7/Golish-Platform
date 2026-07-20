-- Candidate observations may describe a contextual URL while retaining the
-- live pointer of the in-scope domain/IP that produced that observation.  The
-- old shared tuple trigger incorrectly required those two identities to have
-- the same type/value.  Keep the live-pointer scope check, but bind the
-- contextual tuple to the immutable source work item instead.

CREATE FUNCTION enforce_attack_candidate_contextual_target_exact()
RETURNS trigger AS $$
BEGIN
    -- Preserve the retained-identity transition when an FK clears a deleted
    -- live target pointer.  The frozen tuple remains immutable and reportable.
    IF TG_OP = 'UPDATE'
        AND OLD.target_live_id IS NOT NULL
        AND NEW.target_live_id IS NULL
        AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id)
        AND (to_jsonb(NEW) - 'target_live_id')
            IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_live_id')
    THEN
        RETURN NEW;
    END IF;

    IF NEW.operation_uuid IS NOT NULL
        AND NEW.candidate_plan_hash IS DISTINCT FROM
            'sha256:' || attack_fact_delta_sha256_jsonb(NEW.execution_plan)
    THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_PLAN_HASH_MISMATCH'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.operation_uuid IS NOT NULL
        AND NEW.target_identity_hash IS DISTINCT FROM
            'sha256:' || encode(
                digest(
                    convert_to(NEW.target_type_at_time, 'UTF8')
                    || decode('00', 'hex')
                    || convert_to(NEW.target_value_at_time, 'UTF8'),
                    'sha256'
                ),
                'hex'
            )
    THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_TARGET_HASH_MISMATCH'
            USING ERRCODE = '23514';
    END IF;

    -- The Candidate tuple is the contextual observation identity.  It must be
    -- copied exactly from the immutable work item accepted by this WaveUnit;
    -- callers cannot invent or retarget it.
    IF NEW.operation_uuid IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
              FROM attack_candidate_work_items AS item
             WHERE item.id = NEW.source_work_item_id
               AND item.operation_id = NEW.operation_uuid
               AND item.scope_snapshot_id = NEW.scope_snapshot_id
               AND item.wave_unit_id = NEW.wave_unit_id
               AND item.organization_id = NEW.organization_id
               AND item.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
               AND item.target_type_at_time = NEW.target_type_at_time
               AND item.target_value_at_time = NEW.target_value_at_time
               AND item.target_identity_hash = NEW.target_identity_hash
        )
    THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_TARGET_SCOPE_MISMATCH'
            USING ERRCODE = '23514';
    END IF;

    -- A non-null live pointer remains independently constrained to an active,
    -- in-scope target in the exact frozen project and organization.
    IF NEW.operation_uuid IS NOT NULL
        AND NEW.target_live_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
              FROM operation_org_scope_snapshots AS snapshot
              JOIN targets AS target
                ON target.id = NEW.target_live_id
               AND target.organization_id = NEW.organization_id
               AND target.scope = 'in'
               AND target.project_path = snapshot.project_path_at_freeze
             WHERE snapshot.id = NEW.scope_snapshot_id
               AND snapshot.operation_id = NEW.operation_uuid
               AND snapshot.sealed_at IS NOT NULL
        )
    THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_TARGET_SCOPE_MISMATCH'
            USING ERRCODE = '23514';
    END IF;

    -- A stage-fork operation is narrower than the project snapshot: its live
    -- pointer must also occur in the exact immutable fork-target manifest.
    IF NEW.operation_uuid IS NOT NULL
        AND NEW.target_live_id IS NOT NULL
        AND EXISTS (
            SELECT 1
              FROM operation_stage_forks AS fork
             WHERE fork.operation_id = NEW.operation_uuid
        )
        AND NOT EXISTS (
            SELECT 1
              FROM operation_stage_fork_targets AS fork_target
              JOIN operation_org_scope_snapshots AS snapshot
                ON snapshot.id = fork_target.scope_snapshot_id
               AND snapshot.operation_id = fork_target.operation_id
               AND snapshot.sealed_at IS NOT NULL
             WHERE fork_target.operation_id = NEW.operation_uuid
               AND fork_target.scope_snapshot_id = NEW.scope_snapshot_id
               AND fork_target.organization_id = NEW.organization_id
               AND fork_target.live_target_id = NEW.target_live_id
               AND fork_target.target_scope_at_fork = 'in'
               AND fork_target.project_path_at_fork = snapshot.project_path_at_freeze
        )
    THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_TARGET_SCOPE_MISMATCH'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER attack_candidates_target_scope_exact ON attack_candidates;

CREATE CONSTRAINT TRIGGER attack_candidates_target_scope_exact
AFTER INSERT OR UPDATE ON attack_candidates
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_candidate_contextual_target_exact();
