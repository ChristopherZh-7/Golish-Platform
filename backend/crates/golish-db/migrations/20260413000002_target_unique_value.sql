-- Deduplicate targets: keep the most recently updated row for each (value, project_path).
DELETE FROM targets a USING targets b
WHERE a.id < b.id
  AND a.value = b.value
  AND a.project_path IS NOT DISTINCT FROM b.project_path;
