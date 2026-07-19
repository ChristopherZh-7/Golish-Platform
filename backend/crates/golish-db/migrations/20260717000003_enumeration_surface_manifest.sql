-- Enumeration surface manifest: operation-bound endpoint/parameter inventory and
-- exact-origin fingerprint provenance for deterministic Vuln applicability.

CREATE TABLE fingerprint_origin_observations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    fingerprint_id UUID NOT NULL REFERENCES fingerprints(id) ON DELETE CASCADE,
    web_origin_id UUID NOT NULL REFERENCES web_origins(id) ON DELETE CASCADE,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    target_id UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    project_path TEXT NOT NULL,
    source TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT fingerprint_origin_source_not_empty CHECK (btrim(source) <> ''),
    UNIQUE (fingerprint_id, web_origin_id)
);

CREATE INDEX fingerprint_origin_observations_origin_idx
    ON fingerprint_origin_observations(web_origin_id, target_id);
CREATE INDEX fingerprint_origin_observations_owner_idx
    ON fingerprint_origin_observations(organization_id, project_path);

CREATE TABLE enumeration_endpoint_observations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    target_id UUID NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    web_origin_id UUID NOT NULL REFERENCES web_origins(id) ON DELETE CASCADE,
    endpoint_id UUID NOT NULL REFERENCES api_endpoints(id) ON DELETE CASCADE,
    project_path TEXT NOT NULL,
    source TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT enumeration_endpoint_source_not_empty CHECK (btrim(source) <> ''),
    UNIQUE (operation_id, web_origin_id, endpoint_id)
);

CREATE INDEX enumeration_endpoint_observations_operation_origin_idx
    ON enumeration_endpoint_observations(operation_id, organization_id, web_origin_id);
CREATE INDEX enumeration_endpoint_observations_target_idx
    ON enumeration_endpoint_observations(target_id, endpoint_id);

CREATE TABLE enumeration_endpoint_parameters (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    endpoint_observation_id UUID NOT NULL
        REFERENCES enumeration_endpoint_observations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    location TEXT NOT NULL,
    value_type TEXT NOT NULL DEFAULT 'unknown',
    required BOOLEAN NOT NULL DEFAULT FALSE,
    source TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT enumeration_endpoint_parameter_name_not_empty CHECK (btrim(name) <> ''),
    CONSTRAINT enumeration_endpoint_parameter_source_not_empty CHECK (btrim(source) <> ''),
    CONSTRAINT enumeration_endpoint_parameter_location_check
        CHECK (location IN ('query', 'body_or_form', 'path', 'header', 'unknown')),
    UNIQUE (endpoint_observation_id, location, name)
);

CREATE INDEX enumeration_endpoint_parameters_observation_location_idx
    ON enumeration_endpoint_parameters(endpoint_observation_id, location);

-- Fail closed if callers bypass the guarded repository and attempt to bind rows
-- with different owners, targets, origins or operation scope.
CREATE FUNCTION validate_fingerprint_origin_observation()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM fingerprints f
        JOIN targets t ON t.id = f.target_id
        JOIN web_origins wo ON wo.id = NEW.web_origin_id
        JOIN web_origin_observations woo
          ON woo.web_origin_id = wo.id AND woo.target_id = t.id
        WHERE f.id = NEW.fingerprint_id
          AND f.target_id = NEW.target_id
          AND t.id = NEW.target_id
          AND t.scope::text = 'in'
          AND t.organization_id = NEW.organization_id
          AND t.project_path IS NOT DISTINCT FROM NEW.project_path
          AND f.project_path IS NOT DISTINCT FROM NEW.project_path
          AND wo.organization_id = NEW.organization_id
          AND wo.project_path = NEW.project_path
    ) THEN
        RAISE EXCEPTION 'fingerprint origin observation owner/origin mismatch'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER fingerprint_origin_observations_validate
BEFORE INSERT OR UPDATE ON fingerprint_origin_observations
FOR EACH ROW EXECUTE FUNCTION validate_fingerprint_origin_observation();

CREATE FUNCTION validate_enumeration_endpoint_observation()
RETURNS trigger AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM targets t
        JOIN web_origins wo ON wo.id = NEW.web_origin_id
        JOIN web_origin_observations woo
          ON woo.web_origin_id = wo.id AND woo.target_id = t.id
        JOIN api_endpoints ae ON ae.id = NEW.endpoint_id AND ae.target_id = t.id
        JOIN operation_org_scope_snapshots oss ON oss.operation_id = NEW.operation_id
        JOIN operation_org_scope_units osu
          ON osu.snapshot_id = oss.id AND osu.organization_id = NEW.organization_id
        WHERE t.id = NEW.target_id
          AND t.scope::text = 'in'
          AND t.organization_id = NEW.organization_id
          AND t.project_path IS NOT DISTINCT FROM NEW.project_path
          AND ae.project_path IS NOT DISTINCT FROM NEW.project_path
          AND wo.organization_id = NEW.organization_id
          AND wo.project_path = NEW.project_path
          AND oss.sealed_at IS NOT NULL
          AND oss.project_path_at_freeze = NEW.project_path
          AND (
              ae.url = wo.origin
              OR ae.url LIKE wo.origin || '/%'
              OR ae.url LIKE wo.origin || '?%'
          )
    ) THEN
        RAISE EXCEPTION 'enumeration endpoint observation owner/operation/origin mismatch'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER enumeration_endpoint_observations_validate
BEFORE INSERT OR UPDATE ON enumeration_endpoint_observations
FOR EACH ROW EXECUTE FUNCTION validate_enumeration_endpoint_observation();
