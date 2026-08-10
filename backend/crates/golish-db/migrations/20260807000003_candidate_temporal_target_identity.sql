-- Candidate temporal authority is per target-state identity, not merely per
-- receipt/evidence class. A single capability receipt can legitimately cover
-- multiple denominator inputs, each with its own target-state epoch.

ALTER TABLE candidate_analysis_temporal_validity_census_members
    ADD COLUMN target_scope_identity_hash TEXT;

ALTER TABLE candidate_analysis_temporal_validity_census_members
    DISABLE TRIGGER candidate_analysis_temporal_validity_census_members_append_only;

UPDATE candidate_analysis_temporal_validity_census_members candidate
   SET target_scope_identity_hash=source.target_scope_identity_hash
  FROM capability_execution_temporal_census_members source
 WHERE source.census_id=candidate.temporal_census_id
   AND source.receipt_id IS NOT DISTINCT FROM candidate.receipt_id
   AND source.policy_member_id IS NOT DISTINCT FROM candidate.policy_member_id
   AND source.temporal_fact_class=candidate.evidence_class
   AND source.member_hash=candidate.decision_hash;

ALTER TABLE candidate_analysis_temporal_validity_census_members
    ENABLE TRIGGER candidate_analysis_temporal_validity_census_members_append_only;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM candidate_analysis_temporal_validity_census_members
         WHERE target_scope_identity_hash IS NULL
    ) THEN
        RAISE EXCEPTION 'candidate_temporal_target_identity_backfill_incomplete';
    END IF;
END
$$;

ALTER TABLE candidate_analysis_temporal_validity_census_members
    ALTER COLUMN target_scope_identity_hash SET NOT NULL,
    ADD CONSTRAINT candidate_temporal_target_identity_hash_check
        CHECK (target_scope_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    DROP CONSTRAINT candidate_analysis_temporal_v_census_id_bundle_member_id_re_key,
    ADD CONSTRAINT candidate_temporal_census_target_identity_key UNIQUE (
        census_id,
        bundle_member_id,
        receipt_id,
        evidence_class,
        target_scope_identity_hash
    );
