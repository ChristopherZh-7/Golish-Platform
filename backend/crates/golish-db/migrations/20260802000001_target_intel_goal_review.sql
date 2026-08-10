-- Target Intel Goal reviewed authority (additive, operation-frozen).
-- Historical operations are intentionally not backfilled: absence means
-- legacy_six_axis_v1. No table in this migration grants active scope.

-- One semantic pivot terminal receipt per fixture session/query. The receipt
-- payload already binds operation, organization and session; the expression
-- index closes the response-loss/concurrent replay race before review state can
-- consume duplicate terminal actions.
CREATE UNIQUE INDEX target_intel_semantic_receipt_unique
    ON audit_log(
        session_id,
        (detail ->> 'operation_id'),
        (detail ->> 'organization_id'),
        (detail ->> 'stable_query_key')
    )
    WHERE details = 'intel.semantic_pivot_receipt.v1'
      AND source IN ('target_intel_goal_shadow','target_intel_goal')
      AND status IN ('succeeded','empty','blocked','unsupported');

-- The artifact ref is a bounded content hash embedded in the evidence
-- raw_output JSON. The evidence ledger's operation advisory lock serializes
-- the hash chain; this unique key additionally makes response-loss and
-- concurrent semantic retries converge on the same evidence row.
CREATE UNIQUE INDEX target_intel_semantic_evidence_unique
    ON audit_log(
        run_id,
        session_id,
        (detail ->> 'organization_id'),
        (((detail ->> 'raw_output')::jsonb) ->> 'artifact_ref')
    )
    WHERE audit_role = 'evidence'
      AND source = 'harness'
      AND tool_name = 'recon_search_intel'
      AND detail ->> 'kind' = 'target_intel.semantic_pivot';

CREATE TABLE target_intel_semantic_artifacts (
    artifact_ref TEXT NOT NULL
        CHECK (artifact_ref ~ '^intel-artifact:sha256:[0-9a-f]{64}$'),
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL,
    session_id UUID NOT NULL,
    artifact_sha256 TEXT NOT NULL CHECK (artifact_sha256 ~ '^[0-9a-f]{64}$'),
    redacted_payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (operation_id, organization_id, session_id, artifact_ref),
    UNIQUE (operation_id, organization_id, session_id, artifact_sha256)
);

CREATE FUNCTION reject_target_intel_semantic_artifact_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'TARGET_INTEL_SEMANTIC_ARTIFACT_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_semantic_artifact_immutable
BEFORE UPDATE OR DELETE ON target_intel_semantic_artifacts
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_semantic_artifact_mutation();

ALTER TABLE stage_work_items
    ADD COLUMN execution_profile TEXT NOT NULL DEFAULT 'worker'
        CHECK (execution_profile IN ('worker', 'read_only_reviewer')),
    ADD COLUMN terminal_contract TEXT NOT NULL DEFAULT 'worker_output_v1'
        CHECK (terminal_contract IN ('worker_output_v1', 'intel_review_v1')),
    ADD COLUMN display_name TEXT,
    ADD COLUMN task_prompt_sha256 TEXT,
    ADD COLUMN host_prompt_version TEXT,
    ADD COLUMN host_prompt_sha256 TEXT,
    ADD CONSTRAINT stage_work_item_reviewer_contract_ck CHECK (
        (execution_profile = 'read_only_reviewer' AND terminal_contract = 'intel_review_v1')
        OR (execution_profile = 'worker' AND terminal_contract = 'worker_output_v1')
    ),
    ADD CONSTRAINT stage_work_item_display_name_ck CHECK (
        display_name IS NULL
        OR (char_length(btrim(display_name)) BETWEEN 1 AND 80)
    ),
    ADD CONSTRAINT stage_work_item_prompt_hashes_ck CHECK (
        (task_prompt_sha256 IS NULL OR task_prompt_sha256 ~ '^sha256:[0-9a-f]{64}$')
        AND (host_prompt_sha256 IS NULL OR host_prompt_sha256 ~ '^sha256:[0-9a-f]{64}$')
        AND (host_prompt_version IS NULL OR btrim(host_prompt_version) <> '')
    );

CREATE FUNCTION enforce_target_intel_work_item_profile_immutable()
RETURNS trigger AS $$
BEGIN
    IF ROW(
        NEW.execution_profile,
        NEW.terminal_contract,
        NEW.display_name,
        NEW.task_prompt_sha256,
        NEW.host_prompt_version,
        NEW.host_prompt_sha256
    ) IS DISTINCT FROM ROW(
        OLD.execution_profile,
        OLD.terminal_contract,
        OLD.display_name,
        OLD.task_prompt_sha256,
        OLD.host_prompt_version,
        OLD.host_prompt_sha256
    ) THEN
        RAISE EXCEPTION 'TARGET_INTEL_WORK_ITEM_EXECUTION_CONTRACT_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_work_item_profile_immutable
BEFORE UPDATE ON stage_work_items
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_work_item_profile_immutable();

CREATE TABLE target_intel_goal_operation_contracts (
    operation_id UUID PRIMARY KEY REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    profile_id TEXT NOT NULL CHECK (btrim(profile_id) <> ''),
    runtime_mode TEXT NOT NULL CHECK (
        runtime_mode IN ('observe_shadow', 'advisory_rework', 'intel_goal_v1')
    ),
    completion_authority TEXT NOT NULL CHECK (
        completion_authority IN ('legacy_six_axis_v1', 'intel_goal_v1')
    ),
    goal_contract_version TEXT NOT NULL CHECK (goal_contract_version = 'target_intel_goal.v1'),
    canonical_goal_contract JSONB NOT NULL CHECK (jsonb_typeof(canonical_goal_contract) = 'object'),
    goal_contract_sha256 TEXT NOT NULL CHECK (goal_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    methodology_payload JSONB NOT NULL,
    methodology_sha256 TEXT NOT NULL CHECK (methodology_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    tool_manifest JSONB NOT NULL CHECK (jsonb_typeof(tool_manifest) = 'object'),
    tool_manifest_sha256 TEXT NOT NULL CHECK (tool_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    provider_capability_manifest JSONB NOT NULL
        CHECK (jsonb_typeof(provider_capability_manifest) = 'object'),
    provider_capability_sha256 TEXT NOT NULL
        CHECK (provider_capability_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    browser_policy JSONB NOT NULL CHECK (jsonb_typeof(browser_policy) = 'object'),
    budget_policy JSONB NOT NULL CHECK (jsonb_typeof(budget_policy) = 'object'),
    max_review_rounds INTEGER NOT NULL CHECK (max_review_rounds > 0),
    reviewer_retry_fuel INTEGER NOT NULL CHECK (reviewer_retry_fuel >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (runtime_mode = 'intel_goal_v1' AND completion_authority = 'intel_goal_v1')
        OR (runtime_mode <> 'intel_goal_v1' AND completion_authority = 'legacy_six_axis_v1')
    )
);

CREATE FUNCTION reject_target_intel_goal_operation_contract_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'TARGET_INTEL_GOAL_OPERATION_CONTRACT_IMMUTABLE';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_goal_operation_contract_immutable
BEFORE UPDATE OR DELETE ON target_intel_goal_operation_contracts
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_goal_operation_contract_mutation();

CREATE TABLE target_intel_goal_material_revisions (
    operation_id UUID NOT NULL REFERENCES target_intel_goal_operation_contracts(operation_id),
    organization_id UUID NOT NULL,
    state_revision BIGINT NOT NULL DEFAULT 0 CHECK (state_revision >= 0),
    action_revision BIGINT NOT NULL DEFAULT 0 CHECK (action_revision >= 0),
    evidence_high_water BIGINT NOT NULL DEFAULT 0 CHECK (evidence_high_water >= 0),
    tool_high_water BIGINT NOT NULL DEFAULT 0 CHECK (tool_high_water >= 0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (operation_id, organization_id)
);

CREATE TABLE target_intel_goal_epochs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES target_intel_goal_operation_contracts(operation_id),
    organization_id UUID NOT NULL,
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    epoch BIGINT NOT NULL CHECK (epoch >= 0),
    status TEXT NOT NULL CHECK (status IN ('open', 'sealed_for_review', 'held', 'finalized', 'superseded')),
    controller_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    controller_worker_run_id UUID REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    controller_message_chain_id UUID,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sealed_at TIMESTAMPTZ,
    terminal_at TIMESTAMPTZ,
    UNIQUE (team_plan_id, epoch)
);

CREATE TABLE target_intel_goal_reviews (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES target_intel_goal_operation_contracts(operation_id),
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    team_plan_id UUID NOT NULL REFERENCES stage_team_plans(id) ON DELETE RESTRICT,
    goal_epoch_id UUID NOT NULL REFERENCES target_intel_goal_epochs(id) ON DELETE RESTRICT,
    goal_epoch BIGINT NOT NULL CHECK (goal_epoch >= 0),
    review_generation BIGINT NOT NULL CHECK (review_generation >= 0),
    round INTEGER NOT NULL CHECK (round > 0),
    controller_work_item_id UUID NOT NULL REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    controller_worker_run_id UUID REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    controller_message_chain_id UUID,
    reviewer_work_item_id UUID REFERENCES stage_work_items(id) ON DELETE RESTRICT,
    reviewer_worker_run_id UUID REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    operation_contract_sha256 TEXT NOT NULL CHECK (operation_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    material_revision_vector JSONB NOT NULL CHECK (jsonb_typeof(material_revision_vector) = 'object'),
    material_state_sha256 TEXT NOT NULL CHECK (material_state_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    material_actions_sha256 TEXT NOT NULL CHECK (material_actions_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    durable_state JSONB NOT NULL,
    durable_state_sha256 TEXT NOT NULL CHECK (durable_state_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    observable_actions JSONB NOT NULL,
    observable_actions_sha256 TEXT NOT NULL CHECK (observable_actions_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    frozen_contract JSONB NOT NULL,
    frozen_contract_sha256 TEXT NOT NULL CHECK (frozen_contract_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    completion_claim JSONB NOT NULL,
    completion_claim_sha256 TEXT NOT NULL CHECK (completion_claim_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    bundle_sha256 TEXT NOT NULL CHECK (bundle_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK (
        status IN ('building', 'frozen', 'reviewing', 'rework', 'pass', 'needs_human', 'stale', 'superseded')
    ),
    verdict JSONB,
    verdict_sha256 TEXT CHECK (verdict_sha256 IS NULL OR verdict_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    frozen_at TIMESTAMPTZ,
    terminal_at TIMESTAMPTZ,
    UNIQUE (team_plan_id, round),
    UNIQUE (operation_id, organization_id, review_generation)
);

CREATE TABLE target_intel_goal_review_section_reads (
    review_id UUID NOT NULL REFERENCES target_intel_goal_reviews(id) ON DELETE RESTRICT,
    reviewer_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id) ON DELETE RESTRICT,
    section_ordinal SMALLINT NOT NULL CHECK (section_ordinal BETWEEN 1 AND 4),
    section_kind TEXT NOT NULL CHECK (
        section_kind IN ('durable_state', 'observable_actions', 'frozen_contract', 'completion_claim')
    ),
    section_sha256 TEXT NOT NULL CHECK (section_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    read_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (review_id, section_ordinal)
);

CREATE FUNCTION enforce_target_intel_review_section_cursor()
RETURNS trigger AS $$
DECLARE
    expected SMALLINT;
    expected_kind TEXT;
    review target_intel_goal_reviews%ROWTYPE;
BEGIN
    SELECT * INTO review FROM target_intel_goal_reviews WHERE id = NEW.review_id FOR UPDATE;
    IF NOT FOUND OR review.status NOT IN ('frozen', 'reviewing')
       OR review.reviewer_worker_run_id IS DISTINCT FROM NEW.reviewer_worker_run_id THEN
        RAISE EXCEPTION 'TARGET_INTEL_REVIEW_FOREIGN_OR_STALE_READER';
    END IF;
    SELECT COALESCE(MAX(section_ordinal), 0) + 1 INTO expected
      FROM target_intel_goal_review_section_reads WHERE review_id = NEW.review_id;
    IF NEW.section_ordinal <> expected THEN
        RAISE EXCEPTION 'TARGET_INTEL_REVIEW_SECTION_OUT_OF_ORDER';
    END IF;
    expected_kind := (ARRAY['durable_state','observable_actions','frozen_contract','completion_claim'])[expected];
    IF NEW.section_kind <> expected_kind THEN
        RAISE EXCEPTION 'TARGET_INTEL_REVIEW_SECTION_KIND_MISMATCH';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_review_section_cursor
BEFORE INSERT ON target_intel_goal_review_section_reads
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_review_section_cursor();

CREATE TABLE target_intel_goal_review_findings (
    id UUID PRIMARY KEY,
    review_id UUID NOT NULL REFERENCES target_intel_goal_reviews(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL CHECK (fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    materiality TEXT NOT NULL CHECK (materiality IN ('critical', 'major', 'minor', 'advisory')),
    subject_refs JSONB NOT NULL CHECK (jsonb_typeof(subject_refs) = 'array'),
    reason TEXT NOT NULL CHECK (btrim(reason) <> ''),
    recommended_action JSONB,
    close_condition TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (review_id, fingerprint)
);

CREATE TABLE target_intel_goal_review_finding_resolutions (
    id UUID PRIMARY KEY,
    finding_id UUID NOT NULL REFERENCES target_intel_goal_review_findings(id) ON DELETE RESTRICT,
    review_id UUID NOT NULL REFERENCES target_intel_goal_reviews(id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN ('resolved', 'still_open', 'needs_human')),
    resolution_refs JSONB NOT NULL CHECK (jsonb_typeof(resolution_refs) = 'array'),
    reason TEXT NOT NULL CHECK (btrim(reason) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (finding_id, review_id)
);

CREATE TABLE target_intel_goal_holds (
    id UUID PRIMARY KEY,
    review_id UUID NOT NULL UNIQUE REFERENCES target_intel_goal_reviews(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    requirement_kind TEXT NOT NULL CHECK (
        requirement_kind IN ('credential', 'scope_confirmation', 'subject_confirmation', 'provider_recovery', 'review_fixed_point')
    ),
    requirement_payload JSONB NOT NULL CHECK (jsonb_typeof(requirement_payload) = 'object'),
    status TEXT NOT NULL CHECK (status IN ('open', 'fulfilled', 'superseded')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    fulfilled_at TIMESTAMPTZ
);

CREATE TABLE target_intel_goal_hold_fulfillments (
    id UUID PRIMARY KEY,
    hold_id UUID NOT NULL UNIQUE REFERENCES target_intel_goal_holds(id) ON DELETE RESTRICT,
    expected_hold_row_version BIGINT NOT NULL CHECK (expected_hold_row_version >= 0),
    fulfillment_kind TEXT NOT NULL CHECK (btrim(fulfillment_kind) <> ''),
    authority_ref TEXT NOT NULL CHECK (btrim(authority_ref) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE target_intel_goal_review_jobs (
    id UUID PRIMARY KEY,
    review_id UUID NOT NULL UNIQUE REFERENCES target_intel_goal_reviews(id) ON DELETE RESTRICT,
    mode TEXT NOT NULL CHECK (mode = 'observe_shadow'),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed', 'superseded')),
    attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX target_intel_goal_reviews_current_idx
    ON target_intel_goal_reviews(operation_id, organization_id, round DESC);
CREATE INDEX target_intel_goal_review_jobs_ready_idx
    ON target_intel_goal_review_jobs(status, available_at);

-- ---------------------------------------------------------------------------
-- Exact owner tuples and host-owned Goal/review state machines
-- ---------------------------------------------------------------------------

-- Reviewer rows are a distinct server-created WorkItem class. Merely setting
-- this discriminator does not grant authority: enforce_stage_work_item_contract
-- below also requires a matching building freeze-authority row.
ALTER TABLE stage_work_items
    DROP CONSTRAINT stage_work_items_created_by_check,
    ADD CONSTRAINT stage_work_items_created_by_check CHECK (
        created_by IN (
            'server_seed', 'accepted_worker_request', 'gate_repair',
            'server_phase_transition', 'target_intel_review_freeze'
        )
    );

ALTER TABLE target_intel_goal_epochs
    ADD COLUMN stage_execution_id UUID NOT NULL,
    ADD COLUMN stage_run_unit_id UUID NOT NULL,
    ADD COLUMN scope_snapshot_id UUID NOT NULL,
    ADD COLUMN review_fuel_remaining INTEGER NOT NULL CHECK (review_fuel_remaining >= 0),
    ADD COLUMN resume_authority_id UUID,
    ADD CONSTRAINT target_intel_goal_epochs_owner_unique UNIQUE (
        id, operation_id, organization_id, team_plan_id, epoch
    ),
    ADD CONSTRAINT target_intel_goal_epochs_plan_owner_fk FOREIGN KEY (
        team_plan_id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) REFERENCES stage_team_plans (
        id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_epochs_controller_item_owner_fk FOREIGN KEY (
        controller_work_item_id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) REFERENCES stage_work_items (
        id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_epochs_controller_worker_owner_fk FOREIGN KEY (
        controller_worker_run_id, controller_work_item_id, operation_id,
        stage_execution_id, stage_run_unit_id, organization_id
    ) REFERENCES stage_worker_runs (
        id, work_item_id, operation_id, stage_execution_id,
        stage_run_unit_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_epochs_controller_chain_owner_fk FOREIGN KEY (
        controller_message_chain_id, operation_id
    ) REFERENCES message_chains(id, task_id) ON DELETE RESTRICT;

ALTER TABLE target_intel_goal_reviews
    ADD COLUMN finding_set_sha256 TEXT
        CHECK (finding_set_sha256 IS NULL OR finding_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN recommended_actions_sha256 TEXT
        CHECK (recommended_actions_sha256 IS NULL OR recommended_actions_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN effective_decision_reason TEXT,
    ADD CONSTRAINT target_intel_goal_reviews_owner_unique UNIQUE (
        id, operation_id, organization_id
    ),
    ADD CONSTRAINT target_intel_goal_reviews_plan_owner_fk FOREIGN KEY (
        team_plan_id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) REFERENCES stage_team_plans (
        id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_reviews_epoch_owner_fk FOREIGN KEY (
        goal_epoch_id, operation_id, organization_id, team_plan_id, goal_epoch
    ) REFERENCES target_intel_goal_epochs (
        id, operation_id, organization_id, team_plan_id, epoch
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_reviews_controller_item_owner_fk FOREIGN KEY (
        controller_work_item_id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) REFERENCES stage_work_items (
        id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_reviews_controller_worker_owner_fk FOREIGN KEY (
        controller_worker_run_id, controller_work_item_id, operation_id,
        stage_execution_id, stage_run_unit_id, organization_id
    ) REFERENCES stage_worker_runs (
        id, work_item_id, operation_id, stage_execution_id,
        stage_run_unit_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_reviews_controller_chain_owner_fk FOREIGN KEY (
        controller_message_chain_id, operation_id
    ) REFERENCES message_chains(id, task_id) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_reviews_reviewer_item_owner_fk FOREIGN KEY (
        reviewer_work_item_id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) REFERENCES stage_work_items (
        id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_reviews_reviewer_worker_owner_fk FOREIGN KEY (
        reviewer_worker_run_id, reviewer_work_item_id, operation_id,
        stage_execution_id, stage_run_unit_id, organization_id
    ) REFERENCES stage_worker_runs (
        id, work_item_id, operation_id, stage_execution_id,
        stage_run_unit_id, organization_id
    ) ON DELETE RESTRICT;

ALTER TABLE target_intel_goal_review_section_reads
    ADD COLUMN operation_id UUID NOT NULL,
    ADD COLUMN organization_id UUID NOT NULL,
    ADD CONSTRAINT target_intel_goal_review_section_reads_owner_fk FOREIGN KEY (
        review_id, operation_id, organization_id
    ) REFERENCES target_intel_goal_reviews(id, operation_id, organization_id)
      ON DELETE RESTRICT;

ALTER TABLE target_intel_goal_review_findings
    ADD COLUMN operation_id UUID NOT NULL,
    ADD COLUMN organization_id UUID NOT NULL,
    ADD CONSTRAINT target_intel_goal_review_findings_owner_unique UNIQUE (
        id, operation_id, organization_id
    ),
    ADD CONSTRAINT target_intel_goal_review_findings_review_owner_fk FOREIGN KEY (
        review_id, operation_id, organization_id
    ) REFERENCES target_intel_goal_reviews(id, operation_id, organization_id)
      ON DELETE RESTRICT;

ALTER TABLE target_intel_goal_review_finding_resolutions
    ADD COLUMN operation_id UUID NOT NULL,
    ADD COLUMN organization_id UUID NOT NULL,
    ADD CONSTRAINT target_intel_goal_review_resolutions_finding_owner_fk FOREIGN KEY (
        finding_id, operation_id, organization_id
    ) REFERENCES target_intel_goal_review_findings(id, operation_id, organization_id)
      ON DELETE RESTRICT,
    ADD CONSTRAINT target_intel_goal_review_resolutions_review_owner_fk FOREIGN KEY (
        review_id, operation_id, organization_id
    ) REFERENCES target_intel_goal_reviews(id, operation_id, organization_id)
      ON DELETE RESTRICT;

ALTER TABLE target_intel_goal_holds
    ADD CONSTRAINT target_intel_goal_holds_review_owner_fk FOREIGN KEY (
        review_id, operation_id, organization_id
    ) REFERENCES target_intel_goal_reviews(id, operation_id, organization_id)
      ON DELETE RESTRICT,
    DROP CONSTRAINT target_intel_goal_holds_requirement_kind_check,
    ADD CONSTRAINT target_intel_goal_holds_requirement_kind_check CHECK (
        requirement_kind IN (
            'credential', 'scope_confirmation', 'subject_confirmation',
            'provider_recovery', 'review_fixed_point', 'review_fuel_exhausted'
        )
    );

ALTER TABLE target_intel_goal_hold_fulfillments
    ADD COLUMN material_input JSONB NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(material_input) = 'object');

CREATE TABLE target_intel_goal_review_freeze_authorities (
    review_id UUID PRIMARY KEY,
    reviewer_work_item_id UUID UNIQUE,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    team_plan_id UUID NOT NULL,
    goal_epoch_id UUID NOT NULL,
    goal_epoch BIGINT NOT NULL CHECK (goal_epoch >= 0),
    source_plan_row_version BIGINT NOT NULL CHECK (source_plan_row_version >= 0),
    source_epoch_row_version BIGINT NOT NULL CHECK (source_epoch_row_version >= 0),
    bundle_sha256 TEXT NOT NULL CHECK (bundle_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK (status IN ('building','applied')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    applied_at TIMESTAMPTZ,
    FOREIGN KEY (
        team_plan_id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) REFERENCES stage_team_plans (
        id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        goal_epoch_id, operation_id, organization_id, team_plan_id, goal_epoch
    ) REFERENCES target_intel_goal_epochs (
        id, operation_id, organization_id, team_plan_id, epoch
    ) ON DELETE RESTRICT
);

CREATE TABLE target_intel_goal_resume_authorities (
    id UUID PRIMARY KEY,
    authority_kind TEXT NOT NULL CHECK (
        authority_kind IN ('review_rework','human_fulfillment','finalizer_repair')
    ),
    source_review_id UUID NOT NULL REFERENCES target_intel_goal_reviews(id) ON DELETE RESTRICT,
    source_hold_id UUID REFERENCES target_intel_goal_holds(id) ON DELETE RESTRICT,
    fulfillment_id UUID REFERENCES target_intel_goal_hold_fulfillments(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    team_plan_id UUID NOT NULL,
    source_goal_epoch_id UUID NOT NULL,
    source_goal_epoch BIGINT NOT NULL CHECK (source_goal_epoch >= 0),
    successor_goal_epoch_id UUID NOT NULL UNIQUE,
    successor_goal_epoch BIGINT NOT NULL CHECK (successor_goal_epoch > 0),
    controller_work_item_id UUID NOT NULL,
    controller_worker_run_id UUID NOT NULL,
    controller_message_chain_id UUID NOT NULL,
    source_plan_row_version BIGINT NOT NULL CHECK (source_plan_row_version >= 0),
    source_item_row_version BIGINT NOT NULL CHECK (source_item_row_version >= 0),
    finding_set_sha256 TEXT NOT NULL CHECK (finding_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    recommended_actions_sha256 TEXT NOT NULL CHECK (recommended_actions_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    material_state_sha256 TEXT NOT NULL CHECK (material_state_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    material_actions_sha256 TEXT NOT NULL CHECK (material_actions_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    fuel_before INTEGER NOT NULL CHECK (fuel_before > 0),
    fuel_after INTEGER NOT NULL CHECK (fuel_after >= 0 AND fuel_after = fuel_before - 1),
    server_message JSONB NOT NULL CHECK (jsonb_typeof(server_message) = 'object'),
    server_message_sha256 TEXT NOT NULL CHECK (server_message_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    status TEXT NOT NULL CHECK (status IN ('building','applied')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    applied_at TIMESTAMPTZ,
    UNIQUE (source_review_id, authority_kind),
    FOREIGN KEY (
        team_plan_id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) REFERENCES stage_team_plans (
        id, operation_id, stage_execution_id, stage_run_unit_id,
        scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        source_goal_epoch_id, operation_id, organization_id,
        team_plan_id, source_goal_epoch
    ) REFERENCES target_intel_goal_epochs (
        id, operation_id, organization_id, team_plan_id, epoch
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        controller_work_item_id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) REFERENCES stage_work_items (
        id, team_plan_id, operation_id, stage_execution_id,
        stage_run_unit_id, scope_snapshot_id, organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (
        controller_worker_run_id, controller_work_item_id, operation_id,
        stage_execution_id, stage_run_unit_id, organization_id
    ) REFERENCES stage_worker_runs (
        id, work_item_id, operation_id, stage_execution_id,
        stage_run_unit_id, organization_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY (controller_message_chain_id, operation_id)
        REFERENCES message_chains(id, task_id) ON DELETE RESTRICT
);

ALTER TABLE target_intel_goal_epochs
    ADD CONSTRAINT target_intel_goal_epochs_resume_authority_fk
    FOREIGN KEY (resume_authority_id)
        REFERENCES target_intel_goal_resume_authorities(id) ON DELETE RESTRICT;

CREATE FUNCTION enforce_target_intel_review_freeze_authority_applied_at_commit()
RETURNS trigger AS $$
DECLARE
    authority target_intel_goal_review_freeze_authorities%ROWTYPE;
BEGIN
    SELECT * INTO authority
      FROM target_intel_goal_review_freeze_authorities WHERE review_id=NEW.review_id;
    IF NOT FOUND OR authority.status <> 'applied' OR authority.applied_at IS NULL OR NOT EXISTS (
        SELECT 1
          FROM target_intel_goal_reviews review
         WHERE review.id=authority.review_id
           AND review.operation_id=authority.operation_id
           AND review.organization_id=authority.organization_id
           AND review.team_plan_id=authority.team_plan_id
           AND review.goal_epoch_id=authority.goal_epoch_id
           AND review.goal_epoch=authority.goal_epoch
           AND review.reviewer_work_item_id IS NOT DISTINCT FROM authority.reviewer_work_item_id
           AND review.bundle_sha256=authority.bundle_sha256
    ) THEN
        RAISE EXCEPTION 'TARGET_INTEL_REVIEW_FREEZE_MUST_APPLY_AT_COMMIT';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER target_intel_review_freeze_authority_applied_at_commit
AFTER INSERT OR UPDATE ON target_intel_goal_review_freeze_authorities
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_review_freeze_authority_applied_at_commit();

CREATE FUNCTION enforce_target_intel_resume_authority_applied_at_commit()
RETURNS trigger AS $$
DECLARE
    authority target_intel_goal_resume_authorities%ROWTYPE;
BEGIN
    SELECT * INTO authority FROM target_intel_goal_resume_authorities WHERE id=NEW.id;
    IF NOT FOUND OR authority.status <> 'applied' OR authority.applied_at IS NULL OR NOT EXISTS (
        SELECT 1 FROM target_intel_goal_epochs successor
         WHERE successor.id=authority.successor_goal_epoch_id
           AND successor.operation_id=authority.operation_id
           AND successor.organization_id=authority.organization_id
           AND successor.team_plan_id=authority.team_plan_id
           AND successor.epoch=authority.successor_goal_epoch
           AND successor.status='open'
           AND successor.resume_authority_id=authority.id
           AND successor.controller_work_item_id=authority.controller_work_item_id
           AND successor.controller_worker_run_id=authority.controller_worker_run_id
           AND successor.controller_message_chain_id=authority.controller_message_chain_id
    ) THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_RESUME_MUST_APPLY_AT_COMMIT';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER target_intel_resume_authority_applied_at_commit
AFTER INSERT OR UPDATE ON target_intel_goal_resume_authorities
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_resume_authority_applied_at_commit();

CREATE FUNCTION enforce_target_intel_goal_epoch_contract()
RETURNS trigger AS $$
DECLARE
    authority target_intel_goal_resume_authorities%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_EPOCH_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.epoch=0 THEN
            IF NEW.status<>'open' OR NEW.resume_authority_id IS NOT NULL THEN
                RAISE EXCEPTION 'TARGET_INTEL_GOAL_INITIAL_EPOCH_INVALID';
            END IF;
        ELSE
            SELECT * INTO authority FROM target_intel_goal_resume_authorities
             WHERE id=NEW.resume_authority_id AND status='building'
               AND successor_goal_epoch_id=NEW.id
               AND successor_goal_epoch=NEW.epoch
               AND operation_id=NEW.operation_id
               AND organization_id=NEW.organization_id
               AND team_plan_id=NEW.team_plan_id
               AND controller_work_item_id=NEW.controller_work_item_id
               AND controller_worker_run_id=NEW.controller_worker_run_id
               AND controller_message_chain_id=NEW.controller_message_chain_id;
            IF NOT FOUND OR NEW.status<>'open'
                OR NEW.review_fuel_remaining<>authority.fuel_after
            THEN
                RAISE EXCEPTION 'TARGET_INTEL_GOAL_SUCCESSOR_EPOCH_AUTHORITY_REQUIRED';
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(
        NEW.id,NEW.operation_id,NEW.organization_id,NEW.team_plan_id,NEW.epoch,
        NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
        NEW.controller_work_item_id,NEW.controller_worker_run_id,
        NEW.controller_message_chain_id,NEW.review_fuel_remaining,
        NEW.resume_authority_id,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.operation_id,OLD.organization_id,OLD.team_plan_id,OLD.epoch,
        OLD.stage_execution_id,OLD.stage_run_unit_id,OLD.scope_snapshot_id,
        OLD.controller_work_item_id,OLD.controller_worker_run_id,
        OLD.controller_message_chain_id,OLD.review_fuel_remaining,
        OLD.resume_authority_id,OLD.created_at
    ) OR NEW.row_version<>OLD.row_version+1 THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_EPOCH_IDENTITY_OR_CAS_INVALID';
    END IF;
    IF NOT (
        (OLD.status='open' AND NEW.status='sealed_for_review'
            AND OLD.sealed_at IS NULL AND NEW.sealed_at IS NOT NULL
            AND NEW.terminal_at IS NULL)
        OR (OLD.status='sealed_for_review' AND NEW.status IN ('held','finalized','superseded')
            AND OLD.sealed_at IS NOT NULL AND NEW.sealed_at=OLD.sealed_at
            AND NEW.terminal_at IS NOT NULL)
        OR (OLD.status='held' AND NEW.status='superseded'
            AND NEW.terminal_at IS NOT NULL)
    ) THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_EPOCH_INVALID_TRANSITION';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_goal_epoch_contract
BEFORE INSERT OR UPDATE OR DELETE ON target_intel_goal_epochs
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_goal_epoch_contract();

CREATE FUNCTION enforce_target_intel_goal_review_contract()
RETURNS trigger AS $$
DECLARE
    expected_generation BIGINT;
    expected_round INTEGER;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_REVIEW_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
        SELECT COALESCE(MAX(review_generation)+1,1) INTO expected_generation
          FROM target_intel_goal_reviews
         WHERE operation_id=NEW.operation_id AND organization_id=NEW.organization_id;
        SELECT COALESCE(MAX(round)+1,1) INTO expected_round
          FROM target_intel_goal_reviews WHERE team_plan_id=NEW.team_plan_id;
        IF NEW.status<>'frozen' OR NEW.frozen_at IS NULL OR NEW.terminal_at IS NOT NULL
            OR NEW.verdict IS NOT NULL OR NEW.verdict_sha256 IS NOT NULL
            OR NEW.review_generation<>expected_generation OR NEW.round<>expected_round
            OR NOT EXISTS (
                SELECT 1 FROM target_intel_goal_epochs epoch
                 WHERE epoch.id=NEW.goal_epoch_id
                   AND epoch.operation_id=NEW.operation_id
                   AND epoch.organization_id=NEW.organization_id
                   AND epoch.team_plan_id=NEW.team_plan_id
                   AND epoch.epoch=NEW.goal_epoch
                   AND (
                       epoch.status='sealed_for_review'
                       OR (
                           NEW.reviewer_work_item_id IS NULL
                           AND EXISTS (
                               SELECT 1 FROM target_intel_goal_operation_contracts contract
                                WHERE contract.operation_id=NEW.operation_id
                                  AND contract.runtime_mode='observe_shadow'
                           )
                       )
                   )
            )
        THEN
            RAISE EXCEPTION 'TARGET_INTEL_GOAL_REVIEW_FREEZE_INVALID';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(
        NEW.id,NEW.operation_id,NEW.organization_id,NEW.stage_execution_id,
        NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.team_plan_id,
        NEW.goal_epoch_id,NEW.goal_epoch,NEW.review_generation,NEW.round,
        NEW.controller_work_item_id,NEW.controller_worker_run_id,
        NEW.controller_message_chain_id,NEW.reviewer_work_item_id,
        NEW.operation_contract_sha256,NEW.material_revision_vector,
        NEW.material_state_sha256,NEW.material_actions_sha256,
        NEW.durable_state,NEW.durable_state_sha256,NEW.observable_actions,
        NEW.observable_actions_sha256,NEW.frozen_contract,
        NEW.frozen_contract_sha256,NEW.completion_claim,
        NEW.completion_claim_sha256,NEW.bundle_sha256,NEW.created_at,NEW.frozen_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.operation_id,OLD.organization_id,OLD.stage_execution_id,
        OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.team_plan_id,
        OLD.goal_epoch_id,OLD.goal_epoch,OLD.review_generation,OLD.round,
        OLD.controller_work_item_id,OLD.controller_worker_run_id,
        OLD.controller_message_chain_id,OLD.reviewer_work_item_id,
        OLD.operation_contract_sha256,OLD.material_revision_vector,
        OLD.material_state_sha256,OLD.material_actions_sha256,
        OLD.durable_state,OLD.durable_state_sha256,OLD.observable_actions,
        OLD.observable_actions_sha256,OLD.frozen_contract,
        OLD.frozen_contract_sha256,OLD.completion_claim,
        OLD.completion_claim_sha256,OLD.bundle_sha256,OLD.created_at,OLD.frozen_at
    ) OR NEW.row_version<>OLD.row_version+1 THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_REVIEW_IDENTITY_OR_CAS_INVALID';
    END IF;
    IF OLD.status='frozen' AND NEW.status='frozen'
        AND OLD.reviewer_worker_run_id IS NULL
        AND NEW.reviewer_worker_run_id IS NOT NULL
        AND NEW.verdict IS NULL AND NEW.verdict_sha256 IS NULL
        AND NEW.terminal_at IS NULL
    THEN
        RETURN NEW;
    END IF;
    IF OLD.status IN ('frozen','reviewing') AND NEW.status=OLD.status
        AND OLD.reviewer_worker_run_id IS NOT NULL
        AND NEW.reviewer_worker_run_id IS NOT NULL
        AND NEW.reviewer_worker_run_id<>OLD.reviewer_worker_run_id
        AND EXISTS (
            SELECT 1 FROM stage_worker_runs prior
             WHERE prior.id=OLD.reviewer_worker_run_id
               AND prior.status IN ('failed','exhausted','superseded')
        )
        AND EXISTS (
            SELECT 1 FROM stage_worker_runs replacement
             WHERE replacement.id=NEW.reviewer_worker_run_id
               AND replacement.work_item_id=NEW.reviewer_work_item_id
               AND replacement.operation_id=NEW.operation_id
               AND replacement.stage_execution_id=NEW.stage_execution_id
               AND replacement.stage_run_unit_id=NEW.stage_run_unit_id
               AND replacement.organization_id=NEW.organization_id
               AND replacement.status IN ('queued','running','waiting_background','gate_blocked')
        )
        AND NEW.verdict IS NULL AND NEW.verdict_sha256 IS NULL
        AND NEW.terminal_at IS NULL
    THEN
        RETURN NEW;
    END IF;
    IF OLD.status='frozen' AND NEW.status='reviewing'
        AND NEW.reviewer_worker_run_id IS NOT NULL
        AND NEW.verdict IS NULL AND NEW.verdict_sha256 IS NULL
        AND NEW.terminal_at IS NULL
    THEN
        RETURN NEW;
    END IF;
    IF OLD.status='reviewing' AND NEW.status IN ('pass','rework','needs_human')
        AND NEW.reviewer_worker_run_id=OLD.reviewer_worker_run_id
        AND NEW.verdict IS NOT NULL AND NEW.verdict_sha256 IS NOT NULL
        AND NEW.finding_set_sha256 IS NOT NULL
        AND NEW.recommended_actions_sha256 IS NOT NULL
        AND NEW.terminal_at IS NOT NULL
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'TARGET_INTEL_GOAL_REVIEW_INVALID_TRANSITION';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_goal_review_contract
BEFORE INSERT OR UPDATE OR DELETE ON target_intel_goal_reviews
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_goal_review_contract();

CREATE FUNCTION enforce_target_intel_goal_hold_contract()
RETURNS trigger AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_HOLD_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.status<>'open' OR NEW.row_version<>0 OR NEW.fulfilled_at IS NOT NULL THEN
            RAISE EXCEPTION 'TARGET_INTEL_GOAL_HOLD_INSERT_INVALID';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(
        NEW.id,NEW.review_id,NEW.operation_id,NEW.organization_id,
        NEW.requirement_kind,NEW.requirement_payload,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.review_id,OLD.operation_id,OLD.organization_id,
        OLD.requirement_kind,OLD.requirement_payload,OLD.created_at
    ) OR NEW.row_version<>OLD.row_version+1 THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_HOLD_IDENTITY_OR_CAS_INVALID';
    END IF;
    IF OLD.status='open' AND NEW.status='fulfilled' AND NEW.fulfilled_at IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM target_intel_goal_hold_fulfillments fulfillment
            WHERE fulfillment.hold_id=OLD.id
              AND fulfillment.expected_hold_row_version=OLD.row_version
       )
    THEN
        RETURN NEW;
    END IF;
    IF OLD.status='open' AND NEW.status='superseded' THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'TARGET_INTEL_GOAL_HOLD_INVALID_TRANSITION';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_goal_hold_contract
BEFORE INSERT OR UPDATE OR DELETE ON target_intel_goal_holds
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_goal_hold_contract();

CREATE FUNCTION reject_target_intel_goal_append_only_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'TARGET_INTEL_GOAL_APPEND_ONLY';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_review_section_reads_append_only
BEFORE UPDATE OR DELETE ON target_intel_goal_review_section_reads
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_goal_append_only_mutation();
CREATE TRIGGER target_intel_review_findings_append_only
BEFORE UPDATE OR DELETE ON target_intel_goal_review_findings
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_goal_append_only_mutation();
CREATE TRIGGER target_intel_review_resolutions_append_only
BEFORE UPDATE OR DELETE ON target_intel_goal_review_finding_resolutions
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_goal_append_only_mutation();
CREATE TRIGGER target_intel_hold_fulfillments_append_only
BEFORE UPDATE OR DELETE ON target_intel_goal_hold_fulfillments
FOR EACH ROW EXECUTE FUNCTION reject_target_intel_goal_append_only_mutation();

CREATE FUNCTION bump_target_intel_hold_fulfillment_revision()
RETURNS trigger AS $$
BEGIN
    UPDATE target_intel_goal_material_revisions material
       SET state_revision=material.state_revision+1,
           row_version=material.row_version+1,
           updated_at=NOW()
      FROM target_intel_goal_holds hold
      JOIN target_intel_goal_reviews review ON review.id=hold.review_id
     WHERE hold.id=NEW.hold_id
       AND material.operation_id=review.operation_id
       AND material.organization_id=review.organization_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'TARGET_INTEL_MATERIAL_REVISION_MISSING';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_hold_fulfillment_revision
AFTER INSERT ON target_intel_goal_hold_fulfillments
FOR EACH ROW EXECUTE FUNCTION bump_target_intel_hold_fulfillment_revision();

CREATE FUNCTION enforce_target_intel_freeze_authority_transition()
RETURNS trigger AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'TARGET_INTEL_REVIEW_FREEZE_AUTHORITY_IMMUTABLE';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF ROW(
            NEW.review_id,NEW.reviewer_work_item_id,NEW.operation_id,
            NEW.organization_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
            NEW.scope_snapshot_id,NEW.team_plan_id,NEW.goal_epoch_id,
            NEW.goal_epoch,NEW.source_plan_row_version,
            NEW.source_epoch_row_version,NEW.bundle_sha256,NEW.created_at
        ) IS DISTINCT FROM ROW(
            OLD.review_id,OLD.reviewer_work_item_id,OLD.operation_id,
            OLD.organization_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
            OLD.scope_snapshot_id,OLD.team_plan_id,OLD.goal_epoch_id,
            OLD.goal_epoch,OLD.source_plan_row_version,
            OLD.source_epoch_row_version,OLD.bundle_sha256,OLD.created_at
        ) OR OLD.status<>'building' OR NEW.status<>'applied'
          OR NEW.row_version<>OLD.row_version+1 OR NEW.applied_at IS NULL
        THEN
            RAISE EXCEPTION 'TARGET_INTEL_REVIEW_FREEZE_AUTHORITY_INVALID_TRANSITION';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_freeze_authority_transition
BEFORE UPDATE OR DELETE ON target_intel_goal_review_freeze_authorities
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_freeze_authority_transition();

CREATE FUNCTION enforce_target_intel_resume_authority_transition()
RETURNS trigger AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'TARGET_INTEL_GOAL_RESUME_AUTHORITY_IMMUTABLE';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF ROW(
            NEW.id,NEW.authority_kind,NEW.source_review_id,NEW.source_hold_id,
            NEW.fulfillment_id,NEW.operation_id,NEW.organization_id,
            NEW.stage_execution_id,NEW.stage_run_unit_id,NEW.scope_snapshot_id,
            NEW.team_plan_id,NEW.source_goal_epoch_id,NEW.source_goal_epoch,
            NEW.successor_goal_epoch_id,NEW.successor_goal_epoch,
            NEW.controller_work_item_id,NEW.controller_worker_run_id,
            NEW.controller_message_chain_id,NEW.source_plan_row_version,
            NEW.source_item_row_version,NEW.finding_set_sha256,
            NEW.recommended_actions_sha256,NEW.material_state_sha256,
            NEW.material_actions_sha256,NEW.fuel_before,NEW.fuel_after,
            NEW.server_message,NEW.server_message_sha256,NEW.created_at
        ) IS DISTINCT FROM ROW(
            OLD.id,OLD.authority_kind,OLD.source_review_id,OLD.source_hold_id,
            OLD.fulfillment_id,OLD.operation_id,OLD.organization_id,
            OLD.stage_execution_id,OLD.stage_run_unit_id,OLD.scope_snapshot_id,
            OLD.team_plan_id,OLD.source_goal_epoch_id,OLD.source_goal_epoch,
            OLD.successor_goal_epoch_id,OLD.successor_goal_epoch,
            OLD.controller_work_item_id,OLD.controller_worker_run_id,
            OLD.controller_message_chain_id,OLD.source_plan_row_version,
            OLD.source_item_row_version,OLD.finding_set_sha256,
            OLD.recommended_actions_sha256,OLD.material_state_sha256,
            OLD.material_actions_sha256,OLD.fuel_before,OLD.fuel_after,
            OLD.server_message,OLD.server_message_sha256,OLD.created_at
        ) OR OLD.status<>'building' OR NEW.status<>'applied'
          OR NEW.row_version<>OLD.row_version+1 OR NEW.applied_at IS NULL
        THEN
            RAISE EXCEPTION 'TARGET_INTEL_GOAL_RESUME_AUTHORITY_INVALID_TRANSITION';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER target_intel_resume_authority_transition
BEFORE UPDATE OR DELETE ON target_intel_goal_resume_authorities
FOR EACH ROW EXECUTE FUNCTION enforce_target_intel_resume_authority_transition();

-- Preserve all existing StageTeam rules while opening exactly two new seams:
-- an authority-backed read-only reviewer can be inserted after the Goal epoch
-- is sealed, and an applied Goal resume can advance only the original
-- Controller WorkItem to the successor dispatch epoch.
CREATE OR REPLACE FUNCTION enforce_stage_work_item_contract()
RETURNS trigger AS $$
DECLARE
    plan stage_team_plans%ROWTYPE;
    controller_turn_resume BOOLEAN := FALSE;
    target_intel_reviewer_insert BOOLEAN := FALSE;
    target_intel_goal_resume BOOLEAN := FALSE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
        SELECT * INTO plan FROM stage_team_plans persisted
         WHERE persisted.id=NEW.team_plan_id
           AND persisted.operation_id=NEW.operation_id
           AND persisted.stage_execution_id=NEW.stage_execution_id
           AND persisted.stage_run_unit_id=NEW.stage_run_unit_id
           AND persisted.scope_snapshot_id=NEW.scope_snapshot_id
           AND persisted.organization_id=NEW.organization_id
         FOR UPDATE;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_OWNER_MISMATCH';
        END IF;
        target_intel_reviewer_insert :=
            NEW.created_by='target_intel_review_freeze'
            AND NEW.execution_profile='read_only_reviewer'
            AND NEW.terminal_contract='intel_review_v1'
            AND NEW.kind='target_intel_read_only_review'
            AND NEW.role='intel_goal_reviewer'
            AND NEW.output_schema='intel_review.v1'
            AND NEW.required_for_barrier=FALSE
            AND NEW.dispatch_epoch=plan.dispatch_epoch
            AND plan.stage_kind='target_intel'
            AND plan.requests_closed_at IS NOT NULL
            AND EXISTS (
                SELECT 1 FROM target_intel_goal_review_freeze_authorities authority
                 JOIN target_intel_goal_epochs epoch ON epoch.id=authority.goal_epoch_id
                 WHERE authority.reviewer_work_item_id=NEW.id
                   AND authority.operation_id=NEW.operation_id
                   AND authority.organization_id=NEW.organization_id
                   AND authority.stage_execution_id=NEW.stage_execution_id
                   AND authority.stage_run_unit_id=NEW.stage_run_unit_id
                   AND authority.scope_snapshot_id=NEW.scope_snapshot_id
                   AND authority.team_plan_id=NEW.team_plan_id
                   AND authority.bundle_sha256=NEW.input_manifest_hash
                   AND authority.status='building'
                   AND epoch.status='sealed_for_review'
                   AND (
                       plan.final_submitter_worker_run_id IS NULL
                       OR (
                           plan.final_submitter_worker_run_id=epoch.controller_worker_run_id
                           AND EXISTS (
                               SELECT 1 FROM stage_deliverable_submissions submission
                                WHERE submission.worker_run_id=plan.final_submitter_worker_run_id
                                  AND submission.operation_id=plan.operation_id
                                  AND submission.stage_execution_id=plan.stage_execution_id
                                  AND submission.stage_run_unit_id=plan.stage_run_unit_id
                                  AND submission.organization_id=plan.organization_id
                                  AND submission.stage_kind='target_intel'
                           )
                       )
                   )
            );
        IF (plan.requests_closed_at IS NOT NULL OR NEW.dispatch_epoch<>plan.dispatch_epoch)
            AND NOT target_intel_reviewer_insert
        THEN
            RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CLOSED';
        END IF;
        IF NOT (plan.allowed_worker_roles ? NEW.role) AND NOT target_intel_reviewer_insert THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_ROLE_NOT_ALLOWED';
        END IF;
        IF NEW.created_by='target_intel_review_freeze' AND NOT target_intel_reviewer_insert THEN
            RAISE EXCEPTION 'TARGET_INTEL_REVIEWER_FREEZE_AUTHORITY_REQUIRED';
        END IF;
        IF NEW.created_by='gate_repair' AND NOT EXISTS (
            SELECT 1 FROM stage_team_repair_generations generation
             WHERE generation.team_plan_id=plan.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
        ) THEN
            RAISE EXCEPTION 'STAGE_WORK_ITEM_REPAIR_GENERATION_REQUIRED';
        END IF;
        RETURN NEW;
    END IF;

    target_intel_goal_resume :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.status IN ('running','waiting_dependency')
        AND NEW.status='waiting_dependency'
        AND NEW.terminal_at IS NOT DISTINCT FROM OLD.terminal_at
        AND EXISTS (
            SELECT 1 FROM target_intel_goal_resume_authorities authority
             WHERE authority.controller_work_item_id=OLD.id
               AND authority.team_plan_id=OLD.team_plan_id
               AND authority.source_item_row_version=OLD.row_version
               AND authority.successor_goal_epoch=NEW.dispatch_epoch
               AND authority.status='building'
        );
    IF ROW(
        NEW.id,NEW.team_plan_id,NEW.operation_id,NEW.stage_execution_id,
        NEW.stage_run_unit_id,NEW.scope_snapshot_id,NEW.organization_id,
        NEW.kind,NEW.stable_key,NEW.role,NEW.input_manifest_hash,NEW.input_refs,
        NEW.required_for_barrier,NEW.conflict_key,NEW.priority,
        NEW.attempt_policy,NEW.budget,NEW.output_schema,NEW.created_by,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.team_plan_id,OLD.operation_id,OLD.stage_execution_id,
        OLD.stage_run_unit_id,OLD.scope_snapshot_id,OLD.organization_id,
        OLD.kind,OLD.stable_key,OLD.role,OLD.input_manifest_hash,OLD.input_refs,
        OLD.required_for_barrier,OLD.conflict_key,OLD.priority,
        OLD.attempt_policy,OLD.budget,OLD.output_schema,OLD.created_by,OLD.created_at
    ) OR (NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch AND NOT target_intel_goal_resume)
    THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_IMMUTABLE';
    END IF;
    IF NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_VERSION_CAS_REQUIRED';
    END IF;
    IF target_intel_goal_resume THEN
        RETURN NEW;
    END IF;
    controller_turn_resume :=
        OLD.status='superseded' AND NEW.status='waiting_dependency'
        AND OLD.terminal_at IS NOT NULL AND NEW.terminal_at IS NULL
        AND EXISTS (
            SELECT 1 FROM stage_team_controller_turn_resumes authority
            JOIN stage_team_plans resumed_plan ON resumed_plan.id=authority.team_plan_id
             WHERE authority.team_plan_id=OLD.team_plan_id
               AND authority.leader_work_item_id=OLD.id
               AND authority.status='building'
               AND authority.source_item_row_version=OLD.row_version
               AND resumed_plan.dispatch_epoch=authority.resume_dispatch_epoch
               AND resumed_plan.requests_closed_at IS NULL
        );
    IF NOT (
        (OLD.status='queued' AND NEW.status IN ('claimed','running','superseded'))
        OR (OLD.status='claimed' AND NEW.status IN ('queued','running','recovery_required','superseded'))
        OR (OLD.status='running' AND NEW.status IN ('waiting_dependency','completed','retry_pending','recovery_required','exhausted','superseded'))
        OR (OLD.status='waiting_dependency' AND NEW.status IN ('queued','running','recovery_required','superseded'))
        OR (OLD.status='retry_pending' AND NEW.status IN ('queued','exhausted','superseded'))
        OR (OLD.status='recovery_required' AND NEW.status IN ('queued','completed','exhausted','superseded'))
        OR controller_turn_resume
    ) THEN
        RAISE EXCEPTION 'STAGE_WORK_ITEM_INVALID_TRANSITION';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()
RETURNS trigger AS $$
DECLARE
    repair_advance BOOLEAN := FALSE;
    controller_turn_resume_advance BOOLEAN := FALSE;
    target_intel_goal_resume_advance BOOLEAN := FALSE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NOT (NEW.allowed_worker_roles ? NEW.leader_role)
            OR (NEW.aggregator_kind='worker' AND NOT (NEW.allowed_worker_roles ? NEW.aggregator_role))
        THEN
            RAISE EXCEPTION 'STAGE_TEAM_PLAN_ROLE_NOT_ALLOWED';
        END IF;
        IF EXISTS (SELECT 1 FROM stage_worker_runs worker WHERE worker.stage_run_unit_id=NEW.stage_run_unit_id) THEN
            RAISE EXCEPTION 'STAGE_TEAM_PLAN_MUST_PRECEDE_WORKERS';
        END IF;
        RETURN NEW;
    END IF;
    IF ROW(
        NEW.id,NEW.operation_id,NEW.stage_execution_id,NEW.stage_run_unit_id,
        NEW.scope_snapshot_id,NEW.organization_id,NEW.stage_kind,NEW.unit_generation,
        NEW.schema_version,NEW.plan_version,NEW.plan_hash,NEW.leader_role,
        NEW.aggregator_kind,NEW.aggregator_role,NEW.allowed_worker_roles,
        NEW.max_workers_total,NEW.max_workers_active,NEW.dynamic_requests_allowed,
        NEW.dynamic_request_policy,NEW.final_submitter_kind,
        NEW.created_from_stage_spec_hash,NEW.created_at
    ) IS DISTINCT FROM ROW(
        OLD.id,OLD.operation_id,OLD.stage_execution_id,OLD.stage_run_unit_id,
        OLD.scope_snapshot_id,OLD.organization_id,OLD.stage_kind,OLD.unit_generation,
        OLD.schema_version,OLD.plan_version,OLD.plan_hash,OLD.leader_role,
        OLD.aggregator_kind,OLD.aggregator_role,OLD.allowed_worker_roles,
        OLD.max_workers_total,OLD.max_workers_active,OLD.dynamic_requests_allowed,
        OLD.dynamic_request_policy,OLD.final_submitter_kind,
        OLD.created_from_stage_spec_hash,OLD.created_at
    ) THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_IMMUTABLE';
    END IF;
    repair_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1 FROM stage_team_repair_generations generation
            JOIN stage_team_unit_gaps gap ON gap.id=generation.source_gap_id
             WHERE generation.team_plan_id=OLD.id
               AND generation.dispatch_epoch=NEW.dispatch_epoch
               AND generation.status='building'
               AND gap.source_dispatch_epoch=OLD.dispatch_epoch
               AND gap.source_aggregator_worker_run_id=OLD.final_submitter_worker_run_id
        );
    controller_turn_resume_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1 FROM stage_team_controller_turn_resumes authority
             WHERE authority.team_plan_id=OLD.id AND authority.status='building'
               AND authority.source_dispatch_epoch=OLD.dispatch_epoch
               AND authority.resume_dispatch_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
               AND authority.leader_worker_run_id=OLD.final_submitter_worker_run_id
        );
    target_intel_goal_resume_advance :=
        NEW.dispatch_epoch=OLD.dispatch_epoch+1
        AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
        AND NEW.final_submitter_worker_run_id IS NULL
        AND EXISTS (
            SELECT 1 FROM target_intel_goal_resume_authorities authority
             WHERE authority.team_plan_id=OLD.id AND authority.status='building'
               AND authority.source_goal_epoch=OLD.dispatch_epoch
               AND authority.successor_goal_epoch=NEW.dispatch_epoch
               AND authority.source_plan_row_version=OLD.row_version
               AND authority.controller_worker_run_id IS NOT DISTINCT FROM OLD.final_submitter_worker_run_id
        );
    -- Goal review freezes before final-submitter binding, therefore the exact
    -- Controller worker is not expected in final_submitter_worker_run_id.
    IF NOT target_intel_goal_resume_advance THEN
        target_intel_goal_resume_advance :=
            NEW.dispatch_epoch=OLD.dispatch_epoch+1
            AND OLD.requests_closed_at IS NOT NULL AND NEW.requests_closed_at IS NULL
            AND OLD.final_submitter_worker_run_id IS NULL
            AND NEW.final_submitter_worker_run_id IS NULL
            AND EXISTS (
                SELECT 1 FROM target_intel_goal_resume_authorities authority
                 WHERE authority.team_plan_id=OLD.id AND authority.status='building'
                   AND authority.source_goal_epoch=OLD.dispatch_epoch
                   AND authority.successor_goal_epoch=NEW.dispatch_epoch
                   AND authority.source_plan_row_version=OLD.row_version
            );
    END IF;
    IF NEW.dispatch_epoch IS DISTINCT FROM OLD.dispatch_epoch
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_DISPATCH_EPOCH_IMMUTABLE_OUTSIDE_REPAIR';
    END IF;
    IF NEW.row_version<>OLD.row_version+1 OR NEW.updated_at<OLD.updated_at THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_VERSION_CAS_REQUIRED';
    END IF;
    IF OLD.requests_closed_at IS NOT NULL
        AND NEW.requests_closed_at IS DISTINCT FROM OLD.requests_closed_at
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN';
    END IF;
    IF OLD.final_submitter_worker_run_id IS NOT NULL
        AND NEW.final_submitter_worker_run_id IS DISTINCT FROM OLD.final_submitter_worker_run_id
        AND NOT repair_advance AND NOT controller_turn_resume_advance
        AND NOT target_intel_goal_resume_advance
        AND NOT (
            NEW.final_submitter_worker_run_id IS NOT NULL
            AND EXISTS (
                SELECT 1 FROM stage_worker_runs previous_submitter
                 WHERE previous_submitter.id=OLD.final_submitter_worker_run_id
                   AND previous_submitter.status='superseded'
            )
        )
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_FINAL_SUBMITTER_IMMUTABLE';
    END IF;
    IF NEW.requests_closed_at IS NOT DISTINCT FROM OLD.requests_closed_at
        AND NEW.dispatch_epoch IS NOT DISTINCT FROM OLD.dispatch_epoch
        AND NEW.final_submitter_worker_run_id IS NOT DISTINCT FROM OLD.final_submitter_worker_run_id
    THEN
        RAISE EXCEPTION 'STAGE_TEAM_PLAN_NOOP_UPDATE_FORBIDDEN';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
