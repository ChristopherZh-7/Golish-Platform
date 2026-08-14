-- Mandatory asset-lane and exact-target authority for Investigation execution.
-- Archived assignments may predate the asset queue, but every new assignment
-- is rejected unless the whole task/campaign/revision/action chain names one
-- exact server-frozen lane and its exact live target.

ALTER TABLE investigation_verification_execution_assignments
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    ADD COLUMN target_live_id UUID REFERENCES targets(id) ON DELETE RESTRICT;

ALTER TABLE investigation_verification_execution_assignments
    ADD CONSTRAINT investigation_verification_execution_assignments_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID,
    ADD CONSTRAINT investigation_verification_execution_assignments_target_required
    CHECK(target_live_id IS NOT NULL) NOT VALID;

CREATE INDEX investigation_verification_execution_assignments_asset_lane_idx
    ON investigation_verification_execution_assignments(asset_lane_id,created_at,assignment_id)
    WHERE asset_lane_id IS NOT NULL;

CREATE OR REPLACE FUNCTION investigation_guard_verification_execution_assignment_asset()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    task hypothesis_verification_tasks%ROWTYPE;
    campaign verification_campaigns%ROWTYPE;
    revision attack_hypothesis_revisions%ROWTYPE;
    action verification_prepared_actions%ROWTYPE;
    lane investigation_asset_lanes%ROWTYPE;
BEGIN
    SELECT * INTO STRICT task FROM hypothesis_verification_tasks
     WHERE task_id=NEW.verification_task_id FOR SHARE;
    SELECT * INTO STRICT campaign FROM verification_campaigns
     WHERE campaign_id=NEW.campaign_id FOR SHARE;
    SELECT * INTO STRICT revision FROM attack_hypothesis_revisions
     WHERE revision_id=NEW.hypothesis_revision_id FOR SHARE;
    SELECT * INTO STRICT action FROM verification_prepared_actions
     WHERE prepared_action_id=NEW.prepared_action_id FOR SHARE;
    SELECT * INTO STRICT lane FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;

    IF NEW.asset_lane_id IS NULL OR NEW.target_live_id IS NULL
       OR task.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR campaign.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR revision.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR revision.target_live_id IS DISTINCT FROM NEW.target_live_id
       OR action.target_live_id IS DISTINCT FROM NEW.target_live_id
       OR lane.target_id IS DISTINCT FROM NEW.target_live_id
       OR ROW(lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,
              lane.organization_id)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,
                               NEW.scope_snapshot_id,NEW.organization_id)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_EXECUTION_ASSIGNMENT_ASSET_LANE_DRIFT'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_verification_execution_assignment_asset_guard
BEFORE INSERT ON investigation_verification_execution_assignments
FOR EACH ROW EXECUTE FUNCTION investigation_guard_verification_execution_assignment_asset();
