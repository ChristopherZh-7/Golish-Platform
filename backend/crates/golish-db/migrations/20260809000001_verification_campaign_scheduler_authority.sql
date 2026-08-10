-- Restore the closed server-policy lane for T0/T1 Prepared Actions and make
-- review/authorization expiry a durable terminal decision.  Human JIT remains
-- exclusive to T2/T3 and the campaign-dispatch hold still fences every
-- authorized action before any external I/O.

ALTER TABLE verification_prepared_action_authorizations
    ALTER COLUMN decided_by DROP NOT NULL;

DO $$
DECLARE
    actor_kind_constraint TEXT;
    operator_channel_constraint TEXT;
BEGIN
    SELECT conname INTO STRICT actor_kind_constraint
      FROM pg_constraint
     WHERE conrelid='verification_prepared_action_authorizations'::REGCLASS
       AND contype='c'
       AND pg_get_constraintdef(oid) LIKE '%actor_kind%local_operator%';
    SELECT conname INTO STRICT operator_channel_constraint
      FROM pg_constraint
     WHERE conrelid='verification_prepared_action_authorizations'::REGCLASS
       AND contype='c'
       AND pg_get_constraintdef(oid) LIKE '%operator_channel%local_ui%local_cli%local_admin%';
    EXECUTE format(
        'ALTER TABLE verification_prepared_action_authorizations DROP CONSTRAINT %I',
        actor_kind_constraint
    );
    EXECUTE format(
        'ALTER TABLE verification_prepared_action_authorizations DROP CONSTRAINT %I',
        operator_channel_constraint
    );
END;
$$;

ALTER TABLE verification_prepared_action_authorizations
    ADD CONSTRAINT verification_prepared_action_authorizations_actor_kind_check
        CHECK (actor_kind IN ('local_operator','server_policy')),
    ADD CONSTRAINT verification_prepared_action_authorizations_operator_channel_check
        CHECK (operator_channel IN ('local_ui','local_cli','local_admin','server_policy')),
    ADD CONSTRAINT verification_prepared_action_authorizations_actor_shape_check
        CHECK (
            (actor_kind='local_operator' AND decided_by IS NOT NULL
                AND operator_channel IN ('local_ui','local_cli','local_admin'))
            OR
            (actor_kind='server_policy' AND decided_by IS NULL
                AND operator_channel='server_policy')
        );

CREATE OR REPLACE FUNCTION verification_guard_action_authorization()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    hold verification_campaign_safety_holds%ROWTYPE;
    action verification_prepared_actions%ROWTYPE;
    denominator_sealed TIMESTAMPTZ;
    conflict_sealed TIMESTAMPTZ;
    capability_status TEXT;
    current_authorization_expiry TIMESTAMPTZ;
BEGIN
    SELECT * INTO STRICT action FROM verification_prepared_actions
     WHERE prepared_action_id=NEW.prepared_action_id FOR UPDATE;

    IF action.row_version<>NEW.expected_action_row_version
       OR action.display_projection_hash<>NEW.expected_display_projection_hash
       OR NEW.reviewed_action_hash<>action.display_projection_hash
       OR action.private_manifest_hash<>NEW.expected_private_manifest_hash
       OR action.renderer_version<>NEW.renderer_version
    THEN
        RAISE EXCEPTION 'VERIFICATION_ACTION_REVIEW_AUTHORITY_STALE' USING ERRCODE='23514';
    END IF;

    IF NEW.actor_kind='local_operator' THEN
        IF action.state<>'pending_authorization'
           OR action.risk_tier NOT IN ('T2','T3')
           OR action.review_expires_at<=statement_timestamp()
           OR NEW.decision NOT IN ('authorized','denied')
           OR (NEW.decision<>'authorized' AND NEW.expires_at IS NOT NULL)
           OR NOT EXISTS(
               SELECT 1 FROM operator_principals principal
                WHERE principal.id=NEW.decided_by
                  AND principal.principal_kind='local_operator' AND principal.active
           )
        THEN
            RAISE EXCEPTION 'VERIFICATION_ACTION_REVIEW_AUTHORITY_STALE' USING ERRCODE='23514';
        END IF;
    ELSIF NEW.actor_kind='server_policy' THEN
        IF NEW.decided_by IS NOT NULL OR NEW.operator_channel<>'server_policy' THEN
            RAISE EXCEPTION 'VERIFICATION_ACTION_POLICY_AUTHORITY_INVALID' USING ERRCODE='23514';
        END IF;
        IF NEW.decision='authorized' THEN
            IF action.state<>'pending_authorization'
               OR action.risk_tier NOT IN ('T0','T1')
               OR action.review_expires_at<=statement_timestamp()
               OR NEW.decision_reason_code<>'server_policy_auto_authorized_t0_t1'
            THEN
                RAISE EXCEPTION 'VERIFICATION_ACTION_POLICY_AUTHORITY_INVALID' USING ERRCODE='23514';
            END IF;
        ELSIF NEW.decision='expired' THEN
            IF NEW.expires_at IS NOT NULL OR NEW.residual_id IS NULL THEN
                RAISE EXCEPTION 'VERIFICATION_ACTION_POLICY_AUTHORITY_INVALID' USING ERRCODE='23514';
            END IF;
            IF action.state='pending_authorization' THEN
                IF action.review_expires_at>statement_timestamp()
                   OR NEW.decision_reason_code<>'server_policy_review_expired'
                THEN
                    RAISE EXCEPTION 'VERIFICATION_ACTION_POLICY_AUTHORITY_INVALID' USING ERRCODE='23514';
                END IF;
            ELSIF action.state='authorized' THEN
                SELECT latest_auth.expires_at INTO current_authorization_expiry
                  FROM verification_prepared_action_authorizations latest_auth
                 WHERE latest_auth.prepared_action_id=action.prepared_action_id
                   AND latest_auth.decision='authorized'
                 ORDER BY latest_auth.decided_at DESC,
                          latest_auth.authorization_receipt_id DESC
                 LIMIT 1 FOR SHARE;
                IF current_authorization_expiry IS NULL
                   OR current_authorization_expiry>statement_timestamp()
                   OR NEW.decision_reason_code<>'server_policy_authorization_expired'
                THEN
                    RAISE EXCEPTION 'VERIFICATION_ACTION_POLICY_AUTHORITY_INVALID' USING ERRCODE='23514';
                END IF;
            ELSE
                RAISE EXCEPTION 'VERIFICATION_ACTION_POLICY_AUTHORITY_INVALID' USING ERRCODE='23514';
            END IF;
        ELSE
            RAISE EXCEPTION 'VERIFICATION_ACTION_POLICY_AUTHORITY_INVALID' USING ERRCODE='23514';
        END IF;
    ELSE
        RAISE EXCEPTION 'VERIFICATION_ACTION_REVIEW_AUTHORITY_STALE' USING ERRCODE='23514';
    END IF;

    IF NEW.decision<>'authorized' THEN
        RETURN NEW;
    END IF;

    SELECT * INTO STRICT hold FROM verification_campaign_safety_holds
     WHERE singleton=TRUE FOR SHARE;
    SELECT denominator.sealed_at INTO denominator_sealed
      FROM verification_campaign_coverage_denominators denominator
     WHERE denominator.campaign_id=action.campaign_id FOR SHARE;
    SELECT conflict_set.sealed_at INTO conflict_sealed
      FROM verification_action_conflict_sets conflict_set
     WHERE conflict_set.prepared_action_id=action.prepared_action_id FOR SHARE;
    SELECT status INTO capability_status FROM verification_capability_assessments
     WHERE assessment_id=action.capability_assessment_id FOR SHARE;
    IF hold.campaign_dispatch_held
       OR NEW.campaign_dispatch_generation<>hold.campaign_dispatch_generation
       OR action.target_live_id IS NULL OR denominator_sealed IS NULL OR conflict_sealed IS NULL
       OR capability_status<>'available'
       OR NEW.expires_at IS NULL OR NEW.expires_at<=statement_timestamp()
       OR NEW.expires_at>action.review_expires_at
    THEN
        RAISE EXCEPTION 'VERIFICATION_ACTION_AUTHORIZATION_AUTHORITY_STALE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
