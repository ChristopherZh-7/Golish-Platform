-- A deterministic in-scope static route template is promotion eligible even
-- when the browser did not observe a concrete runtime request.  Preserve the
-- abstract template in the legacy compatibility projection in that case; if
-- any runtime sample exists, the projection must continue to bind that sample.
CREATE OR REPLACE FUNCTION enumeration_validate_group_api_link()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM enumeration_endpoint_groups endpoint_group
        JOIN operation_state operation ON operation.operation_id=endpoint_group.operation_id
        JOIN api_endpoints endpoint ON endpoint.id=NEW.endpoint_id
        JOIN enumeration_endpoint_observations observation
          ON observation.id=NEW.endpoint_observation_id
         AND observation.endpoint_id=endpoint.id
         AND observation.operation_id=endpoint_group.operation_id
         AND observation.organization_id=endpoint_group.organization_id
         AND observation.target_id=endpoint_group.resolved_target_id
         AND observation.web_origin_id=endpoint_group.resolved_web_origin_id
        WHERE endpoint_group.id=NEW.group_id AND endpoint_group.operation_id=NEW.operation_id
          AND operation.enumeration_analysis_contract='agent_team_v2'
          AND operation.tool_truth_contract='receipt_v1'
          AND endpoint_group.protocol IN ('http','https')
          AND endpoint.target_id=endpoint_group.resolved_target_id
          AND endpoint.method=endpoint_group.method
          AND endpoint.source='occurrence_v2_aggregate'
          AND observation.source='occurrence_v2_aggregate'
          AND (
              (endpoint_group.route_kind='exact' AND endpoint.url=endpoint_group.route_template)
              OR (
                  endpoint_group.route_kind='template'
                  AND (
                      EXISTS (
                          SELECT 1 FROM enumeration_endpoint_occurrence_group_links link
                          JOIN enumeration_endpoint_occurrences occurrence
                            ON occurrence.id=link.occurrence_id
                          WHERE link.group_id=endpoint_group.id
                            AND occurrence.runtime_sample_url=endpoint.url
                      )
                      OR (
                          endpoint.url=endpoint_group.route_template
                          AND NOT EXISTS (
                              SELECT 1 FROM enumeration_endpoint_occurrence_group_links link
                              JOIN enumeration_endpoint_occurrences occurrence
                                ON occurrence.id=link.occurrence_id
                              WHERE link.group_id=endpoint_group.id
                                AND occurrence.runtime_sample_url IS NOT NULL
                          )
                      )
                  )
              )
          )
        FOR SHARE OF endpoint_group,operation,endpoint,observation
    ) THEN
        RAISE EXCEPTION 'enumeration_endpoint_group_api_projection_invalid' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;
