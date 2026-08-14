-- Durable strict company -> asset queues are mandatory authority for every
-- new Investigation run. Historical rows remain retained for audit only; a
-- runtime without this sealed queue may not fall back to organization-wide
-- scheduling.

-- Scope ordinals are sibling ordinals and restart at each depth. Replace the
-- original snapshot-global key with its exact depth-scoped authority key.
ALTER TABLE operation_org_scope_units
    DROP CONSTRAINT operation_org_scope_units_snapshot_id_ordinal_key;
ALTER TABLE operation_org_scope_units
    ADD CONSTRAINT operation_org_scope_units_depth_ordinal_unique
    UNIQUE(snapshot_id,depth,ordinal);

CREATE TABLE investigation_company_queues (
    company_queue_id UUID PRIMARY KEY,
    stable_freeze_request_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    owning_stage_run_request_id TEXT NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    member_count BIGINT NOT NULL CHECK(member_count>0),
    member_set_sha256 TEXT NOT NULL CHECK(member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    max_evolution_epochs INTEGER NOT NULL CHECK(max_evolution_epochs>=0),
    state TEXT NOT NULL DEFAULT 'open' CHECK(state IN('open','completed','blocked')),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK(head_version>=0),
    latest_event_id UUID,
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(authority_id,operation_id,stage_execution_id,scope_snapshot_id),
    UNIQUE(company_queue_id,operation_id,scope_snapshot_id),
    UNIQUE(company_queue_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id),
    FOREIGN KEY(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) REFERENCES investigation_run_heads(
        authority_id,operation_id,stage_execution_id,
        owning_stage_run_request_id,scope_snapshot_id
    ) ON DELETE RESTRICT
);

CREATE TABLE investigation_company_queue_members (
    company_member_id UUID PRIMARY KEY,
    company_queue_id UUID NOT NULL,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    organization_name_at_freeze TEXT NOT NULL,
    depth INTEGER NOT NULL CHECK(depth>=0),
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN('queued','active','completed','blocked')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    latest_event_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(company_queue_id,organization_id),
    UNIQUE(company_queue_id,depth,ordinal),
    UNIQUE(
        company_member_id,company_queue_id,operation_id,
        scope_snapshot_id,organization_id
    ),
    FOREIGN KEY(company_queue_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id)
        REFERENCES investigation_company_queues(
            company_queue_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,organization_id)
        REFERENCES operation_org_scope_units(snapshot_id,organization_id) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX investigation_company_queue_one_active
    ON investigation_company_queue_members(company_queue_id)
    WHERE state='active';

CREATE TABLE investigation_asset_queues (
    asset_queue_id UUID PRIMARY KEY,
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    member_count BIGINT NOT NULL CHECK(member_count>=0),
    member_set_sha256 TEXT NOT NULL CHECK(member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    state TEXT NOT NULL DEFAULT 'open' CHECK(state IN('open','completed','blocked')),
    head_version BIGINT NOT NULL DEFAULT 0 CHECK(head_version>=0),
    latest_event_id UUID,
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(asset_queue_id,company_queue_id,company_member_id,operation_id,scope_snapshot_id,organization_id),
    UNIQUE(asset_queue_id,operation_id,scope_snapshot_id,organization_id),
    FOREIGN KEY(company_member_id,company_queue_id,operation_id,scope_snapshot_id,organization_id)
        REFERENCES investigation_company_queue_members(
            company_member_id,company_queue_id,operation_id,scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE investigation_asset_lanes (
    asset_lane_id UUID PRIMARY KEY,
    asset_queue_id UUID NOT NULL,
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_id UUID NOT NULL,
    target_type_at_freeze TEXT NOT NULL,
    target_value_at_freeze TEXT NOT NULL CHECK(btrim(target_value_at_freeze)<>''),
    target_source_at_freeze TEXT NOT NULL,
    target_created_at TIMESTAMPTZ NOT NULL,
    target_identity_sha256 TEXT NOT NULL CHECK(target_identity_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN(
        'queued','analyzing','verifying','consolidating','evolving',
        'fixed_point','blocked','residual'
    )),
    evolution_epoch INTEGER NOT NULL DEFAULT 0 CHECK(evolution_epoch>=0),
    max_evolution_epochs INTEGER NOT NULL CHECK(max_evolution_epochs>=0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK(row_version>=0),
    latest_event_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(asset_queue_id,target_id),
    UNIQUE(asset_queue_id,ordinal),
    UNIQUE(
        asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ),
    FOREIGN KEY(
        asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) REFERENCES investigation_asset_queues(
        asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX investigation_asset_queue_one_active
    ON investigation_asset_lanes(asset_queue_id)
    WHERE state IN('analyzing','verifying','consolidating','evolving');

-- Sealed membership is complete at commit. These deferred guards allow the
-- repository to insert the header before its members in one transaction while
-- preventing any later raw INSERT from extending either frozen denominator.
CREATE FUNCTION investigation_validate_company_queue_member_count()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    queue_id UUID := COALESCE(NEW.company_queue_id,OLD.company_queue_id);
BEGIN
    IF EXISTS(
        SELECT 1 FROM investigation_company_queues queue
         WHERE queue.company_queue_id=queue_id
           AND queue.member_count<>(
               SELECT count(*) FROM investigation_company_queue_members member
                WHERE member.company_queue_id=queue_id
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPANY_QUEUE_MEMBER_COUNT_DRIFT' USING ERRCODE='23514';
    END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_company_queue_member_count_exact
AFTER INSERT OR UPDATE OR DELETE ON investigation_company_queue_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_validate_company_queue_member_count();

CREATE FUNCTION investigation_validate_asset_queue_member_count()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    queue_id UUID := COALESCE(NEW.asset_queue_id,OLD.asset_queue_id);
BEGIN
    IF EXISTS(
        SELECT 1 FROM investigation_asset_queues queue
         WHERE queue.asset_queue_id=queue_id
           AND queue.member_count<>(
               SELECT count(*) FROM investigation_asset_lanes lane
                WHERE lane.asset_queue_id=queue_id
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_MEMBER_COUNT_DRIFT' USING ERRCODE='23514';
    END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_asset_queue_member_count_exact
AFTER INSERT OR UPDATE OR DELETE ON investigation_asset_lanes
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_validate_asset_queue_member_count();

CREATE TABLE investigation_company_queue_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    event_ordinal BIGINT NOT NULL CHECK(event_ordinal>0),
    expected_queue_head_version BIGINT NOT NULL CHECK(expected_queue_head_version>=0),
    expected_member_row_version BIGINT NOT NULL CHECK(expected_member_row_version>=0),
    from_state TEXT NOT NULL CHECK(from_state IN('queued','active')),
    to_state TEXT NOT NULL CHECK(to_state IN('active','completed','blocked')),
    event_sha256 TEXT NOT NULL CHECK(event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(company_queue_id,event_ordinal),
    UNIQUE(event_id,company_queue_id,company_member_id),
    FOREIGN KEY(company_queue_id,operation_id,scope_snapshot_id)
        REFERENCES investigation_company_queues(company_queue_id,operation_id,scope_snapshot_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(company_member_id,company_queue_id,operation_id,scope_snapshot_id,organization_id)
        REFERENCES investigation_company_queue_members(
            company_member_id,company_queue_id,operation_id,scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TABLE investigation_asset_lane_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    asset_queue_id UUID NOT NULL,
    asset_lane_id UUID NOT NULL,
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    event_ordinal BIGINT NOT NULL CHECK(event_ordinal>0),
    expected_queue_head_version BIGINT NOT NULL CHECK(expected_queue_head_version>=0),
    expected_lane_row_version BIGINT NOT NULL CHECK(expected_lane_row_version>=0),
    from_state TEXT NOT NULL CHECK(from_state IN(
        'queued','analyzing','verifying','consolidating','evolving'
    )),
    to_state TEXT NOT NULL CHECK(to_state IN(
        'analyzing','verifying','consolidating','evolving',
        'fixed_point','blocked','residual'
    )),
    event_kind TEXT NOT NULL CHECK(event_kind IN(
        'claim','verification_started','consolidation_started','evolution_requested',
        'analysis_resumed','zero_hypothesis_fixed_point','fixed_point','blocked','residual'
    )),
    evolution_epoch INTEGER NOT NULL CHECK(evolution_epoch>=0),
    event_sha256 TEXT NOT NULL CHECK(event_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(asset_queue_id,event_ordinal),
    UNIQUE(event_id,asset_queue_id,asset_lane_id),
    FOREIGN KEY(
        asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) REFERENCES investigation_asset_lanes(
        asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT
);

CREATE TABLE investigation_asset_zero_hypothesis_fixed_point_receipts (
    fixed_point_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    asset_lane_id UUID NOT NULL UNIQUE,
    asset_queue_id UUID NOT NULL,
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    compilation_decision_id UUID NOT NULL UNIQUE,
    generation_id UUID NOT NULL UNIQUE,
    generation_seal_id UUID NOT NULL UNIQUE,
    canonical_apply_receipt_id UUID NOT NULL UNIQUE,
    backlog_set_sha256 TEXT NOT NULL CHECK(backlog_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    obligation_set_sha256 TEXT NOT NULL CHECK(obligation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    residual_set_sha256 TEXT NOT NULL CHECK(residual_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(fixed_point_receipt_id,asset_lane_id,operation_id,organization_id),
    FOREIGN KEY(
        asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) REFERENCES investigation_asset_lanes(
        asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(compilation_decision_id,operation_id,organization_id)
        REFERENCES investigation_hypothesis_compilation_decisions(
            decision_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(generation_seal_id,generation_id)
        REFERENCES hypothesis_generation_seals(seal_id,generation_id) ON DELETE RESTRICT,
    FOREIGN KEY(canonical_apply_receipt_id,operation_id,organization_id)
        REFERENCES investigation_hypothesis_canonical_apply_receipts(
            apply_receipt_id,operation_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_reject_queue_append_only_mutation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_APPEND_ONLY';
END;
$$;

-- Queue heads and lane heads are mutable only as the nested effect of one of
-- the immutable event inserts below.  Direct SQL cannot rewrite frozen
-- membership, ordering, target identity, fuel, or terminal state.
CREATE FUNCTION investigation_guard_queue_head_mutation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' OR pg_trigger_depth()<2 THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_EVENT_REQUIRED' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_company_queue_event_mutation_only
BEFORE UPDATE OR DELETE ON investigation_company_queues
FOR EACH ROW EXECUTE FUNCTION investigation_guard_queue_head_mutation();
CREATE TRIGGER investigation_company_member_event_mutation_only
BEFORE UPDATE OR DELETE ON investigation_company_queue_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_queue_head_mutation();
CREATE TRIGGER investigation_asset_queue_event_mutation_only
BEFORE UPDATE OR DELETE ON investigation_asset_queues
FOR EACH ROW EXECUTE FUNCTION investigation_guard_queue_head_mutation();
CREATE TRIGGER investigation_asset_lane_event_mutation_only
BEFORE UPDATE OR DELETE ON investigation_asset_lanes
FOR EACH ROW EXECUTE FUNCTION investigation_guard_queue_head_mutation();

CREATE TRIGGER investigation_company_queue_events_append_only
BEFORE UPDATE OR DELETE ON investigation_company_queue_events
FOR EACH ROW EXECUTE FUNCTION investigation_reject_queue_append_only_mutation();
CREATE TRIGGER investigation_asset_lane_events_append_only
BEFORE UPDATE OR DELETE ON investigation_asset_lane_events
FOR EACH ROW EXECUTE FUNCTION investigation_reject_queue_append_only_mutation();
CREATE TRIGGER investigation_asset_zero_fixed_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_asset_zero_hypothesis_fixed_point_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_queue_append_only_mutation();

CREATE FUNCTION investigation_apply_company_queue_event()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    queue_row investigation_company_queues%ROWTYPE;
    member_row investigation_company_queue_members%ROWTYPE;
BEGIN
    SELECT * INTO queue_row FROM investigation_company_queues
     WHERE company_queue_id=NEW.company_queue_id FOR UPDATE;
    SELECT * INTO member_row FROM investigation_company_queue_members
     WHERE company_member_id=NEW.company_member_id FOR UPDATE;
    IF NOT FOUND OR queue_row.operation_id<>NEW.operation_id
       OR queue_row.scope_snapshot_id<>NEW.scope_snapshot_id
       OR member_row.company_queue_id<>NEW.company_queue_id
       OR member_row.organization_id<>NEW.organization_id THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPANY_QUEUE_AUTHORITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF queue_row.head_version<>NEW.expected_queue_head_version
       OR member_row.row_version<>NEW.expected_member_row_version
       OR member_row.state<>NEW.from_state
       OR NEW.event_ordinal<>queue_row.head_version+1 THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPANY_QUEUE_CAS_CONFLICT' USING ERRCODE='40001';
    END IF;
    IF queue_row.state<>'open' THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPANY_QUEUE_CLOSED' USING ERRCODE='23514';
    END IF;
    IF NEW.from_state='queued' AND NEW.to_state='active' THEN
        IF EXISTS(SELECT 1 FROM investigation_company_queue_members
                   WHERE company_queue_id=NEW.company_queue_id AND state='active')
           OR NEW.company_member_id<>(
               SELECT company_member_id FROM investigation_company_queue_members
                WHERE company_queue_id=NEW.company_queue_id AND state='queued'
                ORDER BY depth,ordinal,organization_id LIMIT 1
           ) THEN
            RAISE EXCEPTION 'INVESTIGATION_COMPANY_QUEUE_ORDER_CONFLICT' USING ERRCODE='23514';
        END IF;
    ELSIF NEW.from_state='active' AND NEW.to_state IN('completed','blocked') THEN
        IF NEW.to_state='completed' AND EXISTS(
            SELECT 1 FROM investigation_asset_lanes lane
             WHERE lane.company_member_id=NEW.company_member_id
               AND lane.state NOT IN('fixed_point','blocked','residual')
        ) THEN
            RAISE EXCEPTION 'INVESTIGATION_COMPANY_QUEUE_ASSETS_OPEN' USING ERRCODE='23514';
        END IF;
    ELSE
        RAISE EXCEPTION 'INVESTIGATION_COMPANY_QUEUE_TRANSITION_INVALID' USING ERRCODE='23514';
    END IF;
    UPDATE investigation_company_queue_members
       SET state=NEW.to_state,row_version=row_version+1,
           latest_event_id=NEW.event_id,updated_at=statement_timestamp()
     WHERE company_member_id=NEW.company_member_id;
    UPDATE investigation_company_queues
       SET head_version=head_version+1,latest_event_id=NEW.event_id,
           state=CASE
               WHEN NEW.to_state='blocked' THEN 'blocked'
               WHEN NEW.to_state='completed' AND NOT EXISTS(
                   SELECT 1 FROM investigation_company_queue_members
                    WHERE company_queue_id=NEW.company_queue_id
                      AND company_member_id<>NEW.company_member_id
                      AND state NOT IN('completed','blocked')
               ) THEN 'completed'
               ELSE state END,
           updated_at=statement_timestamp()
     WHERE company_queue_id=NEW.company_queue_id;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_company_queue_event_apply
BEFORE INSERT ON investigation_company_queue_events
FOR EACH ROW EXECUTE FUNCTION investigation_apply_company_queue_event();

CREATE FUNCTION investigation_apply_asset_lane_event()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    queue_row investigation_asset_queues%ROWTYPE;
    lane_row investigation_asset_lanes%ROWTYPE;
    company_state TEXT;
    next_epoch INTEGER;
BEGIN
    SELECT * INTO queue_row FROM investigation_asset_queues
     WHERE asset_queue_id=NEW.asset_queue_id FOR UPDATE;
    SELECT * INTO lane_row FROM investigation_asset_lanes
     WHERE asset_lane_id=NEW.asset_lane_id FOR UPDATE;
    SELECT state INTO company_state FROM investigation_company_queue_members
     WHERE company_member_id=NEW.company_member_id FOR SHARE;
    IF NOT FOUND OR queue_row.operation_id<>NEW.operation_id
       OR queue_row.scope_snapshot_id<>NEW.scope_snapshot_id
       OR queue_row.company_member_id<>NEW.company_member_id
       OR lane_row.asset_queue_id<>NEW.asset_queue_id
       OR lane_row.company_queue_id<>NEW.company_queue_id
       OR lane_row.organization_id<>NEW.organization_id THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_AUTHORITY_MISMATCH' USING ERRCODE='23514';
    END IF;
    IF queue_row.head_version<>NEW.expected_queue_head_version
       OR lane_row.row_version<>NEW.expected_lane_row_version
       OR lane_row.state<>NEW.from_state
       OR NEW.event_ordinal<>queue_row.head_version+1 THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_CAS_CONFLICT' USING ERRCODE='40001';
    END IF;
    IF queue_row.state<>'open' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_CLOSED' USING ERRCODE='23514';
    END IF;
    next_epoch := lane_row.evolution_epoch;
    IF NEW.from_state='queued' AND NEW.to_state='analyzing' AND NEW.event_kind='claim' THEN
        IF company_state<>'active'
           OR EXISTS(SELECT 1 FROM investigation_asset_lanes
                      WHERE asset_queue_id=NEW.asset_queue_id
                        AND state IN('analyzing','verifying','consolidating','evolving'))
           OR NEW.asset_lane_id<>(
               SELECT asset_lane_id FROM investigation_asset_lanes
                WHERE asset_queue_id=NEW.asset_queue_id AND state='queued'
                ORDER BY target_created_at,target_value_at_freeze,target_id LIMIT 1
           ) THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_ORDER_CONFLICT' USING ERRCODE='23514';
        END IF;
    ELSIF NEW.from_state='analyzing' AND NEW.to_state='verifying'
          AND NEW.event_kind='verification_started' THEN NULL;
    ELSIF NEW.from_state='verifying' AND NEW.to_state='consolidating'
          AND NEW.event_kind='consolidation_started' THEN NULL;
    ELSIF NEW.from_state='consolidating' AND NEW.to_state='evolving'
          AND NEW.event_kind='evolution_requested' THEN
        IF lane_row.evolution_epoch>=lane_row.max_evolution_epochs THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_EVOLUTION_FUEL_EXHAUSTED' USING ERRCODE='23514';
        END IF;
        next_epoch := lane_row.evolution_epoch+1;
    ELSIF NEW.from_state='evolving' AND NEW.to_state='analyzing'
          AND NEW.event_kind='analysis_resumed' THEN NULL;
    ELSIF NEW.from_state='analyzing' AND NEW.to_state='fixed_point'
          AND NEW.event_kind='zero_hypothesis_fixed_point' THEN
        IF NOT EXISTS(
            SELECT 1 FROM investigation_asset_zero_hypothesis_fixed_point_receipts receipt
             WHERE receipt.asset_lane_id=NEW.asset_lane_id
               AND receipt.stable_request_id=NEW.stable_request_id
        ) THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_ZERO_FIXED_RECEIPT_REQUIRED' USING ERRCODE='23514';
        END IF;
    -- Ordinary fixed-point remains closed until the lane-scoped backlog and
    -- receipt authority from Task 2/5 is installed.  This migration only opens
    -- the independently guarded zero-hypothesis fixed-point seam above.
    ELSIF NEW.from_state='consolidating' AND NEW.to_state IN('blocked','residual')
          AND NEW.event_kind IN('blocked','residual') THEN NULL;
    ELSE
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_TRANSITION_INVALID' USING ERRCODE='23514';
    END IF;
    IF NEW.evolution_epoch<>next_epoch THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_EVOLUTION_EPOCH_DRIFT' USING ERRCODE='23514';
    END IF;
    UPDATE investigation_asset_lanes
       SET state=NEW.to_state,evolution_epoch=next_epoch,row_version=row_version+1,
           latest_event_id=NEW.event_id,updated_at=statement_timestamp()
     WHERE asset_lane_id=NEW.asset_lane_id;
    UPDATE investigation_asset_queues
       SET head_version=head_version+1,latest_event_id=NEW.event_id,
           state=CASE
               WHEN NEW.to_state IN('fixed_point','blocked','residual') AND NOT EXISTS(
                   SELECT 1 FROM investigation_asset_lanes
                    WHERE asset_queue_id=NEW.asset_queue_id
                      AND asset_lane_id<>NEW.asset_lane_id
                      AND state NOT IN('fixed_point','blocked','residual')
               ) THEN 'completed'
               ELSE state END,
           updated_at=statement_timestamp()
     WHERE asset_queue_id=NEW.asset_queue_id;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_asset_lane_event_apply
BEFORE INSERT ON investigation_asset_lane_events
FOR EACH ROW EXECUTE FUNCTION investigation_apply_asset_lane_event();

CREATE FUNCTION investigation_validate_zero_hypothesis_fixed_point()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS(
        SELECT 1
          FROM investigation_asset_lanes lane
          JOIN investigation_hypothesis_compilation_decisions decision
            ON decision.decision_id=NEW.compilation_decision_id
           AND decision.operation_id=lane.operation_id
           AND decision.organization_id=lane.organization_id
           AND decision.proposal_count=0
          JOIN hypothesis_generations generation
            ON generation.generation_id=NEW.generation_id
           AND generation.operation_id=lane.operation_id
           AND generation.organization_id=lane.organization_id
          JOIN hypothesis_generation_seals generation_seal
            ON generation_seal.seal_id=NEW.generation_seal_id
           AND generation_seal.generation_id=generation.generation_id
           AND generation_seal.member_count=0
          JOIN investigation_hypothesis_canonical_apply_receipts apply_receipt
            ON apply_receipt.apply_receipt_id=NEW.canonical_apply_receipt_id
           AND apply_receipt.decision_id=decision.decision_id
           AND apply_receipt.operation_id=lane.operation_id
           AND apply_receipt.organization_id=lane.organization_id
           AND apply_receipt.generation_id=generation.generation_id
           AND apply_receipt.generation_seal_id=generation_seal.seal_id
           AND apply_receipt.revision_count=0
         WHERE lane.asset_lane_id=NEW.asset_lane_id
           AND lane.asset_queue_id=NEW.asset_queue_id
           AND lane.company_queue_id=NEW.company_queue_id
           AND lane.company_member_id=NEW.company_member_id
           AND lane.operation_id=NEW.operation_id
           AND lane.scope_snapshot_id=NEW.scope_snapshot_id
           AND lane.organization_id=NEW.organization_id
           AND lane.state='analyzing'
           AND NEW.backlog_set_sha256=investigation_exact_member_set_hash(
               'golish.investigation.asset_backlog.v1',ARRAY[]::TEXT[])
           AND NEW.obligation_set_sha256=investigation_exact_member_set_hash(
               'golish.investigation.asset_obligations.v1',ARRAY[]::TEXT[])
           AND NEW.residual_set_sha256=investigation_exact_member_set_hash(
               'golish.investigation.asset_residuals.v1',ARRAY[]::TEXT[])
         FOR SHARE OF lane,decision,generation,generation_seal,apply_receipt
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_ZERO_FIXED_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_asset_zero_fixed_receipt_validate
BEFORE INSERT ON investigation_asset_zero_hypothesis_fixed_point_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_validate_zero_hypothesis_fixed_point();
