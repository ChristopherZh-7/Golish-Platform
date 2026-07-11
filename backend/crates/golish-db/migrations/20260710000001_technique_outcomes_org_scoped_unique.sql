-- Multi-organization stage_run sessions share one run_id. Outcome identity must
-- therefore include organization_id or sibling organizations with the same
-- exact origin + technique overwrite each other (I2).
--
-- Forward-only, row-preserving upgrade. Apply with application writers stopped:
-- an older binary still naming either the three-column technique outcome
-- arbiter or the legacy two-column directory arbiter is not compatible after
-- these identities change, even though no row/column is lost.

ALTER TABLE technique_outcomes
    DROP CONSTRAINT IF EXISTS technique_outcomes_run_id_asset_technique_key;

-- Replace even a pre-existing same-name constraint rather than trusting its
-- name to imply the correct column set. The migration transaction holds the
-- table lock across DROP + ADD, so writers never observe an unconstrained
-- committed state. Replaying the SQL remains idempotent and preserves rows.
ALTER TABLE technique_outcomes
    DROP CONSTRAINT IF EXISTS technique_outcomes_organization_run_asset_technique_key;

ALTER TABLE technique_outcomes
    ADD CONSTRAINT technique_outcomes_organization_run_asset_technique_key
    UNIQUE (organization_id, run_id, asset, technique);

-- Directory discoveries are target-owned. The legacy partial index omitted
-- target_id from its key, so the same URL + tool on two targets could make one
-- target overwrite the other's row through ON CONFLICT. Rebuild the index with
-- the owner in the identity; the old key was stricter, so this is a
-- row-preserving expansion and existing data cannot violate the new key.
DROP INDEX IF EXISTS idx_dirent_unique;

CREATE UNIQUE INDEX idx_dirent_unique
    ON directory_entries(target_id, url, tool)
    WHERE target_id IS NOT NULL;

-- Normalize legacy crawler rows before guarded writers switch to strict owner
-- equality. Only previously unbound owner fields are filled, and only when all
-- already-known owner dimensions agree with the current target. Conflicting
-- history is preserved unchanged and will fail closed at write time; never
-- construct a mixed owner from one legacy and one current dimension.
UPDATE crawl_observations AS observation
SET organization_id = COALESCE(observation.organization_id, target.organization_id),
    project_path = CASE
        WHEN observation.project_path = '' THEN COALESCE(target.project_path, '')
        ELSE observation.project_path
    END
FROM targets AS target
WHERE observation.origin_target_id = target.id
  AND (observation.organization_id IS NULL OR observation.project_path = '')
  AND (
      observation.organization_id IS NULL
      OR observation.organization_id = target.organization_id
  )
  AND (
      observation.project_path = ''
      OR observation.project_path IS NOT DISTINCT FROM COALESCE(target.project_path, '')
  );
