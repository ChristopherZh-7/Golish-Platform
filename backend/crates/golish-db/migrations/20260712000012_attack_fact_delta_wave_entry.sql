-- Typed FactDelta acceptance and follow-on Candidate Wave provenance.
--
-- Migration 00004 intentionally froze the initial Wave entry as an exact
-- final-passed vuln_triage handoff.  A follow-on Wave has a different
-- authority: one immutable, evidence-backed consolidation of the previous
-- Wave.  This additive migration makes those entry shapes mutually exclusive
-- without rewriting historical Wave rows or advancing the rollout singleton.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Keep the database-side decision authority byte-for-byte compatible with the
-- Rust canonical JSON contract.  Object keys are sorted, arrays retain order,
-- and no insignificant whitespace is emitted.
CREATE FUNCTION attack_fact_delta_canonical_jsonb(input_value JSONB)
RETURNS TEXT AS $$
DECLARE
    value_kind TEXT;
    rendered TEXT;
BEGIN
    value_kind := jsonb_typeof(input_value);
    CASE value_kind
        WHEN 'null' THEN RETURN 'null';
        WHEN 'boolean' THEN RETURN input_value::TEXT;
        WHEN 'number' THEN RETURN input_value::TEXT;
        WHEN 'string' THEN RETURN to_jsonb(input_value #>> '{}')::TEXT;
        WHEN 'array' THEN
            SELECT '[' || COALESCE(
                       STRING_AGG(
                           attack_fact_delta_canonical_jsonb(element.value),
                           ',' ORDER BY element.ordinal
                       ),
                       ''
                   ) || ']'
              INTO rendered
              FROM jsonb_array_elements(input_value)
                   WITH ORDINALITY AS element(value, ordinal);
            RETURN rendered;
        WHEN 'object' THEN
            SELECT '{' || COALESCE(
                       STRING_AGG(
                           to_jsonb(entry.key)::TEXT || ':' ||
                               attack_fact_delta_canonical_jsonb(entry.value),
                           ',' ORDER BY entry.key
                       ),
                       ''
                   ) || '}'
              INTO rendered
              FROM jsonb_each(input_value) AS entry(key, value);
            RETURN rendered;
        ELSE
            RAISE EXCEPTION 'FactDelta canonical JSON value is unsupported';
    END CASE;
END;
$$ LANGUAGE plpgsql IMMUTABLE STRICT;

CREATE FUNCTION attack_fact_delta_sha256_jsonb(input_value JSONB)
RETURNS TEXT AS $$
    SELECT ENCODE(
        DIGEST(attack_fact_delta_canonical_jsonb(input_value), 'sha256'),
        'hex'
    );
$$ LANGUAGE sql IMMUTABLE STRICT;

ALTER TABLE attack_fact_deltas
    ADD COLUMN accepted_at TIMESTAMPTZ,
    ADD CONSTRAINT attack_fact_deltas_delta_kind_closed CHECK (
        delta_kind IN ('created', 'updated', 'refuted', 'new_surface')
    ) NOT VALID,
    ADD CONSTRAINT attack_fact_deltas_consolidation_identity_unique UNIQUE (
        id,
        source_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id
    ),
    ADD CONSTRAINT attack_fact_deltas_acceptance_time_check CHECK (
        (
            status IN ('accepted', 'consumed')
            AND accepted_at IS NOT NULL
        )
        OR (
            status IN ('proposed', 'rejected')
            AND accepted_at IS NULL
        )
    ) NOT VALID;

UPDATE attack_fact_deltas
   SET accepted_at = updated_at
 WHERE status IN ('accepted', 'consumed')
   AND accepted_at IS NULL;

ALTER TABLE attack_fact_deltas
    VALIDATE CONSTRAINT attack_fact_deltas_acceptance_time_check;

CREATE TABLE attack_fact_delta_decisions (
    fact_delta_id UUID PRIMARY KEY,
    source_attempt_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    source_wave_run_id UUID NOT NULL,
    source_wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('accepted', 'rejected')),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    canonical_ref_kind TEXT NOT NULL CHECK (BTRIM(canonical_ref_kind) <> ''),
    canonical_ref_id UUID NOT NULL,
    canonical_ref_version BIGINT NOT NULL CHECK (canonical_ref_version > 0),
    proposed_ref_hash TEXT NOT NULL CHECK (BTRIM(proposed_ref_hash) <> ''),
    resolved_ref_version BIGINT CHECK (resolved_ref_version > 0),
    resolved_ref_hash TEXT,
    evidence_set_hash TEXT NOT NULL CHECK (BTRIM(evidence_set_hash) <> ''),
    decision_hash TEXT NOT NULL CHECK (BTRIM(decision_hash) <> ''),
    row_version BIGINT NOT NULL DEFAULT 1 CHECK (row_version = 1),
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, decision_hash),
    UNIQUE (
        fact_delta_id,
        operation_id,
        scope_snapshot_id,
        source_wave_run_id,
        source_wave_unit_id,
        organization_id
    ),
    FOREIGN KEY (
        fact_delta_id,
        source_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        source_wave_run_id,
        source_wave_unit_id,
        organization_id
    ) REFERENCES attack_fact_deltas (
        id,
        source_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id
    ) ON DELETE RESTRICT,
    CHECK (
        (
            disposition = 'accepted'
            AND reason_code = 'accepted'
            AND resolved_ref_version = canonical_ref_version
            AND resolved_ref_hash = proposed_ref_hash
        )
        OR (
            disposition = 'rejected'
            AND reason_code <> 'accepted'
        )
    )
);

CREATE TABLE attack_wave_consolidations (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    source_wave_run_id UUID NOT NULL,
    source_generation INTEGER NOT NULL CHECK (source_generation >= 0),
    decision_kind TEXT NOT NULL CHECK (
        decision_kind IN ('opened_next_wave', 'closed_no_delta', 'exhausted')
    ),
    target_wave_run_id UUID,
    target_generation INTEGER CHECK (target_generation >= 0),
    source_wave_version_before BIGINT NOT NULL CHECK (source_wave_version_before >= 0),
    source_wave_version_after BIGINT NOT NULL CHECK (
        source_wave_version_after = source_wave_version_before + 1
    ),
    source_barrier_hash TEXT NOT NULL CHECK (BTRIM(source_barrier_hash) <> ''),
    policy_hash TEXT NOT NULL CHECK (BTRIM(policy_hash) <> ''),
    fact_delta_set_hash TEXT NOT NULL CHECK (BTRIM(fact_delta_set_hash) <> ''),
    fact_delta_count INTEGER NOT NULL CHECK (fact_delta_count >= 0),
    wave_count INTEGER NOT NULL CHECK (wave_count >= 1),
    candidate_count INTEGER NOT NULL CHECK (candidate_count >= 0),
    chain_depth INTEGER NOT NULL CHECK (chain_depth >= 0),
    attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    decision_hash TEXT NOT NULL CHECK (BTRIM(decision_hash) <> ''),
    row_version BIGINT NOT NULL DEFAULT 1 CHECK (row_version = 1),
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (source_wave_run_id),
    UNIQUE (target_wave_run_id),
    UNIQUE (operation_id, decision_hash),
    UNIQUE (
        id,
        operation_id,
        scope_snapshot_id,
        source_wave_run_id
    ),
    UNIQUE (
        id,
        target_wave_run_id,
        operation_id,
        scope_snapshot_id
    ),
    FOREIGN KEY (source_wave_run_id, operation_id, scope_snapshot_id)
        REFERENCES attack_wave_runs(id, operation_id, scope_snapshot_id) ON DELETE RESTRICT,
    FOREIGN KEY (target_wave_run_id, operation_id, scope_snapshot_id)
        REFERENCES attack_wave_runs(id, operation_id, scope_snapshot_id) ON DELETE RESTRICT,
    CHECK (
        (
            decision_kind = 'opened_next_wave'
            AND target_wave_run_id IS NOT NULL
            AND target_generation = source_generation + 1
            AND fact_delta_count > 0
            AND reason_code = 'accepted_fact_delta'
        )
        OR (
            decision_kind = 'closed_no_delta'
            AND target_wave_run_id IS NULL
            AND target_generation IS NULL
            AND fact_delta_count = 0
            AND reason_code = 'no_accepted_fact_delta'
        )
        OR (
            decision_kind = 'exhausted'
            AND target_wave_run_id IS NULL
            AND target_generation IS NULL
            AND fact_delta_count > 0
            AND reason_code IN (
                'max_waves',
                'max_candidates_total',
                'max_chain_depth',
                'max_attempts_total'
            )
        )
    )
);

ALTER TABLE attack_wave_units
    ADD COLUMN entry_consolidation_id UUID,
    ALTER COLUMN entry_stage_execution_id DROP NOT NULL,
    ALTER COLUMN entry_stage_run_unit_id DROP NOT NULL,
    ALTER COLUMN entry_deliverable_submission_id DROP NOT NULL,
    ALTER COLUMN entry_stage_kind DROP NOT NULL,
    ADD CONSTRAINT attack_wave_units_entry_shape_check CHECK (
        (
            entry_consolidation_id IS NULL
            AND entry_stage_execution_id IS NOT NULL
            AND entry_stage_run_unit_id IS NOT NULL
            AND entry_deliverable_submission_id IS NOT NULL
            AND entry_stage_kind = 'vuln_triage'
        )
        OR (
            entry_consolidation_id IS NOT NULL
            AND entry_stage_execution_id IS NULL
            AND entry_stage_run_unit_id IS NULL
            AND entry_deliverable_submission_id IS NULL
            AND entry_stage_kind IS NULL
        )
    ) NOT VALID,
    ADD CONSTRAINT attack_wave_units_entry_consolidation_fk FOREIGN KEY (
        entry_consolidation_id,
        wave_run_id,
        operation_id,
        scope_snapshot_id
    ) REFERENCES attack_wave_consolidations (
        id,
        target_wave_run_id,
        operation_id,
        scope_snapshot_id
    ) ON DELETE RESTRICT;

ALTER TABLE attack_wave_units
    VALIDATE CONSTRAINT attack_wave_units_entry_shape_check;

ALTER TABLE attack_candidate_work_items
    ADD CONSTRAINT attack_candidate_work_items_consolidation_identity_unique UNIQUE (
        id,
        wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    );

ALTER TABLE attack_residual_risks
    ADD CONSTRAINT attack_residual_risks_consolidation_identity_unique UNIQUE (
        id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id
    );

CREATE TABLE attack_wave_consolidation_members (
    consolidation_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    fact_delta_id UUID NOT NULL,
    source_attempt_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    source_wave_run_id UUID NOT NULL,
    source_wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_wave_run_id UUID,
    target_wave_unit_id UUID,
    target_work_item_id UUID,
    residual_risk_id UUID,
    member_hash TEXT NOT NULL CHECK (BTRIM(member_hash) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (consolidation_id, ordinal),
    UNIQUE (consolidation_id, fact_delta_id),
    UNIQUE (fact_delta_id),
    FOREIGN KEY (
        consolidation_id,
        operation_id,
        scope_snapshot_id,
        source_wave_run_id
    ) REFERENCES attack_wave_consolidations (
        id,
        operation_id,
        scope_snapshot_id,
        source_wave_run_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        fact_delta_id,
        source_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        source_wave_run_id,
        source_wave_unit_id,
        organization_id
    ) REFERENCES attack_fact_deltas (
        id,
        source_attempt_id,
        candidate_id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        target_wave_unit_id,
        target_wave_run_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES attack_wave_units (
        id,
        wave_run_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        target_work_item_id,
        target_wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES attack_candidate_work_items (
        id,
        wave_unit_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        residual_risk_id,
        operation_id,
        scope_snapshot_id,
        source_wave_run_id,
        source_wave_unit_id,
        organization_id
    ) REFERENCES attack_residual_risks (
        id,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id
    ) ON DELETE RESTRICT,
    CHECK (
        (
            target_wave_run_id IS NOT NULL
            AND target_wave_unit_id IS NOT NULL
            AND target_work_item_id IS NOT NULL
            AND residual_risk_id IS NULL
        )
        OR (
            target_wave_run_id IS NULL
            AND target_wave_unit_id IS NULL
            AND target_work_item_id IS NULL
            AND residual_risk_id IS NOT NULL
        )
    )
);

CREATE INDEX attack_fact_delta_decisions_source_idx
    ON attack_fact_delta_decisions(operation_id, source_wave_run_id, organization_id);
CREATE INDEX attack_wave_consolidation_members_target_idx
    ON attack_wave_consolidation_members(target_wave_run_id, organization_id);

CREATE FUNCTION reject_attack_fact_delta_decision_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'FactDelta decision is immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fact_delta_decisions_immutable
BEFORE UPDATE OR DELETE ON attack_fact_delta_decisions
FOR EACH ROW EXECUTE FUNCTION reject_attack_fact_delta_decision_change();

CREATE FUNCTION reject_attack_wave_consolidation_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'attack Wave consolidation provenance is immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_consolidations_immutable
BEFORE UPDATE OR DELETE ON attack_wave_consolidations
FOR EACH ROW EXECUTE FUNCTION reject_attack_wave_consolidation_change();

CREATE TRIGGER attack_wave_consolidation_members_immutable
BEFORE UPDATE OR DELETE ON attack_wave_consolidation_members
FOR EACH ROW EXECUTE FUNCTION reject_attack_wave_consolidation_change();

CREATE FUNCTION reject_consolidated_source_wave_change()
RETURNS trigger AS $$
DECLARE
    frozen_source_wave_run_id UUID;
BEGIN
    frozen_source_wave_run_id := CASE
        WHEN TG_TABLE_NAME = 'attack_wave_runs'
            THEN (to_jsonb(OLD) ->> 'id')::UUID
        ELSE (to_jsonb(OLD) ->> 'wave_run_id')::UUID
    END;
    IF EXISTS (
        SELECT 1 FROM attack_wave_consolidations
         WHERE attack_wave_consolidations.source_wave_run_id = frozen_source_wave_run_id
    ) THEN
        RAISE EXCEPTION 'consolidated source Wave and WaveUnits are immutable';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_runs_freeze_after_consolidation
BEFORE UPDATE OR DELETE ON attack_wave_runs
FOR EACH ROW EXECUTE FUNCTION reject_consolidated_source_wave_change();

CREATE TRIGGER attack_wave_units_freeze_after_consolidation
BEFORE UPDATE OR DELETE ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION reject_consolidated_source_wave_change();

CREATE FUNCTION enforce_fact_delta_evidence_attempt_time()
RETURNS trigger AS $$
DECLARE
    attempt_started_at TIMESTAMPTZ;
    attempt_terminal_at TIMESTAMPTZ;
    evidence_observed_at TIMESTAMPTZ;
BEGIN
    SELECT attempt.created_at, attempt.terminal_at, evidence.created_at
      INTO attempt_started_at, attempt_terminal_at, evidence_observed_at
      FROM attack_fact_deltas AS delta
      JOIN candidate_attempts AS attempt ON attempt.id=delta.source_attempt_id
      JOIN audit_log AS evidence ON evidence.id=NEW.evidence_id
     WHERE delta.id=NEW.fact_delta_id;

    IF NOT FOUND
        OR attempt_terminal_at IS NULL
        OR evidence_observed_at < attempt_started_at
        OR evidence_observed_at > attempt_terminal_at
    THEN
        RAISE EXCEPTION 'FactDelta evidence must be observed during the source Attempt';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fact_delta_evidence_attempt_time
BEFORE INSERT OR UPDATE ON attack_fact_delta_evidence
FOR EACH ROW EXECUTE FUNCTION enforce_fact_delta_evidence_attempt_time();

CREATE FUNCTION protect_decided_fact_delta_evidence()
RETURNS trigger AS $$
DECLARE
    owner_fact_delta_id UUID;
BEGIN
    owner_fact_delta_id := CASE WHEN TG_OP = 'DELETE'
        THEN OLD.fact_delta_id
        ELSE NEW.fact_delta_id
    END;
    IF EXISTS (
        SELECT 1 FROM attack_fact_delta_decisions
         WHERE fact_delta_id = owner_fact_delta_id
    ) THEN
        RAISE EXCEPTION 'decided FactDelta evidence membership is immutable';
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fact_delta_evidence_freeze_after_decision
BEFORE INSERT OR UPDATE OR DELETE ON attack_fact_delta_evidence
FOR EACH ROW EXECUTE FUNCTION protect_decided_fact_delta_evidence();

-- Rehydrate the exact canonical source row rather than trusting the proposed
-- kind/version/hash.  FactDelta currently exposes only the UUID-keyed closed
-- catalog below; prose, memory and arbitrary table names are never accepted.
CREATE FUNCTION attack_fact_delta_canonical_ref_exact(expected_fact_delta_id UUID)
RETURNS BOOLEAN AS $$
DECLARE
    delta attack_fact_deltas%ROWTYPE;
    attempt candidate_attempts%ROWTYPE;
    project_path_at_freeze TEXT;
    canonical_content JSONB;
    canonical_observed_at TIMESTAMPTZ;
BEGIN
    SELECT *
      INTO delta
      FROM attack_fact_deltas
     WHERE id = expected_fact_delta_id;
    IF NOT FOUND OR delta.canonical_ref_version <> 1 THEN
        RETURN FALSE;
    END IF;

    SELECT *
      INTO attempt
      FROM candidate_attempts
     WHERE id = delta.source_attempt_id
       AND candidate_id = delta.candidate_id
       AND operation_id = delta.operation_id
       AND scope_snapshot_id = delta.scope_snapshot_id
       AND wave_run_id = delta.wave_run_id
       AND wave_unit_id = delta.wave_unit_id
       AND organization_id = delta.organization_id
       AND target_live_id IS NOT DISTINCT FROM delta.target_live_id
       AND target_type_at_time = delta.target_type_at_time
       AND target_value_at_time = delta.target_value_at_time
       AND target_identity_hash = delta.target_identity_hash
       AND candidate_plan_hash = delta.candidate_plan_hash
       AND status IN ('verified', 'refuted', 'blocked')
       AND terminal_at IS NOT NULL;
    IF NOT FOUND THEN
        RETURN FALSE;
    END IF;

    SELECT snapshot.project_path_at_freeze
      INTO project_path_at_freeze
      FROM operation_org_scope_snapshots AS snapshot
     WHERE snapshot.id = delta.scope_snapshot_id
       AND snapshot.operation_id = delta.operation_id
       AND snapshot.sealed_at IS NOT NULL;
    IF NOT FOUND THEN
        RETURN FALSE;
    END IF;

    CASE delta.canonical_ref_kind
        WHEN 'target' THEN
            SELECT to_jsonb(target), target.updated_at
              INTO canonical_content, canonical_observed_at
              FROM targets AS target
             WHERE target.id = delta.canonical_ref_id
               AND target.organization_id = delta.organization_id
               AND target.project_path = project_path_at_freeze
               AND target.scope = 'in';
        WHEN 'target_asset' THEN
            SELECT to_jsonb(asset), asset.updated_at
              INTO canonical_content, canonical_observed_at
              FROM target_assets AS asset
              JOIN targets AS target ON target.id = asset.target_id
             WHERE asset.id = delta.canonical_ref_id
               AND target.organization_id = delta.organization_id
               AND target.project_path = project_path_at_freeze
               AND asset.project_path = project_path_at_freeze
               AND target.scope = 'in';
        WHEN 'api_endpoint' THEN
            SELECT to_jsonb(endpoint), endpoint.updated_at
              INTO canonical_content, canonical_observed_at
              FROM api_endpoints AS endpoint
              JOIN targets AS target ON target.id = endpoint.target_id
             WHERE endpoint.id = delta.canonical_ref_id
               AND target.organization_id = delta.organization_id
               AND target.project_path = project_path_at_freeze
               AND endpoint.project_path = project_path_at_freeze
               AND target.scope = 'in';
        WHEN 'directory_entry' THEN
            SELECT to_jsonb(entry), entry.updated_at
              INTO canonical_content, canonical_observed_at
              FROM directory_entries AS entry
              JOIN targets AS target ON target.id = entry.target_id
             WHERE entry.id = delta.canonical_ref_id
               AND target.organization_id = delta.organization_id
               AND target.project_path = project_path_at_freeze
               AND entry.project_path = project_path_at_freeze
               AND target.scope = 'in';
        WHEN 'js_analysis_result' THEN
            SELECT to_jsonb(result), result.updated_at
              INTO canonical_content, canonical_observed_at
              FROM js_analysis_results AS result
              JOIN targets AS target ON target.id = result.target_id
             WHERE result.id = delta.canonical_ref_id
               AND target.organization_id = delta.organization_id
               AND target.project_path = project_path_at_freeze
               AND result.project_path = project_path_at_freeze
               AND target.scope = 'in';
        WHEN 'fingerprint' THEN
            SELECT to_jsonb(fingerprint), fingerprint.updated_at
              INTO canonical_content, canonical_observed_at
              FROM fingerprints AS fingerprint
              JOIN targets AS target ON target.id = fingerprint.target_id
             WHERE fingerprint.id = delta.canonical_ref_id
               AND target.organization_id = delta.organization_id
               AND target.project_path = project_path_at_freeze
               AND fingerprint.project_path = project_path_at_freeze
               AND target.scope = 'in';
        WHEN 'attack_candidate_work_item' THEN
            SELECT jsonb_build_object(
                       'work_item_id', item.id,
                       'seed_id', item.seed_id,
                       'wave_unit_id', item.wave_unit_id,
                       'operation_id', item.operation_id,
                       'scope_snapshot_id', item.scope_snapshot_id,
                       'organization_id', item.organization_id,
                       'target_live_id', item.target_live_id,
                       'target_type_at_time', item.target_type_at_time,
                       'target_value_at_time', item.target_value_at_time,
                       'target_identity_hash', item.target_identity_hash,
                       'work_item_key', item.work_item_key,
                       'technique', seed.technique,
                       'observation_hash', seed.observation_hash,
                       'manifest_hash', wave.manifest_hash,
                       'manifest_count', wave.manifest_count,
                       'manifest_frozen_at', wave.manifest_frozen_at
                   ),
                   item.created_at
              INTO canonical_content, canonical_observed_at
              FROM attack_candidate_work_items AS item
              JOIN attack_candidate_seeds AS seed ON seed.id = item.seed_id
              JOIN attack_wave_units AS wave ON wave.id = item.wave_unit_id
              JOIN organizations AS organization ON organization.id = item.organization_id
             WHERE item.id = delta.canonical_ref_id
               AND item.operation_id = delta.operation_id
               AND item.organization_id = delta.organization_id
               AND organization.project_path = project_path_at_freeze
               AND wave.manifest_frozen_at IS NOT NULL
               AND BTRIM(COALESCE(wave.manifest_hash, '')) <> '';
        WHEN 'finding' THEN
            SELECT to_jsonb(finding), finding.updated_at
              INTO canonical_content, canonical_observed_at
              FROM findings AS finding
              JOIN targets AS target ON target.id = finding.target_id
             WHERE finding.id = delta.canonical_ref_id
               AND target.organization_id = delta.organization_id
               AND target.project_path = project_path_at_freeze
               AND finding.project_path = project_path_at_freeze
               AND target.scope = 'in';
        ELSE
            RETURN FALSE;
    END CASE;

    IF canonical_content IS NULL
       OR canonical_observed_at IS NULL
       OR canonical_observed_at > attempt.terminal_at
       OR (
           delta.delta_kind IN ('created', 'updated', 'new_surface')
           AND canonical_observed_at < attempt.created_at
       )
       OR attack_fact_delta_sha256_jsonb(canonical_content)
            IS DISTINCT FROM delta.canonical_ref_hash
    THEN
        RETURN FALSE;
    END IF;
    RETURN TRUE;
END;
$$ LANGUAGE plpgsql STABLE;

-- Validate every decision from durable rows.  This is deliberately separate
-- from the Rust consolidator: a raw transaction cannot self-author evidence,
-- semantic-dedupe or decision hashes and then enter the Wave graph.
CREATE FUNCTION attack_fact_delta_decision_material_exact(expected_fact_delta_id UUID)
RETURNS BOOLEAN AS $$
DECLARE
    delta attack_fact_deltas%ROWTYPE;
    decision attack_fact_delta_decisions%ROWTYPE;
    attempt candidate_attempts%ROWTYPE;
    evidence_ids JSONB;
    evidence_count BIGINT;
    exact_evidence_count BIGINT;
    expected_evidence_set_hash TEXT;
    expected_dedupe_hash TEXT;
    expected_decision_hash TEXT;
BEGIN
    SELECT *
      INTO delta
      FROM attack_fact_deltas
     WHERE id = expected_fact_delta_id;
    IF NOT FOUND THEN
        RETURN FALSE;
    END IF;
    SELECT *
      INTO decision
      FROM attack_fact_delta_decisions
     WHERE fact_delta_id = expected_fact_delta_id;
    IF NOT FOUND THEN
        RETURN delta.status = 'proposed';
    END IF;
    SELECT *
      INTO attempt
      FROM candidate_attempts
     WHERE id = delta.source_attempt_id;
    IF NOT FOUND OR attempt.terminal_at IS NULL THEN
        RETURN FALSE;
    END IF;

    SELECT COALESCE(jsonb_agg(link.evidence_id ORDER BY link.evidence_id), '[]'::JSONB),
           COUNT(*),
           COUNT(*) FILTER (
               WHERE attempt_link.evidence_id IS NOT NULL
                 AND evidence.created_at >= attempt.created_at
                 AND evidence.created_at <= attempt.terminal_at
           )
      INTO evidence_ids, evidence_count, exact_evidence_count
      FROM attack_fact_delta_evidence AS link
      JOIN audit_log AS evidence ON evidence.id = link.evidence_id
      LEFT JOIN candidate_attempt_evidence AS attempt_link
        ON attempt_link.attempt_id = delta.source_attempt_id
       AND attempt_link.evidence_id = link.evidence_id
       AND attempt_link.role = 'fact_delta'
     WHERE link.fact_delta_id = expected_fact_delta_id
       AND link.role = 'fact_delta';
    IF evidence_count = 0 THEN
        RETURN FALSE;
    END IF;
    expected_evidence_set_hash :=
        'sha256:' || attack_fact_delta_sha256_jsonb(evidence_ids);
    IF decision.evidence_set_hash IS DISTINCT FROM expected_evidence_set_hash
       OR decision.canonical_ref_kind IS DISTINCT FROM delta.canonical_ref_kind
       OR decision.canonical_ref_id IS DISTINCT FROM delta.canonical_ref_id
       OR decision.canonical_ref_version IS DISTINCT FROM delta.canonical_ref_version
       OR decision.proposed_ref_hash IS DISTINCT FROM delta.canonical_ref_hash
    THEN
        RETURN FALSE;
    END IF;

    expected_decision_hash := 'sha256:' || attack_fact_delta_sha256_jsonb(
        jsonb_build_object(
            'canonical_ref_hash', delta.canonical_ref_hash,
            'canonical_ref_id', delta.canonical_ref_id,
            'canonical_ref_kind', delta.canonical_ref_kind,
            'canonical_ref_version', delta.canonical_ref_version,
            'disposition', decision.disposition,
            'evidence_set_hash', expected_evidence_set_hash,
            'fact_delta_id', delta.id,
            'reason_code', decision.reason_code,
            'resolved_ref_hash', decision.resolved_ref_hash,
            'resolved_ref_version', decision.resolved_ref_version
        )
    );
    IF decision.decision_hash IS DISTINCT FROM expected_decision_hash THEN
        RETURN FALSE;
    END IF;

    IF decision.disposition = 'accepted' THEN
        IF exact_evidence_count <> evidence_count
           OR delta.status NOT IN ('accepted', 'consumed')
           OR decision.reason_code <> 'accepted'
           OR decision.resolved_ref_version <> 1
           OR decision.resolved_ref_hash IS DISTINCT FROM delta.canonical_ref_hash
           OR NOT attack_fact_delta_canonical_ref_exact(delta.id)
        THEN
            RETURN FALSE;
        END IF;
        expected_dedupe_hash := 'sha256:' || attack_fact_delta_sha256_jsonb(
            jsonb_build_object(
                'schema_version', 'attack-fact-delta-semantic-v1',
                'target_identity_hash', delta.target_identity_hash,
                'canonical_ref_kind', delta.canonical_ref_kind,
                'canonical_ref_id', delta.canonical_ref_id,
                'canonical_ref_version', delta.canonical_ref_version,
                'canonical_ref_hash', delta.canonical_ref_hash,
                'delta_kind', delta.delta_kind
            )
        );
        RETURN delta.dedupe_hash = expected_dedupe_hash;
    END IF;
    RETURN decision.disposition = 'rejected'
       AND delta.status = 'rejected'
       AND decision.reason_code <> 'accepted';
END;
$$ LANGUAGE plpgsql STABLE;

CREATE FUNCTION enforce_attack_fact_delta_decision_material()
RETURNS trigger AS $$
DECLARE
    owner_fact_delta_id UUID;
    materialized_status TEXT;
BEGIN
    IF TG_TABLE_NAME = 'attack_fact_deltas' THEN
        owner_fact_delta_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
    ELSE
        owner_fact_delta_id := CASE WHEN TG_OP = 'DELETE'
            THEN OLD.fact_delta_id
            ELSE NEW.fact_delta_id
        END;
    END IF;
    SELECT status
      INTO materialized_status
      FROM attack_fact_deltas
     WHERE id = owner_fact_delta_id;
    IF NOT FOUND OR materialized_status = 'proposed' THEN
        RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    END IF;
    IF NOT attack_fact_delta_decision_material_exact(owner_fact_delta_id) THEN
        RAISE EXCEPTION
            'FactDelta % materialized decision does not match durable semantic truth',
            owner_fact_delta_id;
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_fact_deltas_require_exact_decision_material
AFTER INSERT OR UPDATE ON attack_fact_deltas
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_fact_delta_decision_material();

CREATE CONSTRAINT TRIGGER attack_fact_delta_decisions_require_exact_material
AFTER INSERT OR UPDATE ON attack_fact_delta_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_fact_delta_decision_material();

CREATE CONSTRAINT TRIGGER attack_fact_delta_evidence_requires_exact_decision_material
AFTER INSERT OR UPDATE OR DELETE ON attack_fact_delta_evidence
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_fact_delta_decision_material();

DO $fact_delta_existing_truth$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM attack_fact_deltas
         WHERE delta_kind NOT IN ('created', 'updated', 'refuted', 'new_surface')
    ) THEN
        RAISE EXCEPTION 'existing FactDelta kind is outside the closed catalog';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM attack_fact_deltas AS delta
          LEFT JOIN attack_fact_delta_decisions AS decision
            ON decision.fact_delta_id = delta.id
         WHERE delta.status <> 'proposed'
           AND (
               decision.fact_delta_id IS NULL
               OR (delta.status IN ('accepted', 'consumed') AND decision.disposition <> 'accepted')
               OR (delta.status = 'rejected' AND decision.disposition <> 'rejected')
           )
    ) THEN
        RAISE EXCEPTION 'existing materialized FactDelta lacks one matching immutable decision';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM attack_fact_delta_evidence AS link
          JOIN attack_fact_deltas AS delta ON delta.id = link.fact_delta_id
          JOIN candidate_attempts AS attempt ON attempt.id = delta.source_attempt_id
          JOIN audit_log AS evidence ON evidence.id = link.evidence_id
         WHERE attempt.terminal_at IS NULL
            OR evidence.created_at < attempt.created_at
            OR evidence.created_at > attempt.terminal_at
    ) THEN
        RAISE EXCEPTION 'existing FactDelta evidence is outside its source Attempt interval';
    END IF;
END;
$fact_delta_existing_truth$;

ALTER TABLE attack_fact_deltas
    VALIDATE CONSTRAINT attack_fact_deltas_delta_kind_closed;

CREATE FUNCTION reject_late_attack_fact_delta_proposal()
RETURNS trigger AS $$
BEGIN
    PERFORM 1
      FROM attack_wave_runs AS source_wave
     WHERE source_wave.id = NEW.wave_run_id
       AND source_wave.operation_id = NEW.operation_id
       AND source_wave.scope_snapshot_id = NEW.scope_snapshot_id
       AND source_wave.status = 'verification'
       AND source_wave.terminal_at IS NULL
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'FactDelta proposal requires its exact nonterminal verification source Wave';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM attack_wave_consolidations AS consolidation
         WHERE consolidation.source_wave_run_id = NEW.wave_run_id
           AND consolidation.operation_id = NEW.operation_id
           AND consolidation.scope_snapshot_id = NEW.scope_snapshot_id
    ) THEN
        RAISE EXCEPTION 'consolidated source Wave rejects late FactDelta proposals';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fact_deltas_reject_late_proposal
BEFORE INSERT ON attack_fact_deltas
FOR EACH ROW EXECUTE FUNCTION reject_late_attack_fact_delta_proposal();

CREATE FUNCTION enforce_attack_fact_delta_state_transition()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'FactDelta audit row cannot be deleted';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.status <> 'proposed'
            OR NEW.accepted_at IS NOT NULL
            OR NEW.consumed_by_wave_run_id IS NOT NULL
            OR NEW.consumed_at IS NOT NULL
        THEN
            RAISE EXCEPTION 'FactDelta must start proposed';
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.source_attempt_id IS DISTINCT FROM OLD.source_attempt_id
        OR NEW.candidate_id IS DISTINCT FROM OLD.candidate_id
        OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
        OR NEW.scope_snapshot_id IS DISTINCT FROM OLD.scope_snapshot_id
        OR NEW.wave_run_id IS DISTINCT FROM OLD.wave_run_id
        OR NEW.wave_unit_id IS DISTINCT FROM OLD.wave_unit_id
        OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
        OR NEW.target_type_at_time IS DISTINCT FROM OLD.target_type_at_time
        OR NEW.target_value_at_time IS DISTINCT FROM OLD.target_value_at_time
        OR NEW.target_identity_hash IS DISTINCT FROM OLD.target_identity_hash
        OR NEW.candidate_plan_hash IS DISTINCT FROM OLD.candidate_plan_hash
        OR NEW.canonical_ref_kind IS DISTINCT FROM OLD.canonical_ref_kind
        OR NEW.canonical_ref_id IS DISTINCT FROM OLD.canonical_ref_id
        OR NEW.canonical_ref_version IS DISTINCT FROM OLD.canonical_ref_version
        OR NEW.canonical_ref_hash IS DISTINCT FROM OLD.canonical_ref_hash
        OR NEW.delta_kind IS DISTINCT FROM OLD.delta_kind
        OR NEW.dedupe_hash IS DISTINCT FROM OLD.dedupe_hash
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR (
            NEW.target_live_id IS DISTINCT FROM OLD.target_live_id
            AND NOT (OLD.target_live_id IS NOT NULL AND NEW.target_live_id IS NULL)
        )
    THEN
        RAISE EXCEPTION 'FactDelta frozen identity is immutable';
    END IF;
    IF OLD.status = 'consumed' THEN
        IF (to_jsonb(NEW) - 'target_live_id') IS DISTINCT FROM
           (to_jsonb(OLD) - 'target_live_id')
        THEN
            RAISE EXCEPTION 'consumed FactDelta terminal audit row is immutable';
        END IF;
        RETURN NEW;
    END IF;
    IF NOT (
        (NEW.status = OLD.status)
        OR (OLD.status = 'proposed' AND NEW.status IN ('accepted', 'rejected'))
        OR (OLD.status = 'accepted' AND NEW.status = 'consumed')
    ) THEN
        RAISE EXCEPTION 'invalid FactDelta status transition';
    END IF;
    IF OLD.accepted_at IS NOT NULL AND NEW.accepted_at IS DISTINCT FROM OLD.accepted_at THEN
        RAISE EXCEPTION 'FactDelta acceptance timestamp is immutable';
    END IF;
    IF OLD.status = 'proposed' AND NEW.status = 'accepted' THEN
        NEW.accepted_at := NOW();
    ELSIF OLD.status = 'accepted' AND NEW.status = 'consumed' THEN
        NEW.accepted_at := OLD.accepted_at;
        NEW.consumed_at := NOW();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fact_deltas_state_transition
BEFORE INSERT OR UPDATE OR DELETE ON attack_fact_deltas
FOR EACH ROW EXECUTE FUNCTION enforce_attack_fact_delta_state_transition();

CREATE FUNCTION enforce_attack_fact_delta_decision_status()
RETURNS trigger AS $$
DECLARE
    delta_id UUID;
    delta_status TEXT;
    decision_disposition TEXT;
BEGIN
    delta_id := COALESCE(
        NULLIF(to_jsonb(NEW) ->> 'fact_delta_id', '')::UUID,
        NULLIF(to_jsonb(NEW) ->> 'id', '')::UUID
    );
    SELECT status
      INTO delta_status
      FROM attack_fact_deltas
     WHERE id = delta_id;
    SELECT disposition
      INTO decision_disposition
      FROM attack_fact_delta_decisions
     WHERE fact_delta_id = delta_id;
    IF delta_status = 'proposed' AND decision_disposition IS NULL THEN
        RETURN NEW;
    END IF;
    IF delta_status IN ('accepted', 'consumed')
        AND decision_disposition = 'accepted'
    THEN
        RETURN NEW;
    END IF;
    IF delta_status = 'rejected' AND decision_disposition = 'rejected' THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'FactDelta materialized status requires one matching immutable decision';
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_fact_deltas_require_decision
AFTER INSERT OR UPDATE OF status ON attack_fact_deltas
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_fact_delta_decision_status();

CREATE CONSTRAINT TRIGGER attack_fact_delta_decisions_require_status
AFTER INSERT ON attack_fact_delta_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_fact_delta_decision_status();

DROP TRIGGER attack_wave_units_require_final_pass_entry ON attack_wave_units;

CREATE OR REPLACE FUNCTION enforce_attack_wave_entry_final_pass()
RETURNS trigger AS $$
BEGIN
    IF NEW.entry_consolidation_id IS NULL THEN
        IF NOT EXISTS (
            SELECT 1
              FROM stage_run_units AS source_unit
              JOIN stage_handoffs AS handoff
                ON handoff.operation_id = source_unit.operation_id
               AND handoff.scope_snapshot_id = source_unit.scope_snapshot_id
               AND handoff.organization_id = source_unit.organization_id
               AND handoff.source_stage_run_unit_id = source_unit.id
               AND handoff.deliverable_submission_id = NEW.entry_deliverable_submission_id
               AND handoff.invalidated_at IS NULL
             WHERE source_unit.id = NEW.entry_stage_run_unit_id
               AND source_unit.operation_id = NEW.operation_id
               AND source_unit.stage_execution_id = NEW.entry_stage_execution_id
               AND source_unit.scope_snapshot_id = NEW.scope_snapshot_id
               AND source_unit.organization_id = NEW.organization_id
               AND source_unit.stage_kind = NEW.entry_stage_kind
               AND source_unit.status = 'passed'
               AND source_unit.terminal_at IS NOT NULL
        ) THEN
            RAISE EXCEPTION 'attack wave entry requires exact final-passed StageRunUnit and immutable handoff';
        END IF;
    ELSIF NOT EXISTS (
        SELECT 1
          FROM attack_wave_consolidations AS consolidation
          JOIN attack_wave_runs AS target_wave
            ON target_wave.id = consolidation.target_wave_run_id
           AND target_wave.operation_id = consolidation.operation_id
           AND target_wave.scope_snapshot_id = consolidation.scope_snapshot_id
         WHERE consolidation.id = NEW.entry_consolidation_id
           AND consolidation.decision_kind = 'opened_next_wave'
           AND consolidation.target_wave_run_id = NEW.wave_run_id
           AND consolidation.operation_id = NEW.operation_id
           AND consolidation.scope_snapshot_id = NEW.scope_snapshot_id
           AND consolidation.target_generation = target_wave.generation
    ) THEN
        RAISE EXCEPTION 'attack follow-on wave entry requires exact immutable FactDelta consolidation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_wave_units_require_final_pass_entry
BEFORE INSERT OR UPDATE OF operation_id, scope_snapshot_id, organization_id,
    entry_stage_execution_id, entry_stage_run_unit_id,
    entry_deliverable_submission_id, entry_stage_kind, entry_consolidation_id
ON attack_wave_units
FOR EACH ROW EXECUTE FUNCTION enforce_attack_wave_entry_final_pass();

CREATE FUNCTION enforce_attack_wave_consolidation_graph()
RETURNS trigger AS $$
DECLARE
    graph_id UUID;
    consolidation attack_wave_consolidations%ROWTYPE;
    member_count BIGINT;
    minimum_ordinal INTEGER;
    maximum_ordinal INTEGER;
    frozen_org_count BIGINT;
    accepted_decision_count BIGINT;
BEGIN
    graph_id := COALESCE(
        NULLIF(to_jsonb(NEW) ->> 'consolidation_id', '')::UUID,
        NULLIF(to_jsonb(NEW) ->> 'id', '')::UUID
    );
    SELECT *
      INTO consolidation
      FROM attack_wave_consolidations
     WHERE id = graph_id;
    IF NOT FOUND THEN
        RETURN NEW;
    END IF;

    SELECT COUNT(*), MIN(ordinal), MAX(ordinal)
      INTO member_count, minimum_ordinal, maximum_ordinal
      FROM attack_wave_consolidation_members
     WHERE consolidation_id = graph_id;
    IF member_count <> consolidation.fact_delta_count
        OR (
            member_count > 0
            AND (minimum_ordinal <> 0 OR maximum_ordinal <> member_count - 1)
        )
    THEN
        RAISE EXCEPTION 'attack Wave consolidation member count or ordinal set is incomplete';
    END IF;

    SELECT COUNT(*)
      INTO frozen_org_count
      FROM operation_org_scope_units
     WHERE snapshot_id = consolidation.scope_snapshot_id;
    IF frozen_org_count = 0
        OR NOT EXISTS (
            SELECT 1
              FROM attack_wave_runs AS source_wave
             WHERE source_wave.id = consolidation.source_wave_run_id
               AND source_wave.operation_id = consolidation.operation_id
               AND source_wave.scope_snapshot_id = consolidation.scope_snapshot_id
               AND source_wave.generation = consolidation.source_generation
               AND source_wave.status = 'terminal'
               AND source_wave.terminal_at IS NOT NULL
               AND source_wave.row_version = consolidation.source_wave_version_after
               AND source_wave.policy_hash = consolidation.policy_hash
        )
        OR (
            SELECT COUNT(*)
              FROM attack_wave_units AS source_unit
             WHERE source_unit.wave_run_id = consolidation.source_wave_run_id
               AND source_unit.operation_id = consolidation.operation_id
               AND source_unit.scope_snapshot_id = consolidation.scope_snapshot_id
        ) <> frozen_org_count
        OR EXISTS (
            SELECT 1
              FROM operation_org_scope_units AS scope_unit
              LEFT JOIN attack_wave_units AS source_unit
                ON source_unit.wave_run_id = consolidation.source_wave_run_id
               AND source_unit.operation_id = consolidation.operation_id
               AND source_unit.scope_snapshot_id = consolidation.scope_snapshot_id
               AND source_unit.organization_id = scope_unit.organization_id
               AND source_unit.ordinal = scope_unit.ordinal
             WHERE scope_unit.snapshot_id = consolidation.scope_snapshot_id
               AND (
                   source_unit.id IS NULL
                   OR source_unit.status <> 'terminal'
                   OR source_unit.terminal_at IS NULL
                   OR NOT source_unit.review_closed
                   OR NOT source_unit.verification_closed
                   OR source_unit.consolidation_status <> 'terminal'
               )
        )
    THEN
        RAISE EXCEPTION 'attack Wave consolidation requires one terminal source unit per frozen organization';
    END IF;

    SELECT COUNT(*)
      INTO accepted_decision_count
      FROM attack_fact_delta_decisions
     WHERE operation_id = consolidation.operation_id
       AND scope_snapshot_id = consolidation.scope_snapshot_id
       AND source_wave_run_id = consolidation.source_wave_run_id
       AND disposition = 'accepted';
    IF accepted_decision_count <> consolidation.fact_delta_count
        OR EXISTS (
            SELECT 1
              FROM attack_wave_consolidation_members AS member
              LEFT JOIN attack_fact_delta_decisions AS decision
                ON decision.fact_delta_id = member.fact_delta_id
               AND decision.operation_id = member.operation_id
               AND decision.scope_snapshot_id = member.scope_snapshot_id
               AND decision.source_wave_run_id = member.source_wave_run_id
               AND decision.source_wave_unit_id = member.source_wave_unit_id
               AND decision.organization_id = member.organization_id
             WHERE member.consolidation_id = graph_id
               AND (
                   decision.fact_delta_id IS NULL
                   OR decision.disposition <> 'accepted'
                   OR NOT attack_fact_delta_decision_material_exact(member.fact_delta_id)
               )
        )
    THEN
        RAISE EXCEPTION 'attack Wave consolidation must contain the exact accepted FactDelta set';
    END IF;

    IF consolidation.decision_kind = 'opened_next_wave' THEN
        IF NOT EXISTS (
            SELECT 1
              FROM attack_wave_runs AS source_wave
              JOIN attack_wave_runs AS target_wave
                ON target_wave.id = consolidation.target_wave_run_id
               AND target_wave.operation_id = consolidation.operation_id
               AND target_wave.scope_snapshot_id = consolidation.scope_snapshot_id
               AND target_wave.generation = consolidation.target_generation
               AND target_wave.policy_hash = source_wave.policy_hash
               AND target_wave.policy_snapshot = source_wave.policy_snapshot
               AND target_wave.max_waves = source_wave.max_waves
               AND target_wave.max_candidates_total = source_wave.max_candidates_total
               AND target_wave.max_chain_depth = source_wave.max_chain_depth
               AND target_wave.max_attempts_total = source_wave.max_attempts_total
             WHERE source_wave.id = consolidation.source_wave_run_id
        )
            OR (
                SELECT COUNT(*)
                  FROM attack_wave_units AS target_unit
                 WHERE target_unit.wave_run_id = consolidation.target_wave_run_id
                   AND target_unit.operation_id = consolidation.operation_id
                   AND target_unit.scope_snapshot_id = consolidation.scope_snapshot_id
                   AND target_unit.entry_consolidation_id = graph_id
            ) <> frozen_org_count
            OR EXISTS (
                SELECT 1
                  FROM operation_org_scope_units AS scope_unit
                  LEFT JOIN attack_wave_units AS target_unit
                    ON target_unit.wave_run_id = consolidation.target_wave_run_id
                   AND target_unit.operation_id = consolidation.operation_id
                   AND target_unit.scope_snapshot_id = consolidation.scope_snapshot_id
                   AND target_unit.organization_id = scope_unit.organization_id
                   AND target_unit.ordinal = scope_unit.ordinal
                   AND target_unit.entry_consolidation_id = graph_id
                 WHERE scope_unit.snapshot_id = consolidation.scope_snapshot_id
                   AND target_unit.id IS NULL
            )
            OR EXISTS (
                SELECT 1
                  FROM attack_wave_consolidation_members AS member
                  JOIN attack_fact_deltas AS delta ON delta.id = member.fact_delta_id
                  LEFT JOIN attack_wave_units AS target_unit
                    ON target_unit.id = member.target_wave_unit_id
                   AND target_unit.wave_run_id = member.target_wave_run_id
                   AND target_unit.organization_id = member.organization_id
                  LEFT JOIN attack_candidate_work_items AS work_item
                    ON work_item.id = member.target_work_item_id
                   AND work_item.wave_unit_id = member.target_wave_unit_id
                   AND work_item.organization_id = member.organization_id
                 WHERE member.consolidation_id = graph_id
                   AND (
                       member.target_wave_run_id IS DISTINCT FROM consolidation.target_wave_run_id
                       OR target_unit.id IS NULL
                       OR work_item.id IS NULL
                       OR delta.status <> 'consumed'
                       OR delta.consumed_by_wave_run_id IS DISTINCT FROM consolidation.target_wave_run_id
                   )
            )
            OR EXISTS (
                SELECT 1
                  FROM attack_wave_units AS target_unit
                 WHERE target_unit.wave_run_id = consolidation.target_wave_run_id
                   AND target_unit.entry_consolidation_id = graph_id
                   AND (
                       (
                           EXISTS (
                               SELECT 1
                                 FROM attack_wave_consolidation_members AS member
                                WHERE member.consolidation_id = graph_id
                                  AND member.organization_id = target_unit.organization_id
                           )
                           AND (
                               target_unit.status <> 'open'
                               OR target_unit.manifest_hash IS NULL
                               OR target_unit.manifest_count <= 0
                               OR target_unit.manifest_frozen_at IS NULL
                           )
                       )
                       OR (
                           NOT EXISTS (
                               SELECT 1
                                 FROM attack_wave_consolidation_members AS member
                                WHERE member.consolidation_id = graph_id
                                  AND member.organization_id = target_unit.organization_id
                           )
                           AND (
                               target_unit.status <> 'terminal'
                               OR target_unit.terminal_at IS NULL
                               OR NOT target_unit.review_closed
                               OR NOT target_unit.verification_closed
                               OR target_unit.consolidation_status <> 'terminal'
                               OR target_unit.manifest_hash IS NOT NULL
                               OR target_unit.manifest_count <> 0
                               OR target_unit.manifest_frozen_at IS NOT NULL
                           )
                       )
                   )
            )
        THEN
            RAISE EXCEPTION 'opened attack Wave consolidation graph is incomplete';
        END IF;
    ELSIF consolidation.decision_kind = 'exhausted' THEN
        IF EXISTS (
            SELECT 1
              FROM attack_wave_consolidation_members AS member
              JOIN attack_fact_deltas AS delta ON delta.id = member.fact_delta_id
              LEFT JOIN attack_residual_risks AS residual
                ON residual.id = member.residual_risk_id
               AND residual.operation_id = consolidation.operation_id
               AND residual.scope_snapshot_id = consolidation.scope_snapshot_id
               AND residual.wave_run_id = consolidation.source_wave_run_id
               AND residual.wave_unit_id = member.source_wave_unit_id
               AND residual.organization_id = member.organization_id
             WHERE member.consolidation_id = graph_id
               AND (
                   delta.status <> 'accepted'
                   OR delta.consumed_by_wave_run_id IS NOT NULL
                   OR residual.id IS NULL
                   OR residual.policy_hash <> consolidation.policy_hash
                   OR residual.wave_count <> consolidation.wave_count
                   OR residual.candidate_count <> consolidation.candidate_count
                   OR residual.chain_depth <> consolidation.chain_depth
                   OR residual.attempt_count <> consolidation.attempt_count
                   OR NOT EXISTS (
                       SELECT 1
                         FROM attack_residual_risk_evidence AS residual_evidence
                        WHERE residual_evidence.residual_risk_id = residual.id
                          AND residual_evidence.role = 'residual'
                   )
               )
        ) THEN
            RAISE EXCEPTION 'exhausted attack Wave consolidation requires exact evidence-backed residuals';
        END IF;
    ELSIF consolidation.decision_kind = 'closed_no_delta'
        AND member_count <> 0
    THEN
        RAISE EXCEPTION 'no-delta attack Wave consolidation cannot contain members';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_wave_consolidations_require_complete_graph
AFTER INSERT ON attack_wave_consolidations
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_wave_consolidation_graph();

CREATE CONSTRAINT TRIGGER attack_wave_consolidation_members_require_complete_graph
AFTER INSERT ON attack_wave_consolidation_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_attack_wave_consolidation_graph();

CREATE FUNCTION enforce_consumed_fact_delta_consolidation_membership()
RETURNS trigger AS $$
DECLARE
    exact_membership_count BIGINT;
    total_membership_count BIGINT;
BEGIN
    IF NEW.status <> 'consumed' THEN
        RETURN NEW;
    END IF;

    SELECT COUNT(*)
      INTO total_membership_count
      FROM attack_wave_consolidation_members
     WHERE fact_delta_id = NEW.id;

    SELECT COUNT(*)
      INTO exact_membership_count
      FROM attack_wave_consolidation_members AS member
      JOIN attack_wave_consolidations AS consolidation
        ON consolidation.id = member.consolidation_id
       AND consolidation.operation_id = member.operation_id
       AND consolidation.scope_snapshot_id = member.scope_snapshot_id
       AND consolidation.source_wave_run_id = member.source_wave_run_id
       AND consolidation.decision_kind = 'opened_next_wave'
       AND consolidation.target_wave_run_id = member.target_wave_run_id
     WHERE member.fact_delta_id = NEW.id
       AND member.source_attempt_id = NEW.source_attempt_id
       AND member.candidate_id = NEW.candidate_id
       AND member.operation_id = NEW.operation_id
       AND member.scope_snapshot_id = NEW.scope_snapshot_id
       AND member.source_wave_run_id = NEW.wave_run_id
       AND member.source_wave_unit_id = NEW.wave_unit_id
       AND member.organization_id = NEW.organization_id
       AND member.target_wave_run_id = NEW.consumed_by_wave_run_id
       AND member.target_wave_unit_id IS NOT NULL
       AND member.target_work_item_id IS NOT NULL
       AND member.residual_risk_id IS NULL;

    IF total_membership_count <> 1 OR exact_membership_count <> 1 THEN
        RAISE EXCEPTION 'consumed FactDelta requires one exact opened-Wave consolidation membership';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER attack_fact_deltas_require_consolidation_membership
AFTER UPDATE OF status, consumed_by_wave_run_id ON attack_fact_deltas
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_consumed_fact_delta_consolidation_membership();
