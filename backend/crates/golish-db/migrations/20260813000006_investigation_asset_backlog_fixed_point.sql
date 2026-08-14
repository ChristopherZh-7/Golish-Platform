-- Close one mandatory Investigation asset lane only after every canonical
-- hypothesis head is explicitly resolved. Verification/evolution bookkeeping
-- remains audit-only; the later discovery migration adds the only dynamic-new-
-- hypothesis backlog guard once that authority table exists.
-- The close receipt and queue advancement events commit in one transaction.

CREATE TABLE investigation_asset_backlog_fixed_point_receipts (
    fixed_point_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    asset_lane_id UUID NOT NULL UNIQUE,
    asset_queue_id UUID NOT NULL,
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    generation_count BIGINT NOT NULL CHECK(generation_count>=0),
    hypothesis_root_count BIGINT NOT NULL CHECK(hypothesis_root_count>0),
    revision_count BIGINT NOT NULL CHECK(revision_count>0),
    verification_task_count BIGINT NOT NULL CHECK(verification_task_count>=0),
    campaign_count BIGINT NOT NULL CHECK(campaign_count>=0),
    prepared_action_count BIGINT NOT NULL CHECK(prepared_action_count>=0),
    action_execution_count BIGINT NOT NULL CHECK(action_execution_count>=0),
    oracle_count BIGINT NOT NULL CHECK(oracle_count>=0),
    fact_delta_count BIGINT NOT NULL CHECK(fact_delta_count>=0),
    wave_count BIGINT NOT NULL CHECK(wave_count>=0),
    advanced_wave_count BIGINT NOT NULL CHECK(advanced_wave_count>=0),
    fixed_point_wave_count BIGINT NOT NULL CHECK(fixed_point_wave_count>=0),
    backlog_member_count BIGINT NOT NULL CHECK(backlog_member_count=0),
    backlog_set_sha256 TEXT NOT NULL CHECK(backlog_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    obligation_set_sha256 TEXT NOT NULL CHECK(obligation_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    residual_set_sha256 TEXT NOT NULL CHECK(residual_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    request_fingerprint_sha256 TEXT NOT NULL CHECK(request_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
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
    CHECK(hypothesis_root_count<=revision_count)
);

CREATE TABLE investigation_asset_progression_receipts (
    progression_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    source_fixed_point_receipt_id UUID UNIQUE
        REFERENCES investigation_asset_backlog_fixed_point_receipts(fixed_point_receipt_id)
        ON DELETE RESTRICT,
    source_zero_fixed_point_receipt_id UUID UNIQUE
        REFERENCES investigation_asset_zero_hypothesis_fixed_point_receipts(fixed_point_receipt_id)
        ON DELETE RESTRICT,
    fixed_asset_lane_id UUID NOT NULL UNIQUE,
    fixed_asset_event_id UUID NOT NULL,
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL,
    asset_queue_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    expected_company_queue_head_version BIGINT NOT NULL CHECK(expected_company_queue_head_version>=0),
    expected_company_member_row_version BIGINT NOT NULL CHECK(expected_company_member_row_version>=0),
    expected_asset_queue_head_version BIGINT NOT NULL CHECK(expected_asset_queue_head_version>=0),
    expected_asset_lane_row_version BIGINT NOT NULL CHECK(expected_asset_lane_row_version>=0),
    disposition TEXT NOT NULL CHECK(disposition IN('next_asset','next_company','investigation_complete')),
    next_company_member_id UUID,
    next_asset_lane_id UUID,
    next_company_claim_event_id UUID,
    next_asset_claim_event_id UUID,
    auto_completed_company_count BIGINT NOT NULL CHECK(auto_completed_company_count>=0),
    result_company_queue_head_version BIGINT NOT NULL CHECK(result_company_queue_head_version>=0),
    request_fingerprint_sha256 TEXT NOT NULL CHECK(request_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK((source_fixed_point_receipt_id IS NOT NULL)::INTEGER+
          (source_zero_fixed_point_receipt_id IS NOT NULL)::INTEGER=1),
    CHECK(
        (disposition='next_asset' AND next_company_member_id=company_member_id
             AND next_asset_lane_id IS NOT NULL AND next_company_claim_event_id IS NULL
             AND next_asset_claim_event_id IS NOT NULL)
        OR
        (disposition='next_company' AND next_company_member_id IS NOT NULL
             AND next_company_member_id<>company_member_id
             AND next_asset_lane_id IS NOT NULL AND next_company_claim_event_id IS NOT NULL
             AND next_asset_claim_event_id IS NOT NULL)
        OR
        (disposition='investigation_complete' AND next_company_member_id IS NULL
             AND next_asset_lane_id IS NULL AND next_company_claim_event_id IS NULL
             AND next_asset_claim_event_id IS NULL)
    ),
    FOREIGN KEY(
        fixed_asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) REFERENCES investigation_asset_lanes(
        asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
        operation_id,scope_snapshot_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(next_company_claim_event_id)
        REFERENCES investigation_company_queue_events(event_id) ON DELETE RESTRICT,
    FOREIGN KEY(next_asset_claim_event_id)
        REFERENCES investigation_asset_lane_events(event_id) ON DELETE RESTRICT,
    FOREIGN KEY(next_company_member_id)
        REFERENCES investigation_company_queue_members(company_member_id) ON DELETE RESTRICT,
    FOREIGN KEY(next_asset_lane_id)
        REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_asset_backlog_fixed_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_asset_backlog_fixed_point_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_queue_append_only_mutation();
CREATE TRIGGER investigation_asset_progression_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_asset_progression_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_queue_append_only_mutation();

CREATE FUNCTION investigation_validate_asset_backlog_fixed_point()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    actual_generation_count BIGINT;
    actual_hypothesis_root_count BIGINT;
    actual_revision_count BIGINT;
    actual_task_count BIGINT;
    actual_campaign_count BIGINT;
    actual_action_count BIGINT;
    actual_execution_count BIGINT;
    actual_oracle_count BIGINT;
    actual_fact_delta_count BIGINT;
    actual_wave_count BIGINT;
    actual_advanced_count BIGINT;
    actual_fixed_count BIGINT;
    exact_empty_backlog_hash TEXT;
BEGIN
    exact_empty_backlog_hash := investigation_exact_member_set_hash(
        'golish.investigation.asset_backlog.v1',ARRAY[]::TEXT[]
    );
    IF NOT EXISTS(
            SELECT 1 FROM investigation_asset_lanes lane
             WHERE lane.asset_lane_id=NEW.asset_lane_id
               AND lane.asset_queue_id=NEW.asset_queue_id
               AND lane.company_queue_id=NEW.company_queue_id
               AND lane.company_member_id=NEW.company_member_id
               AND lane.operation_id=NEW.operation_id
               AND lane.scope_snapshot_id=NEW.scope_snapshot_id
               AND lane.organization_id=NEW.organization_id
               AND lane.state='consolidating'
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_FIXED_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;

    IF EXISTS(
        SELECT 1 FROM attack_hypotheses root
        LEFT JOIN attack_hypothesis_heads head
          ON head.root_id=root.root_id
         AND head.operation_id=root.operation_id
         AND head.organization_id=root.organization_id
        LEFT JOIN attack_hypothesis_revisions revision
          ON revision.revision_id=head.head_revision_id
         AND revision.root_id=head.root_id
         AND revision.operation_id=head.operation_id
         AND revision.organization_id=head.organization_id
         WHERE root.asset_lane_id=NEW.asset_lane_id AND (
               head.root_id IS NULL OR revision.revision_id IS NULL OR NOT (
               head.head_lifecycle_state='closed'
           AND head.head_epistemic_state IN('verified','refuted','invalid')
           AND revision.lifecycle_state='closed'
           AND revision.epistemic_state=head.head_epistemic_state
         ))
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_NOT_DRAINED' USING ERRCODE='23514';
    END IF;

    SELECT count(*) INTO actual_generation_count FROM hypothesis_generations
     WHERE asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_hypothesis_root_count FROM attack_hypotheses
     WHERE asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_revision_count FROM attack_hypothesis_revisions
     WHERE asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_task_count FROM hypothesis_verification_tasks
     WHERE asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_campaign_count FROM verification_campaigns
     WHERE asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_action_count
      FROM verification_prepared_actions prepared JOIN verification_campaigns campaign
        ON campaign.campaign_id=prepared.campaign_id
     WHERE campaign.asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_execution_count
      FROM verification_action_executions execution
      JOIN verification_prepared_actions prepared ON prepared.prepared_action_id=execution.prepared_action_id
      JOIN verification_campaigns campaign ON campaign.campaign_id=prepared.campaign_id
     WHERE campaign.asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_oracle_count
      FROM verification_oracle_assessments oracle JOIN verification_campaigns campaign
        ON campaign.campaign_id=oracle.campaign_id
     WHERE campaign.asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_fact_delta_count
      FROM verification_fact_delta_bundles delta JOIN verification_campaigns campaign
        ON campaign.campaign_id=delta.campaign_id
     WHERE campaign.asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_wave_count FROM verification_wave_coverage_denominators
     WHERE asset_lane_id=NEW.asset_lane_id AND sealed_at IS NOT NULL;
    SELECT count(*) INTO actual_advanced_count
      FROM verification_wave_coverage_denominators wave
      JOIN verification_wave_coverage_receipts coverage
        ON coverage.wave_denominator_id=wave.wave_denominator_id
      JOIN hypothesis_consolidation_batches batch
        ON batch.wave_coverage_receipt_id=coverage.wave_coverage_receipt_id
      JOIN hypothesis_consolidation_receipts receipt
        ON receipt.consolidation_batch_id=batch.consolidation_batch_id
       AND receipt.disposition='advanced'
     WHERE wave.asset_lane_id=NEW.asset_lane_id;
    SELECT count(*) INTO actual_fixed_count FROM hypothesis_fixed_point_receipts
     WHERE asset_lane_id=NEW.asset_lane_id;

    IF (actual_generation_count,actual_hypothesis_root_count,actual_revision_count,
        actual_task_count,actual_campaign_count,
        actual_action_count,actual_execution_count,actual_oracle_count,actual_fact_delta_count,
        actual_wave_count,actual_advanced_count,actual_fixed_count)
       IS DISTINCT FROM
       (NEW.generation_count,NEW.hypothesis_root_count,NEW.revision_count,
        NEW.verification_task_count,NEW.campaign_count,
        NEW.prepared_action_count,NEW.action_execution_count,NEW.oracle_count,NEW.fact_delta_count,
        NEW.wave_count,NEW.advanced_wave_count,NEW.fixed_point_wave_count)
       OR NEW.backlog_member_count<>0
       OR NEW.backlog_set_sha256<>exact_empty_backlog_hash
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_CENSUS_MISMATCH' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_asset_backlog_fixed_receipt_validate
BEFORE INSERT ON investigation_asset_backlog_fixed_point_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_validate_asset_backlog_fixed_point();

-- Resolution-only stage publication. This authority is owned by the sealed
-- company/asset queue and never depends on the historical operation-global
-- Campaign/Wave closure reducer.
CREATE TABLE investigation_asset_queue_closure_publications (
    publication_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    company_queue_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL UNIQUE,
    stage_execution_id UUID NOT NULL UNIQUE,
    owning_stage_run_request_id TEXT NOT NULL CHECK(BTRIM(owning_stage_run_request_id)<>''),
    scope_snapshot_id UUID NOT NULL,
    member_count BIGINT NOT NULL CHECK(member_count>0),
    member_set_sha256 TEXT NOT NULL CHECK(member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    publication_sha256 TEXT NOT NULL CHECK(publication_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    published_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(publication_id,operation_id,stage_execution_id,scope_snapshot_id),
    FOREIGN KEY(company_queue_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id)
        REFERENCES investigation_company_queues(
            company_queue_id,authority_id,operation_id,stage_execution_id,scope_snapshot_id
        ) ON DELETE RESTRICT
);

CREATE TABLE investigation_asset_queue_closure_publication_members (
    publication_member_id UUID PRIMARY KEY,
    publication_id UUID NOT NULL REFERENCES investigation_asset_queue_closure_publications(
        publication_id) ON DELETE RESTRICT,
    member_ordinal INTEGER NOT NULL CHECK(member_ordinal>=0),
    company_queue_id UUID NOT NULL,
    company_member_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL UNIQUE,
    stage_team_plan_id UUID NOT NULL UNIQUE,
    member_sha256 TEXT NOT NULL CHECK(member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    passed_at TIMESTAMPTZ NOT NULL,
    UNIQUE(publication_id,member_ordinal),
    UNIQUE(publication_id,organization_id),
    UNIQUE(publication_id,member_sha256),
    FOREIGN KEY(publication_id,operation_id,stage_execution_id,scope_snapshot_id)
        REFERENCES investigation_asset_queue_closure_publications(
            publication_id,operation_id,stage_execution_id,scope_snapshot_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(company_member_id,company_queue_id,operation_id,scope_snapshot_id,organization_id)
        REFERENCES investigation_company_queue_members(
            company_member_id,company_queue_id,operation_id,scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,organization_id)
        REFERENCES stage_run_units(
            id,operation_id,stage_execution_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES stage_team_plans(
            id,operation_id,stage_execution_id,stage_run_unit_id,
            scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_asset_queue_closure_publication()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS(
        SELECT 1 FROM investigation_company_queues queue
         WHERE queue.company_queue_id=NEW.company_queue_id
           AND queue.authority_id=NEW.authority_id
           AND queue.operation_id=NEW.operation_id
           AND queue.stage_execution_id=NEW.stage_execution_id
           AND queue.owning_stage_run_request_id=NEW.owning_stage_run_request_id
           AND queue.scope_snapshot_id=NEW.scope_snapshot_id
           AND queue.state='completed'
           AND queue.member_count=NEW.member_count
           AND queue.member_count=(
               SELECT COUNT(*)
                 FROM investigation_company_queue_members company
                 JOIN stage_run_units unit
                   ON unit.operation_id=queue.operation_id
                  AND unit.stage_execution_id=queue.stage_execution_id
                  AND unit.scope_snapshot_id=queue.scope_snapshot_id
                  AND unit.organization_id=company.organization_id
                  AND unit.stage_kind='investigation'
                  AND unit.status='running'
                 JOIN stage_team_plans plan
                   ON plan.stage_run_unit_id=unit.id
                  AND plan.operation_id=unit.operation_id
                  AND plan.stage_execution_id=unit.stage_execution_id
                  AND plan.scope_snapshot_id=unit.scope_snapshot_id
                  AND plan.organization_id=unit.organization_id
                  AND plan.stage_kind='investigation'
                WHERE company.company_queue_id=queue.company_queue_id
                  AND company.state='completed'
           )
           AND NOT EXISTS(
               SELECT 1 FROM stage_run_units unit
                WHERE unit.operation_id=queue.operation_id
                  AND unit.stage_execution_id=queue.stage_execution_id
                  AND unit.scope_snapshot_id=queue.scope_snapshot_id
                  AND NOT EXISTS(
                      SELECT 1 FROM investigation_company_queue_members company
                       WHERE company.company_queue_id=queue.company_queue_id
                         AND company.organization_id=unit.organization_id
                  )
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_CLOSURE_PUBLICATION_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_queue_closure_publication_guard
BEFORE INSERT ON investigation_asset_queue_closure_publications
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_queue_closure_publication();

CREATE FUNCTION investigation_guard_asset_queue_closure_member()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS(
        SELECT 1
          FROM investigation_asset_queue_closure_publications publication
          JOIN investigation_company_queue_members company
            ON company.company_member_id=NEW.company_member_id
           AND company.company_queue_id=publication.company_queue_id
           AND company.operation_id=publication.operation_id
           AND company.scope_snapshot_id=publication.scope_snapshot_id
           AND company.organization_id=NEW.organization_id
           AND company.state='completed'
          JOIN stage_run_units unit
            ON unit.id=NEW.stage_run_unit_id
           AND unit.operation_id=publication.operation_id
           AND unit.stage_execution_id=publication.stage_execution_id
           AND unit.scope_snapshot_id=publication.scope_snapshot_id
           AND unit.organization_id=NEW.organization_id
           AND unit.stage_kind='investigation'
           AND unit.status='passed'
           AND unit.terminal_at=NEW.passed_at
          JOIN stage_team_plans plan
            ON plan.id=NEW.stage_team_plan_id
           AND plan.stage_run_unit_id=unit.id
           AND plan.operation_id=unit.operation_id
           AND plan.stage_execution_id=unit.stage_execution_id
           AND plan.scope_snapshot_id=unit.scope_snapshot_id
           AND plan.organization_id=unit.organization_id
           AND plan.stage_kind='investigation'
           AND plan.requests_closed_at IS NOT NULL
         WHERE publication.publication_id=NEW.publication_id
           AND NEW.company_queue_id=publication.company_queue_id
           AND NEW.operation_id=publication.operation_id
           AND NEW.stage_execution_id=publication.stage_execution_id
           AND NEW.scope_snapshot_id=publication.scope_snapshot_id
           AND NEW.passed_at=publication.published_at
           AND NEW.member_sha256=tool_truth_sha256(format(
               'golish.investigation.asset_queue_closure_member.v1:%s:%s:%s:%s:%s',
               NEW.publication_id,NEW.company_member_id,NEW.organization_id,
               NEW.stage_run_unit_id,NEW.stage_team_plan_id
           ))
           AND unit.pass_watermark=jsonb_build_object(
               'schema','investigation_asset_queue_closure_publication.v1',
               'publication_id',NEW.publication_id,
               'company_queue_id',NEW.company_queue_id,
               'company_member_id',NEW.company_member_id,
               'member_sha256',NEW.member_sha256
           )
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_QUEUE_CLOSURE_MEMBER_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER investigation_asset_queue_closure_member_guard
BEFORE INSERT ON investigation_asset_queue_closure_publication_members
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_queue_closure_member();

CREATE TRIGGER investigation_asset_queue_closure_publications_append_only
BEFORE UPDATE OR DELETE ON investigation_asset_queue_closure_publications
FOR EACH ROW EXECUTE FUNCTION investigation_reject_queue_append_only_mutation();
CREATE TRIGGER investigation_asset_queue_closure_publication_members_append_only
BEFORE UPDATE OR DELETE ON investigation_asset_queue_closure_publication_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_queue_append_only_mutation();

ALTER TABLE investigation_asset_progression_receipts
    ADD COLUMN stage_closure_publication_id UUID UNIQUE REFERENCES
        investigation_asset_queue_closure_publications(publication_id) ON DELETE RESTRICT,
    ADD CONSTRAINT investigation_asset_progression_exact_closure_publication CHECK(
        (disposition='investigation_complete')=(stage_closure_publication_id IS NOT NULL)
    );

-- Forward-replace the lane event reducer: ordinary consolidating -> fixed_point
-- is reachable only when the exact lane backlog receipt is already present in
-- this transaction.  All legacy transition branches remain unchanged.
CREATE OR REPLACE FUNCTION investigation_apply_asset_lane_event()
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
    ELSIF NEW.from_state='consolidating' AND NEW.to_state='fixed_point'
          AND NEW.event_kind='fixed_point' THEN
        IF NOT EXISTS(
            SELECT 1 FROM investigation_asset_backlog_fixed_point_receipts receipt
             WHERE receipt.asset_lane_id=NEW.asset_lane_id
               AND receipt.stable_request_id=NEW.stable_request_id
        ) THEN
            RAISE EXCEPTION 'INVESTIGATION_ASSET_BACKLOG_FIXED_RECEIPT_REQUIRED' USING ERRCODE='23514';
        END IF;
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

CREATE FUNCTION investigation_validate_asset_progression_receipt()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    company_member_count BIGINT;
BEGIN
    SELECT member_count INTO company_member_count FROM investigation_company_queues
     WHERE company_queue_id=NEW.company_queue_id;
    IF company_member_count IS NULL
       OR NEW.auto_completed_company_count>company_member_count
       OR NEW.result_company_queue_head_version-NEW.expected_company_queue_head_version<>
          (CASE NEW.disposition
            WHEN 'next_asset' THEN 0
            WHEN 'next_company' THEN 2+2*NEW.auto_completed_company_count
            WHEN 'investigation_complete' THEN 1+2*NEW.auto_completed_company_count
          END)
       OR NOT (
            (NEW.source_fixed_point_receipt_id IS NOT NULL AND EXISTS(
                SELECT 1 FROM investigation_asset_backlog_fixed_point_receipts fixed
                 WHERE fixed.fixed_point_receipt_id=NEW.source_fixed_point_receipt_id
                   AND fixed.asset_lane_id=NEW.fixed_asset_lane_id
                   AND fixed.stable_request_id=NEW.stable_request_id
            ))
            OR
            (NEW.source_zero_fixed_point_receipt_id IS NOT NULL AND EXISTS(
                SELECT 1 FROM investigation_asset_zero_hypothesis_fixed_point_receipts fixed
                 WHERE fixed.fixed_point_receipt_id=NEW.source_zero_fixed_point_receipt_id
                   AND fixed.asset_lane_id=NEW.fixed_asset_lane_id
            ))
       )
       OR NOT EXISTS(
            SELECT 1 FROM investigation_asset_lanes lane
             WHERE lane.asset_lane_id=NEW.fixed_asset_lane_id AND lane.state='fixed_point'
               AND lane.latest_event_id=NEW.fixed_asset_event_id
       )
       OR NOT (
            (NEW.source_fixed_point_receipt_id IS NOT NULL AND EXISTS(
                SELECT 1 FROM investigation_asset_lane_events lane_event
                 WHERE lane_event.event_id=NEW.fixed_asset_event_id
                   AND lane_event.asset_lane_id=NEW.fixed_asset_lane_id
                   AND lane_event.asset_queue_id=NEW.asset_queue_id
                   AND lane_event.event_kind='fixed_point'
            ))
            OR
            (NEW.source_zero_fixed_point_receipt_id IS NOT NULL AND EXISTS(
                SELECT 1 FROM investigation_asset_zero_hypothesis_fixed_point_receipts fixed
                 WHERE fixed.fixed_point_receipt_id=NEW.source_zero_fixed_point_receipt_id
                   AND fixed.asset_lane_id=NEW.fixed_asset_lane_id
            ))
       )
       OR (NEW.disposition='next_asset' AND NOT EXISTS(
            SELECT 1 FROM investigation_asset_lanes lane
             WHERE lane.asset_lane_id=NEW.next_asset_lane_id AND lane.state='analyzing'
               AND lane.latest_event_id=NEW.next_asset_claim_event_id
               AND lane.company_member_id=NEW.company_member_id
       ))
       OR (NEW.disposition='next_company' AND NOT EXISTS(
            SELECT 1 FROM investigation_company_queue_members member
            JOIN investigation_asset_lanes lane ON lane.company_member_id=member.company_member_id
             WHERE member.company_member_id=NEW.next_company_member_id AND member.state='active'
               AND member.latest_event_id=NEW.next_company_claim_event_id
               AND lane.asset_lane_id=NEW.next_asset_lane_id AND lane.state='analyzing'
               AND lane.latest_event_id=NEW.next_asset_claim_event_id
       ))
       OR (NEW.disposition='investigation_complete' AND NOT EXISTS(
            SELECT 1 FROM investigation_company_queues queue
             WHERE queue.company_queue_id=NEW.company_queue_id AND queue.state='completed'
       ))
       OR (NEW.disposition='investigation_complete' AND NOT EXISTS(
            SELECT 1 FROM investigation_asset_queue_closure_publications publication
             WHERE publication.publication_id=NEW.stage_closure_publication_id
               AND publication.stable_request_id=NEW.stable_request_id
               AND publication.company_queue_id=NEW.company_queue_id
               AND publication.operation_id=NEW.operation_id
               AND publication.scope_snapshot_id=NEW.scope_snapshot_id
               AND publication.member_count=(
                   SELECT COUNT(*) FROM investigation_company_queue_members member
                    WHERE member.company_queue_id=NEW.company_queue_id
                      AND member.state='completed'
               )
               AND publication.member_count=(
                   SELECT COUNT(*)
                     FROM investigation_asset_queue_closure_publication_members member
                    WHERE member.publication_id=publication.publication_id
               )
               AND publication.member_set_sha256=investigation_exact_member_set_hash(
                   'golish.investigation.asset_queue_closure_members.v1',
                   ARRAY(SELECT member.member_sha256
                           FROM investigation_asset_queue_closure_publication_members member
                          WHERE member.publication_id=publication.publication_id
                          ORDER BY member.member_ordinal)
               )
               AND publication.publication_sha256=tool_truth_sha256(format(
                   'golish.investigation.asset_queue_closure_publication.v1:%s:%s:%s:%s:%s:%s:%s',
                   publication.publication_id,publication.company_queue_id,
                   publication.authority_id,publication.operation_id,
                   publication.stage_execution_id,publication.scope_snapshot_id,
                   publication.member_set_sha256
               ))
               AND NOT EXISTS(
                   SELECT 1
                     FROM investigation_asset_queue_closure_publication_members member
                     JOIN stage_run_units unit ON unit.id=member.stage_run_unit_id
                     JOIN stage_team_plans plan ON plan.id=member.stage_team_plan_id
                     LEFT JOIN org_stage_completions completion
                       ON completion.organization_id=member.organization_id
                      AND completion.stage_kind='investigation'
                    WHERE member.publication_id=publication.publication_id
                      AND (completion.stage_run_id IS DISTINCT FROM NEW.operation_id::TEXT
                           OR completion.passed_at IS DISTINCT FROM member.passed_at)
               )
               AND NOT EXISTS(
                   SELECT 1
                     FROM investigation_asset_queue_closure_publication_members member
                     JOIN stage_run_units unit ON unit.id=member.stage_run_unit_id
                     JOIN stage_team_plans plan ON plan.id=member.stage_team_plan_id
                    WHERE member.publication_id=publication.publication_id
                      AND (unit.status<>'passed'
                           OR unit.terminal_at IS DISTINCT FROM member.passed_at
                           OR plan.requests_closed_at IS NULL
                           OR unit.pass_watermark IS DISTINCT FROM jsonb_build_object(
                               'schema','investigation_asset_queue_closure_publication.v1',
                               'publication_id',publication.publication_id,
                               'company_queue_id',publication.company_queue_id,
                               'company_member_id',member.company_member_id,
                               'member_sha256',member.member_sha256
                           ))
               )
       ))
    THEN
        RAISE EXCEPTION 'INVESTIGATION_ASSET_PROGRESSION_AUTHORITY_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_asset_progression_receipt_validate
BEFORE INSERT ON investigation_asset_progression_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_validate_asset_progression_receipt();
