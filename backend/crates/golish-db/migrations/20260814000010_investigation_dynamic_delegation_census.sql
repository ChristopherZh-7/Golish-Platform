-- Dynamic Refiner plans retain every proposed subtask for audit, including
-- members later dropped from the ordered active denominator.  Delegation
-- closure therefore counts only the exact worker dispatches which have an
-- exact result-barrier event; fixed-denominator v1 plans retain their original
-- all-runnable census.

DO $$
DECLARE
    legacy_constraint_name TEXT;
BEGIN
    SELECT constraint_row.conname
      INTO STRICT legacy_constraint_name
      FROM pg_catalog.pg_constraint constraint_row
     WHERE constraint_row.conrelid='investigation_pentagi_task_plans'::regclass
       AND constraint_row.contype='c'
       AND pg_get_constraintdef(constraint_row.oid) LIKE '%status%sealed%'
       AND pg_get_constraintdef(constraint_row.oid) LIKE '%subtask_count > 0%';
    EXECUTE format(
        'ALTER TABLE investigation_pentagi_task_plans DROP CONSTRAINT %I',
        legacy_constraint_name
    );
END;
$$;

ALTER TABLE investigation_pentagi_task_plans
    ADD CONSTRAINT pentagi_plan_open_or_nonnegative_sealed
    CHECK(
        (status='open' AND subtask_count IS NULL
         AND subtask_set_sha256 IS NULL AND sealed_at IS NULL)
        OR
        (status='sealed' AND subtask_count IS NOT NULL AND subtask_count>=0
         AND subtask_set_sha256 IS NOT NULL AND sealed_at IS NOT NULL)
    );

CREATE FUNCTION investigation_effective_delegation_census_v2(p_task_plan_id UUID)
RETURNS TABLE(
    runnable_subtask_count BIGINT,
    runnable_subtask_set_sha256 TEXT,
    dispatch_count BIGINT,
    dispatch_set_sha256 TEXT,
    pipeline_event_count BIGINT,
    pipeline_event_set_sha256 TEXT
)
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    v_dynamic BOOLEAN;
BEGIN
    SELECT COALESCE(bool_or(ledger.ledger_contract='dynamic_ordered_v2'),FALSE)
      INTO v_dynamic
      FROM investigation_refiner_plan_ledgers ledger
     WHERE ledger.task_plan_id=p_task_plan_id;

    IF v_dynamic THEN
        SELECT COUNT(*),unified_investigation_exact_set_hash(
                   'investigation_pentagi_runnable_subtasks.v1',
                   COALESCE(array_agg(subtask.member_sha256 ORDER BY subtask.subtask_ordinal),
                            ARRAY[]::TEXT[])
               )
          INTO runnable_subtask_count,runnable_subtask_set_sha256
          FROM investigation_pentagi_subtasks subtask
         WHERE subtask.task_plan_id=p_task_plan_id
           AND subtask.runnable
           AND EXISTS(
               SELECT 1
                 FROM pentagi_logical_dispatch_receipts dispatch
                 JOIN investigation_pentagi_pipeline_events barrier
                   ON barrier.task_plan_id=dispatch.task_plan_id
                  AND barrier.subtask_id=dispatch.subtask_id
                  AND barrier.event_kind='result_barrier'
                  AND barrier.actor_worker_run_id=dispatch.worker_run_id
                  AND barrier.parent_dispatch_receipt_id=dispatch.dispatch_receipt_id
                 JOIN pentagi_logical_dispatch_attempts attempt
                   ON attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                  AND attempt.outcome IN('completed','residual')
                  AND attempt.result_sha256=barrier.event_sha256
                WHERE dispatch.task_plan_id=p_task_plan_id
                  AND dispatch.subtask_id=subtask.subtask_id
                  AND dispatch.actor_kind='worker'
                  AND dispatch.parent_dispatch_receipt_id=(
                      SELECT primary_dispatch.dispatch_receipt_id
                        FROM pentagi_logical_dispatch_receipts primary_dispatch
                       WHERE primary_dispatch.task_plan_id=p_task_plan_id
                         AND primary_dispatch.actor_kind='primary'
                  )
                  AND dispatch.worker_run_id<>(
                      SELECT primary_dispatch.worker_run_id
                        FROM pentagi_logical_dispatch_receipts primary_dispatch
                       WHERE primary_dispatch.task_plan_id=p_task_plan_id
                         AND primary_dispatch.actor_kind='primary'
                  )
           );
    ELSE
        SELECT COUNT(*),unified_investigation_exact_set_hash(
                   'investigation_pentagi_runnable_subtasks.v1',
                   COALESCE(array_agg(subtask.member_sha256 ORDER BY subtask.subtask_ordinal),
                            ARRAY[]::TEXT[])
               )
          INTO runnable_subtask_count,runnable_subtask_set_sha256
          FROM investigation_pentagi_subtasks subtask
         WHERE subtask.task_plan_id=p_task_plan_id AND subtask.runnable;
    END IF;

    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'pentagi_logical_dispatch_receipts.v1',
               COALESCE(array_agg(dispatch.receipt_sha256 ORDER BY dispatch.dispatch_receipt_id),
                        ARRAY[]::TEXT[])
           )
      INTO dispatch_count,dispatch_set_sha256
      FROM pentagi_logical_dispatch_receipts dispatch
     WHERE dispatch.task_plan_id=p_task_plan_id;

    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_pentagi_pipeline_events.v1',
               COALESCE(array_agg(event.event_sha256 ORDER BY event.pipeline_event_id),
                        ARRAY[]::TEXT[])
           )
      INTO pipeline_event_count,pipeline_event_set_sha256
      FROM investigation_pentagi_pipeline_events event
     WHERE event.task_plan_id=p_task_plan_id;

    RETURN NEXT;
END;
$$;

CREATE OR REPLACE FUNCTION unified_investigation_guard_delegation_census_seal()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    primary_count BIGINT;
    actual_primary_receipt UUID;
    actual_primary_worker UUID;
    runnable_count BIGINT;
    runnable_hash TEXT;
    actual_dispatch_count BIGINT;
    dispatch_hash TEXT;
    pipeline_count BIGINT;
    pipeline_hash TEXT;
    refiner_contract TEXT;
    final_active_count BIGINT;
    final_completed_subtask_ids JSONB;
BEGIN
    PERFORM 1 FROM investigation_pentagi_task_plans
     WHERE task_plan_id=NEW.task_plan_id AND status='open' FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_NOT_OPEN' USING ERRCODE='23514';
    END IF;

    SELECT COUNT(*),
           (array_agg(dispatch_receipt_id ORDER BY dispatch_receipt_id))[1],
           (array_agg(worker_run_id ORDER BY dispatch_receipt_id))[1]
      INTO primary_count,actual_primary_receipt,actual_primary_worker
      FROM pentagi_logical_dispatch_receipts
     WHERE task_plan_id=NEW.task_plan_id AND actor_kind='primary';
    IF primary_count<>1
       OR NEW.primary_dispatch_receipt_id<>actual_primary_receipt
       OR NEW.primary_worker_run_id<>actual_primary_worker
    THEN
        RAISE EXCEPTION 'PENTAGI_CENSUS_REQUIRES_SINGLE_PRIMARY' USING ERRCODE='23514';
    END IF;

    SELECT ledger.ledger_contract
      INTO refiner_contract
      FROM investigation_refiner_plan_ledgers ledger
     WHERE ledger.task_plan_id=NEW.task_plan_id;

    IF refiner_contract='dynamic_ordered_v2' THEN
        SELECT seal.final_active_realized_subtask_count,
               patch.remaining_plan_payload->'completed_subtask_ids'
          INTO final_active_count,final_completed_subtask_ids
          FROM investigation_refiner_plan_ledger_seals seal
          JOIN investigation_refiner_plan_patches patch
            ON patch.patch_id=seal.final_patch_id
           AND patch.task_plan_id=seal.task_plan_id
         WHERE seal.task_plan_id=NEW.task_plan_id
           AND seal.seal_contract='dynamic_ordered_v2';
        IF NOT FOUND OR final_active_count<>0
           OR final_completed_subtask_ids IS NULL
           OR jsonb_typeof(final_completed_subtask_ids)<>'array'
           OR EXISTS(
               SELECT 1
                 FROM jsonb_array_elements(final_completed_subtask_ids) member(value)
                WHERE jsonb_typeof(member.value)<>'string'
                  OR NOT (trim(BOTH '"' FROM member.value::TEXT) ~*
                          '^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$')
           )
           OR (SELECT COUNT(*) FROM jsonb_array_elements(final_completed_subtask_ids))<>
              (SELECT COUNT(DISTINCT member.value#>>'{}')
                 FROM jsonb_array_elements(final_completed_subtask_ids) member(value))
        THEN
            RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_DELEGATION_REQUIRES_FINAL_ZERO'
                USING ERRCODE='23514';
        END IF;

        IF EXISTS(
            SELECT 1
              FROM pentagi_logical_dispatch_receipts dispatch
              LEFT JOIN investigation_pentagi_subtasks subtask
                ON subtask.task_plan_id=dispatch.task_plan_id
               AND subtask.subtask_id=dispatch.subtask_id
             WHERE dispatch.task_plan_id=NEW.task_plan_id
               AND dispatch.actor_kind IN('worker','nested_worker')
               AND (subtask.subtask_id IS NULL OR NOT subtask.runnable
                    OR dispatch.worker_run_id=actual_primary_worker)
        ) THEN
            RAISE EXCEPTION 'PENTAGI_DYNAMIC_DISPATCH_SUBTASK_AUTHORITY_MISMATCH'
                USING ERRCODE='23514';
        END IF;

        IF EXISTS(
            SELECT 1
              FROM investigation_pentagi_subtasks subtask
             WHERE subtask.task_plan_id=NEW.task_plan_id
               AND subtask.runnable
               AND NOT EXISTS(
                   SELECT 1
                     FROM investigation_refiner_plan_patch_members member
                    WHERE member.task_plan_id=subtask.task_plan_id
                      AND member.subtask_id=subtask.subtask_id
               )
        ) THEN
            RAISE EXCEPTION 'PENTAGI_DYNAMIC_SUBTASK_NEVER_ENTERED_ACTIVE_DENOMINATOR'
                USING ERRCODE='23514';
        END IF;

        IF EXISTS(
            SELECT 1
              FROM pentagi_logical_dispatch_receipts dispatch
             WHERE dispatch.task_plan_id=NEW.task_plan_id
               AND dispatch.actor_kind='worker'
               AND dispatch.parent_dispatch_receipt_id=actual_primary_receipt
               AND dispatch.worker_run_id<>actual_primary_worker
               AND (SELECT COUNT(*)
                      FROM investigation_pentagi_pipeline_events barrier
                      JOIN pentagi_logical_dispatch_attempts attempt
                        ON attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                       AND attempt.outcome IN('completed','residual')
                       AND attempt.result_sha256=barrier.event_sha256
                     WHERE barrier.task_plan_id=dispatch.task_plan_id
                       AND barrier.subtask_id=dispatch.subtask_id
                       AND barrier.event_kind='result_barrier'
                       AND barrier.actor_worker_run_id=dispatch.worker_run_id
                       AND barrier.parent_dispatch_receipt_id=dispatch.dispatch_receipt_id)<>1
        ) THEN
            RAISE EXCEPTION 'PENTAGI_DYNAMIC_WORKER_DISPATCH_REQUIRES_RESULT_BARRIER'
                USING ERRCODE='23514';
        END IF;

        IF EXISTS(
            SELECT 1
              FROM investigation_pentagi_pipeline_events barrier
             WHERE barrier.task_plan_id=NEW.task_plan_id
               AND barrier.event_kind='result_barrier'
               AND barrier.subtask_id IS NOT NULL
               AND NOT EXISTS(
                   SELECT 1
                     FROM pentagi_logical_dispatch_receipts dispatch
                    WHERE dispatch.task_plan_id=barrier.task_plan_id
                      AND dispatch.subtask_id=barrier.subtask_id
                      AND dispatch.actor_kind='nested_worker'
                      AND dispatch.worker_run_id=barrier.actor_worker_run_id
                      AND dispatch.dispatch_receipt_id=barrier.parent_dispatch_receipt_id
                   UNION ALL
                   SELECT 1
                     FROM pentagi_logical_dispatch_receipts dispatch
                     JOIN pentagi_logical_dispatch_attempts attempt
                       ON attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                      AND attempt.outcome IN('completed','residual')
                      AND attempt.result_sha256=barrier.event_sha256
                    WHERE dispatch.task_plan_id=barrier.task_plan_id
                      AND dispatch.subtask_id=barrier.subtask_id
                      AND dispatch.actor_kind='worker'
                      AND dispatch.parent_dispatch_receipt_id=actual_primary_receipt
                      AND dispatch.worker_run_id<>actual_primary_worker
                      AND dispatch.worker_run_id=barrier.actor_worker_run_id
                      AND dispatch.dispatch_receipt_id=barrier.parent_dispatch_receipt_id
               )
        ) THEN
            RAISE EXCEPTION 'PENTAGI_DYNAMIC_RESULT_BARRIER_REQUIRES_WORKER_DISPATCH'
                USING ERRCODE='23514';
        END IF;

        IF EXISTS(
            SELECT completed.subtask_id
              FROM (
                  SELECT dispatch.subtask_id
                    FROM pentagi_logical_dispatch_receipts dispatch
                    JOIN investigation_pentagi_pipeline_events barrier
                      ON barrier.task_plan_id=dispatch.task_plan_id
                     AND barrier.subtask_id=dispatch.subtask_id
                     AND barrier.event_kind='result_barrier'
                     AND barrier.actor_worker_run_id=dispatch.worker_run_id
                     AND barrier.parent_dispatch_receipt_id=dispatch.dispatch_receipt_id
                    JOIN pentagi_logical_dispatch_attempts attempt
                      ON attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                     AND attempt.outcome IN('completed','residual')
                     AND attempt.result_sha256=barrier.event_sha256
                   WHERE dispatch.task_plan_id=NEW.task_plan_id
                     AND dispatch.actor_kind='worker'
                     AND dispatch.parent_dispatch_receipt_id=actual_primary_receipt
                     AND dispatch.worker_run_id<>actual_primary_worker
                  GROUP BY dispatch.subtask_id
              ) completed
            EXCEPT
            SELECT (member.value#>>'{}')::UUID
              FROM jsonb_array_elements(final_completed_subtask_ids) member(value)
        ) OR EXISTS(
            SELECT (member.value#>>'{}')::UUID
              FROM jsonb_array_elements(final_completed_subtask_ids) member(value)
            EXCEPT
            SELECT completed.subtask_id
              FROM (
                  SELECT dispatch.subtask_id
                    FROM pentagi_logical_dispatch_receipts dispatch
                    JOIN investigation_pentagi_pipeline_events barrier
                      ON barrier.task_plan_id=dispatch.task_plan_id
                     AND barrier.subtask_id=dispatch.subtask_id
                     AND barrier.event_kind='result_barrier'
                     AND barrier.actor_worker_run_id=dispatch.worker_run_id
                     AND barrier.parent_dispatch_receipt_id=dispatch.dispatch_receipt_id
                    JOIN pentagi_logical_dispatch_attempts attempt
                      ON attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                     AND attempt.outcome IN('completed','residual')
                     AND attempt.result_sha256=barrier.event_sha256
                   WHERE dispatch.task_plan_id=NEW.task_plan_id
                     AND dispatch.actor_kind='worker'
                     AND dispatch.parent_dispatch_receipt_id=actual_primary_receipt
                     AND dispatch.worker_run_id<>actual_primary_worker
                  GROUP BY dispatch.subtask_id
              ) completed
        ) THEN
            RAISE EXCEPTION 'PENTAGI_DYNAMIC_COMPLETED_SUBTASK_SET_MISMATCH'
                USING ERRCODE='23514';
        END IF;
    ELSE
        IF EXISTS(
            SELECT 1 FROM investigation_pentagi_subtasks subtask
             WHERE subtask.task_plan_id=NEW.task_plan_id AND subtask.runnable
               AND NOT EXISTS(
                   SELECT 1 FROM pentagi_logical_dispatch_receipts dispatch
                    WHERE dispatch.task_plan_id=NEW.task_plan_id
                      AND dispatch.subtask_id=subtask.subtask_id
                      AND dispatch.actor_kind IN ('worker','nested_worker')
                      AND dispatch.worker_run_id<>actual_primary_worker
               )
        ) THEN
            RAISE EXCEPTION 'PENTAGI_RUNNABLE_SUBTASK_REQUIRES_DISTINCT_WORKER'
                USING ERRCODE='23514';
        END IF;
    END IF;

    IF EXISTS(
        SELECT 1 FROM pentagi_logical_dispatch_receipts dispatch
         WHERE dispatch.task_plan_id=NEW.task_plan_id
           AND NOT EXISTS(
               SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
                WHERE attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                  AND attempt.outcome<>'unknown_held'
           )
    ) OR EXISTS(
        SELECT 1 FROM pentagi_logical_dispatch_attempts attempt
        JOIN pentagi_logical_dispatch_receipts dispatch
          ON dispatch.dispatch_receipt_id=attempt.dispatch_receipt_id
        WHERE dispatch.task_plan_id=NEW.task_plan_id
          AND attempt.outcome='unknown_held'
    ) THEN
        RAISE EXCEPTION 'PENTAGI_CENSUS_HAS_UNSETTLED_DISPATCH' USING ERRCODE='23514';
    END IF;

    SELECT census.runnable_subtask_count,census.runnable_subtask_set_sha256,
           census.dispatch_count,census.dispatch_set_sha256,
           census.pipeline_event_count,census.pipeline_event_set_sha256
      INTO runnable_count,runnable_hash,actual_dispatch_count,dispatch_hash,
           pipeline_count,pipeline_hash
      FROM investigation_effective_delegation_census_v2(NEW.task_plan_id) census;

    IF ROW(NEW.runnable_subtask_count,NEW.runnable_subtask_set_sha256,
           NEW.dispatch_count,NEW.dispatch_set_sha256,
           NEW.pipeline_event_count,NEW.pipeline_event_set_sha256)
       IS DISTINCT FROM
       ROW(runnable_count,runnable_hash,actual_dispatch_count,dispatch_hash,
           pipeline_count,pipeline_hash)
    THEN
        RAISE EXCEPTION 'PENTAGI_DELEGATION_CENSUS_EXACT_SET_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

-- A genuinely empty dynamic Generator is a first-class observation.  The
-- historical v1 contract still requires at least one subtask; only the exact
-- dynamic ledger/seal/census/Primary terminal chain can seal a zero-member
-- task plan.
CREATE OR REPLACE FUNCTION unified_investigation_guard_pentagi_plan_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    actual_hash TEXT;
    dynamic_zero_authorized BOOLEAN;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_DELETE_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF OLD.status<>'open' OR NEW.status<>'sealed' OR NEW.row_version<>OLD.row_version+1
       OR ROW(NEW.task_plan_id,NEW.stable_request_id,NEW.run_request_id,
              NEW.authority_id,NEW.stage_team_plan_id,NEW.operation_id,
              NEW.stage_execution_id,NEW.owning_stage_run_request_id,
              NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
              NEW.subject_kind,NEW.subject_id,NEW.subject_fingerprint_sha256,
              NEW.task_plan_version,NEW.task_plan_sha256,NEW.allowed_role_catalog,
              NEW.cognitive_tool_envelope_sha256,NEW.created_at)
          IS DISTINCT FROM
          ROW(OLD.task_plan_id,OLD.stable_request_id,OLD.run_request_id,
              OLD.authority_id,OLD.stage_team_plan_id,OLD.operation_id,
              OLD.stage_execution_id,OLD.owning_stage_run_request_id,
              OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
              OLD.subject_kind,OLD.subject_id,OLD.subject_fingerprint_sha256,
              OLD.task_plan_version,OLD.task_plan_sha256,OLD.allowed_role_catalog,
              OLD.cognitive_tool_envelope_sha256,OLD.created_at)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_SEAL_CAS_INVALID' USING ERRCODE='23514';
    END IF;

    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_pentagi_subtasks.v1',
               COALESCE(array_agg(member_sha256 ORDER BY subtask_ordinal),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM investigation_pentagi_subtasks WHERE task_plan_id=NEW.task_plan_id;

    SELECT EXISTS(
               SELECT 1
                 FROM investigation_refiner_plan_ledgers ledger
                 JOIN investigation_refiner_plan_ledger_seals refiner_seal
                   ON refiner_seal.ledger_id=ledger.ledger_id
                  AND refiner_seal.task_plan_id=ledger.task_plan_id
                  AND refiner_seal.seal_contract='dynamic_ordered_v2'
                  AND refiner_seal.final_active_realized_subtask_count=0
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.task_plan_id=ledger.task_plan_id
                  AND census.runnable_subtask_count=0
                 JOIN investigation_effective_delegation_census_v2(ledger.task_plan_id) effective
                   ON effective.runnable_subtask_count=0
                  AND effective.runnable_subtask_set_sha256=census.runnable_subtask_set_sha256
                 JOIN pentagi_logical_dispatch_receipts primary_dispatch
                   ON primary_dispatch.task_plan_id=ledger.task_plan_id
                  AND primary_dispatch.actor_kind='primary'
                  AND primary_dispatch.dispatch_receipt_id=census.primary_dispatch_receipt_id
                  AND primary_dispatch.worker_run_id=census.primary_worker_run_id
                WHERE ledger.task_plan_id=NEW.task_plan_id
                  AND ledger.ledger_contract='dynamic_ordered_v2'
                  AND ledger.generator_subtask_count=0
                  AND (SELECT COUNT(*)
                         FROM pentagi_logical_dispatch_attempts primary_attempt
                         JOIN investigation_pentagi_pipeline_events synthesis
                           ON synthesis.task_plan_id=ledger.task_plan_id
                          AND synthesis.subtask_id IS NULL
                          AND synthesis.event_kind='primary_synthesis'
                          AND synthesis.actor_worker_run_id=
                              primary_dispatch.worker_run_id
                          AND synthesis.parent_dispatch_receipt_id=
                              primary_dispatch.dispatch_receipt_id
                          AND synthesis.event_sha256=primary_attempt.result_sha256
                        WHERE primary_attempt.dispatch_receipt_id=
                              primary_dispatch.dispatch_receipt_id
                          AND primary_attempt.outcome IN('completed','residual'))=1
           )
      INTO dynamic_zero_authorized;

    IF (actual_count=0 AND NOT dynamic_zero_authorized)
       OR NEW.subtask_count<>actual_count OR NEW.subtask_set_sha256<>actual_hash
       OR NOT EXISTS(
            SELECT 1 FROM investigation_pentagi_delegation_census_seals
             WHERE task_plan_id=NEW.task_plan_id
       )
       OR NOT EXISTS(
            SELECT 1 FROM pentagi_task_run_requests request
             WHERE request.run_request_id=NEW.run_request_id
               AND request.task_plan_id IS NULL
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PENTAGI_PLAN_EXACT_SEAL_INCOMPLETE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
