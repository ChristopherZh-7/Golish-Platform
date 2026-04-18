-- Backfill fingerprints table from existing targets data (webserver, cdn_waf, os_info, ports→technologies).
-- Uses ON CONFLICT to deduplicate with any fingerprints already stored by whatweb or new httpx code.

-- 1) webserver → fingerprints (category: "webserver")
INSERT INTO fingerprints (target_id, project_path, category, name, version, confidence, evidence, source)
SELECT
    t.id,
    COALESCE(t.project_path, ''),
    'webserver',
    CASE
        WHEN t.webserver LIKE '%/%' THEN split_part(t.webserver, '/', 1)
        ELSE t.webserver
    END,
    CASE
        WHEN t.webserver LIKE '%/%' THEN NULLIF(split_part(t.webserver, '/', 2), '')
        ELSE NULL
    END,
    0.8,
    jsonb_build_object('source', 'backfill', 'raw', t.webserver),
    'httpx'
FROM targets t
WHERE t.webserver != ''
ON CONFLICT (target_id, category, name) DO NOTHING;

-- 2) cdn_waf → fingerprints (category: "cdn")
INSERT INTO fingerprints (target_id, project_path, category, name, confidence, evidence, source)
SELECT
    t.id,
    COALESCE(t.project_path, ''),
    'cdn',
    t.cdn_waf,
    0.9,
    jsonb_build_object('source', 'backfill', 'raw', t.cdn_waf),
    'httpx'
FROM targets t
WHERE t.cdn_waf != ''
ON CONFLICT (target_id, category, name) DO NOTHING;

-- 3) os_info → fingerprints (category: "os")
INSERT INTO fingerprints (target_id, project_path, category, name, version, confidence, evidence, source)
SELECT
    t.id,
    COALESCE(t.project_path, ''),
    'os',
    CASE
        WHEN t.os_info LIKE '%/%' THEN split_part(t.os_info, '/', 1)
        ELSE t.os_info
    END,
    CASE
        WHEN t.os_info LIKE '%/%' THEN NULLIF(split_part(t.os_info, '/', 2), '')
        ELSE NULL
    END,
    0.6,
    jsonb_build_object('source', 'backfill', 'raw', t.os_info),
    'httpx'
FROM targets t
WHERE t.os_info != ''
ON CONFLICT (target_id, category, name) DO NOTHING;

-- 4) ports[].technologies → fingerprints (category: "technology")
INSERT INTO fingerprints (target_id, project_path, category, name, version, confidence, evidence, source)
SELECT
    t.id,
    COALESCE(t.project_path, ''),
    'technology',
    CASE
        WHEN tech_val::text LIKE '%/%' THEN split_part(tech_val::text, '/', 1)
        ELSE trim(both '"' from tech_val::text)
    END,
    CASE
        WHEN tech_val::text LIKE '%/%' THEN NULLIF(split_part(trim(both '"' from tech_val::text), '/', 2), '')
        ELSE NULL
    END,
    0.7,
    jsonb_build_object('source', 'backfill', 'port', port_entry->>'port'),
    'httpx'
FROM targets t,
     jsonb_array_elements(t.ports) AS port_entry,
     jsonb_array_elements(CASE WHEN port_entry->'technologies' IS NOT NULL AND jsonb_typeof(port_entry->'technologies') = 'array'
                               THEN port_entry->'technologies'
                               ELSE '[]'::jsonb END) AS tech_val
WHERE jsonb_array_length(t.ports) > 0
  AND trim(both '"' from tech_val::text) != ''
ON CONFLICT (target_id, category, name) DO NOTHING;
