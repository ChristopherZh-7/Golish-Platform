# `auth_probe` Tool Contract

Stage 2 of the API security pipeline (after `js_extract_apis`).
Probes a list of API endpoints for **未授权访问 / IDOR / 越权** issues.

> Status: **draft v0.2** (after sync with agent-3 on Endpoint schema +
> summary fields). To be implemented in a follow-up commit; this doc is
> the contract that fixture authors and the impl PR will both reference.

---

## 1. Mission

For each endpoint extracted by `js_extract_apis`, run **3 controlled
requests** and compare responses to detect:

| Scenario       | What it catches                                                |
|----------------|----------------------------------------------------------------|
| `anonymous`    | Endpoint serves data without any auth header (Critical)        |
| `cross_user`   | User A's token reads/writes User B's resource (High, classic IDOR) |
| `privilege`    | Low-privilege token can hit admin endpoint (High)              |

The tool itself **does not run an LLM** — it's a deterministic Rust
probe. The downstream `ai_api_security_review` (Stage 3) is what writes
the human-readable report.

---

## 2. Tool Surface

### Name

```
auth_probe
```

### Input schema (LLM-facing)

```jsonc
{
  "target_id":      "uuid",                  // required, FK into targets
  "endpoints":      [Endpoint],              // required, output of js_extract_apis
  "vault_tokens": {
    "user_a":       { "vault_key": "tester_a_token" },  // looked up via credential_vault.get
    "user_b":       { "vault_key": "tester_b_token" }   // optional — only used for cross_user
  },
  "id_pool": {                               // optional — IDs known to belong to each user
    "user_a":       ["123", "456"],
    "user_b":       ["789", "abc"]
  },
  "scenarios":      ["anonymous", "cross_user", "privilege"], // default: all three
  "rate_limit_ms":  1000,                    // default 1000 — per-request gap
  "concurrency":    1,                       // default 1 — keep deterministic
  "timeout_ms":     10000,                   // default 10000
  "max_endpoints":  500,                     // safety cap, defaults to 500
  "user_agent":     "Mozilla/5.0 ..."        // optional override; defaults to browser-like UA
}
```

`Endpoint` re-uses the schema produced by `js_extract_apis` — see
`backend/crates/golish-js-analyzer/src/lib.rs::Endpoint`. Required fields
read by `auth_probe`:

- `method` — uppercase HTTP verb
- `path` — URL path (literal, concatenated, or template_literal)
- `auth` — `AuthHint` (`bearer | cookie | header | none | unknown`)
- `has_path_params` — bool
- `id_param_position` — `Option<usize>` (0-based index of the ID segment)
- `url_kind` — `literal | concatenated | template_literal`

### Output schema (LLM-facing)

```jsonc
{
  "tested_count":         87,
  "skipped_count":         3,                // endpoints we couldn't safely probe (no path)
  "findings": [
    {
      "endpoint": { ...Endpoint },
      "scenario": "anonymous",
      "severity": "critical",                // critical | high | medium | low | info
      "result":   "vulnerable",              // vulnerable | not_vulnerable | potential | error
      "evidence": {
        "round_1": { "status": 200, "body_len": 1234, "snippet": "{...}" },
        "round_2": { "status": 200, "body_len": 1234, "snippet": "{...}" },
        "round_3": null,                     // only present when scenario triggers it
        "diff_summary": "anon and authed return identical body"
      }
    }
  ],
  "summary": {
    "by_severity":           { "critical": 3, "high": 7, "medium": 12, "low": 5 },
    "by_scenario":           { "anonymous": 15, "cross_user": 7, "privilege": 5 },
    "total_requests":        261,            // tested * scenarios
    "rate_limited_count":    2,              // 429 responses encountered
    "network_error_count":   4               // request failures (timeout, DNS, TLS)
  }
}
```

### Side effects

For each `finding`:

1. Always call `log_scan_result` with:
   - `test_type` ∈ `auth_bypass | idor | privilege_escalation`
   - `result` mirroring the finding result
   - `evidence` containing the diff_summary
   - `severity` mirroring the finding severity
2. If `severity ∈ {critical, high}` **and** `result == vulnerable`,
   additionally call `record_finding` so it surfaces in the Findings panel.
3. Append one `log_operation` entry summarising the run
   (`op_type: scan`, `tool_name: auth_probe`).

---

## 3. Per-scenario decision matrix

### 3.1 `anonymous`

```
Round 1: no auth headers
Round 2: with token A
```

| Round 1 status | Round 2 status | Body comparison         | Verdict           | Severity |
|----------------|----------------|--------------------------|-------------------|----------|
| 200            | 200            | bodies identical         | `anonymous_access` | critical |
| 200            | 200            | bodies similar (>80%)    | `anonymous_access` | high     |
| 401 / 403      | 200            | (any)                    | `not_vulnerable`  | info     |
| 5xx            | (any)          | (any)                    | `error`           | —        |

### 3.2 `cross_user`

Only run when `endpoint.has_path_params == true` and `id_pool` provides
both users. Skip otherwise (record into `skipped_count`).

```
Round 1: token A, A's ID
Round 2: token A, B's ID    (substituted at id_param_position)
Round 3: token B, B's ID    (sanity baseline)
```

| Round 1 | Round 2 | Round 3 | Verdict        | Severity |
|---------|---------|---------|----------------|----------|
| 200     | 200     | 200     | `cross_user_idor` | high  |
| 200     | 200     | 4xx     | `cross_user_idor` (suspicious) | medium |
| 200     | 403/404 | 200     | `not_vulnerable`  | info   |

### 3.3 `privilege`

Heuristic: path contains `admin` / `internal` / `manage` segment.

```
Round 1: low-privilege token (user_a)
Round 2: no auth — sanity
```

| Round 1 | Round 2 | Verdict                | Severity |
|---------|---------|------------------------|----------|
| 200     | (any)   | `privilege_escalation` | high     |
| 403     | 401     | `not_vulnerable`       | info     |
| 401     | 401     | `inconclusive`         | info     |

---

## 4. Safety constraints

1. **Idempotent verbs only by default**: only `GET / HEAD / OPTIONS` are
   probed. `POST / PUT / PATCH / DELETE` skipped unless `--include-mutating`
   is set (separate parameter, defaults to `false`).
2. **Rate limit honored**: respect `Retry-After` on 429, exponential
   back-off, abort scenario for that endpoint after 3 consecutive 429.
3. **No body modification**: requests are sent as-is — we never construct
   payloads that could write to the target.
4. **Per-endpoint timeout**: `timeout_ms` (default 10s); soft-fail and
   record `network_error_count`.
5. **Max requests cap**: `max_endpoints * scenarios.len()` enforced
   (default 500 × 3 = 1500); abort with clean summary if exceeded.

---

## 5. Implementation sketch (preview only — not part of contract)

```
auth_probe
├── probe::run_single(endpoint, scenario, ctx)  -> Round[]
├── probe::compare(rounds: Round[]) -> Verdict
├── probe::vault_resolve(vault_key) -> Token
├── probe::request(method, url, headers, timeout) -> Round
├── probe::path_substitute_id(path, pos, new_id) -> String
└── persist::write_finding + log_scan_result + log_operation
```

Dependencies: `reqwest::Client` (already in workspace), `tokio::time` for
back-off, `golish-db::repo::audit / repo::pentest` for persistence,
`golish-db::repo::scan` for `log_scan_result`.

---

## 6. Open questions (to confirm with user)

1. **id_pool source**: should `auth_probe` accept `id_pool` directly
   (current draft) or look it up via a separate `id_discovery` tool?
2. **Privilege detection beyond keyword**: do we want a YAML/JSON
   allow-list of "admin" path patterns, or rely on the heuristic plus
   user override?
3. **Rate-limit policy** for cross-tenant probes — single global
   `rate_limit_ms` enough, or per-host?
4. **`--include-mutating` flag**: ok to keep behind opt-in, or do we
   ever want it on by default (clearly destructive risk)?
