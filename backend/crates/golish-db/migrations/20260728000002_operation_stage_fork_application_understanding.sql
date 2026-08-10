-- Extend immutable operation stage forks for the Application Model V1 graph.
--
-- The original fork lineage predates Application Understanding and ranked
-- Attack Candidate immediately after Vuln Triage.  The application layer now
-- permits an AU-only fork, so the database authority must recognize the same
-- topology.  Existing fork rows remain valid; this migration changes no data.

ALTER TABLE operation_stage_forks
    DROP CONSTRAINT operation_stage_forks_entry_stage_check,
    DROP CONSTRAINT operation_stage_forks_check4;

CREATE OR REPLACE FUNCTION operation_stage_fork_stage_rank(stage_kind TEXT)
RETURNS SMALLINT AS $$
    SELECT CASE stage_kind
        WHEN 'scoping' THEN 1
        WHEN 'target_intel' THEN 2
        WHEN 'external_attack_surface' THEN 3
        WHEN 'enumeration' THEN 4
        WHEN 'vuln_triage' THEN 5
        WHEN 'application_understanding' THEN 6
        WHEN 'attack_candidate' THEN 7
        ELSE NULL
    END
$$ LANGUAGE sql IMMUTABLE PARALLEL SAFE;

ALTER TABLE operation_stage_forks
    ADD CONSTRAINT operation_stage_forks_entry_stage_check
        CHECK (operation_stage_fork_stage_rank(entry_stage) BETWEEN 2 AND 7),
    ADD CONSTRAINT operation_stage_forks_check4
        CHECK (
            operation_stage_fork_stage_rank(terminal_stage)
                BETWEEN operation_stage_fork_stage_rank(entry_stage) AND 7
        );
