-- Canonical receipt for an evolution Analysis that inspected a pending
-- Verification FactDelta batch and produced no new semantic revision.  The
-- compiler closes that source batch at a real fixed point without inventing
-- an empty successor generation.

CREATE TABLE investigation_evolution_fixed_point_apply_receipts (
    evolution_fixed_point_apply_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    decision_id UUID NOT NULL UNIQUE,
    pending_evolution_authority_id UUID NOT NULL UNIQUE,
    consolidation_batch_id UUID NOT NULL,
    consolidation_receipt_id UUID NOT NULL UNIQUE,
    fixed_point_receipt_id UUID NOT NULL UNIQUE,
    source_generation_id UUID NOT NULL,
    source_wave_denominator_id UUID NOT NULL,
    wave_coverage_receipt_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    receipt_sha256 TEXT NOT NULL CHECK(receipt_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(evolution_fixed_point_apply_receipt_id,operation_id,organization_id),
    FOREIGN KEY(decision_id,operation_id,organization_id)
        REFERENCES investigation_hypothesis_compilation_decisions(
            decision_id,operation_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(
        pending_evolution_authority_id,consolidation_batch_id,operation_id,
        project_scope_id,organization_id,source_generation_id,
        source_wave_denominator_id,wave_coverage_receipt_id
    ) REFERENCES hypothesis_pending_evolution_authorities(
        pending_evolution_authority_id,consolidation_batch_id,operation_id,
        project_scope_id,organization_id,source_generation_id,
        source_wave_denominator_id,wave_coverage_receipt_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(consolidation_receipt_id)
        REFERENCES hypothesis_consolidation_receipts(consolidation_receipt_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(fixed_point_receipt_id)
        REFERENCES hypothesis_fixed_point_receipts(fixed_point_receipt_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(source_generation_id,operation_id,organization_id)
        REFERENCES hypothesis_generations(generation_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_guard_evolution_fixed_point_apply_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS(
        SELECT 1
          FROM hypothesis_consolidation_receipts consolidation
          JOIN hypothesis_fixed_point_receipts fixed
            ON fixed.fixed_point_receipt_id=NEW.fixed_point_receipt_id
           AND fixed.consolidation_receipt_id=consolidation.consolidation_receipt_id
           AND fixed.generation_id=NEW.source_generation_id
         WHERE consolidation.consolidation_receipt_id=NEW.consolidation_receipt_id
           AND consolidation.consolidation_batch_id=NEW.consolidation_batch_id
           AND consolidation.disposition='fixed_point'
           AND consolidation.successor_generation_id IS NULL
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_EVOLUTION_FIXED_POINT_AUTHORITY_INVALID'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_evolution_fixed_point_apply_receipt_guard
BEFORE INSERT ON investigation_evolution_fixed_point_apply_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_guard_evolution_fixed_point_apply_receipt();

CREATE TRIGGER investigation_evolution_fixed_point_apply_receipts_append_only
BEFORE UPDATE OR DELETE ON investigation_evolution_fixed_point_apply_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
