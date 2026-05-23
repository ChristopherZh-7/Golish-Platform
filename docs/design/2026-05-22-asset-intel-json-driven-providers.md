# Asset Intel JSON-Driven Providers

> 日期：2026-05-22
> 状态：Draft
> Supersedes: `docs/design/2026-05-22-asset-intel-provider-abstraction.md`

## 1. 问题结论

当前 Asset Intel Phase 4 的方向需要调整。`TargetPanel` 已经不直接认识 ENScan_GO，但 `backend/crates/golish/src/tools/asset_intel.rs` 仍然在 Rust 里写死 provider id、capabilities、auto mode、ENScan CLI 参数、0.zone query types 和 normalize 字段。这会导致新增或替换工具时继续修改 Rust 代码，违背“后续换工具只加载外部 JSON”的目标。

新的设计目标是：Rust 只实现稳定的 provider runtime 和 normalize runtime；每个 provider 的身份、能力、凭据依赖、执行方式和输出映射都从 JSON descriptor 读取。

## 2. 非目标

- 不在 Target / Engagement UI 增加工具专属页面。
- 不让 hydrate 自动把结果写入 active scan scope。
- 不在第一轮实现任意第三方 API 的完整声明式 HTTP DSL；先把 CLI provider JSON 化，再为 HTTP provider 留出明确契约。
- 不把已有 `golish-intel-providers` crate 继续扩展成 Asset Intel 的主注册表；它可以作为迁移期兼容实现，但不再是“新增 provider 必须改 Rust”的路径。

## 3. 分层边界

```text
TargetPanel / Engagement Workspace
  └─ asset_intel_* IPC contract
       └─ Asset Intel Service
            ├─ Descriptor Loader
            │    └─ resources/toolsconfig/*.json / user toolsconfig/*.json
            ├─ Provider Registry
            │    └─ built from JSON `tool.asset_intel`
            ├─ Provider Runtime
            │    ├─ cli_json runtime
            │    └─ http_json runtime
            └─ Candidate Normalizer
                 └─ JSON path / bucket mapping from descriptor

Integrations
  └─ credentials / cookies / external file rendering / connection test / auto-capture
```

The only stable business contract above the service is `AssetIntelProviderDescriptor`, `AssetIntelHydrateArgs`, `AssetIntelRun`, `ProviderStatus`, and `OrganizationCandidate`.

## 4. JSON Descriptor Contract

Provider descriptors live inside existing tool config files under `tool.asset_intel`. A tool without this section is ignored by Asset Intel.

```json
{
  "tool": {
    "id": "enscan-go",
    "name": "ENScan_GO",
    "integration": {},
    "asset_intel": {
      "enabled": true,
      "provider_id": "enscan-go",
      "display_name": "ENScan_GO",
      "capabilities": [
        "subsidiaries",
        "domains",
        "icp",
        "apps",
        "mini_programs",
        "social_accounts"
      ],
      "requires_integration": {
        "tool_id": "enscan-go",
        "group_ids": ["aqc", "tyc", "kc", "rb", "miit"]
      },
      "auto": {
        "default": true,
        "priority": 100
      },
      "runtime": {
        "kind": "cli_json",
        "skill_id": "company-default-json",
        "timeout_secs": 180,
        "artifact_globs": ["**/*.json"],
        "arg_bindings": {
          "org": "{{company_name}}",
          "min_ownership_percent": "{{config.min_ownership_percent}}",
          "depth": "{{config.depth}}",
          "include_branches": "{{config.include_branches}}"
        }
      },
      "normalize": {
        "organization": [
          {
            "path": "$..invest[*]",
            "label": "name",
            "value": "name",
            "confidence": 0.82
          },
          {
            "path": "$..branch[*]",
            "label": "name",
            "value": "name",
            "confidence": 0.78
          }
        ],
        "target": [
          {
            "path": "$..icp[*]",
            "label": "domain",
            "value": "domain",
            "confidence": 0.78
          },
          {
            "path": "$..app[*]",
            "label": "name",
            "value": ["link", "app_url", "name"],
            "confidence": 0.68
          }
        ]
      }
    }
  }
}
```

## 5. Runtime Kinds

### `cli_json`

Executes an installed local tool resolved through existing toolsconfig + `build_run_command` / executable resolution. The descriptor chooses one `skills[].id` as the command template. Runtime injects company name, config values, JSON output mode and temp output directory only through declared bindings.

Rules:

- Provider-specific flags belong in JSON `skills[].args`, not in Rust constants.
- Artifact discovery uses descriptor `artifact_globs`.
- Timeout comes from JSON and is clamped by Rust to a safe range.
- stdout, stderr preview, exit code and artifact paths are evidence metadata.

### `http_json`

Executes one or more HTTP requests declared by JSON. This is the path for providers such as 0.zone once their request shape is moved out of Rust.

First supported shape:

```json
{
  "kind": "http_json",
  "requests": [
    {
      "id": "domains",
      "method": "POST",
      "url": "https://0.zone/api/data/",
      "headers": {
        "X-Token": "{{secret:api_key}}"
      },
      "json": {
        "query": "{{company_name}}",
        "query_type": "domain",
        "page": 1
      },
      "timeout_secs": 30
    }
  ]
}
```

Secrets are referenced by integration field key and resolved through Integrations. Secret values are never written into evidence.

## 6. Candidate Normalize Contract

The normalizer converts JSON values into `OrganizationCandidate`.

Required mapping fields:

- `path`: JSON selector producing objects or scalar values.
- `label`: field name or ordered list of field names.
- `value`: field name or ordered list of field names.
- `confidence`: static number from `0.0` to `1.0`.

Optional mapping fields:

- `when`: simple equality filter against a JSON field.
- `source_field`: value stored into evidence to explain which upstream section produced it.
- `kind`: only needed if mapping is stored in a mixed bucket; default comes from the bucket name.

Candidate ids remain deterministic:

```text
<kind-prefix>:<provider-id>:<trimmed-value>
```

Merge dedupe remains service-owned:

```text
kind + lower(trim(value))
```

When two providers return the same candidate, the first candidate is kept and evidence from the later duplicate is appended to `evidence.alternates`.

## 7. IPC Contract

No frontend breaking change is needed.

```text
asset_intel_list_providers() -> AssetIntelProviderDescriptor[]
asset_intel_hydrate(args: AssetIntelHydrateArgs) -> AssetIntelRun
```

`asset_intel_list_providers` now reads JSON descriptors instead of returning a Rust-built vector.

`asset_intel_hydrate` behavior:

1. Load descriptors from active toolsconfig dirs.
2. If `providerIds` is empty, select descriptors where `auto.default == true`, ordered by `auto.priority`.
3. If `providerIds` is provided, run only matching descriptors.
4. Execute each descriptor through its runtime.
5. Normalize all outputs with the descriptor mappings.
6. Write candidates only when `config.createCandidates != false`.
7. Return provider status and evidence for completed, checked-empty, unavailable and failed states.

## 8. Error And Status Semantics

- `unavailable`: descriptor exists, but executable, integration value, or required runtime is missing.
- `failed`: descriptor ran but returned non-zero, timed out, produced invalid JSON, or HTTP failed.
- `checked_empty`: descriptor ran successfully and normalize produced no candidates.
- `completed`: descriptor ran successfully and produced at least one candidate.

Provider failure must not collapse the whole hydrate run when at least one other provider completes. The run status remains `partial`.

## 9. Migration Strategy

Phase A replaces Rust-built provider descriptors with JSON-loaded descriptors while keeping ENScan behavior equivalent.

Phase B replaces ENScan-specific command construction and JSON parsing with `cli_json` runtime + descriptor normalize mappings.

Phase C removes the Rust `0.zone` special branch. Either ship a `0.zone` JSON `http_json` descriptor or omit it from auto mode until the descriptor is ready.

Phase D adds tests that prove a fake provider can be added by creating only a JSON fixture.

## 10. Acceptance Criteria

- Adding a new CLI Asset Intel provider requires only a toolsconfig JSON file and no Rust code change.
- `asset_intel.rs` no longer contains provider id constants for ENScan_GO or 0.zone.
- `provider_descriptors()` is replaced by descriptor loading from tool config.
- ENScan default args and output mappings live in `resources/toolsconfig/enscan-go.json`.
- Auto mode is based on descriptor metadata, not a hardcoded vector.
- Unit tests include a fixture-only provider and verify it appears in `asset_intel_list_providers` and can normalize candidates.
- Target UI behavior remains unchanged.
