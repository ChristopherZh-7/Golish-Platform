-- Expand the append-only verification receipt finalization vocabulary for the
-- versioned directory fingerprint witness.  The witness contains every
-- authority-checked request hop and a full-body hash, but deliberately does
-- not claim that raw response bytes were retained.

DO $$
DECLARE
    check_name TEXT;
BEGIN
    SELECT constraint_name
      INTO check_name
      FROM information_schema.check_constraints
     WHERE constraint_schema=current_schema()
       AND constraint_name IN (
           SELECT constraint_name
             FROM information_schema.constraint_column_usage
            WHERE table_schema=current_schema()
              AND table_name='verification_action_capability_receipt_finalizations'
              AND column_name='witness_completeness'
       )
     LIMIT 1;
    IF check_name IS NULL THEN
        RAISE EXCEPTION 'VERIFICATION_WITNESS_COMPLETENESS_CONSTRAINT_MISSING';
    END IF;
    EXECUTE format(
        'ALTER TABLE verification_action_capability_receipt_finalizations DROP CONSTRAINT %I',
        check_name
    );
END;
$$;

ALTER TABLE verification_action_capability_receipt_finalizations
    ADD CONSTRAINT verification_action_witness_completeness_v2
    CHECK (witness_completeness IN (
        'complete_raw','complete_fingerprint_v1','metadata_only','unknown'
    ));

-- Preserve the existing raw-witness proof path and inconclusive path.  Add a
-- narrow third path for the directory fingerprint: the exact non-raw witness
-- finalization, receipt projection and independently recomputed Oracle must all
-- agree before the execution row can become terminal.
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
                  LEFT JOIN verification_prepared_actions action
                    ON action.prepared_action_id=NEW.prepared_action_id
                  LEFT JOIN hypothesis_residual_risks residual
                    ON residual.residual_id=oracle.residual_id
                   AND residual.operation_id=oracle.operation_id
                   AND residual.organization_id=oracle.organization_id
                  LEFT JOIN verification_action_capability_receipt_finalizations finalization
                    ON finalization.action_execution_id=NEW.action_execution_id
                   AND finalization.prepared_action_id=NEW.prepared_action_id
                   AND finalization.capability_execution_receipt_id=receipt.id
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
                       (
                           action.action_kind='verify.directory_fingerprint.v1'
                           AND oracle.verdict IN ('proof','refutation')
                           AND oracle.residual_id IS NULL
                           AND oracle.precondition_validity='valid'
                           AND oracle.control_validity IN ('valid','not_required')
                           AND oracle.assessment_body->>'witness_completeness'='complete_fingerprint_v1'
                           AND oracle.assessment_body->>'recomputed_verdict'=oracle.verdict
                           AND finalization.witness_completeness='complete_fingerprint_v1'
                           AND finalization.terminal_state='succeeded'
                           AND receipt.attempt_state='succeeded'
                           AND receipt.landing_state='committed'
                           AND receipt.coverage_extent='sampled'
                           AND receipt.coverage_gap_reason='none'
                           AND receipt.reconciliation_state='consistent'
                           AND (
                               (oracle.verdict='proof' AND receipt.observation_state='found')
                               OR
                               (oracle.verdict='refutation' AND receipt.observation_state='no_match')
                           )
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
