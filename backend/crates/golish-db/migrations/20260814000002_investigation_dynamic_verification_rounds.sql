-- Supersede the fixed-five verification round topology for new writes.
-- Historical 00008 sessions remain immutable audit records. Dynamic-v2 keeps
-- one asset Primary conversation across hypotheses while allowing that
-- Primary to append zero or more independently fenced specialist calls.

CREATE TABLE investigation_dynamic_verification_rounds (
    session_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    verification_task_id UUID NOT NULL,
    asset_primary_schedule_receipt_id UUID NOT NULL,
    evolution_epoch INTEGER NOT NULL CHECK(evolution_epoch>=0),
    round_rearm_id UUID NOT NULL UNIQUE REFERENCES
        investigation_asset_verification_round_rearms(round_rearm_id) ON DELETE RESTRICT,
    stage_team_plan_id UUID NOT NULL,
    dispatch_epoch BIGINT NOT NULL CHECK(dispatch_epoch>=0),
    session_authorization_id UUID NOT NULL UNIQUE REFERENCES
        investigation_asset_verification_authorizations(session_authorization_id) ON DELETE RESTRICT,
    session_budget_envelope_id UUID NOT NULL UNIQUE REFERENCES
        investigation_asset_verification_budget_envelopes(session_budget_envelope_id) ON DELETE RESTRICT,
    authorization_expires_at TIMESTAMPTZ NOT NULL,
    source_primary_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    source_primary_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    primary_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    primary_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    primary_message_chain_id UUID NOT NULL REFERENCES message_chains(id) ON DELETE RESTRICT,
    state TEXT NOT NULL DEFAULT 'open' CHECK(state IN('open','resolved')),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK(head_version>=0),
    resolution_authority_id UUID UNIQUE,
    opened_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    resolved_at TIMESTAMPTZ,
    CHECK((state='open' AND resolution_authority_id IS NULL AND resolved_at IS NULL)
       OR (state='resolved' AND resolution_authority_id IS NOT NULL AND resolved_at IS NOT NULL)),
    FOREIGN KEY(asset_lane_id) REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    FOREIGN KEY(target_live_id) REFERENCES targets(id) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(verification_task_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(task_id,operation_id,stage_execution_id,
                stage_run_unit_id,scope_snapshot_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(asset_primary_schedule_receipt_id)
        REFERENCES investigation_asset_primary_schedules(schedule_receipt_id) ON DELETE RESTRICT,
    FOREIGN KEY(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES stage_team_plans(id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id) ON DELETE RESTRICT
);

CREATE TABLE investigation_dynamic_verification_actor_calls (
    actor_call_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL REFERENCES investigation_dynamic_verification_rounds(session_id)
        ON DELETE RESTRICT,
    actor_ordinal BIGINT NOT NULL CHECK(actor_ordinal>0),
    subtask_id UUID NOT NULL UNIQUE REFERENCES subtasks(id) ON DELETE RESTRICT,
    specialist_role TEXT NOT NULL CHECK(
        specialist_role IN('browser','researcher','pentester','adviser','coder',
                           'installer','enricher','memorist')),
    objective_redacted JSONB NOT NULL CHECK(jsonb_typeof(objective_redacted)='object'),
    objective_sha256 TEXT NOT NULL CHECK(objective_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    worker_run_id UUID NOT NULL UNIQUE REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    message_chain_id UUID NOT NULL UNIQUE REFERENCES message_chains(id) ON DELETE RESTRICT,
    primary_turn_id UUID NOT NULL,
    turn_actor_ordinal INTEGER NOT NULL CHECK(turn_actor_ordinal BETWEEN 0 AND 7),
    actor_call_sha256 TEXT NOT NULL CHECK(actor_call_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN('queued','running','parked','completed','archived')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    completed_at TIMESTAMPTZ,
    UNIQUE(session_id,actor_ordinal),
    CHECK((state IN('completed','archived'))=(completed_at IS NOT NULL)),
    UNIQUE(primary_turn_id,turn_actor_ordinal)
);

-- One accepted Primary turn is recorded and materialized atomically.  The
-- checkpoint tuple proves which durable model turn produced the batch; the
-- counters bound even zero-tool delegation across crash/retry cycles.
ALTER TABLE investigation_dynamic_verification_rounds
    ADD COLUMN maximum_primary_turns BIGINT NOT NULL DEFAULT 32 CHECK(maximum_primary_turns>0),
    ADD COLUMN consumed_primary_turns BIGINT NOT NULL DEFAULT 0
        CHECK(consumed_primary_turns>=0 AND consumed_primary_turns<=maximum_primary_turns),
    ADD COLUMN maximum_actor_calls BIGINT NOT NULL DEFAULT 64 CHECK(maximum_actor_calls>=0),
    ADD COLUMN consumed_actor_calls BIGINT NOT NULL DEFAULT 0
        CHECK(consumed_actor_calls>=0 AND consumed_actor_calls<=maximum_actor_calls);

-- An expired session authorization may be renewed for the same frozen round.
-- Renewal cannot widen effects, credentials, risk, or replenish spent budget.
CREATE TABLE investigation_dynamic_verification_authorization_renewals (
    renewal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL REFERENCES investigation_dynamic_verification_rounds(session_id)
        ON DELETE RESTRICT,
    previous_expires_at TIMESTAMPTZ NOT NULL,
    renewed_expires_at TIMESTAMPTZ NOT NULL,
    renewal_sha256 TEXT NOT NULL CHECK(renewal_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    renewed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK(renewed_expires_at>previous_expires_at)
);

CREATE TABLE investigation_dynamic_verification_primary_turns (
    primary_turn_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL REFERENCES investigation_dynamic_verification_rounds(session_id)
        ON DELETE RESTRICT,
    turn_ordinal BIGINT NOT NULL CHECK(turn_ordinal>0),
    decision_kind TEXT NOT NULL CHECK(decision_kind IN('delegate','resolve')),
    expected_session_head_version BIGINT NOT NULL CHECK(expected_session_head_version>=0),
    source_primary_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    source_primary_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    source_primary_lease_token UUID NOT NULL,
    source_primary_attempt_epoch BIGINT NOT NULL CHECK(source_primary_attempt_epoch>=0),
    consumer_primary_lease_token UUID NOT NULL,
    consumer_primary_attempt_epoch BIGINT NOT NULL CHECK(consumer_primary_attempt_epoch>=0),
    consumer_primary_checkpoint_version BIGINT NOT NULL CHECK(consumer_primary_checkpoint_version>=0),
    consumer_primary_checkpoint_sha256 TEXT NOT NULL
        CHECK(consumer_primary_checkpoint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_tool_call_record_id UUID NOT NULL UNIQUE REFERENCES tool_calls(id) ON DELETE RESTRICT,
    source_provider_call_id TEXT NOT NULL CHECK(BTRIM(source_provider_call_id)<>''),
    canonical_turn_sha256 TEXT NOT NULL CHECK(canonical_turn_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    actor_call_count BIGINT NOT NULL CHECK(actor_call_count BETWEEN 0 AND 8),
    actor_call_set_sha256 TEXT NOT NULL CHECK(actor_call_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(session_id,turn_ordinal),
    UNIQUE(session_id,consumer_primary_checkpoint_version),
    UNIQUE(primary_turn_id,session_id),
    CHECK((decision_kind='delegate' AND actor_call_count BETWEEN 1 AND 8)
       OR (decision_kind='resolve' AND actor_call_count=0))
);
ALTER TABLE investigation_dynamic_verification_actor_calls
    ADD CONSTRAINT investigation_dynamic_verification_actor_turn_fk
        FOREIGN KEY(primary_turn_id,session_id)
        REFERENCES investigation_dynamic_verification_primary_turns(primary_turn_id,session_id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE investigation_dynamic_verification_actor_calls
    ADD COLUMN source_tool_call_record_id UUID UNIQUE REFERENCES tool_calls(id) ON DELETE RESTRICT,
    ADD COLUMN source_provider_call_id TEXT,
    ADD COLUMN canonical_observation_sha256 TEXT,
    ADD CONSTRAINT investigation_dynamic_verification_actor_submission_shape CHECK(
        (state='completed' AND source_tool_call_record_id IS NOT NULL
          AND source_provider_call_id IS NOT NULL
          AND canonical_observation_sha256 ~ '^sha256:[0-9a-f]{64}$')
        OR (state<>'completed' AND source_tool_call_record_id IS NULL
          AND source_provider_call_id IS NULL
          AND canonical_observation_sha256 IS NULL));

ALTER TABLE investigation_asset_verification_round_rearms
    ADD COLUMN round_contract TEXT NOT NULL DEFAULT 'fixed_roster_v1'
        CHECK(round_contract IN('fixed_roster_v1','primary_dynamic_v2')),
    ADD COLUMN source_primary_work_item_id UUID REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    ADD COLUMN source_primary_worker_run_id UUID REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    ADD COLUMN primary_message_chain_id UUID REFERENCES message_chains(id) ON DELETE RESTRICT;
ALTER TABLE investigation_asset_verification_round_rearms
    ADD CONSTRAINT investigation_dynamic_verification_round_rearm_shape CHECK(
        (round_contract='fixed_roster_v1' AND source_primary_work_item_id IS NULL
            AND source_primary_worker_run_id IS NULL AND primary_message_chain_id IS NULL)
        OR (round_contract='primary_dynamic_v2' AND source_primary_work_item_id IS NOT NULL
            AND source_primary_worker_run_id IS NOT NULL AND primary_message_chain_id IS NOT NULL));

ALTER TABLE investigation_dynamic_tool_inventory_snapshots
    ALTER COLUMN session_id DROP NOT NULL,
    ADD COLUMN dynamic_session_id UUID REFERENCES
        investigation_dynamic_verification_rounds(session_id) ON DELETE RESTRICT,
    ADD CONSTRAINT investigation_dynamic_tool_inventory_snapshot_one_session CHECK(
        (session_id IS NOT NULL)::INTEGER+(dynamic_session_id IS NOT NULL)::INTEGER=1);

ALTER TABLE investigation_asset_verification_invocations
    ALTER COLUMN session_id DROP NOT NULL,
    ADD COLUMN dynamic_session_id UUID REFERENCES
        investigation_dynamic_verification_rounds(session_id) ON DELETE RESTRICT,
    ADD COLUMN actor_call_id UUID REFERENCES investigation_dynamic_verification_actor_calls(actor_call_id)
        ON DELETE RESTRICT,
    ADD CONSTRAINT investigation_asset_verification_invocation_one_session CHECK(
        (session_id IS NOT NULL)::INTEGER+(dynamic_session_id IS NOT NULL)::INTEGER=1);
ALTER TABLE investigation_asset_verification_invocations
    ALTER COLUMN actor_role DROP NOT NULL,
    ALTER COLUMN actor_work_item_id DROP NOT NULL,
    ALTER COLUMN actor_worker_run_id DROP NOT NULL,
    ALTER COLUMN actor_message_chain_id DROP NOT NULL;
ALTER TABLE investigation_asset_verification_invocations
    DROP CONSTRAINT investigation_asset_verification_invocations_actor_role_check;
ALTER TABLE investigation_asset_verification_invocations
    ADD CONSTRAINT investigation_asset_verification_invocations_dynamic_actor_check CHECK(
        (dynamic_session_id IS NULL AND actor_call_id IS NULL AND actor_role IS NOT NULL
            AND actor_work_item_id IS NOT NULL AND actor_worker_run_id IS NOT NULL
            AND actor_message_chain_id IS NOT NULL)
        OR (dynamic_session_id IS NOT NULL AND session_id IS NULL AND actor_call_id IS NOT NULL
            AND actor_role IS NOT NULL AND actor_work_item_id IS NOT NULL
            AND actor_worker_run_id IS NOT NULL AND actor_message_chain_id IS NOT NULL));
DROP INDEX investigation_asset_verification_one_running_per_actor;
CREATE UNIQUE INDEX investigation_asset_verification_one_running_per_legacy_actor
    ON investigation_asset_verification_invocations(session_id,actor_role)
    WHERE dynamic_session_id IS NULL AND actor_call_id IS NULL AND state='running';
CREATE UNIQUE INDEX investigation_asset_verification_one_running_per_dynamic_actor
    ON investigation_asset_verification_invocations(dynamic_session_id,actor_call_id)
    WHERE dynamic_session_id IS NOT NULL AND actor_call_id IS NOT NULL AND state='running';

CREATE OR REPLACE FUNCTION investigation_guard_dynamic_tool_inventory_snapshot()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.dynamic_session_id IS NULL OR NEW.session_id IS NOT NULL THEN
        RAISE EXCEPTION 'INVESTIGATION_FIXED_VERIFICATION_ROUND_AUDIT_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM investigation_dynamic_verification_rounds dynamic_round
         WHERE dynamic_round.session_id=NEW.dynamic_session_id
           AND dynamic_round.state='open'
           AND dynamic_round.authorization_expires_at>statement_timestamp()
         FOR SHARE)
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TOOL_INVENTORY_SESSION_NOT_OPEN'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION investigation_guard_asset_verification_invocation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE round_row investigation_dynamic_verification_rounds%ROWTYPE;
DECLARE actor investigation_dynamic_verification_actor_calls%ROWTYPE;
DECLARE worker stage_worker_runs%ROWTYPE;
DECLARE member investigation_dynamic_tool_inventory_members%ROWTYPE;
DECLARE session_auth investigation_asset_verification_authorizations%ROWTYPE;
DECLARE session_budget investigation_asset_verification_budget_envelopes%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_INVOCATION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF OLD.dynamic_session_id IS NULL THEN
            RAISE EXCEPTION 'INVESTIGATION_FIXED_VERIFICATION_ROUND_AUDIT_ONLY'
                USING ERRCODE='23514';
        END IF;
        SELECT * INTO STRICT worker FROM stage_worker_runs
         WHERE id=OLD.actor_worker_run_id FOR SHARE;
        IF (to_jsonb(NEW)-ARRAY['state','row_version','capability_execution_receipt_id',
             'oracle_receipt_id','audit_evidence_ids','evidence_set_sha256','redacted_result',
             'result_sha256','completed_at']) IS DISTINCT FROM
           (to_jsonb(OLD)-ARRAY['state','row_version','capability_execution_receipt_id',
             'oracle_receipt_id','audit_evidence_ids','evidence_set_sha256','redacted_result',
             'result_sha256','completed_at'])
           OR OLD.state<>'running' OR NEW.state='running' OR NEW.row_version<>OLD.row_version+1
           OR worker.lease_token<>OLD.started_lease_token
           OR worker.attempt_epoch<>OLD.started_attempt_epoch
           OR worker.checkpoint_version<>OLD.started_checkpoint_version
           OR worker.status NOT IN('running','waiting_background')
           OR worker.lease_expires_at IS NULL
           OR worker.lease_expires_at<=statement_timestamp()
        THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_INVOCATION_CAS_CONFLICT'
             USING ERRCODE='40001'; END IF;
        RETURN NEW;
    END IF;
    IF NEW.dynamic_session_id IS NULL OR NEW.session_id IS NOT NULL
       OR NEW.actor_call_id IS NULL
    THEN RAISE EXCEPTION 'INVESTIGATION_FIXED_VERIFICATION_ROUND_AUDIT_ONLY'
         USING ERRCODE='23514'; END IF;
    SELECT * INTO STRICT round_row FROM investigation_dynamic_verification_rounds
     WHERE session_id=NEW.dynamic_session_id FOR SHARE;
    SELECT * INTO STRICT actor FROM investigation_dynamic_verification_actor_calls
     WHERE actor_call_id=NEW.actor_call_id AND session_id=round_row.session_id FOR SHARE;
    SELECT * INTO STRICT session_auth FROM investigation_asset_verification_authorizations
     WHERE session_authorization_id=round_row.session_authorization_id FOR SHARE;
    SELECT * INTO STRICT session_budget FROM investigation_asset_verification_budget_envelopes
     WHERE session_budget_envelope_id=round_row.session_budget_envelope_id FOR UPDATE;
    SELECT * INTO STRICT worker FROM stage_worker_runs
     WHERE id=actor.worker_run_id FOR SHARE;
    IF NEW.inventory_member_id IS NOT NULL THEN
        SELECT * INTO STRICT member FROM investigation_dynamic_tool_inventory_members
         WHERE inventory_member_id=NEW.inventory_member_id FOR SHARE;
    END IF;
    IF round_row.state<>'open' OR round_row.authorization_expires_at<=statement_timestamp()
       OR actor.state<>'running'
       OR NEW.invocation_authorization_expires_at<=statement_timestamp()
       OR NEW.invocation_authorization_expires_at>round_row.authorization_expires_at
       OR NEW.invocation_authorization_id<>uuid_generate_v5(
            round_row.session_id,'investigation-asset-verification-invocation-authorization-v1:'||
            NEW.invocation_id::TEXT)
       OR NEW.invocation_authorization_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_asset_verification_invocation_authorization.v1',
            'invocation_id',NEW.invocation_id,'session_id',round_row.session_id,
            'inventory_snapshot_id',NEW.inventory_snapshot_id,
            'inventory_member_id',NEW.inventory_member_id,'wrapper_name',NEW.wrapper_name,
            'selected_tool_name',NEW.selected_tool_name,
            'selected_tool_config_sha256',NEW.selected_tool_config_sha256,
            'effect_class',NEW.effect_class,'risk_tier',NEW.risk_tier,
            'credential_binding_sha256',NEW.credential_binding_sha256,
            'network_request_limit',NEW.network_request_limit,
            'wall_time_limit_ms',NEW.wall_time_limit_ms,
            'output_byte_limit',NEW.output_byte_limit,
            'model_args_sha256',NEW.model_args_sha256,
            'request_manifest_sha256',NEW.request_manifest_sha256,
            'expires_at',NEW.invocation_authorization_expires_at)::TEXT)
       OR NOT (session_auth.allowed_effect_classes ? NEW.effect_class)
       OR array_position(ARRAY['T0','T1','T2','T3'],NEW.risk_tier)>
          array_position(ARRAY['T0','T1','T2','T3'],session_auth.maximum_risk_tier)
       OR (NEW.credential_binding_sha256 IS NOT NULL AND NOT(
            session_auth.allowed_credential_binding_sha256s ? NEW.credential_binding_sha256))
       OR session_budget.remaining_invocations<=0
       OR session_budget.remaining_network_requests<NEW.network_request_limit
       OR session_budget.remaining_wall_time_ms<NEW.wall_time_limit_ms
       OR session_budget.remaining_output_bytes<NEW.output_byte_limit
       OR (SELECT COUNT(*) FROM investigation_asset_verification_invocations current
            WHERE current.dynamic_session_id=round_row.session_id AND current.state='running')>=
          session_budget.maximum_parallel_invocations
       OR NEW.state<>'running' OR NEW.row_version<>0 OR NEW.completed_at IS NOT NULL
       OR ROW(NEW.actor_role,NEW.actor_work_item_id,NEW.actor_worker_run_id,
              NEW.actor_message_chain_id)
          IS DISTINCT FROM ROW(actor.specialist_role,actor.work_item_id,
              actor.worker_run_id,actor.message_chain_id)
       OR worker.work_item_id<>actor.work_item_id
       OR worker.status NOT IN('running','waiting_background')
       OR worker.lease_token<>NEW.started_lease_token
       OR worker.attempt_epoch<>NEW.started_attempt_epoch
       OR worker.checkpoint_version<>NEW.started_checkpoint_version
       OR worker.lease_expires_at IS NULL OR worker.lease_expires_at<=statement_timestamp()
       OR NOT EXISTS(SELECT 1 FROM investigation_dynamic_tool_inventory_snapshots snapshot
             WHERE snapshot.inventory_snapshot_id=NEW.inventory_snapshot_id
               AND snapshot.dynamic_session_id=round_row.session_id)
       OR (NEW.inventory_member_id IS NOT NULL AND(
            member.inventory_snapshot_id<>NEW.inventory_snapshot_id
            OR member.tool_name<>NEW.selected_tool_name
            OR member.config_sha256<>NEW.selected_tool_config_sha256))
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_INVOCATION_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    UPDATE investigation_asset_verification_budget_envelopes
       SET remaining_invocations=remaining_invocations-1,
           remaining_network_requests=remaining_network_requests-NEW.network_request_limit,
           remaining_wall_time_ms=remaining_wall_time_ms-NEW.wall_time_limit_ms,
           remaining_output_bytes=remaining_output_bytes-NEW.output_byte_limit,
           row_version=row_version+1
     WHERE session_budget_envelope_id=round_row.session_budget_envelope_id;
    RETURN NEW;
END;
$$;

CREATE TABLE investigation_dynamic_hypothesis_resolutions (
    resolution_authority_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    session_id UUID NOT NULL UNIQUE REFERENCES investigation_dynamic_verification_rounds(session_id)
        ON DELETE RESTRICT,
    primary_turn_id UUID NOT NULL UNIQUE REFERENCES
        investigation_dynamic_verification_primary_turns(primary_turn_id) ON DELETE RESTRICT,
    asset_lane_id UUID NOT NULL,
    target_live_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    expected_session_head_version BIGINT NOT NULL CHECK(expected_session_head_version>=0),
    primary_work_item_id UUID NOT NULL,
    primary_worker_run_id UUID NOT NULL,
    primary_message_chain_id UUID NOT NULL,
    primary_lease_token UUID NOT NULL,
    primary_attempt_epoch BIGINT NOT NULL CHECK(primary_attempt_epoch>=0),
    primary_checkpoint_version BIGINT NOT NULL CHECK(primary_checkpoint_version>=0),
    disposition TEXT NOT NULL CHECK(disposition IN('verified','refuted','invalid')),
    primary_conclusion_sha256 TEXT NOT NULL CHECK(primary_conclusion_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    conclusion_redacted JSONB NOT NULL CHECK(jsonb_typeof(conclusion_redacted)='object'),
    citation_count BIGINT NOT NULL CHECK(citation_count>=0),
    citation_set_sha256 TEXT NOT NULL CHECK(citation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    resolution_sha256 TEXT NOT NULL CHECK(resolution_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    resolved_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE investigation_dynamic_hypothesis_terminal_transitions (
    terminal_transition_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    resolution_authority_id UUID NOT NULL UNIQUE REFERENCES
        investigation_dynamic_hypothesis_resolutions(resolution_authority_id) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED,
    asset_lane_id UUID NOT NULL REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT,
    source_revision_id UUID NOT NULL UNIQUE,
    terminal_revision_id UUID NOT NULL UNIQUE,
    state_event_id UUID NOT NULL UNIQUE,
    disposition TEXT NOT NULL CHECK(disposition IN('verified','refuted','invalid')),
    transition_sha256 TEXT NOT NULL CHECK(transition_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE investigation_dynamic_hypothesis_terminal_transition_sources (
    source_id UUID PRIMARY KEY,
    terminal_transition_id UUID NOT NULL REFERENCES
        investigation_dynamic_hypothesis_terminal_transitions(terminal_transition_id)
        ON DELETE RESTRICT,
    source_revision_id UUID NOT NULL,
    terminal_revision_id UUID NOT NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN(
        'revision_source','verification_objective','claim_component','verification_contract',
        'verification_plan')),
    source_count BIGINT NOT NULL CHECK(source_count>=0),
    source_set_sha256 TEXT NOT NULL CHECK(source_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(terminal_transition_id,source_kind)
);

CREATE TABLE investigation_dynamic_hypothesis_resolution_citations (
    citation_id UUID PRIMARY KEY,
    resolution_authority_id UUID NOT NULL REFERENCES
        investigation_dynamic_hypothesis_resolutions(resolution_authority_id) ON DELETE RESTRICT,
    citation_ordinal INTEGER NOT NULL CHECK(citation_ordinal>=0),
    citation_kind TEXT NOT NULL CHECK(citation_kind IN(
        'audit_evidence','capability_receipt','oracle_receipt','tool_invocation','other_authority')),
    audit_evidence_id BIGINT REFERENCES audit_log(id) ON DELETE RESTRICT,
    authority_id UUID,
    citation_sha256 TEXT NOT NULL CHECK(citation_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    UNIQUE(resolution_authority_id,citation_ordinal),
    CHECK((audit_evidence_id IS NOT NULL)::INTEGER+(authority_id IS NOT NULL)::INTEGER=1),
    CHECK(audit_evidence_id IS NULL OR audit_evidence_id>0)
);

CREATE TABLE investigation_dynamic_verification_actor_archives (
    archive_id UUID PRIMARY KEY,
    resolution_authority_id UUID NOT NULL REFERENCES
        investigation_dynamic_hypothesis_resolutions(resolution_authority_id) ON DELETE RESTRICT,
    session_id UUID NOT NULL REFERENCES investigation_dynamic_verification_rounds(session_id)
        ON DELETE RESTRICT,
    actor_call_id UUID NOT NULL UNIQUE REFERENCES
        investigation_dynamic_verification_actor_calls(actor_call_id) ON DELETE RESTRICT,
    prior_state TEXT NOT NULL CHECK(prior_state IN('queued','running','parked')),
    archive_sha256 TEXT NOT NULL CHECK(archive_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    archived_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE investigation_dynamic_verification_primary_completions (
    completion_id UUID PRIMARY KEY,
    session_id UUID NOT NULL UNIQUE REFERENCES
        investigation_dynamic_verification_rounds(session_id) ON DELETE RESTRICT,
    resolution_authority_id UUID NOT NULL UNIQUE REFERENCES
        investigation_dynamic_hypothesis_resolutions(resolution_authority_id) ON DELETE RESTRICT,
    primary_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    primary_lease_token UUID NOT NULL,
    primary_attempt_epoch BIGINT NOT NULL CHECK(primary_attempt_epoch>=0),
    expected_primary_checkpoint_version BIGINT NOT NULL
        CHECK(expected_primary_checkpoint_version>=0),
    expected_work_item_row_version BIGINT NOT NULL CHECK(expected_work_item_row_version>=0),
    expected_plan_row_version BIGINT NOT NULL CHECK(expected_plan_row_version>=0),
    terminal_checkpoint_sha256 TEXT NOT NULL
        CHECK(terminal_checkpoint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    stage_worker_output_id UUID NOT NULL UNIQUE REFERENCES stage_worker_outputs(id) ON DELETE RESTRICT,
    completion_sha256 TEXT NOT NULL CHECK(completion_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    completed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

ALTER TABLE investigation_pending_hypothesis_discoveries
    ALTER COLUMN resolution_authority_id DROP NOT NULL,
    ADD COLUMN dynamic_resolution_authority_id UUID REFERENCES
        investigation_dynamic_hypothesis_resolutions(resolution_authority_id) ON DELETE RESTRICT,
    ADD CONSTRAINT investigation_pending_hypothesis_discovery_one_resolution CHECK(
        (resolution_authority_id IS NOT NULL)::INTEGER+
        (dynamic_resolution_authority_id IS NOT NULL)::INTEGER=1);
ALTER TABLE investigation_pending_hypothesis_discoveries
    ADD CONSTRAINT investigation_pending_hypothesis_discovery_dynamic_ordinal_unique
        UNIQUE(dynamic_resolution_authority_id,discovery_ordinal);

-- New dynamic rows use a fresh per-hypothesis Primary phase lease while
-- retaining the single durable Asset Primary chain. The predecessor is the
-- applied dynamic schedule Primary for the first hypothesis and the previous
-- resolved round Primary thereafter.
CREATE TABLE investigation_dynamic_verification_primary_continuities (
    continuity_id UUID PRIMARY KEY,
    session_id UUID NOT NULL UNIQUE REFERENCES
        investigation_dynamic_verification_rounds(session_id) ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED,
    asset_lane_id UUID NOT NULL REFERENCES investigation_asset_lanes(asset_lane_id)
        ON DELETE RESTRICT,
    hypothesis_revision_id UUID NOT NULL UNIQUE,
    predecessor_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    predecessor_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    verification_work_item_id UUID NOT NULL UNIQUE REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    verification_worker_run_id UUID NOT NULL UNIQUE REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    durable_primary_message_chain_id UUID NOT NULL REFERENCES message_chains(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK(predecessor_work_item_id<>verification_work_item_id),
    CHECK(predecessor_worker_run_id<>verification_worker_run_id)
);
CREATE TRIGGER investigation_dynamic_verification_primary_continuity_immutable
BEFORE UPDATE OR DELETE ON investigation_dynamic_verification_primary_continuities
FOR EACH ROW EXECUTE FUNCTION investigation_reject_asset_verification_append_only();

CREATE OR REPLACE FUNCTION investigation_guard_asset_verification_round_rearm()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE schedule investigation_asset_primary_schedules%ROWTYPE;
DECLARE source_item stage_work_items%ROWTYPE;
DECLARE source_worker stage_worker_runs%ROWTYPE;
BEGIN
    IF TG_OP='DELETE'
       OR (TG_OP='UPDATE' AND (
          (to_jsonb(NEW)-ARRAY['status','applied_at']) IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY['status','applied_at'])
          OR OLD.status<>'building' OR NEW.status<>'applied'
          OR NEW.applied_at IS NULL))
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_VERIFICATION_ROUND_REARM_IMMUTABLE'
         USING ERRCODE='23514'; END IF;
    IF TG_OP='INSERT' AND NEW.round_contract='primary_dynamic_v2' THEN
        SELECT * INTO STRICT schedule FROM investigation_asset_primary_schedules
         WHERE schedule_receipt_id=NEW.asset_primary_schedule_receipt_id FOR SHARE;
        SELECT * INTO STRICT source_item FROM stage_work_items
         WHERE id=NEW.source_primary_work_item_id FOR SHARE;
        SELECT * INTO STRICT source_worker FROM stage_worker_runs
         WHERE id=NEW.source_primary_worker_run_id FOR SHARE;
        IF NEW.status<>'building' OR NEW.applied_at IS NOT NULL
           OR schedule.schedule_contract<>'primary_dynamic_v2' OR schedule.status<>'applied'
           OR ROW(schedule.asset_lane_id,schedule.target_id,schedule.stage_team_plan_id,
                  schedule.primary_message_chain_id)
              IS DISTINCT FROM ROW(NEW.asset_lane_id,NEW.target_live_id,
                  NEW.stage_team_plan_id,NEW.primary_message_chain_id)
           OR source_item.team_plan_id<>NEW.stage_team_plan_id
           OR source_item.status<>'completed'
           OR source_worker.work_item_id<>source_item.id
           OR source_worker.message_chain_id<>NEW.primary_message_chain_id
           OR source_worker.status<>'passed'
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_REARM_MISMATCH'
             USING ERRCODE='23514'; END IF;
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER investigation_asset_verification_round_rearms_guard
    ON investigation_asset_verification_round_rearms;
CREATE TRIGGER investigation_asset_verification_round_rearms_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_asset_verification_round_rearms
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_verification_round_rearm();

-- Dynamic-v2 Primary phase output is its own immutable authority. It must not
-- be misclassified as a Candidate Analysis artifact.
CREATE OR REPLACE FUNCTION enforce_candidate_artifact_recorded_output()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE artifact_count BIGINT;
BEGIN
    IF NEW.business_disposition<>'artifact_recorded'
       OR NEW.output_schema IN(
            'investigation_asset_verification_actor_observation.v1',
            'investigation_asset_verification_adviser_review.v1',
            'investigation_asset_verification_primary_resolution.v1',
            'investigation_dynamic_verification_actor_observation.v2',
            'investigation_asset_verification_primary_resolution.v2')
    THEN RETURN NULL; END IF;
    SELECT count(*) INTO artifact_count
      FROM candidate_analysis_artifacts artifact
      JOIN candidate_analysis_work_items candidate_item
        ON candidate_item.candidate_work_item_id=artifact.candidate_work_item_id
     WHERE artifact.stage_worker_output_id=NEW.id
       AND artifact.worker_run_id=NEW.worker_run_id
       AND candidate_item.stage_work_item_id=NEW.work_item_id;
    IF artifact_count<>1 THEN
        RAISE EXCEPTION 'CANDIDATE_ANALYSIS_ARTIFACT_RECORDED_EXACT_ARTIFACT_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE VIEW investigation_pending_hypothesis_discovery_backlog AS
SELECT discovery.*
  FROM investigation_pending_hypothesis_discoveries discovery
 WHERE ((discovery.resolution_authority_id IS NOT NULL AND EXISTS(
            SELECT 1 FROM investigation_asset_verification_sessions legacy
             WHERE legacy.session_id=discovery.session_id AND legacy.state='resolved'))
        OR (discovery.dynamic_resolution_authority_id IS NOT NULL AND EXISTS(
            SELECT 1 FROM investigation_dynamic_verification_rounds dynamic_round
             WHERE dynamic_round.session_id=discovery.session_id
               AND dynamic_round.state='resolved')))
   AND NOT EXISTS(SELECT 1 FROM investigation_pending_hypothesis_discovery_consumptions consumed
                   WHERE consumed.discovery_authority_id=discovery.discovery_authority_id);

CREATE FUNCTION investigation_guard_dynamic_verification_round()
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
        -- An accepted delegate turn consumes durable Primary/actor fuel while
        -- keeping the same open round authority.  The just-inserted immutable
        -- turn is the only writer permitted to advance these counters.
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
        expected_source_work_item_id:=schedule.primary_work_item_id;
        expected_source_worker_run_id:=schedule.primary_worker_run_id;
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
CREATE TRIGGER investigation_dynamic_verification_rounds_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_dynamic_verification_rounds
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_verification_round();

CREATE FUNCTION investigation_guard_dynamic_verification_authorization_renewal()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE round_row investigation_dynamic_verification_rounds%ROWTYPE;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_RENEWAL_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT round_row FROM investigation_dynamic_verification_rounds
     WHERE session_id=NEW.session_id FOR UPDATE;
    IF round_row.state<>'open'
       OR round_row.authorization_expires_at<>NEW.renewed_expires_at
       OR NEW.renewed_expires_at<=statement_timestamp()
       OR NEW.renewed_expires_at>statement_timestamp()+INTERVAL '4 hours 1 minute'
       OR NEW.renewal_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_dynamic_verification_authorization_renewal.v1',
            'renewal_id',NEW.renewal_id,'session_id',NEW.session_id,
            'previous_expires_at',NEW.previous_expires_at,
            'renewed_expires_at',NEW.renewed_expires_at)::TEXT)
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_RENEWAL_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_verification_authorization_renewals_guard
BEFORE UPDATE OR DELETE ON investigation_dynamic_verification_authorization_renewals
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_verification_authorization_renewal();
CREATE CONSTRAINT TRIGGER investigation_dynamic_verification_authorization_renewals_insert_guard
AFTER INSERT ON investigation_dynamic_verification_authorization_renewals
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_guard_dynamic_verification_authorization_renewal();
CREATE FUNCTION investigation_guard_dynamic_verification_round_renewed_expiry()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM investigation_dynamic_verification_authorization_renewals renewal
       WHERE renewal.session_id=NEW.session_id
         AND renewal.previous_expires_at=OLD.authorization_expires_at
         AND renewal.renewed_expires_at=NEW.authorization_expires_at)
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_RENEWAL_REQUIRED'
         USING ERRCODE='23514'; END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_verification_round_renewed_expiry_exact
AFTER UPDATE OF authorization_expires_at ON investigation_dynamic_verification_rounds
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_guard_dynamic_verification_round_renewed_expiry();

CREATE OR REPLACE FUNCTION investigation_guard_pending_hypothesis_discovery()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE legacy_resolution investigation_hypothesis_resolution_authorities%ROWTYPE;
DECLARE dynamic_resolution investigation_dynamic_hypothesis_resolutions%ROWTYPE;
DECLARE resolved_session_id UUID;
DECLARE resolved_asset_lane_id UUID;
DECLARE resolved_target_live_id UUID;
DECLARE resolved_revision_id UUID;
DECLARE lane investigation_asset_lanes%ROWTYPE;
DECLARE expected_subject_identity_sha256 TEXT;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_PENDING_HYPOTHESIS_DISCOVERY_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF NEW.dynamic_resolution_authority_id IS NOT NULL THEN
        SELECT * INTO STRICT dynamic_resolution FROM investigation_dynamic_hypothesis_resolutions
         WHERE resolution_authority_id=NEW.dynamic_resolution_authority_id FOR SHARE;
        resolved_session_id:=dynamic_resolution.session_id;
        resolved_asset_lane_id:=dynamic_resolution.asset_lane_id;
        resolved_target_live_id:=dynamic_resolution.target_live_id;
        resolved_revision_id:=dynamic_resolution.hypothesis_revision_id;
    ELSE
        SELECT * INTO STRICT legacy_resolution FROM investigation_hypothesis_resolution_authorities
         WHERE resolution_authority_id=NEW.resolution_authority_id FOR SHARE;
        resolved_session_id:=legacy_resolution.session_id;
        resolved_asset_lane_id:=legacy_resolution.asset_lane_id;
        resolved_target_live_id:=legacy_resolution.target_live_id;
        resolved_revision_id:=legacy_resolution.hypothesis_revision_id;
    END IF;
    SELECT * INTO STRICT lane FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR SHARE;
    expected_subject_identity_sha256:=tool_truth_sha256(jsonb_build_object(
        'domain','investigation_subject_identity.v1','subject_kind','asset',
        'subject_id',lane.target_id,'display_value',lane.target_value_at_freeze)::TEXT);
    IF ROW(NEW.session_id,NEW.asset_lane_id,NEW.target_live_id,NEW.source_hypothesis_revision_id)
       IS DISTINCT FROM ROW(resolved_session_id,resolved_asset_lane_id,
           resolved_target_live_id,resolved_revision_id)
       OR NEW.subject_kind<>'asset'
       OR NEW.subject_identity_sha256<>expected_subject_identity_sha256
       OR NEW.semantic_key_sha256<>(
          'sha256:'||encode(digest(convert_to('hypothesis_semantic_key.v1','UTF8')||
          decode('00','hex')||convert_to(NEW.semantic_key_canonical_json,'UTF8'),'sha256'),'hex'))
       OR NEW.semantic_key_canonical_json::JSONB #>> '{subject,kind}'<>'asset'
       OR NEW.semantic_key_canonical_json::JSONB #>> '{subject,identity_hash}'<>
          expected_subject_identity_sha256
       OR NEW.semantic_key_canonical_json::JSONB->>'organization_id'<>lane.organization_id::TEXT
       OR NEW.structured_claim_sha256<>tool_truth_sha256(to_jsonb(NEW.structured_claim)::TEXT)
       OR NEW.canonical_proposal->>'proposal_id'<>NEW.discovery_authority_id::TEXT
       OR NEW.canonical_proposal->>'subject_kind'<>'asset'
       OR NEW.canonical_proposal->>'subject_identity_hash'<>expected_subject_identity_sha256
       OR NEW.canonical_proposal->>'predicate_schema'<>
          NEW.semantic_key_canonical_json::JSONB #>> '{predicate,schema}'
       OR NEW.canonical_proposal->>'predicate_version'<>
          NEW.semantic_key_canonical_json::JSONB #>> '{predicate,version}'
       OR NEW.canonical_proposal->'predicate_arguments'<>
          NEW.semantic_key_canonical_json::JSONB #> '{predicate,normalized_arguments}'
       OR NEW.canonical_proposal->>'trust_boundary'<>
          NEW.semantic_key_canonical_json::JSONB->>'trust_boundary'
       OR NEW.canonical_proposal->>'polarity'<>
          NEW.semantic_key_canonical_json::JSONB->>'polarity'
       OR NEW.canonical_proposal->>'structured_claim'<>NEW.structured_claim
       OR NEW.canonical_proposal->'proof_refs'<>'[]'::JSONB
       OR NEW.canonical_proposal->'knowledge_signals'<>'[]'::JSONB
       OR NEW.canonical_proposal->>'readiness'<>'ready_for_strategy'
       OR NEW.discovery_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_pending_hypothesis_discovery.v1',
            'resolution_authority_id',COALESCE(NEW.dynamic_resolution_authority_id,
                                               NEW.resolution_authority_id),
            'asset_lane_id',NEW.asset_lane_id,'target_live_id',NEW.target_live_id,
            'source_hypothesis_revision_id',NEW.source_hypothesis_revision_id,
            'discovery_ordinal',NEW.discovery_ordinal,'subject_kind',NEW.subject_kind,
            'subject_identity_sha256',NEW.subject_identity_sha256,
            'semantic_key_sha256',NEW.semantic_key_sha256,
            'structured_claim_sha256',NEW.structured_claim_sha256,
            'canonical_proposal',NEW.canonical_proposal,
            'preconditions',NEW.canonical_proposal->'preconditions',
            'impact',NEW.canonical_proposal->'impact',
            'rationale',NEW.rationale_redacted->'rationale')::TEXT)
    THEN RAISE EXCEPTION 'INVESTIGATION_PENDING_HYPOTHESIS_DISCOVERY_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION investigation_guard_dynamic_verification_actor_call()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE round_row investigation_dynamic_verification_rounds%ROWTYPE;
DECLARE item stage_work_items%ROWTYPE;
DECLARE worker stage_worker_runs%ROWTYPE;
DECLARE chain message_chains%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ACTOR_APPEND_ONLY' USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(NEW)-ARRAY['state','completed_at','source_tool_call_record_id',
             'source_provider_call_id','canonical_observation_sha256']) IS DISTINCT FROM
           (to_jsonb(OLD)-ARRAY['state','completed_at','source_tool_call_record_id',
             'source_provider_call_id','canonical_observation_sha256'])
           OR NOT ((OLD.state='queued' AND NEW.state='running')
                OR (OLD.state='running' AND NEW.state IN('parked','completed'))
                OR (OLD.state='parked' AND NEW.state IN('running','completed'))
                OR (OLD.state IN('queued','running','parked') AND NEW.state='archived'))
           OR (NEW.state='archived' AND NOT EXISTS(
                SELECT 1 FROM investigation_dynamic_verification_actor_archives archive
                 WHERE archive.actor_call_id=NEW.actor_call_id
                   AND archive.session_id=NEW.session_id
                   AND archive.prior_state=OLD.state))
           OR (NEW.state='archived' AND
               ROW(NEW.source_tool_call_record_id,NEW.source_provider_call_id,
                   NEW.canonical_observation_sha256) IS DISTINCT FROM
               ROW(OLD.source_tool_call_record_id,OLD.source_provider_call_id,
                   OLD.canonical_observation_sha256))
           OR (NEW.state='completed' AND NOT EXISTS(
                SELECT 1 FROM tool_calls source_call
                 JOIN stage_work_items completed_item ON completed_item.id=NEW.work_item_id
                 JOIN stage_worker_runs completed_worker ON completed_worker.id=NEW.worker_run_id
                 JOIN stage_worker_outputs output
                   ON output.work_item_id=completed_item.id
                  AND output.worker_run_id=completed_worker.id
                 WHERE source_call.id=NEW.source_tool_call_record_id
                   AND source_call.call_id=NEW.source_provider_call_id
                   AND source_call.worker_run_id=NEW.worker_run_id
                   AND source_call.name='submit_result'
                   AND source_call.status='finished'
                   AND source_call.result IS NOT NULL
                   AND source_call.result::JSONB->>'status'='result submitted'
                   AND source_call.args ? 'result'
                   AND tool_truth_sha256((source_call.args->'result')::TEXT)=
                       NEW.canonical_observation_sha256
                   AND completed_item.status='completed'
                   AND completed_worker.status='passed'
                   AND completed_worker.active_tool_call_id IS NULL
                   AND output.output_schema=
                       'investigation_dynamic_verification_actor_observation.v2'
                   AND output.canonical_output=source_call.args->'result'))
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ACTOR_CAS_CONFLICT'
             USING ERRCODE='40001'; END IF;
        RETURN NEW;
    END IF;
    SELECT * INTO STRICT round_row FROM investigation_dynamic_verification_rounds
     WHERE session_id=NEW.session_id FOR UPDATE;
    SELECT * INTO STRICT item FROM stage_work_items WHERE id=NEW.work_item_id FOR SHARE;
    SELECT * INTO STRICT worker FROM stage_worker_runs WHERE id=NEW.worker_run_id FOR SHARE;
    SELECT * INTO STRICT chain FROM message_chains WHERE id=NEW.message_chain_id FOR SHARE;
    IF round_row.state<>'open' OR round_row.authorization_expires_at<=statement_timestamp()
       OR NEW.actor_ordinal<>(SELECT COALESCE(MAX(existing.actor_ordinal),0)+1
            FROM investigation_dynamic_verification_actor_calls existing
            WHERE existing.session_id=NEW.session_id)
       OR ROW(item.team_plan_id,item.operation_id,item.stage_execution_id,item.stage_run_unit_id,
              item.scope_snapshot_id,item.organization_id,item.dispatch_epoch,item.kind,item.role,
              item.required_for_barrier,item.output_schema,item.created_by)
          IS DISTINCT FROM ROW(round_row.stage_team_plan_id,round_row.operation_id,
              round_row.stage_execution_id,round_row.stage_run_unit_id,round_row.scope_snapshot_id,
              round_row.organization_id,round_row.dispatch_epoch,
              'investigation_dynamic_verification_actor',NEW.specialist_role,FALSE,
              'investigation_dynamic_verification_actor_observation.v2','accepted_worker_request')
       OR ROW(worker.work_item_id,worker.operation_id,worker.stage_execution_id,
              worker.stage_run_unit_id,worker.organization_id,worker.specialist,
              worker.work_item_kind,worker.work_item_key,worker.message_chain_id)
          IS DISTINCT FROM ROW(NEW.work_item_id,round_row.operation_id,
              round_row.stage_execution_id,round_row.stage_run_unit_id,
              round_row.organization_id,NEW.specialist_role,item.kind,item.stable_key,NEW.message_chain_id)
       OR worker.status<>'queued'
       OR ROW(chain.id,chain.task_id,chain.subtask_id,chain.agent)
          IS DISTINCT FROM ROW(NEW.message_chain_id,round_row.operation_id,NEW.subtask_id,
              CASE NEW.specialist_role
                WHEN 'browser' THEN 'pentester'::agent_type
                WHEN 'researcher' THEN 'searcher'::agent_type
                ELSE NEW.specialist_role::agent_type END)
       OR NEW.objective_sha256<>tool_truth_sha256(NEW.objective_redacted::TEXT)
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ACTOR_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_verification_actor_calls_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_dynamic_verification_actor_calls
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_verification_actor_call();

CREATE FUNCTION investigation_guard_dynamic_verification_primary_turn()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE round_row investigation_dynamic_verification_rounds%ROWTYPE;
DECLARE worker stage_worker_runs%ROWTYPE;
DECLARE source_call tool_calls%ROWTYPE;
DECLARE actual_count BIGINT;
DECLARE actual_set TEXT;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT round_row FROM investigation_dynamic_verification_rounds
     WHERE session_id=NEW.session_id FOR UPDATE;
    SELECT * INTO STRICT worker FROM stage_worker_runs
     WHERE id=NEW.source_primary_worker_run_id FOR SHARE;
    SELECT * INTO STRICT source_call FROM tool_calls
     WHERE id=NEW.source_tool_call_record_id FOR SHARE;
    IF round_row.state<>'open'
       OR ROW(NEW.source_primary_work_item_id,NEW.source_primary_worker_run_id)
          IS DISTINCT FROM ROW(round_row.primary_work_item_id,round_row.primary_worker_run_id)
       OR worker.work_item_id<>NEW.source_primary_work_item_id
       OR worker.lease_token<>NEW.consumer_primary_lease_token
       OR worker.attempt_epoch<>NEW.consumer_primary_attempt_epoch
       OR worker.checkpoint_version<>NEW.consumer_primary_checkpoint_version
       OR tool_truth_sha256(worker.checkpoint::TEXT)<>NEW.consumer_primary_checkpoint_sha256
       OR worker.status<>'running' OR worker.active_tool_call_id IS NOT NULL
       OR worker.lease_expires_at IS NULL OR worker.lease_expires_at<=statement_timestamp()
       OR ROW(source_call.call_id,source_call.operation_id,source_call.stage_execution_id,
              source_call.stage_run_unit_id,source_call.organization_id,
              source_call.worker_run_id,source_call.name,source_call.status)
          IS DISTINCT FROM ROW(NEW.source_provider_call_id,round_row.operation_id,
              round_row.stage_execution_id,round_row.stage_run_unit_id,
              round_row.organization_id,round_row.primary_worker_run_id,
              'submit_result','finished'::toolcall_status)
       OR source_call.attempt_epoch<>NEW.source_primary_attempt_epoch
       OR source_call.lease_token<>NEW.source_primary_lease_token
       OR source_call.result IS NULL
       OR source_call.result::JSONB->>'status'<>'result submitted'
       OR NOT(source_call.args ? 'result')
       OR tool_truth_sha256((source_call.args->'result')::TEXT)<>NEW.canonical_turn_sha256
       OR NEW.expected_session_head_version<>round_row.head_version
       OR NEW.turn_ordinal<>round_row.consumed_primary_turns+1
       OR round_row.consumed_primary_turns>=round_row.maximum_primary_turns
       OR round_row.consumed_actor_calls+NEW.actor_call_count>round_row.maximum_actor_calls
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_verification_primary_turns_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_dynamic_verification_primary_turns
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_verification_primary_turn();

CREATE FUNCTION investigation_validate_dynamic_verification_primary_turn_census()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_turn_id UUID := COALESCE(NEW.primary_turn_id,OLD.primary_turn_id);
DECLARE actual_count BIGINT;
DECLARE actual_set TEXT;
BEGIN
    SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(actor_call_sha256
             ORDER BY turn_actor_ordinal)::TEXT,'[]'))
      INTO actual_count,actual_set
      FROM investigation_dynamic_verification_actor_calls
     WHERE primary_turn_id=requested_turn_id;
    IF EXISTS(SELECT 1 FROM investigation_dynamic_verification_primary_turns turn_row
       WHERE turn_row.primary_turn_id=requested_turn_id
         AND ROW(turn_row.actor_call_count,turn_row.actor_call_set_sha256)
             IS DISTINCT FROM ROW(actual_count,actual_set))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_CENSUS_DRIFT'
         USING ERRCODE='23514'; END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_verification_primary_turn_actor_census
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_verification_actor_calls
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_verification_primary_turn_census();
CREATE CONSTRAINT TRIGGER investigation_dynamic_verification_primary_turn_parent_census
AFTER INSERT ON investigation_dynamic_verification_primary_turns
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_verification_primary_turn_census();

CREATE CONSTRAINT TRIGGER investigation_dynamic_tool_inventory_parent_census_exact
AFTER INSERT ON investigation_dynamic_tool_inventory_snapshots
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_tool_inventory();

CREATE FUNCTION investigation_guard_dynamic_hypothesis_resolution()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE round_row investigation_dynamic_verification_rounds%ROWTYPE;
DECLARE primary_worker stage_worker_runs%ROWTYPE;
DECLARE primary_turn investigation_dynamic_verification_primary_turns%ROWTYPE;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_HYPOTHESIS_RESOLUTION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT round_row FROM investigation_dynamic_verification_rounds
     WHERE session_id=NEW.session_id FOR UPDATE;
    SELECT * INTO STRICT primary_worker FROM stage_worker_runs
     WHERE id=NEW.primary_worker_run_id FOR SHARE;
    SELECT * INTO STRICT primary_turn FROM investigation_dynamic_verification_primary_turns
     WHERE primary_turn_id=NEW.primary_turn_id FOR SHARE;
    IF round_row.state<>'open' OR round_row.head_version<>NEW.expected_session_head_version
       OR ROW(NEW.asset_lane_id,NEW.target_live_id,NEW.hypothesis_revision_id,
              NEW.primary_work_item_id,NEW.primary_worker_run_id,NEW.primary_message_chain_id)
          IS DISTINCT FROM ROW(round_row.asset_lane_id,round_row.target_live_id,
              round_row.hypothesis_revision_id,round_row.primary_work_item_id,
              round_row.primary_worker_run_id,round_row.primary_message_chain_id)
       OR NOT EXISTS(SELECT 1 FROM investigation_dynamic_verification_primary_turns turn_row
              WHERE turn_row.primary_turn_id=NEW.primary_turn_id
                AND turn_row.session_id=NEW.session_id
                AND turn_row.decision_kind='resolve'
                AND turn_row.actor_call_count=0
                AND turn_row.expected_session_head_version=NEW.expected_session_head_version)
       OR primary_worker.lease_token<>NEW.primary_lease_token
       OR primary_worker.attempt_epoch<>NEW.primary_attempt_epoch
       OR primary_worker.checkpoint_version<>NEW.primary_checkpoint_version
       OR primary_worker.status NOT IN('running','waiting_background')
       OR primary_worker.lease_expires_at IS NULL
       OR primary_worker.lease_expires_at<=statement_timestamp()
       OR NEW.resolution_sha256<>tool_truth_sha256(jsonb_build_object(
            'domain','investigation_dynamic_hypothesis_resolution.v2',
            'session_id',NEW.session_id,'hypothesis_revision_id',NEW.hypothesis_revision_id,
            'primary_turn_id',NEW.primary_turn_id,
            'canonical_turn_sha256',primary_turn.canonical_turn_sha256,
            'disposition',NEW.disposition,
            'primary_conclusion_sha256',NEW.primary_conclusion_sha256,
            'citation_set_sha256',NEW.citation_set_sha256)::TEXT)
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_HYPOTHESIS_RESOLUTION_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_hypothesis_resolutions_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_dynamic_hypothesis_resolutions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_hypothesis_resolution();

CREATE FUNCTION investigation_guard_dynamic_hypothesis_resolution_citation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE resolution investigation_dynamic_hypothesis_resolutions%ROWTYPE;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_HYPOTHESIS_RESOLUTION_CITATION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO STRICT resolution FROM investigation_dynamic_hypothesis_resolutions
     WHERE resolution_authority_id=NEW.resolution_authority_id FOR SHARE;
    IF (NEW.audit_evidence_id IS NOT NULL AND NOT EXISTS(
            SELECT 1 FROM investigation_asset_verification_invocations invocation
             WHERE invocation.dynamic_session_id=resolution.session_id
               AND invocation.state='succeeded'
               AND NEW.audit_evidence_id=ANY(invocation.audit_evidence_ids)))
       OR (NEW.authority_id IS NOT NULL AND NOT EXISTS(
            SELECT 1 FROM investigation_asset_verification_invocations invocation
             WHERE invocation.dynamic_session_id=resolution.session_id
               AND invocation.state='succeeded'
               AND NEW.authority_id IN(invocation.invocation_id,
                   invocation.capability_execution_receipt_id,invocation.oracle_receipt_id)))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_HYPOTHESIS_RESOLUTION_CITATION_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_hypothesis_resolution_citations_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_dynamic_hypothesis_resolution_citations
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_hypothesis_resolution_citation();

CREATE FUNCTION investigation_guard_dynamic_verification_actor_archive()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ACTOR_ARCHIVE_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM investigation_dynamic_hypothesis_resolutions resolution
         JOIN investigation_dynamic_verification_actor_calls actor
           ON actor.session_id=resolution.session_id
         JOIN stage_work_items item ON item.id=actor.work_item_id
         JOIN stage_worker_runs worker ON worker.id=actor.worker_run_id
        WHERE resolution.resolution_authority_id=NEW.resolution_authority_id
          AND resolution.session_id=NEW.session_id AND actor.actor_call_id=NEW.actor_call_id
          AND actor.state='archived'
          AND item.status IN('superseded','exhausted')
          AND item.terminal_at IS NOT NULL
          AND worker.status IN('superseded','failed','exhausted')
          AND worker.terminal_at IS NOT NULL
          AND worker.active_tool_call_id IS NULL
          AND NEW.archive_sha256=tool_truth_sha256(jsonb_build_object(
              'domain','investigation_dynamic_verification_actor_archive.v1',
              'resolution_authority_id',NEW.resolution_authority_id,
              'session_id',NEW.session_id,'actor_call_id',NEW.actor_call_id,
              'prior_state',NEW.prior_state)::TEXT))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_ACTOR_ARCHIVE_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_verification_actor_archives_guard
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_verification_actor_archives
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_guard_dynamic_verification_actor_archive();

-- A resolved round has no live child work. Completed actors keep their
-- observation authority; every other accepted actor has exactly one immutable
-- archive receipt and a superseded WorkItem/WorkerRun.
CREATE FUNCTION investigation_validate_dynamic_resolution_actor_census()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_resolution_id UUID := COALESCE(NEW.resolution_authority_id,
                                                  OLD.resolution_authority_id);
BEGIN
    IF EXISTS(
        SELECT 1 FROM investigation_dynamic_hypothesis_resolutions resolution
         JOIN investigation_dynamic_verification_rounds dynamic_round
           ON dynamic_round.session_id=resolution.session_id
        WHERE resolution.resolution_authority_id=requested_resolution_id
          AND (dynamic_round.state<>'resolved'
            OR dynamic_round.resolution_authority_id<>resolution.resolution_authority_id
            OR EXISTS(
                SELECT 1 FROM investigation_dynamic_verification_actor_calls actor
                 WHERE actor.session_id=resolution.session_id
                   AND actor.state NOT IN('completed','archived'))
            OR EXISTS(
                SELECT 1 FROM investigation_dynamic_verification_actor_calls actor
                 LEFT JOIN stage_work_items item ON item.id=actor.work_item_id
                 LEFT JOIN stage_worker_runs worker ON worker.id=actor.worker_run_id
                 LEFT JOIN stage_worker_outputs output
                   ON output.work_item_id=actor.work_item_id
                  AND output.worker_run_id=actor.worker_run_id
                 WHERE actor.session_id=resolution.session_id AND actor.state='completed'
                   AND (item.status<>'completed' OR worker.status<>'passed'
                     OR worker.active_tool_call_id IS NOT NULL OR output.id IS NULL
                     OR output.output_schema<>
                        'investigation_dynamic_verification_actor_observation.v2'
                     OR tool_truth_sha256(output.canonical_output::TEXT)<>
                        actor.canonical_observation_sha256))
            OR EXISTS(
                SELECT 1 FROM investigation_dynamic_verification_actor_calls actor
                 WHERE actor.session_id=resolution.session_id AND actor.state='archived'
                   AND NOT EXISTS(
                       SELECT 1 FROM investigation_dynamic_verification_actor_archives archive
                        WHERE archive.resolution_authority_id=resolution.resolution_authority_id
                          AND archive.session_id=resolution.session_id
                          AND archive.actor_call_id=actor.actor_call_id))))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_RESOLUTION_ACTOR_CENSUS_DRIFT'
         USING ERRCODE='23514'; END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_resolution_actor_parent_census
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_hypothesis_resolutions
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_resolution_actor_census();
CREATE CONSTRAINT TRIGGER investigation_dynamic_resolution_actor_archive_census
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_verification_actor_archives
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_resolution_actor_census();

-- Exact census prevents committing a resolution that merely claims a set hash.
CREATE FUNCTION investigation_validate_dynamic_hypothesis_resolution_census()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_resolution_id UUID := COALESCE(NEW.resolution_authority_id,OLD.resolution_authority_id);
BEGIN
    IF EXISTS(SELECT 1 FROM investigation_dynamic_hypothesis_resolutions resolution
       WHERE resolution.resolution_authority_id=requested_resolution_id
         AND ROW(resolution.citation_count,resolution.citation_set_sha256) IS DISTINCT FROM ROW(
           (SELECT COUNT(*) FROM investigation_dynamic_hypothesis_resolution_citations citation
             WHERE citation.resolution_authority_id=requested_resolution_id),
           tool_truth_sha256(COALESCE((SELECT jsonb_agg(citation.citation_sha256
             ORDER BY citation.citation_ordinal)::TEXT
             FROM investigation_dynamic_hypothesis_resolution_citations citation
             WHERE citation.resolution_authority_id=requested_resolution_id),'[]'))))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_HYPOTHESIS_RESOLUTION_CENSUS_DRIFT'
         USING ERRCODE='23514'; END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_hypothesis_resolution_census_exact
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_hypothesis_resolution_citations
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_hypothesis_resolution_census();
CREATE CONSTRAINT TRIGGER investigation_dynamic_hypothesis_resolution_parent_census_exact
AFTER INSERT ON investigation_dynamic_hypothesis_resolutions
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_hypothesis_resolution_census();

ALTER TABLE investigation_dynamic_verification_rounds
    ADD CONSTRAINT investigation_dynamic_verification_round_resolution_fk
    FOREIGN KEY(resolution_authority_id)
        REFERENCES investigation_dynamic_hypothesis_resolutions(resolution_authority_id)
        ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE attack_hypothesis_state_events
    DROP CONSTRAINT attack_hypothesis_state_events_origin_authority_check,
    ADD CONSTRAINT attack_hypothesis_state_events_origin_authority_check CHECK(
        origin_authority IN('candidate_analysis','server_validator',
                            'hypothesis_revision_adjudication','investigation_compiler',
                            'dynamic_verification_resolution'));
ALTER TABLE attack_hypothesis_state_events
    DROP CONSTRAINT attack_hypothesis_state_events_authority_receipt_kind_check,
    ADD CONSTRAINT attack_hypothesis_state_events_authority_receipt_kind_check CHECK(
        authority_receipt_kind IN('candidate_gate_decision','server_validation',
                                  'revision_transition_decision',
                                  'investigation_compilation_decision','dynamic_resolution'));

-- Keep the established immutable-registry validator and add exactly one new
-- origin branch for terminal successors authored by a dynamic resolution.
CREATE OR REPLACE FUNCTION enforce_hypothesis_revision_creating_event()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    creating attack_hypothesis_state_events%ROWTYPE;
    event_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO event_count FROM attack_hypothesis_state_events
     WHERE successor_revision_id=NEW.revision_id;
    IF event_count<>1 THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_REQUIRED' USING ERRCODE='23514';
    END IF;
    SELECT * INTO creating FROM attack_hypothesis_state_events
     WHERE successor_revision_id=NEW.revision_id;
    IF ROW(creating.root_id,creating.operation_id,creating.organization_id,creating.successor_epistemic_state)
       IS DISTINCT FROM ROW(NEW.root_id,NEW.operation_id,NEW.organization_id,NEW.epistemic_state)
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_SCOPE_STATE_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF creating.predecessor_revision_id IS DISTINCT FROM NEW.predecessor_revision_id THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_PREDECESSOR_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF creating.server_decision_hash<>NEW.origin_decision_hash THEN
        RAISE EXCEPTION 'HYPOTHESIS_CREATING_EVENT_DECISION_HASH_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF (NEW.revision_ordinal=0) IS DISTINCT FROM (NEW.predecessor_revision_id IS NULL)
       OR (NEW.predecessor_revision_id IS NOT NULL AND NOT EXISTS(
           SELECT 1 FROM attack_hypothesis_revisions predecessor
            WHERE predecessor.revision_id=NEW.predecessor_revision_id
              AND predecessor.root_id=NEW.root_id
              AND predecessor.operation_id=NEW.operation_id
              AND predecessor.organization_id=NEW.organization_id
              AND predecessor.revision_ordinal=NEW.revision_ordinal-1
       ))
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_PREDECESSOR_INVALID' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='candidate_analysis' AND (
        NEW.epistemic_state NOT IN ('proposed','supported','contested','inconclusive')
        OR ROW(creating.event_kind,NEW.epistemic_state) NOT IN (
            ROW('created','proposed'),ROW('supported','supported'),
            ROW('contested','contested'),ROW('inconclusive','inconclusive')
        )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_TERMINAL_FORBIDDEN' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='candidate_analysis' AND (
        creating.authority_receipt_kind<>'candidate_gate_decision'
        OR NOT EXISTS(
            SELECT 1 FROM hypothesis_candidate_gate_decision_members mutation
             WHERE mutation.mutation_id=creating.server_decision_id
               AND mutation.operation_id=NEW.operation_id
               AND mutation.organization_id=NEW.organization_id
               AND mutation.root_id=NEW.root_id
               AND mutation.predecessor_revision_id IS NOT DISTINCT FROM NEW.predecessor_revision_id
               AND mutation.successor_revision_id=NEW.revision_id
               AND mutation.semantic_key_hash=NEW.semantic_key_hash
               AND mutation.successor_epistemic_state=NEW.epistemic_state
               AND mutation.origin_decision_hash=creating.server_decision_hash
               AND (mutation.route_kind IN ('reopen_historical','narrow_successor') OR EXISTS(
                   SELECT 1 FROM attack_hypotheses routed_root
                    WHERE routed_root.root_id=mutation.root_id
                      AND routed_root.root_kind=CASE mutation.route_kind
                          WHEN 'create_initial' THEN 'initial'
                          WHEN 'split' THEN 'split'
                          WHEN 'merge' THEN 'merge'
                          WHEN 'derive' THEN 'derive'
                      END
               ))
        )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_CANDIDATE_GATE_DECISION_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='server_validator' AND (
        NEW.epistemic_state<>'invalid'
        OR creating.authority_receipt_kind<>'server_validation'
        OR creating.authority_receipt_id IS NULL OR creating.authority_receipt_hash IS NULL
        OR creating.event_kind<>'invalidated'
        OR NOT EXISTS(
            SELECT 1 FROM hypothesis_server_validation_receipts receipt
             WHERE receipt.receipt_id=creating.authority_receipt_id
               AND receipt.operation_id=NEW.operation_id
               AND receipt.organization_id=NEW.organization_id
               AND receipt.root_id=NEW.root_id
               AND receipt.predecessor_revision_id IS NOT DISTINCT FROM NEW.predecessor_revision_id
               AND receipt.validated_revision_id=NEW.revision_id
               AND receipt.validated_revision_ingredients_hash=NEW.revision_ingredients_hash
               AND receipt.receipt_hash=creating.authority_receipt_hash
        )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_SERVER_VALIDATION_RECEIPT_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='hypothesis_revision_adjudication' AND (
        NEW.epistemic_state NOT IN ('verified','refuted') OR NEW.lifecycle_state<>'closed'
        OR creating.event_kind<>NEW.epistemic_state
        OR creating.authority_receipt_kind<>'revision_transition_decision'
        OR NOT EXISTS(
            SELECT 1
              FROM hypothesis_revision_terminal_decisions terminal
              JOIN hypothesis_revision_adjudications adjudication
                ON adjudication.revision_adjudication_id=terminal.revision_adjudication_id
              JOIN hypothesis_objective_outcome_set_seals outcome_set
                ON outcome_set.objective_outcome_set_seal_id=adjudication.objective_outcome_set_seal_id
             WHERE terminal.revision_terminal_decision_id=creating.authority_receipt_id
               AND terminal.decision_hash=creating.authority_receipt_hash
               AND terminal.state_event_id=creating.event_id
               AND terminal.terminal_successor_revision_id=NEW.revision_id
               AND terminal.hypothesis_revision_id=NEW.predecessor_revision_id
               AND terminal.operation_id=NEW.operation_id
               AND terminal.organization_id=NEW.organization_id
               AND terminal.decision=NEW.epistemic_state
               AND adjudication.outcome=terminal.decision
               AND adjudication.effective_valid_until>statement_timestamp()
               AND outcome_set.sealed_at IS NOT NULL
               AND NOT EXISTS(
                   SELECT 1
                     FROM hypothesis_objective_outcome_set_members outcome_member
                     JOIN verification_authority_quarantine_events quarantine
                       ON quarantine.objective_outcome_receipt_id=outcome_member.selected_current_outcome_id
                    WHERE outcome_member.objective_outcome_set_seal_id=outcome_set.objective_outcome_set_seal_id
               )
        )
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_ADJUDICATION_AUTHORITY_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF creating.origin_authority='dynamic_verification_resolution' AND (
        NEW.epistemic_state NOT IN ('verified','refuted','invalid')
        OR NEW.lifecycle_state<>'closed'
        OR creating.event_kind<>CASE WHEN NEW.epistemic_state='invalid'
                                     THEN 'invalidated' ELSE NEW.epistemic_state END
        OR creating.authority_receipt_kind<>'dynamic_resolution'
        OR NOT EXISTS(
            SELECT 1
              FROM investigation_dynamic_hypothesis_resolutions resolution
              JOIN investigation_dynamic_hypothesis_terminal_transitions transition
                ON transition.resolution_authority_id=resolution.resolution_authority_id
             WHERE resolution.resolution_authority_id=creating.authority_receipt_id
               AND resolution.resolution_sha256=creating.authority_receipt_hash
               AND resolution.asset_lane_id=transition.asset_lane_id
               AND resolution.hypothesis_revision_id=NEW.predecessor_revision_id
               AND resolution.disposition=NEW.epistemic_state
               AND transition.state_event_id=creating.event_id
               AND transition.source_revision_id=NEW.predecessor_revision_id
               AND transition.terminal_revision_id=NEW.revision_id
               AND transition.disposition=NEW.epistemic_state
        )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_REVISION_AUTHORITY_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM attack_hypothesis_heads head
         WHERE head.root_id=NEW.root_id AND head.operation_id=NEW.operation_id
           AND head.organization_id=NEW.organization_id AND head.head_revision_id=NEW.revision_id
           AND head.head_revision_hash=NEW.revision_hash
           AND head.head_semantic_key_hash=NEW.semantic_key_hash
           AND head.head_epistemic_state=NEW.epistemic_state
           AND head.head_lifecycle_state=NEW.lifecycle_state
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_HEAD_CAS_REQUIRED' USING ERRCODE='23514';
    END IF;
    IF (SELECT COUNT(*) FROM attack_hypothesis_verification_plans plan
         WHERE plan.revision_id=NEW.revision_id)<>1
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_REVISION_VERIFICATION_PLAN_EXACT_ONE_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

-- successor whose authority is the dynamic Primary resolution itself.
-- A dynamic resolution is an immutable terminal epistemic authority for the
-- exact current revision. Only that authority may move a current revision
-- from open/untested into one of the three accepted terminal states.
CREATE OR REPLACE FUNCTION investigation_guard_dynamic_resolution_terminal_transition()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_TRANSITION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM investigation_dynamic_hypothesis_resolutions resolution
         JOIN investigation_dynamic_verification_rounds dynamic_round
           ON dynamic_round.session_id=resolution.session_id
         JOIN attack_hypothesis_revisions source
           ON source.revision_id=NEW.source_revision_id
         JOIN attack_hypothesis_revisions terminal
           ON terminal.revision_id=NEW.terminal_revision_id
         JOIN attack_hypothesis_state_events event ON event.event_id=NEW.state_event_id
        WHERE resolution.resolution_authority_id=NEW.resolution_authority_id
          AND resolution.asset_lane_id=NEW.asset_lane_id
          AND resolution.hypothesis_revision_id=NEW.source_revision_id
          AND resolution.disposition=NEW.disposition
          AND dynamic_round.state='resolved'
          AND dynamic_round.resolution_authority_id=resolution.resolution_authority_id
          AND terminal.predecessor_revision_id=source.revision_id
          AND terminal.root_id=source.root_id
          AND terminal.operation_id=source.operation_id
          AND terminal.organization_id=source.organization_id
          AND ROW(terminal.semantic_key,terminal.semantic_key_hash,
                  terminal.subject_kind,terminal.subject_identity_hash,
                  terminal.target_live_id,terminal.target_type_at_time,
                  terminal.target_value_at_time,terminal.predicate_schema,
                  terminal.predicate_version,terminal.normalized_arguments,
                  terminal.trust_boundary,terminal.polarity,
                  terminal.structured_claim,terminal.assumptions,
                  terminal.missing_facts,terminal.priority,terminal.risk_impact,
                  terminal.asset_lane_id)
              IS NOT DISTINCT FROM
              ROW(source.semantic_key,source.semantic_key_hash,
                  source.subject_kind,source.subject_identity_hash,
                  source.target_live_id,source.target_type_at_time,
                  source.target_value_at_time,source.predicate_schema,
                  source.predicate_version,source.normalized_arguments,
                  source.trust_boundary,source.polarity,
                  source.structured_claim,source.assumptions,
                  source.missing_facts,source.priority,source.risk_impact,
                  source.asset_lane_id)
          AND terminal.revision_ordinal=source.revision_ordinal+1
          AND terminal.planning_readiness='deferred'
          AND terminal.epistemic_state=NEW.disposition
          AND terminal.lifecycle_state='closed'
          AND event.successor_revision_id=terminal.revision_id
          AND event.predecessor_revision_id=source.revision_id
          AND event.authority_receipt_id=resolution.resolution_authority_id
          AND event.authority_receipt_hash=resolution.resolution_sha256
          AND NEW.transition_sha256=tool_truth_sha256(jsonb_build_object(
              'domain','investigation_dynamic_hypothesis_terminal_transition.v2',
              'resolution_authority_id',resolution.resolution_authority_id,
              'asset_lane_id',resolution.asset_lane_id,
              'source_revision_id',source.revision_id,
              'terminal_revision_id',terminal.revision_id,
              'state_event_id',event.event_id,'disposition',resolution.disposition)::TEXT))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_TRANSITION_AUTHORITY_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_hypothesis_terminal_transitions_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_dynamic_hypothesis_terminal_transitions
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_resolution_terminal_transition();

CREATE FUNCTION investigation_guard_dynamic_terminal_transition_source()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_TRANSITION_SOURCE_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM investigation_dynamic_hypothesis_terminal_transitions transition
         WHERE transition.terminal_transition_id=NEW.terminal_transition_id
           AND transition.source_revision_id=NEW.source_revision_id
           AND transition.terminal_revision_id=NEW.terminal_revision_id)
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_TRANSITION_SOURCE_SCOPE_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_dynamic_hypothesis_terminal_transition_sources_guard
BEFORE INSERT OR UPDATE OR DELETE
ON investigation_dynamic_hypothesis_terminal_transition_sources
FOR EACH ROW EXECUTE FUNCTION investigation_guard_dynamic_terminal_transition_source();

CREATE FUNCTION investigation_validate_dynamic_terminal_transition_source_census()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_transition_id UUID := COALESCE(NEW.terminal_transition_id,
                                                  OLD.terminal_transition_id);
DECLARE source_revision UUID;
DECLARE terminal_revision UUID;
DECLARE expected_count BIGINT;
DECLARE expected_set TEXT;
DECLARE source_count BIGINT;
DECLARE source_set TEXT;
DECLARE kind TEXT;
BEGIN
    SELECT source_revision_id,terminal_revision_id
      INTO STRICT source_revision,terminal_revision
      FROM investigation_dynamic_hypothesis_terminal_transitions
     WHERE terminal_transition_id=requested_transition_id;
    FOREACH kind IN ARRAY ARRAY['revision_source','verification_objective','claim_component',
                                'verification_contract','verification_plan'] LOOP
        EXECUTE CASE kind
          WHEN 'revision_source' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(member_hash ORDER BY ordinal)::TEXT,''[]'')) FROM attack_hypothesis_revision_sources WHERE revision_id=$1'
          WHEN 'verification_objective' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(objective_hash ORDER BY objective_ordinal)::TEXT,''[]'')) FROM attack_hypothesis_verification_objectives WHERE revision_id=$1'
          WHEN 'claim_component' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(member_hash ORDER BY component_ordinal)::TEXT,''[]'')) FROM attack_hypothesis_claim_components WHERE revision_id=$1'
          WHEN 'verification_contract' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(contract_hash ORDER BY objective_id)::TEXT,''[]'')) FROM attack_hypothesis_verification_contracts WHERE revision_id=$1'
          ELSE
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(plan_hash ORDER BY plan_id)::TEXT,''[]'')) FROM attack_hypothesis_verification_plans WHERE revision_id=$1'
        END INTO expected_count,expected_set USING terminal_revision;
        EXECUTE CASE kind
          WHEN 'revision_source' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(member_hash ORDER BY ordinal)::TEXT,''[]'')) FROM attack_hypothesis_revision_sources WHERE revision_id=$1'
          WHEN 'verification_objective' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(objective_hash ORDER BY objective_ordinal)::TEXT,''[]'')) FROM attack_hypothesis_verification_objectives WHERE revision_id=$1'
          WHEN 'claim_component' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(member_hash ORDER BY component_ordinal)::TEXT,''[]'')) FROM attack_hypothesis_claim_components WHERE revision_id=$1'
          WHEN 'verification_contract' THEN
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(contract_hash ORDER BY objective_id)::TEXT,''[]'')) FROM attack_hypothesis_verification_contracts WHERE revision_id=$1'
          ELSE
            'SELECT COUNT(*),tool_truth_sha256(COALESCE(jsonb_agg(plan_hash ORDER BY plan_id)::TEXT,''[]'')) FROM attack_hypothesis_verification_plans WHERE revision_id=$1'
        END INTO source_count,source_set USING source_revision;
        IF ROW(source_count,source_set) IS DISTINCT FROM ROW(expected_count,expected_set) THEN
            RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_TRANSITION_CHANGED_HYPOTHESIS_AUTHORITY'
                USING ERRCODE='23514';
        END IF;
        IF NOT EXISTS(
            SELECT 1 FROM investigation_dynamic_hypothesis_terminal_transition_sources source
             WHERE source.terminal_transition_id=requested_transition_id
               AND source.source_kind=kind AND source.source_count=expected_count
               AND source.source_set_sha256=expected_set)
        THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_TRANSITION_SOURCE_CENSUS_DRIFT'
             USING ERRCODE='23514'; END IF;
    END LOOP;
    IF (SELECT COUNT(*) FROM investigation_dynamic_hypothesis_terminal_transition_sources source
         WHERE source.terminal_transition_id=requested_transition_id)<>5
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_TERMINAL_TRANSITION_SOURCE_CENSUS_DRIFT'
         USING ERRCODE='23514'; END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_terminal_transition_source_census
AFTER INSERT OR UPDATE OR DELETE
ON investigation_dynamic_hypothesis_terminal_transition_sources
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_terminal_transition_source_census();
CREATE CONSTRAINT TRIGGER investigation_dynamic_terminal_transition_parent_census
AFTER INSERT ON investigation_dynamic_hypothesis_terminal_transitions
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_terminal_transition_source_census();

-- Bidirectional terminal chain: neither a raw resolution nor a detached
-- terminal transition may commit.  The resolved round, resolution, transition,
-- state event and current terminal head are one exact authority chain.
CREATE FUNCTION investigation_validate_dynamic_resolution_terminal_chain()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_resolution_id UUID := COALESCE(NEW.resolution_authority_id,
                                                  OLD.resolution_authority_id);
BEGIN
    IF EXISTS(
        SELECT 1 FROM investigation_dynamic_hypothesis_resolutions resolution
        WHERE resolution.resolution_authority_id=requested_resolution_id
          AND NOT EXISTS(
            SELECT 1 FROM investigation_dynamic_verification_rounds dynamic_round
             JOIN investigation_dynamic_hypothesis_terminal_transitions transition
               ON transition.resolution_authority_id=resolution.resolution_authority_id
             JOIN attack_hypothesis_state_events event
               ON event.event_id=transition.state_event_id
             JOIN attack_hypothesis_revisions terminal
               ON terminal.revision_id=transition.terminal_revision_id
             JOIN attack_hypothesis_heads head
               ON head.root_id=terminal.root_id
              AND head.operation_id=terminal.operation_id
              AND head.organization_id=terminal.organization_id
            WHERE dynamic_round.session_id=resolution.session_id
              AND dynamic_round.state='resolved'
              AND dynamic_round.resolution_authority_id=resolution.resolution_authority_id
              AND transition.asset_lane_id=resolution.asset_lane_id
              AND transition.source_revision_id=resolution.hypothesis_revision_id
              AND transition.disposition=resolution.disposition
              AND event.predecessor_revision_id=transition.source_revision_id
              AND event.successor_revision_id=transition.terminal_revision_id
              AND event.authority_receipt_id=resolution.resolution_authority_id
              AND event.authority_receipt_hash=resolution.resolution_sha256
              AND terminal.lifecycle_state='closed'
              AND terminal.epistemic_state=resolution.disposition
              AND head.head_revision_id=terminal.revision_id
              AND head.head_revision_hash=terminal.revision_hash
              AND head.head_lifecycle_state='closed'
              AND head.head_epistemic_state=resolution.disposition))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_RESOLUTION_TERMINAL_CHAIN_REQUIRED'
         USING ERRCODE='23514'; END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_resolution_terminal_chain_parent
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_hypothesis_resolutions
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_resolution_terminal_chain();
CREATE CONSTRAINT TRIGGER investigation_dynamic_resolution_terminal_chain_transition
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_hypothesis_terminal_transitions
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_dynamic_resolution_terminal_chain();

CREATE FUNCTION investigation_guard_dynamic_verification_primary_completion()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_COMPLETION_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1 FROM investigation_dynamic_verification_rounds dynamic_round
         JOIN investigation_dynamic_hypothesis_resolutions resolution
           ON resolution.resolution_authority_id=dynamic_round.resolution_authority_id
         JOIN stage_work_items item ON item.id=dynamic_round.primary_work_item_id
         JOIN stage_worker_runs worker ON worker.id=dynamic_round.primary_worker_run_id
         JOIN stage_worker_outputs output ON output.id=NEW.stage_worker_output_id
          AND output.work_item_id=item.id AND output.worker_run_id=worker.id
         JOIN stage_team_plans plan ON plan.id=dynamic_round.stage_team_plan_id
        WHERE dynamic_round.session_id=NEW.session_id
          AND dynamic_round.state='resolved'
          AND resolution.resolution_authority_id=NEW.resolution_authority_id
          AND worker.id=NEW.primary_worker_run_id AND worker.status='passed'
          AND worker.active_tool_call_id IS NULL AND item.status='completed'
          AND plan.requests_closed_at IS NOT NULL
          AND plan.final_submitter_worker_run_id IS NULL
          AND output.output_schema='investigation_asset_verification_primary_resolution.v2'
          AND NEW.completion_sha256=tool_truth_sha256(jsonb_build_object(
              'domain','investigation_dynamic_verification_primary_completion.v1',
              'session_id',NEW.session_id,
              'resolution_authority_id',NEW.resolution_authority_id,
              'primary_worker_run_id',NEW.primary_worker_run_id,
              'primary_lease_token',NEW.primary_lease_token,
              'primary_attempt_epoch',NEW.primary_attempt_epoch,
              'expected_primary_checkpoint_version',NEW.expected_primary_checkpoint_version,
              'expected_work_item_row_version',NEW.expected_work_item_row_version,
              'expected_plan_row_version',NEW.expected_plan_row_version,
              'terminal_checkpoint_sha256',NEW.terminal_checkpoint_sha256,
              'stage_worker_output_id',NEW.stage_worker_output_id)::TEXT))
    THEN RAISE EXCEPTION 'INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_COMPLETION_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_dynamic_verification_primary_completions_guard
AFTER INSERT OR UPDATE OR DELETE ON investigation_dynamic_verification_primary_completions
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_guard_dynamic_verification_primary_completion();
