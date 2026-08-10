-- Freeze one immutable AI advisory envelope for each VerificationTask before
-- any Campaign-local mutation. Per-Campaign checkpoints and the final exact
-- seal make partial application response-loss safe without accepting a mixed
-- envelope on resume.

CREATE TABLE investigation_verification_task_advisory_receipts (
    advisory_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    verification_task_id UUID NOT NULL UNIQUE
        REFERENCES hypothesis_verification_tasks(task_id) ON DELETE RESTRICT,
    authority_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    hypothesis_revision_sha256 TEXT NOT NULL
        CHECK (hypothesis_revision_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    verification_plan_id UUID NOT NULL,
    verification_plan_sha256 TEXT NOT NULL
        CHECK (verification_plan_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    assignment_set_id UUID NOT NULL,
    assignment_set_sha256 TEXT NOT NULL
        CHECK (assignment_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    campaign_denominator_sha256 TEXT NOT NULL
        CHECK (campaign_denominator_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    subject_fingerprint_sha256 TEXT NOT NULL
        CHECK (subject_fingerprint_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    task_plan_id UUID NOT NULL UNIQUE,
    delegation_census_seal_id UUID NOT NULL UNIQUE,
    primary_worker_run_id UUID NOT NULL,
    accepted_output_count BIGINT NOT NULL CHECK (accepted_output_count>0),
    accepted_output_set_sha256 TEXT NOT NULL
        CHECK (accepted_output_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    primary_residual_sha256 TEXT[] NOT NULL,
    primary_residual_count BIGINT NOT NULL CHECK (primary_residual_count>=0),
    primary_residual_set_sha256 TEXT NOT NULL
        CHECK (primary_residual_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    campaign_member_count BIGINT NOT NULL CHECK (campaign_member_count>0),
    campaign_member_set_sha256 TEXT NOT NULL
        CHECK (campaign_member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    envelope_sha256 TEXT NOT NULL CHECK (envelope_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'building' CHECK (status IN ('building','applied')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version>=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    applied_at TIMESTAMPTZ,
    UNIQUE(advisory_receipt_id,verification_task_id),
    UNIQUE(advisory_receipt_id,operation_id,stage_execution_id,stage_run_unit_id,
           scope_snapshot_id,organization_id),
    FOREIGN KEY(verification_task_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES hypothesis_verification_tasks(
            task_id,operation_id,stage_execution_id,stage_run_unit_id,
            scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(assignment_set_id,verification_task_id)
        REFERENCES hypothesis_verification_task_assignment_sets(assignment_set_id,task_id)
        ON DELETE RESTRICT
);

CREATE TABLE investigation_verification_task_advisory_members (
    advisory_member_id UUID PRIMARY KEY,
    advisory_receipt_id UUID NOT NULL
        REFERENCES investigation_verification_task_advisory_receipts(advisory_receipt_id)
        ON DELETE RESTRICT,
    verification_task_id UUID NOT NULL,
    assignment_set_id UUID NOT NULL,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    campaign_id UUID NOT NULL,
    plan_objective_id UUID NOT NULL,
    verification_objective_id UUID NOT NULL,
    reservation_sha256 TEXT NOT NULL CHECK (reservation_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    capability_assessment_set_sha256 TEXT NOT NULL
        CHECK (capability_assessment_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    strategy_id UUID NOT NULL,
    capability_key TEXT NOT NULL,
    typed_strategy JSONB NOT NULL,
    strategy_sha256 TEXT NOT NULL CHECK (strategy_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    intent_id UUID NOT NULL,
    typed_intent JSONB NOT NULL,
    intent_sha256 TEXT NOT NULL CHECK (intent_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    member_sha256 TEXT NOT NULL CHECK (member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(advisory_receipt_id,member_ordinal),
    UNIQUE(advisory_receipt_id,campaign_id),
    UNIQUE(advisory_receipt_id,strategy_id),
    UNIQUE(advisory_receipt_id,intent_id),
    UNIQUE(advisory_member_id,advisory_receipt_id,campaign_id),
    FOREIGN KEY(advisory_receipt_id,verification_task_id)
        REFERENCES investigation_verification_task_advisory_receipts(
            advisory_receipt_id,verification_task_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(campaign_id,assignment_set_id,verification_task_id,plan_objective_id)
        REFERENCES hypothesis_verification_task_campaigns(
            campaign_id,assignment_set_id,task_id,plan_objective_id
        ) ON DELETE RESTRICT
);

CREATE TABLE investigation_verification_advisory_campaign_applies (
    campaign_apply_receipt_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    advisory_receipt_id UUID NOT NULL,
    advisory_member_id UUID NOT NULL,
    campaign_id UUID NOT NULL,
    round_id UUID NOT NULL,
    strategy_artifact_id UUID NOT NULL,
    strategy_obligation_id UUID NOT NULL,
    campaign_denominator_id UUID NOT NULL,
    campaign_coverage_member_id UUID NOT NULL,
    intent_id UUID NOT NULL,
    compiler_contract_version TEXT NOT NULL
        CHECK(compiler_contract_version='investigation-verification-action-compiler.v1'),
    compiler_input_sha256 TEXT NOT NULL
        CHECK(compiler_input_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    compiler_result_authority_sha256 TEXT NOT NULL
        CHECK(compiler_result_authority_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    compiler_detail_sha256 TEXT
        CHECK(compiler_detail_sha256 IS NULL OR compiler_detail_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    result_kind TEXT NOT NULL CHECK (result_kind IN ('prepared_action','residual')),
    result_id UUID NOT NULL,
    result_sha256 TEXT NOT NULL CHECK (result_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    apply_sha256 TEXT NOT NULL CHECK (apply_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK((result_kind='prepared_action')=(compiler_detail_sha256 IS NULL)),
    UNIQUE(advisory_receipt_id,campaign_id,strategy_obligation_id),
    UNIQUE(advisory_receipt_id,campaign_id,campaign_coverage_member_id),
    UNIQUE(campaign_apply_receipt_id,advisory_receipt_id),
    FOREIGN KEY(advisory_member_id,advisory_receipt_id,campaign_id)
        REFERENCES investigation_verification_task_advisory_members(
            advisory_member_id,advisory_receipt_id,campaign_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(round_id)
        REFERENCES verification_campaign_rounds(round_id) ON DELETE RESTRICT,
    FOREIGN KEY(strategy_artifact_id)
        REFERENCES verification_strategy_artifacts(strategy_artifact_id) ON DELETE RESTRICT,
    FOREIGN KEY(strategy_artifact_id,strategy_obligation_id)
        REFERENCES verification_strategy_obligations(strategy_artifact_id,obligation_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(campaign_denominator_id)
        REFERENCES verification_campaign_coverage_denominators(campaign_denominator_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(campaign_coverage_member_id,campaign_denominator_id)
        REFERENCES verification_campaign_coverage_members(
            campaign_coverage_member_id,campaign_denominator_id
        ) ON DELETE RESTRICT
);

CREATE FUNCTION investigation_verification_action_compiler_input_sha256_v1(
    p_advisory_member_id UUID,
    p_strategy_artifact_id UUID,
    p_strategy_obligation_id UUID,
    p_campaign_denominator_id UUID,
    p_campaign_coverage_member_id UUID
)
RETURNS TEXT
LANGUAGE SQL
STABLE
PARALLEL SAFE
AS $$
    SELECT tool_truth_sha256(jsonb_build_object(
        'contract_version','investigation-verification-action-compiler-input.v1',
        'advisory_member_sha256',member.member_sha256,
        'strategy_artifact_id',strategy.strategy_artifact_id,
        'strategy_sha256',strategy.strategy_hash,
        'strategy_obligation_id',obligation.obligation_id,
        'strategy_obligation_sha256',obligation.member_hash,
        'campaign_denominator_id',coverage.campaign_denominator_id,
        'campaign_coverage_member_id',coverage.campaign_coverage_member_id,
        'campaign_coverage_member_sha256',coverage.member_hash,
        'intent_sha256',member.intent_sha256,
        'campaign_authority_sha256',
            unified_investigation_campaign_authority_sha256_v4(
                campaign.campaign_id,member.reservation_sha256
            ),
        'capability_assessment_id',assessment.assessment_id,
        'capability_assessment_sha256',assessment.assessment_hash,
        'capability_assessment_status',assessment.status,
        'capability_contract_sha256',assessment.capability_contract_hash,
        'adapter_contract_version',assessment.adapter_contract_version,
        'adapter_contract_sha256',assessment.adapter_contract_digest,
        'assessment_policy_snapshot_sha256',assessment.policy_snapshot_hash,
        'assessment_set_sha256',assessment_set.member_set_hash,
        'capability_registry_contract_sha256',assessment_set.registry_contract_hash,
        'wave_coverage_member_sha256',wave_member.member_hash,
        'target_live_id',revision.target_live_id,
        'target_type_at_time',revision.target_type_at_time,
        'target_value_at_time',revision.target_value_at_time,
        'target_identity_sha256',tool_truth_sha256(jsonb_build_object(
            'target_live_id',revision.target_live_id,
            'project_scope_id',campaign.project_scope_id,
            'organization_id',campaign.organization_id,
            'target_type',revision.target_type_at_time,
            'target_value',revision.target_value_at_time
        )::TEXT),
        'semantic_scope_authority_sha256',campaign.semantic_authority_bundle_hash,
        'expected_oracle_kind',coverage.expected_oracle_kind,
        'operation_budget_contract_sha256',(
            SELECT budget.contract_hash FROM verification_budget_contracts budget
             WHERE budget.scope_kind='operation' AND budget.scope_id=campaign.operation_id
               AND budget.operation_id=campaign.operation_id
               AND budget.organization_id=campaign.organization_id
               AND budget.sealed_at IS NOT NULL
        ),
        'wave_budget_contract_sha256',(
            SELECT budget.contract_hash FROM verification_budget_contracts budget
             WHERE budget.scope_kind='wave' AND budget.scope_id=campaign.wave_denominator_id
               AND budget.operation_id=campaign.operation_id
               AND budget.organization_id=campaign.organization_id
               AND budget.sealed_at IS NOT NULL
        ),
        'campaign_budget_contract_sha256',(
            SELECT budget.contract_hash FROM verification_budget_contracts budget
             WHERE budget.scope_kind='campaign' AND budget.scope_id=campaign.campaign_id
               AND budget.operation_id=campaign.operation_id
               AND budget.organization_id=campaign.organization_id
               AND budget.sealed_at IS NOT NULL
        ),
        'action_budget_policy_sha256',tool_truth_sha256(jsonb_build_object(
            'contract_version','verification-action-budget-policy.v1',
            'requests',4,'response_bytes',4194304,'wall_clock_ms',180000,
            'retries',1,'browser_steps',0,'oast_tokens',0
        )::TEXT)
    )::TEXT)
      FROM investigation_verification_task_advisory_members member
      JOIN verification_strategy_artifacts strategy
        ON strategy.strategy_artifact_id=p_strategy_artifact_id
      JOIN verification_strategy_obligations obligation
        ON obligation.strategy_artifact_id=strategy.strategy_artifact_id
       AND obligation.obligation_id=p_strategy_obligation_id
      JOIN verification_campaign_coverage_members coverage
        ON coverage.campaign_denominator_id=p_campaign_denominator_id
       AND coverage.campaign_coverage_member_id=p_campaign_coverage_member_id
      JOIN verification_campaigns campaign
        ON campaign.campaign_id=member.campaign_id
       AND campaign.operation_id=coverage.operation_id
       AND campaign.organization_id=coverage.organization_id
      JOIN verification_wave_coverage_members wave_member
        ON wave_member.wave_coverage_member_id=coverage.wave_coverage_member_id
       AND wave_member.wave_denominator_id=coverage.wave_denominator_id
      JOIN verification_capability_assessments assessment
        ON assessment.assessment_id=coverage.capability_assessment_id
      JOIN verification_capability_assessment_set_seals assessment_set
        ON assessment_set.assessment_set_seal_id=
           campaign.capability_assessment_set_seal_id
       AND assessment_set.sealed_at IS NOT NULL
      JOIN attack_hypothesis_revisions revision
        ON revision.revision_id=campaign.hypothesis_revision_id
       AND revision.operation_id=campaign.operation_id
       AND revision.organization_id=campaign.organization_id
     WHERE member.advisory_member_id=p_advisory_member_id;
$$;

CREATE FUNCTION investigation_verification_action_compiler_result_sha256_v1(
    p_result_kind TEXT,
    p_result_id UUID
)
RETURNS TEXT
LANGUAGE SQL
STABLE
PARALLEL SAFE
AS $$
    SELECT CASE p_result_kind
        WHEN 'prepared_action' THEN (
            SELECT tool_truth_sha256(action.private_manifest::TEXT)
              FROM verification_prepared_actions action
             WHERE action.prepared_action_id=p_result_id
        )
        WHEN 'residual' THEN (
            SELECT residual.residual_hash
              FROM hypothesis_residual_risks residual
             WHERE residual.residual_id=p_result_id
        )
        ELSE NULL
    END;
$$;

CREATE TABLE investigation_verification_task_advisory_seals (
    advisory_seal_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    advisory_receipt_id UUID NOT NULL UNIQUE
        REFERENCES investigation_verification_task_advisory_receipts(advisory_receipt_id)
        ON DELETE RESTRICT,
    verification_task_id UUID NOT NULL UNIQUE,
    campaign_apply_count BIGINT NOT NULL CHECK (campaign_apply_count>0),
    campaign_apply_set_sha256 TEXT NOT NULL
        CHECK (campaign_apply_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    prepared_action_count BIGINT NOT NULL CHECK (prepared_action_count>=0),
    prepared_action_set_sha256 TEXT NOT NULL
        CHECK (prepared_action_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    residual_count BIGINT NOT NULL CHECK (residual_count>=0),
    residual_set_sha256 TEXT NOT NULL
        CHECK (residual_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    seal_sha256 TEXT NOT NULL CHECK (seal_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(advisory_receipt_id,verification_task_id)
        REFERENCES investigation_verification_task_advisory_receipts(
            advisory_receipt_id,verification_task_id
        ) ON DELETE RESTRICT
);

CREATE OR REPLACE FUNCTION enforce_investigation_verification_advisory_header()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_campaign_denominator_sha256 TEXT;
    expected_subject_fingerprint_sha256 TEXT;
    expected_accepted_output_count BIGINT;
    expected_accepted_output_set_sha256 TEXT;
    expected_campaign_member_count BIGINT;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_APPEND_ONLY';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'building' OR NEW.row_version<>0 OR NEW.applied_at IS NOT NULL THEN
            RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_MUST_BUILD_FIRST';
        END IF;
        IF NEW.primary_residual_count<>cardinality(NEW.primary_residual_sha256)
           OR EXISTS(
                SELECT 1 FROM unnest(NEW.primary_residual_sha256) residual(value)
                 WHERE value IS NULL OR value !~ '^sha256:[0-9a-f]{64}$'
           )
           OR NEW.primary_residual_sha256 IS DISTINCT FROM (
                SELECT COALESCE(array_agg(DISTINCT value ORDER BY value),ARRAY[]::TEXT[])
                  FROM unnest(NEW.primary_residual_sha256) residual(value)
           )
           OR NEW.primary_residual_set_sha256<>
              unified_investigation_exact_set_hash(
                  'investigation_verification_primary_residuals.v1',
                  NEW.primary_residual_sha256
              )
        THEN
            RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_PRIMARY_RESIDUAL_SET_MISMATCH';
        END IF;
        SELECT unified_investigation_verification_campaign_denominator_v4(
                   task.task_id,assignment.assignment_set_id
               ),
               tool_truth_sha256(jsonb_build_object(
                   'task_id',task.task_id,
                   'revision_sha256',task.hypothesis_revision_sha256,
                   'plan_sha256',task.verification_plan_sha256,
                   'assignment_sha256',assignment.member_set_sha256,
                   'campaign_denominator_sha256',
                       unified_investigation_verification_campaign_denominator_v4(
                           task.task_id,assignment.assignment_set_id
                       ),
                   'semantic_attempt_fingerprint',task.semantic_attempt_fingerprint
               )::TEXT),
               (SELECT COUNT(*)
                  FROM hypothesis_verification_task_campaigns reservation
                 WHERE reservation.task_id=task.task_id
                   AND reservation.assignment_set_id=assignment.assignment_set_id)
          INTO STRICT expected_campaign_denominator_sha256,
                      expected_subject_fingerprint_sha256,
                      expected_campaign_member_count
          FROM hypothesis_verification_tasks task
          JOIN hypothesis_verification_task_assignment_sets assignment
            ON assignment.assignment_set_id=NEW.assignment_set_id
           AND assignment.task_id=task.task_id
           AND assignment.status='sealed'
         WHERE task.task_id=NEW.verification_task_id
           AND task.operation_id=NEW.operation_id
           AND task.stage_execution_id=NEW.stage_execution_id
           AND task.stage_run_unit_id=NEW.stage_run_unit_id
           AND task.scope_snapshot_id=NEW.scope_snapshot_id
           AND task.organization_id=NEW.organization_id
           AND task.hypothesis_revision_id=NEW.hypothesis_revision_id
           AND task.hypothesis_revision_sha256=NEW.hypothesis_revision_sha256
           AND task.verification_plan_id=NEW.verification_plan_id
           AND task.verification_plan_sha256=NEW.verification_plan_sha256
           AND assignment.member_set_sha256=NEW.assignment_set_sha256;
        SELECT COUNT(*),
               unified_investigation_exact_set_hash(
                   'investigation_verification_accepted_outputs.v1',
                   COALESCE(array_agg(latest.result_sha256 ORDER BY latest.result_sha256),
                            ARRAY[]::TEXT[])
               )
          INTO expected_accepted_output_count,expected_accepted_output_set_sha256
          FROM (
              SELECT DISTINCT dispatch_latest.result_sha256
                FROM (
                    SELECT DISTINCT ON(dispatch.dispatch_receipt_id)
                           dispatch.dispatch_receipt_id,attempt.result_sha256
                      FROM pentagi_logical_dispatch_receipts dispatch
                      JOIN pentagi_logical_dispatch_attempts attempt
                        ON attempt.dispatch_receipt_id=dispatch.dispatch_receipt_id
                     WHERE dispatch.task_plan_id=NEW.task_plan_id
                       AND dispatch.actor_kind IN ('worker','nested_worker')
                     ORDER BY dispatch.dispatch_receipt_id,attempt.attempt_epoch DESC
                ) dispatch_latest
          ) latest;
        IF expected_campaign_denominator_sha256 IS NULL
           OR NEW.campaign_denominator_sha256 IS DISTINCT FROM
              expected_campaign_denominator_sha256
           OR NEW.subject_fingerprint_sha256 IS DISTINCT FROM
              expected_subject_fingerprint_sha256
           OR NEW.campaign_member_count<>expected_campaign_member_count
           OR NEW.accepted_output_count<>expected_accepted_output_count
           OR NEW.accepted_output_set_sha256<>expected_accepted_output_set_sha256
           OR NOT EXISTS(
                SELECT 1
                  FROM investigation_pentagi_task_plans plan
                  JOIN investigation_pentagi_delegation_census_seals census
                    ON census.census_seal_id=NEW.delegation_census_seal_id
                   AND census.task_plan_id=plan.task_plan_id
                   AND census.primary_worker_run_id=NEW.primary_worker_run_id
                 WHERE plan.task_plan_id=NEW.task_plan_id
                   AND plan.authority_id=NEW.authority_id
                   AND plan.operation_id=NEW.operation_id
                   AND plan.stage_execution_id=NEW.stage_execution_id
                   AND plan.stage_run_unit_id=NEW.stage_run_unit_id
                   AND plan.scope_snapshot_id=NEW.scope_snapshot_id
                   AND plan.organization_id=NEW.organization_id
                   AND plan.subject_kind='verification_task'
                   AND plan.subject_id=NEW.verification_task_id
                   AND plan.subject_fingerprint_sha256=NEW.subject_fingerprint_sha256
                   AND plan.status='sealed'
                   AND EXISTS(
                        SELECT 1 FROM investigation_pentagi_pipeline_events event
                         WHERE event.task_plan_id=plan.task_plan_id
                           AND event.event_kind='primary_synthesis'
                           AND event.actor_worker_run_id=NEW.primary_worker_run_id
                   )
           )
        THEN
            RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_HEADER_AUTHORITY_MISMATCH';
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.status<>'building' OR NEW.status<>'applied'
       OR NEW.row_version<>OLD.row_version+1 OR NEW.applied_at IS NULL
       OR ROW(
            NEW.advisory_receipt_id,NEW.stable_request_id,NEW.verification_task_id,
            NEW.authority_id,NEW.operation_id,NEW.stage_execution_id,
            NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
            NEW.hypothesis_revision_id,NEW.hypothesis_revision_sha256,
            NEW.verification_plan_id,NEW.verification_plan_sha256,
            NEW.assignment_set_id,NEW.assignment_set_sha256,
            NEW.campaign_denominator_sha256,NEW.subject_fingerprint_sha256,
            NEW.task_plan_id,NEW.delegation_census_seal_id,NEW.primary_worker_run_id,
            NEW.accepted_output_count,NEW.accepted_output_set_sha256,
            NEW.primary_residual_sha256,NEW.primary_residual_count,
            NEW.primary_residual_set_sha256,
            NEW.campaign_member_count,NEW.campaign_member_set_sha256,
            NEW.envelope_sha256,NEW.created_at
       ) IS DISTINCT FROM ROW(
            OLD.advisory_receipt_id,OLD.stable_request_id,OLD.verification_task_id,
            OLD.authority_id,OLD.operation_id,OLD.stage_execution_id,
            OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
            OLD.hypothesis_revision_id,OLD.hypothesis_revision_sha256,
            OLD.verification_plan_id,OLD.verification_plan_sha256,
            OLD.assignment_set_id,OLD.assignment_set_sha256,
            OLD.campaign_denominator_sha256,OLD.subject_fingerprint_sha256,
            OLD.task_plan_id,OLD.delegation_census_seal_id,OLD.primary_worker_run_id,
            OLD.accepted_output_count,OLD.accepted_output_set_sha256,
            OLD.primary_residual_sha256,OLD.primary_residual_count,
            OLD.primary_residual_set_sha256,
            OLD.campaign_member_count,OLD.campaign_member_set_sha256,
            OLD.envelope_sha256,OLD.created_at
       ) OR NOT EXISTS(
            SELECT 1 FROM investigation_verification_task_advisory_seals seal
             WHERE seal.advisory_receipt_id=NEW.advisory_receipt_id
               AND seal.verification_task_id=NEW.verification_task_id
               AND (SELECT COUNT(DISTINCT apply.campaign_id)
                      FROM investigation_verification_advisory_campaign_applies apply
                     WHERE apply.advisory_receipt_id=NEW.advisory_receipt_id)=
                   NEW.campaign_member_count
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_APPLY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_verification_advisory_header_guard
BEFORE INSERT OR UPDATE OR DELETE ON investigation_verification_task_advisory_receipts
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_verification_advisory_header();

CREATE OR REPLACE FUNCTION reject_investigation_verification_advisory_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_APPEND_ONLY';
END;
$$;

CREATE FUNCTION enforce_investigation_verification_advisory_member()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    header investigation_verification_task_advisory_receipts%ROWTYPE;
    expected_strategy_sha256 TEXT;
    expected_intent_sha256 TEXT;
    expected_member_sha256 TEXT;
    strategy_output_set_sha256 TEXT;
BEGIN
    SELECT * INTO STRICT header
      FROM investigation_verification_task_advisory_receipts
     WHERE advisory_receipt_id=NEW.advisory_receipt_id
       AND verification_task_id=NEW.verification_task_id FOR SHARE;
    expected_strategy_sha256 := tool_truth_sha256(NEW.typed_strategy::TEXT);
    expected_intent_sha256 := tool_truth_sha256(NEW.typed_intent::TEXT);
    expected_member_sha256 := tool_truth_sha256(jsonb_build_object(
        'contract_version','investigation-verification-advisory-member.v1',
        'campaign_id',NEW.campaign_id,'plan_objective_id',NEW.plan_objective_id,
        'objective_id',NEW.verification_objective_id,
        'reservation_sha256',NEW.reservation_sha256,
        'capability_assessment_set_sha256',NEW.capability_assessment_set_sha256,
        'strategy_sha256',expected_strategy_sha256,
        'intent_sha256',expected_intent_sha256
    )::TEXT);
    IF jsonb_typeof(NEW.typed_strategy->'accepted_output_sha256') IS DISTINCT FROM 'array'
       OR jsonb_typeof(NEW.typed_strategy->'action_intents') IS DISTINCT FROM 'array'
    THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_MEMBER_ENVELOPE_MALFORMED';
    END IF;
    SELECT unified_investigation_exact_set_hash(
               'investigation_verification_accepted_outputs.v1',
               COALESCE(array_agg(output.value ORDER BY output.value),ARRAY[]::TEXT[])
           )
      INTO strategy_output_set_sha256
      FROM (
          SELECT DISTINCT value
            FROM jsonb_array_elements_text(
                     NEW.typed_strategy->'accepted_output_sha256'
                 ) output(value)
      ) output;
    IF header.status<>'building' OR NEW.assignment_set_id<>header.assignment_set_id
       OR NEW.strategy_sha256 IS DISTINCT FROM expected_strategy_sha256
       OR NEW.intent_sha256 IS DISTINCT FROM expected_intent_sha256
       OR NEW.member_sha256 IS DISTINCT FROM expected_member_sha256
       OR NEW.typed_strategy->>'schema' IS DISTINCT FROM
          'investigation_verification_strategy.v1'
       OR NEW.typed_strategy->>'advisory_request_id' IS DISTINCT FROM
          header.stable_request_id::TEXT
       OR NEW.typed_strategy->>'strategy_id' IS DISTINCT FROM NEW.strategy_id::TEXT
       OR NEW.typed_strategy->>'campaign_id' IS DISTINCT FROM NEW.campaign_id::TEXT
       OR NEW.typed_strategy->>'objective_id' IS DISTINCT FROM
          NEW.verification_objective_id::TEXT
       OR NEW.typed_strategy->>'capability' IS DISTINCT FROM NEW.capability_key
       OR strategy_output_set_sha256 IS DISTINCT FROM header.accepted_output_set_sha256
       OR jsonb_array_length(NEW.typed_strategy->'action_intents')<>1
       OR NEW.typed_strategy->'action_intents'->0 IS DISTINCT FROM NEW.typed_intent
       OR NEW.typed_intent->>'schema' IS DISTINCT FROM
          'investigation_verification_action_intent.v1'
       OR NEW.typed_intent->>'intent_id' IS DISTINCT FROM NEW.intent_id::TEXT
       OR NEW.typed_intent->>'strategy_id' IS DISTINCT FROM NEW.strategy_id::TEXT
       OR NEW.typed_intent->>'campaign_id' IS DISTINCT FROM NEW.campaign_id::TEXT
       OR NEW.typed_intent->>'capability' IS DISTINCT FROM NEW.capability_key
       OR NOT EXISTS(
            SELECT 1
              FROM hypothesis_verification_task_campaigns reservation
              JOIN verification_campaigns campaign
                ON campaign.campaign_id=reservation.campaign_id
               AND campaign.operation_id=header.operation_id
               AND campaign.organization_id=header.organization_id
               AND campaign.hypothesis_revision_id=header.hypothesis_revision_id
               AND campaign.state IN ('admitted','running')
               AND campaign.terminal_at IS NULL
               AND campaign.superseded_at IS NULL
               AND campaign.effective_valid_until>statement_timestamp()
              JOIN verification_capability_assessment_set_seals assessment_set
                ON assessment_set.assessment_set_seal_id=
                   campaign.capability_assessment_set_seal_id
               AND assessment_set.member_set_hash=NEW.capability_assessment_set_sha256
               AND assessment_set.sealed_at IS NOT NULL
             WHERE reservation.campaign_id=NEW.campaign_id
               AND reservation.assignment_set_id=NEW.assignment_set_id
               AND reservation.task_id=NEW.verification_task_id
               AND reservation.plan_objective_id=NEW.plan_objective_id
               AND reservation.verification_objective_id=NEW.verification_objective_id
               AND reservation.reservation_sha256=NEW.reservation_sha256
               AND EXISTS(
                    SELECT 1 FROM verification_wave_coverage_members coverage
                     WHERE coverage.wave_denominator_id=campaign.wave_denominator_id
                       AND coverage.verification_objective_id=
                           NEW.verification_objective_id
                       AND coverage.expected_capability_kind=NEW.capability_key
               )
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_MEMBER_AUTHORITY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION enforce_investigation_verification_advisory_campaign_apply()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    header investigation_verification_task_advisory_receipts%ROWTYPE;
    member investigation_verification_task_advisory_members%ROWTYPE;
    expected_compiler_input_sha256 TEXT;
    expected_compiler_result_authority_sha256 TEXT;
    expected_apply_sha256 TEXT;
BEGIN
    SELECT * INTO STRICT header
      FROM investigation_verification_task_advisory_receipts
     WHERE advisory_receipt_id=NEW.advisory_receipt_id FOR SHARE;
    SELECT * INTO STRICT member
      FROM investigation_verification_task_advisory_members
     WHERE advisory_member_id=NEW.advisory_member_id
       AND advisory_receipt_id=NEW.advisory_receipt_id
       AND campaign_id=NEW.campaign_id FOR SHARE;
    expected_compiler_input_sha256 :=
        investigation_verification_action_compiler_input_sha256_v1(
            NEW.advisory_member_id,NEW.strategy_artifact_id,
            NEW.strategy_obligation_id,NEW.campaign_denominator_id,
            NEW.campaign_coverage_member_id
        );
    expected_compiler_result_authority_sha256 :=
        investigation_verification_action_compiler_result_sha256_v1(
            NEW.result_kind,NEW.result_id
        );
    IF header.status<>'building'
       OR NEW.intent_id<>member.intent_id
       OR NEW.compiler_contract_version<>
          'investigation-verification-action-compiler.v1'
       OR NEW.compiler_input_sha256 IS DISTINCT FROM expected_compiler_input_sha256
       OR NEW.compiler_result_authority_sha256 IS DISTINCT FROM
          expected_compiler_result_authority_sha256
       OR NOT EXISTS(
            SELECT 1 FROM verification_campaign_rounds round
             WHERE round.round_id=NEW.round_id AND round.campaign_id=NEW.campaign_id
               AND round.operation_id=header.operation_id
               AND round.organization_id=header.organization_id
       )
       OR NOT EXISTS(
            SELECT 1 FROM verification_strategy_artifacts strategy
             WHERE strategy.strategy_artifact_id=NEW.strategy_artifact_id
               AND strategy.round_id=NEW.round_id AND strategy.campaign_id=NEW.campaign_id
               AND strategy.operation_id=header.operation_id
               AND strategy.organization_id=header.organization_id
               AND strategy.typed_strategy->>'strategy_id'=member.strategy_id::TEXT
               AND strategy.typed_strategy=member.typed_strategy
               AND strategy.strategy_hash=member.strategy_sha256
       )
       OR NOT EXISTS(
            SELECT 1 FROM verification_strategy_obligations obligation
            JOIN verification_campaign_coverage_members coverage
              ON coverage.campaign_denominator_id=NEW.campaign_denominator_id
             AND coverage.campaign_coverage_member_id=NEW.campaign_coverage_member_id
             AND coverage.semantic_key=obligation.semantic_key
             AND coverage.expected_capability_kind=obligation.obligation_kind
             WHERE obligation.strategy_artifact_id=NEW.strategy_artifact_id
               AND obligation.obligation_id=NEW.strategy_obligation_id
               AND obligation.disposition='planned'
       )
       OR NOT EXISTS(
            SELECT 1 FROM verification_campaign_coverage_denominators denominator
             WHERE denominator.campaign_denominator_id=NEW.campaign_denominator_id
               AND denominator.campaign_id=NEW.campaign_id
               AND denominator.operation_id=header.operation_id
               AND denominator.organization_id=header.organization_id
               AND denominator.sealed_at IS NOT NULL
       )
       OR (NEW.result_kind='prepared_action' AND NOT EXISTS(
            SELECT 1 FROM verification_prepared_actions action
             WHERE action.prepared_action_id=NEW.result_id
               AND action.campaign_id=NEW.campaign_id AND action.round_id=NEW.round_id
               AND action.strategy_artifact_id=NEW.strategy_artifact_id
               AND action.operation_id=header.operation_id
               AND action.organization_id=header.organization_id
               AND action.private_manifest_hash=NEW.result_sha256
               AND action.private_manifest->>'strategy_obligation_id'=
                   NEW.strategy_obligation_id::TEXT
               AND action.private_manifest->>'strategy_decision_id'=
                   NEW.strategy_artifact_id::TEXT
               AND action.private_manifest->>'coverage_member_hash'=(
                    SELECT coverage.member_hash
                      FROM verification_campaign_coverage_members coverage
                     WHERE coverage.campaign_coverage_member_id=
                           NEW.campaign_coverage_member_id
                       AND coverage.campaign_denominator_id=NEW.campaign_denominator_id
               )
               AND action.private_manifest->>'strategy_decision_hash'=(
                    SELECT strategy.strategy_hash
                      FROM verification_strategy_artifacts strategy
                     WHERE strategy.strategy_artifact_id=NEW.strategy_artifact_id
               )
               AND action.private_manifest->>'wave_coverage_member_hash'=(
                    SELECT wave.member_hash
                      FROM verification_campaign_coverage_members coverage
                      JOIN verification_wave_coverage_members wave
                        ON wave.wave_coverage_member_id=coverage.wave_coverage_member_id
                       AND wave.wave_denominator_id=coverage.wave_denominator_id
                     WHERE coverage.campaign_coverage_member_id=
                           NEW.campaign_coverage_member_id
                       AND coverage.campaign_denominator_id=NEW.campaign_denominator_id
               )
               AND action.private_manifest->>'capability_assessment_id'=(
                    SELECT coverage.capability_assessment_id::TEXT
                      FROM verification_campaign_coverage_members coverage
                     WHERE coverage.campaign_coverage_member_id=
                           NEW.campaign_coverage_member_id
                       AND coverage.campaign_denominator_id=NEW.campaign_denominator_id
               )
               AND action.private_manifest->>'capability_id'=(
                    SELECT coverage.expected_capability_kind
                      FROM verification_campaign_coverage_members coverage
                     WHERE coverage.campaign_coverage_member_id=
                           NEW.campaign_coverage_member_id
                       AND coverage.campaign_denominator_id=NEW.campaign_denominator_id
               )
               AND action.private_manifest->>'capability_assessment_set_hash'=(
                    SELECT assessment_set.member_set_hash
                      FROM verification_campaigns campaign
                      JOIN verification_capability_assessment_set_seals assessment_set
                        ON assessment_set.assessment_set_seal_id=
                           campaign.capability_assessment_set_seal_id
                     WHERE campaign.campaign_id=NEW.campaign_id
               )
               AND action.private_manifest->>'capability_registry_contract_hash'=(
                    SELECT assessment_set.registry_contract_hash
                      FROM verification_campaigns campaign
                      JOIN verification_capability_assessment_set_seals assessment_set
                        ON assessment_set.assessment_set_seal_id=
                           campaign.capability_assessment_set_seal_id
                     WHERE campaign.campaign_id=NEW.campaign_id
               )
               AND NEW.compiler_detail_sha256 IS NULL
       ))
       OR (NEW.result_kind='residual' AND NOT EXISTS(
            SELECT 1 FROM hypothesis_residual_risks residual
             WHERE residual.residual_id=NEW.result_id
               AND residual.operation_id=header.operation_id
               AND residual.organization_id=header.organization_id
               AND residual.revision_id=header.hypothesis_revision_id
               AND residual.closed_at IS NULL
               AND residual.reason_code='investigation_verification_action_not_compilable'
               AND residual.owner_kind='plan_c'
               AND residual.residual_hash=NEW.result_sha256
               AND jsonb_typeof(residual.affected_inputs)='array'
               AND jsonb_array_length(residual.affected_inputs)=4
               AND residual.affected_inputs=jsonb_build_array(
                    'verification_task:' || header.verification_task_id::TEXT,
                    'campaign:' || NEW.campaign_id::TEXT,
                    'strategy_obligation:' || NEW.strategy_obligation_id::TEXT,
                    residual.affected_inputs->>3
               )
               AND residual.affected_inputs->>3 ~
                   '^compiler_detail_sha256:sha256:[0-9a-f]{64}$'
               AND residual.next_action=jsonb_build_object(
                    'kind','verification_strategy_refinement_required','retry',FALSE
               )
               AND residual.residual_hash=tool_truth_sha256(jsonb_build_object(
                    'reason_code','investigation_verification_action_not_compilable',
                    'affected_inputs',residual.affected_inputs,
                    'next_action',residual.next_action,
                    'compiler_detail_sha256',substr(
                        residual.affected_inputs->>3,
                        length('compiler_detail_sha256:')+1
                    )
               )::TEXT)
               AND NEW.compiler_detail_sha256=substr(
                    residual.affected_inputs->>3,
                    length('compiler_detail_sha256:')+1
               )
       ))
    THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_CAMPAIGN_APPLY_AUTHORITY_MISMATCH';
    END IF;
    expected_apply_sha256 := tool_truth_sha256(jsonb_build_object(
        'contract_version','investigation-verification-campaign-apply.v1',
        'advisory_receipt_id',NEW.advisory_receipt_id,
        'advisory_member_id',NEW.advisory_member_id,
        'campaign_id',NEW.campaign_id,'round_id',NEW.round_id,
        'strategy_artifact_id',NEW.strategy_artifact_id,
        'strategy_obligation_id',NEW.strategy_obligation_id,
        'campaign_denominator_id',NEW.campaign_denominator_id,
        'campaign_coverage_member_id',NEW.campaign_coverage_member_id,
        'intent_id',NEW.intent_id,
        'compiler_contract_version',NEW.compiler_contract_version,
        'compiler_input_sha256',NEW.compiler_input_sha256,
        'compiler_result_authority_sha256',NEW.compiler_result_authority_sha256,
        'compiler_detail_sha256',NEW.compiler_detail_sha256,
        'result_kind',NEW.result_kind,'result_id',NEW.result_id,
        'result_sha256',NEW.result_sha256
    )::TEXT);
    IF NEW.apply_sha256<>expected_apply_sha256 THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_CAMPAIGN_APPLY_HASH_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION enforce_investigation_verification_advisory_seal()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    header investigation_verification_task_advisory_receipts%ROWTYPE;
    apply_count BIGINT;
    apply_set TEXT;
    action_count BIGINT;
    action_set TEXT;
    residual_count BIGINT;
    residual_set TEXT;
    planned_obligation_count BIGINT;
    member_count BIGINT;
    member_set TEXT;
    expected_envelope TEXT;
    expected_seal TEXT;
BEGIN
    SELECT * INTO STRICT header
      FROM investigation_verification_task_advisory_receipts
     WHERE advisory_receipt_id=NEW.advisory_receipt_id
       AND verification_task_id=NEW.verification_task_id FOR SHARE;
    SELECT COUNT(*),
           unified_investigation_exact_set_hash(
               'investigation_verification_campaign_applies.v1',
               COALESCE(array_agg(apply.apply_sha256
                                  ORDER BY apply.campaign_id,apply.strategy_obligation_id),
                        ARRAY[]::TEXT[])
           ),
           COUNT(*) FILTER(WHERE apply.result_kind='prepared_action'),
           unified_investigation_exact_set_hash(
               'investigation_verification_prepared_actions.v1',
               COALESCE(array_agg(apply.result_sha256
                                  ORDER BY apply.campaign_id,apply.strategy_obligation_id)
                        FILTER(WHERE apply.result_kind='prepared_action'),ARRAY[]::TEXT[])
           ),
           COUNT(*) FILTER(WHERE apply.result_kind='residual'),
           unified_investigation_exact_set_hash(
               'investigation_verification_residuals.v1',
               COALESCE(array_agg(apply.result_sha256
                                  ORDER BY apply.campaign_id,apply.strategy_obligation_id)
                        FILTER(WHERE apply.result_kind='residual'),ARRAY[]::TEXT[])
           )
      INTO apply_count,apply_set,action_count,action_set,residual_count,residual_set
      FROM investigation_verification_advisory_campaign_applies apply
     WHERE apply.advisory_receipt_id=NEW.advisory_receipt_id;
    SELECT COUNT(*) INTO planned_obligation_count
      FROM investigation_verification_task_advisory_members member
      JOIN (SELECT DISTINCT advisory_receipt_id,advisory_member_id,campaign_id,
                            strategy_artifact_id
              FROM investigation_verification_advisory_campaign_applies) apply
        ON apply.advisory_receipt_id=member.advisory_receipt_id
       AND apply.advisory_member_id=member.advisory_member_id
       AND apply.campaign_id=member.campaign_id
      JOIN verification_strategy_obligations obligation
        ON obligation.strategy_artifact_id=apply.strategy_artifact_id
       AND obligation.disposition='planned'
     WHERE member.advisory_receipt_id=NEW.advisory_receipt_id;
    SELECT COUNT(*),
           unified_investigation_exact_set_hash(
               'investigation_verification_advisory_members.v1',
               COALESCE(array_agg(member.member_sha256 ORDER BY member.campaign_id),
                        ARRAY[]::TEXT[])
           )
      INTO member_count,member_set
      FROM investigation_verification_task_advisory_members member
     WHERE member.advisory_receipt_id=NEW.advisory_receipt_id;
    expected_envelope := tool_truth_sha256(jsonb_build_object(
        'contract_version','investigation-verification-task-advisory.v1',
        'authority_id',header.authority_id,
        'operation_id',header.operation_id,
        'stage_execution_id',header.stage_execution_id,
        'stage_run_unit_id',header.stage_run_unit_id,
        'scope_snapshot_id',header.scope_snapshot_id,
        'organization_id',header.organization_id,
        'verification_task_id',header.verification_task_id,
        'hypothesis_revision_id',header.hypothesis_revision_id,
        'hypothesis_revision_sha256',header.hypothesis_revision_sha256,
        'verification_plan_id',header.verification_plan_id,
        'verification_plan_sha256',header.verification_plan_sha256,
        'assignment_set_id',header.assignment_set_id,
        'assignment_set_sha256',header.assignment_set_sha256,
        'campaign_denominator_sha256',header.campaign_denominator_sha256,
        'subject_fingerprint_sha256',header.subject_fingerprint_sha256,
        'task_plan_id',header.task_plan_id,
        'delegation_census_seal_id',header.delegation_census_seal_id,
        'primary_worker_run_id',header.primary_worker_run_id,
        'accepted_output_set_sha256',header.accepted_output_set_sha256,
        'primary_residual_count',header.primary_residual_count,
        'primary_residual_set_sha256',header.primary_residual_set_sha256,
        'campaign_member_set_sha256',member_set
    )::TEXT);
    expected_seal := tool_truth_sha256(jsonb_build_object(
        'contract_version','investigation-verification-task-advisory-seal.v1',
        'advisory_receipt_id',NEW.advisory_receipt_id,
        'verification_task_id',NEW.verification_task_id,
        'envelope_sha256',header.envelope_sha256,
        'campaign_apply_set_sha256',apply_set,
        'prepared_action_set_sha256',action_set,
        'residual_set_sha256',residual_set
    )::TEXT);
    IF header.status<>'building'
       OR member_count<>header.campaign_member_count
       OR (SELECT MIN(member.member_ordinal)
             FROM investigation_verification_task_advisory_members member
            WHERE member.advisory_receipt_id=NEW.advisory_receipt_id)<>0
       OR (SELECT MAX(member.member_ordinal)
             FROM investigation_verification_task_advisory_members member
            WHERE member.advisory_receipt_id=NEW.advisory_receipt_id)<>
          member_count-1
       OR member_set<>header.campaign_member_set_sha256
       OR expected_envelope<>header.envelope_sha256
       OR apply_count<>planned_obligation_count
       OR (SELECT COUNT(DISTINCT apply.campaign_id)
             FROM investigation_verification_advisory_campaign_applies apply
            WHERE apply.advisory_receipt_id=NEW.advisory_receipt_id)<>
          header.campaign_member_count
       OR NEW.campaign_apply_count<>apply_count
       OR NEW.campaign_apply_set_sha256<>apply_set
       OR NEW.prepared_action_count<>action_count
       OR NEW.prepared_action_set_sha256<>action_set
       OR NEW.residual_count<>residual_count
       OR NEW.residual_set_sha256<>residual_set
       OR NEW.seal_sha256<>expected_seal
       OR EXISTS(
            SELECT 1
              FROM investigation_verification_task_advisory_members member
              JOIN (SELECT DISTINCT advisory_receipt_id,advisory_member_id,campaign_id,
                                    strategy_artifact_id
                      FROM investigation_verification_advisory_campaign_applies) representative
                ON representative.advisory_receipt_id=member.advisory_receipt_id
               AND representative.advisory_member_id=member.advisory_member_id
               AND representative.campaign_id=member.campaign_id
              JOIN verification_strategy_obligations obligation
                ON obligation.strategy_artifact_id=representative.strategy_artifact_id
               AND obligation.disposition='planned'
             WHERE member.advisory_receipt_id=NEW.advisory_receipt_id
               AND NOT EXISTS(
                    SELECT 1 FROM investigation_verification_advisory_campaign_applies apply
                     WHERE apply.advisory_receipt_id=member.advisory_receipt_id
                       AND apply.advisory_member_id=member.advisory_member_id
                       AND apply.campaign_id=member.campaign_id
                       AND apply.strategy_artifact_id=representative.strategy_artifact_id
                       AND apply.strategy_obligation_id=obligation.obligation_id
               )
       )
       OR EXISTS(
            SELECT 1
              FROM investigation_verification_task_advisory_members member
              JOIN investigation_verification_advisory_campaign_applies apply
                ON apply.advisory_receipt_id=member.advisory_receipt_id
               AND apply.advisory_member_id=member.advisory_member_id
               AND apply.campaign_id=member.campaign_id
             WHERE member.advisory_receipt_id=NEW.advisory_receipt_id
             GROUP BY member.advisory_member_id
            HAVING COUNT(DISTINCT apply.strategy_artifact_id)<>1
       )
       OR EXISTS(
            SELECT 1
              FROM (
                   SELECT DISTINCT advisory_receipt_id,strategy_artifact_id
                     FROM investigation_verification_advisory_campaign_applies
                    WHERE advisory_receipt_id=NEW.advisory_receipt_id
              ) representative
              JOIN verification_strategy_artifacts strategy
                ON strategy.strategy_artifact_id=representative.strategy_artifact_id
             WHERE strategy.obligation_member_count<>(
                       SELECT COUNT(*)
                         FROM verification_strategy_obligations obligation
                        WHERE obligation.strategy_artifact_id=strategy.strategy_artifact_id
                   )
                OR (SELECT MIN(obligation.obligation_ordinal)
                      FROM verification_strategy_obligations obligation
                     WHERE obligation.strategy_artifact_id=
                           strategy.strategy_artifact_id)<>0
                OR (SELECT MAX(obligation.obligation_ordinal)
                      FROM verification_strategy_obligations obligation
                     WHERE obligation.strategy_artifact_id=
                           strategy.strategy_artifact_id)<>
                   strategy.obligation_member_count-1
                OR strategy.obligation_member_set_hash<>(
                       SELECT investigation_exact_member_set_hash(
                                  'verification_strategy_obligations.v1',
                                  COALESCE(array_agg(obligation.member_hash
                                                     ORDER BY obligation.obligation_ordinal),
                                           ARRAY[]::TEXT[])
                              )
                         FROM verification_strategy_obligations obligation
                        WHERE obligation.strategy_artifact_id=strategy.strategy_artifact_id
                   )
                OR EXISTS(
                       SELECT 1
                         FROM verification_strategy_obligations obligation
                        WHERE obligation.strategy_artifact_id=strategy.strategy_artifact_id
                          AND obligation.member_hash<>tool_truth_sha256(
                              jsonb_build_object(
                                  'ordinal',obligation.obligation_ordinal,
                                  'kind',obligation.obligation_kind,
                                  'semantic_key',obligation.semantic_key,
                                  'disposition',obligation.disposition,
                                  'residual_id',obligation.residual_id
                              )::TEXT
                          )
                   )
       )
       OR EXISTS(
            SELECT 1
              FROM investigation_verification_task_advisory_members member
              JOIN (
                   SELECT DISTINCT advisory_receipt_id,advisory_member_id,campaign_id,
                                   strategy_artifact_id,campaign_denominator_id
                     FROM investigation_verification_advisory_campaign_applies
              ) representative
                ON representative.advisory_receipt_id=member.advisory_receipt_id
               AND representative.advisory_member_id=member.advisory_member_id
               AND representative.campaign_id=member.campaign_id
             WHERE member.advisory_receipt_id=NEW.advisory_receipt_id
               AND EXISTS(
                    (SELECT obligation.semantic_key,obligation.obligation_kind
                       FROM verification_strategy_obligations obligation
                      WHERE obligation.strategy_artifact_id=
                            representative.strategy_artifact_id
                        AND obligation.disposition='planned'
                     EXCEPT
                     SELECT coverage.semantic_key,coverage.expected_capability_kind
                       FROM verification_campaign_coverage_members coverage
                      WHERE coverage.campaign_denominator_id=
                            representative.campaign_denominator_id
                        AND coverage.expected_capability_kind=member.capability_key)
                    UNION ALL
                    (SELECT coverage.semantic_key,coverage.expected_capability_kind
                       FROM verification_campaign_coverage_members coverage
                      WHERE coverage.campaign_denominator_id=
                            representative.campaign_denominator_id
                        AND coverage.expected_capability_kind=member.capability_key
                     EXCEPT
                     SELECT obligation.semantic_key,obligation.obligation_kind
                       FROM verification_strategy_obligations obligation
                      WHERE obligation.strategy_artifact_id=
                            representative.strategy_artifact_id
                        AND obligation.disposition='planned')
               )
       )
    THEN
        RAISE EXCEPTION 'INVESTIGATION_VERIFICATION_ADVISORY_SEAL_AUTHORITY_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER investigation_verification_advisory_member_append_only
BEFORE UPDATE OR DELETE ON investigation_verification_task_advisory_members
FOR EACH ROW EXECUTE FUNCTION reject_investigation_verification_advisory_mutation();
CREATE TRIGGER investigation_verification_advisory_member_guard
BEFORE INSERT ON investigation_verification_task_advisory_members
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_verification_advisory_member();
CREATE TRIGGER investigation_verification_advisory_apply_guard
BEFORE INSERT ON investigation_verification_advisory_campaign_applies
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_verification_advisory_campaign_apply();
CREATE TRIGGER investigation_verification_advisory_apply_append_only
BEFORE UPDATE OR DELETE ON investigation_verification_advisory_campaign_applies
FOR EACH ROW EXECUTE FUNCTION reject_investigation_verification_advisory_mutation();
CREATE TRIGGER investigation_verification_advisory_seal_guard
BEFORE INSERT ON investigation_verification_task_advisory_seals
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_verification_advisory_seal();
CREATE TRIGGER investigation_verification_advisory_seal_append_only
BEFORE UPDATE OR DELETE ON investigation_verification_task_advisory_seals
FOR EACH ROW EXECUTE FUNCTION reject_investigation_verification_advisory_mutation();
