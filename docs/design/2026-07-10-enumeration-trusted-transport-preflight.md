# Enumeration trusted transport preflight

> Supersedes `2026-07-10-enumeration-planned-terminal-exceptions.md`.

## Decision

Enumeration must not let a model convert a pending exact-origin cell into
`blocked` or `not_applicable`. The platform owns terminal truth:

- content producers own current-run `found` / `empty`;
- `enum_preflight_web_origins` owns transport `blocked`;
- deterministic gate context alone owns `not_applicable`;
- `error` / `partial` remain unfinished;
- the Enumeration deliverable always submits `coverage: []`.

## Trusted preflight contract

The direct tool accepts 1..=50 `{target_id,target_url}` exact-origin roots from
the current worklist. Before any network request it verifies the active session,
Enumeration operation, organization, workspace, in-scope target, and exact
origin. It captures the operation epoch (`operation_id` + stage +
`stage_started_at` + supersede/engagement binding), creates an opaque per-origin
attempt generation, then locks and revalidates that epoch and the target in the
same short transaction that refreshes all four Enumeration outcomes to
`partial`. This invalidates a stale blocked row when an origin later recovers,
without allowing an old request to write markers after `restart_stage`.

For each root it uses fixed finite timeouts and concurrency, disables redirects,
tries direct transport first and the configured or environment proxy second when
available, sends HEAD, and sends GET with `Range: bytes=0-0` only if HEAD fails at
the transport/TLS layer. Explicit platform proxy configuration applies to both
HTTP and HTTPS; environment fallback selects `HTTP_PROXY` for HTTP,
`HTTPS_PROXY` for HTTPS, then `ALL_PROXY`, and applies `NO_PROXY`. Certificate
validation is permissive, matching the browser and route producers: self-signed,
expired, or hostname-mismatched certificates do not make a producer-reachable
origin blocked. Any HTTP response, including 3xx/4xx/5xx, proves the origin is
reachable and leaves the four partial markers for normal producers.

Only when every available bounded attempt fails before an HTTP response may the
tool prepare four target-bound evidence rows and atomically publish
JS/DIR/PARAM/JSAPI as `blocked`. Terminal publication takes the same short
operation/target locks and performs compare-and-set against the attempt
generation stored in all four current `partial` rows. A newer attempt or changed
operation epoch therefore wins; an older blocked result cannot overwrite it.
Missing evidence, owner/epoch drift, invalid proxy setup, generation mismatch, or
publication failure leaves the group pending/partial. No external I/O occurs
inside these DB transactions. Proxy values, credentials, and raw proxy build
errors never enter results or evidence.

The fixed network-timeout budget is intentionally visible rather than appearing
hung: one root can consume at most two strategies (direct + one proxy) x two
requests (HEAD + GET Range) x 5 seconds = about 20 seconds. With 50 roots and
fixed concurrency 8, seven waves make the worst-case network timeout budget
about 140 seconds, plus bounded local DB/scheduling overhead. The tool emits one
stderr batch-start event and one event as each origin completes. These events
contain only `completed/total` and the normalized `reachable` / `blocked` /
`pending` state; they never contain target URLs, proxy URLs, credentials, or raw
errors, and they do not introduce any additional network request.

## Read and gate contract

`EvidenceOutcome::Blocked` is distinct from `Error`. Enumeration read models
project a blocked `technique_outcome` only when a positive evidence id matches a
fresh target-bound audit fact with the same organization, current target owner,
exact origin, technique, and outcome. The audit row must also have
`tool_name=enum_preflight_web_origins` and
`detail.kind=enumeration_transport_blocked`, while the materialized outcome must
have `source=enum_preflight_web_origins`. UI coverage, submit preview, and final
org gate all enforce this producer identity; correct org/target/evidence ids with
the wrong tool, kind, or source remain pending. The final gate consumes the
trusted Blocked fact directly; no deliverable mirror is needed or accepted.

The outcome projection DTO preserves `source` across the DB trait boundary.
Discarding it into a four-column tuple is forbidden because that would make the
final and submit gates unable to distinguish trusted preflight from a forged
blocked row.

## Origin identity and authorization contract

Enumeration denominator expansion, producer authorization/revalidation, and
target-bound evidence ownership use the same pure
`confirmed_target_web_origins(name,value,ports)` helper. It accepts direct
HTTP(S) URL-shaped `name`/`value`, explicit `url` on a confirmed-open port, and
confirmed-open ports whose service metadata explicitly contains HTTP. Service
synthesis uses `target.value` only (display `name` must not invent an alias
origin); SSL/TLS/HTTPS hints select HTTPS and all other explicit HTTP hints select
HTTP. Closed, missing-state, non-HTTP, invalid-port, and CIDR service rows do not
produce origins. This keeps the 303 current no-`ports[].url` HTTP services usable
without widening scope beyond the exact denominator.

An explicit `target_id` is looked up with workspace, scope, and optional current
organization predicates before exact-origin validation. Missing ids and ids that
fail any authorization predicate return the same public denial, so callers
cannot use error text as a UUID existence/ownership oracle; detailed denial
reasons remain internal logs.

Capability metadata is stage-specific: EAS Web fingerprint advertises
`recon/http`, while Enumeration preflight advertises only the exact
`enum_preflight_web_origins` selector. A contract test prevents those two values
from being swapped again.

The legacy `terminal_exceptions` wire property remains nullable for strict
provider compatibility, but only null/omitted/`[]` is accepted. Any non-empty
array fails closed and never changes pagination or readiness.

## Operational flow

For each worklist page: read status/next, deduplicate roots, call the trusted
preflight, remove only `blocked_origins` from producer inputs, run deterministic
`enum_crawl_same_origin_urls` → `browser_collect_js_api(ai_assist=false)` →
`js_extract_apis(ai=false)` → `route_probe_paths`, refresh the worklist, and
submit only when DB-backed readiness is true.
