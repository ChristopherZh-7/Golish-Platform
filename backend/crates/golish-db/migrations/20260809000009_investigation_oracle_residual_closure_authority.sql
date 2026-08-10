-- Oracle-census residuals are stage-owned through their immutable
-- census-member -> Campaign -> VerificationTask -> admitted-revision chain.
-- Older writers left revision_id NULL on these rows, so closure must follow
-- that exact chain rather than treating a valid typed residual as ambiguous.

CREATE FUNCTION unified_investigation_residual_has_stage_authority_v1(
    p_residual_id UUID,
    p_operation_id UUID,
    p_stage_execution_id UUID,
    p_scope_snapshot_id UUID
)
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
STRICT
AS $$
    SELECT EXISTS(
        SELECT 1
          FROM hypothesis_residual_risks residual
         WHERE residual.residual_id=p_residual_id
           AND residual.operation_id=p_operation_id
           AND (
               EXISTS(
                   SELECT 1
                     FROM verification_admission_sets admission
                     JOIN hypothesis_generations generation
                       ON generation.generation_id=admission.generation_id
                     LEFT JOIN hypothesis_generation_members generation_member
                       ON generation_member.generation_id=generation.generation_id
                    WHERE admission.operation_id=p_operation_id
                      AND admission.stage_execution_id=p_stage_execution_id
                      AND admission.scope_snapshot_id=p_scope_snapshot_id
                      AND admission.organization_id=residual.organization_id
                      AND admission.status='sealed'
                      AND (generation_member.revision_id=residual.revision_id
                           OR generation.candidate_snapshot_id=residual.snapshot_id)
               )
               OR (
                   residual.reason_code='oracle_missing'
                   AND residual.owner_kind='plan_c'
                   AND EXISTS(
                       SELECT 1
                         FROM verification_oracle_census_members oracle_member
                         JOIN verification_oracle_census_seals oracle_census
                           ON oracle_census.oracle_census_seal_id=
                              oracle_member.oracle_census_seal_id
                          AND oracle_census.campaign_id=oracle_member.campaign_id
                          AND oracle_census.operation_id=oracle_member.operation_id
                          AND oracle_census.organization_id=oracle_member.organization_id
                          AND oracle_census.sealed_at IS NOT NULL
                         JOIN verification_campaigns campaign
                           ON campaign.campaign_id=oracle_member.campaign_id
                          AND campaign.operation_id=oracle_member.operation_id
                          AND campaign.organization_id=oracle_member.organization_id
                         JOIN hypothesis_verification_task_campaigns reservation
                           ON reservation.campaign_id=campaign.campaign_id
                          AND reservation.hypothesis_revision_id=
                              campaign.hypothesis_revision_id
                         JOIN hypothesis_verification_tasks task
                           ON task.task_id=reservation.task_id
                          AND task.operation_id=p_operation_id
                          AND task.stage_execution_id=p_stage_execution_id
                          AND task.scope_snapshot_id=p_scope_snapshot_id
                          AND task.organization_id=residual.organization_id
                         JOIN verification_admission_sets admission
                           ON admission.operation_id=task.operation_id
                          AND admission.stage_execution_id=task.stage_execution_id
                          AND admission.scope_snapshot_id=task.scope_snapshot_id
                          AND admission.organization_id=task.organization_id
                          AND admission.status='sealed'
                         JOIN verification_admission_members admission_member
                           ON admission_member.admission_set_id=
                              admission.admission_set_id
                          AND admission_member.hypothesis_revision_id=
                              campaign.hypothesis_revision_id
                        WHERE oracle_member.residual_id=residual.residual_id
                          AND oracle_member.operation_id=p_operation_id
                          AND oracle_member.organization_id=residual.organization_id
                          AND oracle_member.disposition='untested'
                   )
               )
           )
    )
$$;

DO $migration$
DECLARE
    closure_definition TEXT;
    authority_subquery CONSTANT TEXT := $obsolete$SELECT 1
                 FROM verification_admission_sets admission
                 JOIN hypothesis_generations generation
                   ON generation.generation_id=admission.generation_id
                 LEFT JOIN hypothesis_generation_members generation_member
                   ON generation_member.generation_id=generation.generation_id
                WHERE admission.operation_id=head.operation_id
                  AND admission.stage_execution_id=head.stage_execution_id
                  AND admission.scope_snapshot_id=head.scope_snapshot_id
                  AND admission.organization_id=residual.organization_id
                  AND admission.status='sealed'
                  AND (generation_member.revision_id=residual.revision_id
                       OR generation.candidate_snapshot_id=residual.snapshot_id)$obsolete$;
    compatible_subquery CONSTANT TEXT := $compatible$SELECT 1
                 WHERE unified_investigation_residual_has_stage_authority_v1(
                           residual.residual_id,
                           head.operation_id,
                           head.stage_execution_id,
                           head.scope_snapshot_id
                       )$compatible$;
    first_position INTEGER;
    second_position INTEGER;
BEGIN
    SELECT pg_get_functiondef(
               'seal_investigation_run_closure_v1(uuid,uuid,uuid,text)'::regprocedure
           )
      INTO STRICT closure_definition;
    first_position := POSITION(authority_subquery IN closure_definition);
    IF first_position=0 THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_RESIDUAL_PREDICATE_DRIFT'
            USING ERRCODE='23514';
    END IF;
    second_position := POSITION(
        authority_subquery IN SUBSTRING(
            closure_definition FROM first_position+LENGTH(authority_subquery)
        )
    );
    IF second_position=0
       OR POSITION(
              authority_subquery IN SUBSTRING(
                  closure_definition
                  FROM first_position+LENGTH(authority_subquery)
                       +second_position+LENGTH(authority_subquery)-1
              )
          )<>0
    THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_RESIDUAL_PREDICATE_DRIFT'
            USING ERRCODE='23514';
    END IF;

    EXECUTE REPLACE(
        closure_definition,
        authority_subquery,
        compatible_subquery
    );
END
$migration$;
