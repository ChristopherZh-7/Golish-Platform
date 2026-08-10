-- Company Identity and autonomous Target Intel asset authority.
--
-- This is an additive cutover.  Existing organizations/targets remain readable,
-- but an IntelGoalV1 operation can only publish Targets through the observation
-- lifecycle below.  Provider facts are immutable; attribution/reachability and
-- promotion are CAS transitions with append-only events.

CREATE TABLE scoping_company_identity_receipts (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    stage_execution_id UUID NOT NULL,
    resolution_attempt BIGINT NOT NULL CHECK (resolution_attempt >= 0),
    supersedes_receipt_id UUID REFERENCES scoping_company_identity_receipts(id) ON DELETE RESTRICT,
    organization_id UUID,
    subject_hint TEXT NOT NULL CHECK (btrim(subject_hint) <> ''),
    canonical_legal_name TEXT,
    aliases JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(aliases) = 'array'),
    brands JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(brands) = 'array'),
    registration_identifiers JSONB NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(registration_identifiers) = 'object'),
    disambiguation_fields JSONB NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(disambiguation_fields) = 'object'),
    confirmation_method TEXT NOT NULL CHECK (
        confirmation_method IN ('exact_reuse','provider_corroborated','human_selected','none')
    ),
    resolution_status TEXT NOT NULL CHECK (
        resolution_status IN ('confirmed','needs_human','unresolved')
    ),
    scope_policy JSONB NOT NULL CHECK (jsonb_typeof(scope_policy) = 'object'),
    source_receipt_refs JSONB NOT NULL CHECK (jsonb_typeof(source_receipt_refs) = 'array'),
    artifact_refs JSONB NOT NULL CHECK (jsonb_typeof(artifact_refs) = 'array'),
    evidence_refs JSONB NOT NULL CHECK (jsonb_typeof(evidence_refs) = 'array'),
    identity_payload JSONB NOT NULL CHECK (jsonb_typeof(identity_payload) = 'object'),
    identity_sha256 TEXT NOT NULL CHECK (identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    scope_policy_sha256 TEXT NOT NULL CHECK (scope_policy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, resolution_attempt),
    UNIQUE (id, operation_id, organization_id),
    CHECK (
        (resolution_status='confirmed'
         AND confirmation_method<>'none'
         AND organization_id IS NOT NULL
         AND canonical_legal_name IS NOT NULL
         AND btrim(canonical_legal_name)<>''
         AND jsonb_array_length(source_receipt_refs)>0
         AND jsonb_array_length(evidence_refs)>0)
        OR
        (resolution_status<>'confirmed' AND confirmation_method='none' AND organization_id IS NULL)
    )
);

CREATE UNIQUE INDEX scoping_one_confirmed_company_identity_per_operation
    ON scoping_company_identity_receipts(operation_id)
    WHERE resolution_status='confirmed';

CREATE FUNCTION reject_scoping_company_identity_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'SCOPING_COMPANY_IDENTITY_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER scoping_company_identity_immutable
BEFORE UPDATE OR DELETE ON scoping_company_identity_receipts
FOR EACH ROW EXECUTE FUNCTION reject_scoping_company_identity_mutation();

CREATE TABLE target_intel_goal_company_identity_bindings (
    operation_id UUID PRIMARY KEY REFERENCES target_intel_goal_operation_contracts(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL,
    company_identity_receipt_id UUID NOT NULL UNIQUE,
    company_identity_sha256 TEXT NOT NULL CHECK (company_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    scope_policy_sha256 TEXT NOT NULL CHECK (scope_policy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (company_identity_receipt_id, operation_id, organization_id)
        REFERENCES scoping_company_identity_receipts(id, operation_id, organization_id) ON DELETE RESTRICT
);

CREATE TRIGGER target_intel_company_identity_binding_immutable
BEFORE UPDATE OR DELETE ON target_intel_goal_company_identity_bindings
FOR EACH ROW EXECUTE FUNCTION reject_scoping_company_identity_mutation();

CREATE TABLE target_intel_goal_work_journal_entries (
    id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    goal_epoch_id UUID NOT NULL REFERENCES target_intel_goal_epochs(id) ON DELETE RESTRICT,
    goal_epoch BIGINT NOT NULL CHECK (goal_epoch >= 0),
    controller_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    controller_message_chain_id UUID NOT NULL,
    ordinal BIGINT NOT NULL CHECK (ordinal >= 0),
    entry_kind TEXT NOT NULL CHECK (entry_kind IN (
        'plan_snapshot','planned_pivot','attempted_action','tool_result','checked_empty',
        'failure','landed_data','attribution_result','reachability_result','plan_changed',
        'residual','review_finding_response','completion_checkpoint'
    )),
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    related_frontier_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(related_frontier_refs)='array'),
    evidence_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(evidence_refs)='array'),
    tool_call_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(tool_call_refs)='array'),
    observation_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(observation_refs)='array'),
    entry_sha256 TEXT NOT NULL CHECK (entry_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (team_plan_id, ordinal),
    FOREIGN KEY (goal_epoch_id, operation_id, organization_id, team_plan_id, goal_epoch)
        REFERENCES target_intel_goal_epochs(id, operation_id, organization_id, team_plan_id, epoch)
        ON DELETE RESTRICT,
    FOREIGN KEY (controller_message_chain_id, operation_id)
        REFERENCES message_chains(id, task_id) ON DELETE RESTRICT
);

CREATE FUNCTION reject_target_intel_append_only_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'TARGET_INTEL_APPEND_ONLY';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_goal_work_journal_append_only
BEFORE UPDATE OR DELETE ON target_intel_goal_work_journal_entries
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_append_only_mutation();

CREATE TABLE target_intel_asset_observations (
    id UUID PRIMARY KEY,
    stable_observation_key TEXT NOT NULL CHECK (btrim(stable_observation_key)<>''),
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    goal_epoch_id UUID NOT NULL REFERENCES target_intel_goal_epochs(id) ON DELETE RESTRICT,
    goal_epoch BIGINT NOT NULL CHECK (goal_epoch >= 0),
    producer_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    producer_tool_call_id UUID REFERENCES tool_calls(id) ON DELETE RESTRICT,
    semantic_receipt_audit_id BIGINT REFERENCES audit_log(id) ON DELETE RESTRICT,
    evidence_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    artifact_ref TEXT NOT NULL CHECK (artifact_ref ~ '^intel-artifact:sha256:[0-9a-f]{64}$'),
    artifact_sha256 TEXT NOT NULL CHECK (artifact_sha256 ~ '^[0-9a-f]{64}$'),
    provider_id TEXT NOT NULL CHECK (btrim(provider_id)<>''),
    provider_query_type TEXT NOT NULL CHECK (btrim(provider_query_type)<>''),
    adapter_version TEXT NOT NULL CHECK (btrim(adapter_version)<>''),
    stable_query_key TEXT NOT NULL CHECK (btrim(stable_query_key)<>''),
    provider_record_ordinal INTEGER NOT NULL CHECK (provider_record_ordinal >= 0),
    provider_fetched_at TIMESTAMPTZ NOT NULL,
    asset_kind TEXT NOT NULL CHECK (asset_kind IN (
        'domain','hostname','ip','cidr','web_origin','network_endpoint','certificate',
        'asn','icp','email_domain','github_org','repository','app_id'
    )),
    canonical_value TEXT NOT NULL CHECK (btrim(canonical_value)<>''),
    canonical_identity JSONB NOT NULL CHECK (jsonb_typeof(canonical_identity)='object'),
    canonical_identity_sha256 TEXT NOT NULL CHECK (canonical_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    typed_core JSONB NOT NULL CHECK (jsonb_typeof(typed_core)='object'),
    provider_fields JSONB NOT NULL CHECK (jsonb_typeof(provider_fields)='object'),
    provider_metadata JSONB NOT NULL CHECK (jsonb_typeof(provider_metadata)='object'),
    observation_sha256 TEXT NOT NULL CHECK (observation_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    attribution_disposition TEXT NOT NULL DEFAULT 'unassessed' CHECK (
        attribution_disposition IN ('unassessed','owned','shared','third_party','ambiguous','rejected')
    ),
    attribution_method TEXT,
    attribution_basis JSONB,
    attribution_decided_at TIMESTAMPTZ,
    reachability_state TEXT NOT NULL DEFAULT 'unverified' CHECK (
        reachability_state IN ('unverified','reachable','unreachable','failed','blocked')
    ),
    reachability_method TEXT,
    reachability_tool_call_id UUID REFERENCES tool_calls(id) ON DELETE RESTRICT,
    reachability_evidence_id BIGINT REFERENCES audit_log(id) ON DELETE RESTRICT,
    reachability_checked_at TIMESTAMPTZ,
    reachability_valid_until TIMESTAMPTZ,
    promotion_target_id UUID REFERENCES targets(id) ON DELETE RESTRICT,
    promoted_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    observed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, organization_id, stable_observation_key),
    UNIQUE (operation_id, organization_id, provider_id, stable_query_key, artifact_ref, provider_record_ordinal),
    UNIQUE (id, operation_id, organization_id),
    FOREIGN KEY (goal_epoch_id, operation_id, organization_id, team_plan_id, goal_epoch)
        REFERENCES target_intel_goal_epochs(id, operation_id, organization_id, team_plan_id, epoch)
        ON DELETE RESTRICT,
    CHECK (
        (attribution_disposition='unassessed' AND attribution_method IS NULL
         AND attribution_basis IS NULL AND attribution_decided_at IS NULL)
        OR
        (attribution_disposition<>'unassessed' AND attribution_method IS NOT NULL
         AND attribution_basis IS NOT NULL AND attribution_decided_at IS NOT NULL)
    ),
    CHECK (
        (reachability_state='unverified' AND reachability_method IS NULL
         AND reachability_tool_call_id IS NULL AND reachability_evidence_id IS NULL
         AND reachability_checked_at IS NULL AND reachability_valid_until IS NULL)
        OR
        (reachability_state='reachable'
         AND reachability_method IN ('bounded_http_probe_v1','bounded_tcp_protocol_probe_v1')
         AND reachability_tool_call_id IS NOT NULL AND reachability_evidence_id IS NOT NULL
         AND reachability_checked_at IS NOT NULL AND reachability_valid_until IS NOT NULL
         AND reachability_valid_until > reachability_checked_at)
        OR
        (reachability_state IN ('unreachable','failed','blocked')
         AND reachability_method IN ('bounded_http_probe_v1','bounded_tcp_protocol_probe_v1')
         AND reachability_evidence_id IS NOT NULL AND reachability_checked_at IS NOT NULL
         AND reachability_valid_until IS NULL)
    ),
    CHECK (
        (promotion_target_id IS NULL AND promoted_at IS NULL)
        OR
        (promotion_target_id IS NOT NULL AND promoted_at IS NOT NULL
         AND attribution_disposition='owned' AND reachability_state='reachable'
         AND reachability_method IN ('bounded_http_probe_v1','bounded_tcp_protocol_probe_v1')
         AND reachability_tool_call_id IS NOT NULL AND reachability_evidence_id IS NOT NULL
         AND reachability_checked_at IS NOT NULL
         AND reachability_valid_until IS NOT NULL AND reachability_valid_until > promoted_at)
    )
);

CREATE UNIQUE INDEX target_intel_one_promotion_per_canonical_identity
    ON target_intel_asset_observations(operation_id, organization_id, canonical_identity_sha256)
    WHERE promotion_target_id IS NOT NULL;

CREATE TABLE target_intel_asset_observation_events (
    id UUID PRIMARY KEY,
    observation_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('observed','attribution','reachability','promotion')),
    expected_row_version BIGINT NOT NULL CHECK (expected_row_version >= 0),
    before_state JSONB NOT NULL CHECK (jsonb_typeof(before_state)='object'),
    after_state JSONB NOT NULL CHECK (jsonb_typeof(after_state)='object'),
    evidence_refs JSONB NOT NULL CHECK (jsonb_typeof(evidence_refs)='array'),
    tool_call_refs JSONB NOT NULL CHECK (jsonb_typeof(tool_call_refs)='array'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (observation_id, operation_id, organization_id)
        REFERENCES target_intel_asset_observations(id, operation_id, organization_id) ON DELETE RESTRICT
);

CREATE TRIGGER target_intel_asset_observation_events_append_only
BEFORE UPDATE OR DELETE ON target_intel_asset_observation_events
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_append_only_mutation();

CREATE FUNCTION enforce_target_intel_observation_transition()
RETURNS trigger AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'TARGET_INTEL_OBSERVATION_IMMUTABLE';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF ROW(
            NEW.stable_observation_key,NEW.operation_id,NEW.organization_id,NEW.team_plan_id,
            NEW.goal_epoch_id,NEW.goal_epoch,NEW.producer_worker_run_id,NEW.producer_tool_call_id,
            NEW.semantic_receipt_audit_id,NEW.evidence_id,NEW.artifact_ref,NEW.artifact_sha256,
            NEW.provider_id,NEW.provider_query_type,NEW.adapter_version,NEW.stable_query_key,
            NEW.provider_record_ordinal,NEW.provider_fetched_at,NEW.asset_kind,NEW.canonical_value,
            NEW.canonical_identity,NEW.canonical_identity_sha256,NEW.typed_core,NEW.provider_fields,
            NEW.provider_metadata,NEW.observation_sha256,NEW.observed_at,NEW.created_at
        ) IS DISTINCT FROM ROW(
            OLD.stable_observation_key,OLD.operation_id,OLD.organization_id,OLD.team_plan_id,
            OLD.goal_epoch_id,OLD.goal_epoch,OLD.producer_worker_run_id,OLD.producer_tool_call_id,
            OLD.semantic_receipt_audit_id,OLD.evidence_id,OLD.artifact_ref,OLD.artifact_sha256,
            OLD.provider_id,OLD.provider_query_type,OLD.adapter_version,OLD.stable_query_key,
            OLD.provider_record_ordinal,OLD.provider_fetched_at,OLD.asset_kind,OLD.canonical_value,
            OLD.canonical_identity,OLD.canonical_identity_sha256,OLD.typed_core,OLD.provider_fields,
            OLD.provider_metadata,OLD.observation_sha256,OLD.observed_at,OLD.created_at
        ) THEN
            RAISE EXCEPTION 'TARGET_INTEL_OBSERVATION_PROVIDER_FACT_IMMUTABLE';
        END IF;
        IF NEW.row_version<>OLD.row_version+1 THEN
            RAISE EXCEPTION 'TARGET_INTEL_OBSERVATION_ROW_VERSION_INVALID';
        END IF;
        IF OLD.promotion_target_id IS NOT NULL AND ROW(NEW.promotion_target_id,NEW.promoted_at)
            IS DISTINCT FROM ROW(OLD.promotion_target_id,OLD.promoted_at) THEN
            RAISE EXCEPTION 'TARGET_INTEL_OBSERVATION_PROMOTION_IMMUTABLE';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_asset_observation_transition
BEFORE UPDATE OR DELETE ON target_intel_asset_observations
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_observation_transition();

CREATE FUNCTION bump_target_intel_material_for_insert()
RETURNS trigger AS $$
BEGIN
    UPDATE target_intel_goal_material_revisions
       SET state_revision=state_revision+1,row_version=row_version+1,updated_at=NOW()
     WHERE operation_id=NEW.operation_id AND organization_id=NEW.organization_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'TARGET_INTEL_MATERIAL_REVISION_MISSING'; END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_work_journal_revision
AFTER INSERT ON target_intel_goal_work_journal_entries
FOR EACH ROW EXECUTE FUNCTION bump_target_intel_material_for_insert();
CREATE TRIGGER target_intel_asset_observation_insert_revision
AFTER INSERT ON target_intel_asset_observations
FOR EACH ROW EXECUTE FUNCTION bump_target_intel_material_for_insert();
CREATE TRIGGER target_intel_asset_observation_update_revision
AFTER UPDATE ON target_intel_asset_observations
FOR EACH ROW EXECUTE FUNCTION bump_target_intel_material_for_insert();
