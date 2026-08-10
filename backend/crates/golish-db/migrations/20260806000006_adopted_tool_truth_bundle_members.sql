-- A stage-run fork is a new operation, but its strict predecessor prefix is
-- represented by immutable operation_stage_fork_inputs.  Permit a target
-- consumer bundle to reference a source Tool Truth root only when that exact
-- adopted stage handoff proves the cross-operation lineage.
CREATE OR REPLACE FUNCTION tool_truth_validate_authority_bundle_member()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE parent_sealed TIMESTAMPTZ;
DECLARE parent_operation UUID;
DECLARE parent_scope_snapshot UUID;
DECLARE parent_organization UUID;
DECLARE set_sealed TIMESTAMPTZ;
DECLARE stage TEXT;
DECLARE root_operation UUID;
DECLARE root_organization UUID;
BEGIN
    IF TG_OP<>'INSERT' THEN
        RAISE EXCEPTION 'tool_truth_member_append_only' USING ERRCODE='23514';
    END IF;
    SELECT sealed_at,operation_id,scope_snapshot_id,organization_id
      INTO parent_sealed,parent_operation,parent_scope_snapshot,parent_organization
      FROM tool_truth_authority_bundle_seals
     WHERE id=NEW.bundle_seal_id FOR SHARE;
    IF parent_sealed IS NOT NULL THEN
        RAISE EXCEPTION 'tool_truth_sealed_parent_immutable' USING ERRCODE='23514';
    END IF;
    IF parent_operation IS DISTINCT FROM NEW.operation_id
       OR parent_organization IS DISTINCT FROM NEW.organization_id THEN
        RAISE EXCEPTION 'tool_truth_authority_bundle_scope_mismatch' USING ERRCODE='23514';
    END IF;
    SELECT s.sealed_at,a.stage_kind,a.operation_id,a.organization_id
      INTO set_sealed,stage,root_operation,root_organization
      FROM tool_truth_authority_set_seals s
      JOIN tool_truth_execution_authorities a ON a.id=s.execution_authority_id
     WHERE s.id=NEW.authority_set_seal_id
       AND s.execution_authority_id=NEW.root_execution_authority_id
       AND s.denominator_id=NEW.root_denominator_id FOR SHARE;
    IF set_sealed IS NULL THEN
        RAISE EXCEPTION 'tool_truth_unsealed_authority' USING ERRCODE='23514';
    END IF;
    IF root_organization<>NEW.organization_id THEN
        RAISE EXCEPTION 'tool_truth_authority_bundle_scope_mismatch' USING ERRCODE='23514';
    END IF;
    IF root_operation<>NEW.operation_id AND NOT EXISTS (
        SELECT 1
          FROM operation_stage_forks fork
          JOIN operation_stage_fork_inputs input
            ON input.operation_id=fork.operation_id
           AND input.source_operation_id=fork.source_operation_id
           AND input.source_scope_snapshot_id=fork.source_scope_snapshot_id
           AND input.organization_id=NEW.organization_id
           AND input.source_stage_kind=stage
          JOIN operation_state source_operation
            ON source_operation.operation_id=input.source_operation_id
           AND source_operation.superseded_by IS NULL
          JOIN stage_handoffs source_handoff
            ON source_handoff.id=input.source_handoff_id
           AND source_handoff.operation_id=input.source_operation_id
           AND source_handoff.organization_id=input.organization_id
           AND source_handoff.from_stage_kind=input.source_stage_kind
           AND source_handoff.invalidated_at IS NULL
         WHERE fork.operation_id=NEW.operation_id
           AND fork.target_scope_snapshot_id=parent_scope_snapshot
           AND input.source_operation_id=root_operation
    ) THEN
        RAISE EXCEPTION 'tool_truth_authority_bundle_scope_mismatch' USING ERRCODE='23514';
    END IF;
    IF (NEW.root_family='ti' AND stage<>'target_intel')
       OR (NEW.root_family='eas' AND stage<>'external_attack_surface')
       OR (NEW.root_family='enum' AND stage<>'enumeration')
       OR (NEW.root_family='vuln' AND stage<>'vuln_triage') THEN
        RAISE EXCEPTION 'tool_truth_authority_bundle_root_family_mismatch' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION tool_truth_validate_authority_bundle_member() IS
'Allows cross-operation Tool Truth bundle members only through an exact, valid stage-fork predecessor handoff; ordinary bundles remain operation-local.';
