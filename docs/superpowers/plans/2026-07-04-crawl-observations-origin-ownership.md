# Crawl Observations Origin Ownership Plan

## Goal

Record crawler-discovered third-party or otherwise unpromoted URLs under the web origin that
produced them, without promoting them into `targets` or counting them as `api_endpoints` coverage.

## Tasks

1. Add additive DB schema and repo
   - migration `crawl_observations`
   - repo module with `upsert`, `list_for_origin_targets`, and focused SQL tests

2. Wire crawler output
   - extend `endpoint_add` to derive command origins from `-u` and `-list`
   - when a crawler URL is outside the matched origin, write `crawl_observations`
   - keep same-origin/current-org `api_endpoints` behavior intact

3. Expose Target Surface data
   - add DTOs to `target_surface_hierarchy`
   - attach observations to `WebOriginHierarchyDto` by normalized `origin_key`
   - preserve legacy fallback behavior when the table is empty

4. Add frontend view
   - normalize DTOs in `frontend/lib/api/security-analysis.ts`
   - extend `surfaceHierarchy.ts`
   - add a Web Origin detail `Crawl` tab grouped by host

5. Validate
   - Rust fmt/check/clippy for touched crates
   - focused nextest for `golish-db`, `golish-pentest`, `golish-pentest-app`
   - frontend biome/typecheck and focused TargetPanel tests when applicable
   - record evidence in `agent-progress.md`

## Acceptance

- third-party crawler URLs are visible under the source target/origin;
- those URLs do not create `targets`;
- those URLs do not populate `api_endpoints` unless same-origin/current-org target ownership is
  known;
- TargetPanel shows the observations without requiring the user to inspect logs.
