-- Generalize the action terminal commit marker from the metadata-only V1
-- Oracle to every typed Oracle verdict. The receipt, Oracle and (for an
-- inconclusive verdict) exact residual must already exist in the same commit.

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
                  LEFT JOIN hypothesis_residual_risks residual
                    ON residual.residual_id=oracle.residual_id
                   AND residual.operation_id=oracle.operation_id
                   AND residual.organization_id=oracle.organization_id
                 WHERE receipt.id=NEW.capability_execution_receipt_id
                   AND receipt.finalized_at IS NOT NULL
                   AND receipt.attempt_state IN ('succeeded','failed','outcome_unknown')
                   AND (
                       (
                           oracle.verdict IN ('proof','refutation')
                           AND oracle.residual_id IS NULL
                           AND receipt.attempt_state='succeeded'
                           AND receipt.landing_state='committed'
                           AND receipt.observation_state IN ('found','no_match')
                           AND receipt.coverage_extent='complete'
                           AND receipt.coverage_gap_reason='none'
                           AND receipt.reconciliation_state='consistent'
                           AND receipt.raw_witness_artifact_id IS NOT NULL
                           AND receipt.parser_census_id IS NOT NULL
                           AND receipt.temporal_census_id IS NOT NULL
                       )
                       OR
                       (oracle.verdict='inconclusive' AND residual.residual_id IS NOT NULL)
                   )
           )
        THEN
            RAISE EXCEPTION 'VERIFICATION_ACTION_ORACLE_LANDING_REQUIRED';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
