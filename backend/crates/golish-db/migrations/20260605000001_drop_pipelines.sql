-- Drop the `pipelines` table: the pipeline feature has been fully removed
-- (frontend + backend). See docs/design/2026-06-05-remove-pipeline-feature.md.
-- Forward-only cleanup; this discards any user-saved pipeline definitions.
DROP TABLE IF EXISTS pipelines;
