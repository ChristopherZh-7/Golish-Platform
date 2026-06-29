# Enumeration Tool Boundary Tightening

## Context

During a Ping An `enumeration` run, `browser_collect_js_api` failed and the
Enumerator fell back to `whatweb` service fingerprinting. That is the wrong stage
boundary: `external_attack_surface` owns liveness, ports, and service
fingerprints; `enumeration` consumes the EAS-confirmed live web roots and maps
content units: directories, parameters, and JS/API endpoints.

The same run also showed third-party browser/crawler URLs being promoted into
`targets(source='active_discovered')` under the active organization, for example
analytics and social-login hosts. Those URLs may be observed while rendering a
page, but they are not automatically in scope and must not expand the
enumeration denominator.

## Decision

- `enumeration.allowed_tool_types` excludes `recon/http`. It permits only
  content enumeration selectors: `recon/crawler`, `web/fuzzer`, and `web/param`.
- `whatweb`, `httpx`, `curl`, and `wget` remain EAS tools. They should not be
  visible as enumeration fallback tools.
- `endpoint_add` storage records the command base host when the command includes
  an absolute `-u` / `--url` target. Parsed endpoint URLs whose host differs from
  that base host are skipped instead of creating scoped targets.
- `browser_collect_js_api` continues to persist same-origin XHR/fetch requests
  under the resolved target id; third-party page resources can appear in helper
  context, but they are not scope expansion.

## Expected Behavior

If the browser JS/API helper fails, the Enumerator should repair JS/API with
`js_collect` / `js_extract_apis`, continue to `route_probe_paths` or bounded
directory/parameter enumeration, or mark a truthful terminal state. It should not
run service fingerprinting.

Observed third-party URLs from a crawler do not become new in-scope targets for
the current organization unless a separate authorized scope-expansion path
admits them.
