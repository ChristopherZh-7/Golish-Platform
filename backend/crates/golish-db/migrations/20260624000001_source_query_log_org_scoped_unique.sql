-- Fix #5 source_query_log org isolation for multi-org stage_run.
--
-- The original unique key was (run_id, source, query, target), which collapsed
-- root-org and child-org provider rows in the same run. Per-org gates then read
-- zero source rows for child organizations even after their providers ran. Make
-- the idempotency key org-scoped while preserving the table shape.

ALTER TABLE source_query_log
    DROP CONSTRAINT IF EXISTS source_query_log_run_id_source_query_target_key;

CREATE UNIQUE INDEX IF NOT EXISTS idx_source_query_log_org_run_source_query_target_unique
    ON source_query_log (organization_id, run_id, source, query, target);

COMMENT ON TABLE source_query_log IS
    'Per (org x run x source x query x target) passive-intel data-source query log. Proves which sources were queried and with what result (found|empty|error|blocked) + evidence. Org is part of the idempotency key so stage_run fan-out rows for sibling orgs never collapse into the root org.';
