-- Server-owned local operator identity.
--
-- Request DTOs never carry this UUID. Desktop and CLI command handlers load
-- the single active local row and construct an opaque principal in process.
-- Historical identities are retained so approval/waiver/report audit rows can
-- keep a durable actor foreign key.

CREATE TABLE operator_principals (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    principal_kind TEXT NOT NULL CHECK (principal_kind IN ('local_operator')),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX operator_principals_one_active_kind
    ON operator_principals(principal_kind)
    WHERE active;

CREATE FUNCTION retain_operator_principal_identity()
RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'operator principal history cannot be deleted';
    END IF;
    IF NEW.id IS DISTINCT FROM OLD.id
        OR NEW.principal_kind IS DISTINCT FROM OLD.principal_kind
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'operator principal identity is immutable';
    END IF;
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operator_principals_retain_identity
BEFORE UPDATE OR DELETE ON operator_principals
FOR EACH ROW EXECUTE FUNCTION retain_operator_principal_identity();

INSERT INTO operator_principals(principal_kind, active)
VALUES ('local_operator', TRUE);
