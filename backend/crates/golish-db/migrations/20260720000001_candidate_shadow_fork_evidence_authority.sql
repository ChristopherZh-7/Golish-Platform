-- Candidate stage forks intentionally reason over the immutable evidence owned
-- by their source operation. Keep the whole-record shadow validator aligned
-- with the repository's exact operation_stage_fork_inputs authority instead of
-- treating those inherited rows as foreign evidence.

CREATE OR REPLACE FUNCTION rebuild_attack_execution_v2_shadow_record(
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
           snapshot.project_path_at_freeze,
           CASE
               WHEN wave_unit.entry_stage_fork_input_id IS NULL
               THEN input_operation_id
               ELSE (
                   SELECT fork_input.source_operation_id
                     FROM operation_stage_fork_inputs AS fork_input
                    WHERE fork_input.id=wave_unit.entry_stage_fork_input_id
                      AND fork_input.operation_id=wave_unit.operation_id
                      AND fork_input.target_scope_snapshot_id=wave_unit.scope_snapshot_id
                      AND fork_input.organization_id=wave_unit.organization_id
                      AND fork_input.source_stage_kind='vuln_triage'
               )
           END AS evidence_operation_id
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
    IF NOT FOUND OR authority.evidence_operation_id IS NULL THEN
        RAISE EXCEPTION 'attack shadow requires exact terminal Unit/handoff/submission/manifest authority'
            USING ERRCODE = '23514';
    END IF;

    -- Close the read-to-freeze race. Once the shadow row becomes visible the
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
                   'observation',item_source.observation,
                   'observation_hash',item_source.observation_hash,
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
                 seed.observation,seed.observation_hash,
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

    -- The accepted decisions may reference the current operation's evidence or
    -- the exact source operation frozen into a Candidate stage fork. No other
    -- operation identity is accepted.
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
              SELECT item.operation_id,item.organization_id,item.id AS work_item_id,
                     item.target_live_id,link.evidence_id
                FROM attack_candidate_work_items AS item
                JOIN attack_candidate_work_item_evidence AS link
                  ON link.work_item_id=item.id AND link.role='decision'
               WHERE item.operation_id=input_operation_id
                 AND item.wave_unit_id=authority.wave_unit_id
                 AND item.organization_id=input_organization_id
              UNION ALL
              SELECT candidate.operation_uuid,candidate.organization_id,
                     candidate.source_work_item_id AS work_item_id,
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
            OR evidence.run_id IS DISTINCT FROM authority.evidence_operation_id
            OR NULLIF(evidence.detail->>'organization_id','')::UUID
                   IS DISTINCT FROM source.organization_id
            OR evidence.project_path IS DISTINCT FROM authority.project_path_at_freeze
            OR (
                evidence.target_id IS DISTINCT FROM source.target_live_id
                AND NOT (
                    evidence.target_id IS NULL
                    AND EXISTS (
                        SELECT 1
                          FROM attack_candidate_work_item_evidence AS grounded
                         WHERE grounded.work_item_id=source.work_item_id
                           AND grounded.evidence_id=source.evidence_id
                           AND grounded.role='observation'
                    )
                )
            )
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
