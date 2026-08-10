-- One closure-fence function serves both verification_prepared_actions and
-- verification_fact_delta_bundles. PostgreSQL resolves NEW fields for the
-- whole boolean expression, so directly referencing NEW.state fails for the
-- FactDelta row shape even when the table-name predicate is false.

CREATE OR REPLACE FUNCTION investigation_guard_late_campaign_child()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
BEGIN
    SELECT task_row.* INTO task
      FROM hypothesis_verification_task_campaigns reservation
      JOIN hypothesis_verification_tasks task_row ON task_row.task_id=reservation.task_id
     WHERE reservation.campaign_id=NEW.campaign_id FOR SHARE OF task_row;
    IF FOUND THEN
        IF TG_OP='INSERT'
           OR (TG_TABLE_NAME='verification_prepared_actions'
               AND (to_jsonb(NEW)->>'state') IN('authorized','started'))
        THEN
            PERFORM investigation_assert_stage_accepts_new_work(
                task.operation_id,task.stage_execution_id,task.scope_snapshot_id
            );
        ELSE
            PERFORM investigation_assert_stage_not_closed(
                task.operation_id,task.stage_execution_id,task.scope_snapshot_id
            );
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
