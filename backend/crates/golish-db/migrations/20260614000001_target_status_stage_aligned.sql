-- Stage-aligned target_status (design docs/design/2026-06-14-target-status-stage-aligned.md).
--
-- Old enum: new, recon, recon_done, scanning, tested
-- New enum: new, passive, active, enumerated, vuln_scan, verified
--
-- Makes the per-target lifecycle the AI's resume/skip signal: before running a
-- stage on a target the agent skips it when status >= that stage (mirrors the
-- engine's org-level resume oracle, pushed down to the target level).
--
-- Postgres enums can't reorder / drop values in place, so round-trip through
-- text: drop default -> column to text -> remap values -> drop+recreate the type
-- -> column back to the enum -> restore default. Only targets.status uses
-- target_status, so recreating the type is safe. All DDL is transactional and
-- runs once. Forward-only (invariant I10): never edits the original migration.

ALTER TABLE targets ALTER COLUMN status DROP DEFAULT;

ALTER TABLE targets ALTER COLUMN status TYPE text USING status::text;

UPDATE targets
SET status = CASE status
    WHEN 'recon' THEN 'passive'
    WHEN 'recon_done' THEN 'active'
    WHEN 'scanning' THEN 'vuln_scan'
    WHEN 'tested' THEN 'verified'
    ELSE 'new'
END;

DROP TYPE target_status;

CREATE TYPE target_status AS ENUM ('new', 'passive', 'active', 'enumerated', 'vuln_scan', 'verified');

ALTER TABLE targets
    ALTER COLUMN status TYPE target_status USING status::target_status;

ALTER TABLE targets ALTER COLUMN status SET DEFAULT 'new';
