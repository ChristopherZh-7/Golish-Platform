-- Investigation compiler cutover: newly admitted canonical roots carry one
-- durable verification task whose verification strategy is materialized by
-- the dynamic per-root runtime. Historical v1 assignment/campaign rows remain
-- immutable audit records, but dynamic v2 tasks can never acquire them.

DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    SELECT task_contract_constraint.conname
      INTO STRICT constraint_name
      FROM pg_constraint task_contract_constraint
     WHERE task_contract_constraint.conrelid='hypothesis_verification_tasks'::regclass
       AND task_contract_constraint.contype='c'
       AND pg_get_constraintdef(task_contract_constraint.oid) LIKE '%task_contract_version%'
       AND pg_get_constraintdef(task_contract_constraint.oid) LIKE '%hypothesis_verification_task.v1%';
    EXECUTE format(
        'ALTER TABLE hypothesis_verification_tasks DROP CONSTRAINT %I',
        constraint_name
    );
END;
$$;

ALTER TABLE hypothesis_verification_tasks
    ADD CONSTRAINT investigation_verification_task_contract_ck
    CHECK(task_contract_version IN(
        'hypothesis_verification_task.v1',
        'hypothesis_verification_task.dynamic_v2'
    ));

COMMENT ON COLUMN hypothesis_verification_tasks.task_contract_version IS
    'v1 tasks retain historical objective/campaign reservations; dynamic_v2 tasks admit one canonical root and delegate 0..N tool/actor work to the dynamic verification runtime.';

CREATE FUNCTION investigation_guard_dynamic_task_assignment_cutover()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    task_contract TEXT;
BEGIN
    SELECT task.task_contract_version
      INTO STRICT task_contract
      FROM hypothesis_verification_tasks task
     WHERE task.task_id=NEW.task_id
       AND task.hypothesis_revision_id=NEW.hypothesis_revision_id
       AND task.verification_plan_id=NEW.verification_plan_id
     FOR SHARE;
    IF task_contract<>'hypothesis_verification_task.v1' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TASK_ASSIGNMENT_FORBIDDEN'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_verification_task_assignment_sets_dynamic_cutover
BEFORE INSERT ON hypothesis_verification_task_assignment_sets
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_task_assignment_cutover();

-- Reservation/member history was designed as immutable authority but the
-- original schema only guarded insertion while a set was open. Make the
-- retained v1 rows physically read-only during the cutover.
CREATE TRIGGER hypothesis_verification_task_campaigns_history_read_only
BEFORE UPDATE OR DELETE ON hypothesis_verification_task_campaigns
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TRIGGER hypothesis_verification_task_assignment_members_history_read_only
BEFORE UPDATE OR DELETE ON hypothesis_verification_task_assignment_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();
