-- Fresh PostgreSQL installs reject the original durable-begin trigger body
-- because its PL/pgSQL variable `action_contract_kind` collides with the
-- unqualified prepared-action column of the same name.  Replace only the
-- function body; the existing trigger and all durable rows remain intact.
CREATE OR REPLACE FUNCTION verification_guard_durable_action_begin()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    hold verification_campaign_safety_holds%ROWTYPE;
    selected_action_state TEXT;
    selected_action_contract_kind TEXT;
    selected_action_target_live_id UUID;
    selected_authorization_decision TEXT;
    selected_authorization_expires_at TIMESTAMPTZ;
    selected_reservation_state TEXT;
    missing_key_count BIGINT;
BEGIN
    SELECT safety_hold.* INTO STRICT hold
      FROM verification_campaign_safety_holds safety_hold
     WHERE safety_hold.singleton=TRUE FOR SHARE;
    SELECT prepared.state,prepared.action_contract_kind,prepared.target_live_id
      INTO selected_action_state,selected_action_contract_kind,
           selected_action_target_live_id
      FROM verification_prepared_actions prepared
     WHERE prepared.prepared_action_id=NEW.prepared_action_id FOR UPDATE;
    SELECT auth.decision,auth.expires_at
      INTO selected_authorization_decision,selected_authorization_expires_at
      FROM verification_prepared_action_authorizations auth
     WHERE auth.authorization_receipt_id=NEW.authorization_receipt_id FOR SHARE;
    SELECT reservation.state INTO selected_reservation_state
      FROM verification_budget_reservations reservation
     WHERE reservation.budget_reservation_id=NEW.budget_reservation_id FOR SHARE;
    SELECT COUNT(*) INTO missing_key_count
      FROM verification_action_conflict_set_members member
      JOIN verification_action_conflict_sets conflict_set
        ON conflict_set.conflict_set_id=member.conflict_set_id
      LEFT JOIN verification_conflict_key_heads head
        ON head.operation_id=NEW.operation_id AND head.organization_id=NEW.organization_id
       AND head.key_kind=member.key_kind AND head.key_identity_hash=member.key_identity_hash
       AND head.state='active' AND head.owner_prepared_action_id=NEW.prepared_action_id
     WHERE conflict_set.conflict_set_id=NEW.conflict_set_id AND head.operation_id IS NULL;
    IF hold.campaign_dispatch_held
       OR NEW.campaign_dispatch_generation<>hold.campaign_dispatch_generation
       OR selected_action_state<>'authorized'
       OR selected_authorization_decision<>'authorized'
       OR selected_action_contract_kind<>NEW.execution_kind
       OR selected_action_target_live_id IS NULL
       OR selected_authorization_expires_at IS NULL
       OR selected_authorization_expires_at<=statement_timestamp()
       OR selected_reservation_state<>'active' OR missing_key_count<>0
    THEN
        RAISE EXCEPTION 'VERIFICATION_ACTION_DURABLE_BEGIN_AUTHORITY_STALE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
