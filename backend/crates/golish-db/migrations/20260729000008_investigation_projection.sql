-- Plan D investigation workspace foundation.
--
-- This migration only extends the Plan B projection/rollout authority.  The
-- unique source head, whole-batch outbox, materialized head/change ledger,
-- comparison samples and operation adoption receipts remain owned by 00006.

CREATE INDEX investigation_projection_changes_timeline_idx
    ON investigation_projection_changes(operation_id,change_seq,entity_kind,entity_id);

ALTER TABLE terminal_state
    ADD COLUMN investigation_workspace_json JSONB CHECK (
        investigation_workspace_json IS NULL
        OR jsonb_typeof(investigation_workspace_json)='object'
    );

ALTER TABLE report_source_manifest
    ADD COLUMN authority_class TEXT NOT NULL DEFAULT 'method_audit_only' CHECK (
        authority_class IN (
            'security_verdict_authority','grandfathered_legacy_security_verdict',
            'coverage_authority','execution_observation_audit','method_audit_only',
            'authorization_audit','historical_artifact_read_only'
        )
    );

ALTER TABLE report_claims
    ADD COLUMN authority_class TEXT NOT NULL DEFAULT 'method_audit_only' CHECK (
        authority_class IN (
            'security_verdict_authority','grandfathered_legacy_security_verdict',
            'coverage_authority','execution_observation_audit','method_audit_only',
            'authorization_audit','historical_artifact_read_only'
        )
    );

ALTER TABLE report_claim_citations
    ALTER COLUMN evidence_audit_id DROP NOT NULL;

ALTER TABLE report_source_manifest
    ADD CONSTRAINT report_source_manifest_authority_member_unique
    UNIQUE(revision_id,ordinal,authority_class,content_hash);

CREATE TABLE report_input_tool_truth_authority_sets (
    authority_set_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL UNIQUE REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    authority_member_count BIGINT NOT NULL CHECK (authority_member_count>0),
    authority_set_hash BYTEA NOT NULL CHECK (octet_length(authority_set_hash)=32),
    earliest_effective_valid_until TIMESTAMPTZ NOT NULL,
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (earliest_effective_valid_until>sealed_at),
    UNIQUE(authority_set_id,revision_id,operation_id)
);

CREATE TABLE report_input_tool_truth_authority_members (
    authority_set_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    organization_id UUID NOT NULL,
    tool_truth_authority_bundle_id UUID NOT NULL,
    typed_member JSONB NOT NULL CHECK (jsonb_typeof(typed_member)='object'),
    effective_valid_until TIMESTAMPTZ NOT NULL,
    member_hash BYTEA NOT NULL CHECK (octet_length(member_hash)=32),
    PRIMARY KEY(authority_set_id,ordinal),
    UNIQUE(authority_set_id,organization_id),
    UNIQUE(authority_set_id,member_hash),
    FOREIGN KEY(authority_set_id,revision_id,operation_id)
        REFERENCES report_input_tool_truth_authority_sets(
            authority_set_id,revision_id,operation_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(tool_truth_authority_bundle_id,operation_id,organization_id)
        REFERENCES tool_truth_authority_bundle_seals(id,operation_id,organization_id)
        ON DELETE RESTRICT
);

CREATE TABLE report_input_revision_adjudication_sets (
    authority_set_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL UNIQUE REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    authority_member_count BIGINT NOT NULL CHECK (authority_member_count>0),
    authority_set_hash BYTEA NOT NULL CHECK (octet_length(authority_set_hash)=32),
    coverage_membership_hash BYTEA NOT NULL CHECK (octet_length(coverage_membership_hash)=32),
    residual_membership_hash BYTEA NOT NULL CHECK (octet_length(residual_membership_hash)=32),
    earliest_effective_valid_until TIMESTAMPTZ NOT NULL,
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (earliest_effective_valid_until>sealed_at),
    UNIQUE(authority_set_id,revision_id,operation_id)
);

CREATE TABLE report_input_revision_adjudication_members (
    authority_set_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    organization_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    adjudication_tool_truth_bundle_id UUID NOT NULL,
    generation_seal_id UUID NOT NULL REFERENCES hypothesis_generation_seals(seal_id) ON DELETE RESTRICT,
    verification_plan_seal_id UUID NOT NULL REFERENCES attack_hypothesis_verification_plans(plan_id) ON DELETE RESTRICT,
    revision_adjudication_id UUID NOT NULL REFERENCES hypothesis_revision_adjudications(revision_adjudication_id) ON DELETE RESTRICT,
    adjudication_outcome TEXT NOT NULL CHECK (
        adjudication_outcome IN ('nonterminal','verified','refuted')
    ),
    revision_terminal_decision_id UUID REFERENCES hypothesis_revision_terminal_decisions(revision_terminal_decision_id) ON DELETE RESTRICT,
    revision_terminal_decision_hash TEXT CHECK (
        revision_terminal_decision_hash IS NULL
        OR revision_terminal_decision_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    final_wave_coverage_receipt_id UUID NOT NULL REFERENCES verification_wave_coverage_receipts(wave_coverage_receipt_id) ON DELETE RESTRICT,
    consolidation_receipt_id UUID REFERENCES hypothesis_consolidation_receipts(consolidation_receipt_id) ON DELETE RESTRICT,
    fixed_point_receipt_id UUID REFERENCES hypothesis_fixed_point_receipts(fixed_point_receipt_id) ON DELETE RESTRICT,
    typed_member JSONB NOT NULL CHECK (jsonb_typeof(typed_member)='object'),
    effective_valid_until TIMESTAMPTZ NOT NULL,
    member_hash BYTEA NOT NULL CHECK (octet_length(member_hash)=32),
    PRIMARY KEY(authority_set_id,ordinal),
    UNIQUE(authority_set_id,revision_terminal_decision_id),
    UNIQUE(authority_set_id,member_hash),
    CHECK (num_nonnulls(consolidation_receipt_id,fixed_point_receipt_id)=1),
    CHECK (
        (adjudication_outcome='nonterminal'
         AND revision_terminal_decision_id IS NULL
         AND revision_terminal_decision_hash IS NULL
         AND fixed_point_receipt_id IS NOT NULL)
        OR
        (adjudication_outcome IN ('verified','refuted')
         AND revision_terminal_decision_id IS NOT NULL
         AND revision_terminal_decision_hash IS NOT NULL)
    ),
    FOREIGN KEY(authority_set_id,revision_id,operation_id)
        REFERENCES report_input_revision_adjudication_sets(
            authority_set_id,revision_id,operation_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(adjudication_tool_truth_bundle_id,operation_id,organization_id)
        REFERENCES tool_truth_authority_bundle_seals(id,operation_id,organization_id)
        ON DELETE RESTRICT
);

CREATE TABLE report_input_open_headers (
    open_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL UNIQUE REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    authority_contract TEXT NOT NULL CHECK (
        authority_contract IN ('revision_adjudication','legacy')
    ),
    expected_source_member_count BIGINT NOT NULL CHECK (expected_source_member_count>0),
    expected_source_set_hash BYTEA NOT NULL CHECK (octet_length(expected_source_set_hash)=32),
    opened_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(open_id,revision_id),
    UNIQUE(open_id,revision_id,operation_id)
);

CREATE TABLE report_input_seals (
    seal_id UUID PRIMARY KEY,
    open_id UUID NOT NULL UNIQUE REFERENCES report_input_open_headers(open_id) ON DELETE RESTRICT,
    revision_id UUID NOT NULL UNIQUE REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    tool_truth_authority_set_id UUID NOT NULL,
    revision_adjudication_authority_set_id UUID,
    legacy_report_authority_seal_id UUID,
    source_member_count BIGINT NOT NULL CHECK (source_member_count>0),
    source_set_hash BYTEA NOT NULL CHECK (octet_length(source_set_hash)=32),
    typed_seal JSONB NOT NULL CHECK (jsonb_typeof(typed_seal)='object'),
    report_input_hash BYTEA NOT NULL CHECK (octet_length(report_input_hash)=32),
    effective_valid_until TIMESTAMPTZ NOT NULL,
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (effective_valid_until>sealed_at),
    CHECK ((revision_adjudication_authority_set_id IS NOT NULL)
           <> (legacy_report_authority_seal_id IS NOT NULL)),
    UNIQUE(seal_id,revision_id),
    FOREIGN KEY(open_id,revision_id,operation_id)
        REFERENCES report_input_open_headers(open_id,revision_id,operation_id) ON DELETE RESTRICT,
    FOREIGN KEY(tool_truth_authority_set_id,revision_id,operation_id)
        REFERENCES report_input_tool_truth_authority_sets(authority_set_id,revision_id,operation_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(revision_adjudication_authority_set_id,revision_id,operation_id)
        REFERENCES report_input_revision_adjudication_sets(
            authority_set_id,revision_id,operation_id
        )
        ON DELETE RESTRICT
);

CREATE TABLE report_input_seal_members (
    open_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    authority_class TEXT NOT NULL,
    source_hash BYTEA NOT NULL CHECK (octet_length(source_hash)=32),
    member_hash BYTEA NOT NULL CHECK (octet_length(member_hash)=32),
    PRIMARY KEY(open_id,ordinal),
    UNIQUE(open_id,member_hash),
    FOREIGN KEY(open_id,revision_id)
        REFERENCES report_input_open_headers(open_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(revision_id,ordinal,authority_class,source_hash)
        REFERENCES report_source_manifest(revision_id,ordinal,authority_class,content_hash)
        ON DELETE RESTRICT
);

ALTER TABLE candidate_attempts
    ADD CONSTRAINT candidate_attempts_plan_d_authority_unique
    UNIQUE(id,candidate_id,operation_id,organization_id);

CREATE TABLE investigation_projection_compare_aggregates (
    cohort_id UUID PRIMARY KEY,
    from_joint_rank SMALLINT NOT NULL CHECK (from_joint_rank BETWEEN 0 AND 5),
    to_joint_rank SMALLINT NOT NULL CHECK (to_joint_rank=from_joint_rank+1),
    criteria_version TEXT NOT NULL CHECK (BTRIM(criteria_version)<>''),
    projection_schema_version INTEGER NOT NULL DEFAULT 1 CHECK (projection_schema_version=1),
    cutoff_manifest_hash TEXT NOT NULL CHECK (cutoff_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    admitted_operation_count BIGINT NOT NULL CHECK (admitted_operation_count>=0),
    expected_record_count BIGINT NOT NULL CHECK (expected_record_count>=0),
    sampled_record_count BIGINT NOT NULL CHECK (sampled_record_count>=0),
    matched_record_count BIGINT NOT NULL CHECK (matched_record_count>=0),
    mismatch_record_count BIGINT NOT NULL CHECK (mismatch_record_count>=0),
    missing_record_count BIGINT NOT NULL CHECK (missing_record_count>=0),
    incomplete_record_count BIGINT NOT NULL CHECK (incomplete_record_count>=0),
    corrupt_record_count BIGINT NOT NULL CHECK (corrupt_record_count>=0),
    comparison_set_hash TEXT CHECK (
        comparison_set_hash IS NULL OR comparison_set_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    admission_closed BOOLEAN NOT NULL,
    aggregated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (sampled_record_count=matched_record_count+mismatch_record_count
        +missing_record_count+incomplete_record_count+corrupt_record_count),
    CHECK (sampled_record_count<=expected_record_count)
);

CREATE TABLE investigation_projection_compare_cohort_members (
    cohort_id UUID NOT NULL
        REFERENCES investigation_projection_compare_aggregates(cohort_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    as_of_change_seq BIGINT NOT NULL CHECK (as_of_change_seq>=0),
    expected_record_count BIGINT NOT NULL CHECK (expected_record_count>=0),
    sample_set_hash TEXT NOT NULL CHECK (sample_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(cohort_id,operation_id)
);

CREATE TABLE tool_truth_shadow_writer_readiness_receipts (
    receipt_id UUID PRIMARY KEY,
    criteria_version TEXT NOT NULL CHECK (BTRIM(criteria_version)<>''),
    deployment_digest TEXT NOT NULL CHECK (deployment_digest ~ '^sha256:[0-9a-f]{64}$'),
    observation_window_started_at TIMESTAMPTZ NOT NULL,
    observation_window_ended_at TIMESTAMPTZ NOT NULL,
    observed_operation_count BIGINT NOT NULL CHECK (observed_operation_count>0),
    readiness_member_count BIGINT NOT NULL CHECK (readiness_member_count=observed_operation_count),
    readiness_membership_hash TEXT NOT NULL CHECK (readiness_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    assessment_set_hash TEXT NOT NULL CHECK (assessment_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    missing_assessment_count BIGINT NOT NULL CHECK (missing_assessment_count=0),
    orphan_reconciliation_count BIGINT NOT NULL CHECK (orphan_reconciliation_count=0),
    corrupt_artifact_count BIGINT NOT NULL CHECK (corrupt_artifact_count=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (observation_window_ended_at>observation_window_started_at)
);

CREATE TABLE tool_truth_shadow_writer_readiness_members (
    receipt_id UUID NOT NULL
        REFERENCES tool_truth_shadow_writer_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    assessment_set_id UUID NOT NULL,
    assessment_set_hash TEXT NOT NULL CHECK (assessment_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    reconciliation_census_hash TEXT NOT NULL CHECK (reconciliation_census_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,operation_id)
);

CREATE TABLE registry_shadow_evaluator_readiness_receipts (
    receipt_id UUID PRIMARY KEY,
    criteria_version TEXT NOT NULL CHECK (BTRIM(criteria_version)<>''),
    evaluator_contract_version TEXT NOT NULL CHECK (BTRIM(evaluator_contract_version)<>''),
    evaluator_digest TEXT NOT NULL CHECK (evaluator_digest ~ '^sha256:[0-9a-f]{64}$'),
    fixture_manifest_hash TEXT NOT NULL CHECK (fixture_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    evaluation_count BIGINT NOT NULL CHECK (evaluation_count>0),
    evaluation_member_count BIGINT NOT NULL CHECK (evaluation_member_count=evaluation_count),
    evaluation_membership_hash TEXT NOT NULL CHECK (evaluation_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    external_port_call_count BIGINT NOT NULL CHECK (external_port_call_count=0),
    canonical_mutation_count BIGINT NOT NULL CHECK (canonical_mutation_count=0),
    incomplete_or_corrupt_count BIGINT NOT NULL CHECK (incomplete_or_corrupt_count=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE registry_shadow_evaluator_readiness_members (
    receipt_id UUID NOT NULL
        REFERENCES registry_shadow_evaluator_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    fixture_id TEXT NOT NULL CHECK (BTRIM(fixture_id)<>''),
    fixture_hash TEXT NOT NULL CHECK (fixture_hash ~ '^sha256:[0-9a-f]{64}$'),
    evaluation_result_hash TEXT NOT NULL CHECK (evaluation_result_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,fixture_id)
);

CREATE TABLE compatibility_projection_health_receipts (
    receipt_id UUID PRIMARY KEY,
    cohort_manifest_hash TEXT NOT NULL CHECK (cohort_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_projection_count BIGINT NOT NULL CHECK (expected_projection_count>=0),
    projection_member_count BIGINT NOT NULL CHECK (projection_member_count=expected_projection_count),
    projection_membership_hash TEXT NOT NULL CHECK (projection_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    pending_batch_count BIGINT NOT NULL CHECK (pending_batch_count=0),
    projection_error_count BIGINT NOT NULL CHECK (projection_error_count=0),
    divergence_count BIGINT NOT NULL CHECK (divergence_count=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE compatibility_projection_health_members (
    receipt_id UUID NOT NULL
        REFERENCES compatibility_projection_health_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    as_of_change_seq BIGINT NOT NULL CHECK (as_of_change_seq>=0),
    projection_head_hash TEXT NOT NULL CHECK (projection_head_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,operation_id)
);

CREATE TABLE authoritative_report_dry_run_receipts (
    receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    generation_seal_id UUID NOT NULL
        REFERENCES hypothesis_generation_seals(seal_id) ON DELETE RESTRICT,
    wave_coverage_receipt_id UUID NOT NULL
        REFERENCES verification_wave_coverage_receipts(wave_coverage_receipt_id) ON DELETE RESTRICT,
    report_input_hash TEXT NOT NULL CHECK (report_input_hash ~ '^sha256:[0-9a-f]{64}$'),
    renderer_contract_version TEXT NOT NULL CHECK (BTRIM(renderer_contract_version)<>''),
    redaction_sentinel_passed BOOLEAN NOT NULL CHECK (redaction_sentinel_passed),
    external_export_count BIGINT NOT NULL CHECK (external_export_count=0),
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,generation_seal_id,wave_coverage_receipt_id,report_input_hash)
);

CREATE TABLE historical_read_adapter_probe_receipts (
    receipt_id UUID PRIMARY KEY,
    adapter_version TEXT NOT NULL CHECK (BTRIM(adapter_version)<>''),
    adapter_digest TEXT NOT NULL CHECK (adapter_digest ~ '^sha256:[0-9a-f]{64}$'),
    expected_artifact_count BIGINT NOT NULL CHECK (expected_artifact_count>=0),
    expected_artifact_manifest_hash TEXT NOT NULL CHECK (expected_artifact_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    probed_artifact_count BIGINT NOT NULL CHECK (probed_artifact_count>=0),
    probe_member_count BIGINT NOT NULL CHECK (probe_member_count=probed_artifact_count),
    probe_membership_hash TEXT NOT NULL CHECK (probe_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    failed_probe_count BIGINT NOT NULL CHECK (failed_probe_count=0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (probed_artifact_count=expected_artifact_count)
);

CREATE TABLE historical_read_adapter_probe_members (
    receipt_id UUID NOT NULL
        REFERENCES historical_read_adapter_probe_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    report_revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    historical_artifact_receipt_id UUID NOT NULL,
    read_attestation_id UUID NOT NULL,
    attestation_hash TEXT NOT NULL CHECK (attestation_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,report_revision_id,historical_artifact_receipt_id)
);

CREATE TABLE legacy_consumer_retirement_receipts (
    receipt_id UUID PRIMARY KEY,
    criteria_version TEXT NOT NULL CHECK (BTRIM(criteria_version)<>''),
    observation_window_started_at TIMESTAMPTZ NOT NULL,
    observation_window_ended_at TIMESTAMPTZ NOT NULL,
    consumer_inventory_hash TEXT NOT NULL CHECK (consumer_inventory_hash ~ '^sha256:[0-9a-f]{64}$'),
    consumer_inventory_count BIGINT NOT NULL CHECK (consumer_inventory_count>=0),
    consumer_member_count BIGINT NOT NULL CHECK (consumer_member_count=consumer_inventory_count),
    consumer_membership_hash TEXT NOT NULL CHECK (consumer_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    unmigrated_consumer_count BIGINT NOT NULL CHECK (unmigrated_consumer_count=0),
    legacy_mutation_call_count BIGINT NOT NULL CHECK (legacy_mutation_call_count=0),
    legacy_read_fallback_call_count BIGINT NOT NULL CHECK (legacy_read_fallback_call_count=0),
    compatibility_projection_health_receipt_id UUID NOT NULL
        REFERENCES compatibility_projection_health_receipts(receipt_id) ON DELETE RESTRICT,
    historical_adapter_probe_receipt_id UUID NOT NULL
        REFERENCES historical_read_adapter_probe_receipts(receipt_id) ON DELETE RESTRICT,
    retirement_manifest_hash TEXT NOT NULL CHECK (retirement_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (observation_window_ended_at>observation_window_started_at)
);

CREATE TABLE legacy_consumer_retirement_members (
    receipt_id UUID NOT NULL
        REFERENCES legacy_consumer_retirement_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    consumer_key TEXT NOT NULL CHECK (BTRIM(consumer_key)<>''),
    consumer_binary_digest TEXT NOT NULL CHECK (consumer_binary_digest ~ '^sha256:[0-9a-f]{64}$'),
    retirement_evidence_hash TEXT NOT NULL CHECK (retirement_evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,consumer_key)
);

CREATE TABLE adversarial_acceptance_corpus_receipts (
    receipt_id UUID PRIMARY KEY,
    corpus_contract_version TEXT NOT NULL CHECK (BTRIM(corpus_contract_version)<>''),
    corpus_digest TEXT NOT NULL CHECK (corpus_digest ~ '^sha256:[0-9a-f]{64}$'),
    evaluator_binary_digest TEXT NOT NULL CHECK (evaluator_binary_digest ~ '^sha256:[0-9a-f]{64}$'),
    fixture_member_count BIGINT NOT NULL CHECK (fixture_member_count=9),
    fixture_membership_hash TEXT NOT NULL CHECK (fixture_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_outcome_membership_hash TEXT NOT NULL CHECK (expected_outcome_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    observed_outcome_membership_hash TEXT NOT NULL CHECK (observed_outcome_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    mismatch_count BIGINT NOT NULL CHECK (mismatch_count=0),
    missing_count BIGINT NOT NULL CHECK (missing_count=0),
    extra_count BIGINT NOT NULL CHECK (extra_count=0),
    sealed_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE adversarial_acceptance_corpus_members (
    receipt_id UUID NOT NULL
        REFERENCES adversarial_acceptance_corpus_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    fixture_kind TEXT NOT NULL CHECK (fixture_kind IN (
        'known_vulnerable','known_safe','control_failure','soft_404','waf_interstitial',
        'dynamic_content','multi_role_idor','race','adapter_missing'
    )),
    fixture_id TEXT NOT NULL CHECK (BTRIM(fixture_id)<>''),
    fixture_hash TEXT NOT NULL CHECK (fixture_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_verdict TEXT NOT NULL CHECK (expected_verdict IN (
        'verified','refuted','inconclusive','blocked','not_assessed'
    )),
    expected_residual_set_hash TEXT NOT NULL CHECK (expected_residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    observed_verdict TEXT NOT NULL CHECK (observed_verdict IN (
        'verified','refuted','inconclusive','blocked','not_assessed'
    )),
    observed_residual_set_hash TEXT NOT NULL CHECK (observed_residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    outcome_hash TEXT NOT NULL CHECK (outcome_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,fixture_kind,fixture_id),
    UNIQUE(receipt_id,fixture_kind),
    CHECK (expected_verdict=observed_verdict),
    CHECK (expected_residual_set_hash=observed_residual_set_hash)
);

CREATE TABLE operation_default_promotion_receipts (
    receipt_id UUID PRIMARY KEY,
    from_joint_rank SMALLINT NOT NULL CHECK (from_joint_rank BETWEEN 0 AND 5),
    to_joint_rank SMALLINT NOT NULL CHECK (to_joint_rank=from_joint_rank+1),
    criteria_version TEXT NOT NULL CHECK (BTRIM(criteria_version)<>''),
    tool_truth_from TEXT NOT NULL,
    tool_truth_to TEXT NOT NULL,
    investigation_contract_from TEXT NOT NULL,
    investigation_mode_from TEXT NOT NULL,
    investigation_contract_to TEXT NOT NULL,
    investigation_mode_to TEXT NOT NULL,
    cohort_id UUID REFERENCES investigation_projection_compare_aggregates(cohort_id) ON DELETE RESTRICT,
    cohort_cutoff_manifest_hash TEXT CHECK (
        cohort_cutoff_manifest_hash IS NULL OR cohort_cutoff_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    evidence_manifest_hash TEXT NOT NULL CHECK (evidence_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_member_count BIGINT NOT NULL CHECK (evidence_member_count>0),
    canary_manifest_hash TEXT CHECK (canary_manifest_hash IS NULL OR canary_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    adversarial_acceptance_receipt_id UUID
        REFERENCES adversarial_acceptance_corpus_receipts(receipt_id) ON DELETE RESTRICT,
    expected_tool_truth_row_version BIGINT NOT NULL CHECK (expected_tool_truth_row_version>=0),
    expected_investigation_row_version BIGINT NOT NULL CHECK (expected_investigation_row_version>=0),
    promoted_by_principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (BTRIM(reason)<>''),
    promoted_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(to_joint_rank,evidence_manifest_hash),
    CHECK ((cohort_id IS NULL)=(cohort_cutoff_manifest_hash IS NULL)),
    CHECK ((from_joint_rank IN (2,4))=(cohort_id IS NOT NULL)),
    CHECK ((from_joint_rank=4)=(canary_manifest_hash IS NOT NULL)),
    CHECK ((from_joint_rank=4)=(adversarial_acceptance_receipt_id IS NOT NULL)),
    CHECK (operation_joint_contract_rank(
        tool_truth_from,investigation_contract_from,investigation_mode_from
    )=from_joint_rank),
    CHECK (operation_joint_contract_rank(
        tool_truth_to,investigation_contract_to,investigation_mode_to
    )=to_joint_rank)
);

CREATE TABLE operation_default_promotion_evidence_members (
    receipt_id UUID NOT NULL
        REFERENCES operation_default_promotion_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    evidence_kind TEXT NOT NULL CHECK (evidence_kind IN (
        'tool_truth_shadow_writer_readiness_receipt',
        'registry_shadow_evaluator_readiness_receipt',
        'shadow_comparison_sample','tool_truth_all_fresh_authority_bundle',
        'dual_comparison_sample','authoritative_canary_action_receipt',
        'authoritative_canary_oracle_receipt','authoritative_canary_coverage_receipt',
        'authoritative_canary_revision_adjudication',
        'authoritative_canary_report_dry_run_receipt',
        'adversarial_acceptance_corpus_receipt','legacy_consumer_retirement_receipt'
    )),
    operation_id UUID REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    tool_truth_readiness_receipt_id UUID
        REFERENCES tool_truth_shadow_writer_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    registry_readiness_receipt_id UUID
        REFERENCES registry_shadow_evaluator_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    comparison_id UUID
        REFERENCES investigation_projection_compare_samples(comparison_id) ON DELETE RESTRICT,
    tool_truth_authority_bundle_id UUID
        REFERENCES tool_truth_authority_bundle_seals(id) ON DELETE RESTRICT,
    canary_action_execution_id UUID
        REFERENCES verification_action_executions(action_execution_id) ON DELETE RESTRICT,
    canary_oracle_assessment_id UUID
        REFERENCES verification_oracle_assessments(oracle_assessment_id) ON DELETE RESTRICT,
    canary_wave_coverage_receipt_id UUID
        REFERENCES verification_wave_coverage_receipts(wave_coverage_receipt_id) ON DELETE RESTRICT,
    canary_revision_adjudication_id UUID
        REFERENCES hypothesis_revision_adjudications(revision_adjudication_id) ON DELETE RESTRICT,
    canary_report_dry_run_receipt_id UUID
        REFERENCES authoritative_report_dry_run_receipts(receipt_id) ON DELETE RESTRICT,
    adversarial_acceptance_receipt_id UUID
        REFERENCES adversarial_acceptance_corpus_receipts(receipt_id) ON DELETE RESTRICT,
    legacy_retirement_receipt_id UUID
        REFERENCES legacy_consumer_retirement_receipts(receipt_id) ON DELETE RESTRICT,
    source_ref_hash TEXT NOT NULL CHECK (source_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,evidence_kind,member_hash),
    CHECK (num_nonnulls(
        tool_truth_readiness_receipt_id,registry_readiness_receipt_id,comparison_id,
        tool_truth_authority_bundle_id,canary_action_execution_id,
        canary_oracle_assessment_id,canary_wave_coverage_receipt_id,
        canary_revision_adjudication_id,canary_report_dry_run_receipt_id,
        adversarial_acceptance_receipt_id,
        legacy_retirement_receipt_id
    )=1),
    CHECK (CASE evidence_kind
        WHEN 'tool_truth_shadow_writer_readiness_receipt'
            THEN tool_truth_readiness_receipt_id IS NOT NULL
        WHEN 'registry_shadow_evaluator_readiness_receipt'
            THEN registry_readiness_receipt_id IS NOT NULL
        WHEN 'shadow_comparison_sample' THEN comparison_id IS NOT NULL
        WHEN 'tool_truth_all_fresh_authority_bundle'
            THEN tool_truth_authority_bundle_id IS NOT NULL
        WHEN 'dual_comparison_sample' THEN comparison_id IS NOT NULL
        WHEN 'authoritative_canary_action_receipt'
            THEN canary_action_execution_id IS NOT NULL
        WHEN 'authoritative_canary_oracle_receipt'
            THEN canary_oracle_assessment_id IS NOT NULL
        WHEN 'authoritative_canary_coverage_receipt'
            THEN canary_wave_coverage_receipt_id IS NOT NULL
        WHEN 'authoritative_canary_revision_adjudication'
            THEN canary_revision_adjudication_id IS NOT NULL
        WHEN 'authoritative_canary_report_dry_run_receipt'
            THEN canary_report_dry_run_receipt_id IS NOT NULL
        WHEN 'adversarial_acceptance_corpus_receipt'
            THEN adversarial_acceptance_receipt_id IS NOT NULL
        WHEN 'legacy_consumer_retirement_receipt'
            THEN legacy_retirement_receipt_id IS NOT NULL
        ELSE FALSE
    END)
);

CREATE TABLE operation_rollout_safety_hold_events (
    event_id UUID PRIMARY KEY,
    hold_scope TEXT NOT NULL CHECK (hold_scope IN ('campaign_dispatch','operation_admission')),
    previous_held BOOLEAN NOT NULL,
    next_held BOOLEAN NOT NULL,
    previous_scope_generation BIGINT NOT NULL CHECK (previous_scope_generation>=0),
    next_scope_generation BIGINT NOT NULL CHECK (next_scope_generation=previous_scope_generation+1),
    previous_row_version BIGINT NOT NULL CHECK (previous_row_version>=0),
    next_row_version BIGINT NOT NULL CHECK (next_row_version=previous_row_version+1),
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code)<>''),
    evidence_manifest_hash TEXT NOT NULL CHECK (evidence_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (previous_held IS DISTINCT FROM next_held)
);

CREATE OR REPLACE FUNCTION verification_guard_safety_hold_cas()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    authority_event_id UUID;
BEGIN
    IF TG_OP='DELETE' OR NEW.singleton IS DISTINCT FROM OLD.singleton
       OR NEW.row_version<>OLD.row_version+1
       OR NEW.campaign_dispatch_generation<OLD.campaign_dispatch_generation
       OR NEW.operation_admission_generation<OLD.operation_admission_generation
       OR NEW.campaign_dispatch_generation>OLD.campaign_dispatch_generation+1
       OR NEW.operation_admission_generation>OLD.operation_admission_generation+1
       OR ((NEW.campaign_dispatch_held IS DISTINCT FROM OLD.campaign_dispatch_held)
           <> (NEW.campaign_dispatch_generation=OLD.campaign_dispatch_generation+1))
       OR ((NEW.operation_admission_held IS DISTINCT FROM OLD.operation_admission_held)
           <> (NEW.operation_admission_generation=OLD.operation_admission_generation+1))
       OR (NEW.campaign_dispatch_held IS DISTINCT FROM OLD.campaign_dispatch_held)
          = (NEW.operation_admission_held IS DISTINCT FROM OLD.operation_admission_held)
       OR BTRIM(NEW.reason_code)=''
    THEN
        RAISE EXCEPTION 'VERIFICATION_SAFETY_HOLD_CAS_INVALID' USING ERRCODE='23514';
    END IF;
    authority_event_id := NULLIF(current_setting(
        'golish.operation_safety_hold_event_id',TRUE
    ),'')::UUID;
    IF authority_event_id IS NULL OR NOT EXISTS(
        SELECT 1 FROM operation_rollout_safety_hold_events event
         WHERE event.event_id=authority_event_id
           AND event.hold_scope=CASE
               WHEN NEW.campaign_dispatch_held IS DISTINCT FROM OLD.campaign_dispatch_held
                   THEN 'campaign_dispatch'
               ELSE 'operation_admission'
           END
           AND event.previous_held=CASE
               WHEN event.hold_scope='campaign_dispatch' THEN OLD.campaign_dispatch_held
               ELSE OLD.operation_admission_held
           END
           AND event.next_held=CASE
               WHEN event.hold_scope='campaign_dispatch' THEN NEW.campaign_dispatch_held
               ELSE NEW.operation_admission_held
           END
           AND event.previous_scope_generation=CASE
               WHEN event.hold_scope='campaign_dispatch'
                   THEN OLD.campaign_dispatch_generation
               ELSE OLD.operation_admission_generation
           END
           AND event.next_scope_generation=CASE
               WHEN event.hold_scope='campaign_dispatch'
                   THEN NEW.campaign_dispatch_generation
               ELSE NEW.operation_admission_generation
           END
           AND event.previous_row_version=OLD.row_version
           AND event.next_row_version=NEW.row_version
           AND event.reason_code=NEW.reason_code
    ) THEN
        RAISE EXCEPTION 'VERIFICATION_SAFETY_HOLD_EVENT_REQUIRED' USING ERRCODE='23514';
    END IF;
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;

-- Replace the Plan A deliberately-absent production setter with one narrow
-- event-bound local-admin CAS.  Historical initialized events remain valid;
-- every release/hold transition must carry complete operator evidence.
ALTER TABLE tool_truth_revalidation_dispatch_events
    ADD COLUMN previous_dispatch_state TEXT CHECK (
        previous_dispatch_state IS NULL
        OR previous_dispatch_state IN ('held','released')
    ),
    ADD COLUMN previous_row_version BIGINT CHECK (
        previous_row_version IS NULL OR previous_row_version>=0
    ),
    ADD COLUMN next_row_version BIGINT CHECK (
        next_row_version IS NULL OR next_row_version>=1
    ),
    ADD COLUMN reason_code TEXT CHECK (
        reason_code IS NULL OR BTRIM(reason_code)<>''
    ),
    ADD COLUMN evidence_manifest_hash TEXT CHECK (
        evidence_manifest_hash IS NULL
        OR evidence_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    ADD COLUMN principal_id UUID REFERENCES operator_principals(id) ON DELETE RESTRICT,
    ADD CONSTRAINT tool_truth_revalidation_admin_event_complete CHECK (
        event_type='initialized'
        OR (previous_dispatch_state IS NOT NULL
            AND previous_dispatch_state<>dispatch_state
            AND previous_row_version IS NOT NULL
            AND next_row_version=previous_row_version+1
            AND reason_code IS NOT NULL
            AND evidence_manifest_hash IS NOT NULL
            AND principal_id IS NOT NULL)
    );

CREATE OR REPLACE FUNCTION tool_truth_initialize_revalidation_control()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    policy_hash TEXT;
    source_operation_id UUID;
    selected_dispatch_mode TEXT;
    selected_max_risk_tier TEXT;
    selected_max_attempts INTEGER;
    selected_max_retries INTEGER;
    selected_max_no_progress INTEGER;
    selected_lease_seconds INTEGER;
    selected_deadline_seconds INTEGER;
BEGIN
    source_operation_id := NULLIF(current_setting(
        'golish.revalidation_policy_source_operation_id',TRUE
    ),'')::UUID;
    IF source_operation_id IS NULL THEN
        SELECT 'manual_only','T1',3,2,2,60,3600
          INTO selected_dispatch_mode,selected_max_risk_tier,
               selected_max_attempts,selected_max_retries,
               selected_max_no_progress,selected_lease_seconds,
               selected_deadline_seconds;
    ELSE
        SELECT dispatch_mode,max_risk_tier,max_attempts,max_retries,
               max_no_progress,lease_seconds,deadline_seconds
          INTO STRICT selected_dispatch_mode,selected_max_risk_tier,
               selected_max_attempts,selected_max_retries,
               selected_max_no_progress,selected_lease_seconds,
               selected_deadline_seconds
          FROM tool_truth_revalidation_dispatch_policies
         WHERE operation_id=source_operation_id FOR SHARE;
    END IF;
    policy_hash := tool_truth_sha256(jsonb_build_object(
        'operation_id',NEW.operation_id,
        'dispatch_mode',selected_dispatch_mode,
        'max_risk_tier',selected_max_risk_tier,
        'max_attempts',selected_max_attempts,
        'max_retries',selected_max_retries,
        'max_no_progress',selected_max_no_progress,
        'lease_seconds',selected_lease_seconds,
        'deadline_seconds',selected_deadline_seconds
    )::TEXT);
    INSERT INTO tool_truth_revalidation_dispatch_policies(
        operation_id,dispatch_mode,max_risk_tier,max_attempts,max_retries,
        max_no_progress,lease_seconds,deadline_seconds,policy_hash
    ) VALUES(
        NEW.operation_id,selected_dispatch_mode,selected_max_risk_tier,
        selected_max_attempts,selected_max_retries,selected_max_no_progress,
        selected_lease_seconds,selected_deadline_seconds,policy_hash
    );
    INSERT INTO tool_truth_revalidation_dispatch_heads(operation_id)
    VALUES(NEW.operation_id);
    INSERT INTO tool_truth_revalidation_dispatch_events(
        id,operation_id,generation,event_type,dispatch_state,event_hash
    ) VALUES(
        gen_random_uuid(),NEW.operation_id,0,'initialized','held',
        tool_truth_sha256(jsonb_build_object(
            'operation_id',NEW.operation_id,'generation',0,
            'event_type','initialized','dispatch_state','held',
            'policy_hash',policy_hash
        )::TEXT)
    );
    RETURN NEW;
END;
$$;

DROP TRIGGER tool_truth_revalidation_dispatch_head_immutable
    ON tool_truth_revalidation_dispatch_heads;
CREATE FUNCTION tool_truth_guard_revalidation_dispatch_cas_v1()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    authority_event_id UUID;
BEGIN
    IF TG_OP='DELETE' OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
       OR NEW.dispatch_state=OLD.dispatch_state
       OR NEW.generation<>OLD.generation+1
       OR NEW.row_version<>OLD.row_version+1
       OR NEW.updated_at<=OLD.updated_at
    THEN
        RAISE EXCEPTION 'tool_truth_revalidation_dispatch_cas_invalid'
            USING ERRCODE='23514';
    END IF;
    authority_event_id := NULLIF(current_setting(
        'golish.tool_truth_revalidation_dispatch_event_id',TRUE
    ),'')::UUID;
    IF authority_event_id IS NULL OR NOT EXISTS(
        SELECT 1 FROM tool_truth_revalidation_dispatch_events event
         WHERE event.id=authority_event_id
           AND event.operation_id=OLD.operation_id
           AND event.generation=NEW.generation
           AND event.dispatch_state=NEW.dispatch_state
           AND event.previous_dispatch_state=OLD.dispatch_state
           AND event.previous_row_version=OLD.row_version
           AND event.next_row_version=NEW.row_version
           AND event.event_type=NEW.dispatch_state
    ) THEN
        RAISE EXCEPTION 'tool_truth_revalidation_dispatch_event_required'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER tool_truth_revalidation_dispatch_head_admin_cas
BEFORE UPDATE OR DELETE ON tool_truth_revalidation_dispatch_heads
FOR EACH ROW EXECUTE FUNCTION tool_truth_guard_revalidation_dispatch_cas_v1();

CREATE OR REPLACE FUNCTION tool_truth_reject_rollout_direct_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    promotion_id UUID;
BEGIN
    IF TG_OP<>'UPDATE' OR NEW.singleton IS DISTINCT FROM OLD.singleton
       OR NEW.row_version<>OLD.row_version+1
       OR NEW.updated_at<=OLD.updated_at
    THEN
        RAISE EXCEPTION 'tool_truth_rollout_direct_mutation_forbidden' USING ERRCODE='23514';
    END IF;
    promotion_id := NULLIF(current_setting(
        'golish.operation_default_promotion_receipt_id',TRUE
    ),'')::UUID;
    IF promotion_id IS NULL OR NOT EXISTS(
        SELECT 1 FROM operation_default_promotion_receipts receipt
         WHERE receipt.receipt_id=promotion_id
           AND receipt.tool_truth_from=OLD.new_operation_contract
           AND receipt.tool_truth_to=NEW.new_operation_contract
           AND receipt.expected_tool_truth_row_version=OLD.row_version
    ) THEN
        RAISE EXCEPTION 'tool_truth_rollout_direct_mutation_forbidden' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION investigation_reject_rollout_direct_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    promotion_id UUID;
BEGIN
    IF TG_OP<>'UPDATE' OR NEW.singleton IS DISTINCT FROM OLD.singleton
       OR NEW.row_version<>OLD.row_version+1
       OR NEW.updated_at<=OLD.updated_at
    THEN
        RAISE EXCEPTION 'investigation_rollout_direct_mutation_forbidden' USING ERRCODE='23514';
    END IF;
    promotion_id := NULLIF(current_setting(
        'golish.operation_default_promotion_receipt_id',TRUE
    ),'')::UUID;
    IF promotion_id IS NULL OR NOT EXISTS(
        SELECT 1 FROM operation_default_promotion_receipts receipt
         WHERE receipt.receipt_id=promotion_id
           AND receipt.investigation_contract_from=OLD.contract_version
           AND receipt.investigation_mode_from=OLD.rollout_mode
           AND receipt.investigation_contract_to=NEW.contract_version
           AND receipt.investigation_mode_to=NEW.rollout_mode
           AND receipt.expected_investigation_row_version=OLD.row_version
    ) THEN
        RAISE EXCEPTION 'investigation_rollout_direct_mutation_forbidden' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TABLE legacy_attempt_refutation_receipts (
    receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    candidate_id UUID NOT NULL REFERENCES attack_candidates(candidate_id) ON DELETE RESTRICT,
    attempt_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    candidate_snapshot_hash TEXT NOT NULL CHECK (candidate_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'),
    attempt_terminal_hash TEXT NOT NULL CHECK (attempt_terminal_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_membership_hash TEXT NOT NULL CHECK (evidence_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    refutation_hash TEXT NOT NULL CHECK (refutation_hash ~ '^sha256:[0-9a-f]{64}$'),
    adapter_version TEXT NOT NULL CHECK (BTRIM(adapter_version)<>''),
    adapter_digest TEXT NOT NULL CHECK (adapter_digest ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,attempt_id,adapter_version),
    UNIQUE(receipt_id,operation_id,attempt_id,hypothesis_revision_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(attempt_id,candidate_id,operation_id,organization_id)
        REFERENCES candidate_attempts(id,candidate_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT
);

CREATE TABLE legacy_attempt_authority_receipts (
    receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    terminal_status TEXT NOT NULL CHECK (terminal_status IN ('verified','refuted')),
    source_record_hash TEXT NOT NULL CHECK (source_record_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_member_count BIGINT NOT NULL CHECK (source_member_count>0),
    source_membership_hash TEXT NOT NULL CHECK (source_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_membership_hash TEXT NOT NULL CHECK (evidence_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    finding_id UUID REFERENCES findings(id) ON DELETE RESTRICT,
    refutation_receipt_id UUID,
    limitation_membership_hash TEXT NOT NULL CHECK (limitation_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    adapter_version TEXT NOT NULL CHECK (BTRIM(adapter_version)<>''),
    adapter_digest TEXT NOT NULL CHECK (adapter_digest ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,attempt_id,adapter_version),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(attempt_id,candidate_id,operation_id,organization_id)
        REFERENCES candidate_attempts(id,candidate_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(hypothesis_revision_id,operation_id,organization_id)
        REFERENCES attack_hypothesis_revisions(revision_id,operation_id,organization_id)
        ON DELETE RESTRICT,
    FOREIGN KEY(refutation_receipt_id,operation_id,attempt_id,hypothesis_revision_id)
        REFERENCES legacy_attempt_refutation_receipts(
            receipt_id,operation_id,attempt_id,hypothesis_revision_id
        ) ON DELETE RESTRICT,
    CHECK (
        (terminal_status='verified' AND finding_id IS NOT NULL AND refutation_receipt_id IS NULL)
        OR (terminal_status='refuted' AND finding_id IS NULL AND refutation_receipt_id IS NOT NULL)
    )
);

CREATE TABLE legacy_attempt_authority_source_members (
    receipt_id UUID NOT NULL
        REFERENCES legacy_attempt_authority_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    source_kind TEXT NOT NULL CHECK (source_kind IN (
        'candidate_snapshot','attempt_terminal','evidence','finding_lineage',
        'refutation_lineage','limitation'
    )),
    source_ref_id TEXT NOT NULL CHECK (BTRIM(source_ref_id)<>''),
    source_ref_hash TEXT NOT NULL CHECK (source_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,source_kind,source_ref_id)
);

ALTER TABLE legacy_attempt_authority_receipts
    ADD CONSTRAINT legacy_attempt_authority_receipt_operation_unique
    UNIQUE(receipt_id,operation_id);

CREATE TABLE legacy_report_authority_seals (
    seal_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    source_member_count BIGINT NOT NULL CHECK (source_member_count>0),
    source_membership_hash TEXT NOT NULL CHECK (source_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    limitation_membership_hash TEXT NOT NULL CHECK (limitation_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    adapter_version TEXT NOT NULL CHECK (BTRIM(adapter_version)<>''),
    adapter_digest TEXT NOT NULL CHECK (adapter_digest ~ '^sha256:[0-9a-f]{64}$'),
    seal_hash TEXT NOT NULL UNIQUE CHECK (seal_hash ~ '^sha256:[0-9a-f]{64}$'),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,adapter_version),
    UNIQUE(seal_id,operation_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE legacy_report_authority_members (
    seal_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=0),
    legacy_attempt_authority_receipt_id UUID NOT NULL,
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(seal_id,ordinal),
    UNIQUE(seal_id,legacy_attempt_authority_receipt_id),
    FOREIGN KEY(seal_id,operation_id)
        REFERENCES legacy_report_authority_seals(seal_id,operation_id) ON DELETE RESTRICT,
    FOREIGN KEY(legacy_attempt_authority_receipt_id,operation_id)
        REFERENCES legacy_attempt_authority_receipts(receipt_id,operation_id) ON DELETE RESTRICT
);

ALTER TABLE report_input_seals
    ADD CONSTRAINT report_input_legacy_authority_fk
    FOREIGN KEY(legacy_report_authority_seal_id,operation_id)
    REFERENCES legacy_report_authority_seals(seal_id,operation_id)
    ON DELETE RESTRICT;

CREATE TABLE report_authority_invalidation_events (
    event_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL,
    report_revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    report_input_seal_id UUID NOT NULL,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    dependency_kind TEXT NOT NULL CHECK (dependency_kind IN (
        'tool_truth_authority_member','revision_adjudication_member'
    )),
    tool_truth_authority_set_id UUID,
    tool_truth_authority_member_ordinal INTEGER CHECK (
        tool_truth_authority_member_ordinal IS NULL
        OR tool_truth_authority_member_ordinal>=0
    ),
    revision_adjudication_authority_set_id UUID,
    revision_adjudication_member_ordinal INTEGER CHECK (
        revision_adjudication_member_ordinal IS NULL
        OR revision_adjudication_member_ordinal>=0
    ),
    origin_kind TEXT NOT NULL CHECK (origin_kind IN (
        'tool_truth_semantic_orphan','verification_authority_quarantine'
    )),
    origin_id UUID NOT NULL,
    tool_truth_orphan_reconciliation_id UUID
        REFERENCES capability_execution_reconciliations(id) ON DELETE RESTRICT,
    verification_quarantine_event_id UUID
        REFERENCES verification_authority_quarantine_events(quarantine_event_id) ON DELETE RESTRICT,
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'semantic_authority_orphaned','verification_authority_quarantined'
    )),
    source_batch_id UUID NOT NULL
        REFERENCES investigation_projection_outbox_batches(batch_id) ON DELETE RESTRICT,
    event_hash TEXT NOT NULL UNIQUE CHECK (event_hash ~ '^sha256:[0-9a-f]{64}$'),
    invalidated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (dependency_kind='tool_truth_authority_member'
         AND tool_truth_authority_set_id IS NOT NULL
         AND tool_truth_authority_member_ordinal IS NOT NULL
         AND revision_adjudication_authority_set_id IS NULL
         AND revision_adjudication_member_ordinal IS NULL)
        OR
        (dependency_kind='revision_adjudication_member'
         AND tool_truth_authority_set_id IS NULL
         AND tool_truth_authority_member_ordinal IS NULL
         AND revision_adjudication_authority_set_id IS NOT NULL
         AND revision_adjudication_member_ordinal IS NOT NULL)
    ),
    CHECK (
        (origin_kind='tool_truth_semantic_orphan'
         AND origin_id=tool_truth_orphan_reconciliation_id
         AND tool_truth_orphan_reconciliation_id IS NOT NULL
         AND verification_quarantine_event_id IS NULL
         AND reason_code='semantic_authority_orphaned')
        OR
        (origin_kind='verification_authority_quarantine'
         AND origin_id=verification_quarantine_event_id
         AND tool_truth_orphan_reconciliation_id IS NULL
         AND verification_quarantine_event_id IS NOT NULL
         AND reason_code='verification_authority_quarantined')
    ),
    UNIQUE(report_revision_id,dependency_kind,origin_kind,origin_id),
    UNIQUE(event_id,source_batch_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(report_input_seal_id,report_revision_id)
        REFERENCES report_input_seals(seal_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(tool_truth_authority_set_id,tool_truth_authority_member_ordinal)
        REFERENCES report_input_tool_truth_authority_members(authority_set_id,ordinal)
        ON DELETE RESTRICT,
    FOREIGN KEY(revision_adjudication_authority_set_id,revision_adjudication_member_ordinal)
        REFERENCES report_input_revision_adjudication_members(authority_set_id,ordinal)
        ON DELETE RESTRICT,
    FOREIGN KEY(verification_quarantine_event_id,operation_id,project_scope_id,organization_id)
        REFERENCES verification_authority_quarantine_events(
            quarantine_event_id,operation_id,project_scope_id,organization_id
        ) ON DELETE RESTRICT
);

ALTER TABLE report_revision_artifacts
    ADD CONSTRAINT report_revision_artifact_content_unique
    UNIQUE(revision_id,artifact_kind,content_key);

ALTER TABLE reports
    ADD CONSTRAINT reports_operation_scope_identity_unique
    UNIQUE(report_id,operation_id,project_scope_id);

CREATE TABLE historical_report_artifact_receipts (
    receipt_id UUID PRIMARY KEY,
    report_id UUID NOT NULL,
    revision_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    artifact_kind TEXT NOT NULL CHECK (BTRIM(artifact_kind)<>''),
    content_key TEXT NOT NULL,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    storage_path TEXT NOT NULL CHECK (BTRIM(storage_path)<>''),
    byte_len BIGINT NOT NULL CHECK (byte_len>=0),
    metadata_manifest_hash TEXT NOT NULL UNIQUE CHECK (metadata_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(revision_id,artifact_kind),
    UNIQUE(receipt_id,operation_id),
    UNIQUE(receipt_id,revision_id),
    FOREIGN KEY(report_id,operation_id,project_scope_id)
        REFERENCES reports(report_id,operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY(report_id,revision_id)
        REFERENCES report_revisions(report_id,revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(revision_id,artifact_kind,content_key)
        REFERENCES report_revision_artifacts(revision_id,artifact_kind,content_key) ON DELETE RESTRICT
);

CREATE TABLE historical_report_artifact_read_attestations (
    attestation_id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    request_private_snapshot_hash TEXT NOT NULL CHECK (
        request_private_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    observed_sha256 TEXT NOT NULL CHECK (observed_sha256 ~ '^[0-9a-f]{64}$'),
    observed_byte_len BIGINT NOT NULL CHECK (observed_byte_len>=0),
    authority_time_status TEXT NOT NULL CHECK (
        authority_time_status IN ('as_of_fresh','temporally_stale','revoked_history')
    ),
    attestation_hash TEXT NOT NULL UNIQUE CHECK (attestation_hash ~ '^sha256:[0-9a-f]{64}$'),
    attested_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(receipt_id,request_private_snapshot_hash),
    UNIQUE(attestation_id,receipt_id),
    FOREIGN KEY(receipt_id,operation_id)
        REFERENCES historical_report_artifact_receipts(receipt_id,operation_id) ON DELETE RESTRICT
);

ALTER TABLE historical_read_adapter_probe_members
    ADD CONSTRAINT historical_probe_artifact_revision_authority_fk
        FOREIGN KEY(historical_artifact_receipt_id,report_revision_id)
        REFERENCES historical_report_artifact_receipts(receipt_id,revision_id)
        ON DELETE RESTRICT,
    ADD CONSTRAINT historical_probe_attestation_authority_fk
        FOREIGN KEY(read_attestation_id,historical_artifact_receipt_id)
        REFERENCES historical_report_artifact_read_attestations(attestation_id,receipt_id)
        ON DELETE RESTRICT;

CREATE FUNCTION investigation_validate_exact_member_count_v1()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    owner_id UUID;
    expected_count BIGINT;
    actual_count BIGINT;
BEGIN
    owner_id := (to_jsonb(NEW)->>TG_ARGV[0])::UUID;
    EXECUTE format(
        'SELECT %I FROM %I WHERE %I=$1',TG_ARGV[3],TG_ARGV[1],TG_ARGV[2]
    ) INTO expected_count USING owner_id;
    EXECUTE format(
        'SELECT COUNT(*) FROM %I WHERE %I=$1',TG_ARGV[4],TG_ARGV[5]
    ) INTO actual_count USING owner_id;
    IF expected_count IS NULL OR actual_count IS DISTINCT FROM expected_count THEN
        RAISE EXCEPTION 'INVESTIGATION_RECEIPT_MEMBER_SET_INCOMPLETE'
            USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE CONSTRAINT TRIGGER investigation_compare_aggregate_exact_members
AFTER INSERT ON investigation_projection_compare_aggregates
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'cohort_id','investigation_projection_compare_aggregates','cohort_id',
    'admitted_operation_count','investigation_projection_compare_cohort_members','cohort_id'
);
CREATE CONSTRAINT TRIGGER investigation_compare_member_exact_set
AFTER INSERT ON investigation_projection_compare_cohort_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'cohort_id','investigation_projection_compare_aggregates','cohort_id',
    'admitted_operation_count','investigation_projection_compare_cohort_members','cohort_id'
);

CREATE CONSTRAINT TRIGGER tool_truth_readiness_receipt_exact_members
AFTER INSERT ON tool_truth_shadow_writer_readiness_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','tool_truth_shadow_writer_readiness_receipts','receipt_id',
    'readiness_member_count','tool_truth_shadow_writer_readiness_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER tool_truth_readiness_member_exact_set
AFTER INSERT ON tool_truth_shadow_writer_readiness_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','tool_truth_shadow_writer_readiness_receipts','receipt_id',
    'readiness_member_count','tool_truth_shadow_writer_readiness_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER registry_readiness_receipt_exact_members
AFTER INSERT ON registry_shadow_evaluator_readiness_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','registry_shadow_evaluator_readiness_receipts','receipt_id',
    'evaluation_member_count','registry_shadow_evaluator_readiness_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER registry_readiness_member_exact_set
AFTER INSERT ON registry_shadow_evaluator_readiness_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','registry_shadow_evaluator_readiness_receipts','receipt_id',
    'evaluation_member_count','registry_shadow_evaluator_readiness_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER compatibility_health_receipt_exact_members
AFTER INSERT ON compatibility_projection_health_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','compatibility_projection_health_receipts','receipt_id',
    'projection_member_count','compatibility_projection_health_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER compatibility_health_member_exact_set
AFTER INSERT ON compatibility_projection_health_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','compatibility_projection_health_receipts','receipt_id',
    'projection_member_count','compatibility_projection_health_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER historical_probe_receipt_exact_members
AFTER INSERT ON historical_read_adapter_probe_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','historical_read_adapter_probe_receipts','receipt_id',
    'probe_member_count','historical_read_adapter_probe_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER historical_probe_member_exact_set
AFTER INSERT ON historical_read_adapter_probe_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','historical_read_adapter_probe_receipts','receipt_id',
    'probe_member_count','historical_read_adapter_probe_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER legacy_retirement_receipt_exact_members
AFTER INSERT ON legacy_consumer_retirement_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','legacy_consumer_retirement_receipts','receipt_id',
    'consumer_member_count','legacy_consumer_retirement_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER legacy_retirement_member_exact_set
AFTER INSERT ON legacy_consumer_retirement_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','legacy_consumer_retirement_receipts','receipt_id',
    'consumer_member_count','legacy_consumer_retirement_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER adversarial_acceptance_receipt_exact_members
AFTER INSERT ON adversarial_acceptance_corpus_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','adversarial_acceptance_corpus_receipts','receipt_id',
    'fixture_member_count','adversarial_acceptance_corpus_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER adversarial_acceptance_member_exact_set
AFTER INSERT ON adversarial_acceptance_corpus_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','adversarial_acceptance_corpus_receipts','receipt_id',
    'fixture_member_count','adversarial_acceptance_corpus_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER operation_promotion_receipt_exact_members
AFTER INSERT ON operation_default_promotion_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','operation_default_promotion_receipts','receipt_id',
    'evidence_member_count','operation_default_promotion_evidence_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER operation_promotion_member_exact_set
AFTER INSERT ON operation_default_promotion_evidence_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','operation_default_promotion_receipts','receipt_id',
    'evidence_member_count','operation_default_promotion_evidence_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER legacy_attempt_authority_receipt_exact_members
AFTER INSERT ON legacy_attempt_authority_receipts
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','legacy_attempt_authority_receipts','receipt_id',
    'source_member_count','legacy_attempt_authority_source_members','receipt_id'
);
CREATE CONSTRAINT TRIGGER legacy_attempt_authority_member_exact_set
AFTER INSERT ON legacy_attempt_authority_source_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'receipt_id','legacy_attempt_authority_receipts','receipt_id',
    'source_member_count','legacy_attempt_authority_source_members','receipt_id'
);

CREATE CONSTRAINT TRIGGER legacy_report_authority_seal_exact_members
AFTER INSERT ON legacy_report_authority_seals
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'seal_id','legacy_report_authority_seals','seal_id',
    'source_member_count','legacy_report_authority_members','seal_id'
);
CREATE CONSTRAINT TRIGGER legacy_report_authority_member_exact_set
AFTER INSERT ON legacy_report_authority_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'seal_id','legacy_report_authority_seals','seal_id',
    'source_member_count','legacy_report_authority_members','seal_id'
);

CREATE CONSTRAINT TRIGGER report_input_seal_exact_members
AFTER INSERT ON report_input_seals
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'open_id','report_input_seals','open_id',
    'source_member_count','report_input_seal_members','open_id'
);
CREATE CONSTRAINT TRIGGER report_input_seal_member_exact_set
AFTER INSERT ON report_input_seal_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'seal_id','report_input_seals','seal_id',
    'source_member_count','report_input_seal_members','seal_id'
);

CREATE CONSTRAINT TRIGGER report_tool_truth_authority_set_exact_members
AFTER INSERT ON report_input_tool_truth_authority_sets
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'authority_set_id','report_input_tool_truth_authority_sets','authority_set_id',
    'authority_member_count','report_input_tool_truth_authority_members','authority_set_id'
);
CREATE CONSTRAINT TRIGGER report_tool_truth_authority_member_exact_set
AFTER INSERT ON report_input_tool_truth_authority_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'authority_set_id','report_input_tool_truth_authority_sets','authority_set_id',
    'authority_member_count','report_input_tool_truth_authority_members','authority_set_id'
);

CREATE CONSTRAINT TRIGGER report_revision_adjudication_set_exact_members
AFTER INSERT ON report_input_revision_adjudication_sets
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'authority_set_id','report_input_revision_adjudication_sets','authority_set_id',
    'authority_member_count','report_input_revision_adjudication_members','authority_set_id'
);
CREATE CONSTRAINT TRIGGER report_revision_adjudication_member_exact_set
AFTER INSERT ON report_input_revision_adjudication_members
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
EXECUTE FUNCTION investigation_validate_exact_member_count_v1(
    'authority_set_id','report_input_revision_adjudication_sets','authority_set_id',
    'authority_member_count','report_input_revision_adjudication_members','authority_set_id'
);

-- Every Plan D receipt/event is immutable evidence.  Member exact-set hashes
-- are recomputed by the typed repository before the header is inserted; both
-- header and ordered members become append-only at commit.
CREATE TRIGGER investigation_projection_compare_aggregates_append_only
BEFORE UPDATE OR DELETE ON investigation_projection_compare_aggregates
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_input_open_headers_append_only
BEFORE UPDATE OR DELETE ON report_input_open_headers
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_input_tool_truth_authority_sets_append_only
BEFORE UPDATE OR DELETE ON report_input_tool_truth_authority_sets
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_input_tool_truth_authority_members_append_only
BEFORE UPDATE OR DELETE ON report_input_tool_truth_authority_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_input_revision_adjudication_sets_append_only
BEFORE UPDATE OR DELETE ON report_input_revision_adjudication_sets
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_input_revision_adjudication_members_append_only
BEFORE UPDATE OR DELETE ON report_input_revision_adjudication_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_input_seals_append_only
BEFORE UPDATE OR DELETE ON report_input_seals
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_input_seal_members_append_only
BEFORE UPDATE OR DELETE ON report_input_seal_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER investigation_projection_compare_cohort_members_append_only
BEFORE UPDATE OR DELETE ON investigation_projection_compare_cohort_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER tool_truth_shadow_writer_readiness_receipts_append_only
BEFORE UPDATE OR DELETE ON tool_truth_shadow_writer_readiness_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER tool_truth_shadow_writer_readiness_members_append_only
BEFORE UPDATE OR DELETE ON tool_truth_shadow_writer_readiness_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER registry_shadow_evaluator_readiness_receipts_append_only
BEFORE UPDATE OR DELETE ON registry_shadow_evaluator_readiness_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER registry_shadow_evaluator_readiness_members_append_only
BEFORE UPDATE OR DELETE ON registry_shadow_evaluator_readiness_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER compatibility_projection_health_receipts_append_only
BEFORE UPDATE OR DELETE ON compatibility_projection_health_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER compatibility_projection_health_members_append_only
BEFORE UPDATE OR DELETE ON compatibility_projection_health_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER authoritative_report_dry_run_receipts_append_only
BEFORE UPDATE OR DELETE ON authoritative_report_dry_run_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER historical_read_adapter_probe_receipts_append_only
BEFORE UPDATE OR DELETE ON historical_read_adapter_probe_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER historical_read_adapter_probe_members_append_only
BEFORE UPDATE OR DELETE ON historical_read_adapter_probe_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER legacy_consumer_retirement_receipts_append_only
BEFORE UPDATE OR DELETE ON legacy_consumer_retirement_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER legacy_consumer_retirement_members_append_only
BEFORE UPDATE OR DELETE ON legacy_consumer_retirement_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER adversarial_acceptance_corpus_receipts_append_only
BEFORE UPDATE OR DELETE ON adversarial_acceptance_corpus_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER adversarial_acceptance_corpus_members_append_only
BEFORE UPDATE OR DELETE ON adversarial_acceptance_corpus_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER operation_default_promotion_receipts_append_only
BEFORE UPDATE OR DELETE ON operation_default_promotion_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER operation_default_promotion_evidence_members_append_only
BEFORE UPDATE OR DELETE ON operation_default_promotion_evidence_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER operation_rollout_safety_hold_events_append_only
BEFORE UPDATE OR DELETE ON operation_rollout_safety_hold_events
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER legacy_attempt_refutation_receipts_append_only
BEFORE UPDATE OR DELETE ON legacy_attempt_refutation_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER legacy_attempt_authority_receipts_append_only
BEFORE UPDATE OR DELETE ON legacy_attempt_authority_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER legacy_attempt_authority_source_members_append_only
BEFORE UPDATE OR DELETE ON legacy_attempt_authority_source_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER legacy_report_authority_seals_append_only
BEFORE UPDATE OR DELETE ON legacy_report_authority_seals
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER legacy_report_authority_members_append_only
BEFORE UPDATE OR DELETE ON legacy_report_authority_members
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER report_authority_invalidation_events_append_only
BEFORE UPDATE OR DELETE ON report_authority_invalidation_events
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER historical_report_artifact_receipts_append_only
BEFORE UPDATE OR DELETE ON historical_report_artifact_receipts
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
CREATE TRIGGER historical_report_artifact_read_attestations_append_only
BEFORE UPDATE OR DELETE ON historical_report_artifact_read_attestations
FOR EACH ROW EXECUTE FUNCTION investigation_reject_append_only();
