-- Unified Investigation may analyze an exact checked Tool Truth bundle while
-- preserving incomplete/stale roots as typed residual inputs. This does not
-- make those roots fresh and does not alter the legacy Candidate contract.
ALTER TABLE candidate_analysis_snapshots
    DROP CONSTRAINT candidate_analysis_snapshots_snapshot_status_check;

ALTER TABLE candidate_analysis_snapshots
    ADD CONSTRAINT candidate_analysis_snapshots_snapshot_status_check CHECK (
        snapshot_status IN (
            'sealed_ready',
            'sealed_analysis_ready_with_residuals',
            'blocked_authority_bundle'
        )
    );

COMMENT ON COLUMN candidate_analysis_snapshots.snapshot_status IS
    'sealed_analysis_ready_with_residuals is restricted by the writer to unified_investigation_v1 and preserves checked Tool Truth gaps as analysis inputs; it is not all-fresh authority';
