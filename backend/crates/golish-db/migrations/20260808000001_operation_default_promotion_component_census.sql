-- Bind every post-v1 default promotion to the exact implementation surface
-- required by the unified Investigation rollout. Existing promotion receipts
-- remain readable as explicitly grandfathered history; the v2 writer must
-- select and revalidate one sealed seven-component census in its transaction.

CREATE TABLE operation_default_promotion_component_censuses (
    census_id UUID PRIMARY KEY,
    criteria_version TEXT NOT NULL CHECK (
        criteria_version='operation_default_promotion.v2'
    ),
    component_member_count BIGINT NOT NULL CHECK (component_member_count=7),
    component_set_sha256 TEXT NOT NULL CHECK (
        component_set_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    sealed_by_principal_id UUID NOT NULL
        REFERENCES operator_principals(id) ON DELETE RESTRICT,
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(census_id,component_set_sha256)
);

CREATE TABLE operation_default_promotion_component_members (
    census_id UUID NOT NULL
        REFERENCES operation_default_promotion_component_censuses(census_id)
        ON DELETE RESTRICT,
    ordinal SMALLINT NOT NULL CHECK (ordinal BETWEEN 0 AND 6),
    component_kind TEXT NOT NULL CHECK (component_kind IN (
        'profile','graph','read_model','report','pentagi_task_identity',
        'legacy_replay','whole_record_compatibility'
    )),
    component_sha256 TEXT NOT NULL CHECK (
        component_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    member_sha256 TEXT NOT NULL CHECK (
        member_sha256 ~ '^sha256:[0-9a-f]{64}$'
        AND member_sha256=tool_truth_sha256(
            'operation_default_promotion_component_member.v1' || E'\n'
            || component_kind || E'\n' || component_sha256
        )
    ),
    PRIMARY KEY(census_id,ordinal),
    UNIQUE(census_id,component_kind),
    UNIQUE(census_id,member_sha256)
);

CREATE FUNCTION validate_operation_default_promotion_component_census()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_count BIGINT;
    expected_hash TEXT;
    actual_count BIGINT;
    actual_hash TEXT;
BEGIN
    SELECT component_member_count,component_set_sha256
      INTO expected_count,expected_hash
      FROM operation_default_promotion_component_censuses
     WHERE census_id=NEW.census_id;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;
    SELECT COUNT(*),unified_investigation_exact_set_hash(
               'operation_default_promotion_components.v1',
               COALESCE(array_agg(member_sha256 ORDER BY component_kind),ARRAY[]::TEXT[])
           )
      INTO actual_count,actual_hash
      FROM operation_default_promotion_component_members
     WHERE census_id=NEW.census_id;
    IF actual_count<>expected_count THEN
        RAISE EXCEPTION 'OPERATION_PROMOTION_COMPONENT_SET_INCOMPLETE'
            USING ERRCODE='23514';
    END IF;
    IF actual_hash<>expected_hash THEN
        RAISE EXCEPTION 'OPERATION_PROMOTION_COMPONENT_SET_HASH_DRIFT'
            USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER operation_promotion_component_census_exact_set
AFTER INSERT ON operation_default_promotion_component_censuses
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION validate_operation_default_promotion_component_census();

CREATE CONSTRAINT TRIGGER operation_promotion_component_member_exact_set
AFTER INSERT ON operation_default_promotion_component_members
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION validate_operation_default_promotion_component_census();

CREATE TRIGGER operation_promotion_component_censuses_append_only
BEFORE UPDATE OR DELETE ON operation_default_promotion_component_censuses
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

CREATE TRIGGER operation_promotion_component_members_append_only
BEFORE UPDATE OR DELETE ON operation_default_promotion_component_members
FOR EACH ROW EXECUTE FUNCTION unified_investigation_reject_append_only();

ALTER TABLE operation_default_promotion_receipts
    ADD COLUMN component_census_grandfathered BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN component_census_id UUID,
    ADD COLUMN component_census_sha256 TEXT,
    ADD CONSTRAINT operation_default_promotion_component_census_shape
        CHECK (
            (component_census_grandfathered
             AND component_census_id IS NULL
             AND component_census_sha256 IS NULL)
            OR
            (NOT component_census_grandfathered
             AND criteria_version='operation_default_promotion.v2'
             AND component_census_id IS NOT NULL
             AND component_census_sha256 IS NOT NULL)
        ),
    ADD CONSTRAINT operation_default_promotion_component_census_fk
        FOREIGN KEY(component_census_id,component_census_sha256)
        REFERENCES operation_default_promotion_component_censuses(
            census_id,component_set_sha256
        ) ON DELETE RESTRICT;

-- Backfilled history keeps TRUE. Any later writer that omits the v2 binding
-- fails the shape constraint instead of silently creating grandfathered truth.
ALTER TABLE operation_default_promotion_receipts
    ALTER COLUMN component_census_grandfathered SET DEFAULT FALSE;
