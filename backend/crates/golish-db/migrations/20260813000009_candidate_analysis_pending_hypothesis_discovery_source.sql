-- Asset-scoped Investigation analysis freezes unconsumed hypothesis discoveries
-- as immutable Candidate snapshot sources before the compiler admits them.  Keep
-- the source vocabulary closed while adding that server-authored authority kind.
ALTER TABLE candidate_analysis_snapshot_source_sets
    DROP CONSTRAINT candidate_analysis_snapshot_source_sets_source_kind_check;

ALTER TABLE candidate_analysis_snapshot_source_sets
    ADD CONSTRAINT candidate_analysis_snapshot_source_sets_source_kind_check
    CHECK (source_kind IN (
        'tool_truth_bundle',
        'previous_generation',
        'state_events',
        'relations',
        'open_obligations',
        'expected_fact_deltas',
        'unconsumed_fact_deltas',
        'consumed_fact_deltas',
        'verification_fact_deltas',
        'managed_knowledge_feed',
        'predecessor_handoff',
        'predecessor_evidence',
        'pending_hypothesis_discovery'
    ));
