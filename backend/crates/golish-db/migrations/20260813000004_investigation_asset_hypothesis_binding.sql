-- Bind the canonical hypothesis spine to the server-frozen Investigation asset
-- queue. Columns are added without rewriting archived rows, but NULL has no
-- executable meaning: the compiler and all new authority writes fail closed
-- unless one exact asset lane is present end-to-end.

ALTER TABLE candidate_analysis_snapshots
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE candidate_analysis_attempts
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE investigation_run_work_items
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE attack_hypotheses
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE attack_hypothesis_revisions
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE hypothesis_generations
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE hypothesis_generation_members
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE hypothesis_verification_tasks
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE hypothesis_verification_task_campaigns
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE verification_campaigns
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE verification_wave_coverage_denominators
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE hypothesis_pending_evolution_authorities
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE investigation_evolution_analysis_primary_rearms
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;
ALTER TABLE hypothesis_fixed_point_receipts
    ADD COLUMN asset_lane_id UUID REFERENCES investigation_asset_lanes(asset_lane_id) ON DELETE RESTRICT;

DO $$
DECLARE
    old_wave_constraint NAME;
BEGIN
    SELECT constraint_name INTO old_wave_constraint
      FROM information_schema.table_constraints
     WHERE table_schema=current_schema()
       AND table_name='candidate_analysis_snapshots'
       AND constraint_type='UNIQUE'
       AND constraint_name <> 'candidate_analysis_snapshots_pkey'
       AND (SELECT array_agg(column_name::TEXT ORDER BY ordinal_position)
              FROM information_schema.key_column_usage key_column
             WHERE key_column.constraint_schema=current_schema()
               AND key_column.constraint_name=table_constraints.constraint_name)=
           ARRAY['operation_id','organization_id','wave_ordinal']::TEXT[];
    IF old_wave_constraint IS NOT NULL THEN
        EXECUTE format('ALTER TABLE candidate_analysis_snapshots DROP CONSTRAINT %I',
                       old_wave_constraint);
    END IF;
END;
$$;
ALTER TABLE candidate_analysis_snapshots
    ADD CONSTRAINT candidate_analysis_snapshots_asset_lane_wave_key
    UNIQUE(asset_lane_id,wave_ordinal);

-- NOT VALID preserves immutable archive rows created before the asset queue
-- contract, while PostgreSQL still enforces the check on every new or updated
-- row. A historical NULL row therefore cannot re-enter execution.
ALTER TABLE candidate_analysis_snapshots
    ADD CONSTRAINT candidate_analysis_snapshots_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE candidate_analysis_attempts
    ADD CONSTRAINT candidate_analysis_attempts_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE attack_hypotheses
    ADD CONSTRAINT attack_hypotheses_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE attack_hypothesis_revisions
    ADD CONSTRAINT attack_hypothesis_revisions_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE hypothesis_generations
    ADD CONSTRAINT hypothesis_generations_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE hypothesis_generation_members
    ADD CONSTRAINT hypothesis_generation_members_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE hypothesis_verification_tasks
    ADD CONSTRAINT hypothesis_verification_tasks_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE hypothesis_verification_task_campaigns
    ADD CONSTRAINT hypothesis_verification_task_campaigns_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE verification_campaigns
    ADD CONSTRAINT verification_campaigns_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE verification_wave_coverage_denominators
    ADD CONSTRAINT verification_wave_coverage_denominators_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE hypothesis_pending_evolution_authorities
    ADD CONSTRAINT hypothesis_pending_evolution_authorities_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE investigation_evolution_analysis_primary_rearms
    ADD CONSTRAINT investigation_evolution_analysis_primary_rearms_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;
ALTER TABLE hypothesis_fixed_point_receipts
    ADD CONSTRAINT hypothesis_fixed_point_receipts_asset_lane_required
    CHECK(asset_lane_id IS NOT NULL) NOT VALID;

CREATE INDEX candidate_analysis_snapshots_asset_lane_idx
    ON candidate_analysis_snapshots(asset_lane_id) WHERE asset_lane_id IS NOT NULL;
CREATE INDEX attack_hypotheses_asset_lane_idx
    ON attack_hypotheses(asset_lane_id) WHERE asset_lane_id IS NOT NULL;
CREATE INDEX hypothesis_generations_asset_lane_idx
    ON hypothesis_generations(asset_lane_id,generation_ordinal)
    WHERE asset_lane_id IS NOT NULL;
CREATE INDEX hypothesis_verification_tasks_asset_lane_idx
    ON hypothesis_verification_tasks(asset_lane_id,created_at,task_id)
    WHERE asset_lane_id IS NOT NULL;
CREATE INDEX verification_campaigns_asset_lane_idx
    ON verification_campaigns(asset_lane_id,state,campaign_id)
    WHERE asset_lane_id IS NOT NULL;

-- Resolve an AI proposal subject back to exactly one server-owned target lane.
-- This function is an admission primitive only; it does not authorize I/O and
-- does not accept target ids, values, or URLs from model prose.
CREATE FUNCTION investigation_resolve_proposal_asset_lane(
    requested_asset_lane_id UUID,
    requested_subject_kind TEXT,
    requested_subject_identity_sha256 TEXT
) RETURNS UUID
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    lane investigation_asset_lanes%ROWTYPE;
    expected_identity_sha256 TEXT;
    subject_belongs_to_lane BOOLEAN := FALSE;
BEGIN
    SELECT * INTO lane
      FROM investigation_asset_lanes
     WHERE asset_lane_id=requested_asset_lane_id;
    IF NOT FOUND OR requested_subject_identity_sha256 !~ '^sha256:[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PROPOSAL_ASSET_LANE_REQUIRED'
            USING ERRCODE='23514';
    END IF;

    CASE requested_subject_kind
        WHEN 'asset' THEN
            expected_identity_sha256 := tool_truth_sha256(jsonb_build_object(
                'domain','investigation_subject_identity.v1',
                'subject_kind','asset',
                'subject_id',lane.target_id,
                'display_value',lane.target_value_at_freeze
            )::TEXT);
            subject_belongs_to_lane :=
                expected_identity_sha256=requested_subject_identity_sha256;
        WHEN 'endpoint' THEN
            SELECT tool_truth_sha256(jsonb_build_object(
                       'domain','investigation_subject_identity.v1',
                       'subject_kind','endpoint',
                       'subject_id',endpoint.id,
                       'display_value',endpoint.url
                   )::TEXT)
              INTO expected_identity_sha256
              FROM api_endpoints endpoint
             WHERE endpoint.target_id=lane.target_id
               AND tool_truth_sha256(jsonb_build_object(
                       'domain','investigation_subject_identity.v1',
                       'subject_kind','endpoint',
                       'subject_id',endpoint.id,
                       'display_value',endpoint.url
                   )::TEXT)=requested_subject_identity_sha256;
            subject_belongs_to_lane := FOUND;
        WHEN 'web_origin' THEN
            SELECT tool_truth_sha256(jsonb_build_object(
                       'domain','investigation_subject_identity.v1',
                       'subject_kind','web_origin',
                       'subject_id',origin.id,
                       'display_value',origin.origin
                   )::TEXT)
              INTO expected_identity_sha256
              FROM web_origins origin
             WHERE origin.organization_id=lane.organization_id
               AND EXISTS(
                    SELECT 1
                      FROM fingerprint_origin_observations observation
                     WHERE observation.web_origin_id=origin.id
                       AND observation.target_id=lane.target_id
               )
               AND tool_truth_sha256(jsonb_build_object(
                       'domain','investigation_subject_identity.v1',
                       'subject_kind','web_origin',
                       'subject_id',origin.id,
                       'display_value',origin.origin
                   )::TEXT)=requested_subject_identity_sha256;
            subject_belongs_to_lane := FOUND;
        ELSE
            RAISE EXCEPTION 'INVESTIGATION_PROPOSAL_SUBJECT_UNASSIGNED'
                USING ERRCODE='23514';
    END CASE;

    IF NOT subject_belongs_to_lane
       OR expected_identity_sha256 IS DISTINCT FROM requested_subject_identity_sha256
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PROPOSAL_SUBJECT_ASSET_LANE_MISMATCH'
            USING ERRCODE='23514';
    END IF;
    RETURN lane.target_id;
END;
$$;

-- One deferred guard covers the lane columns. It runs after all rows in the
-- compiler transaction exist, but still fails that transaction closed if any
-- executable authority is unbound or points at a sibling lane.
CREATE FUNCTION investigation_guard_asset_hypothesis_lane()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    lane investigation_asset_lanes%ROWTYPE;
    parent_lane_id UUID;
BEGIN
    IF TG_TABLE_NAME='investigation_run_work_items'
       AND NEW.work_kind NOT IN ('analysis','verification_task','campaign','prepared_action',
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
        IF NEW.work_kind IN ('analysis','verification_task','campaign','prepared_action',
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

CREATE CONSTRAINT TRIGGER candidate_analysis_snapshots_asset_lane_guard
AFTER INSERT OR UPDATE ON candidate_analysis_snapshots DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER candidate_analysis_attempts_asset_lane_guard
AFTER INSERT OR UPDATE ON candidate_analysis_attempts DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER investigation_run_work_items_asset_lane_guard
AFTER INSERT OR UPDATE ON investigation_run_work_items DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER attack_hypotheses_asset_lane_guard
AFTER INSERT OR UPDATE ON attack_hypotheses DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER attack_hypothesis_revisions_asset_lane_guard
AFTER INSERT OR UPDATE ON attack_hypothesis_revisions DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER hypothesis_generations_asset_lane_guard
AFTER INSERT OR UPDATE ON hypothesis_generations DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER hypothesis_generation_members_asset_lane_guard
AFTER INSERT OR UPDATE ON hypothesis_generation_members DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER hypothesis_verification_tasks_asset_lane_guard
AFTER INSERT OR UPDATE ON hypothesis_verification_tasks DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER hypothesis_verification_task_campaigns_asset_lane_guard
AFTER INSERT OR UPDATE ON hypothesis_verification_task_campaigns DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER verification_campaigns_asset_lane_guard
AFTER INSERT OR UPDATE ON verification_campaigns DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER verification_wave_coverage_denominators_asset_lane_guard
AFTER INSERT OR UPDATE ON verification_wave_coverage_denominators DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER hypothesis_pending_evolution_authorities_asset_lane_guard
AFTER INSERT OR UPDATE ON hypothesis_pending_evolution_authorities DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER investigation_evolution_analysis_primary_rearms_asset_lane_guard
AFTER INSERT OR UPDATE ON investigation_evolution_analysis_primary_rearms DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
CREATE CONSTRAINT TRIGGER hypothesis_fixed_point_receipts_asset_lane_guard
AFTER INSERT OR UPDATE ON hypothesis_fixed_point_receipts DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION investigation_guard_asset_hypothesis_lane();
