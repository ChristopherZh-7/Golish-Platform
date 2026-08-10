-- A terminal action is the commit marker for its semantic landing. The
-- executor must durably record the raw-witness residual and inconclusive
-- Oracle first; while either is missing the action remains `started` and is
-- safely re-entered by the scheduler after response loss.

CREATE OR REPLACE FUNCTION enforce_verification_action_oracle_commit_marker()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.state='started' AND NEW.state IN ('succeeded','failed','outcome_unknown') THEN
        IF NEW.capability_execution_receipt_id IS NULL
           OR NOT EXISTS(
                SELECT 1
                  FROM capability_execution_receipts receipt
                  JOIN verification_oracle_assessments oracle
                    ON oracle.action_execution_id=NEW.action_execution_id
                   AND oracle.prepared_action_id=NEW.prepared_action_id
                   AND oracle.observation_receipt_hash=receipt.receipt_authority_hash
                   AND oracle.verdict='inconclusive'
                  JOIN hypothesis_residual_risks residual
                    ON residual.residual_id=oracle.residual_id
                   AND residual.reason_code='raw_witness_incomplete'
                 WHERE receipt.id=NEW.capability_execution_receipt_id
                   AND receipt.finalized_at IS NOT NULL
                   AND receipt.attempt_state IN ('succeeded','failed','outcome_unknown')
           )
        THEN
            RAISE EXCEPTION 'VERIFICATION_ACTION_ORACLE_LANDING_REQUIRED';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER verification_action_oracle_commit_marker
BEFORE UPDATE ON verification_action_executions
FOR EACH ROW EXECUTE FUNCTION enforce_verification_action_oracle_commit_marker();
