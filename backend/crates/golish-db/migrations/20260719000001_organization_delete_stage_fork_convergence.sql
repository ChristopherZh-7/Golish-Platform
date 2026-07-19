-- Prevent a stage-test fork from snapshotting live Targets after an
-- organization deletion job has committed. The Rust materializer performs the
-- same check after acquiring live organization locks; this trigger is the
-- database-level fail-closed boundary for raw or future writers.

CREATE OR REPLACE FUNCTION validate_operation_stage_fork_target()
RETURNS trigger AS $$
DECLARE
    fork operation_stage_forks%ROWTYPE;
    canonical_project_path TEXT;
    live_target RECORD;
BEGIN
    SELECT * INTO STRICT fork
      FROM operation_stage_forks
     WHERE operation_id = NEW.operation_id
       AND target_scope_snapshot_id = NEW.scope_snapshot_id
     FOR SHARE;
    SELECT project.canonical_project_path INTO STRICT canonical_project_path
      FROM project_scopes AS project
     WHERE project.project_scope_id = fork.project_scope_id
     FOR SHARE;
    SELECT name,
           target_type::TEXT AS target_type,
           value,
           scope::TEXT AS target_scope,
           source,
           project_path,
           organization_id
      INTO STRICT live_target
      FROM targets
     WHERE id = NEW.live_target_id
     FOR SHARE;
    IF EXISTS (
        SELECT 1
          FROM organization_deletion_job_units AS unit
          JOIN organization_deletion_jobs AS job ON job.id=unit.job_id
         WHERE unit.organization_id_at_time=live_target.organization_id
           AND job.state<>'hard_delete_committed'
    ) THEN
        RAISE EXCEPTION 'stage fork Target organization deletion in progress'
            USING ERRCODE='55000';
    END IF;
    IF live_target.organization_id IS DISTINCT FROM NEW.organization_id
        OR live_target.project_path IS DISTINCT FROM canonical_project_path
        OR live_target.name IS DISTINCT FROM NEW.target_name_at_fork
        OR live_target.target_type IS DISTINCT FROM NEW.target_type_at_fork
        OR live_target.value IS DISTINCT FROM NEW.target_value_at_fork
        OR live_target.target_scope IS DISTINCT FROM NEW.target_scope_at_fork
        OR live_target.source IS DISTINCT FROM NEW.target_source_at_fork
        OR live_target.project_path IS DISTINCT FROM NEW.project_path_at_fork
    THEN
        RAISE EXCEPTION 'stage fork Target snapshot does not match the current database row';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
