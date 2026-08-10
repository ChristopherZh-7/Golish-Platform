-- Keep immutable at-time authority rows after a two-phase organization delete.
--
-- These relations used live parent FKs even though their UUIDs are sealed
-- historical identity. Parent deletion must not cascade into or rewrite that
-- history. Future child writes retain FK-equivalent admission through the
-- key-share validators below, including the active-deletion fence.

CREATE FUNCTION organization_deletion_require_live_organization_reference()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    organization_ref UUID;
BEGIN
    organization_ref := NULLIF(to_jsonb(NEW)->>TG_ARGV[0], '')::UUID;
    IF organization_ref IS NULL THEN
        RETURN NEW;
    END IF;

    PERFORM 1 FROM organizations WHERE id=organization_ref FOR KEY SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'retained_authority_live_organization_missing'
            USING ERRCODE='23503';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM organization_deletion_job_units deletion_unit
          JOIN organization_deletion_jobs deletion_job
            ON deletion_job.id=deletion_unit.job_id
         WHERE deletion_unit.organization_id_at_time=organization_ref
           AND deletion_job.state<>'hard_delete_committed'
    ) THEN
        RAISE EXCEPTION 'organization_deletion_in_progress' USING ERRCODE='55000';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION organization_deletion_require_live_target_reference()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    target_ref UUID;
    expected_organization UUID;
    expected_project_path TEXT;
    live_organization UUID;
    live_project_path TEXT;
BEGIN
    target_ref := NULLIF(to_jsonb(NEW)->>TG_ARGV[0], '')::UUID;
    IF target_ref IS NULL THEN
        RETURN NEW;
    END IF;

    SELECT target.organization_id,target.project_path
      INTO live_organization,live_project_path
      FROM targets target
     WHERE target.id=target_ref
     FOR KEY SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'retained_authority_live_target_missing'
            USING ERRCODE='23503';
    END IF;

    IF TG_NARGS>1 AND TG_ARGV[1]<>'' THEN
        expected_organization := NULLIF(to_jsonb(NEW)->>TG_ARGV[1], '')::UUID;
        IF live_organization IS DISTINCT FROM expected_organization THEN
            RAISE EXCEPTION 'retained_authority_target_organization_mismatch'
                USING ERRCODE='23503';
        END IF;
    END IF;
    IF TG_NARGS>2 AND TG_ARGV[2]<>'' THEN
        expected_project_path := to_jsonb(NEW)->>TG_ARGV[2];
        IF live_project_path IS DISTINCT FROM expected_project_path THEN
            RAISE EXCEPTION 'retained_authority_target_project_mismatch'
                USING ERRCODE='23503';
        END IF;
    END IF;
    IF EXISTS (
        SELECT 1
          FROM organization_deletion_job_units deletion_unit
          JOIN organization_deletion_jobs deletion_job
            ON deletion_job.id=deletion_unit.job_id
         WHERE deletion_unit.organization_id_at_time=live_organization
           AND deletion_job.state<>'hard_delete_committed'
    ) THEN
        RAISE EXCEPTION 'organization_deletion_in_progress' USING ERRCODE='55000';
    END IF;
    RETURN NEW;
END;
$$;

-- Organization identities retained by sealed runtime/evidence graphs.
ALTER TABLE stage_asset_waves
    DROP CONSTRAINT stage_asset_waves_organization_id_fkey;
CREATE TRIGGER a_stage_asset_waves_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON stage_asset_waves
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE web_origins
    DROP CONSTRAINT web_origins_organization_id_fkey;
CREATE TRIGGER a_web_origins_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON web_origins
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE enumeration_endpoint_observations
    DROP CONSTRAINT enumeration_endpoint_observations_organization_id_fkey;
CREATE TRIGGER a_enumeration_endpoint_observations_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON enumeration_endpoint_observations
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE attack_hypotheses
    DROP CONSTRAINT attack_hypotheses_organization_id_fkey;
CREATE TRIGGER a_attack_hypotheses_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON attack_hypotheses
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE attack_hypothesis_relations
    DROP CONSTRAINT attack_hypothesis_relations_organization_id_fkey;
CREATE TRIGGER a_attack_hypothesis_relations_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON attack_hypothesis_relations
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE attack_hypothesis_state_events
    DROP CONSTRAINT attack_hypothesis_state_events_organization_id_fkey;
CREATE TRIGGER a_attack_hypothesis_state_events_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON attack_hypothesis_state_events
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE candidate_analysis_snapshots
    DROP CONSTRAINT candidate_analysis_snapshots_organization_id_fkey;
CREATE TRIGGER a_candidate_analysis_snapshots_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON candidate_analysis_snapshots
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE hypothesis_candidate_gate_decisions
    DROP CONSTRAINT hypothesis_candidate_gate_decisions_organization_id_fkey;
CREATE TRIGGER a_hypothesis_candidate_gate_decisions_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON hypothesis_candidate_gate_decisions
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE hypothesis_generations
    DROP CONSTRAINT hypothesis_generations_organization_id_fkey;
CREATE TRIGGER a_hypothesis_generations_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON hypothesis_generations
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE hypothesis_residual_risks
    DROP CONSTRAINT hypothesis_residual_risks_organization_id_fkey;
CREATE TRIGGER a_hypothesis_residual_risks_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON hypothesis_residual_risks
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE hypothesis_server_validation_receipts
    DROP CONSTRAINT hypothesis_server_validation_receipts_organization_id_fkey;
CREATE TRIGGER a_hypothesis_server_validation_receipts_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON hypothesis_server_validation_receipts
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE report_authority_invalidation_events
    DROP CONSTRAINT report_authority_invalidation_events_organization_id_fkey;
CREATE TRIGGER a_report_authority_invalidations_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON report_authority_invalidation_events
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

ALTER TABLE tool_truth_revalidation_obligations
    DROP CONSTRAINT tool_truth_revalidation_obligations_organization_id_fkey;
CREATE TRIGGER a_tool_truth_revalidation_live_organization
BEFORE INSERT OR UPDATE OF organization_id ON tool_truth_revalidation_obligations
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_organization_reference(
    'organization_id'
);

-- Target identities retained by sealed runtime/evidence graphs.
ALTER TABLE stage_asset_wave_items
    DROP CONSTRAINT stage_asset_wave_items_target_id_fkey;
CREATE TRIGGER a_stage_asset_wave_items_live_target
BEFORE INSERT OR UPDATE OF target_id ON stage_asset_wave_items
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_id','',''
);

ALTER TABLE api_endpoints
    DROP CONSTRAINT api_endpoints_target_id_fkey;
CREATE TRIGGER a_api_endpoints_live_target
BEFORE INSERT OR UPDATE OF target_id ON api_endpoints
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_id','',''
);

ALTER TABLE enumeration_endpoint_observations
    DROP CONSTRAINT enumeration_endpoint_observations_target_id_fkey;
CREATE TRIGGER a_enumeration_endpoint_observations_live_target
BEFORE INSERT OR UPDATE OF target_id ON enumeration_endpoint_observations
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_id','organization_id','project_path'
);

ALTER TABLE coverage_denominator_items
    DROP CONSTRAINT coverage_denominator_items_target_id_fkey;
CREATE TRIGGER a_coverage_denominator_items_live_target
BEFORE INSERT OR UPDATE OF target_id ON coverage_denominator_items
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_id','',''
);

ALTER TABLE enumeration_endpoint_groups
    DROP CONSTRAINT enumeration_endpoint_groups_resolved_target_id_organizatio_fkey;
CREATE TRIGGER a_enumeration_endpoint_groups_live_target
BEFORE INSERT OR UPDATE OF resolved_target_id,organization_id,project_path_at_freeze
ON enumeration_endpoint_groups
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'resolved_target_id','organization_id','project_path_at_freeze'
);

ALTER TABLE enumeration_endpoint_occurrences
    DROP CONSTRAINT enumeration_endpoint_occurren_resolved_target_id_organizat_fkey,
    DROP CONSTRAINT enumeration_endpoint_occurren_source_target_id_organizatio_fkey;
CREATE TRIGGER a_enumeration_endpoint_occurrences_live_resolved_target
BEFORE INSERT OR UPDATE OF resolved_target_id,organization_id,project_path_at_freeze
ON enumeration_endpoint_occurrences
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'resolved_target_id','organization_id','project_path_at_freeze'
);
CREATE TRIGGER a_enumeration_endpoint_occurrences_live_source_target
BEFORE INSERT OR UPDATE OF source_target_id,organization_id,project_path_at_freeze
ON enumeration_endpoint_occurrences
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'source_target_id','organization_id','project_path_at_freeze'
);

ALTER TABLE enumeration_lane_commit_receipts
    DROP CONSTRAINT enumeration_lane_commit_receipts_target_id_fkey;
CREATE TRIGGER a_enumeration_lane_commit_receipts_live_target
BEFORE INSERT OR UPDATE OF target_id ON enumeration_lane_commit_receipts
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_id','',''
);

ALTER TABLE target_intel_asset_observations
    DROP CONSTRAINT target_intel_asset_observations_promotion_target_id_fkey;
CREATE TRIGGER a_target_intel_asset_observations_live_promotion_target
BEFORE INSERT OR UPDATE OF promotion_target_id ON target_intel_asset_observations
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'promotion_target_id','',''
);
