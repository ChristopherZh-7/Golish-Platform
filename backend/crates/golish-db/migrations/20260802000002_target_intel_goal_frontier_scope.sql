-- Target Intel Goal V2 frontier. This migration intentionally does not alter
-- expansion_queue; the legacy queue remains a best-effort diagnostic mirror.

CREATE TABLE target_intel_goal_frontier_v2 (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES target_intel_goal_operation_contracts(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    goal_epoch_id UUID NOT NULL REFERENCES target_intel_goal_epochs(id) ON DELETE RESTRICT,
    semantic_pivot_key TEXT NOT NULL CHECK (btrim(semantic_pivot_key) <> ''),
    pivot_kind TEXT NOT NULL CHECK (
        pivot_kind IN ('company_name','brand','domain','hostname','ip','cidr','asn','certificate','icp','email_domain','github_org','repository','app_id')
    ),
    pivot_value_sha256 TEXT NOT NULL CHECK (pivot_value_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    intent TEXT NOT NULL CHECK (intent IN ('discover_related_assets','verify_attribution','enrich_known_asset')),
    status TEXT NOT NULL CHECK (
        status IN ('pending','in_progress','resolved','blocked','unsupported','needs_human','rejected_noise','third_party','ambiguous')
    ),
    provenance JSONB NOT NULL CHECK (jsonb_typeof(provenance) = 'object'),
    terminal_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(terminal_refs) = 'array'),
    capability_ref TEXT,
    reason TEXT,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (operation_id, organization_id, semantic_pivot_key),
    CHECK (
        (status IN ('resolved','blocked','unsupported','needs_human','rejected_noise','third_party','ambiguous') AND terminal_at IS NOT NULL)
        OR (status IN ('pending','in_progress') AND terminal_at IS NULL)
    )
);

CREATE TABLE target_intel_goal_frontier_events (
    id UUID PRIMARY KEY,
    frontier_id UUID NOT NULL REFERENCES target_intel_goal_frontier_v2(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    from_status TEXT,
    to_status TEXT NOT NULL,
    expected_row_version BIGINT NOT NULL CHECK (expected_row_version >= 0),
    evidence_refs JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(evidence_refs) = 'array'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (frontier_id, expected_row_version)
);

CREATE FUNCTION enforce_target_intel_frontier_identity_immutable()
RETURNS trigger AS $$
BEGIN
    IF ROW(
        NEW.operation_id, NEW.organization_id, NEW.stage_execution_id,
        NEW.goal_epoch_id, NEW.semantic_pivot_key, NEW.pivot_kind,
        NEW.pivot_value_sha256, NEW.intent, NEW.provenance
    ) IS DISTINCT FROM ROW(
        OLD.operation_id, OLD.organization_id, OLD.stage_execution_id,
        OLD.goal_epoch_id, OLD.semantic_pivot_key, OLD.pivot_kind,
        OLD.pivot_value_sha256, OLD.intent, OLD.provenance
    ) THEN
        RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_IDENTITY_IMMUTABLE';
    END IF;
    IF NEW.row_version <> OLD.row_version + 1 THEN
        RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_ROW_VERSION_CAS_REQUIRED';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_frontier_identity_immutable
BEFORE UPDATE ON target_intel_goal_frontier_v2
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_frontier_identity_immutable();

CREATE INDEX target_intel_goal_frontier_open_idx
    ON target_intel_goal_frontier_v2(operation_id, organization_id, status, updated_at)
    WHERE status IN ('pending', 'in_progress');

ALTER TABLE target_intel_goal_frontier_v2
    ADD COLUMN stage_run_unit_id UUID NOT NULL,
    ADD COLUMN scope_snapshot_id UUID NOT NULL,
    ADD COLUMN team_plan_id UUID NOT NULL,
    ADD COLUMN goal_epoch BIGINT NOT NULL CHECK (goal_epoch >= 0),
    ADD COLUMN materiality TEXT NOT NULL CHECK (materiality IN ('material','supporting')),
    ADD COLUMN claimed_by_worker_run_id UUID,
    ADD COLUMN claim_attempt_epoch BIGINT CHECK (claim_attempt_epoch IS NULL OR claim_attempt_epoch >= 0),
    ADD COLUMN claim_lease_token UUID,
    ADD COLUMN claim_lease_expires_at TIMESTAMPTZ,
    ADD CONSTRAINT target_intel_goal_frontier_owner_unique UNIQUE (
        id, operation_id, organization_id
    ),
    ADD CONSTRAINT target_intel_goal_frontier_plan_owner_fk FOREIGN KEY (
        team_plan_id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) REFERENCES stage_team_plans (
        id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_frontier_epoch_owner_fk FOREIGN KEY (
        goal_epoch_id, operation_id, organization_id, team_plan_id, goal_epoch
    ) REFERENCES target_intel_goal_epochs (
        id, operation_id, organization_id, team_plan_id, epoch
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_frontier_claim_owner_fk FOREIGN KEY (
        claimed_by_worker_run_id, operation_id, stage_execution_id,
        stage_run_unit_id, organization_id
    ) REFERENCES stage_worker_runs (
        id, operation_id, stage_execution_id, stage_run_unit_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_frontier_claim_tuple_ck CHECK (
        (
            claimed_by_worker_run_id IS NULL
            AND claim_attempt_epoch IS NULL
            AND claim_lease_token IS NULL
            AND claim_lease_expires_at IS NULL
        ) OR (
            claimed_by_worker_run_id IS NOT NULL
            AND claim_attempt_epoch IS NOT NULL
            AND claim_lease_token IS NOT NULL
            AND claim_lease_expires_at IS NOT NULL
        )
    );

ALTER TABLE target_intel_goal_frontier_events
    ADD COLUMN claimed_by_worker_run_id UUID,
    ADD COLUMN claim_attempt_epoch BIGINT,
    ADD COLUMN claim_lease_token UUID,
    ADD COLUMN capability_ref TEXT,
    ADD COLUMN reason TEXT,
    ADD CONSTRAINT target_intel_goal_frontier_events_owner_fk FOREIGN KEY (
        frontier_id, operation_id, organization_id
    ) REFERENCES target_intel_goal_frontier_v2(id, operation_id, organization_id)
      ON DELETE RESTRICT;

CREATE TABLE target_intel_goal_frontier_waivers (
    id UUID PRIMARY KEY,
    frontier_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    expected_frontier_row_version BIGINT NOT NULL CHECK (expected_frontier_row_version >= 0),
    authority_kind TEXT NOT NULL CHECK (authority_kind IN ('operation_policy','human_operator')),
    authority_ref TEXT NOT NULL CHECK (btrim(authority_ref) <> ''),
    evidence_refs JSONB NOT NULL CHECK (jsonb_typeof(evidence_refs) = 'array'),
    reason TEXT NOT NULL CHECK (btrim(reason) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (frontier_id, operation_id, organization_id)
        REFERENCES target_intel_goal_frontier_v2(id, operation_id, organization_id)
        ON DELETE RESTRICT
);

DROP TRIGGER target_intel_frontier_identity_immutable ON target_intel_goal_frontier_v2;
DROP FUNCTION enforce_target_intel_frontier_identity_immutable();

CREATE FUNCTION enforce_target_intel_frontier_contract()
RETURNS trigger AS $$
DECLARE
    worker stage_worker_runs%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'pending' OR NEW.row_version<>0 OR NEW.terminal_at IS NOT NULL
            OR NEW.terminal_refs<>'[]'::jsonb OR NEW.capability_ref IS NOT NULL
            OR NEW.reason IS NOT NULL OR NEW.claimed_by_worker_run_id IS NOT NULL
            OR NOT EXISTS (
                SELECT 1 FROM target_intel_goal_epochs epoch
                 WHERE epoch.id=NEW.goal_epoch_id
                   AND epoch.operation_id=NEW.operation_id
                   AND epoch.organization_id=NEW.organization_id
                   AND epoch.team_plan_id=NEW.team_plan_id
                   AND epoch.epoch=NEW.goal_epoch
                   AND epoch.status='open'
            )
        THEN
            RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_INSERT_INVALID';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(
        NEW.id,NEW.operation_id,NEW.organization_id,NEW.stage_execution_id,
        NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.team_plan_id,
        NEW.goal_epoch_id,NEW.goal_epoch,NEW.semantic_pivot_key,NEW.pivot_kind,
        NEW.pivot_value_sha256,NEW.intent,NEW.materiality,NEW.provenance,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.operation_id,OLD.organization_id,OLD.stage_execution_id,
        OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.team_plan_id,
        OLD.goal_epoch_id,OLD.goal_epoch,OLD.semantic_pivot_key,OLD.pivot_kind,
        OLD.pivot_value_sha256,OLD.intent,OLD.materiality,OLD.provenance,OLD.created_at
    ) OR NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at
    THEN
        RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_IDENTITY_OR_CAS_INVALID';
    END IF;
    IF OLD.status='pending' AND NEW.status='in_progress' THEN
        IF NOT EXISTS (
            SELECT 1
              FROM stage_worker_runs persisted
              JOIN stage_work_items item ON item.id=persisted.work_item_id
             WHERE persisted.id=NEW.claimed_by_worker_run_id
               AND persisted.operation_id=NEW.operation_id
               AND persisted.stage_execution_id=NEW.stage_execution_id
               AND persisted.stage_run_unit_id=NEW.stage_run_unit_id
               AND persisted.organization_id=NEW.organization_id
               AND persisted.attempt_epoch=NEW.claim_attempt_epoch
               AND persisted.status IN ('running','waiting_background','gate_blocked')
               AND item.team_plan_id=NEW.team_plan_id
               AND item.operation_id=NEW.operation_id
               AND item.stage_execution_id=NEW.stage_execution_id
               AND item.stage_run_unit_id=NEW.stage_run_unit_id
               AND item.scope_snapshot_id=NEW.scope_snapshot_id
               AND item.organization_id=NEW.organization_id
               AND item.dispatch_epoch=NEW.goal_epoch
               AND item.execution_profile='worker'
               AND item.status IN ('claimed','running','waiting_dependency')
        ) OR NEW.claim_lease_token IS NULL
            OR NEW.claim_lease_expires_at<=NOW()
            OR NEW.terminal_at IS NOT NULL OR NEW.terminal_refs<>'[]'::jsonb
            OR NEW.capability_ref IS NOT NULL OR NEW.reason IS NOT NULL
        THEN
            RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_CLAIM_AUTHORITY_INVALID';
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.status='in_progress'
        AND NEW.status IN ('resolved','blocked','unsupported','needs_human','rejected_noise','third_party','ambiguous')
    THEN
        IF NEW.claimed_by_worker_run_id IS DISTINCT FROM OLD.claimed_by_worker_run_id
            OR NEW.claim_attempt_epoch IS DISTINCT FROM OLD.claim_attempt_epoch
            OR NEW.claim_lease_token IS DISTINCT FROM OLD.claim_lease_token
            OR NEW.claim_lease_expires_at IS DISTINCT FROM OLD.claim_lease_expires_at
            OR OLD.claim_lease_expires_at<=NOW()
            OR NEW.terminal_at IS NULL
            OR (NEW.status='resolved' AND jsonb_array_length(NEW.terminal_refs)=0)
            OR (NEW.status IN ('blocked','unsupported')
                AND (NEW.capability_ref IS NULL OR btrim(NEW.capability_ref)=''
                     OR NEW.reason IS NULL OR btrim(NEW.reason)=''))
            OR (NEW.status IN ('needs_human','rejected_noise','third_party','ambiguous')
                AND (NEW.reason IS NULL OR btrim(NEW.reason)=''))
        THEN
            RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_TERMINAL_AUTHORITY_INVALID';
        END IF;
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_INVALID_TRANSITION';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_frontier_contract
BEFORE INSERT OR UPDATE OR DELETE ON target_intel_goal_frontier_v2
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_frontier_contract();

CREATE FUNCTION append_target_intel_frontier_event()
RETURNS trigger AS $$
BEGIN
    INSERT INTO target_intel_goal_frontier_events(
        id,frontier_id,operation_id,organization_id,from_status,to_status,
        expected_row_version,evidence_refs,claimed_by_worker_run_id,
        claim_attempt_epoch,claim_lease_token,capability_ref,reason
    ) VALUES(
        gen_random_uuid(),NEW.id,NEW.operation_id,NEW.organization_id,
        OLD.status,NEW.status,OLD.row_version,NEW.terminal_refs,
        NEW.claimed_by_worker_run_id,NEW.claim_attempt_epoch,
        NEW.claim_lease_token,NEW.capability_ref,NEW.reason
    );
    UPDATE target_intel_goal_material_revisions
       SET state_revision=state_revision+1,row_version=row_version+1,updated_at=NOW()
     WHERE operation_id=NEW.operation_id AND organization_id=NEW.organization_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'TARGET_INTEL_MATERIAL_REVISION_MISSING';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_frontier_event_append
AFTER UPDATE ON target_intel_goal_frontier_v2
FOR EACH ROW EXECUTE FUNCTION append_target_intel_frontier_event();

CREATE FUNCTION bump_target_intel_frontier_insert_revision()
RETURNS trigger AS $$
BEGIN
    UPDATE target_intel_goal_material_revisions
       SET state_revision=state_revision+1,row_version=row_version+1,updated_at=NOW()
     WHERE operation_id=NEW.operation_id AND organization_id=NEW.organization_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'TARGET_INTEL_MATERIAL_REVISION_MISSING';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_frontier_insert_revision
AFTER INSERT ON target_intel_goal_frontier_v2
FOR EACH ROW EXECUTE FUNCTION bump_target_intel_frontier_insert_revision();

CREATE FUNCTION reject_target_intel_frontier_append_only_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_APPEND_ONLY';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_frontier_events_append_only
BEFORE UPDATE OR DELETE ON target_intel_goal_frontier_events
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_frontier_append_only_mutation();
CREATE TRIGGER target_intel_frontier_waivers_append_only
BEFORE UPDATE OR DELETE ON target_intel_goal_frontier_waivers
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_frontier_append_only_mutation();

CREATE FUNCTION enforce_target_intel_frontier_waiver_insert()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM target_intel_goal_frontier_v2 frontier
         WHERE frontier.id=NEW.frontier_id
           AND frontier.operation_id=NEW.operation_id
           AND frontier.organization_id=NEW.organization_id
           AND frontier.row_version=NEW.expected_frontier_row_version
           AND frontier.materiality='material'
           AND frontier.status IN ('blocked','unsupported')
    ) OR jsonb_array_length(NEW.evidence_refs)=0
      OR EXISTS (
          SELECT 1 FROM jsonb_array_elements_text(NEW.evidence_refs) ref(value)
           WHERE ref.value !~ '^audit:[0-9]+$'
              OR NOT EXISTS (
                  SELECT 1 FROM audit_log evidence
                   WHERE evidence.id=CASE
                       WHEN ref.value ~ '^audit:[0-9]+$'
                       THEN substring(ref.value FROM 7)::bigint
                       ELSE NULL
                   END
                     AND evidence.audit_role='evidence'
                     AND evidence.run_id=NEW.operation_id
                     AND evidence.detail ->> 'organization_id'=NEW.organization_id::text
              )
      )
      OR (
          NEW.authority_kind='operation_policy'
          AND NOT EXISTS (
              SELECT 1 FROM target_intel_goal_operation_contracts contract
               WHERE contract.operation_id=NEW.operation_id
                 AND NEW.authority_ref=contract.goal_contract_sha256
          )
      )
    THEN
        RAISE EXCEPTION 'TARGET_INTEL_FRONTIER_WAIVER_AUTHORITY_INVALID';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_frontier_waiver_insert
BEFORE INSERT ON target_intel_goal_frontier_waivers
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_frontier_waiver_insert();

CREATE FUNCTION bump_target_intel_frontier_waiver_revision()
RETURNS trigger AS $$
BEGIN
    UPDATE target_intel_goal_material_revisions
       SET state_revision=state_revision+1,row_version=row_version+1,updated_at=NOW()
     WHERE operation_id=NEW.operation_id AND organization_id=NEW.organization_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'TARGET_INTEL_MATERIAL_REVISION_MISSING';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_frontier_waiver_revision
AFTER INSERT ON target_intel_goal_frontier_waivers
FOR EACH ROW EXECUTE FUNCTION bump_target_intel_frontier_waiver_revision();
