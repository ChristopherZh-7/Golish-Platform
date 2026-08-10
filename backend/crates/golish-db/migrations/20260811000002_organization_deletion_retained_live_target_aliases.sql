-- Preserve target UUIDs inside append-only or sealed historical rows.
--
-- These columns were introduced as nullable live aliases with ON DELETE SET
-- NULL. That action is still an UPDATE and therefore conflicts with the
-- tables' immutable/CAS triggers once a run is sealed. Keep the at-time UUID
-- unchanged and enforce live-target admission only for future writes through
-- the validator installed by 20260811000001.

ALTER TABLE audit_log
    DROP CONSTRAINT audit_log_target_id_fkey;
CREATE TRIGGER a_audit_log_live_target
BEFORE INSERT OR UPDATE OF target_id ON audit_log
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_id','',''
);

ALTER TABLE attack_candidate_approvals
    DROP CONSTRAINT attack_candidate_approvals_live_target_id_fkey,
    DROP CONSTRAINT attack_candidate_approvals_target_live_id_fkey;
CREATE TRIGGER a_attack_candidate_approvals_live_target
BEFORE INSERT OR UPDATE OF live_target_id ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'live_target_id','',''
);
CREATE TRIGGER a_attack_candidate_approvals_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON attack_candidate_approvals
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE attack_candidate_seeds
    DROP CONSTRAINT attack_candidate_seeds_target_live_id_fkey;
CREATE TRIGGER a_attack_candidate_seeds_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON attack_candidate_seeds
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE attack_candidate_work_items
    DROP CONSTRAINT attack_candidate_work_items_target_live_id_fkey;
CREATE TRIGGER a_attack_candidate_work_items_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON attack_candidate_work_items
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE attack_candidates
    DROP CONSTRAINT attack_candidates_live_target_id_fkey,
    DROP CONSTRAINT attack_candidates_target_live_id_fkey;
CREATE TRIGGER a_attack_candidates_live_target
BEFORE INSERT OR UPDATE OF live_target_id ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'live_target_id','',''
);
CREATE TRIGGER a_attack_candidates_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE attack_fact_deltas
    DROP CONSTRAINT attack_fact_deltas_target_live_id_fkey;
CREATE TRIGGER a_attack_fact_deltas_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON attack_fact_deltas
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE attack_hypothesis_revisions
    DROP CONSTRAINT attack_hypothesis_revisions_target_live_id_fkey;
CREATE TRIGGER a_attack_hypothesis_revisions_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON attack_hypothesis_revisions
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE attack_residual_risks
    DROP CONSTRAINT attack_residual_risks_target_live_id_fkey;
CREATE TRIGGER a_attack_residual_risks_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON attack_residual_risks
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE candidate_attempts
    DROP CONSTRAINT candidate_attempts_target_live_id_fkey;
CREATE TRIGGER a_candidate_attempts_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON candidate_attempts
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE candidate_recovery_cases
    DROP CONSTRAINT candidate_recovery_cases_target_live_id_fkey;
CREATE TRIGGER a_candidate_recovery_cases_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON candidate_recovery_cases
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE finding_lineage
    DROP CONSTRAINT finding_lineage_live_target_id_fkey,
    DROP CONSTRAINT finding_lineage_target_live_id_fkey;
CREATE TRIGGER a_finding_lineage_live_target
BEFORE INSERT OR UPDATE OF live_target_id ON finding_lineage
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'live_target_id','',''
);
CREATE TRIGGER a_finding_lineage_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON finding_lineage
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE findings
    DROP CONSTRAINT findings_target_id_fkey;
CREATE TRIGGER a_findings_live_target
BEFORE INSERT OR UPDATE OF target_id ON findings
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_id','',''
);

ALTER TABLE foothold_candidates
    DROP CONSTRAINT foothold_candidates_target_live_id_fkey;
CREATE TRIGGER a_foothold_candidates_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON foothold_candidates
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE footholds
    DROP CONSTRAINT footholds_target_live_id_fkey;
CREATE TRIGGER a_footholds_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON footholds
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);

ALTER TABLE verification_prepared_actions
    DROP CONSTRAINT verification_prepared_actions_target_live_id_fkey;
CREATE TRIGGER a_verification_prepared_actions_target_live
BEFORE INSERT OR UPDATE OF target_live_id ON verification_prepared_actions
FOR EACH ROW EXECUTE FUNCTION organization_deletion_require_live_target_reference(
    'target_live_id','',''
);
