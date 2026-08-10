-- Publish one already-sealed unified Investigation closure into the generic
-- stage runtime. The closure, not a fabricated Worker deliverable, owns this
-- transition. Publication is atomic and replay-stable: exact Units become
-- passed, per-org completion rows are written, and Reporting receives one
-- typed closure/member authority.

CREATE TABLE investigation_stage_closure_publications (
    publication_id UUID PRIMARY KEY,
    stable_request_id UUID NOT NULL UNIQUE,
    closure_id UUID NOT NULL UNIQUE,
    authority_id UUID NOT NULL UNIQUE,
    operation_id UUID NOT NULL UNIQUE,
    stage_execution_id UUID NOT NULL UNIQUE,
    owning_stage_run_request_id TEXT NOT NULL CHECK (btrim(owning_stage_run_request_id)<>''),
    scope_snapshot_id UUID NOT NULL,
    closure_sha256 TEXT NOT NULL CHECK (closure_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    disposition TEXT NOT NULL CHECK (disposition IN ('pass','pass_with_gaps')),
    member_count BIGINT NOT NULL CHECK (member_count>0),
    member_set_sha256 TEXT NOT NULL CHECK (member_set_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    publication_sha256 TEXT NOT NULL CHECK (publication_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    published_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(publication_id,closure_id,authority_id),
    UNIQUE(publication_id,operation_id,stage_execution_id,scope_snapshot_id),
    FOREIGN KEY(closure_id,authority_id)
        REFERENCES investigation_run_closures(closure_id,authority_id) ON DELETE RESTRICT,
    FOREIGN KEY(authority_id,operation_id,stage_execution_id,
                owning_stage_run_request_id,scope_snapshot_id)
        REFERENCES investigation_run_heads(
            authority_id,operation_id,stage_execution_id,
            owning_stage_run_request_id,scope_snapshot_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(stage_execution_id,operation_id)
        REFERENCES stage_runs(id,operation_id) ON DELETE RESTRICT,
    FOREIGN KEY(scope_snapshot_id,operation_id)
        REFERENCES operation_org_scope_snapshots(id,operation_id) ON DELETE RESTRICT
);

CREATE TABLE investigation_stage_closure_publication_members (
    publication_member_id UUID PRIMARY KEY,
    publication_id UUID NOT NULL
        REFERENCES investigation_stage_closure_publications(publication_id) ON DELETE RESTRICT,
    member_ordinal INTEGER NOT NULL CHECK (member_ordinal>=0),
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL UNIQUE,
    organization_id UUID NOT NULL,
    stage_team_plan_id UUID NOT NULL UNIQUE,
    member_sha256 TEXT NOT NULL CHECK (member_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    passed_at TIMESTAMPTZ NOT NULL,
    UNIQUE(publication_id,member_ordinal),
    UNIQUE(publication_id,organization_id),
    UNIQUE(publication_id,member_sha256),
    FOREIGN KEY(publication_id,operation_id,stage_execution_id,scope_snapshot_id)
        REFERENCES investigation_stage_closure_publications(
            publication_id,operation_id,stage_execution_id,scope_snapshot_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(stage_run_unit_id,operation_id,stage_execution_id,organization_id)
        REFERENCES stage_run_units(
            id,operation_id,stage_execution_id,organization_id
        ) ON DELETE RESTRICT,
    FOREIGN KEY(stage_team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                scope_snapshot_id,organization_id)
        REFERENCES stage_team_plans(
            id,operation_id,stage_execution_id,stage_run_unit_id,
            scope_snapshot_id,organization_id
        ) ON DELETE RESTRICT
);

CREATE TRIGGER investigation_stage_closure_publications_append_only
BEFORE UPDATE OR DELETE ON investigation_stage_closure_publications
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TRIGGER investigation_stage_closure_publication_members_append_only
BEFORE UPDATE OR DELETE ON investigation_stage_closure_publication_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE FUNCTION publish_investigation_stage_closure_v1(
    p_publication_id UUID,
    p_stable_request_id UUID,
    p_closure_id UUID
)
RETURNS investigation_stage_closure_publications
LANGUAGE plpgsql
AS $$
DECLARE
    existing investigation_stage_closure_publications%ROWTYPE;
    closure investigation_run_closure_v1_authorities%ROWTYPE;
    closure_hash TEXT;
    actual_member_count BIGINT;
    actual_member_hash TEXT;
    publication_hash TEXT;
    publication_time TIMESTAMPTZ := statement_timestamp();
    unit_record RECORD;
    member_hash TEXT;
    member_id UUID;
    member_ordinal INTEGER := 0;
    updated_count BIGINT;
    result investigation_stage_closure_publications%ROWTYPE;
BEGIN
    IF p_publication_id IS NULL OR p_stable_request_id IS NULL OR p_closure_id IS NULL THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_IDENTITY_INVALID'
            USING ERRCODE='23514';
    END IF;
    SELECT * INTO existing
      FROM investigation_stage_closure_publications
     WHERE stable_request_id=p_stable_request_id OR closure_id=p_closure_id
     FOR UPDATE;
    IF FOUND THEN
        IF existing.publication_id<>p_publication_id
           OR existing.stable_request_id<>p_stable_request_id
           OR existing.closure_id<>p_closure_id
           OR (SELECT COUNT(*) FROM investigation_stage_closure_publication_members member
                WHERE member.publication_id=existing.publication_id)<>existing.member_count
           OR existing.member_set_sha256 IS DISTINCT FROM (
                SELECT unified_investigation_exact_set_hash(
                    'investigation_stage_closure_publication_members.v1',
                    COALESCE(array_agg(member.member_sha256
                                       ORDER BY member.organization_id,member.stage_run_unit_id),
                             ARRAY[]::TEXT[])
                )
                  FROM investigation_stage_closure_publication_members member
                 WHERE member.publication_id=existing.publication_id
           )
           OR existing.publication_sha256 IS DISTINCT FROM tool_truth_sha256(
                jsonb_build_object(
                    'contract_version','investigation-stage-closure-publication.v1',
                    'publication_id',existing.publication_id,
                    'closure_id',existing.closure_id,
                    'closure_sha256',existing.closure_sha256,
                    'authority_id',existing.authority_id,
                    'operation_id',existing.operation_id,
                    'stage_execution_id',existing.stage_execution_id,
                    'owning_stage_run_request_id',existing.owning_stage_run_request_id,
                    'scope_snapshot_id',existing.scope_snapshot_id,
                    'disposition',existing.disposition,
                    'member_count',existing.member_count,
                    'member_set_sha256',existing.member_set_sha256
                )::TEXT
           )
           OR EXISTS(
                SELECT 1
                  FROM investigation_run_closures header
                  JOIN investigation_run_closure_v1_authorities detail
                    ON detail.closure_id=header.closure_id
                   AND detail.authority_id=header.authority_id
                 WHERE header.closure_id=existing.closure_id
                   AND (header.closure_sha256 IS DISTINCT FROM existing.closure_sha256
                        OR detail.closure_sha256 IS DISTINCT FROM existing.closure_sha256
                        OR detail.disposition IS DISTINCT FROM existing.disposition
                        OR detail.operation_id<>existing.operation_id
                        OR detail.stage_execution_id<>existing.stage_execution_id
                        OR detail.owning_stage_run_request_id<>
                           existing.owning_stage_run_request_id
                        OR detail.scope_snapshot_id<>existing.scope_snapshot_id
                        OR detail.snapshot_member_count<>existing.member_count)
           )
           OR EXISTS(
                SELECT 1
                  FROM investigation_stage_closure_publication_members member
                  JOIN stage_run_units unit ON unit.id=member.stage_run_unit_id
                  JOIN stage_team_plans plan ON plan.id=member.stage_team_plan_id
                 WHERE member.publication_id=existing.publication_id
                   AND (member.member_sha256 IS DISTINCT FROM tool_truth_sha256(
                            jsonb_build_object(
                                'contract_version','investigation-stage-closure-member.v1',
                                'closure_id',existing.closure_id,
                                'closure_sha256',existing.closure_sha256,
                                'stage_run_unit_id',member.stage_run_unit_id,
                                'organization_id',member.organization_id,
                                'stage_team_plan_id',member.stage_team_plan_id
                            )::TEXT)
                        OR unit.status<>'passed'
                        OR unit.terminal_at IS DISTINCT FROM member.passed_at
                        OR plan.requests_closed_at IS NULL
                        OR unit.pass_watermark->>'schema' IS DISTINCT FROM
                           'investigation_stage_closure_publication.v1'
                        OR unit.pass_watermark->>'publication_id' IS DISTINCT FROM
                           existing.publication_id::TEXT
                        OR unit.pass_watermark->>'closure_id' IS DISTINCT FROM
                           existing.closure_id::TEXT
                        OR unit.pass_watermark->>'closure_sha256' IS DISTINCT FROM
                           existing.closure_sha256
                        OR unit.pass_watermark->>'disposition' IS DISTINCT FROM
                           existing.disposition
                        OR unit.pass_watermark->>'member_sha256' IS DISTINCT FROM
                           member.member_sha256)
           )
           OR EXISTS(
                SELECT 1
                  FROM investigation_stage_closure_publication_members member
                  LEFT JOIN org_stage_completions completion
                    ON completion.organization_id=member.organization_id
                   AND completion.stage_kind='investigation'
                 WHERE member.publication_id=existing.publication_id
                   AND (completion.stage_run_id IS DISTINCT FROM existing.operation_id::TEXT
                        OR completion.passed_at IS DISTINCT FROM member.passed_at)
           )
        THEN
            RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_REPLAY_MISMATCH'
                USING ERRCODE='23514';
        END IF;
        RETURN existing;
    END IF;

    SELECT * INTO STRICT closure
      FROM investigation_run_closure_v1_authorities
     WHERE closure_id=p_closure_id;
    SELECT header.closure_sha256 INTO STRICT closure_hash
      FROM investigation_run_closures header
     WHERE header.closure_id=p_closure_id AND header.authority_id=closure.authority_id
     FOR SHARE;
    PERFORM 1 FROM investigation_run_heads head
     WHERE head.authority_id=closure.authority_id
       AND head.operation_id=closure.operation_id
       AND head.stage_execution_id=closure.stage_execution_id
       AND head.scope_snapshot_id=closure.scope_snapshot_id
       AND head.run_state='closed' AND NOT head.admission_open
     FOR SHARE;
    IF NOT FOUND OR closure.disposition NOT IN ('pass','pass_with_gaps') THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_CLOSURE_INVALID'
            USING ERRCODE='23514';
    END IF;
    PERFORM 1 FROM operation_state operation
     WHERE operation.operation_id=closure.operation_id
       AND operation.current_stage='investigation'
       AND operation.superseded_by IS NULL
       AND operation.runtime_memory_contract='v2_only'
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_OPERATION_INVALID'
            USING ERRCODE='23514';
    END IF;
    PERFORM 1 FROM stage_runs execution
     WHERE execution.id=closure.stage_execution_id
       AND execution.operation_id=closure.operation_id
       AND execution.stage_kind='investigation'
       AND execution.status='started'
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_EXECUTION_INVALID'
            USING ERRCODE='23514';
    END IF;
    IF EXISTS(
        SELECT 1
          FROM stage_run_units unit
         WHERE unit.operation_id=closure.operation_id
           AND unit.stage_execution_id=closure.stage_execution_id
           AND unit.scope_snapshot_id=closure.scope_snapshot_id
           AND (unit.stage_kind<>'investigation' OR unit.status<>'running')
    ) OR EXISTS(
        SELECT 1
          FROM operation_org_scope_units scope_member
         WHERE scope_member.snapshot_id=closure.scope_snapshot_id
           AND NOT EXISTS(
                SELECT 1 FROM stage_run_units unit
                 WHERE unit.stage_execution_id=closure.stage_execution_id
                   AND unit.organization_id=scope_member.organization_id
           )
    ) OR EXISTS(
        SELECT 1
          FROM stage_team_plans plan
         WHERE plan.operation_id=closure.operation_id
           AND plan.stage_execution_id=closure.stage_execution_id
           AND (plan.stage_kind<>'investigation' OR plan.requests_closed_at IS NULL)
    ) OR EXISTS(
        SELECT 1 FROM stage_work_items item
         WHERE item.operation_id=closure.operation_id
           AND item.stage_execution_id=closure.stage_execution_id
           AND item.status NOT IN ('completed','exhausted','superseded')
    ) OR EXISTS(
        SELECT 1 FROM stage_worker_runs worker
         WHERE worker.operation_id=closure.operation_id
           AND worker.stage_execution_id=closure.stage_execution_id
           AND worker.status IN ('queued','running','waiting_background','recovery_required')
    ) THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_RUNTIME_NOT_TERMINAL'
            USING ERRCODE='23514';
    END IF;

    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_stage_closure_publication_members.v1',
               COALESCE(array_agg(
                   tool_truth_sha256(jsonb_build_object(
                       'contract_version','investigation-stage-closure-member.v1',
                       'closure_id',p_closure_id,
                       'closure_sha256',closure_hash,
                       'stage_run_unit_id',unit.id,
                       'organization_id',unit.organization_id,
                       'stage_team_plan_id',plan.id
                   )::TEXT) ORDER BY unit.organization_id,unit.id
               ),ARRAY[]::TEXT[])
           )
      INTO actual_member_count,actual_member_hash
      FROM stage_run_units unit
      JOIN stage_team_plans plan ON plan.stage_run_unit_id=unit.id
     WHERE unit.operation_id=closure.operation_id
       AND unit.stage_execution_id=closure.stage_execution_id
       AND unit.scope_snapshot_id=closure.scope_snapshot_id;
    IF actual_member_count=0 OR actual_member_count<>closure.snapshot_member_count THEN
        RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_MEMBER_SET_INCOMPLETE'
            USING ERRCODE='23514';
    END IF;
    publication_hash := tool_truth_sha256(jsonb_build_object(
        'contract_version','investigation-stage-closure-publication.v1',
        'publication_id',p_publication_id,
        'closure_id',p_closure_id,
        'closure_sha256',closure_hash,
        'authority_id',closure.authority_id,
        'operation_id',closure.operation_id,
        'stage_execution_id',closure.stage_execution_id,
        'owning_stage_run_request_id',closure.owning_stage_run_request_id,
        'scope_snapshot_id',closure.scope_snapshot_id,
        'disposition',closure.disposition,
        'member_count',actual_member_count,
        'member_set_sha256',actual_member_hash
    )::TEXT);
    INSERT INTO investigation_stage_closure_publications(
        publication_id,stable_request_id,closure_id,authority_id,operation_id,
        stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
        closure_sha256,disposition,
        member_count,member_set_sha256,publication_sha256,published_at
    ) VALUES(
        p_publication_id,p_stable_request_id,p_closure_id,closure.authority_id,
        closure.operation_id,closure.stage_execution_id,
        closure.owning_stage_run_request_id,closure.scope_snapshot_id,
        closure_hash,closure.disposition,actual_member_count,actual_member_hash,
        publication_hash,publication_time
    );

    FOR unit_record IN
        SELECT unit.id AS unit_id,unit.organization_id,unit.row_version,
               plan.id AS team_plan_id
          FROM stage_run_units unit
          JOIN stage_team_plans plan ON plan.stage_run_unit_id=unit.id
         WHERE unit.operation_id=closure.operation_id
           AND unit.stage_execution_id=closure.stage_execution_id
           AND unit.scope_snapshot_id=closure.scope_snapshot_id
         ORDER BY unit.organization_id,unit.id
         FOR UPDATE OF unit,plan
    LOOP
        member_hash := tool_truth_sha256(jsonb_build_object(
            'contract_version','investigation-stage-closure-member.v1',
            'closure_id',p_closure_id,
            'closure_sha256',closure_hash,
            'stage_run_unit_id',unit_record.unit_id,
            'organization_id',unit_record.organization_id,
            'stage_team_plan_id',unit_record.team_plan_id
        )::TEXT);
        member_id := uuid_generate_v5(
            p_publication_id,
            ('investigation-stage-closure-member:' || unit_record.unit_id::TEXT)::TEXT
        );
        UPDATE stage_run_units
           SET status='passed',
               pass_watermark=jsonb_build_object(
                   'schema','investigation_stage_closure_publication.v1',
                   'publication_id',p_publication_id,
                   'closure_id',p_closure_id,
                   'closure_sha256',closure_hash,
                   'disposition',closure.disposition,
                   'member_sha256',member_hash
               ),
               row_version=row_version+1,terminal_at=publication_time,updated_at=publication_time
         WHERE id=unit_record.unit_id AND status='running'
           AND row_version=unit_record.row_version;
        GET DIAGNOSTICS updated_count = ROW_COUNT;
        IF updated_count<>1 THEN
            RAISE EXCEPTION 'INVESTIGATION_CLOSURE_PUBLICATION_UNIT_CAS_FAILED'
                USING ERRCODE='40001';
        END IF;
        INSERT INTO investigation_stage_closure_publication_members(
            publication_member_id,publication_id,member_ordinal,operation_id,
            stage_execution_id,scope_snapshot_id,stage_run_unit_id,organization_id,
            stage_team_plan_id,member_sha256,passed_at
        ) VALUES(
            member_id,p_publication_id,member_ordinal,closure.operation_id,
            closure.stage_execution_id,closure.scope_snapshot_id,unit_record.unit_id,
            unit_record.organization_id,unit_record.team_plan_id,member_hash,publication_time
        );
        INSERT INTO org_stage_completions(
            organization_id,stage_kind,passed_at,stage_run_id,updated_at
        ) VALUES(
            unit_record.organization_id,'investigation',publication_time,
            closure.operation_id::TEXT,publication_time
        ) ON CONFLICT(organization_id,stage_kind) DO UPDATE
              SET passed_at=EXCLUDED.passed_at,stage_run_id=EXCLUDED.stage_run_id,
                  updated_at=EXCLUDED.updated_at;
        member_ordinal := member_ordinal+1;
    END LOOP;
    SELECT * INTO STRICT result FROM investigation_stage_closure_publications
     WHERE publication_id=p_publication_id;
    RETURN result;
END;
$$;
