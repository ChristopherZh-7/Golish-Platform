-- New Tool Truth authority bundles contain the three execution-receipt roots:
-- EAS, Enumeration and Vuln. Target Intel is authorized separately by its
-- finalized adaptive Goal review/finalizer.
CREATE OR REPLACE FUNCTION tool_truth_guard_authority_bundle_header()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE actual_count BIGINT;
DECLARE min_ordinal BIGINT;
DECLARE max_ordinal BIGINT;
DECLARE root_family_count BIGINT;
DECLARE actual_member_hash TEXT;
DECLARE root_hash TEXT;
DECLARE semantic_hash TEXT;
DECLARE freshness_hash TEXT;
DECLARE temporal_hash TEXT;
DECLARE policy_hash TEXT;
DECLARE epoch_hash TEXT;
DECLARE window_started TIMESTAMPTZ;
DECLARE window_completed TIMESTAMPTZ;
DECLARE valid_until TIMESTAMPTZ;
DECLARE fresh_count BIGINT;
BEGIN
    IF TG_OP='INSERT' THEN
        IF NEW.sealed_at IS NOT NULL OR NEW.member_count IS NOT NULL
           OR NEW.relevant_root_count IS NOT NULL THEN
            RAISE EXCEPTION 'tool_truth_unsealed_authority' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP='DELETE' OR OLD.sealed_at IS NOT NULL OR NEW.sealed_at IS NULL
       OR (to_jsonb(NEW)-ARRAY[
            'sealed_at','relevant_root_count','relevant_root_set_hash','member_count',
            'member_set_hash','sealed_empty','semantic_authority_bundle_hash',
            'freshness_attestation_bundle_hash','temporal_validity_bundle_hash',
            'temporal_validity_policy_set_hash','target_state_epoch_set_hash',
            'observation_window_started_at','observation_window_completed_at',
            'effective_valid_until','consistent_fresh_count','stale_or_invalid_count'
          ]) IS DISTINCT FROM
          (to_jsonb(OLD)-ARRAY[
            'sealed_at','relevant_root_count','relevant_root_set_hash','member_count',
            'member_set_hash','sealed_empty','semantic_authority_bundle_hash',
            'freshness_attestation_bundle_hash','temporal_validity_bundle_hash',
            'temporal_validity_policy_set_hash','target_state_epoch_set_hash',
            'observation_window_started_at','observation_window_completed_at',
            'effective_valid_until','consistent_fresh_count','stale_or_invalid_count'
          ]) THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE='23514';
    END IF;
    SELECT count(*)::BIGINT,coalesce(min(ordinal),0)::BIGINT,
           coalesce(max(ordinal),-1)::BIGINT,count(DISTINCT root_family)::BIGINT,
           tool_truth_sha256(coalesce(jsonb_agg(member_hash ORDER BY ordinal),'[]'::JSONB)::TEXT),
           tool_truth_sha256(coalesce(jsonb_agg(jsonb_build_object(
               'root_family',root_family,'root_denominator_id',root_denominator_id,
               'root_denominator_hash',root_denominator_hash
           ) ORDER BY ordinal),'[]'::JSONB)::TEXT),
           tool_truth_sha256(coalesce(jsonb_agg(jsonb_build_object(
               'root_family',root_family,'semantic_status',semantic_status,
               'authority_set_semantic_hash',authority_set_semantic_hash
           ) ORDER BY ordinal),'[]'::JSONB)::TEXT),
           tool_truth_sha256(coalesce(jsonb_agg(jsonb_build_object(
               'root_family',root_family,'authority_set_freshness_hash',authority_set_freshness_hash
           ) ORDER BY ordinal),'[]'::JSONB)::TEXT),
           tool_truth_sha256(coalesce(jsonb_agg(jsonb_build_object(
               'root_family',root_family,'status',temporal_validity_status,
               'policy',temporal_validity_policy_set_hash,
               'epochs',target_state_epoch_set_hash,
               'window_started',observation_window_started_at,
               'window_completed',observation_window_completed_at,
               'valid_until',effective_valid_until
           ) ORDER BY ordinal),'[]'::JSONB)::TEXT),
           tool_truth_sha256(coalesce(jsonb_agg(temporal_validity_policy_set_hash ORDER BY ordinal),'[]'::JSONB)::TEXT),
           tool_truth_sha256(coalesce(jsonb_agg(target_state_epoch_set_hash ORDER BY ordinal),'[]'::JSONB)::TEXT),
           min(observation_window_started_at),max(observation_window_completed_at),
           min(effective_valid_until),
           count(*) FILTER (WHERE member_status='consistent_fresh')::BIGINT
      INTO actual_count,min_ordinal,max_ordinal,root_family_count,actual_member_hash,
           root_hash,semantic_hash,freshness_hash,temporal_hash,policy_hash,epoch_hash,
           window_started,window_completed,valid_until,fresh_count
      FROM tool_truth_authority_bundle_members WHERE bundle_seal_id=NEW.id;
    IF actual_count<>3 OR root_family_count<>3 OR min_ordinal<>0 OR max_ordinal<>2 THEN
        RAISE EXCEPTION 'tool_truth_authority_bundle_root_census_incomplete' USING ERRCODE='23514';
    END IF;
    NEW.relevant_root_count:=actual_count;
    NEW.relevant_root_set_hash:=root_hash;
    NEW.member_count:=actual_count;
    NEW.member_set_hash:=actual_member_hash;
    NEW.sealed_empty:=FALSE;
    NEW.semantic_authority_bundle_hash:=semantic_hash;
    NEW.freshness_attestation_bundle_hash:=freshness_hash;
    NEW.temporal_validity_bundle_hash:=temporal_hash;
    NEW.temporal_validity_policy_set_hash:=policy_hash;
    NEW.target_state_epoch_set_hash:=epoch_hash;
    NEW.observation_window_started_at:=window_started;
    NEW.observation_window_completed_at:=window_completed;
    NEW.effective_valid_until:=valid_until;
    NEW.consistent_fresh_count:=fresh_count;
    NEW.stale_or_invalid_count:=actual_count-fresh_count;
    NEW.sealed_at:=statement_timestamp();
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION tool_truth_guard_authority_bundle_header() IS
    'Seals new three-root execution-receipt authority bundles; Target Intel completion is governed by finalized adaptive Goal authority.';

CREATE OR REPLACE FUNCTION enforce_candidate_snapshot_exact_authority_bundle()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actual_count BIGINT;
    ordinal_count BIGINT;
    root_family_count BIGINT;
    fresh_count BIGINT;
BEGIN
    SELECT COUNT(*),COUNT(DISTINCT ordinal),COUNT(DISTINCT root_family),
           COUNT(*) FILTER (WHERE member_status='consistent_fresh')
      INTO actual_count,ordinal_count,root_family_count,fresh_count
      FROM candidate_analysis_snapshot_authority_bundle_members
     WHERE snapshot_id=NEW.snapshot_id;
    IF actual_count<>3 OR ordinal_count<>3 OR root_family_count<>3 THEN
        RAISE EXCEPTION 'CANDIDATE_SNAPSHOT_AUTHORITY_BUNDLE_EXACT_SET_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    IF NEW.snapshot_status='sealed_ready' AND fresh_count<>3 THEN
        RAISE EXCEPTION 'CANDIDATE_SNAPSHOT_ALL_FRESH_AUTHORITY_REQUIRED'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION enforce_candidate_snapshot_exact_authority_bundle() IS
    'Requires the exact three execution-receipt roots copied from the Tool Truth authority bundle.';
