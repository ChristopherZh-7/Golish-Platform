-- Per-(organization, stage) completion ledger so `stage_run` can resume-skip an
-- org whose stage already passed its own gate (design: user-requested resume in
-- chat stage_run; replaces the AlwaysRunOracle "照跑所有 org" placeholder for the
-- chat fan-out path). One row per (organization_id, stage_kind), upserted to the
-- latest pass; `passed_at` is the freshness clock ("上次测的是啥时候"), so the
-- skip judgment is "has this org passed this stage within the TTL window".
--
-- New table, additive, replayable (IF NOT EXISTS) — I10 backward-compatible: the
-- read/write code ships after this migration; nothing reads it until then. An
-- absent row simply means "never completed" → run normally (prior behaviour).
CREATE TABLE IF NOT EXISTS org_stage_completions (
    id              BIGSERIAL PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    stage_kind      TEXT NOT NULL,          -- StageKind::as_str() (e.g. 'target_intel')
    passed_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    stage_run_id    TEXT,                   -- informational: the stage_run/tool id that last passed
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (organization_id, stage_kind)
);

CREATE INDEX IF NOT EXISTS idx_org_stage_completions_org
    ON org_stage_completions(organization_id);
