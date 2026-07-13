-- Canonical cited Reporting read model.
--
-- A revision freezes the complete ordered set of reportable canonical rows,
-- not merely the rows a renderer happened to consume. Validation and
-- publication are independent axes. Final/superseded history and all cited
-- provenance are retained with RESTRICT FKs.

-- ---------------------------------------------------------------------------
-- Monotonic versions for every mutable canonical source consumed by reports.
-- ---------------------------------------------------------------------------

ALTER TABLE findings
    ADD COLUMN row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0);
ALTER TABLE technique_outcomes
    ADD COLUMN row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0);
ALTER TABLE finding_lineage
    ADD COLUMN row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0);
ALTER TABLE internal_asset_observations
    ADD COLUMN row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0);
ALTER TABLE objective_attempts
    ADD COLUMN row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0);

CREATE FUNCTION bump_reportable_row_version()
RETURNS trigger AS $$
BEGIN
    -- target_live_id is only a nullable pointer to the live inventory row. The
    -- immutable at-time target fields remain the canonical audit snapshot. An
    -- FK-driven SET NULL after that live row has been deleted must therefore
    -- neither mutate canonical content nor advance its source version.
    IF TG_TABLE_NAME = 'candidate_attempts' THEN
        IF OLD.status IN (
                'verified','refuted','blocked','retryable_failed','abandoned'
            )
            AND OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (
                SELECT 1 FROM targets WHERE id = OLD.target_live_id
            )
            AND (to_jsonb(NEW) - 'target_live_id')
                = (to_jsonb(OLD) - 'target_live_id')
        THEN
            RETURN NEW;
        END IF;
    END IF;
    NEW.row_version := OLD.row_version + 1;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER findings_reportable_row_version
BEFORE UPDATE ON findings FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER technique_outcomes_reportable_row_version
BEFORE UPDATE ON technique_outcomes FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER attack_candidates_reportable_row_version
BEFORE UPDATE ON attack_candidates FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER candidate_attempts_reportable_row_version
BEFORE UPDATE ON candidate_attempts FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER finding_lineage_reportable_row_version
BEFORE UPDATE ON finding_lineage FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER footholds_reportable_row_version
BEFORE UPDATE ON footholds FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER internal_asset_observations_reportable_row_version
BEFORE UPDATE ON internal_asset_observations FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER attack_paths_reportable_row_version
BEFORE UPDATE ON attack_paths FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER objective_attempts_reportable_row_version
BEFORE UPDATE ON objective_attempts FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER cleanup_obligations_reportable_row_version
BEFORE UPDATE ON cleanup_obligations FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();
CREATE TRIGGER cleanup_waivers_reportable_row_version
BEFORE UPDATE ON cleanup_waivers FOR EACH ROW EXECUTE FUNCTION bump_reportable_row_version();

-- Terminal Candidate/Cleanup rows are canonical event sources. Their source
-- version is embedded in deterministic outbox identity, so even a no-op
-- UPDATE would make an exact replay derive a different source version. Allow
-- the single nonterminal -> terminal transition, then freeze every canonical
-- at-time/event field. Only FK-driven clearing of the non-canonical live
-- target pointer remains permitted, without changing the source version.
CREATE FUNCTION reject_terminal_canonical_source_change()
RETURNS trigger AS $$
BEGIN
    -- Permit only the FK-driven removal of a stale live target pointer. A
    -- direct UPDATE while the target still exists, or any simultaneous change
    -- to the frozen audit snapshot, remains rejected below.
    IF TG_TABLE_NAME = 'candidate_attempts' THEN
        IF TG_OP = 'UPDATE'
            AND OLD.status IN (
                'verified','refuted','blocked','retryable_failed','abandoned'
            )
            AND OLD.target_live_id IS NOT NULL
            AND NEW.target_live_id IS NULL
            AND NOT EXISTS (
                SELECT 1 FROM targets WHERE id = OLD.target_live_id
            )
            AND (to_jsonb(NEW) - 'target_live_id')
                = (to_jsonb(OLD) - 'target_live_id')
        THEN
            RETURN NEW;
        END IF;
    END IF;
    IF TG_TABLE_NAME = 'candidate_attempts'
        AND OLD.status IN (
            'verified','refuted','blocked','retryable_failed','abandoned'
        )
    THEN
        RAISE EXCEPTION 'TERMINAL_CANONICAL_SOURCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF TG_TABLE_NAME = 'cleanup_obligations'
        AND OLD.status IN ('verified_absent','blocked','waived_by_user')
    THEN
        RAISE EXCEPTION 'TERMINAL_CANONICAL_SOURCE_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER candidate_attempts_canonical_terminal_source_guard
BEFORE UPDATE OR DELETE ON candidate_attempts
FOR EACH ROW EXECUTE FUNCTION reject_terminal_canonical_source_change();

CREATE TRIGGER cleanup_obligations_canonical_terminal_source_guard
BEFORE UPDATE OR DELETE ON cleanup_obligations
FOR EACH ROW EXECUTE FUNCTION reject_terminal_canonical_source_change();

-- ---------------------------------------------------------------------------
-- Report aggregate and two independent state axes.
-- ---------------------------------------------------------------------------

CREATE TABLE reports (
    report_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL UNIQUE,
    project_scope_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    scope_snapshot_hash TEXT NOT NULL CHECK (scope_snapshot_hash ~ '^[0-9a-f]{64}$'),
    current_revision_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (report_id,operation_id,project_scope_id,scope_snapshot_id),
    FOREIGN KEY (operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    FOREIGN KEY (scope_snapshot_id,operation_id)
        REFERENCES operation_org_scope_snapshots(id,operation_id) ON DELETE RESTRICT
);

CREATE TABLE report_revisions (
    revision_id UUID PRIMARY KEY,
    report_id UUID NOT NULL REFERENCES reports(report_id) ON DELETE RESTRICT,
    revision_number INTEGER NOT NULL CHECK (revision_number > 0),
    row_version BIGINT NOT NULL DEFAULT 1 CHECK (row_version > 0),
    transaction_snapshot TEXT NOT NULL CHECK (BTRIM(transaction_snapshot) <> ''),
    source_set_hash TEXT NOT NULL CHECK (source_set_hash ~ '^[0-9a-f]{64}$'),
    validation_status TEXT NOT NULL DEFAULT 'building' CHECK (
        validation_status IN ('building','draft','validated','invalid')
    ),
    publication_status TEXT NOT NULL DEFAULT 'unpublished' CHECK (
        publication_status IN ('unpublished','final','superseded')
    ),
    supersedes_revision_id UUID,
    validation_result JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    validated_at TIMESTAMPTZ,
    finalized_at TIMESTAMPTZ,
    finalized_by_principal_id UUID REFERENCES operator_principals(id) ON DELETE RESTRICT,
    UNIQUE (report_id,revision_id),
    UNIQUE (report_id,revision_number),
    CONSTRAINT report_revision_supersedes_same_report
        FOREIGN KEY (report_id,supersedes_revision_id)
        REFERENCES report_revisions(report_id,revision_id) ON DELETE RESTRICT,
    CHECK (
        (validation_status = 'validated' AND validated_at IS NOT NULL)
        OR (validation_status <> 'validated')
    ),
    CHECK (
        (publication_status IN ('final','superseded')
            AND finalized_at IS NOT NULL
            AND finalized_by_principal_id IS NOT NULL)
        OR (publication_status = 'unpublished'
            AND finalized_at IS NULL
            AND finalized_by_principal_id IS NULL)
    )
);

ALTER TABLE reports
    ADD CONSTRAINT reports_current_revision_belongs_to_report
    FOREIGN KEY (report_id,current_revision_id)
    REFERENCES report_revisions(report_id,revision_id)
    ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

CREATE UNIQUE INDEX report_revisions_one_final
    ON report_revisions(report_id) WHERE publication_status = 'final';

CREATE TABLE report_source_manifest (
    revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    source_kind TEXT NOT NULL CHECK (BTRIM(source_kind) <> ''),
    source_id_kind TEXT NOT NULL CHECK (source_id_kind IN ('uuid','int64','text')),
    source_id_value TEXT NOT NULL CHECK (BTRIM(source_id_value) <> ''),
    source_row_version BIGINT NOT NULL CHECK (source_row_version >= 0),
    content_hash BYTEA NOT NULL CHECK (octet_length(content_hash) = 32),
    PRIMARY KEY (revision_id,ordinal),
    UNIQUE (
        revision_id,source_kind,source_id_kind,source_id_value
    ),
    UNIQUE (
        revision_id,source_kind,source_id_kind,source_id_value,
        source_row_version,content_hash
    )
);

CREATE TABLE report_sections (
    section_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    organization_name_at_snapshot TEXT,
    section_kind TEXT NOT NULL CHECK (section_kind IN (
        'executive_summary','organization','findings','attack_paths',
        'cleanup_residuals','methodology','limitations'
    )),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    rendered_content TEXT,
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^[0-9a-f]{64}$'),
    UNIQUE (revision_id,section_id),
    UNIQUE NULLS NOT DISTINCT (
        revision_id,organization_id_at_time,section_kind,ordinal
    )
);

CREATE TABLE report_claims (
    claim_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL,
    section_id UUID NOT NULL,
    organization_id_at_time UUID,
    claim_kind TEXT NOT NULL CHECK (BTRIM(claim_kind) <> ''),
    subject_ref TEXT NOT NULL CHECK (BTRIM(subject_ref) <> ''),
    predicate TEXT NOT NULL CHECK (BTRIM(predicate) <> ''),
    object_value JSONB NOT NULL,
    claim_hash TEXT NOT NULL CHECK (claim_hash ~ '^[0-9a-f]{64}$'),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    UNIQUE (revision_id,claim_id),
    UNIQUE (revision_id,section_id,ordinal),
    CONSTRAINT report_claims_same_revision_section
        FOREIGN KEY (revision_id,section_id)
        REFERENCES report_sections(revision_id,section_id) ON DELETE RESTRICT
);

CREATE TABLE report_claim_citations (
    citation_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL,
    claim_id UUID NOT NULL,
    citation_ordinal INTEGER NOT NULL CHECK (citation_ordinal >= 0),
    source_type TEXT NOT NULL CHECK (source_type IN ('canonical_fact','evidence_audit')),
    source_kind TEXT NOT NULL,
    source_id_kind TEXT NOT NULL,
    source_id_value TEXT NOT NULL,
    source_row_version BIGINT NOT NULL CHECK (source_row_version >= 0),
    source_hash BYTEA NOT NULL CHECK (octet_length(source_hash) = 32),
    evidence_audit_id BIGINT NOT NULL REFERENCES audit_log(id) ON DELETE RESTRICT,
    organization_id_at_time UUID NOT NULL,
    display_label TEXT NOT NULL CHECK (BTRIM(display_label) <> ''),
    UNIQUE (revision_id,claim_id,citation_ordinal),
    CONSTRAINT report_claim_citations_same_revision
        FOREIGN KEY (revision_id,claim_id)
        REFERENCES report_claims(revision_id,claim_id) ON DELETE RESTRICT,
    CONSTRAINT report_claim_citations_frozen_source
        FOREIGN KEY (
            revision_id,source_kind,source_id_kind,source_id_value,
            source_row_version,source_hash
        ) REFERENCES report_source_manifest(
            revision_id,source_kind,source_id_kind,source_id_value,
            source_row_version,content_hash
        ) ON DELETE RESTRICT
);

CREATE TABLE report_artifact_blobs (
    content_key TEXT PRIMARY KEY CHECK (BTRIM(content_key) <> ''),
    sha256 TEXT NOT NULL UNIQUE CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    storage_path TEXT NOT NULL UNIQUE CHECK (BTRIM(storage_path) <> ''),
    byte_len BIGINT NOT NULL CHECK (byte_len >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE report_revision_artifacts (
    revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    artifact_kind TEXT NOT NULL CHECK (artifact_kind IN ('markdown','json','pdf','docx')),
    content_key TEXT NOT NULL REFERENCES report_artifact_blobs(content_key) ON DELETE RESTRICT,
    redaction_version INTEGER NOT NULL CHECK (redaction_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (revision_id,artifact_kind)
);

-- ---------------------------------------------------------------------------
-- State transitions and immutable historical content.
-- ---------------------------------------------------------------------------

CREATE FUNCTION enforce_report_revision_transition()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.validation_status IN ('validated','invalid')
            OR OLD.publication_status <> 'unpublished'
        THEN
            RAISE EXCEPTION 'FINAL_HISTORY_IMMUTABLE' USING ERRCODE = '23514';
        END IF;
        RETURN OLD;
    END IF;

    IF OLD.publication_status IN ('final','superseded') THEN
        IF NOT (
            OLD.publication_status = 'final'
            AND NEW.publication_status = 'superseded'
            AND (
                to_jsonb(NEW) - 'publication_status' - 'row_version'
            ) = (
                to_jsonb(OLD) - 'publication_status' - 'row_version'
            )
        ) THEN
            RAISE EXCEPTION 'FINAL_HISTORY_IMMUTABLE' USING ERRCODE = '23514';
        END IF;
    END IF;

    IF OLD.validation_status = 'validated'
        AND OLD.publication_status = 'unpublished'
    THEN
        IF NOT (
            NEW.publication_status = 'final'
            AND (
                to_jsonb(NEW)
                    - 'publication_status'
                    - 'finalized_at'
                    - 'finalized_by_principal_id'
                    - 'row_version'
            ) = (
                to_jsonb(OLD)
                    - 'publication_status'
                    - 'finalized_at'
                    - 'finalized_by_principal_id'
                    - 'row_version'
            )
        ) THEN
            RAISE EXCEPTION 'REPORT_VALIDATED_REVISION_IMMUTABLE'
                USING ERRCODE = '23514';
        END IF;
    ELSIF OLD.validation_status = 'validated'
        AND NEW.validation_status <> 'validated'
    THEN
        RAISE EXCEPTION 'REPORT_VALIDATION_IMMUTABLE'
            USING ERRCODE = '23514';
    END IF;
    IF OLD.publication_status = 'unpublished'
        AND NEW.publication_status NOT IN ('unpublished','final')
    THEN
        RAISE EXCEPTION 'REPORT_PUBLICATION_TRANSITION_INVALID' USING ERRCODE = '23514';
    END IF;
    NEW.row_version := OLD.row_version + 1;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER report_revisions_guard
BEFORE UPDATE OR DELETE ON report_revisions
FOR EACH ROW EXECUTE FUNCTION enforce_report_revision_transition();

-- The final transition is a transaction protocol, not an enum flip. The
-- constraint is deferred so the repository can first update the revision and
-- then append the exact immutable outbox event in the same transaction.
CREATE FUNCTION enforce_report_finalization_authority()
RETURNS trigger AS $$
DECLARE
    has_authorized_finalization BOOLEAN;
BEGIN
    IF NEW.publication_status = 'final'
        AND OLD.publication_status IS DISTINCT FROM 'final'
    THEN
        SELECT EXISTS (
            SELECT 1
              FROM reports AS report
              JOIN operator_principals AS principal
                ON principal.id = NEW.finalized_by_principal_id
               AND principal.principal_kind = 'local_operator'
               AND principal.active
              JOIN report_revision_artifacts AS artifact
                ON artifact.revision_id = NEW.revision_id
              JOIN report_artifact_blobs AS blob
                ON blob.content_key = artifact.content_key
              JOIN knowledge_outbox_events AS outbox
                ON outbox.event_name = 'ReportRevisionFinalized.v1'
               AND outbox.schema_version = 1
               AND outbox.project_scope_id = report.project_scope_id
               AND outbox.organization_id_at_time IS NULL
               AND outbox.source_operation_id = report.operation_id
               AND outbox.source_kind = 'report_revision'
               AND outbox.source_id_kind = 'uuid'
               AND outbox.source_id_value = NEW.revision_id::TEXT
               AND outbox.source_stream_key = 'report:' || report.report_id::TEXT
               AND outbox.source_version = NEW.row_version
               AND outbox.occurred_at = NEW.finalized_at
               AND outbox.payload->>'source_stream_key' =
                   'report:' || report.report_id::TEXT
               AND outbox.payload->>'source_version' = NEW.row_version::TEXT
               AND outbox.payload->'structured_payload'->>'reportId' =
                   report.report_id::TEXT
               AND outbox.payload->'structured_payload'->>'revisionId' =
                   NEW.revision_id::TEXT
             WHERE report.report_id = NEW.report_id
               AND report.current_revision_id = NEW.revision_id
               AND NEW.finalized_at IS NOT NULL
        ) INTO has_authorized_finalization;
        IF NOT has_authorized_finalization THEN
            RAISE EXCEPTION 'REPORT_FINALIZATION_AUTHORITY_REQUIRED'
                USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER report_revision_finalization_authority
AFTER UPDATE OF publication_status ON report_revisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_report_finalization_authority();

CREATE FUNCTION reject_final_report_child_mutation()
RETURNS trigger AS $$
DECLARE
    old_publication_status TEXT;
    old_validation_status TEXT;
    new_publication_status TEXT;
    new_validation_status TEXT;
BEGIN
    IF TG_OP IN ('UPDATE','DELETE') THEN
        SELECT publication_status,validation_status
          INTO old_publication_status,old_validation_status
          FROM report_revisions WHERE revision_id = OLD.revision_id
          FOR UPDATE;
        IF old_publication_status IN ('final','superseded')
            OR (
                TG_TABLE_NAME <> 'report_revision_artifacts'
                AND old_validation_status IN ('validated','invalid')
            )
        THEN
            RAISE EXCEPTION 'FINAL_HISTORY_IMMUTABLE' USING ERRCODE = '23514';
        END IF;
    END IF;
    IF TG_OP IN ('INSERT','UPDATE') THEN
        SELECT publication_status,validation_status
          INTO new_publication_status,new_validation_status
          FROM report_revisions WHERE revision_id = NEW.revision_id
          FOR UPDATE;
        IF new_publication_status IN ('final','superseded')
            OR (
                TG_TABLE_NAME <> 'report_revision_artifacts'
                AND new_validation_status IN ('validated','invalid')
            )
        THEN
            RAISE EXCEPTION 'FINAL_HISTORY_IMMUTABLE' USING ERRCODE = '23514';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER report_source_manifest_immutable
BEFORE INSERT OR UPDATE OR DELETE ON report_source_manifest
FOR EACH ROW EXECUTE FUNCTION reject_final_report_child_mutation();
CREATE TRIGGER report_sections_immutable
BEFORE INSERT OR UPDATE OR DELETE ON report_sections
FOR EACH ROW EXECUTE FUNCTION reject_final_report_child_mutation();
CREATE TRIGGER report_claims_immutable
BEFORE INSERT OR UPDATE OR DELETE ON report_claims
FOR EACH ROW EXECUTE FUNCTION reject_final_report_child_mutation();
CREATE TRIGGER report_claim_citations_immutable
BEFORE INSERT OR UPDATE OR DELETE ON report_claim_citations
FOR EACH ROW EXECUTE FUNCTION reject_final_report_child_mutation();
CREATE TRIGGER report_revision_artifacts_immutable
BEFORE INSERT OR UPDATE OR DELETE ON report_revision_artifacts
FOR EACH ROW EXECUTE FUNCTION reject_final_report_child_mutation();

CREATE FUNCTION reject_referenced_report_artifact_blob_mutation()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM report_revision_artifacts AS artifact
         WHERE artifact.content_key = OLD.content_key
    ) THEN
        RAISE EXCEPTION 'REPORT_ARTIFACT_BLOB_IMMUTABLE' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER report_artifact_blobs_immutable_when_referenced
BEFORE UPDATE ON report_artifact_blobs
FOR EACH ROW EXECUTE FUNCTION reject_referenced_report_artifact_blob_mutation();

CREATE FUNCTION reject_report_with_retained_revision_delete()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM report_revisions
         WHERE report_id = OLD.report_id
           AND (validation_status IN ('validated','invalid')
                OR publication_status <> 'unpublished')
    ) THEN
        RAISE EXCEPTION 'FINAL_HISTORY_IMMUTABLE' USING ERRCODE = '23514';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER reports_retain_history
BEFORE DELETE ON reports
FOR EACH ROW EXECUTE FUNCTION reject_report_with_retained_revision_delete();

-- A TechniqueOutcome report source is authoritative only through the exact
-- final-sealed StageHandoff ref that named its row hash and evidence set. Once
-- that handoff is retained by terminal Reporting validation/history, even the
-- otherwise-permitted one-way invalidation would destroy the authority chain.
CREATE FUNCTION reject_retained_report_stage_handoff_change()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM report_source_manifest AS source
          JOIN report_revisions AS revision
            ON revision.revision_id=source.revision_id
         WHERE source.source_kind='stage_handoff'
           AND source.source_id_kind='uuid'
           AND source.source_id_value=OLD.id::text
           AND (
               revision.validation_status IN ('validated','invalid')
               OR revision.publication_status IN ('final','superseded')
           )
    ) THEN
        RAISE EXCEPTION 'REPORT_SEALED_REF_RETAINED' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stage_handoffs_report_sealed_ref_retention
BEFORE UPDATE OR DELETE ON stage_handoffs
FOR EACH ROW EXECUTE FUNCTION reject_retained_report_stage_handoff_change();
