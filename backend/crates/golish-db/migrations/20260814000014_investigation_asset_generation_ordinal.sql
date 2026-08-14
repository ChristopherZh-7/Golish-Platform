-- A canonical generation is scoped to one frozen Investigation asset lane.
-- The asset binding migration moved the compiler's ordinal allocator to that
-- scope, but retained the older organization-wide uniqueness constraint.  A
-- second asset in the same operation therefore attempted its valid ordinal
-- zero generation and collided with the first asset.  Preserve historical
-- rows (including pre-lane NULL rows) while making the catalog match the
-- server-owned allocator.

DO $$
DECLARE
    organization_generation_ordinal_constraint NAME;
BEGIN
    SELECT constraint_row.conname
      INTO organization_generation_ordinal_constraint
      FROM pg_constraint constraint_row
     WHERE constraint_row.conrelid='hypothesis_generations'::REGCLASS
       AND constraint_row.contype='u'
       AND (SELECT array_agg(attribute.attname::TEXT ORDER BY key_column.ordinality)
              FROM unnest(constraint_row.conkey) WITH ORDINALITY
                   key_column(attnum,ordinality)
              JOIN pg_attribute attribute
                ON attribute.attrelid=constraint_row.conrelid
               AND attribute.attnum=key_column.attnum)=
           ARRAY['operation_id','organization_id','generation_ordinal']::TEXT[];
    IF organization_generation_ordinal_constraint IS NOT NULL THEN
        EXECUTE format(
            'ALTER TABLE hypothesis_generations DROP CONSTRAINT %I',
            organization_generation_ordinal_constraint
        );
    END IF;
END;
$$;

ALTER TABLE hypothesis_generations
    ADD CONSTRAINT hypothesis_generations_asset_lane_generation_ordinal_key
    UNIQUE(asset_lane_id,generation_ordinal);
