-- Candidate temporal census membership mirrors the upstream temporal census
-- member set one-for-one. Target identity alone is not sufficient because a
-- receipt may contain multiple distinct observations for the same target and
-- temporal fact class.

ALTER TABLE candidate_analysis_temporal_validity_census_members
    ADD COLUMN source_temporal_census_member_id UUID;

ALTER TABLE candidate_analysis_temporal_validity_census_members
    DISABLE TRIGGER candidate_analysis_temporal_validity_census_members_append_only;

UPDATE candidate_analysis_temporal_validity_census_members candidate
   SET source_temporal_census_member_id=source.id
  FROM capability_execution_temporal_census_members source
 WHERE source.census_id=candidate.temporal_census_id
   AND source.receipt_id IS NOT DISTINCT FROM candidate.receipt_id
   AND source.member_hash=candidate.decision_hash;

ALTER TABLE candidate_analysis_temporal_validity_census_members
    ENABLE TRIGGER candidate_analysis_temporal_validity_census_members_append_only;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM candidate_analysis_temporal_validity_census_members
         WHERE source_temporal_census_member_id IS NULL
    ) THEN
        RAISE EXCEPTION 'candidate_temporal_source_member_backfill_incomplete';
    END IF;
END
$$;

ALTER TABLE candidate_analysis_temporal_validity_census_members
    ALTER COLUMN source_temporal_census_member_id SET NOT NULL,
    DROP CONSTRAINT candidate_temporal_census_target_identity_key,
    ADD CONSTRAINT candidate_temporal_census_source_member_key UNIQUE (
        census_id,
        source_temporal_census_member_id
    ),
    ADD CONSTRAINT candidate_temporal_source_member_fk FOREIGN KEY (
        source_temporal_census_member_id
    ) REFERENCES capability_execution_temporal_census_members(id) ON DELETE RESTRICT;
