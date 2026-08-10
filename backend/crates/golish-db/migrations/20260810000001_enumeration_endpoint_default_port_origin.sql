-- Keep the operation-owned Enumeration endpoint manifest aligned with the
-- canonical WebOrigin identity. URL serializers normally omit the default
-- :80/:443 port while web_origins deliberately stores it explicitly; the
-- original trigger compared those two representations as raw strings and
-- rejected valid same-origin endpoint observations.

CREATE OR REPLACE FUNCTION validate_enumeration_endpoint_observation()
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
          AND enumeration_url_matches_web_origin(
              split_part(split_part(ae.url, '#', 1), '?', 1),
              NEW.web_origin_id
          )
    ) THEN
        RAISE EXCEPTION 'enumeration endpoint observation owner/operation/origin mismatch'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
