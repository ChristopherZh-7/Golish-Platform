-- Candidate FactDelta follow-on route contract.
--
-- Keep fact-change semantics, observation semantics, verifier allowlists and
-- enrichment state as four independent immutable axes.  This is an expand-only
-- migration: historical seeds remain readable through compatibility defaults,
-- while every new FactDelta-backed seed is tied to its exact accepted delta.

ALTER TABLE attack_fact_deltas
    ADD CONSTRAINT attack_fact_deltas_seed_owner_unique UNIQUE (
        id, operation_id, scope_snapshot_id, organization_id
    );

ALTER TABLE attack_candidate_seeds
    ADD COLUMN source_fact_delta_id UUID,
    ADD COLUMN delta_kind TEXT,
    ADD COLUMN observation_kind TEXT NOT NULL DEFAULT 'legacy_observation',
    ADD COLUMN allowed_techniques TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    ADD COLUMN enrichment_required BOOLEAN NOT NULL DEFAULT FALSE,
    ADD CONSTRAINT attack_candidate_seeds_fact_delta_route_shape CHECK (
        BTRIM(observation_kind) <> ''
        AND OCTET_LENGTH(observation_kind) <= 128
        AND CARDINALITY(allowed_techniques) <= 10
        AND ARRAY_POSITION(allowed_techniques, NULL) IS NULL
        AND (
            (
                source_fact_delta_id IS NULL
                AND delta_kind IS NULL
                AND NOT enrichment_required
            )
            OR (
                source_fact_delta_id IS NOT NULL
                AND delta_kind IN ('created', 'updated', 'new_surface')
                AND CARDINALITY(allowed_techniques) > 0
                AND observation->>'fact_delta_id' = source_fact_delta_id::TEXT
                AND observation->>'delta_kind' = delta_kind
                AND observation->>'observation_kind' = observation_kind
                AND (observation->>'enrichment_required')::BOOLEAN = enrichment_required
                AND observation->'allowed_techniques' = TO_JSONB(allowed_techniques)
                AND (
                    (
                        enrichment_required
                        AND observation_kind = 'surface_analysis_v2'
                        AND observation->>'schema' = 'surface_analysis_v2'
                        AND technique = 'GOLISH-SURFACE-ANALYSIS'
                    )
                    OR (
                        NOT enrichment_required
                        AND observation->>'schema' = observation_kind
                        AND CARDINALITY(allowed_techniques) = 1
                        AND allowed_techniques[1] = technique
                    )
                )
            )
        )
    ),
    ADD CONSTRAINT attack_candidate_seeds_source_fact_delta_unique
        UNIQUE (source_fact_delta_id),
    ADD CONSTRAINT attack_candidate_seeds_source_fact_delta_owner_fk FOREIGN KEY (
        source_fact_delta_id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) REFERENCES attack_fact_deltas (
        id,
        operation_id,
        scope_snapshot_id,
        organization_id
    ) ON DELETE RESTRICT;

CREATE TRIGGER attack_candidate_seeds_fact_delta_route_immutable
BEFORE UPDATE OF source_fact_delta_id,delta_kind,observation_kind,
    allowed_techniques,enrichment_required
ON attack_candidate_seeds
FOR EACH ROW EXECUTE FUNCTION reject_frozen_attack_manifest_row_change();

-- The current runtime has no safe observation-replacement executor.  An
-- insufficient FactDelta therefore becomes an explicit pending enrichment
-- authority while the source Wave and delta remain unadvanced.  A future
-- executor can resolve this immutable request through an additive result table;
-- it must never be represented as an unexecutable Candidate seed.
CREATE TABLE attack_fact_delta_enrichment_items (
    id UUID PRIMARY KEY,
    fact_delta_id UUID NOT NULL UNIQUE,
    source_attempt_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    source_wave_run_id UUID NOT NULL,
    source_wave_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    delta_kind TEXT NOT NULL CHECK (delta_kind IN ('created', 'updated', 'new_surface')),
    observation_kind TEXT NOT NULL CHECK (observation_kind = 'surface_analysis_v2'),
    allowed_techniques TEXT[] NOT NULL CHECK (
        CARDINALITY(allowed_techniques) > 0
        AND CARDINALITY(allowed_techniques) <= 10
        AND ARRAY_POSITION(allowed_techniques, NULL) IS NULL
    ),
    enrichment_required BOOLEAN NOT NULL CHECK (enrichment_required),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status = 'pending'),
    request JSONB NOT NULL CHECK (
        jsonb_typeof(request) = 'object'
        AND request->>'schema' = observation_kind
        AND request->>'fact_delta_id' = fact_delta_id::TEXT
        AND request->>'delta_kind' = delta_kind
        AND request->>'observation_kind' = observation_kind
        AND request->>'enrichment_required' = 'true'
        AND request->'allowed_techniques' = TO_JSONB(allowed_techniques)
        AND PG_COLUMN_SIZE(request) <= 65536
    ),
    request_hash TEXT NOT NULL CHECK (BTRIM(request_hash) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
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
    ) ON DELETE RESTRICT
);

CREATE FUNCTION reject_attack_fact_delta_enrichment_change()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'pending FactDelta enrichment authority is immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_fact_delta_enrichment_items_immutable
BEFORE UPDATE OR DELETE ON attack_fact_delta_enrichment_items
FOR EACH ROW EXECUTE FUNCTION reject_attack_fact_delta_enrichment_change();

CREATE INDEX attack_fact_delta_enrichment_items_source_idx
    ON attack_fact_delta_enrichment_items(
        operation_id, source_wave_run_id, organization_id, status
    );

ALTER TABLE attack_wave_consolidation_members
    ADD COLUMN route_kind TEXT;

UPDATE attack_wave_consolidation_members
   SET route_kind = CASE
       WHEN residual_risk_id IS NOT NULL THEN 'residual'
       ELSE 'direct'
   END;

ALTER TABLE attack_wave_consolidation_members
    ALTER COLUMN route_kind SET NOT NULL,
    ADD CONSTRAINT attack_wave_consolidation_members_route_kind_check CHECK (
        route_kind IN ('direct', 'enrichment', 'no_attack', 'residual')
    );

DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    SELECT con.conname
      INTO constraint_name
      FROM pg_constraint AS con
     WHERE con.conrelid = 'attack_wave_consolidation_members'::REGCLASS
       AND con.contype = 'c'
       AND PG_GET_CONSTRAINTDEF(con.oid) LIKE '%target_work_item_id IS NOT NULL%'
       AND PG_GET_CONSTRAINTDEF(con.oid) LIKE '%residual_risk_id IS NOT NULL%'
     LIMIT 1;
    IF constraint_name IS NOT NULL THEN
        EXECUTE FORMAT(
            'ALTER TABLE attack_wave_consolidation_members DROP CONSTRAINT %I',
            constraint_name
        );
    END IF;
END;
$$;

ALTER TABLE attack_wave_consolidation_members
    ADD CONSTRAINT attack_wave_consolidation_members_route_shape_check CHECK (
        (
            route_kind IN ('direct', 'enrichment')
            AND target_wave_run_id IS NOT NULL
            AND target_wave_unit_id IS NOT NULL
            AND target_work_item_id IS NOT NULL
            AND residual_risk_id IS NULL
        )
        OR (
            route_kind = 'no_attack'
            AND target_wave_run_id IS NULL
            AND target_wave_unit_id IS NULL
            AND target_work_item_id IS NULL
            AND residual_risk_id IS NULL
        )
        OR (
            route_kind = 'residual'
            AND target_wave_run_id IS NULL
            AND target_wave_unit_id IS NULL
            AND target_work_item_id IS NULL
            AND residual_risk_id IS NOT NULL
        )
    );

DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    SELECT con.conname
      INTO constraint_name
      FROM pg_constraint AS con
     WHERE con.conrelid = 'attack_wave_consolidations'::REGCLASS
       AND con.contype = 'c'
       AND PG_GET_CONSTRAINTDEF(con.oid) LIKE '%decision_kind%opened_next_wave%'
       AND PG_GET_CONSTRAINTDEF(con.oid) LIKE '%no_accepted_fact_delta%'
     LIMIT 1;
    IF constraint_name IS NOT NULL THEN
        EXECUTE FORMAT(
            'ALTER TABLE attack_wave_consolidations DROP CONSTRAINT %I',
            constraint_name
        );
    END IF;
END;
$$;

ALTER TABLE attack_wave_consolidations
    ADD CONSTRAINT attack_wave_consolidations_decision_shape_v2 CHECK (
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
            AND (
                (fact_delta_count = 0 AND reason_code = 'no_accepted_fact_delta')
                OR (
                    fact_delta_count > 0
                    AND reason_code = 'accepted_refutation_without_attack_follow_on'
                )
            )
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
    );

CREATE OR REPLACE FUNCTION enforce_attack_wave_consolidation_graph()
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
    SELECT * INTO consolidation
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

    SELECT COUNT(*) INTO frozen_org_count
      FROM operation_org_scope_units
     WHERE snapshot_id = consolidation.scope_snapshot_id;
    IF frozen_org_count = 0
       OR NOT EXISTS (
           SELECT 1 FROM attack_wave_runs AS source_wave
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
           SELECT COUNT(*) FROM attack_wave_units AS source_unit
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

    SELECT COUNT(*) INTO accepted_decision_count
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

    IF EXISTS (
        SELECT 1
          FROM attack_wave_consolidation_members AS member
          JOIN attack_fact_deltas AS delta ON delta.id = member.fact_delta_id
         WHERE member.consolidation_id = graph_id
           AND (
               (member.route_kind = 'no_attack' AND (
                   delta.delta_kind <> 'refuted'
                   OR delta.status <> 'accepted'
                   OR delta.consumed_by_wave_run_id IS NOT NULL
               ))
               OR (member.route_kind <> 'no_attack' AND delta.delta_kind = 'refuted')
           )
    ) THEN
        RAISE EXCEPTION 'FactDelta consolidation route kind does not match delta semantics';
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
           OR NOT EXISTS (
               SELECT 1 FROM attack_wave_consolidation_members
                WHERE consolidation_id = graph_id
                  AND route_kind IN ('direct', 'enrichment')
           )
           OR EXISTS (
               SELECT 1 FROM attack_wave_consolidation_members
                WHERE consolidation_id = graph_id
                  AND route_kind = 'residual'
           )
           OR (
               SELECT COUNT(*) FROM attack_wave_units AS target_unit
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
            LEFT JOIN attack_candidate_work_items AS work_item
                   ON work_item.id = member.target_work_item_id
                  AND work_item.wave_unit_id = member.target_wave_unit_id
                  AND work_item.organization_id = member.organization_id
            LEFT JOIN attack_candidate_seeds AS seed ON seed.id = work_item.seed_id
                WHERE member.consolidation_id = graph_id
                  AND member.route_kind IN ('direct', 'enrichment')
                  AND (
                      member.target_wave_run_id IS DISTINCT FROM consolidation.target_wave_run_id
                      OR work_item.id IS NULL
                      OR seed.id IS NULL
                      OR seed.source_fact_delta_id IS DISTINCT FROM delta.id
                      OR seed.delta_kind IS DISTINCT FROM delta.delta_kind
                      OR seed.observation_kind IS NULL
                      OR CARDINALITY(seed.allowed_techniques) = 0
                      OR seed.enrichment_required IS DISTINCT FROM
                          (member.route_kind = 'enrichment')
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
                              SELECT 1 FROM attack_wave_consolidation_members AS member
                               WHERE member.consolidation_id = graph_id
                                 AND member.organization_id = target_unit.organization_id
                                 AND member.route_kind IN ('direct', 'enrichment')
                          )
                          AND (
                              target_unit.status <> 'open'
                              OR target_unit.manifest_hash IS NULL
                              OR target_unit.manifest_count <> (
                                  SELECT COUNT(*) FROM attack_wave_consolidation_members AS member
                                   WHERE member.consolidation_id = graph_id
                                     AND member.organization_id = target_unit.organization_id
                                     AND member.route_kind IN ('direct', 'enrichment')
                              )
                              OR target_unit.manifest_frozen_at IS NULL
                          )
                      )
                      OR (
                          NOT EXISTS (
                              SELECT 1 FROM attack_wave_consolidation_members AS member
                               WHERE member.consolidation_id = graph_id
                                 AND member.organization_id = target_unit.organization_id
                                 AND member.route_kind IN ('direct', 'enrichment')
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
        IF NOT EXISTS (
            SELECT 1 FROM attack_wave_consolidation_members
             WHERE consolidation_id = graph_id AND route_kind = 'residual'
        )
           OR EXISTS (
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
                  AND member.route_kind = 'residual'
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
                          SELECT 1 FROM attack_residual_risk_evidence AS residual_evidence
                           WHERE residual_evidence.residual_risk_id = residual.id
                             AND residual_evidence.role = 'residual'
                      )
                  )
           )
           OR EXISTS (
               SELECT 1 FROM attack_wave_consolidation_members
                WHERE consolidation_id = graph_id
                  AND route_kind NOT IN ('residual', 'no_attack')
           )
        THEN
            RAISE EXCEPTION 'exhausted attack Wave consolidation requires exact evidence-backed residuals';
        END IF;
    ELSIF consolidation.decision_kind = 'closed_no_delta' THEN
        IF EXISTS (
            SELECT 1 FROM attack_wave_consolidation_members
             WHERE consolidation_id = graph_id AND route_kind <> 'no_attack'
        ) THEN
            RAISE EXCEPTION 'closed attack Wave consolidation may contain only no-attack refutations';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION enforce_consumed_fact_delta_consolidation_membership()
RETURNS trigger AS $$
DECLARE
    exact_membership_count BIGINT;
    total_membership_count BIGINT;
BEGIN
    IF NEW.status <> 'consumed' THEN
        RETURN NEW;
    END IF;
    SELECT COUNT(*) INTO total_membership_count
      FROM attack_wave_consolidation_members
     WHERE fact_delta_id = NEW.id;
    SELECT COUNT(*) INTO exact_membership_count
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
       AND member.residual_risk_id IS NULL
       AND member.route_kind IN ('direct', 'enrichment');
    IF total_membership_count <> 1 OR exact_membership_count <> 1 THEN
        RAISE EXCEPTION 'consumed FactDelta requires one exact routed opened-Wave consolidation membership';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
