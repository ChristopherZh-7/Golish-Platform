-- Phase 1 (设计 2026-06-12-redteam-phase1 / active-collection-db-truth-closure §3.1):
-- de-dup anchor for the new `endpoint_add` writer. katana/gau/waybackurls/arjun
-- land into api_endpoints via ON CONFLICT (target_id, url, method) DO NOTHING;
-- without this unique index the upsert has no conflict target.
--
-- I10: additive, idempotent (IF NOT EXISTS), no data migration. Safe to replay.
CREATE UNIQUE INDEX IF NOT EXISTS uq_api_endpoint_target_url_method
    ON api_endpoints (target_id, url, method);
