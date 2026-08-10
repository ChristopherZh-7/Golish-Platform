-- A finalizer interruption may leave an immutable proposed revision whose
-- receipt belongs to an expired Worker attempt. Permit exactly one newer
-- receipt from the same Worker to reauthorize that still-unpublished proposal
-- when the canonical payload and every runtime fence remain identical.

CREATE OR REPLACE FUNCTION application_model_proposed_submission_reauthorization_is_valid(
    requested_revision_id UUID,
    prior_submission_id UUID,
    current_submission_id UUID
)
RETURNS BOOLEAN
STABLE
LANGUAGE SQL
AS $$
    SELECT EXISTS(
        SELECT 1
          FROM application_model_revisions AS revision
          JOIN stage_deliverable_submissions AS prior_submission
            ON prior_submission.id=revision.source_submission_id
           AND prior_submission.id=prior_submission_id
          JOIN stage_deliverable_submissions AS current_submission
            ON current_submission.id=current_submission_id
           AND current_submission.operation_id=prior_submission.operation_id
           AND current_submission.stage_execution_id=prior_submission.stage_execution_id
           AND current_submission.stage_run_unit_id=prior_submission.stage_run_unit_id
           AND current_submission.worker_run_id=prior_submission.worker_run_id
           AND current_submission.organization_id=prior_submission.organization_id
           AND current_submission.stage_kind=prior_submission.stage_kind
          JOIN stage_worker_runs AS worker
            ON worker.id=current_submission.worker_run_id
           AND worker.operation_id=current_submission.operation_id
           AND worker.stage_execution_id=current_submission.stage_execution_id
           AND worker.stage_run_unit_id=current_submission.stage_run_unit_id
           AND worker.organization_id=current_submission.organization_id
          JOIN stage_run_units AS unit
            ON unit.id=worker.stage_run_unit_id
           AND unit.operation_id=worker.operation_id
           AND unit.stage_execution_id=worker.stage_execution_id
           AND unit.organization_id=worker.organization_id
           AND unit.stage_kind=current_submission.stage_kind
          JOIN tool_calls AS prior_tool
            ON prior_tool.id=prior_submission.tool_call_record_id
           AND prior_tool.call_id=prior_submission.tool_request_id
           AND prior_tool.name='submit_stage_deliverable'
           AND prior_tool.status='finished'
          JOIN tool_calls AS current_tool
            ON current_tool.id=current_submission.tool_call_record_id
           AND current_tool.call_id=current_submission.tool_request_id
           AND current_tool.name='submit_stage_deliverable'
           AND current_tool.status='finished'
         WHERE revision.id=requested_revision_id
           AND revision.status='proposed'
           AND revision.row_version=0
           AND revision.finalized_at IS NULL
           AND revision.source_submission_id=prior_submission_id
           AND revision.operation_id=current_submission.operation_id
           AND revision.stage_execution_id=current_submission.stage_execution_id
           AND revision.stage_run_unit_id=current_submission.stage_run_unit_id
           AND revision.organization_id=current_submission.organization_id
           AND current_submission.stage_kind='application_understanding'
           AND prior_submission.attempt_epoch IS NOT NULL
           AND current_submission.attempt_epoch IS NOT NULL
           AND current_submission.attempt_epoch>prior_submission.attempt_epoch
           AND current_submission.attempt_epoch=worker.attempt_epoch
           AND current_submission.lease_token=worker.lease_token
           AND worker.lease_token IS NOT NULL
           AND worker.lease_expires_at>NOW()
           AND worker.status='running'
           AND worker.active_tool_call_id IS NULL
           AND worker.worker_generation=unit.generation
           AND unit.status='running'
           AND current_submission.payload=prior_submission.payload
           AND current_submission.payload_sha256=prior_submission.payload_sha256
           AND ('sha256:' || current_submission.payload_sha256)=
                application_model_sha256_jsonb(current_submission.payload)
           AND prior_tool.operation_id=prior_submission.operation_id
           AND prior_tool.stage_execution_id=prior_submission.stage_execution_id
           AND prior_tool.stage_run_unit_id=prior_submission.stage_run_unit_id
           AND prior_tool.worker_run_id=prior_submission.worker_run_id
           AND prior_tool.organization_id=prior_submission.organization_id
           AND prior_tool.attempt_epoch=prior_submission.attempt_epoch
           AND prior_tool.lease_token=prior_submission.lease_token
           AND current_tool.operation_id=current_submission.operation_id
           AND current_tool.stage_execution_id=current_submission.stage_execution_id
           AND current_tool.stage_run_unit_id=current_submission.stage_run_unit_id
           AND current_tool.worker_run_id=current_submission.worker_run_id
           AND current_tool.organization_id=current_submission.organization_id
           AND current_tool.attempt_epoch=current_submission.attempt_epoch
           AND current_tool.lease_token=current_submission.lease_token
           AND NOT EXISTS(
               SELECT 1
                 FROM application_model_current_revisions AS current_revision
                WHERE current_revision.manifest_id=revision.manifest_id
           )
    )
$$;

CREATE OR REPLACE FUNCTION application_model_restrict_revision_change()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_IMMUTABLE';
    END IF;
    IF OLD.status = 'building'
       AND NEW.status = 'proposed'
       AND NEW.row_version = OLD.row_version
       AND NEW.finalized_at IS NULL
       AND (to_jsonb(NEW) - ARRAY['status', 'row_version', 'finalized_at'])
           = (to_jsonb(OLD) - ARRAY['status', 'row_version', 'finalized_at'])
    THEN
        RETURN NEW;
    END IF;
    IF OLD.status = 'proposed'
       AND NEW.status = 'proposed'
       AND OLD.row_version = 0
       AND NEW.row_version = 0
       AND OLD.finalized_at IS NULL
       AND NEW.finalized_at IS NULL
       AND NEW.source_submission_id <> OLD.source_submission_id
       AND NEW.replay_material_hash = application_model_sha256_jsonb(
            jsonb_set(
                application_model_revision_gate_material(OLD.id),
                '{source_submission_id}',
                to_jsonb(NEW.source_submission_id)
            )
       )
       AND (to_jsonb(NEW) - ARRAY['source_submission_id', 'replay_material_hash'])
           = (to_jsonb(OLD) - ARRAY['source_submission_id', 'replay_material_hash'])
       AND application_model_proposed_submission_reauthorization_is_valid(
            OLD.id,
            OLD.source_submission_id,
            NEW.source_submission_id
       )
    THEN
        RETURN NEW;
    END IF;
    IF OLD.status = 'proposed'
       AND OLD.row_version = 0
       AND OLD.finalized_at IS NULL
       AND NEW.status = 'final'
       AND NEW.row_version = 1
       AND NEW.finalized_at = transaction_timestamp()
       AND (to_jsonb(NEW) - ARRAY['status', 'row_version', 'finalized_at'])
           = (to_jsonb(OLD) - ARRAY['status', 'row_version', 'finalized_at'])
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'APPLICATION_MODEL_REVISION_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;
