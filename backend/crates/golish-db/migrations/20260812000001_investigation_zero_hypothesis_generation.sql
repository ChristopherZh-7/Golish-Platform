-- A sealed Investigation Primary may conclude that the frozen evidence does
-- not support any proof-bound hypothesis.  Preserve that conclusion as an
-- exact empty compiler/generation/admission census instead of forcing the
-- cognitive layer to invent a proposal.

DO $migration$
DECLARE
    constraint_name TEXT;
BEGIN
    FOR constraint_name IN
        SELECT constraint_row.conname
          FROM pg_constraint constraint_row
          JOIN pg_attribute attribute_row
            ON attribute_row.attrelid=constraint_row.conrelid
           AND attribute_row.attnum=constraint_row.conkey[1]
         WHERE constraint_row.conrelid=
                   'investigation_hypothesis_compilation_decisions'::REGCLASS
           AND constraint_row.contype='c'
           AND array_length(constraint_row.conkey,1)=1
           AND attribute_row.attname IN (
               'proposal_count','proof_member_count','mutation_count'
           )
    LOOP
        EXECUTE format(
            'ALTER TABLE investigation_hypothesis_compilation_decisions DROP CONSTRAINT %I',
            constraint_name
        );
    END LOOP;
END;
$migration$;

ALTER TABLE investigation_hypothesis_compilation_decisions
    ADD CONSTRAINT investigation_compilation_proposal_count_nonnegative
        CHECK(proposal_count>=0),
    ADD CONSTRAINT investigation_compilation_proof_count_covers_proposals
        CHECK(proof_member_count>=proposal_count),
    ADD CONSTRAINT investigation_compilation_mutation_count_exact
        CHECK(mutation_count=proposal_count),
    ADD CONSTRAINT investigation_compilation_zero_proposal_actions_empty
        CHECK(proposal_count>0 OR action_intent_count=0);
