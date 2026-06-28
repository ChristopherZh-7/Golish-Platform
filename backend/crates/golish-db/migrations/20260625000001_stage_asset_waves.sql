-- Durable per-stage asset waves (design 2026-06-28-stage-expansion-wave-barrier).
--
-- Pure additive schema (I10): the current no-schema `stage_started_at` cutoff
-- remains the fallback. Runtime starts using these rows only after repo/trait
-- wiring lands. One wave = immutable target set for one operation/stage/org
-- batch; newly discovered targets are promoted into a later wave instead of
-- moving the current gate denominator.

CREATE TABLE IF NOT EXISTS stage_asset_waves (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    operation_id    UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    stage_kind      TEXT NOT NULL,
    wave_index      INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'running', -- running | completed | failed
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    parent_wave_id  UUID REFERENCES stage_asset_waves(id) ON DELETE SET NULL,
    asset_hash      TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, organization_id, stage_kind, wave_index)
);

CREATE INDEX IF NOT EXISTS idx_stage_asset_waves_lookup
    ON stage_asset_waves(operation_id, organization_id, stage_kind, status, wave_index);

CREATE TABLE IF NOT EXISTS stage_asset_wave_items (
    id          BIGSERIAL PRIMARY KEY,
    wave_id     UUID NOT NULL REFERENCES stage_asset_waves(id) ON DELETE CASCADE,
    target_id   UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    asset_value TEXT NOT NULL,
    asset_type  TEXT NOT NULL,
    source      TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (wave_id, target_id)
);

CREATE INDEX IF NOT EXISTS idx_stage_asset_wave_items_wave
    ON stage_asset_wave_items(wave_id);

CREATE INDEX IF NOT EXISTS idx_stage_asset_wave_items_target
    ON stage_asset_wave_items(target_id);

COMMENT ON TABLE stage_asset_waves IS
    'Durable stage expansion waves. A wave freezes the target asset set for one operation/stage/organization batch; targets discovered while a wave runs are promoted to a later wave.';

COMMENT ON TABLE stage_asset_wave_items IS
    'Immutable target membership for a stage_asset_waves row.';
