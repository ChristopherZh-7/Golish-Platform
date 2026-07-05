# Crawl Observations Origin Ownership

## Problem

Enumeration crawlers such as `katana -list ... -jc -silent` can observe URLs from third-party
documentation, libraries, analytics, social embeds, and malformed JavaScript string fragments.
Those observations are useful evidence about a page, but they are not automatically in-scope
assets. Promoting their host into `targets(scope='in')` pollutes the target tree, expands the
enumeration worklist, and makes "unresolved domains" look like owned assets.

`api_endpoints` is also not the right home for third-party observations: it drives
`GOLISH-ENUM-JSAPI` / `GOLISH-ENUM-PARAM` DB truth. Storing an external URL there would make the
gate believe a web root has real API/param coverage.

## Decision

Add an origin-owned crawl observation model:

- same-origin/current-org URLs continue to land in the existing surface tables (`api_endpoints`,
  `directory_entries`, `js_analysis_results`) where they can drive coverage truth;
- external or unpromoted URLs land in a new `crawl_observations` table linked to the source target
  and normalized web origin;
- observations never create `targets` and never count as enumeration coverage;
- a future ownership decision can explicitly promote an observation into a target, but promotion is
  not automatic.

## Schema

`crawl_observations` is additive and idempotent:

- `id uuid primary key`
- `origin_target_id uuid not null references targets(id) on delete cascade`
- `organization_id uuid null references organizations(id) on delete set null`
- `project_path text not null default ''`
- `origin_url text not null`
- `origin_key text not null`
- `observed_url text not null`
- `observed_host text`
- `observed_path text`
- `kind text not null default 'link'`
- `same_origin boolean not null default false`
- `source_tool text not null default 'crawler'`
- `source_record_id text`
- `evidence_id bigint references audit_log(id) on delete set null`
- `metadata jsonb not null default '{}'`
- `discovered_at timestamptz not null default now()`
- `updated_at timestamptz not null default now()`
- unique `(origin_target_id, observed_url, source_tool, kind)`

The table is an observation ledger, not a coverage source.

## Backend Flow

`output_store::endpoint_add` parses command origins from `-u/--url` and `-list` roots files.
For each crawler URL:

1. if the URL exactly matches a command origin and the source target already exists, land it as an
   `api_endpoints(source='crawler')` row for that target;
2. otherwise, if it came from a known command origin, write a `crawl_observations` row under the
   origin target;
3. if no origin can be recovered, only attach to an existing current-org target; do not create one.

This keeps coverage truth clean while preserving third-party crawl evidence.

## UI

Target Surface Web Origin detail gets a new tab, `Crawl`, showing origin-owned observations grouped
by observed host. It is a readonly view: URL, kind, source tool, same-origin flag, and discovery
time. External observations are clearly presented as page context, not in-scope assets.

## Non-Goals

- no target promotion UI in this slice;
- no bulk cleanup of already polluted Test1 rows;
- no schema rewrite of existing `api_endpoints`;
- no automatic gate credit from crawl observations.
