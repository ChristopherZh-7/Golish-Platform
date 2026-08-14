-- Durable hand-off between Verification FactDelta consolidation and the
-- successor Analysis/Registry reducer.  A row remains immutable after an
-- `advanced` consolidation receipt is written: the receipt is the terminal
-- projection, while this row preserves the exact source authority.

CREATE TABLE hypothesis_pending_evolution_authorities (
    pending_evolution_authority_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    consolidation_batch_id UUID NOT NULL UNIQUE
        REFERENCES hypothesis_consolidation_batches(consolidation_batch_id)
        ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    source_generation_id UUID NOT NULL,
    source_wave_denominator_id UUID NOT NULL,
    wave_coverage_receipt_id UUID NOT NULL,
    fact_delta_member_count BIGINT NOT NULL CHECK (fact_delta_member_count>0),
    applied_fact_delta_set_hash TEXT NOT NULL
        CHECK (applied_fact_delta_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    residual_set_hash TEXT NOT NULL
        CHECK (residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_snapshot_hash TEXT NOT NULL
        CHECK (source_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(
        pending_evolution_authority_id,consolidation_batch_id,operation_id,
        project_scope_id,organization_id,source_generation_id,
        source_wave_denominator_id,wave_coverage_receipt_id
    ),
    FOREIGN KEY(source_generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(
        source_wave_denominator_id,operation_id,project_scope_id,organization_id
    ) REFERENCES verification_wave_coverage_denominators(
        wave_denominator_id,operation_id,project_scope_id,organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(wave_coverage_receipt_id,source_wave_denominator_id)
        REFERENCES verification_wave_coverage_receipts(
            wave_coverage_receipt_id,wave_denominator_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE FUNCTION verification_guard_pending_evolution_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    batch hypothesis_consolidation_batches%ROWTYPE;
    wave verification_wave_coverage_denominators%ROWTYPE;
    applied_count BIGINT;
    actual_applied_set_hash TEXT;
BEGIN
    SELECT * INTO batch
      FROM hypothesis_consolidation_batches
     WHERE consolidation_batch_id=NEW.consolidation_batch_id;
    IF NOT FOUND
       OR batch.sealed_at IS NULL
       OR batch.operation_id<>NEW.operation_id
       OR batch.project_scope_id<>NEW.project_scope_id
       OR batch.organization_id<>NEW.organization_id
       OR batch.generation_id<>NEW.source_generation_id
       OR batch.wave_coverage_receipt_id<>NEW.wave_coverage_receipt_id
       OR batch.fact_delta_member_count<>NEW.fact_delta_member_count
       OR batch.source_snapshot_hash<>NEW.source_snapshot_hash
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_PENDING_EVOLUTION_BATCH_AUTHORITY_INVALID'
            USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1 FROM hypothesis_consolidation_receipts receipt
         WHERE receipt.consolidation_batch_id=NEW.consolidation_batch_id
    ) THEN
        RAISE EXCEPTION 'HYPOTHESIS_PENDING_EVOLUTION_ALREADY_CONSOLIDATED'
            USING ERRCODE='23514';
    END IF;

    SELECT * INTO wave
      FROM verification_wave_coverage_denominators
     WHERE wave_denominator_id=NEW.source_wave_denominator_id;
    IF NOT FOUND
       OR wave.sealed_at IS NULL
       OR wave.operation_id<>NEW.operation_id
       OR wave.project_scope_id<>NEW.project_scope_id
       OR wave.organization_id<>NEW.organization_id
       OR wave.source_snapshot_hash<>NEW.source_snapshot_hash
       OR NOT EXISTS(
            SELECT 1
              FROM hypothesis_generation_seals generation_seal
             WHERE generation_seal.seal_id=wave.generation_seal_id
               AND generation_seal.generation_id=NEW.source_generation_id
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_PENDING_EVOLUTION_WAVE_AUTHORITY_INVALID'
            USING ERRCODE='23514';
    END IF;

    SELECT COUNT(*),
           investigation_exact_member_set_hash(
               'hypothesis_consolidation_consumptions.v1',
               COALESCE(array_agg(
                   consumption.consumption_hash
                   ORDER BY consumption.consumption_hash
               ),ARRAY[]::TEXT[])
           )
      INTO applied_count,actual_applied_set_hash
      FROM fact_delta_consumptions consumption
      JOIN verification_fact_delta_bundles delta
        ON delta.fact_delta_bundle_id=consumption.fact_delta_bundle_id
       AND delta.operation_id=consumption.operation_id
       AND delta.project_scope_id=consumption.project_scope_id
       AND delta.organization_id=consumption.organization_id
      JOIN verification_campaigns campaign
        ON campaign.campaign_id=delta.campaign_id
       AND campaign.operation_id=delta.operation_id
     WHERE consumption.operation_id=NEW.operation_id
       AND consumption.project_scope_id=NEW.project_scope_id
       AND consumption.organization_id=NEW.organization_id
       AND consumption.generation_id=NEW.source_generation_id
       AND campaign.wave_denominator_id=NEW.source_wave_denominator_id;
    IF applied_count<>NEW.fact_delta_member_count
       OR actual_applied_set_hash<>NEW.applied_fact_delta_set_hash
       OR NOT EXISTS(
            SELECT 1
              FROM fact_delta_consumptions consumption
             WHERE consumption.operation_id=NEW.operation_id
               AND consumption.project_scope_id=NEW.project_scope_id
               AND consumption.organization_id=NEW.organization_id
               AND consumption.generation_id=NEW.source_generation_id
               AND consumption.disposition='applied'
               AND EXISTS(
                    SELECT 1
                      FROM verification_fact_delta_bundles delta
                      JOIN verification_campaigns campaign
                        ON campaign.campaign_id=delta.campaign_id
                       AND campaign.wave_denominator_id=NEW.source_wave_denominator_id
                     WHERE delta.fact_delta_bundle_id=consumption.fact_delta_bundle_id
                       AND delta.operation_id=consumption.operation_id
                       AND delta.project_scope_id=consumption.project_scope_id
                       AND delta.organization_id=consumption.organization_id
               )
       )
    THEN
        RAISE EXCEPTION 'HYPOTHESIS_PENDING_EVOLUTION_CONSUMPTION_AUTHORITY_INVALID'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER hypothesis_pending_evolution_authority_guard
BEFORE INSERT ON hypothesis_pending_evolution_authorities
FOR EACH ROW EXECUTE FUNCTION verification_guard_pending_evolution_authority();

CREATE TRIGGER hypothesis_pending_evolution_authorities_append_only
BEFORE UPDATE OR DELETE ON hypothesis_pending_evolution_authorities
FOR EACH ROW EXECUTE FUNCTION verification_reject_append_only();

CREATE INDEX hypothesis_pending_evolution_authorities_open_lookup
ON hypothesis_pending_evolution_authorities(
    operation_id,organization_id,source_generation_id,
    pending_evolution_authority_id
);
