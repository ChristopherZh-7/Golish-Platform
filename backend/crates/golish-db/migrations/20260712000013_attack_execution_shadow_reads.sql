-- Candidate execution dual-write whole-record mirror and shadow comparison.
-- This is deliberately separate from operation_state.state_blob: runtime V2Only
-- never resumes or reads through the legacy checkpoint document.

CREATE UNIQUE INDEX operation_state_attack_contract_owner
    ON operation_state(operation_id, attack_execution_contract);

CREATE TABLE attack_execution_shadow_reads (
    stage_run_unit_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    -- Frozen audit identity. Deliberately no FK to the mutable live org tree.
    organization_id UUID NOT NULL,
    attack_execution_contract TEXT NOT NULL CHECK (
        attack_execution_contract IN (
            'dual_write_read_legacy',
            'dual_write_read_v2_fallback'
        )
    ),
    stage_kind TEXT NOT NULL DEFAULT 'attack_candidate'
        CHECK (stage_kind = 'attack_candidate'),
    legacy_record JSONB NOT NULL CHECK (
        jsonb_typeof(legacy_record) = 'object'
        AND OCTET_LENGTH(legacy_record::TEXT) <= 262144
    ),
    legacy_record_hash TEXT NOT NULL CHECK (legacy_record_hash ~ '^[0-9a-f]{64}$'),
    comparison TEXT CHECK (comparison IN ('match', 'mismatch', 'v2_missing')),
    selected_source TEXT CHECK (
        selected_source IN ('legacy', 'v2', 'legacy_fallback')
    ),
    selected_record_hash TEXT CHECK (selected_record_hash ~ '^[0-9a-f]{64}$'),
    compared_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, stage_run_unit_id),
    FOREIGN KEY (
        stage_run_unit_id,operation_id,stage_execution_id,organization_id,stage_kind
    ) REFERENCES stage_run_units(
        id,operation_id,stage_execution_id,organization_id,stage_kind
    ) ON DELETE CASCADE,
    FOREIGN KEY (operation_id,attack_execution_contract)
        REFERENCES operation_state(operation_id,attack_execution_contract)
        ON DELETE CASCADE,
    CHECK (
        (comparison IS NULL AND selected_source IS NULL
            AND selected_record_hash IS NULL AND compared_at IS NULL)
        OR
        (comparison IS NOT NULL AND selected_source IS NOT NULL
            AND selected_record_hash IS NOT NULL AND compared_at IS NOT NULL)
    ),
    -- The INSERT owner rebuilds V2 from exact relational authority and derives
    -- routing, comparison, selected hash, and chronology before the row exists.
    CONSTRAINT attack_execution_shadow_selected_record_contract CHECK (
        comparison IS NULL
        OR (
            attack_execution_contract = 'dual_write_read_legacy'
            AND selected_source = 'legacy'
            AND selected_record_hash = legacy_record_hash
        )
        OR (
            attack_execution_contract = 'dual_write_read_v2_fallback'
            AND (
                (
                    comparison = 'v2_missing'
                    AND selected_source = 'legacy_fallback'
                    AND selected_record_hash = legacy_record_hash
                )
                OR (
                    comparison IN ('match', 'mismatch')
                    AND selected_source = 'v2'
                )
            )
        )
    ),
    CHECK (updated_at >= created_at)
);

-- Rebuild the V2 side from the exact immutable Candidate decision source.  The
-- legacy side remains an audited caller snapshot so adapter divergence is
-- observable, but neither its hash nor any selection conclusion is trusted.
CREATE FUNCTION rebuild_attack_execution_v2_shadow_record(
    input_stage_run_unit_id UUID,
    input_operation_id UUID,
    input_stage_execution_id UUID,
    input_organization_id UUID
)
RETURNS JSONB AS $$
DECLARE
    authority RECORD;
    manifest_projection JSONB;
    actual_manifest_hash TEXT;
    actual_item_count INTEGER;
    terminal_item_count INTEGER;
    decisions JSONB;
    manifest_decisions JSONB;
    candidate_count INTEGER;
    no_candidate_count INTEGER;
    v2_complete BOOLEAN;
BEGIN
    SELECT unit.scope_snapshot_id,
           wave_unit.id AS wave_unit_id,
           wave_unit.manifest_hash,
           wave_unit.manifest_count,
           handoff.deliverable_submission_id AS decision_submission_id,
           snapshot.project_path_at_freeze
      INTO authority
      FROM stage_run_units AS unit
      JOIN stage_handoffs AS handoff
        ON handoff.source_stage_run_unit_id=unit.id
       AND handoff.operation_id=unit.operation_id
       AND handoff.stage_execution_id=unit.stage_execution_id
       AND handoff.organization_id=unit.organization_id
       AND handoff.from_stage_kind=unit.stage_kind
       AND handoff.scope_snapshot_id=unit.scope_snapshot_id
       AND handoff.invalidated_at IS NULL
      JOIN stage_deliverable_submissions AS submission
        ON submission.id=handoff.deliverable_submission_id
       AND submission.operation_id=unit.operation_id
       AND submission.stage_execution_id=unit.stage_execution_id
       AND submission.stage_run_unit_id=unit.id
       AND submission.organization_id=unit.organization_id
       AND submission.stage_kind=unit.stage_kind
      JOIN operation_org_scope_snapshots AS snapshot
        ON snapshot.id=unit.scope_snapshot_id
       AND snapshot.operation_id=unit.operation_id
       AND snapshot.sealed_at IS NOT NULL
      JOIN attack_wave_runs AS wave
        ON wave.operation_id=unit.operation_id
       AND wave.scope_snapshot_id=unit.scope_snapshot_id
       AND wave.generation=unit.generation
      JOIN attack_wave_units AS wave_unit
        ON wave_unit.wave_run_id=wave.id
       AND wave_unit.operation_id=wave.operation_id
       AND wave_unit.scope_snapshot_id=wave.scope_snapshot_id
       AND wave_unit.organization_id=unit.organization_id
     WHERE unit.id=input_stage_run_unit_id
       AND unit.operation_id=input_operation_id
       AND unit.stage_execution_id=input_stage_execution_id
       AND unit.organization_id=input_organization_id
       AND unit.stage_kind='attack_candidate'
       AND unit.status='passed'
       AND unit.terminal_at IS NOT NULL
       AND wave_unit.manifest_hash IS NOT NULL
       AND wave_unit.manifest_count IS NOT NULL
       AND wave_unit.manifest_frozen_at IS NOT NULL
     FOR SHARE OF unit,handoff,submission,snapshot,wave,wave_unit;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'attack shadow requires exact terminal Unit/handoff/submission/manifest authority'
            USING ERRCODE = '23514';
    END IF;

    -- Close the read-to-freeze race.  Once the shadow row becomes visible the
    -- source-change guards below take over from these row locks.
    PERFORM 1 FROM attack_candidate_work_items AS item
     WHERE item.operation_id=input_operation_id
       AND item.wave_unit_id=authority.wave_unit_id
       AND item.organization_id=input_organization_id
     ORDER BY item.work_item_key,item.id FOR UPDATE;
    PERFORM 1 FROM attack_candidates AS candidate
     WHERE candidate.operation_uuid=input_operation_id
       AND candidate.wave_unit_id=authority.wave_unit_id
       AND candidate.organization_id=input_organization_id
     ORDER BY candidate.candidate_id FOR UPDATE;

    SELECT COUNT(*),COUNT(*) FILTER (WHERE item.decision_kind IS NOT NULL)
      INTO actual_item_count,terminal_item_count
      FROM attack_candidate_work_items AS item
     WHERE item.operation_id=input_operation_id
       AND item.wave_unit_id=authority.wave_unit_id
       AND item.organization_id=input_organization_id;
    IF actual_item_count <= 0
       OR actual_item_count IS DISTINCT FROM authority.manifest_count
       OR terminal_item_count IS DISTINCT FROM actual_item_count THEN
        RAISE EXCEPTION 'attack shadow source manifest is incomplete'
            USING ERRCODE = '23514';
    END IF;

    SELECT jsonb_agg(jsonb_build_object(
               'work_item_key',item.work_item_key,
               'kind',item.decision_kind
           ) ORDER BY item.work_item_key,item.decision_kind)
      INTO manifest_decisions
      FROM attack_candidate_work_items AS item
     WHERE item.operation_id=input_operation_id
       AND item.wave_unit_id=authority.wave_unit_id
       AND item.organization_id=input_organization_id;

    SELECT COALESCE(jsonb_agg(
               jsonb_build_object(
                   'evidence_ids',item_source.evidence_ids,
                   'target_identity_hash',item_source.target_identity_hash,
                   'technique',item_source.technique,
                   'work_item_id',item_source.work_item_id,
                   'work_item_key',item_source.work_item_key
               ) ORDER BY item_source.work_item_key,item_source.work_item_id
           ),'[]'::JSONB)
      INTO manifest_projection
      FROM (
          SELECT item.id AS work_item_id,item.work_item_key,
                 item.target_identity_hash,seed.technique,
                 COALESCE((
                     SELECT jsonb_agg(source.evidence_id ORDER BY source.evidence_id)
                       FROM (
                           SELECT evidence_id FROM attack_candidate_seed_evidence
                            WHERE seed_id=item.seed_id
                           UNION
                           SELECT evidence_id FROM attack_candidate_work_item_evidence
                            WHERE work_item_id=item.id
                              AND role IN ('observation','support')
                       ) AS source
                 ),'[]'::JSONB) AS evidence_ids
            FROM attack_candidate_work_items AS item
            JOIN attack_candidate_seeds AS seed ON seed.id=item.seed_id
           WHERE item.operation_id=input_operation_id
             AND item.wave_unit_id=authority.wave_unit_id
             AND item.organization_id=input_organization_id
      ) AS item_source;
    actual_manifest_hash := 'sha256:' ||
        attack_fact_delta_sha256_jsonb(manifest_projection);
    IF actual_manifest_hash IS DISTINCT FROM authority.manifest_hash THEN
        RAISE EXCEPTION 'attack shadow source manifest hash drift'
            USING ERRCODE = '23514';
    END IF;

    v2_complete := NOT EXISTS (
        SELECT 1
          FROM attack_candidate_work_items AS item
     LEFT JOIN attack_candidates AS candidate
            ON candidate.candidate_id=item.candidate_id
           AND candidate.operation_uuid=item.operation_id
           AND candidate.source_work_item_id=item.id
           AND candidate.decision_stage_run_unit_id=input_stage_run_unit_id
           AND candidate.decision_stage_execution_id=input_stage_execution_id
           AND candidate.decision_deliverable_submission_id=authority.decision_submission_id
         WHERE item.operation_id=input_operation_id
           AND item.wave_unit_id=authority.wave_unit_id
           AND item.organization_id=input_organization_id
           AND (
               (item.decision_kind='candidate' AND (
                   candidate.candidate_id IS NULL
                   OR NOT EXISTS (
                       SELECT 1 FROM attack_candidate_evidence AS link
                        WHERE link.candidate_id=candidate.candidate_id
                          AND link.role='support'
                   )
               ))
               OR (item.decision_kind='no_candidate' AND NOT EXISTS (
                   SELECT 1 FROM attack_candidate_work_item_evidence AS link
                    WHERE link.work_item_id=item.id AND link.role='decision'
               ))
           )
    );

    -- Lock every audit semantic source before validating it.  Together with the
    -- post-seal audit trigger below this closes the read-to-freeze interval even
    -- for a raw SQL finalizer that did not use the repository evidence lock.
    PERFORM 1
      FROM audit_log AS evidence
     WHERE evidence.id IN (
         SELECT link.evidence_id
           FROM attack_candidate_work_items AS item
           JOIN attack_candidate_work_item_evidence AS link
             ON link.work_item_id=item.id AND link.role='decision'
          WHERE item.operation_id=input_operation_id
            AND item.wave_unit_id=authority.wave_unit_id
            AND item.organization_id=input_organization_id
         UNION
         SELECT link.evidence_id
           FROM attack_candidates AS candidate
           JOIN attack_candidate_evidence AS link
             ON link.candidate_id=candidate.candidate_id AND link.role='support'
          WHERE candidate.operation_uuid=input_operation_id
            AND candidate.wave_unit_id=authority.wave_unit_id
            AND candidate.organization_id=input_organization_id
     )
     ORDER BY evidence.id
     FOR SHARE;

    IF EXISTS (
        SELECT 1
          FROM (
              SELECT item.operation_id,item.organization_id,item.target_live_id,link.evidence_id
                FROM attack_candidate_work_items AS item
                JOIN attack_candidate_work_item_evidence AS link
                  ON link.work_item_id=item.id AND link.role='decision'
               WHERE item.operation_id=input_operation_id
                 AND item.wave_unit_id=authority.wave_unit_id
                 AND item.organization_id=input_organization_id
              UNION ALL
              SELECT candidate.operation_uuid,candidate.organization_id,
                     candidate.target_live_id,link.evidence_id
                FROM attack_candidates AS candidate
                JOIN attack_candidate_evidence AS link
                  ON link.candidate_id=candidate.candidate_id AND link.role='support'
               WHERE candidate.operation_uuid=input_operation_id
                 AND candidate.wave_unit_id=authority.wave_unit_id
                 AND candidate.organization_id=input_organization_id
          ) AS source
          JOIN audit_log AS evidence ON evidence.id=source.evidence_id
         WHERE evidence.audit_role IS DISTINCT FROM 'evidence'
            OR evidence.run_id IS DISTINCT FROM source.operation_id
            OR NULLIF(evidence.detail->>'organization_id','')::UUID
                   IS DISTINCT FROM source.organization_id
            OR evidence.project_path IS DISTINCT FROM authority.project_path_at_freeze
            OR evidence.target_id IS DISTINCT FROM source.target_live_id
    ) THEN
        RAISE EXCEPTION 'attack shadow evidence source identity drift'
            USING ERRCODE = '23514';
    END IF;

    IF v2_complete THEN
        SELECT COALESCE(jsonb_agg(
               jsonb_build_object(
                   'work_item_key',projection.work_item_key,
                   'kind',projection.kind,
                   'semantic_hash',attack_fact_delta_sha256_jsonb(projection.payload)
               ) ORDER BY projection.work_item_key,projection.kind,
                          attack_fact_delta_sha256_jsonb(projection.payload)
           ),'[]'::JSONB),
           COUNT(*) FILTER (WHERE projection.kind='candidate'),
           COUNT(*) FILTER (WHERE projection.kind='no_candidate')
          INTO decisions,candidate_count,no_candidate_count
          FROM (
          SELECT item.work_item_key,
                 CASE item.decision_kind
                     WHEN 'candidate' THEN 'candidate'
                     ELSE 'no_candidate'
                 END AS kind,
                 CASE item.decision_kind
                     WHEN 'candidate' THEN jsonb_build_object(
                         'work_item_id',item.id,
                         'candidate_id',candidate.candidate_id,
                         'hypothesis',candidate.hypothesis,
                         'technique',candidate.technique,
                         'rationale',candidate.rationale,
                         'prior_refs',candidate.prior_refs,
                         'suggested_approach',candidate.suggested_approach,
                         'priority',candidate.priority,
                         'execution_plan',candidate.execution_plan,
                         'candidate_plan_hash',candidate.candidate_plan_hash,
                         'risk_class',candidate.risk_class,
                         'evidence_ids',COALESCE((
                             SELECT jsonb_agg(link.evidence_id ORDER BY link.evidence_id)
                               FROM attack_candidate_evidence AS link
                              WHERE link.candidate_id=candidate.candidate_id
                                AND link.role='support'
                         ),'[]'::JSONB)
                     )
                     ELSE jsonb_build_object(
                         'work_item_id',item.id,
                         'reason_code',item.no_candidate_reason_code,
                         'detail',item.no_candidate_detail,
                         'evidence_ids',COALESCE((
                             SELECT jsonb_agg(link.evidence_id ORDER BY link.evidence_id)
                               FROM attack_candidate_work_item_evidence AS link
                              WHERE link.work_item_id=item.id AND link.role='decision'
                         ),'[]'::JSONB)
                     )
                 END AS payload
            FROM attack_candidate_work_items AS item
       LEFT JOIN attack_candidates AS candidate
              ON candidate.candidate_id=item.candidate_id
             AND candidate.operation_uuid=item.operation_id
             AND candidate.source_work_item_id=item.id
           WHERE item.operation_id=input_operation_id
             AND item.wave_unit_id=authority.wave_unit_id
             AND item.organization_id=input_organization_id
          ) AS projection;
    ELSE
        decisions := NULL;
        candidate_count := NULL;
        no_candidate_count := NULL;
    END IF;

    RETURN jsonb_build_object(
        'status',CASE WHEN v2_complete THEN 'complete' ELSE 'missing' END,
        'manifest_decisions',manifest_decisions,
        'record',CASE WHEN v2_complete THEN jsonb_build_object(
            'decisions',decisions,
            'review_counts',jsonb_build_object(
                'wave_unit_count',1,
                'review_closed_unit_count',0,
                'candidate_decision_count',candidate_count,
                'no_candidate_decision_count',no_candidate_count
            )
        ) ELSE NULL END
    );
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION validate_attack_execution_legacy_shadow_record(
    legacy_record JSONB,
    expected_manifest_decisions JSONB
)
RETURNS VOID AS $$
DECLARE
    legacy_decisions JSONB;
    expected_decisions JSONB;
    legacy_candidate_count INTEGER;
    legacy_no_candidate_count INTEGER;
BEGIN
    IF jsonb_typeof(legacy_record) <> 'object'
       OR (SELECT COUNT(*) FROM jsonb_object_keys(legacy_record)) <> 2
       OR jsonb_typeof(legacy_record->'decisions') <> 'array'
       OR jsonb_typeof(legacy_record->'review_counts') <> 'object'
       OR (SELECT COUNT(*) FROM jsonb_object_keys(legacy_record->'review_counts')) <> 4
       OR NOT (legacy_record->'review_counts' ?& ARRAY[
           'wave_unit_count','review_closed_unit_count',
           'candidate_decision_count','no_candidate_decision_count'
       ])
       OR jsonb_typeof(legacy_record->'review_counts'->'wave_unit_count') <> 'number'
       OR jsonb_typeof(legacy_record->'review_counts'->'review_closed_unit_count') <> 'number'
       OR jsonb_typeof(legacy_record->'review_counts'->'candidate_decision_count') <> 'number'
       OR jsonb_typeof(legacy_record->'review_counts'->'no_candidate_decision_count') <> 'number'
       OR COALESCE(legacy_record->'review_counts'->>'wave_unit_count','') !~ '^[0-9]+$'
       OR COALESCE(legacy_record->'review_counts'->>'review_closed_unit_count','') !~ '^[0-9]+$'
       OR COALESCE(legacy_record->'review_counts'->>'candidate_decision_count','') !~ '^[0-9]+$'
       OR COALESCE(legacy_record->'review_counts'->>'no_candidate_decision_count','') !~ '^[0-9]+$'
    THEN
        RAISE EXCEPTION 'attack shadow legacy record shape is invalid'
            USING ERRCODE = '23514';
    END IF;
    IF EXISTS (
        SELECT 1 FROM jsonb_array_elements(legacy_record->'decisions') AS decision(value)
         WHERE jsonb_typeof(value) <> 'object'
            OR (SELECT COUNT(*) FROM jsonb_object_keys(value)) <> 3
            OR NOT (value ?& ARRAY['work_item_key','kind','semantic_hash'])
            OR jsonb_typeof(value->'work_item_key') <> 'string'
            OR jsonb_typeof(value->'kind') <> 'string'
            OR jsonb_typeof(value->'semantic_hash') <> 'string'
            OR BTRIM(COALESCE(value->>'work_item_key','')) = ''
            OR value->>'kind' NOT IN ('candidate','no_candidate')
            OR COALESCE(value->>'semantic_hash','') !~ '^[0-9a-f]{64}$'
    ) OR EXISTS (
        SELECT 1 FROM jsonb_array_elements(legacy_record->'decisions') AS decision(value)
         GROUP BY value->>'work_item_key' HAVING COUNT(*) <> 1
    ) THEN
        RAISE EXCEPTION 'attack shadow legacy decisions are invalid'
            USING ERRCODE = '23514';
    END IF;
    SELECT COUNT(*) FILTER (WHERE value->>'kind'='candidate'),
           COUNT(*) FILTER (WHERE value->>'kind'='no_candidate')
      INTO legacy_candidate_count,legacy_no_candidate_count
      FROM jsonb_array_elements(legacy_record->'decisions');
    IF (legacy_record->'review_counts'->>'wave_unit_count')::INTEGER <> 1
       OR (legacy_record->'review_counts'->>'review_closed_unit_count')::INTEGER <> 0
       OR (legacy_record->'review_counts'->>'candidate_decision_count')::INTEGER
              IS DISTINCT FROM legacy_candidate_count
       OR (legacy_record->'review_counts'->>'no_candidate_decision_count')::INTEGER
              IS DISTINCT FROM legacy_no_candidate_count THEN
        RAISE EXCEPTION 'attack shadow legacy review counts are invalid'
            USING ERRCODE = '23514';
    END IF;
    SELECT jsonb_agg(jsonb_build_object(
               'work_item_key',value->>'work_item_key','kind',value->>'kind'
           ) ORDER BY value->>'work_item_key',value->>'kind')
      INTO legacy_decisions FROM jsonb_array_elements(legacy_record->'decisions');
    SELECT jsonb_agg(jsonb_build_object(
               'work_item_key',value->>'work_item_key','kind',value->>'kind'
           ) ORDER BY value->>'work_item_key',value->>'kind')
      INTO expected_decisions FROM jsonb_array_elements(expected_manifest_decisions);
    IF legacy_decisions IS DISTINCT FROM expected_decisions THEN
        RAISE EXCEPTION 'attack shadow legacy source does not cover the exact manifest'
            USING ERRCODE = '23514';
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION validate_attack_execution_shadow_contract()
RETURNS trigger AS $$
DECLARE
    v2_snapshot JSONB;
    v2_record JSONB;
    v2_status TEXT;
    canonical_legacy_hash TEXT;
BEGIN
    IF NEW.comparison IS NOT NULL
       OR NEW.selected_source IS NOT NULL
       OR NEW.selected_record_hash IS NOT NULL
       OR NEW.compared_at IS NOT NULL THEN
        RAISE EXCEPTION 'attack execution shadow rows must be inserted pending'
            USING ERRCODE = '23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM operation_state operation
         WHERE operation.operation_id=NEW.operation_id
           AND operation.superseded_by IS NULL
           AND operation.attack_execution_contract=NEW.attack_execution_contract
    ) THEN
        RAISE EXCEPTION 'attack execution shadow contract does not match frozen operation'
            USING ERRCODE = '23514';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM stage_run_units unit
         WHERE unit.id=NEW.stage_run_unit_id
           AND unit.operation_id=NEW.operation_id
           AND unit.stage_execution_id=NEW.stage_execution_id
           AND unit.organization_id=NEW.organization_id
           AND unit.stage_kind='attack_candidate'
           AND unit.status='passed'
           AND unit.terminal_at IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'attack execution shadow sample requires exact final-passed Candidate Unit'
            USING ERRCODE = '23514';
    END IF;
    v2_snapshot := rebuild_attack_execution_v2_shadow_record(
        NEW.stage_run_unit_id,NEW.operation_id,NEW.stage_execution_id,NEW.organization_id
    );
    v2_status := v2_snapshot->>'status';
    v2_record := v2_snapshot->'record';
    IF v2_status NOT IN ('complete','missing') THEN
        RAISE EXCEPTION 'attack shadow V2 rebuild status is invalid'
            USING ERRCODE = '23514';
    END IF;
    PERFORM validate_attack_execution_legacy_shadow_record(
        NEW.legacy_record,v2_snapshot->'manifest_decisions'
    );
    canonical_legacy_hash := attack_fact_delta_sha256_jsonb(NEW.legacy_record);
    IF NEW.legacy_record_hash IS DISTINCT FROM canonical_legacy_hash THEN
        RAISE EXCEPTION 'attack shadow legacy record hash mismatch'
            USING ERRCODE = '23514';
    END IF;
    NEW.created_at := NOW();
    NEW.updated_at := NEW.created_at;
    NEW.compared_at := NEW.created_at;
    NEW.comparison := CASE
        WHEN v2_status='missing' THEN 'v2_missing'
        WHEN NEW.legacy_record = v2_record THEN 'match'
        ELSE 'mismatch'
    END;
    NEW.selected_source := CASE NEW.attack_execution_contract
        WHEN 'dual_write_read_legacy' THEN 'legacy'
        WHEN 'dual_write_read_v2_fallback' THEN
            CASE WHEN v2_status='complete' THEN 'v2' ELSE 'legacy_fallback' END
    END;
    NEW.selected_record_hash := CASE NEW.selected_source
        WHEN 'legacy' THEN canonical_legacy_hash
        WHEN 'legacy_fallback' THEN canonical_legacy_hash
        ELSE attack_fact_delta_sha256_jsonb(v2_record)
    END;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_execution_shadow_contract_owner
BEFORE INSERT ON attack_execution_shadow_reads
FOR EACH ROW EXECUTE FUNCTION validate_attack_execution_shadow_contract();

CREATE FUNCTION attack_execution_shadow_source_is_frozen(
    input_operation_id UUID,
    input_wave_unit_id UUID,
    input_organization_id UUID
)
RETURNS BOOLEAN AS $$
    SELECT EXISTS (
        SELECT 1
          FROM attack_wave_units AS wave_unit
          JOIN attack_wave_runs AS wave
            ON wave.id=wave_unit.wave_run_id
           AND wave.operation_id=wave_unit.operation_id
           AND wave.scope_snapshot_id=wave_unit.scope_snapshot_id
          JOIN stage_run_units AS unit
            ON unit.operation_id=wave.operation_id
           AND unit.scope_snapshot_id=wave.scope_snapshot_id
           AND unit.organization_id=wave_unit.organization_id
           AND unit.generation=wave.generation
           AND unit.stage_kind='attack_candidate'
           AND unit.status='passed'
           AND unit.terminal_at IS NOT NULL
          JOIN stage_handoffs AS handoff
            ON handoff.source_stage_run_unit_id=unit.id
           AND handoff.operation_id=unit.operation_id
           AND handoff.stage_execution_id=unit.stage_execution_id
           AND handoff.organization_id=unit.organization_id
           AND handoff.from_stage_kind=unit.stage_kind
           AND handoff.scope_snapshot_id=unit.scope_snapshot_id
           AND handoff.invalidated_at IS NULL
         WHERE wave_unit.operation_id=input_operation_id
           AND wave_unit.organization_id=input_organization_id
           AND wave_unit.id=input_wave_unit_id
           AND wave_unit.manifest_frozen_at IS NOT NULL
           AND wave_unit.manifest_count > 0
           AND NOT EXISTS (
               SELECT 1 FROM attack_candidate_work_items AS item
                WHERE item.operation_id=wave_unit.operation_id
                  AND item.wave_unit_id=wave_unit.id
                  AND item.organization_id=wave_unit.organization_id
                  AND item.decision_kind IS NULL
           )
    );
$$ LANGUAGE sql STABLE;

CREATE FUNCTION reject_sealed_attack_shadow_handoff_invalidation()
RETURNS trigger AS $$
BEGIN
    IF OLD.invalidated_at IS NULL
       AND NEW.invalidated_at IS NOT NULL
       AND EXISTS (
           SELECT 1
             FROM attack_execution_shadow_reads AS shadow
            WHERE shadow.operation_id=OLD.operation_id
              AND shadow.stage_run_unit_id=OLD.source_stage_run_unit_id
              AND shadow.stage_execution_id=OLD.stage_execution_id
              AND shadow.organization_id=OLD.organization_id
              AND shadow.stage_kind=OLD.from_stage_kind
       ) THEN
        RAISE EXCEPTION 'sealed Candidate shadow final handoff cannot be invalidated'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_shadow_final_handoff_immutable
BEFORE UPDATE OF invalidated_at ON stage_handoffs
FOR EACH ROW EXECUTE FUNCTION reject_sealed_attack_shadow_handoff_invalidation();

CREATE FUNCTION reject_attack_shadow_projection_source_change()
RETURNS trigger AS $$
DECLARE
    source_operation_id UUID;
    source_wave_unit_id UUID;
    source_organization_id UUID;
BEGIN
    IF TG_TABLE_NAME='attack_candidates' THEN
        source_operation_id := CASE WHEN TG_OP='DELETE' THEN OLD.operation_uuid ELSE NEW.operation_uuid END;
        source_wave_unit_id := CASE WHEN TG_OP='DELETE' THEN OLD.wave_unit_id ELSE NEW.wave_unit_id END;
        source_organization_id := CASE WHEN TG_OP='DELETE' THEN OLD.organization_id ELSE NEW.organization_id END;
        IF TG_OP='DELETE' THEN
            IF attack_execution_shadow_source_is_frozen(
                source_operation_id,source_wave_unit_id,source_organization_id
            ) THEN
                RAISE EXCEPTION 'attack shadow Candidate projection source is immutable'
                    USING ERRCODE = '23514';
            END IF;
            RETURN OLD;
        END IF;
        IF attack_execution_shadow_source_is_frozen(
               source_operation_id,source_wave_unit_id,source_organization_id
           ) AND (
               NEW.candidate_id IS DISTINCT FROM OLD.candidate_id
               OR NEW.operation_uuid IS DISTINCT FROM OLD.operation_uuid
               OR NEW.wave_unit_id IS DISTINCT FROM OLD.wave_unit_id
               OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
               OR NEW.source_work_item_id IS DISTINCT FROM OLD.source_work_item_id
               OR NEW.decision_stage_execution_id IS DISTINCT FROM OLD.decision_stage_execution_id
               OR NEW.decision_stage_run_unit_id IS DISTINCT FROM OLD.decision_stage_run_unit_id
               OR NEW.decision_deliverable_submission_id IS DISTINCT FROM OLD.decision_deliverable_submission_id
               OR NEW.hypothesis IS DISTINCT FROM OLD.hypothesis
               OR NEW.technique IS DISTINCT FROM OLD.technique
               OR NEW.rationale IS DISTINCT FROM OLD.rationale
               OR NEW.prior_refs IS DISTINCT FROM OLD.prior_refs
               OR NEW.suggested_approach IS DISTINCT FROM OLD.suggested_approach
               OR NEW.priority IS DISTINCT FROM OLD.priority
               OR NEW.execution_plan IS DISTINCT FROM OLD.execution_plan
               OR NEW.candidate_plan_hash IS DISTINCT FROM OLD.candidate_plan_hash
               OR NEW.risk_class IS DISTINCT FROM OLD.risk_class
           ) THEN
            RAISE EXCEPTION 'attack shadow Candidate projection source is immutable'
                USING ERRCODE = '23514';
        END IF;
    ELSE
        source_operation_id := CASE WHEN TG_OP='DELETE' THEN OLD.operation_id ELSE NEW.operation_id END;
        source_wave_unit_id := CASE WHEN TG_OP='DELETE' THEN OLD.wave_unit_id ELSE NEW.wave_unit_id END;
        source_organization_id := CASE WHEN TG_OP='DELETE' THEN OLD.organization_id ELSE NEW.organization_id END;
        IF TG_OP='DELETE' THEN
            IF attack_execution_shadow_source_is_frozen(
                source_operation_id,source_wave_unit_id,source_organization_id
            ) THEN
                RAISE EXCEPTION 'attack shadow work-item projection source is immutable'
                    USING ERRCODE = '23514';
            END IF;
            RETURN OLD;
        END IF;
        IF attack_execution_shadow_source_is_frozen(
               source_operation_id,source_wave_unit_id,source_organization_id
           ) AND (
               NEW.id IS DISTINCT FROM OLD.id
               OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
               OR NEW.wave_unit_id IS DISTINCT FROM OLD.wave_unit_id
               OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
               OR NEW.work_item_key IS DISTINCT FROM OLD.work_item_key
               OR NEW.decision_kind IS DISTINCT FROM OLD.decision_kind
               OR NEW.candidate_id IS DISTINCT FROM OLD.candidate_id
               OR NEW.no_candidate_reason_code IS DISTINCT FROM OLD.no_candidate_reason_code
               OR NEW.no_candidate_detail IS DISTINCT FROM OLD.no_candidate_detail
               OR NEW.decided_at IS DISTINCT FROM OLD.decided_at
           ) THEN
            RAISE EXCEPTION 'attack shadow work-item projection source is immutable'
                USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidates_shadow_projection_source_immutable
BEFORE UPDATE OR DELETE ON attack_candidates
FOR EACH ROW EXECUTE FUNCTION reject_attack_shadow_projection_source_change();

CREATE TRIGGER attack_candidate_work_items_shadow_projection_source_immutable
BEFORE UPDATE OR DELETE ON attack_candidate_work_items
FOR EACH ROW EXECUTE FUNCTION reject_attack_shadow_projection_source_change();

CREATE FUNCTION reject_attack_shadow_evidence_membership_change()
RETURNS trigger AS $$
DECLARE
    owner_id UUID;
    prior_owner_id UUID;
    affects_projection BOOLEAN;
    projection_frozen BOOLEAN;
BEGIN
    IF TG_TABLE_NAME='attack_candidate_evidence' THEN
        owner_id := CASE WHEN TG_OP='DELETE' THEN OLD.candidate_id ELSE NEW.candidate_id END;
        prior_owner_id := CASE WHEN TG_OP='UPDATE' THEN OLD.candidate_id ELSE owner_id END;
        PERFORM 1 FROM attack_candidates
         WHERE candidate_id IN (owner_id,prior_owner_id)
         ORDER BY candidate_id FOR SHARE;
        affects_projection := CASE WHEN TG_OP='INSERT' THEN NEW.role='support'
            WHEN TG_OP='DELETE' THEN OLD.role='support'
            ELSE OLD.role='support' OR NEW.role='support' END;
        SELECT COALESCE(BOOL_OR(attack_execution_shadow_source_is_frozen(
                   operation_uuid,wave_unit_id,organization_id
               )),FALSE)
          INTO projection_frozen
          FROM attack_candidates
         WHERE candidate_id IN (owner_id,prior_owner_id);
    ELSE
        owner_id := CASE WHEN TG_OP='DELETE' THEN OLD.work_item_id ELSE NEW.work_item_id END;
        prior_owner_id := CASE WHEN TG_OP='UPDATE' THEN OLD.work_item_id ELSE owner_id END;
        PERFORM 1 FROM attack_candidate_work_items
         WHERE id IN (owner_id,prior_owner_id)
         ORDER BY id FOR SHARE;
        affects_projection := CASE WHEN TG_OP='INSERT' THEN NEW.role='decision'
            WHEN TG_OP='DELETE' THEN OLD.role='decision'
            ELSE OLD.role='decision' OR NEW.role='decision' END;
        SELECT COALESCE(BOOL_OR(attack_execution_shadow_source_is_frozen(
                   operation_id,wave_unit_id,organization_id
               )),FALSE)
          INTO projection_frozen
          FROM attack_candidate_work_items
         WHERE id IN (owner_id,prior_owner_id);
    END IF;
    IF affects_projection AND projection_frozen THEN
        RAISE EXCEPTION 'attack shadow evidence membership is immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_candidate_evidence_shadow_projection_immutable
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_evidence
FOR EACH ROW EXECUTE FUNCTION reject_attack_shadow_evidence_membership_change();

CREATE TRIGGER attack_candidate_work_item_evidence_shadow_projection_immutable
BEFORE INSERT OR UPDATE OR DELETE ON attack_candidate_work_item_evidence
FOR EACH ROW EXECUTE FUNCTION reject_attack_shadow_evidence_membership_change();

CREATE FUNCTION reject_attack_shadow_audit_semantic_change()
RETURNS trigger AS $$
DECLARE
    is_frozen_source BOOLEAN;
    true_target_delete BOOLEAN;
BEGIN
    SELECT EXISTS (
        SELECT 1
          FROM (
              SELECT candidate.operation_uuid AS operation_id,
                     candidate.wave_unit_id,candidate.organization_id
                FROM attack_candidate_evidence AS link
                JOIN attack_candidates AS candidate ON candidate.candidate_id=link.candidate_id
               WHERE link.evidence_id=OLD.id AND link.role='support'
              UNION ALL
              SELECT item.operation_id,item.wave_unit_id,item.organization_id
                FROM attack_candidate_work_item_evidence AS link
                JOIN attack_candidate_work_items AS item ON item.id=link.work_item_id
               WHERE link.evidence_id=OLD.id AND link.role='decision'
          ) AS source
         WHERE attack_execution_shadow_source_is_frozen(
             source.operation_id,source.wave_unit_id,source.organization_id
         )
    ) INTO is_frozen_source;
    IF NOT is_frozen_source THEN
        RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
    END IF;
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'attack shadow evidence audit source cannot be deleted'
            USING ERRCODE = '23514';
    END IF;
    true_target_delete := OLD.target_id IS NOT NULL
        AND NEW.target_id IS NULL
        AND NOT EXISTS (SELECT 1 FROM targets WHERE id=OLD.target_id)
        AND (to_jsonb(NEW)-'target_id') IS NOT DISTINCT FROM (to_jsonb(OLD)-'target_id');
    IF true_target_delete THEN
        RETURN NEW;
    END IF;
    IF to_jsonb(NEW) IS DISTINCT FROM to_jsonb(OLD) THEN
        RAISE EXCEPTION 'attack shadow evidence audit semantics are immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_shadow_audit_semantic_source_immutable
BEFORE UPDATE OR DELETE ON audit_log
FOR EACH ROW EXECUTE FUNCTION reject_attack_shadow_audit_semantic_change();

CREATE INDEX attack_execution_shadow_reads_promotion_idx
    ON attack_execution_shadow_reads(attack_execution_contract, comparison);

CREATE FUNCTION reject_attack_execution_shadow_mirror_change()
RETURNS trigger AS $$
BEGIN
    IF NEW.stage_run_unit_id IS DISTINCT FROM OLD.stage_run_unit_id
       OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
       OR NEW.stage_execution_id IS DISTINCT FROM OLD.stage_execution_id
       OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
       OR NEW.attack_execution_contract IS DISTINCT FROM OLD.attack_execution_contract
       OR NEW.stage_kind IS DISTINCT FROM OLD.stage_kind
       OR NEW.legacy_record IS DISTINCT FROM OLD.legacy_record
       OR NEW.legacy_record_hash IS DISTINCT FROM OLD.legacy_record_hash
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'attack execution legacy semantic mirror is immutable'
            USING ERRCODE = '23514';
    END IF;
    IF OLD.comparison IS NULL THEN
        RAISE EXCEPTION 'attack execution shadow selection is sealed only by INSERT authority'
            USING ERRCODE = '23514';
    ELSIF NEW.comparison IS DISTINCT FROM OLD.comparison
       OR NEW.selected_source IS DISTINCT FROM OLD.selected_source
       OR NEW.selected_record_hash IS DISTINCT FROM OLD.selected_record_hash
       OR NEW.compared_at IS DISTINCT FROM OLD.compared_at
       OR NEW.updated_at IS DISTINCT FROM OLD.updated_at THEN
        RAISE EXCEPTION 'attack execution shadow attestation is immutable once closed'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_execution_shadow_mirror_immutable
BEFORE UPDATE ON attack_execution_shadow_reads
FOR EACH ROW EXECUTE FUNCTION reject_attack_execution_shadow_mirror_change();

CREATE FUNCTION reject_direct_attack_execution_shadow_delete()
RETURNS trigger AS $$
BEGIN
    -- Deployment samples are immutable evidence until their owning runtime
    -- Unit/operation is cleaned up.  Trigger depth is not authority: an
    -- arbitrary nested trigger can otherwise erase a mismatch.  A real FK
    -- cascade observes at least one already-removed owning parent.
    IF EXISTS (
        SELECT 1
          FROM stage_run_units AS unit
         WHERE unit.id = OLD.stage_run_unit_id
           AND unit.operation_id = OLD.operation_id
           AND unit.stage_execution_id = OLD.stage_execution_id
           AND unit.organization_id = OLD.organization_id
           AND unit.stage_kind = OLD.stage_kind
    ) AND EXISTS (
        SELECT 1
          FROM operation_state AS operation
         WHERE operation.operation_id = OLD.operation_id
           AND operation.attack_execution_contract = OLD.attack_execution_contract
    ) THEN
        RAISE EXCEPTION 'attack execution shadow samples cannot be deleted directly'
            USING ERRCODE = '23514';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER attack_execution_shadow_delete_by_owner_only
BEFORE DELETE ON attack_execution_shadow_reads
FOR EACH ROW EXECUTE FUNCTION reject_direct_attack_execution_shadow_delete();
