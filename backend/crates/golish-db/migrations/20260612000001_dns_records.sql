-- DNS records discovered by dig/dnsx (design 2026-06-12 §5.2 / §7 D-schema).
-- New table, additive, replayable (IF NOT EXISTS) — I10 backward-compatible:
-- the read/write code ships after this migration; nothing reads it until then.
-- Backs the coverage gate's GOLISH-INTEL-DNS truth projection (coverage_truth).
CREATE TABLE IF NOT EXISTS dns_records (
    id           BIGSERIAL PRIMARY KEY,
    target_id    UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path TEXT NOT NULL DEFAULT '',
    record_type  TEXT NOT NULL,          -- A / AAAA / NS / MX / TXT / CNAME / SOA / PTR
    name         TEXT NOT NULL,          -- queried name (hostname)
    value        TEXT NOT NULL,          -- record value (ip / target / text)
    source       TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (target_id, record_type, name, value)
);

CREATE INDEX IF NOT EXISTS idx_dns_records_target ON dns_records(target_id);
CREATE INDEX IF NOT EXISTS idx_dns_records_type   ON dns_records(record_type);
