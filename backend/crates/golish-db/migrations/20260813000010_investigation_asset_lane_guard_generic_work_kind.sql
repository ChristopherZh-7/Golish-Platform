-- The lane guard is shared by rows with different shapes.  Reading a field
-- that exists only on investigation_run_work_items through NEW makes PL/pgSQL
-- resolve that field even when the table-name predicate is false.  Project the
-- optional field through JSON so the shared trigger remains valid for snapshot,
-- revision, generation, and receipt rows.
CREATE OR REPLACE FUNCTION investigation_guard_asset_hypothesis_lane()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    lane investigation_asset_lanes%ROWTYPE;
    parent_lane_id UUID;
    row_work_kind TEXT := to_jsonb(NEW)->>'work_kind';
BEGIN
    IF TG_TABLE_NAME='investigation_run_work_items'
       AND row_work_kind NOT IN ('analysis','verification_task','campaign','prepared_action',
                                 'action_execution','fact_delta','consolidation')
       AND NEW.asset_lane_id IS NULL
    THEN
        RETURN NEW;
    END IF;
    SELECT * INTO lane
      FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_LANE_NOT_FOUND' USING ERRCODE='23514';
    END IF;

    IF TG_TABLE_NAME='candidate_analysis_snapshots' THEN
        IF ROW(NEW.operation_id,NEW.scope_snapshot_id,NEW.organization_id)
           IS DISTINCT FROM ROW(lane.operation_id,lane.scope_snapshot_id,lane.organization_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_SNAPSHOT_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='candidate_analysis_attempts' THEN
        SELECT asset_lane_id INTO parent_lane_id
          FROM candidate_analysis_snapshots WHERE snapshot_id=NEW.snapshot_id;
        IF parent_lane_id IS DISTINCT FROM NEW.asset_lane_id
           OR ROW(NEW.operation_id,NEW.organization_id)
              IS DISTINCT FROM ROW(lane.operation_id,lane.organization_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_ATTEMPT_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='investigation_run_work_items' THEN
        IF row_work_kind IN ('analysis','verification_task','campaign','prepared_action',
                             'action_execution','fact_delta','consolidation')
           AND ROW(NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id,NEW.organization_id)
               IS DISTINCT FROM ROW(lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,lane.organization_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_WORK_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='attack_hypotheses' THEN
        IF ROW(NEW.operation_id,NEW.organization_id)
           IS DISTINCT FROM ROW(lane.operation_id,lane.organization_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_HYPOTHESIS_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='attack_hypothesis_revisions' THEN
        SELECT asset_lane_id INTO parent_lane_id
          FROM attack_hypotheses WHERE root_id=NEW.root_id;
        IF parent_lane_id IS DISTINCT FROM NEW.asset_lane_id
           OR ROW(NEW.operation_id,NEW.organization_id,NEW.target_live_id,
                  NEW.target_type_at_time,NEW.target_value_at_time)
              IS DISTINCT FROM ROW(lane.operation_id,lane.organization_id,lane.target_id,
                                   lane.target_type_at_freeze,lane.target_value_at_freeze)
        THEN RAISE EXCEPTION 'INVESTIGATION_REVISION_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='hypothesis_generations' THEN
        SELECT asset_lane_id INTO parent_lane_id
          FROM candidate_analysis_snapshots WHERE snapshot_id=NEW.candidate_snapshot_id;
        IF parent_lane_id IS DISTINCT FROM NEW.asset_lane_id
           OR ROW(NEW.operation_id,NEW.organization_id)
              IS DISTINCT FROM ROW(lane.operation_id,lane.organization_id)
           OR (NEW.previous_generation_id IS NOT NULL AND NOT EXISTS(
                SELECT 1 FROM hypothesis_generations previous
                 WHERE previous.generation_id=NEW.previous_generation_id
                   AND previous.asset_lane_id=NEW.asset_lane_id))
        THEN RAISE EXCEPTION 'INVESTIGATION_GENERATION_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='hypothesis_generation_members' THEN
        IF NOT EXISTS(
                SELECT 1 FROM hypothesis_generations generation
                 WHERE generation.generation_id=NEW.generation_id
                   AND generation.asset_lane_id=NEW.asset_lane_id)
           OR NOT EXISTS(
                SELECT 1 FROM attack_hypothesis_revisions revision
                 WHERE revision.revision_id=NEW.revision_id
                   AND revision.asset_lane_id=NEW.asset_lane_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_GENERATION_MEMBER_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='hypothesis_verification_tasks' THEN
        IF ROW(NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id,NEW.organization_id)
           IS DISTINCT FROM ROW(lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,lane.organization_id)
           OR NOT EXISTS(
                SELECT 1 FROM attack_hypothesis_revisions revision
                 WHERE revision.revision_id=NEW.hypothesis_revision_id
                   AND revision.asset_lane_id=NEW.asset_lane_id)
           OR NOT EXISTS(
                SELECT 1 FROM hypothesis_generations generation
                 WHERE generation.generation_id=NEW.first_admission_generation_id
                   AND generation.asset_lane_id=NEW.asset_lane_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_TASK_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='hypothesis_verification_task_campaigns' THEN
        SELECT asset_lane_id INTO parent_lane_id
          FROM hypothesis_verification_tasks WHERE task_id=NEW.task_id;
        IF parent_lane_id IS DISTINCT FROM NEW.asset_lane_id
        THEN RAISE EXCEPTION 'INVESTIGATION_TASK_CAMPAIGN_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='verification_campaigns' THEN
        IF ROW(NEW.operation_id,NEW.organization_id)
           IS DISTINCT FROM ROW(lane.operation_id,lane.organization_id)
           OR NOT EXISTS(
                SELECT 1 FROM attack_hypothesis_revisions revision
                 WHERE revision.revision_id=NEW.hypothesis_revision_id
                   AND revision.asset_lane_id=NEW.asset_lane_id)
           OR NOT EXISTS(
                SELECT 1 FROM hypothesis_verification_task_campaigns reservation
                 WHERE reservation.campaign_id=NEW.campaign_id
                   AND reservation.asset_lane_id=NEW.asset_lane_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_CAMPAIGN_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='verification_wave_coverage_denominators' THEN
        IF ROW(NEW.operation_id,NEW.organization_id)
           IS DISTINCT FROM ROW(lane.operation_id,lane.organization_id)
           OR NOT EXISTS(
                SELECT 1
                  FROM hypothesis_generation_seals seal
                  JOIN hypothesis_generations generation
                    ON generation.generation_id=seal.generation_id
                 WHERE seal.seal_id=NEW.generation_seal_id
                   AND generation.asset_lane_id=NEW.asset_lane_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_WAVE_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='hypothesis_pending_evolution_authorities' THEN
        IF ROW(NEW.operation_id,NEW.organization_id)
           IS DISTINCT FROM ROW(lane.operation_id,lane.organization_id)
           OR NOT EXISTS(
                SELECT 1 FROM hypothesis_generations generation
                 WHERE generation.generation_id=NEW.source_generation_id
                   AND generation.asset_lane_id=NEW.asset_lane_id)
           OR NOT EXISTS(
                SELECT 1 FROM verification_wave_coverage_denominators wave
                 WHERE wave.wave_denominator_id=NEW.source_wave_denominator_id
                   AND wave.asset_lane_id=NEW.asset_lane_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_PENDING_EVOLUTION_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='investigation_evolution_analysis_primary_rearms' THEN
        IF ROW(NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id,NEW.organization_id)
           IS DISTINCT FROM ROW(lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,lane.organization_id)
           OR NOT EXISTS(
                SELECT 1 FROM hypothesis_pending_evolution_authorities pending
                 WHERE pending.pending_evolution_authority_id=NEW.pending_evolution_authority_id
                   AND pending.asset_lane_id=NEW.asset_lane_id)
           OR NOT EXISTS(
                SELECT 1 FROM hypothesis_generations generation
                 WHERE generation.generation_id=NEW.source_generation_id
                   AND generation.asset_lane_id=NEW.asset_lane_id)
        THEN RAISE EXCEPTION 'INVESTIGATION_EVOLUTION_REARM_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME='hypothesis_fixed_point_receipts' THEN
        SELECT asset_lane_id INTO parent_lane_id
          FROM hypothesis_generations WHERE generation_id=NEW.generation_id;
        IF parent_lane_id IS DISTINCT FROM NEW.asset_lane_id
        THEN RAISE EXCEPTION 'INVESTIGATION_FIXED_POINT_ASSET_LANE_MISMATCH' USING ERRCODE='23514';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
