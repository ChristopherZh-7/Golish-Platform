-- Stage-fork creation intentionally writes the adoption receipt before the
-- target operation inside one transaction. Freeze the target topology from
-- the adopted rollout pair until that target row exists, then let the deferred
-- compound foreign key verify the exact pair at commit.

CREATE OR REPLACE FUNCTION freeze_operation_contract_adoption_topology()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    SELECT stage_topology_contract INTO NEW.source_stage_topology_contract
      FROM operation_state
     WHERE operation_id=NEW.source_operation_id
     FOR SHARE;
    IF NEW.source_stage_topology_contract IS NULL THEN
        RAISE EXCEPTION 'OPERATION_CONTRACT_ADOPTION_SOURCE_TOPOLOGY_MISSING'
            USING ERRCODE='23503';
    END IF;

    SELECT stage_topology_contract INTO NEW.target_stage_topology_contract
      FROM operation_state
     WHERE operation_id=NEW.target_operation_id
     FOR SHARE;
    IF NEW.target_stage_topology_contract IS NULL THEN
        NEW.target_stage_topology_contract :=
            stage_topology_for_investigation_rollout(
                NEW.target_investigation_rollout_mode
            );
    END IF;
    IF NEW.target_stage_topology_contract IS NULL THEN
        RAISE EXCEPTION 'OPERATION_CONTRACT_ADOPTION_TARGET_TOPOLOGY_UNKNOWN'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

ALTER TABLE operation_contract_adoptions
    DROP CONSTRAINT operation_contract_adoptions_target_topology_fk,
    ADD CONSTRAINT operation_contract_adoptions_target_topology_fk
        FOREIGN KEY(target_operation_id,target_stage_topology_contract)
        REFERENCES operation_state(operation_id,stage_topology_contract)
        ON DELETE RESTRICT
        DEFERRABLE INITIALLY DEFERRED;
