-- A dynamic Generator's exact frozen subtask set is the first active
-- denominator.  Refiner patches replace that denominator; they do not need to
-- repeat Generator members which are immediately dropped.  Keep the strict
-- "every known subtask was active" guard, but recognize both the exact frozen
-- Generator prefix and every later patch member.

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
                     FROM investigation_refiner_plan_ledgers generator
                    WHERE generator.task_plan_id=subtask.task_plan_id
                      AND generator.ledger_contract='dynamic_ordered_v2'
                      AND subtask.subtask_ordinal<generator.generator_subtask_count
                      AND generator.generator_subtask_count=(
                          SELECT COUNT(*)
                            FROM investigation_pentagi_subtasks initial_subtask
                           WHERE initial_subtask.task_plan_id=generator.task_plan_id
                             AND initial_subtask.subtask_ordinal<
                                 generator.generator_subtask_count
                      )
                      AND generator.generator_subtask_set_sha256=
                          unified_investigation_exact_set_hash(
                              'investigation_refiner_generator_subtasks.v2',
                              COALESCE((
                                  SELECT array_agg(
                                             initial_subtask.subtask_id::TEXT || ':' ||
                                             initial_subtask.member_sha256
                                             ORDER BY initial_subtask.subtask_ordinal)
                                    FROM investigation_pentagi_subtasks initial_subtask
                                   WHERE initial_subtask.task_plan_id=generator.task_plan_id
                                     AND initial_subtask.subtask_ordinal<
                                         generator.generator_subtask_count
                              ),ARRAY[]::TEXT[])
                          )
               )
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
