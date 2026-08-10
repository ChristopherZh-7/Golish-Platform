-- Target Intel completion is now the adaptive Goal review/finalizer.  The
-- downstream Tool Truth execution bundle therefore contains only the stages
-- that still own exact execution-receipt denominators: EAS, Enumeration and
-- Vuln.  Historical four-root seals remain immutable/auditable, while all new
-- application writers enforce the three-root census.

ALTER TABLE tool_truth_authority_bundle_seals
    DROP CONSTRAINT tool_truth_authority_bundle_seal_shape_check;

ALTER TABLE tool_truth_authority_bundle_seals
    ADD CONSTRAINT tool_truth_authority_bundle_seal_shape_check CHECK (
        (sealed_at IS NULL AND relevant_root_count IS NULL AND relevant_root_set_hash IS NULL
            AND member_count IS NULL AND member_set_hash IS NULL
            AND semantic_authority_bundle_hash IS NULL
            AND freshness_attestation_bundle_hash IS NULL
            AND temporal_validity_bundle_hash IS NULL
            AND temporal_validity_policy_set_hash IS NULL
            AND target_state_epoch_set_hash IS NULL
            AND observation_window_started_at IS NULL
            AND observation_window_completed_at IS NULL
            AND effective_valid_until IS NULL
            AND consistent_fresh_count IS NULL AND stale_or_invalid_count IS NULL)
        OR (sealed_at IS NOT NULL AND relevant_root_count IN (3,4)
            AND member_count=relevant_root_count
            AND sealed_empty=FALSE AND relevant_root_set_hash IS NOT NULL
            AND member_set_hash IS NOT NULL AND semantic_authority_bundle_hash IS NOT NULL
            AND freshness_attestation_bundle_hash IS NOT NULL
            AND temporal_validity_bundle_hash IS NOT NULL
            AND temporal_validity_policy_set_hash IS NOT NULL
            AND target_state_epoch_set_hash IS NOT NULL
            AND (observation_window_started_at IS NULL
                 OR observation_window_completed_at>=observation_window_started_at)
            AND consistent_fresh_count+stale_or_invalid_count=member_count)
    );

ALTER TABLE candidate_analysis_snapshots
    DROP CONSTRAINT candidate_analysis_snapshots_relevant_root_count_check,
    DROP CONSTRAINT candidate_analysis_snapshots_bundle_member_count_check;

ALTER TABLE candidate_analysis_snapshots
    ADD CONSTRAINT candidate_analysis_snapshots_relevant_root_count_check
        CHECK (relevant_root_count IN (3,4)),
    ADD CONSTRAINT candidate_analysis_snapshots_bundle_member_count_check
        CHECK (bundle_member_count=relevant_root_count);

COMMENT ON COLUMN candidate_analysis_snapshots.relevant_root_count IS
    '3 for Goal-authority operations (EAS/Enum/Vuln execution roots); historical sealed snapshots may retain 4 including legacy Target Intel execution root';
