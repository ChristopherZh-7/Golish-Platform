-- Operation-scoped Candidate/Attempt fuel reservation.
--
-- The relational ledger is authoritative:
--   Candidate fuel = every retained V2 attack_candidates row.
--   Attempt fuel   = every candidate_attempts row
--                  + approved Candidates with zero Attempts.
--
-- Repository writers take the same operation -> Wave parent locks before
-- mutating children.  These triggers independently serialize raw SQL writers
-- and re-check both caps at the deferred commit boundary.

CREATE INDEX attack_candidates_operation_fuel_idx
    ON attack_candidates(operation_uuid)
    WHERE operation_uuid IS NOT NULL;

CREATE INDEX candidate_attempts_operation_candidate_fuel_idx
    ON candidate_attempts(operation_id, candidate_id);

-- Existing rows predate the guards below. Refuse the migration instead of
-- blessing a non-canonical Wave policy, a split target tuple, or overbooked
-- Candidate/Attempt fuel as the new authority.
CREATE FUNCTION validate_existing_attack_fuel_state()
RETURNS void AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM attack_wave_runs AS wave
         WHERE NOT EXISTS (
                   SELECT 1
                     FROM attack_wave_runs AS generation_zero
                    WHERE generation_zero.operation_id = wave.operation_id
                      AND generation_zero.generation = 0
               )
            OR (
                wave.generation = 0
                AND (
                    wave.id IS DISTINCT FROM uuid_generate_v5(
                        '6ba7b812-9dad-11d1-80b4-00c04fd430c8'::UUID,
                        wave.operation_id::TEXT || ':candidate-wave:0'
                    )
                    OR wave.policy_snapshot IS DISTINCT FROM
                        '{"max_attempts_total":200,"max_candidates_total":100,"max_chain_depth":3,"max_waves":3}'::JSONB
                    OR wave.policy_hash IS DISTINCT FROM
                        'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326'
                    OR wave.max_waves <> 3
                    OR wave.max_candidates_total <> 100
                    OR wave.max_chain_depth <> 3
                    OR wave.max_attempts_total <> 200
                )
            )
            OR (
                wave.generation > 0
                AND NOT EXISTS (
                    SELECT 1
                      FROM attack_wave_consolidations AS consolidation
                      JOIN attack_wave_runs AS source_wave
                        ON source_wave.id = consolidation.source_wave_run_id
                       AND source_wave.operation_id = consolidation.operation_id
                       AND source_wave.scope_snapshot_id = consolidation.scope_snapshot_id
                     WHERE consolidation.target_wave_run_id = wave.id
                       AND consolidation.operation_id = wave.operation_id
                       AND consolidation.scope_snapshot_id = wave.scope_snapshot_id
                       AND consolidation.decision_kind = 'opened_next_wave'
                       AND consolidation.source_generation + 1 = wave.generation
                       AND consolidation.target_generation = wave.generation
                       AND consolidation.policy_hash = source_wave.policy_hash
                       AND wave.policy_snapshot = source_wave.policy_snapshot
                       AND wave.policy_hash = source_wave.policy_hash
                       AND wave.max_waves = source_wave.max_waves
                       AND wave.max_candidates_total = source_wave.max_candidates_total
                       AND wave.max_chain_depth = source_wave.max_chain_depth
                       AND wave.max_attempts_total = source_wave.max_attempts_total
                )
            )
    ) THEN
        RAISE EXCEPTION 'ATTACK_EXISTING_WAVE_POLICY_INVALID'
            USING ERRCODE = '23514';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_candidates AS candidate
         WHERE candidate.operation_uuid IS NOT NULL
           AND (
               candidate.candidate_plan_hash IS DISTINCT FROM
                   'sha256:' || attack_fact_delta_sha256_jsonb(candidate.execution_plan)
               OR candidate.target_identity_hash IS DISTINCT FROM
                   'sha256:' || encode(
                       digest(
                           convert_to(candidate.target_type_at_time, 'UTF8')
                           || decode('00', 'hex')
                           || convert_to(candidate.target_value_at_time, 'UTF8'),
                           'sha256'
                       ),
                       'hex'
                   )
               OR (
                   candidate.target_live_id IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1
                         FROM operation_org_scope_snapshots AS snapshot
                         JOIN targets AS target
                           ON target.id = candidate.target_live_id
                          AND target.organization_id = candidate.organization_id
                          AND target.scope = 'in'
                          AND target.project_path = snapshot.project_path_at_freeze
                          AND target.target_type::TEXT = candidate.target_type_at_time
                          AND target.value = candidate.target_value_at_time
                        WHERE snapshot.id = candidate.scope_snapshot_id
                          AND snapshot.operation_id = candidate.operation_uuid
                          AND snapshot.sealed_at IS NOT NULL
                   )
               )
           )
    ) OR EXISTS (
        SELECT 1
          FROM attack_candidate_approvals AS approval
         WHERE NOT EXISTS (
             SELECT 1
               FROM attack_candidates AS candidate
              WHERE candidate.candidate_id = approval.candidate_id
                AND candidate.operation_uuid = approval.operation_id
                AND candidate.scope_snapshot_id = approval.scope_snapshot_id
                AND candidate.wave_run_id = approval.wave_run_id
                AND candidate.wave_unit_id = approval.wave_unit_id
                AND candidate.organization_id = approval.organization_id
                AND candidate.source_work_item_id = approval.source_work_item_id
                AND candidate.candidate_plan_hash = approval.candidate_plan_hash
                AND candidate.target_live_id IS NOT DISTINCT FROM approval.target_live_id
                AND candidate.target_type_at_time = approval.target_type_at_time
                AND candidate.target_value_at_time = approval.target_value_at_time
                AND candidate.target_identity_hash = approval.target_identity_hash
                AND candidate.execution_plan = approval.execution_plan
                AND approval.budget = candidate.execution_plan->'budget'
                AND approval.allowed_capability_ids = ARRAY(
                    SELECT DISTINCT action->>'capability_id'
                      FROM jsonb_array_elements(candidate.execution_plan->'actions') AS action
                     ORDER BY action->>'capability_id'
                )
                AND approval.allowed_action_kinds = ARRAY(
                    SELECT DISTINCT action->>'action_kind'
                      FROM jsonb_array_elements(candidate.execution_plan->'actions') AS action
                     ORDER BY action->>'action_kind'
                )
         )
    ) OR EXISTS (
        SELECT 1
          FROM candidate_attempts AS attempt
         WHERE NOT EXISTS (
             SELECT 1
               FROM attack_candidate_approvals AS approval
               JOIN attack_candidates AS candidate
                 ON candidate.candidate_id = approval.candidate_id
                AND candidate.operation_uuid = approval.operation_id
                AND candidate.scope_snapshot_id = approval.scope_snapshot_id
                AND candidate.wave_run_id = approval.wave_run_id
                AND candidate.wave_unit_id = approval.wave_unit_id
                AND candidate.organization_id = approval.organization_id
                AND candidate.source_work_item_id = approval.source_work_item_id
                AND candidate.candidate_plan_hash = approval.candidate_plan_hash
                AND candidate.target_live_id IS NOT DISTINCT FROM approval.target_live_id
                AND candidate.target_type_at_time = approval.target_type_at_time
                AND candidate.target_value_at_time = approval.target_value_at_time
                AND candidate.target_identity_hash = approval.target_identity_hash
              WHERE approval.id = attempt.approval_id
                AND approval.candidate_id = attempt.candidate_id
                AND approval.operation_id = attempt.operation_id
                AND approval.scope_snapshot_id = attempt.scope_snapshot_id
                AND approval.wave_run_id = attempt.wave_run_id
                AND approval.wave_unit_id = attempt.wave_unit_id
                AND approval.organization_id = attempt.organization_id
                AND approval.candidate_plan_hash = attempt.candidate_plan_hash
                AND approval.target_live_id IS NOT DISTINCT FROM attempt.target_live_id
                AND approval.target_type_at_time = attempt.target_type_at_time
                AND approval.target_value_at_time = attempt.target_value_at_time
                AND approval.target_identity_hash = attempt.target_identity_hash
         )
    ) OR EXISTS (
        SELECT 1
          FROM finding_lineage AS lineage
         WHERE NOT EXISTS (
             SELECT 1
               FROM candidate_attempts AS attempt
              WHERE attempt.id = lineage.candidate_attempt_id
                AND attempt.candidate_id = lineage.candidate_id
                AND attempt.operation_id = lineage.operation_id
                AND attempt.scope_snapshot_id = lineage.scope_snapshot_id
                AND attempt.wave_run_id = lineage.wave_run_id
                AND attempt.wave_unit_id = lineage.wave_unit_id
                AND attempt.organization_id = lineage.organization_id
                AND attempt.candidate_plan_hash = lineage.candidate_plan_hash
                AND attempt.target_live_id IS NOT DISTINCT FROM lineage.target_live_id
                AND attempt.target_type_at_time = lineage.target_type_at_time
                AND attempt.target_value_at_time = lineage.target_value_at_time
                AND attempt.target_identity_hash = lineage.target_identity_hash
         )
    ) OR EXISTS (
        SELECT 1
          FROM attack_fact_deltas AS delta
         WHERE NOT EXISTS (
             SELECT 1
               FROM candidate_attempts AS attempt
              WHERE attempt.id = delta.source_attempt_id
                AND attempt.candidate_id = delta.candidate_id
                AND attempt.operation_id = delta.operation_id
                AND attempt.scope_snapshot_id = delta.scope_snapshot_id
                AND attempt.wave_run_id = delta.wave_run_id
                AND attempt.wave_unit_id = delta.wave_unit_id
                AND attempt.organization_id = delta.organization_id
                AND attempt.candidate_plan_hash = delta.candidate_plan_hash
                AND attempt.target_live_id IS NOT DISTINCT FROM delta.target_live_id
                AND attempt.target_type_at_time = delta.target_type_at_time
                AND attempt.target_value_at_time = delta.target_value_at_time
                AND attempt.target_identity_hash = delta.target_identity_hash
         )
    ) THEN
        RAISE EXCEPTION 'ATTACK_EXISTING_TARGET_TUPLE_INVALID'
            USING ERRCODE = '23514';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM attack_wave_runs AS authority
         WHERE authority.generation = 0
           AND (
               (SELECT COUNT(*)
                  FROM attack_candidates AS candidate
                 WHERE candidate.operation_uuid = authority.operation_id)
                   > authority.max_candidates_total
               OR
               (
                   (SELECT COUNT(*)
                      FROM candidate_attempts AS attempt
                     WHERE attempt.operation_id = authority.operation_id)
                   +
                   (SELECT COUNT(*)
                      FROM attack_candidates AS candidate
                     WHERE candidate.operation_uuid = authority.operation_id
                       AND candidate.disposition = 'approved'
                       AND NOT EXISTS (
                           SELECT 1
                             FROM candidate_attempts AS attempt
                            WHERE attempt.candidate_id = candidate.candidate_id
                       ))
                   +
                   (SELECT COUNT(*)
                      FROM attack_candidates AS candidate
                     WHERE candidate.operation_uuid = authority.operation_id
                       AND candidate.disposition = 'approved'
                       AND (
                           SELECT latest.status
                             FROM candidate_attempts AS latest
                            WHERE latest.candidate_id = candidate.candidate_id
                            ORDER BY latest.ordinal DESC
                            LIMIT 1
                       ) = 'retryable_failed')
               ) > authority.max_attempts_total
           )
    ) THEN
        RAISE EXCEPTION 'ATTACK_EXISTING_FUEL_INVALID'
            USING ERRCODE = '23514';
    END IF;
END;
$$ LANGUAGE plpgsql;

-- The manifest's at-time tuple is immutable, but its nullable live pointer is
-- deliberately retained with ON DELETE SET NULL. Keep that declared FK
-- behavior usable after manifest freeze without opening a general UPDATE path.
CREATE OR REPLACE FUNCTION reject_frozen_attack_manifest_row_change()
RETURNS trigger AS $$
DECLARE
    owner_wave_unit_id UUID;
BEGIN
    owner_wave_unit_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.wave_unit_id ELSE NEW.wave_unit_id END;
    IF EXISTS (
        SELECT 1 FROM attack_wave_units
         WHERE id = owner_wave_unit_id AND manifest_frozen_at IS NOT NULL
    ) THEN
        IF TG_OP = 'UPDATE'
            AND OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id)
            AND (to_jsonb(NEW) - 'target_live_id')
                IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_live_id')
        THEN
            RETURN NEW;
        END IF;
        IF TG_OP = 'UPDATE'
            AND to_jsonb(NEW) IS NOT DISTINCT FROM to_jsonb(OLD)
        THEN
            RETURN NEW;
        END IF;
        IF TG_OP = 'INSERT' THEN
            IF TG_TABLE_NAME = 'attack_candidate_seeds' THEN
                IF EXISTS (
                    SELECT 1 FROM attack_candidate_seeds AS existing
                     WHERE existing.wave_unit_id = NEW.wave_unit_id
                       AND existing.target_identity_hash = NEW.target_identity_hash
                       AND existing.technique = NEW.technique
                       AND existing.observation_hash = NEW.observation_hash
                       AND existing.operation_id = NEW.operation_id
                       AND existing.scope_snapshot_id = NEW.scope_snapshot_id
                       AND existing.organization_id = NEW.organization_id
                       AND existing.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
                       AND existing.target_type_at_time = NEW.target_type_at_time
                       AND existing.target_value_at_time = NEW.target_value_at_time
                       AND existing.observation = NEW.observation
                ) THEN
                    RETURN NEW;
                END IF;
            ELSIF TG_TABLE_NAME = 'attack_candidate_work_items' THEN
                IF EXISTS (
                    SELECT 1 FROM attack_candidate_work_items AS existing
                     WHERE existing.wave_unit_id = NEW.wave_unit_id
                       AND existing.work_item_key = NEW.work_item_key
                       AND existing.seed_id = NEW.seed_id
                       AND existing.operation_id = NEW.operation_id
                       AND existing.scope_snapshot_id = NEW.scope_snapshot_id
                       AND existing.organization_id = NEW.organization_id
                       AND existing.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
                       AND existing.target_type_at_time = NEW.target_type_at_time
                       AND existing.target_value_at_time = NEW.target_value_at_time
                       AND existing.target_identity_hash = NEW.target_identity_hash
                ) THEN
                    RETURN NEW;
                END IF;
            END IF;
        END IF;
        RAISE EXCEPTION 'frozen attack manifest rows are immutable';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION lock_attack_fuel_parent()
RETURNS trigger AS $$
DECLARE
    owner_operation_id UUID;
    owner_wave_run_id UUID;
    retention_live_pointer_null BOOLEAN := FALSE;
BEGIN
    IF TG_TABLE_NAME = 'attack_candidates' THEN
        retention_live_pointer_null := TG_OP = 'UPDATE'
            AND OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id)
            AND (to_jsonb(NEW) - 'target_live_id')
                IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_live_id');
        IF TG_OP = 'UPDATE'
            AND OLD.operation_uuid IS NOT NULL
            AND (
                NEW.operation_uuid IS DISTINCT FROM OLD.operation_uuid
                OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
                OR NEW.wave_run_id IS DISTINCT FROM OLD.wave_run_id
                OR NEW.wave_unit_id IS DISTINCT FROM OLD.wave_unit_id
                OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
                OR NEW.source_work_item_id IS DISTINCT FROM OLD.source_work_item_id
                OR NEW.target_type_at_time IS DISTINCT FROM OLD.target_type_at_time
                OR NEW.target_value_at_time IS DISTINCT FROM OLD.target_value_at_time
                OR NEW.target_identity_hash IS DISTINCT FROM OLD.target_identity_hash
                OR NEW.candidate_plan_hash IS DISTINCT FROM OLD.candidate_plan_hash
                OR (
                    NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
                    AND NOT retention_live_pointer_null
                )
            )
        THEN
            RAISE EXCEPTION 'ATTACK_FUEL_LEDGER_IDENTITY_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
        owner_operation_id := NEW.operation_uuid;
        owner_wave_run_id := NEW.wave_run_id;
    ELSIF TG_TABLE_NAME = 'attack_candidate_approvals' THEN
        retention_live_pointer_null := TG_OP = 'UPDATE'
            AND OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id)
            AND (to_jsonb(NEW) - 'target_live_id')
                IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_live_id');
        IF TG_OP = 'UPDATE' AND (
            NEW.candidate_id IS DISTINCT FROM OLD.candidate_id
            OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
            OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
            OR NEW.wave_run_id IS DISTINCT FROM OLD.wave_run_id
            OR NEW.wave_unit_id IS DISTINCT FROM OLD.wave_unit_id
            OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
            OR NEW.target_type_at_time IS DISTINCT FROM OLD.target_type_at_time
            OR NEW.target_value_at_time IS DISTINCT FROM OLD.target_value_at_time
            OR NEW.target_identity_hash IS DISTINCT FROM OLD.target_identity_hash
            OR NEW.candidate_plan_hash IS DISTINCT FROM OLD.candidate_plan_hash
            OR NEW.source_work_item_id IS DISTINCT FROM OLD.source_work_item_id
            OR (
                NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
                AND NOT retention_live_pointer_null
            )
        ) THEN
            RAISE EXCEPTION 'ATTACK_FUEL_LEDGER_IDENTITY_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
        owner_operation_id := NEW.operation_id;
        owner_wave_run_id := NEW.wave_run_id;
    ELSIF TG_TABLE_NAME = 'candidate_attempts' THEN
        retention_live_pointer_null := TG_OP = 'UPDATE'
            AND OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id)
            AND (to_jsonb(NEW) - 'target_live_id')
                IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_live_id');
        IF TG_OP = 'UPDATE' AND (
            NEW.operation_id IS DISTINCT FROM OLD.operation_id
            OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
            OR NEW.wave_run_id IS DISTINCT FROM OLD.wave_run_id
            OR NEW.wave_unit_id IS DISTINCT FROM OLD.wave_unit_id
            OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
            OR NEW.candidate_id IS DISTINCT FROM OLD.candidate_id
            OR NEW.approval_id IS DISTINCT FROM OLD.approval_id
            OR NEW.target_type_at_time IS DISTINCT FROM OLD.target_type_at_time
            OR NEW.target_value_at_time IS DISTINCT FROM OLD.target_value_at_time
            OR NEW.target_identity_hash IS DISTINCT FROM OLD.target_identity_hash
            OR NEW.candidate_plan_hash IS DISTINCT FROM OLD.candidate_plan_hash
            OR NEW.ordinal IS DISTINCT FROM OLD.ordinal
            OR (
                NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
                AND NOT retention_live_pointer_null
            )
        ) THEN
            RAISE EXCEPTION 'ATTACK_FUEL_LEDGER_IDENTITY_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
        owner_operation_id := NEW.operation_id;
        owner_wave_run_id := NEW.wave_run_id;
    ELSIF TG_TABLE_NAME = 'attack_wave_runs' THEN
        owner_operation_id := NEW.operation_id;
        owner_wave_run_id := NEW.id;
    ELSE
        RAISE EXCEPTION 'unsupported attack fuel ledger table %', TG_TABLE_NAME;
    END IF;

    -- ON DELETE SET NULL is retention housekeeping, not a fuel mutation.  It
    -- must remain valid even for preserved legacy/reporting rows whose V2 Wave
    -- parent was never materialized.  Every other trigger still runs; this
    -- function skips only the operation/Wave lock after proving that the live
    -- pointer is the sole changed field and its target has already disappeared.
    IF retention_live_pointer_null THEN
        RETURN NEW;
    END IF;

    IF owner_operation_id IS NULL THEN
        RETURN NEW;
    END IF;

    PERFORM 1
      FROM operation_state
     WHERE operation_id = owner_operation_id
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ATTACK_FUEL_OPERATION_AUTHORITY_MISSING'
            USING ERRCODE = '23503';
    END IF;

    PERFORM 1
      FROM attack_wave_runs
     WHERE id = owner_wave_run_id
       AND operation_id = owner_operation_id
     FOR UPDATE;
    IF NOT FOUND AND TG_TABLE_NAME <> 'attack_wave_runs' THEN
        RAISE EXCEPTION 'ATTACK_FUEL_WAVE_AUTHORITY_MISSING'
            USING ERRCODE = '23503';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidates_fuel_parent_lock
BEFORE INSERT OR UPDATE ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION lock_attack_fuel_parent();

CREATE TRIGGER attack_candidate_approvals_fuel_parent_lock
BEFORE INSERT OR UPDATE ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION lock_attack_fuel_parent();

CREATE TRIGGER candidate_attempts_fuel_parent_lock
BEFORE INSERT OR UPDATE ON candidate_attempts
FOR EACH ROW EXECUTE FUNCTION lock_attack_fuel_parent();

CREATE TRIGGER attack_wave_runs_fuel_parent_lock
BEFORE INSERT OR UPDATE OF max_candidates_total, max_attempts_total
ON attack_wave_runs
FOR EACH ROW EXECUTE FUNCTION lock_attack_fuel_parent();

CREATE FUNCTION reject_frozen_attack_wave_policy_change()
RETURNS trigger AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id
        OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
        OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
        OR NEW.generation IS DISTINCT FROM OLD.generation
        OR NEW.policy_snapshot IS DISTINCT FROM OLD.policy_snapshot
        OR NEW.policy_hash IS DISTINCT FROM OLD.policy_hash
        OR NEW.max_waves IS DISTINCT FROM OLD.max_waves
        OR NEW.max_candidates_total IS DISTINCT FROM OLD.max_candidates_total
        OR NEW.max_chain_depth IS DISTINCT FROM OLD.max_chain_depth
        OR NEW.max_attempts_total IS DISTINCT FROM OLD.max_attempts_total
    THEN
        RAISE EXCEPTION 'ATTACK_WAVE_POLICY_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_policy_immutable
BEFORE UPDATE OF id, operation_id, scope_snapshot_id, generation,
    policy_snapshot, policy_hash, max_waves, max_candidates_total,
    max_chain_depth, max_attempts_total
ON attack_wave_runs
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_wave_policy_change();

CREATE FUNCTION enforce_follow_on_attack_wave_policy_exact()
RETURNS trigger AS $$
BEGIN
    IF NEW.generation = 0 THEN
        IF NEW.id IS DISTINCT FROM uuid_generate_v5(
                '6ba7b812-9dad-11d1-80b4-00c04fd430c8'::UUID,
                NEW.operation_id::TEXT || ':candidate-wave:0'
            )
            OR NEW.policy_snapshot IS DISTINCT FROM
                '{"max_attempts_total":200,"max_candidates_total":100,"max_chain_depth":3,"max_waves":3}'::JSONB
            OR NEW.policy_hash IS DISTINCT FROM
                'sha256:66e50329b4bb217eb060bcccb38f78f4b0eafc163471bebd6554d271d1a6b326'
            OR NEW.max_waves <> 3
            OR NEW.max_candidates_total <> 100
            OR NEW.max_chain_depth <> 3
            OR NEW.max_attempts_total <> 200
        THEN
            RAISE EXCEPTION 'ATTACK_INITIAL_WAVE_POLICY_MISMATCH'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM attack_wave_consolidations AS consolidation
          JOIN attack_wave_runs AS source_wave
            ON source_wave.id = consolidation.source_wave_run_id
           AND source_wave.operation_id = consolidation.operation_id
           AND source_wave.scope_snapshot_id = consolidation.scope_snapshot_id
         WHERE consolidation.target_wave_run_id = NEW.id
           AND consolidation.operation_id = NEW.operation_id
           AND consolidation.scope_snapshot_id = NEW.scope_snapshot_id
           AND consolidation.decision_kind = 'opened_next_wave'
           AND consolidation.source_generation + 1 = NEW.generation
           AND consolidation.target_generation = NEW.generation
           AND consolidation.policy_hash = source_wave.policy_hash
           AND NEW.policy_snapshot = source_wave.policy_snapshot
           AND NEW.policy_hash = source_wave.policy_hash
           AND NEW.max_waves = source_wave.max_waves
           AND NEW.max_candidates_total = source_wave.max_candidates_total
           AND NEW.max_chain_depth = source_wave.max_chain_depth
           AND NEW.max_attempts_total = source_wave.max_attempts_total
    ) THEN
        RAISE EXCEPTION 'ATTACK_FOLLOW_ON_WAVE_POLICY_MISMATCH'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_wave_follow_on_policy_exact
AFTER INSERT OR UPDATE ON attack_wave_runs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_follow_on_attack_wave_policy_exact();

CREATE FUNCTION enforce_attack_target_tuple_exact()
RETURNS trigger AS $$
BEGIN
    -- Revalidating the complete V2 parent graph is unnecessary for the sole
    -- FK-retention transition.  More importantly, historical/reporting rows
    -- can legitimately outlive an unmaterialized V2 graph.  Clearing a dead
    -- live pointer does not alter their frozen at-time tuple.
    IF TG_TABLE_NAME IN (
            'attack_candidates',
            'attack_candidate_approvals',
            'candidate_attempts'
        ) AND TG_OP = 'UPDATE'
    THEN
        IF OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id)
            AND (to_jsonb(NEW) - 'target_live_id')
                IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_live_id')
        THEN
            RETURN NEW;
        END IF;
    END IF;

    IF TG_TABLE_NAME = 'attack_candidates' THEN
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
                   AND target.target_type::TEXT = NEW.target_type_at_time
                   AND target.value = NEW.target_value_at_time
                 WHERE snapshot.id = NEW.scope_snapshot_id
                   AND snapshot.operation_id = NEW.operation_uuid
                   AND snapshot.sealed_at IS NOT NULL
            )
        THEN
            RAISE EXCEPTION 'ATTACK_CANDIDATE_TARGET_SCOPE_MISMATCH'
                USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'attack_candidate_approvals' THEN
        IF NOT EXISTS (
            SELECT 1
              FROM attack_candidates AS candidate
             WHERE candidate.candidate_id = NEW.candidate_id
               AND candidate.operation_uuid = NEW.operation_id
               AND candidate.scope_snapshot_id = NEW.scope_snapshot_id
               AND candidate.wave_run_id = NEW.wave_run_id
               AND candidate.wave_unit_id = NEW.wave_unit_id
               AND candidate.organization_id = NEW.organization_id
               AND candidate.source_work_item_id = NEW.source_work_item_id
               AND candidate.candidate_plan_hash = NEW.candidate_plan_hash
               AND candidate.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
               AND candidate.target_type_at_time = NEW.target_type_at_time
               AND candidate.target_value_at_time = NEW.target_value_at_time
               AND candidate.target_identity_hash = NEW.target_identity_hash
               AND candidate.execution_plan = NEW.execution_plan
               AND NEW.budget = candidate.execution_plan->'budget'
               AND NEW.allowed_capability_ids = ARRAY(
                   SELECT DISTINCT action->>'capability_id'
                     FROM jsonb_array_elements(candidate.execution_plan->'actions') AS action
                    ORDER BY action->>'capability_id'
               )
               AND NEW.allowed_action_kinds = ARRAY(
                   SELECT DISTINCT action->>'action_kind'
                     FROM jsonb_array_elements(candidate.execution_plan->'actions') AS action
                    ORDER BY action->>'action_kind'
               )
        ) THEN
            RAISE EXCEPTION 'ATTACK_APPROVAL_TARGET_TUPLE_MISMATCH'
                USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'candidate_attempts' THEN
        IF NOT EXISTS (
            SELECT 1
              FROM attack_candidate_approvals AS approval
              JOIN attack_candidates AS candidate
                ON candidate.candidate_id = approval.candidate_id
               AND candidate.operation_uuid = approval.operation_id
               AND candidate.scope_snapshot_id = approval.scope_snapshot_id
               AND candidate.wave_run_id = approval.wave_run_id
               AND candidate.wave_unit_id = approval.wave_unit_id
               AND candidate.organization_id = approval.organization_id
               AND candidate.source_work_item_id = approval.source_work_item_id
               AND candidate.candidate_plan_hash = approval.candidate_plan_hash
               AND candidate.target_live_id IS NOT DISTINCT FROM approval.target_live_id
               AND candidate.target_type_at_time = approval.target_type_at_time
               AND candidate.target_value_at_time = approval.target_value_at_time
               AND candidate.target_identity_hash = approval.target_identity_hash
             WHERE approval.id = NEW.approval_id
               AND approval.candidate_id = NEW.candidate_id
               AND approval.operation_id = NEW.operation_id
               AND approval.scope_snapshot_id = NEW.scope_snapshot_id
               AND approval.wave_run_id = NEW.wave_run_id
               AND approval.wave_unit_id = NEW.wave_unit_id
               AND approval.organization_id = NEW.organization_id
               AND approval.candidate_plan_hash = NEW.candidate_plan_hash
               AND approval.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
               AND approval.target_type_at_time = NEW.target_type_at_time
               AND approval.target_value_at_time = NEW.target_value_at_time
               AND approval.target_identity_hash = NEW.target_identity_hash
        ) THEN
            RAISE EXCEPTION 'CANDIDATE_ATTEMPT_TARGET_TUPLE_MISMATCH'
                USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'finding_lineage' THEN
        IF NOT EXISTS (
            SELECT 1
              FROM candidate_attempts AS attempt
             WHERE attempt.id = NEW.candidate_attempt_id
               AND attempt.candidate_id = NEW.candidate_id
               AND attempt.operation_id = NEW.operation_id
               AND attempt.scope_snapshot_id = NEW.scope_snapshot_id
               AND attempt.wave_run_id = NEW.wave_run_id
               AND attempt.wave_unit_id = NEW.wave_unit_id
               AND attempt.organization_id = NEW.organization_id
               AND attempt.candidate_plan_hash = NEW.candidate_plan_hash
               AND attempt.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
               AND attempt.target_type_at_time = NEW.target_type_at_time
               AND attempt.target_value_at_time = NEW.target_value_at_time
               AND attempt.target_identity_hash = NEW.target_identity_hash
        ) THEN
            RAISE EXCEPTION 'FINDING_LINEAGE_TARGET_TUPLE_MISMATCH'
                USING ERRCODE = '23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'attack_fact_deltas' THEN
        IF NOT EXISTS (
            SELECT 1
              FROM candidate_attempts AS attempt
             WHERE attempt.id = NEW.source_attempt_id
               AND attempt.candidate_id = NEW.candidate_id
               AND attempt.operation_id = NEW.operation_id
               AND attempt.scope_snapshot_id = NEW.scope_snapshot_id
               AND attempt.wave_run_id = NEW.wave_run_id
               AND attempt.wave_unit_id = NEW.wave_unit_id
               AND attempt.organization_id = NEW.organization_id
               AND attempt.candidate_plan_hash = NEW.candidate_plan_hash
               AND attempt.target_live_id IS NOT DISTINCT FROM NEW.target_live_id
               AND attempt.target_type_at_time = NEW.target_type_at_time
               AND attempt.target_value_at_time = NEW.target_value_at_time
               AND attempt.target_identity_hash = NEW.target_identity_hash
        ) THEN
            RAISE EXCEPTION 'ATTACK_FACT_DELTA_TARGET_TUPLE_MISMATCH'
                USING ERRCODE = '23514';
        END IF;
    ELSE
        RAISE EXCEPTION 'unsupported attack target tuple table %', TG_TABLE_NAME;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_candidates_target_scope_exact
AFTER INSERT OR UPDATE ON attack_candidates
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_target_tuple_exact();

CREATE CONSTRAINT TRIGGER attack_candidate_approvals_target_tuple_exact
AFTER INSERT OR UPDATE ON attack_candidate_approvals
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_target_tuple_exact();

CREATE CONSTRAINT TRIGGER candidate_attempts_target_tuple_exact
AFTER INSERT OR UPDATE ON candidate_attempts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_target_tuple_exact();

CREATE CONSTRAINT TRIGGER finding_lineage_target_tuple_exact
AFTER INSERT OR UPDATE ON finding_lineage
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_target_tuple_exact();

CREATE CONSTRAINT TRIGGER attack_fact_deltas_target_tuple_exact
AFTER INSERT OR UPDATE ON attack_fact_deltas
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_target_tuple_exact();

CREATE FUNCTION reject_attack_fuel_ledger_delete()
RETURNS trigger AS $$
BEGIN
    IF TG_TABLE_NAME = 'candidate_attempts'
        OR (TG_TABLE_NAME = 'attack_candidates' AND OLD.operation_uuid IS NOT NULL)
    THEN
        RAISE EXCEPTION 'ATTACK_FUEL_LEDGER_DELETE_REJECTED'
            USING ERRCODE = '23514';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidates_fuel_delete_guard
BEFORE DELETE ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION reject_attack_fuel_ledger_delete();

CREATE TRIGGER candidate_attempts_fuel_delete_guard
BEFORE DELETE ON candidate_attempts
FOR EACH ROW EXECUTE FUNCTION reject_attack_fuel_ledger_delete();

CREATE FUNCTION enforce_attack_operation_fuel_hard_cap()
RETURNS trigger AS $$
DECLARE
    owner_operation_id UUID;
    candidate_cap INTEGER;
    attempt_cap INTEGER;
    policy_variant_count BIGINT;
    candidate_fuel BIGINT;
    attempt_fuel BIGINT;
    retryable_backlog BIGINT;
BEGIN
    -- A pure ON DELETE SET NULL transition consumes no Candidate or Attempt
    -- fuel.  Do not require a V2 Wave parent for retained legacy/reporting
    -- rows when the only changed field is their non-canonical live pointer.
    IF TG_TABLE_NAME IN (
            'attack_candidates',
            'attack_candidate_approvals',
            'candidate_attempts'
        ) AND TG_OP = 'UPDATE'
    THEN
        IF OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id)
            AND (to_jsonb(NEW) - 'target_live_id')
                IS NOT DISTINCT FROM (to_jsonb(OLD) - 'target_live_id')
        THEN
            RETURN NEW;
        END IF;
    END IF;

    IF TG_TABLE_NAME = 'attack_candidates' THEN
        owner_operation_id := COALESCE(NEW.operation_uuid, OLD.operation_uuid);
    ELSIF TG_TABLE_NAME = 'attack_candidate_approvals' THEN
        owner_operation_id := COALESCE(NEW.operation_id, OLD.operation_id);
    ELSIF TG_TABLE_NAME = 'candidate_attempts' THEN
        owner_operation_id := COALESCE(NEW.operation_id, OLD.operation_id);
    ELSIF TG_TABLE_NAME = 'attack_wave_runs' THEN
        owner_operation_id := COALESCE(NEW.operation_id, OLD.operation_id);
    ELSE
        RAISE EXCEPTION 'unsupported attack fuel ledger table %', TG_TABLE_NAME;
    END IF;

    IF owner_operation_id IS NULL THEN
        RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    END IF;

    PERFORM 1
      FROM operation_state
     WHERE operation_id = owner_operation_id
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ATTACK_FUEL_OPERATION_AUTHORITY_MISSING'
            USING ERRCODE = '23503';
    END IF;

    PERFORM 1
      FROM attack_wave_runs
     WHERE operation_id = owner_operation_id
     ORDER BY generation, id
     FOR UPDATE;

    SELECT MIN(max_candidates_total), MIN(max_attempts_total),
           COUNT(DISTINCT (
               max_waves,
               max_candidates_total,
               max_chain_depth,
               max_attempts_total
           ))
      INTO candidate_cap, attempt_cap, policy_variant_count
      FROM attack_wave_runs
     WHERE operation_id = owner_operation_id;
    IF candidate_cap IS NULL OR attempt_cap IS NULL THEN
        RAISE EXCEPTION 'ATTACK_FUEL_WAVE_AUTHORITY_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF policy_variant_count <> 1 THEN
        RAISE EXCEPTION 'ATTACK_FUEL_POLICY_DRIFT'
            USING ERRCODE = '23514';
    END IF;

    SELECT COUNT(*)
      INTO candidate_fuel
      FROM attack_candidates
     WHERE operation_uuid = owner_operation_id;
    IF candidate_fuel > candidate_cap THEN
        RAISE EXCEPTION
            'ATTACK_CANDIDATE_FUEL_EXHAUSTED: % Candidate slots exceed frozen cap %',
            candidate_fuel, candidate_cap
            USING ERRCODE = '23514';
    END IF;

    SELECT
        (SELECT COUNT(*)
           FROM candidate_attempts
          WHERE operation_id = owner_operation_id)
        +
        (SELECT COUNT(*)
           FROM attack_candidates AS candidate
          WHERE candidate.operation_uuid = owner_operation_id
            AND candidate.disposition = 'approved'
            AND NOT EXISTS (
                SELECT 1
                  FROM candidate_attempts AS attempt
                 WHERE attempt.candidate_id = candidate.candidate_id
            ))
      INTO attempt_fuel;
    SELECT COUNT(*)
      INTO retryable_backlog
      FROM attack_candidates AS candidate
     WHERE candidate.operation_uuid = owner_operation_id
       AND candidate.disposition = 'approved'
       AND (
           SELECT latest.status
             FROM candidate_attempts AS latest
            WHERE latest.candidate_id = candidate.candidate_id
            ORDER BY latest.ordinal DESC
            LIMIT 1
       ) = 'retryable_failed';
    IF attempt_fuel + retryable_backlog > attempt_cap THEN
        RAISE EXCEPTION
            'ATTACK_ATTEMPT_FUEL_EXHAUSTED: effective fuel % plus retry backlog % exceeds frozen cap %',
            attempt_fuel, retryable_backlog, attempt_cap
            USING ERRCODE = '23514';
    END IF;

    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_candidates_fuel_hard_cap
AFTER INSERT OR UPDATE ON attack_candidates
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_operation_fuel_hard_cap();

CREATE CONSTRAINT TRIGGER attack_candidate_approvals_fuel_hard_cap
AFTER INSERT OR UPDATE OR DELETE ON attack_candidate_approvals
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_operation_fuel_hard_cap();

CREATE CONSTRAINT TRIGGER candidate_attempts_fuel_hard_cap
AFTER INSERT OR UPDATE ON candidate_attempts
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_operation_fuel_hard_cap();

CREATE CONSTRAINT TRIGGER attack_wave_runs_fuel_hard_cap
AFTER INSERT OR UPDATE ON attack_wave_runs
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_operation_fuel_hard_cap();

CREATE FUNCTION reject_attack_candidate_canonical_change()
RETURNS trigger AS $$
DECLARE
    target_pointer_change_allowed BOOLEAN;
    disposition_change_allowed BOOLEAN;
BEGIN
    IF OLD.operation_uuid IS NULL THEN
        RETURN NEW;
    END IF;
    target_pointer_change_allowed :=
        NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
        OR NEW.live_target_id IS DISTINCT FROM OLD.live_target_id;
    IF target_pointer_change_allowed THEN
        target_pointer_change_allowed :=
            COALESCE(OLD.target_live_id, OLD.live_target_id) IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM targets
                 WHERE id = COALESCE(OLD.target_live_id, OLD.live_target_id)
            )
            AND (NEW.target_live_id IS NULL OR NEW.target_live_id = OLD.target_live_id)
            AND (NEW.live_target_id IS NULL OR NEW.live_target_id = OLD.live_target_id)
            AND (NEW.target_live_id IS NULL OR NEW.live_target_id IS NULL);
    ELSE
        target_pointer_change_allowed := TRUE;
    END IF;
    IF NOT target_pointer_change_allowed
        OR (to_jsonb(NEW) - ARRAY[
                'target_live_id','live_target_id','disposition','terminal_attempt_id',
                'terminal_finding_id','row_version','updated_at'
            ]::TEXT[])
           IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'target_live_id','live_target_id','disposition','terminal_attempt_id',
                'terminal_finding_id','row_version','updated_at'
            ]::TEXT[])
    THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_CANONICAL_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;

    disposition_change_allowed :=
        (
            NEW.disposition = OLD.disposition
            AND NEW.terminal_attempt_id IS NOT DISTINCT FROM OLD.terminal_attempt_id
            AND NEW.terminal_finding_id IS NOT DISTINCT FROM OLD.terminal_finding_id
        )
        OR (
            OLD.disposition = 'proposed'
            AND NEW.disposition IN ('approved','rejected')
            AND OLD.terminal_attempt_id IS NULL
            AND OLD.terminal_finding_id IS NULL
            AND NEW.terminal_attempt_id IS NULL
            AND NEW.terminal_finding_id IS NULL
        )
        OR (
            OLD.disposition = 'approved'
            AND NEW.disposition = 'proposed'
            AND OLD.terminal_attempt_id IS NULL
            AND OLD.terminal_finding_id IS NULL
            AND NEW.terminal_attempt_id IS NULL
            AND NEW.terminal_finding_id IS NULL
        )
        OR (
            OLD.disposition = 'approved'
            AND NEW.disposition IN ('verified','refuted','blocked')
            AND OLD.terminal_attempt_id IS NULL
            AND OLD.terminal_finding_id IS NULL
            AND NEW.terminal_attempt_id IS NOT NULL
            AND (NEW.terminal_finding_id IS NULL OR NEW.disposition = 'verified')
        );
    IF NOT disposition_change_allowed THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_DISPOSITION_TRANSITION_INVALID'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidates_canonical_immutable
BEFORE UPDATE ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION reject_attack_candidate_canonical_change();

CREATE FUNCTION reject_attack_approval_decision_change()
RETURNS trigger AS $$
DECLARE
    target_pointer_change_allowed BOOLEAN;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'ATTACK_APPROVAL_AUDIT_DELETE_REJECTED'
            USING ERRCODE = '23514';
    END IF;
    target_pointer_change_allowed :=
        NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
        OR NEW.live_target_id IS DISTINCT FROM OLD.live_target_id;
    IF target_pointer_change_allowed THEN
        target_pointer_change_allowed :=
            COALESCE(OLD.target_live_id, OLD.live_target_id) IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM targets
                 WHERE id = COALESCE(OLD.target_live_id, OLD.live_target_id)
            )
            AND (NEW.target_live_id IS NULL OR NEW.target_live_id = OLD.target_live_id)
            AND (NEW.live_target_id IS NULL OR NEW.live_target_id = OLD.live_target_id)
            AND (NEW.target_live_id IS NULL OR NEW.live_target_id IS NULL);
    ELSE
        target_pointer_change_allowed := TRUE;
    END IF;
    IF NOT target_pointer_change_allowed
        OR (to_jsonb(NEW) - ARRAY[
                'target_live_id','live_target_id','status','row_version'
            ]::TEXT[])
           IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'target_live_id','live_target_id','status','row_version'
            ]::TEXT[])
        OR (
            NEW.status IS DISTINCT FROM OLD.status
            AND NOT (
                OLD.status = 'approved'
                AND NEW.status IN ('revoked','expired')
                AND NEW.row_version = OLD.row_version + 1
            )
        )
        OR (
            NEW.status IS NOT DISTINCT FROM OLD.status
            AND NEW.row_version IS DISTINCT FROM OLD.row_version
        )
    THEN
        RAISE EXCEPTION 'ATTACK_APPROVAL_DECISION_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidate_approvals_decision_immutable
BEFORE UPDATE OR DELETE ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION reject_attack_approval_decision_change();

CREATE FUNCTION reject_finding_lineage_audit_change()
RETURNS trigger AS $$
DECLARE
    target_pointer_change_allowed BOOLEAN;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'FINDING_LINEAGE_AUDIT_DELETE_REJECTED'
            USING ERRCODE = '23514';
    END IF;
    target_pointer_change_allowed :=
        NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
        OR NEW.live_target_id IS DISTINCT FROM OLD.live_target_id;
    IF target_pointer_change_allowed THEN
        target_pointer_change_allowed :=
            COALESCE(OLD.target_live_id, OLD.live_target_id) IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM targets
                 WHERE id = COALESCE(OLD.target_live_id, OLD.live_target_id)
            )
            AND (NEW.target_live_id IS NULL OR NEW.target_live_id = OLD.target_live_id)
            AND (NEW.live_target_id IS NULL OR NEW.live_target_id = OLD.live_target_id)
            AND (NEW.target_live_id IS NULL OR NEW.live_target_id IS NULL);
    ELSE
        target_pointer_change_allowed := TRUE;
    END IF;
    IF (
            NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
            OR NEW.live_target_id IS DISTINCT FROM OLD.live_target_id
        )
        AND target_pointer_change_allowed
        AND (to_jsonb(NEW) - ARRAY[
                'target_live_id','live_target_id','row_version'
            ]::TEXT[])
           IS NOT DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'target_live_id','live_target_id','row_version'
            ]::TEXT[])
    THEN
        -- The reporting migration's generic row-version trigger runs first.
        -- A true FK-driven live-pointer removal is not a new lineage version.
        NEW.row_version := OLD.row_version;
        RETURN NEW;
    END IF;
    IF NOT target_pointer_change_allowed
        OR (to_jsonb(NEW) - ARRAY[
                'target_live_id','live_target_id'
            ]::TEXT[])
           IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'target_live_id','live_target_id'
            ]::TEXT[])
    THEN
        RAISE EXCEPTION 'FINDING_LINEAGE_AUDIT_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER zz_finding_lineage_audit_immutable
BEFORE UPDATE OR DELETE ON finding_lineage
FOR EACH ROW EXECUTE FUNCTION reject_finding_lineage_audit_change();

CREATE FUNCTION enforce_candidate_attempt_audit_transition()
RETURNS trigger AS $$
DECLARE
    old_is_terminal BOOLEAN;
    new_is_terminal BOOLEAN;
    target_pointer_change_allowed BOOLEAN;
BEGIN
    new_is_terminal := NEW.status IN (
        'verified','refuted','blocked','retryable_failed','abandoned'
    );
    IF TG_OP = 'INSERT' THEN
        IF new_is_terminal THEN
            NEW.terminal_at := NOW();
        END IF;
        RETURN NEW;
    END IF;
    old_is_terminal := OLD.status IN (
        'verified','refuted','blocked','retryable_failed','abandoned'
    );
    target_pointer_change_allowed :=
        NEW.target_live_id IS DISTINCT FROM OLD.target_live_id;
    IF target_pointer_change_allowed THEN
        target_pointer_change_allowed :=
            OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id);
    ELSE
        target_pointer_change_allowed := TRUE;
    END IF;
    IF NOT target_pointer_change_allowed
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR (OLD.result_json IS NOT NULL AND (
            NEW.result_json IS DISTINCT FROM OLD.result_json
            OR NEW.result_hash IS DISTINCT FROM OLD.result_hash
        ))
    THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_RESULT_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF old_is_terminal THEN
        IF (to_jsonb(NEW) - ARRAY[
                'target_live_id'
            ]::TEXT[])
           IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'target_live_id'
            ]::TEXT[])
        THEN
            RAISE EXCEPTION 'CANDIDATE_ATTEMPT_TERMINAL_AUDIT_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.status IS DISTINCT FROM OLD.status AND NOT (
        (OLD.status = 'queued' AND NEW.status = 'running')
        OR (OLD.status = 'running' AND NEW.status IN (
            'running','submitted','blocked','retryable_failed','abandoned'
        ))
        OR (OLD.status = 'submitted' AND NEW.status IN ('verified','refuted','blocked'))
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_STATUS_TRANSITION_INVALID'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.result_json IS NOT NULL AND OLD.result_json IS NULL
        AND NOT (
            OLD.status = 'running'
            AND NEW.status IN ('submitted','blocked','retryable_failed')
        )
    THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_RESULT_TRANSITION_INVALID'
            USING ERRCODE = '23514';
    END IF;
    IF new_is_terminal AND NOT old_is_terminal THEN
        NEW.terminal_at := NOW();
    ELSIF NOT new_is_terminal AND NEW.terminal_at IS NOT NULL THEN
        RAISE EXCEPTION 'CANDIDATE_ATTEMPT_TERMINAL_TIME_INVALID'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER zz_candidate_attempts_audit_transition
BEFORE INSERT OR UPDATE ON candidate_attempts
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_attempt_audit_transition();

CREATE FUNCTION reject_decided_attack_work_item_change()
RETURNS trigger AS $$
DECLARE
    target_pointer_change_allowed BOOLEAN;
BEGIN
    IF TG_OP = 'DELETE' AND OLD.decision_kind IS NOT NULL THEN
        RAISE EXCEPTION 'ATTACK_WORK_ITEM_DECISION_DELETE_REJECTED'
            USING ERRCODE = '23514';
    ELSIF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    IF OLD.decision_kind IS NULL THEN
        IF NEW.decision_kind IS NOT NULL THEN
            NEW.decided_at := NOW();
        END IF;
        RETURN NEW;
    END IF;
    target_pointer_change_allowed :=
        NEW.target_live_id IS DISTINCT FROM OLD.target_live_id;
    IF target_pointer_change_allowed THEN
        target_pointer_change_allowed :=
            OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM targets WHERE id = OLD.target_live_id);
    ELSE
        target_pointer_change_allowed := TRUE;
    END IF;
    IF NOT target_pointer_change_allowed
        OR (to_jsonb(NEW) - ARRAY[
                'target_live_id','row_version','updated_at'
            ]::TEXT[])
           IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY[
                'target_live_id','row_version','updated_at'
            ]::TEXT[])
    THEN
        RAISE EXCEPTION 'ATTACK_WORK_ITEM_DECISION_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidate_work_items_decision_immutable
BEFORE UPDATE OR DELETE ON attack_candidate_work_items
FOR EACH ROW EXECUTE FUNCTION reject_decided_attack_work_item_change();

CREATE FUNCTION reject_frozen_attack_candidate_evidence_change()
RETURNS trigger AS $$
DECLARE
    owner_candidate_id UUID;
    prior_candidate_id UUID;
BEGIN
    owner_candidate_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.candidate_id ELSE NEW.candidate_id END;
    prior_candidate_id := CASE WHEN TG_OP = 'UPDATE' THEN OLD.candidate_id ELSE owner_candidate_id END;
    IF EXISTS (
        SELECT 1
          FROM attack_candidates AS candidate
          JOIN attack_candidate_work_items AS work_item
            ON work_item.id = candidate.source_work_item_id
         WHERE candidate.candidate_id IN (owner_candidate_id, prior_candidate_id)
           AND work_item.decision_kind IS NOT NULL
         FOR UPDATE OF work_item
    ) THEN
        RAISE EXCEPTION 'ATTACK_CANDIDATE_EVIDENCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidate_evidence_immutable
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_evidence
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_candidate_evidence_change();

CREATE FUNCTION enforce_candidate_action_journal_audit()
RETURNS trigger AS $$
DECLARE
    owner_attempt_id UUID;
    owner_attempt_status TEXT;
BEGIN
    owner_attempt_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.attempt_id ELSE NEW.attempt_id END;
    SELECT status
      INTO owner_attempt_status
      FROM candidate_attempts
     WHERE id = owner_attempt_id
     FOR UPDATE;
    IF owner_attempt_status IS NULL THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_ATTEMPT_MISSING'
            USING ERRCODE = '23503';
    END IF;
    IF owner_attempt_status <> 'running' THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_TERMINAL_AUDIT_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' AND (
        NEW.attempt_id IS DISTINCT FROM OLD.attempt_id
        OR NEW.action_ordinal IS DISTINCT FROM OLD.action_ordinal
        OR NEW.capability_id IS DISTINCT FROM OLD.capability_id
        OR NEW.action_kind IS DISTINCT FROM OLD.action_kind
        OR NEW.canonical_args IS DISTINCT FROM OLD.canonical_args
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
    ) THEN
        RAISE EXCEPTION 'CANDIDATE_ACTION_IDENTITY_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_attempt_actions_audit_immutable
BEFORE INSERT OR UPDATE OR DELETE ON candidate_attempt_actions
FOR EACH ROW EXECUTE FUNCTION enforce_candidate_action_journal_audit();

CREATE FUNCTION reject_attack_fuel_residual_canonical_change()
RETURNS trigger AS $$
BEGIN
    IF OLD.reason_code NOT IN (
        'max_waves','max_candidates_total','max_chain_depth','max_attempts_total'
    ) THEN
        RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'ATTACK_FUEL_RESIDUAL_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
        OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
        OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
        OR NEW.wave_run_id IS DISTINCT FROM OLD.wave_run_id
        OR NEW.wave_unit_id IS DISTINCT FROM OLD.wave_unit_id
        OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
        OR NEW.target_type_at_time IS DISTINCT FROM OLD.target_type_at_time
        OR NEW.target_value_at_time IS DISTINCT FROM OLD.target_value_at_time
        OR NEW.target_identity_hash IS DISTINCT FROM OLD.target_identity_hash
        OR NEW.reason_code IS DISTINCT FROM OLD.reason_code
        OR NEW.reason_detail IS DISTINCT FROM OLD.reason_detail
        OR NEW.policy_hash IS DISTINCT FROM OLD.policy_hash
        OR NEW.wave_count IS DISTINCT FROM OLD.wave_count
        OR NEW.candidate_count IS DISTINCT FROM OLD.candidate_count
        OR NEW.chain_depth IS DISTINCT FROM OLD.chain_depth
        OR NEW.attempt_count IS DISTINCT FROM OLD.attempt_count
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR (
            NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
            AND NOT (
                OLD.target_live_id IS NOT NULL
                AND NEW.target_live_id IS NULL
                AND NOT EXISTS (
                    SELECT 1 FROM targets WHERE id = OLD.target_live_id
                )
            )
        )
    THEN
        RAISE EXCEPTION 'ATTACK_FUEL_RESIDUAL_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fuel_residual_canonical_immutable
BEFORE UPDATE OR DELETE ON attack_residual_risks
FOR EACH ROW EXECUTE FUNCTION reject_attack_fuel_residual_canonical_change();

SELECT validate_existing_attack_fuel_state();
