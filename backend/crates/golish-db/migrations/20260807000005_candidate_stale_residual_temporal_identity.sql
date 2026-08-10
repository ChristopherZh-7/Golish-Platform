-- A stale residual is one-for-one with a Candidate temporal census member.
-- One tool-truth bundle member can legitimately yield multiple temporal
-- members (for example, repeated observations of the same target).  The
-- temporal_census_member_id UNIQUE/FK already provides the exact source
-- identity; snapshot+bundle_member is therefore an invalid collapsing key.

ALTER TABLE candidate_analysis_stale_evidence_residuals
    DROP CONSTRAINT candidate_analysis_stale_evide_snapshot_id_bundle_member_id_key;
