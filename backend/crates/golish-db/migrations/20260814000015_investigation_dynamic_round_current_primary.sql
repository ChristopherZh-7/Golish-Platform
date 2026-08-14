-- A discovery-triggered Analysis pass may run on an applied Asset Primary
-- execution rearm.  The immutable schedule remains the lineage root, while
-- the next Verification round must continue from the current execution leaf.
-- Keep all round guards from 00002 and replace only the no-previous-round
-- source selection with the exact current-authority projection.
CREATE OR REPLACE FUNCTION investigation_guard_dynamic_verification_round()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE lane investigation_asset_lanes%ROWTYPE;
DECLARE schedule investigation_asset_primary_schedules%ROWTYPE;
DECLARE revision attack_hypothesis_revisions%ROWTYPE;
DECLARE task hypothesis_verification_tasks%ROWTYPE;
DECLARE authz investigation_asset_verification_authorizations%ROWTYPE;
DECLARE budget investigation_asset_verification_budget_envelopes%ROWTYPE;
DECLARE source_primary_item stage_work_items%ROWTYPE;
DECLARE source_primary_worker stage_worker_runs%ROWTYPE;
DECLARE primary_item stage_work_items%ROWTYPE;
DECLARE primary_worker stage_worker_runs%ROWTYPE;
DECLARE expected_source_work_item_id UUID;
DECLARE expected_source_worker_run_id UUID;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ROUND_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF NEW.state=OLD.state AND NEW.head_version=OLD.head_version
           AND NEW.resolution_authority_id IS NOT DISTINCT FROM OLD.resolution_authority_id
           AND NEW.resolved_at IS NOT DISTINCT FROM OLD.resolved_at
           AND NEW.authorization_expires_at>OLD.authorization_expires_at
           AND (to_jsonb(NEW)-ARRAY['authorization_expires_at']) IS NOT DISTINCT FROM
               (to_jsonb(OLD)-ARRAY['authorization_expires_at'])
           AND EXISTS(SELECT 1 FROM investigation_dynamic_verification_authorization_renewals renewal
                WHERE renewal.session_id=NEW.session_id
                  AND renewal.previous_expires_at=OLD.authorization_expires_at
                  AND renewal.renewed_expires_at=NEW.authorization_expires_at)
        THEN RETURN NEW; END IF;
        IF NEW.state='open' AND OLD.state='open'
           AND NEW.head_version=OLD.head_version
           AND NEW.resolution_authority_id IS NOT DISTINCT FROM OLD.resolution_authority_id
           AND NEW.resolved_at IS NOT DISTINCT FROM OLD.resolved_at
           AND NEW.authorization_expires_at=OLD.authorization_expires_at
           AND NEW.consumed_primary_turns=OLD.consumed_primary_turns+1
           AND NEW.consumed_actor_calls>OLD.consumed_actor_calls
           AND (to_jsonb(NEW)-ARRAY['consumed_primary_turns','consumed_actor_calls'])
                IS NOT DISTINCT FROM
               (to_jsonb(OLD)-ARRAY['consumed_primary_turns','consumed_actor_calls'])
           AND EXISTS(
                SELECT 1 FROM investigation_dynamic_verification_primary_turns turn_row
                 WHERE turn_row.session_id=NEW.session_id
                   AND turn_row.decision_kind='delegate'
                   AND turn_row.turn_ordinal=NEW.consumed_primary_turns
                   AND turn_row.expected_session_head_version=OLD.head_version
                   AND turn_row.actor_call_count=
                       NEW.consumed_actor_calls-OLD.consumed_actor_calls)
        THEN RETURN NEW; END IF;
        IF (to_jsonb(NEW)-ARRAY['state','head_version','resolution_authority_id','resolved_at',
                                'consumed_primary_turns'])
             IS DISTINCT FROM
           (to_jsonb(OLD)-ARRAY['state','head_version','resolution_authority_id','resolved_at',
                                'consumed_primary_turns'])
           OR OLD.state<>'open' OR NEW.state<>'resolved'
           OR NEW.head_version<>OLD.head_version+1
           OR NEW.consumed_primary_turns<>OLD.consumed_primary_turns+1
           OR NOT EXISTS(SELECT 1 FROM investigation_dynamic_hypothesis_resolutions resolution
                WHERE resolution.resolution_authority_id=NEW.resolution_authority_id
                  AND resolution.session_id=NEW.session_id
                  AND resolution.expected_session_head_version=OLD.head_version)
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ROUND_CAS_CONFLICT'
             USING ERRCODE='40001'; END IF;
        RETURN NEW;
    END IF;
    SELECT * INTO STRICT lane FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;
    SELECT * INTO STRICT schedule FROM investigation_asset_primary_schedules
     WHERE schedule_receipt_id=NEW.asset_primary_schedule_receipt_id FOR SHARE;
    SELECT * INTO STRICT revision FROM attack_hypothesis_revisions
     WHERE revision_id=NEW.hypothesis_revision_id FOR SHARE;
    SELECT * INTO STRICT task FROM hypothesis_verification_tasks
     WHERE task_id=NEW.verification_task_id FOR SHARE;
    SELECT * INTO STRICT authz FROM investigation_asset_verification_authorizations
     WHERE session_authorization_id=NEW.session_authorization_id FOR SHARE;
    SELECT * INTO STRICT budget FROM investigation_asset_verification_budget_envelopes
     WHERE session_budget_envelope_id=NEW.session_budget_envelope_id FOR SHARE;
    SELECT * INTO STRICT source_primary_item FROM stage_work_items
     WHERE id=NEW.source_primary_work_item_id FOR SHARE;
    SELECT * INTO STRICT source_primary_worker FROM stage_worker_runs
     WHERE id=NEW.source_primary_worker_run_id FOR SHARE;
    SELECT * INTO STRICT primary_item FROM stage_work_items
     WHERE id=NEW.primary_work_item_id FOR SHARE;
    SELECT * INTO STRICT primary_worker FROM stage_worker_runs
     WHERE id=NEW.primary_worker_run_id FOR SHARE;
    SELECT previous.primary_work_item_id,previous.primary_worker_run_id
      INTO expected_source_work_item_id,expected_source_worker_run_id
      FROM investigation_dynamic_verification_rounds previous
     WHERE previous.asset_lane_id=NEW.asset_lane_id
       AND previous.evolution_epoch=NEW.evolution_epoch
       AND previous.state='resolved'
     ORDER BY previous.resolved_at DESC,previous.session_id DESC LIMIT 1;
    IF expected_source_work_item_id IS NULL THEN
        SELECT current_primary.primary_work_item_id,current_primary.primary_worker_run_id
          INTO STRICT expected_source_work_item_id,expected_source_worker_run_id
          FROM investigation_asset_primary_current_authorities current_primary
         WHERE current_primary.source_schedule_receipt_id=NEW.asset_primary_schedule_receipt_id
           AND current_primary.operation_id=NEW.operation_id
           AND current_primary.stage_execution_id=NEW.stage_execution_id
           AND current_primary.stage_run_unit_id=NEW.stage_run_unit_id
           AND current_primary.scope_snapshot_id=NEW.scope_snapshot_id
           AND current_primary.organization_id=NEW.organization_id
           AND current_primary.asset_lane_id=NEW.asset_lane_id
           AND current_primary.target_id=NEW.target_live_id
           AND current_primary.evolution_epoch=NEW.evolution_epoch
           AND current_primary.stage_team_plan_id=NEW.stage_team_plan_id
           AND current_primary.primary_message_chain_id=NEW.primary_message_chain_id;
    END IF;
    IF NEW.state<>'open' OR NEW.head_version<>0
       OR NEW.resolution_authority_id IS NOT NULL OR NEW.resolved_at IS NOT NULL
       OR lane.state<>'verifying'
       OR ROW(lane.operation_id,lane.stage_execution_id,lane.scope_snapshot_id,
              lane.organization_id,lane.target_id,lane.evolution_epoch)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.stage_execution_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.target_live_id,NEW.evolution_epoch)
       OR schedule.status<>'applied' OR schedule.schedule_contract<>'primary_dynamic_v2'
       OR ROW(schedule.asset_lane_id,schedule.target_id,schedule.evolution_epoch,
              schedule.stage_team_plan_id,schedule.primary_message_chain_id)
          IS DISTINCT FROM ROW(NEW.asset_lane_id,NEW.target_live_id,NEW.evolution_epoch,
              NEW.stage_team_plan_id,NEW.primary_message_chain_id)
       OR revision.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR revision.target_live_id IS DISTINCT FROM NEW.target_live_id
       OR revision.lifecycle_state<>'current'
       OR revision.epistemic_state IN('verified','refuted','invalid')
       OR NOT EXISTS(SELECT 1 FROM attack_hypothesis_heads head
            WHERE head.root_id=revision.root_id
              AND head.head_revision_id=NEW.hypothesis_revision_id
              AND head.head_lifecycle_state='current')
       OR task.asset_lane_id IS DISTINCT FROM NEW.asset_lane_id
       OR task.hypothesis_revision_id<>NEW.hypothesis_revision_id
       OR ROW(authz.operation_id,authz.project_scope_id,authz.stage_execution_id,
              authz.stage_run_unit_id,authz.scope_snapshot_id,authz.organization_id,
              authz.asset_lane_id,authz.target_live_id,authz.hypothesis_revision_id,
              authz.verification_task_id,authz.expires_at)
          IS DISTINCT FROM ROW(NEW.operation_id,NEW.project_scope_id,
              NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
              NEW.organization_id,NEW.asset_lane_id,NEW.target_live_id,
              NEW.hypothesis_revision_id,NEW.verification_task_id,NEW.authorization_expires_at)
       OR authz.expires_at<=statement_timestamp()
       OR budget.session_authorization_id<>NEW.session_authorization_id
       OR budget.remaining_invocations<=0
       OR ROW(NEW.source_primary_work_item_id,NEW.source_primary_worker_run_id)
          IS DISTINCT FROM ROW(expected_source_work_item_id,expected_source_worker_run_id)
       OR source_primary_item.team_plan_id<>NEW.stage_team_plan_id
       OR source_primary_item.status<>'completed'
       OR source_primary_worker.work_item_id<>NEW.source_primary_work_item_id
       OR source_primary_worker.message_chain_id<>NEW.primary_message_chain_id
       OR source_primary_worker.status<>'passed'
       OR primary_item.team_plan_id<>NEW.stage_team_plan_id
       OR primary_item.dispatch_epoch<>NEW.dispatch_epoch
       OR primary_item.kind<>'investigation_dynamic_verification_primary'
       OR primary_item.stable_key<>(
            'asset:'||NEW.asset_lane_id::TEXT||':verification:'||
            NEW.hypothesis_revision_id::TEXT||':primary')
       OR primary_item.role<>(SELECT leader_role FROM stage_team_plans
                               WHERE id=NEW.stage_team_plan_id)
       OR primary_item.required_for_barrier
       OR primary_item.output_schema<>'investigation_asset_verification_primary_resolution.v2'
       OR primary_item.created_by<>'server_seed'
       OR primary_item.status<>'queued'
       OR primary_worker.work_item_id<>NEW.primary_work_item_id
       OR primary_worker.message_chain_id<>NEW.primary_message_chain_id
       OR primary_worker.status<>'queued'
       OR NOT EXISTS(
            SELECT 1 FROM investigation_dynamic_verification_primary_continuities continuity
             WHERE continuity.session_id=NEW.session_id
               AND continuity.asset_lane_id=NEW.asset_lane_id
               AND continuity.hypothesis_revision_id=NEW.hypothesis_revision_id
               AND continuity.predecessor_work_item_id=NEW.source_primary_work_item_id
               AND continuity.predecessor_worker_run_id=NEW.source_primary_worker_run_id
               AND continuity.verification_work_item_id=NEW.primary_work_item_id
               AND continuity.verification_worker_run_id=NEW.primary_worker_run_id
               AND continuity.durable_primary_message_chain_id=NEW.primary_message_chain_id)
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ROUND_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
