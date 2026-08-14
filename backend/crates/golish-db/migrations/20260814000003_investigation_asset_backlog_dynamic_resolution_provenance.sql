-- Ordinary asset fixed point is authorized only by the current dynamic-v2
-- Primary resolution chain for every hypothesis root. Historical closed heads
-- authored by adjudication/server validation remain immutable audit records,
-- but cannot satisfy a new mandatory asset-queue close.

ALTER TABLE investigation_asset_backlog_fixed_point_receipts
    ADD COLUMN dynamic_resolution_member_count BIGINT,
    ADD COLUMN dynamic_resolution_member_set_sha256 TEXT,
    ADD CONSTRAINT investigation_asset_fixed_dynamic_resolution_shape CHECK(
        (dynamic_resolution_member_count IS NULL
          AND dynamic_resolution_member_set_sha256 IS NULL)
        OR
        (dynamic_resolution_member_count>0
          AND dynamic_resolution_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'));

CREATE TABLE investigation_asset_backlog_dynamic_resolution_members (
    member_id UUID PRIMARY KEY,
    fixed_point_receipt_id UUID NOT NULL REFERENCES
        investigation_asset_backlog_fixed_point_receipts(fixed_point_receipt_id)
        ON DELETE RESTRICT,
    asset_lane_id UUID NOT NULL REFERENCES investigation_asset_lanes(asset_lane_id)
        ON DELETE RESTRICT,
    hypothesis_root_id UUID NOT NULL REFERENCES attack_hypotheses(root_id)
        ON DELETE RESTRICT,
    source_revision_id UUID NOT NULL REFERENCES attack_hypothesis_revisions(revision_id)
        ON DELETE RESTRICT,
    terminal_revision_id UUID NOT NULL REFERENCES attack_hypothesis_revisions(revision_id)
        ON DELETE RESTRICT,
    dynamic_session_id UUID NOT NULL REFERENCES investigation_dynamic_verification_rounds(session_id)
        ON DELETE RESTRICT,
    resolution_authority_id UUID NOT NULL REFERENCES
        investigation_dynamic_hypothesis_resolutions(resolution_authority_id) ON DELETE RESTRICT,
    terminal_transition_id UUID NOT NULL REFERENCES
        investigation_dynamic_hypothesis_terminal_transitions(terminal_transition_id)
        ON DELETE RESTRICT,
    state_event_id UUID NOT NULL REFERENCES attack_hypothesis_state_events(event_id)
        ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK(disposition IN('verified','refuted','invalid')),
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(fixed_point_receipt_id,hypothesis_root_id),
    UNIQUE(fixed_point_receipt_id,terminal_revision_id),
    UNIQUE(fixed_point_receipt_id,resolution_authority_id),
    UNIQUE(fixed_point_receipt_id,terminal_transition_id)
);

CREATE FUNCTION investigation_guard_asset_backlog_dynamic_resolution_member()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_DYNAMIC_RESOLUTION_MEMBER_APPEND_ONLY'
            USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(
        SELECT 1
          FROM investigation_asset_backlog_fixed_point_receipts receipt
          JOIN attack_hypotheses root
            ON root.root_id=NEW.hypothesis_root_id
           AND root.asset_lane_id=receipt.asset_lane_id
          JOIN attack_hypothesis_heads head
            ON head.root_id=root.root_id
           AND head.operation_id=root.operation_id
           AND head.organization_id=root.organization_id
          JOIN attack_hypothesis_revisions terminal
            ON terminal.revision_id=head.head_revision_id
           AND terminal.root_id=root.root_id
           AND terminal.operation_id=root.operation_id
           AND terminal.organization_id=root.organization_id
          JOIN attack_hypothesis_revisions source_revision
            ON source_revision.revision_id=NEW.source_revision_id
           AND source_revision.root_id=root.root_id
           AND source_revision.operation_id=root.operation_id
           AND source_revision.organization_id=root.organization_id
          JOIN investigation_dynamic_hypothesis_terminal_transitions transition
            ON transition.terminal_transition_id=NEW.terminal_transition_id
           AND transition.terminal_revision_id=terminal.revision_id
           AND transition.source_revision_id=NEW.source_revision_id
           AND transition.state_event_id=NEW.state_event_id
          JOIN investigation_dynamic_hypothesis_resolutions resolution
            ON resolution.resolution_authority_id=NEW.resolution_authority_id
           AND resolution.resolution_authority_id=transition.resolution_authority_id
           AND resolution.hypothesis_revision_id=transition.source_revision_id
           AND resolution.asset_lane_id=receipt.asset_lane_id
           AND resolution.disposition=transition.disposition
          JOIN investigation_dynamic_verification_rounds dynamic_round
            ON dynamic_round.session_id=NEW.dynamic_session_id
           AND dynamic_round.session_id=resolution.session_id
           AND dynamic_round.operation_id=root.operation_id
           AND dynamic_round.organization_id=root.organization_id
           AND dynamic_round.asset_lane_id=receipt.asset_lane_id
           AND dynamic_round.hypothesis_revision_id=source_revision.revision_id
           AND dynamic_round.state='resolved'
           AND dynamic_round.resolution_authority_id=resolution.resolution_authority_id
          JOIN attack_hypothesis_state_events event
            ON event.event_id=transition.state_event_id
           AND event.predecessor_revision_id=transition.source_revision_id
           AND event.successor_revision_id=transition.terminal_revision_id
           AND event.origin_authority='dynamic_verification_resolution'
           AND event.authority_receipt_kind='dynamic_resolution'
           AND event.authority_receipt_id=resolution.resolution_authority_id
           AND event.authority_receipt_hash=resolution.resolution_sha256
         WHERE receipt.fixed_point_receipt_id=NEW.fixed_point_receipt_id
           AND receipt.asset_lane_id=NEW.asset_lane_id
           AND head.head_revision_id=NEW.terminal_revision_id
           AND head.head_lifecycle_state='closed'
           AND head.head_epistemic_state=NEW.disposition
           AND terminal.lifecycle_state='closed'
           AND terminal.epistemic_state=NEW.disposition
           AND transition.asset_lane_id=NEW.asset_lane_id
           AND transition.disposition=NEW.disposition
           AND NEW.member_sha256=tool_truth_sha256(format(
                'golish.investigation.asset_backlog.dynamic_resolution_member.v1:%s:%s:%s:%s:%s:%s:%s:%s:%s:%s:%s',
                NEW.fixed_point_receipt_id,NEW.asset_lane_id,NEW.hypothesis_root_id,
                NEW.source_revision_id,NEW.terminal_revision_id,NEW.dynamic_session_id,
                NEW.resolution_authority_id,NEW.terminal_transition_id,NEW.state_event_id,
                NEW.disposition,receipt.operation_id)))
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_DYNAMIC_RESOLUTION_MEMBER_MISMATCH'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_backlog_dynamic_resolution_members_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_asset_backlog_dynamic_resolution_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_backlog_dynamic_resolution_member();

CREATE FUNCTION investigation_validate_asset_backlog_dynamic_resolution_census()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE requested_receipt_id UUID := COALESCE(NEW.fixed_point_receipt_id,
                                               OLD.fixed_point_receipt_id);
DECLARE actual_count BIGINT;
DECLARE actual_set TEXT;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM investigation_asset_backlog_fixed_point_receipts receipt
                   WHERE receipt.fixed_point_receipt_id=requested_receipt_id)
    THEN RETURN COALESCE(NEW,OLD); END IF;
    SELECT COUNT(*),investigation_exact_member_set_hash(
             'golish.investigation.asset_backlog.dynamic_resolution_members.v1',
             COALESCE(array_agg(member.member_sha256 ORDER BY member.hypothesis_root_id),
                      ARRAY[]::TEXT[]))
      INTO actual_count,actual_set
      FROM investigation_asset_backlog_dynamic_resolution_members member
     WHERE member.fixed_point_receipt_id=requested_receipt_id;
    IF EXISTS(
        SELECT 1 FROM investigation_asset_backlog_fixed_point_receipts receipt
         WHERE receipt.fixed_point_receipt_id=requested_receipt_id
           AND (receipt.dynamic_resolution_member_count IS NULL
             OR receipt.dynamic_resolution_member_set_sha256 IS NULL
             OR ROW(receipt.dynamic_resolution_member_count,
                    receipt.dynamic_resolution_member_set_sha256,
                    receipt.hypothesis_root_count)
                IS DISTINCT FROM ROW(actual_count,actual_set,actual_count)
             OR EXISTS(
                 SELECT 1 FROM attack_hypotheses root
                  WHERE root.asset_lane_id=receipt.asset_lane_id
                    AND NOT EXISTS(
                        SELECT 1 FROM investigation_asset_backlog_dynamic_resolution_members member
                         WHERE member.fixed_point_receipt_id=receipt.fixed_point_receipt_id
                           AND member.hypothesis_root_id=root.root_id))))
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_DYNAMIC_RESOLUTION_CENSUS_DRIFT'
         USING ERRCODE='23514'; END IF;
    RETURN COALESCE(NEW,OLD);
END;
$$;
CREATE CONSTRAINT TRIGGER investigation_asset_backlog_dynamic_resolution_member_census
AFTER INSERT OR UPDATE OR DELETE ON investigation_asset_backlog_dynamic_resolution_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_asset_backlog_dynamic_resolution_census();
CREATE CONSTRAINT TRIGGER investigation_asset_backlog_dynamic_resolution_parent_census
AFTER INSERT ON investigation_asset_backlog_fixed_point_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_asset_backlog_dynamic_resolution_census();

CREATE FUNCTION investigation_guard_asset_backlog_dynamic_resolution_parent()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.dynamic_resolution_member_count IS NULL
       OR NEW.dynamic_resolution_member_set_sha256 IS NULL
       OR NEW.dynamic_resolution_member_count<>NEW.hypothesis_root_count
       OR EXISTS(
          SELECT 1 FROM attack_hypotheses root
           LEFT JOIN attack_hypothesis_heads head
             ON head.root_id=root.root_id
            AND head.operation_id=root.operation_id
            AND head.organization_id=root.organization_id
           LEFT JOIN attack_hypothesis_revisions terminal
             ON terminal.revision_id=head.head_revision_id
           WHERE root.asset_lane_id=NEW.asset_lane_id AND (
                 head.root_id IS NULL OR terminal.revision_id IS NULL
             OR head.head_lifecycle_state<>'closed'
             OR head.head_epistemic_state NOT IN('verified','refuted','invalid')
             OR terminal.lifecycle_state<>'closed'
             OR terminal.epistemic_state<>head.head_epistemic_state
             OR NOT EXISTS(
                SELECT 1
                  FROM investigation_dynamic_hypothesis_terminal_transitions transition
                  JOIN attack_hypothesis_revisions source_revision
                    ON source_revision.revision_id=transition.source_revision_id
                   AND source_revision.root_id=root.root_id
                   AND source_revision.operation_id=root.operation_id
                   AND source_revision.organization_id=root.organization_id
                  JOIN investigation_dynamic_hypothesis_resolutions resolution
                    ON resolution.resolution_authority_id=transition.resolution_authority_id
                   AND resolution.hypothesis_revision_id=transition.source_revision_id
                   AND resolution.asset_lane_id=NEW.asset_lane_id
                   AND resolution.disposition=transition.disposition
                  JOIN investigation_dynamic_verification_rounds dynamic_round
                    ON dynamic_round.session_id=resolution.session_id
                   AND dynamic_round.operation_id=root.operation_id
                   AND dynamic_round.organization_id=root.organization_id
                   AND dynamic_round.asset_lane_id=NEW.asset_lane_id
                   AND dynamic_round.hypothesis_revision_id=transition.source_revision_id
                   AND dynamic_round.state='resolved'
                   AND dynamic_round.resolution_authority_id=resolution.resolution_authority_id
                  JOIN attack_hypothesis_state_events event
                    ON event.event_id=transition.state_event_id
                   AND event.predecessor_revision_id=transition.source_revision_id
                   AND event.successor_revision_id=transition.terminal_revision_id
                   AND event.origin_authority='dynamic_verification_resolution'
                   AND event.authority_receipt_kind='dynamic_resolution'
                   AND event.authority_receipt_id=resolution.resolution_authority_id
                   AND event.authority_receipt_hash=resolution.resolution_sha256
                 WHERE transition.asset_lane_id=NEW.asset_lane_id
                   AND transition.terminal_revision_id=head.head_revision_id
                   AND transition.disposition=head.head_epistemic_state)))
    THEN RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_DYNAMIC_RESOLUTION_REQUIRED'
         USING ERRCODE='23514'; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_backlog_dynamic_resolution_parent_guard
BEFORE INSERT ON investigation_asset_backlog_fixed_point_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_backlog_dynamic_resolution_parent();
