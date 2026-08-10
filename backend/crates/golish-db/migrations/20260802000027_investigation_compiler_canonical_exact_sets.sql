-- Compilation seals are mathematical exact sets. AI proposal order and the
-- server's semantic-key presentation order must therefore produce the same
-- authority hashes. Ordinals remain an immutable presentation sequence, but
-- are not part of the set hash.

CREATE OR REPLACE FUNCTION investigation_enforce_hypothesis_compilation_exact_sets()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    decision investigation_hypothesis_compilation_decisions%ROWTYPE;
    actual_mutation_count BIGINT;
    actual_mutation_set TEXT;
    actual_proposal_count BIGINT;
    actual_proposal_set TEXT;
    actual_proof_count BIGINT;
    actual_proof_set TEXT;
BEGIN
    SELECT * INTO STRICT decision
      FROM investigation_hypothesis_compilation_decisions
     WHERE decision_id=NEW.decision_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_hypothesis_compilation_members.v1',
               COALESCE(
                   array_agg(member_sha256 ORDER BY member_sha256 COLLATE "C"),
                   ARRAY[]::TEXT[]
               )
           )
      INTO actual_mutation_count,actual_mutation_set
      FROM investigation_hypothesis_compilation_members
     WHERE decision_id=decision.decision_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_candidate_proposals.v1',
               COALESCE(
                   array_agg(proposal_sha256 ORDER BY proposal_sha256 COLLATE "C"),
                   ARRAY[]::TEXT[]
               )
           )
      INTO actual_proposal_count,actual_proposal_set
      FROM investigation_hypothesis_compilation_members
     WHERE decision_id=decision.decision_id;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_hypothesis_compilation_proofs.v1',
               COALESCE(
                   array_agg(member_sha256 ORDER BY member_sha256 COLLATE "C"),
                   ARRAY[]::TEXT[]
               )
           )
      INTO actual_proof_count,actual_proof_set
      FROM investigation_hypothesis_compilation_proof_members
     WHERE decision_id=decision.decision_id;
    IF actual_proposal_count<>decision.proposal_count
       OR actual_proposal_set<>decision.proposal_set_sha256
       OR actual_mutation_count<>decision.mutation_count
       OR actual_mutation_set<>decision.mutation_set_sha256
       OR actual_proof_count<>decision.proof_member_count
       OR actual_proof_set<>decision.proof_member_set_sha256
    THEN
        RAISE EXCEPTION 'INVESTIGATION_COMPILER_EXACT_SET_REQUIRED' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;
