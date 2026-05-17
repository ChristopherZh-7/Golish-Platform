-- Drop the legacy methodology_projects table.
--
-- The pentest workflow is moving from a static "OWASP WSTG / PTES
-- checklist" model to a fully executable pipeline DAG (see
-- `golish-pipeline` crate). Static methodology checklists no longer
-- carry their weight and the frontend MethodologyPanel + Tauri
-- `method_*` commands have been removed in the same change.
--
-- Safety net: data is copied to `_backup_methodology_projects` before
-- the table is dropped, so any caller who later regrets the deletion
-- can `SELECT * FROM _backup_methodology_projects` to recover their
-- records. The backup table has no constraints, no indexes, and is
-- not wired into any code path; future migrations may drop it
-- unconditionally once the dust settles.

CREATE TABLE IF NOT EXISTS _backup_methodology_projects AS
TABLE methodology_projects;

DROP TABLE IF EXISTS methodology_projects;
