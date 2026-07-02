-- Attack-stage split (design 2026-07-02-attack-stage-formulaic-candidate-exploit
-- §3.7): the attack_candidate stage produces structured attack hypotheses
-- (AttackCandidate). This table persists them so the chain-wave controller can
-- de-duplicate across waves, track a→b→c lineage (parent_finding_id), and drive
-- the candidate disposition state machine consumed by the verification stage.
--
-- I10 safety: additive migration, no existing table/column touched, all defaults
-- so old rows/deliverables are unaffected. I2: every read/write filters by
-- operation_id + (org scope) organization_id. The (operation_id, target,
-- hypothesis_hash) unique index de-duplicates so a↔b never regenerate endlessly.

CREATE TABLE IF NOT EXISTS attack_candidates (
    candidate_id       UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    operation_id       TEXT NOT NULL,
    organization_id    UUID REFERENCES organizations(id) ON DELETE CASCADE,
    target             TEXT NOT NULL,
    hypothesis         TEXT NOT NULL,
    hypothesis_hash    TEXT NOT NULL,
    technique          TEXT,
    rationale          TEXT NOT NULL DEFAULT '',
    prior_refs         JSONB NOT NULL DEFAULT '[]',
    suggested_approach TEXT NOT NULL DEFAULT '',
    priority           TEXT NOT NULL DEFAULT 'medium',
    wave               INTEGER NOT NULL DEFAULT 0,
    parent_finding_id  UUID,
    disposition        TEXT NOT NULL DEFAULT 'proposed',
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT attack_candidates_priority_check CHECK (priority IN ('high', 'medium', 'low')),
    CONSTRAINT attack_candidates_disposition_check CHECK (disposition IN ('proposed', 'approved', 'rejected', 'verified', 'refuted', 'blocked'))
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_attack_candidates_op_target_hash
ON attack_candidates (operation_id, target, hypothesis_hash);

CREATE INDEX IF NOT EXISTS idx_attack_candidates_org ON attack_candidates(organization_id);
CREATE INDEX IF NOT EXISTS idx_attack_candidates_wave ON attack_candidates(operation_id, wave);
CREATE INDEX IF NOT EXISTS idx_attack_candidates_parent ON attack_candidates(parent_finding_id);

COMMENT ON TABLE attack_candidates IS
    'Structured attack hypotheses (AttackCandidate) synthesized by the attack_candidate stage (design 2026-07-02). disposition state machine: proposed -> approved/rejected (human review) -> verified/refuted/blocked (verification). parent_finding_id + wave capture the a->b->c chain-wave lineage; (operation_id, target, hypothesis_hash) de-duplicates across waves.';
