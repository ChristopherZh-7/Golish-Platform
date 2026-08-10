-- A StageRunUnit generation identifies the no-purge runtime shell, while a
-- StageWorkerRun generation is the ordinal of attempts for one WorkItem.
-- They are independent counters.  Reauthorization remains fenced by the
-- exact Unit, Worker, attempt, lease, finished tool receipt, and payload.

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
