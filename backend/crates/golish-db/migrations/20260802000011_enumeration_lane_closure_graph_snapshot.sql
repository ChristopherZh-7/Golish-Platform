-- Lane receipts are immutable snapshots. Resolution is nevertheless allowed to
-- append the typed closeout for an unresolved producer candidate, and Parameter
-- is allowed to materialize the legacy endpoint projection after JS/API closes.
-- The original graph materializer read those future rows when recomputing an
-- earlier seal, so a valid B -> J -> P -> R -> C run made J/P appear corrupt.
--
-- Preserve legacy receipt semantics before this migration's cutover and apply
-- receipt-time snapshot semantics to every receipt created afterwards.
CREATE TABLE enumeration_lane_closure_graph_snapshot_cutovers (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    activated_at TIMESTAMPTZ NOT NULL
);

INSERT INTO enumeration_lane_closure_graph_snapshot_cutovers(singleton,activated_at)
VALUES(TRUE,statement_timestamp());

DO $migration$
DECLARE original_definition TEXT;
DECLARE repaired_definition TEXT;
BEGIN
    SELECT pg_get_functiondef(
        'enumeration_lane_closure_graph_material(uuid)'::REGPROCEDURE
    ) INTO original_definition;

    IF POSITION('DECLARE parameter_authority_id UUID;' IN original_definition)=0
       OR POSITION(
           'WHERE closure.execution_authority_id=ANY(producer_authority_ids)'
           IN original_definition
       )=0
       OR POSITION(
           'FROM enumeration_endpoint_groups endpoint_group'
           IN original_definition
       )=0
       OR POSITION(
           'FROM enumeration_endpoint_occurrence_group_links link'
           IN original_definition
       )=0
       OR POSITION(
           'FROM enumeration_endpoint_group_api_links link'
           IN original_definition
       )=0
       OR POSITION('FROM api_endpoints endpoint' IN original_definition)=0
       OR POSITION(
           'FROM enumeration_endpoint_observations observation'
           IN original_definition
       )=0
       OR POSITION(
           'FROM enumeration_endpoint_parameters parameter'
           IN original_definition
       )=0 THEN
        RAISE EXCEPTION 'enumeration_closure_graph_snapshot_repair_source_drift'
            USING ERRCODE='23514';
    END IF;

    repaired_definition := REPLACE(
        original_definition,
        'DECLARE parameter_authority_id UUID;',
        E'DECLARE parameter_authority_id UUID;\nDECLARE snapshot_cutover_at TIMESTAMPTZ;'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'BEGIN\n    SELECT * INTO receipt',
        E'BEGIN\n    SELECT activated_at INTO snapshot_cutover_at\n      FROM enumeration_lane_closure_graph_snapshot_cutovers\n     WHERE singleton;\n    SELECT * INTO receipt'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        'WHERE closure.execution_authority_id=ANY(producer_authority_ids)',
        E'WHERE closure.execution_authority_id=ANY(producer_authority_ids)\n               AND (receipt.created_at<snapshot_cutover_at\n                    OR closure.created_at<=receipt.created_at)'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'FROM enumeration_endpoint_groups endpoint_group\n             WHERE EXISTS (',
        E'FROM enumeration_endpoint_groups endpoint_group\n             WHERE (receipt.created_at<snapshot_cutover_at\n                    OR endpoint_group.created_at<=receipt.created_at)\n               AND EXISTS ('
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'FROM enumeration_endpoint_occurrence_group_links link\n             WHERE link.occurrence_id=ANY(producer_occurrence_ids)',
        E'FROM enumeration_endpoint_occurrence_group_links link\n             WHERE (receipt.created_at<snapshot_cutover_at\n                    OR link.created_at<=receipt.created_at)\n               AND link.occurrence_id=ANY(producer_occurrence_ids)'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'FROM enumeration_endpoint_group_api_links link\n             WHERE EXISTS (',
        E'FROM enumeration_endpoint_group_api_links link\n             WHERE (receipt.created_at<snapshot_cutover_at\n                    OR link.created_at<=receipt.created_at)\n               AND EXISTS ('
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'FROM api_endpoints endpoint\n             WHERE EXISTS (',
        E'FROM api_endpoints endpoint\n             WHERE (receipt.created_at<snapshot_cutover_at\n                    OR endpoint.discovered_at<=receipt.created_at)\n               AND EXISTS ('
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'FROM enumeration_endpoint_observations observation\n             WHERE EXISTS (',
        E'FROM enumeration_endpoint_observations observation\n             WHERE (receipt.created_at<snapshot_cutover_at\n                    OR observation.created_at<=receipt.created_at)\n               AND EXISTS ('
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'FROM enumeration_endpoint_parameters parameter\n             WHERE EXISTS (',
        E'FROM enumeration_endpoint_parameters parameter\n             WHERE (receipt.created_at<snapshot_cutover_at\n                    OR parameter.created_at<=receipt.created_at)\n               AND EXISTS ('
    );
    repaired_definition := REPLACE(
        repaired_definition,
        E'WHERE link.group_id=endpoint_group.id\n                    AND link.occurrence_id=ANY(producer_occurrence_ids)',
        E'WHERE link.group_id=endpoint_group.id\n                    AND (receipt.created_at<snapshot_cutover_at\n                         OR link.created_at<=receipt.created_at)\n                    AND link.occurrence_id=ANY(producer_occurrence_ids)'
    );
    repaired_definition := REPLACE(
        repaired_definition,
        'AND occurrence_link.occurrence_id=ANY(producer_occurrence_ids)',
        E'AND (receipt.created_at<snapshot_cutover_at\n                         OR link.created_at<=receipt.created_at)\n                    AND (receipt.created_at<snapshot_cutover_at\n                         OR occurrence_link.created_at<=receipt.created_at)\n                    AND occurrence_link.occurrence_id=ANY(producer_occurrence_ids)'
    );

    EXECUTE repaired_definition;
END;
$migration$;

-- JSON rendering of TIMESTAMPTZ follows the caller session's timezone.  The
-- material contains observation/capture timestamps, so pin the function to a
-- canonical timezone rather than letting the same graph hash differently on
-- pooled connections with different session settings.
ALTER FUNCTION enumeration_lane_closure_graph_material(UUID)
    SET timezone TO 'UTC';
