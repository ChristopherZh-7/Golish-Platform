# fofa/hunter/shodan 接入 target_intel 自动富化 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans` 逐任务实现此计划。每个任务单独 commit。TDD：先写失败测试，跑红，再实现转绿。

**目标：** 让 harness `target_intel` 阶段的自动富化（`recon_enrich_assets`）能像 0.zone/quake 那样，按「配了 key 即可用」自动识别并调用 fofa / hunter / shodan 三个原生网络空间测绘 provider。

**架构（方案 B · 原生 provider 桥接）：** 在 asset_intel 运行时新增第三个 runtime kind `native_provider`，其 runner 调用既有 `golish-intel-providers` 注册表（`FofaProvider`/`HunterProvider`/`ShodanProvider`，已带正确鉴权/编码/字段映射），把 `ProviderRecord` 桥接成既有 `OrganizationCandidates` + `ProfileFieldEntry`。给三家各写一个 toolsconfig JSON（`asset_intel` 块声明 `runtime.kind=native_provider` + `requires_integration` + `auto.default=true`）。0.zone/quake 现有 http_json 路径**不动**。

**技术栈：** Rust（golish-pentest-domain models / golish-recon-app asset_intel）+ golish-intel-providers crate + toolsconfig JSON（运行时直读，无需重编译）。

---

## 背景与关键证据（实现前必读，均已核对真实代码 2026-06-08）

| 事实 | 落点 |
|---|---|
| 自动富化入口 | `golish-recon-app/src/asset_intel/agent_tools/mod.rs::ReconEnrichAssetsTool` → `run_passive_intel(PassiveIntelPhase::Enrich)` |
| runtime 分发点 | `golish-recon-app/src/asset_intel/service/hydrate.rs:149` `match &asset.runtime { CliJson => run_cli_json_provider, HttpJson => run_http_json_provider }` |
| runtime 枚举 | `golish-pentest-domain/src/models/asset_intel.rs:87-103` `enum AssetIntelRuntimeConfig`（`#[serde(tag="kind", rename_all="snake_case")]`，现有 `CliJson` / `HttpJson`） |
| 原生注册表 | `golish-recon-app/src/intel_providers.rs:71-79` `fn provider_registry() -> HashMap<String, Arc<dyn IntelProvider>>`（**私有**，含 0.zone/fofa/quake/hunter/shodan 5 家） |
| IntelProvider trait | `golish-intel-providers/src/lib.rs:52` `query(QueryType, &str, key:&str) -> Vec<ProviderRecord>` |
| ProviderRecord | `golish-intel-providers/src/types.rs`：`{ provider, query_type, fields: HashMap<String,String>, raw }`；fields 键 = host/ip/port/protocol/title/server/country/domain/cert |
| 候选类型 | `golish-recon-app/src/organizations/types.rs:43-75` `OrganizationCandidateKind{Organization,Target}` + `OrganizationCandidate{id,kind,label,value,source,confidence,status,evidence,created_at}` + `OrganizationCandidates{organizations,targets}` |
| profile 桥接类型 | `golish-recon-app/src/asset_intel/types.rs:272` `ProfileFieldEntry{target_kind: AssetIntelProfileFieldTarget, target_field:String, value:String}` |
| profile target 枚举 | `golish-pentest-domain/src/models/asset_intel.rs:213-227` `AssetIntelProfileFieldTarget{Scalar,Intel,Contact}` |
| 凭据读取（http 路径） | `golish-recon-app/src/asset_intel/runtime/mod.rs:16` `read_vault_secret(pool, tool_id, group_id, field_key)` → 先查 `name="{tool_id}.{group_id}.{field_key}"`，group=="default" 时**回退** `name=tool_id, entry_type='api_key'` |
| 凭据读取（原生 UI 路径） | `intel_providers.rs:48` `PgVaultKeyStore`：`name=provider_id, entry_type='api_key'`（与上面 default 回退命中同一行） |
| 可用性检测 | `golish-recon-app/src/asset_intel/availability.rs:79` `list_provider_availability` 按 `requires_integration` 经 integrations resolver 查凭据 → `available` |
| 自动选择 | `golish-recon-app/src/asset_intel/capability.rs:214` `select_asset_intel_providers`（requested 空 → 留 `auto.default==true`，按 priority 排序）；`select_enrichment_providers` 再滤掉带 `subsidiaries` capability 的 |

**关键设计点（原生 provider 按网络标识查询，不按公司名）：** `FofaProvider::render_query`（fofa/mod.rs:77）只支持 `Site=host="..."`/`Domain`/`Cert`；shodan/hunter `Site` 在输入「看起来像 DSL」时透传。富化是按 `company_name`（org 名）驱动的，故 runner 用**每家在 toolsconfig 里声明的 org 查询模板**（如 fofa `org="{{company_name}}"`、shodan `org:"{{company_name}}"`），渲染后以 `QueryType::Site` 透传给 provider。DSL 放配置便于纠错。

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-pentest-domain/src/models/asset_intel.rs` | runtime 枚举加 `NativeProvider` 变体 + `NativeProviderQuery` 子类型 | 改 |
| `backend/crates/golish-recon-app/src/intel_providers.rs` | 把 `provider_registry()` 提为 `pub(crate)` 供桥接 runner 复用 | 改 |
| `backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs` | 新 runner `run_native_provider`：调注册表 → ProviderRecord 桥接成候选+profile | 新建 |
| `backend/crates/golish-recon-app/src/asset_intel/runtime/mod.rs` | `pub(crate) mod native; pub(crate) use native::*;` | 改 |
| `backend/crates/golish-recon-app/src/asset_intel/service/hydrate.rs` | 分发 match 加 `NativeProvider` 臂 | 改 |
| `backend/crates/golish-recon-app/src/asset_intel/runtime/lookup.rs` | lookup 守卫排除 `NativeProvider`（lookup 仅 CliJson） | 改 |
| `resources/toolsconfig/fofa.json` | fofa asset_intel native_provider 声明 | 新建 |
| `resources/toolsconfig/hunter.json` | hunter 同上 | 新建 |
| `resources/toolsconfig/shodan.json` | shodan 同上 | 新建 |
| `backend/crates/golish-recon-app/src/asset_intel/tests.rs` | 解析 + runner 桥接单测 | 改 |

---

## Task 0：确认凭据 schema 解析路径（调查，不改码，先做）

**为什么：** `list_provider_availability` 走 integrations resolver（`DefaultSchemaResolver.get(tool_id)` → schema.groups 找 `default` 组 → `read_cleartext`）。0.zone/quake 的 toolsconfig **没有显式 `integration` 块**却能被检测——必须确认 schema 从哪来（很可能是 `golish_intel_providers::api_key_integration_schema` 默认合成，或 resolver 对 asset_intel provider 的兜底）。fofa/hunter/shodan 必须走同一条路才能被 `available` 检测到。

**步骤：**
1. 读 `golish-recon-app/src/integrations/state.rs::build_integration_resolver` 与 `DefaultSchemaResolver`，搞清 `get("0.zone")` 返回的 schema 来源（是否来自 `ProviderMeta.integration_schema`）。
2. 读 `golish-intel-providers/src/types.rs::api_key_integration_schema`，确认 fofa（`email|key` 单字段）/hunter/shodan 的 `meta().integration_schema` 已声明 `default` 组 + `api_key` secret 字段（fofa/mod.rs:129、hunter/shodan mod.rs 同位置已见 `integration_schema: Some(...)`）。
3. **判定**：若 resolver 用 `ProviderMeta.integration_schema` 合成 schema，则三家**无需**额外 integration 块，`requires_integration{tool_id:<id>,group_ids:["default"]}` 即可被检测；否则需在各 toolsconfig 补 `integration` 块。把结论写进本任务下方「调查结论」。

**验证：** 把结论（schema 来源 + 三家是否需补 integration 块）记录到本文件本任务末尾，再进入 Task 1。无代码改动、无 commit。

> **调查结论（2026-06-08 已核实真实代码，T0 完成）：**
> 1. **schema 来源 = 各 provider 的 `ProviderMeta.integration_schema`（in-code 合成，非 toolsconfig）。** `availability.rs` 用 `build_integration_resolver`（state.rs:296）→ `DefaultSchemaResolver::new(toolsconfig_dir, collect_in_code_schemas())`；`collect_in_code_schemas()`（state.rs:287→239）遍历 5 个 provider（ZoneProvider/FofaProvider/QuakeProvider/HunterProvider/ShodanProvider），对每个 `meta().integration_schema==Some` 推入 `ResolvedIntegration{tool_id, schema}`。这就是 0.zone/quake「toolsconfig 无 integration 块却能被检测」的原因。
> 2. **fofa/hunter/shodan 三家均已声明 `integration_schema: Some(api_key_integration_schema(...))`**（fofa/mod.rs:129、hunter/mod.rs:121、shodan/mod.rs:123，已逐一核对）。`api_key_integration_schema` 产出 `Storage::Vault` + 单 `default` 组 + 1 个 `api_key` secret 字段。fofa 字段语义为 `email|key` 组合串（meta 提示已写明），`FofaProvider::query` 内部 `split_credentials` 拆分。
> 3. **三家无需在 toolsconfig 补 `integration` 块**；`requires_integration{tool_id:<id>, group_ids:["default"]}` 即可被 resolver 命中。
> 4. **凭据读取两条路径都带 legacy 回退，互通**：
>    - 可用性检测：`VaultBackend::read_cleartext`（vault.rs:262）先查 `{tool_id}.default.api_key`，default 组单字段时**回退** `legacy_intel_provider_value_plain(name=tool_id)`（vault.rs:277-282）。
>    - runner 读取：`read_vault_secret`（runtime/mod.rs:16）先查 `{tool_id}.default.api_key`，default 组回退 `name=tool_id, entry_type='api_key'`。
>    → 用户无论从 **Settings→Integrations**（存 `fofa.default.api_key`）还是 **Settings→Intel Providers**（存 `name=fofa`）配的 key，可用性检测与 runner **都能命中**。"之前写进去的" key 直接可用，无需迁移。
>
> **对后续任务的影响**：Task 5 三个 toolsconfig **不需要** `integration` 块（删除该步的「若需补」分支）。其余任务不变。

---

## Task 1：runtime 枚举加 `NativeProvider` 变体（TDD）

**文件：**
- 测试：`backend/crates/golish-pentest-domain/src/models/` 内 `asset_intel.rs` 的 `#[cfg(test)]`（若无则在文件末尾新增 `mod tests`）
- 实现：`backend/crates/golish-pentest-domain/src/models/asset_intel.rs`

**步骤 1（红）：** 在 `asset_intel.rs` 末尾加测试：

```rust
#[cfg(test)]
mod native_provider_tests {
    use super::*;

    #[test]
    fn parses_native_provider_runtime() {
        let json = r#"{
            "kind": "native_provider",
            "provider_id": "fofa",
            "queries": [{ "query_type": "site", "template": "org=\"{{company_name}}\"" }]
        }"#;
        let rt: AssetIntelRuntimeConfig = serde_json::from_str(json).expect("parse");
        match rt {
            AssetIntelRuntimeConfig::NativeProvider { provider_id, queries } => {
                assert_eq!(provider_id, "fofa");
                assert_eq!(queries.len(), 1);
                assert_eq!(queries[0].query_type, "site");
                assert_eq!(queries[0].template, "org=\"{{company_name}}\"");
            }
            other => panic!("expected NativeProvider, got {other:?}"),
        }
    }
}
```

**步骤 2：** 跑 `cd backend && cargo test -p golish-pentest-domain native_provider` → 预期编译失败（变体不存在）。

**步骤 3（绿）：** 在 `enum AssetIntelRuntimeConfig`（asset_intel.rs:89）`HttpJson` 后加变体，并在文件加子类型：

```rust
    NativeProvider {
        /// Registry id (e.g. "fofa" / "hunter" / "shodan").
        provider_id: String,
        /// Org-name → provider query renderings, run in order.
        #[serde(default)]
        queries: Vec<NativeProviderQuery>,
    },
```

```rust
/// One org-name-driven query for a native intel provider. `template` is
/// rendered with `{{company_name}}` then parsed into the provider's DSL via
/// `QueryType::Site` pass-through (see runtime/native.rs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeProviderQuery {
    /// QueryType discriminator string ("site" / "domain" / "cert" / "asn" / "cidr").
    pub query_type: String,
    /// Query template with `{{company_name}}` placeholder.
    pub template: String,
}
```

**步骤 4：** 跑 `cargo test -p golish-pentest-domain native_provider` → 绿。`cargo build -p golish-recon-app` → 预期出现 `non-exhaustive match` 报错于 hydrate.rs/lookup.rs（Task 4/守卫处理）。

**验证：** `cargo test -p golish-pentest-domain native_provider` exit 0。
**提交：** `feat(asset-intel): add NativeProvider runtime variant`

---

## Task 2：把 `provider_registry()` 提为 `pub(crate)`

**文件：** `backend/crates/golish-recon-app/src/intel_providers.rs`

**步骤 1：** 把 `fn provider_registry()`（intel_providers.rs:71）改为 `pub(crate) fn provider_registry()`。不改函数体。

**步骤 2：** 跑 `cargo build -p golish-recon-app`（仍会因 Task 1 的 non-exhaustive match 报错，但 `intel_providers.rs` 本身应无新错）。

**验证：** `cargo clippy -p golish-recon-app --lib 2>&1 | rg "provider_registry"` 无可见性错误（其余 non-exhaustive 错误属预期，下个任务消除）。
**提交：** `refactor(intel): expose provider_registry to crate`

---

## Task 3：实现 `run_native_provider` 桥接 runner（TDD）

**文件：**
- 新建：`backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs`
- 改：`backend/crates/golish-recon-app/src/asset_intel/runtime/mod.rs`（注册子模块）

**步骤 1：** 在 `runtime/mod.rs` 的子模块声明区（mod.rs:8-14）加：

```rust
pub(crate) mod native;
```
并在 `pub(crate) use` 区加：
```rust
pub(crate) use native::*;
```

**步骤 2（实现）：** 新建 `runtime/native.rs`。核心：解析 `NativeProvider` 配置 → `read_vault_secret` 取 key → 从注册表取 provider → 逐条 query 渲染 `{{company_name}}` → `provider.query(QueryType::Site, &rendered, &key)` → `ProviderRecord` 桥接成候选 + profile entries。

```rust
//! Native intel-provider runner: bridges the `golish-intel-providers`
//! registry (fofa / hunter / shodan / ...) into the asset_intel pipeline so
//! the harness target_intel auto-enrich can use them like the http_json
//! providers (0.zone / quake). Credentials come from the same vault path as
//! http_json (`read_vault_secret`), so availability + reading stay consistent.

use super::super::*;
use crate::intel_providers::provider_registry;
use crate::organizations::{OrganizationCandidate, OrganizationCandidateKind};
use golish_intel_providers::{ProviderRecord, QueryType};

fn parse_query_type(s: &str) -> QueryType {
    match s {
        "domain" => QueryType::Domain,
        "cert" => QueryType::Cert,
        "asn" => QueryType::Asn,
        "cidr" => QueryType::Cidr,
        // default + "site" → Site (providers pass DSL through on Site)
        _ => QueryType::Site,
    }
}

/// Map one ProviderRecord into a Target candidate (when it has a host/domain/ip)
/// plus profile field entries (domains / ip_ranges / certificates / fingerprints).
fn bridge_record(provider_id: &str, rec: &ProviderRecord) -> (Option<OrganizationCandidate>, Vec<ProfileFieldEntry>) {
    use golish_pentest::models::AssetIntelProfileFieldTarget as T;
    let f = &rec.fields;
    let mut profile = Vec::new();
    let mut push = |kind: T, field: &str, key: &str| {
        if let Some(v) = f.get(key) {
            if !v.trim().is_empty() {
                profile.push(ProfileFieldEntry { target_kind: kind, target_field: field.to_string(), value: v.trim().to_string() });
            }
        }
    };
    push(T::Scalar, "domains", "domain");
    push(T::Scalar, "ip_ranges", "ip");
    push(T::Scalar, "certificates", "cert");
    push(T::Intel, &format!("{provider_id}_http_titles"), "title");
    push(T::Intel, &format!("{provider_id}_http_servers"), "server");

    // surface candidate keyed on the most stable identifier (domain > host > ip)
    let host = f.get("domain").or_else(|| f.get("host")).or_else(|| f.get("ip"));
    let candidate = host.filter(|v| !v.trim().is_empty()).map(|v| OrganizationCandidate {
        id: String::new(),
        kind: OrganizationCandidateKind::Target,
        label: v.trim().to_string(),
        value: v.trim().to_string(),
        source: provider_id.to_string(),
        confidence: 0.7,
        status: "candidate".to_string(),
        evidence: serde_json::json!({ "provider": provider_id, "query_type": rec.query_type.as_str() }),
        created_at: golish_core::time::now_ms(),
    });
    (candidate, profile)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_native_provider(
    pool: &sqlx::PgPool,
    tool: &ToolConfig,
    project_root: &Path,
    run_id: &str,
    company_name: &str,
    _config: &AssetIntelHydrateConfig,
    sink: Option<&EventEmitterHandle>,
) -> Result<(AssetIntelProviderRunStatus, OrganizationCandidates, Value, Vec<ProfileFieldEntry>), GolishError> {
    let Some(asset) = tool.asset_intel.as_ref() else {
        return Err(GolishError::Validation(format!("tool '{}' has no asset_intel descriptor", tool.id)));
    };
    let (provider_id, display_name) = provider_identity(tool, asset);
    let golish_pentest::models::AssetIntelRuntimeConfig::NativeProvider { provider_id: reg_id, queries } = &asset.runtime else {
        return Err(GolishError::Validation(format!("tool '{}' is not a native_provider", tool.id)));
    };
    emit_provider_started(sink, run_id, &provider_id, display_name, AssetIntelProviderRuntimeKind::HttpJson);

    // credential (same vault path as http_json; "default" group + api_key field,
    // with legacy fallback to name=tool_id/type=api_key — see read_vault_secret).
    let (tool_id, group_id) = match asset.requires_integration.as_ref() {
        Some(req) => (req.tool_id.clone(), req.group_ids.first().cloned().unwrap_or_else(|| "default".into())),
        None => (provider_id.clone(), "default".into()),
    };
    let key = match read_vault_secret(pool, &tool_id, &group_id, "api_key").await? {
        Some(k) if !k.trim().is_empty() => k,
        _ => {
            let status = AssetIntelProviderRunStatus { provider_id: provider_id.clone(), status: AssetIntelProviderRunState::Unavailable, message: format!("no api key configured for {provider_id}") };
            return finish_provider_run(sink, run_id, status, 0, OrganizationCandidates::default(),
                serde_json::json!({ "provider": provider_id, "runId": run_id, "state": "unavailable", "reason": "missing_secrets" }), Vec::new());
        }
    };

    let registry = provider_registry();
    let Some(provider) = registry.get(reg_id) else {
        let status = AssetIntelProviderRunStatus { provider_id: provider_id.clone(), status: AssetIntelProviderRunState::Failed, message: format!("native provider '{reg_id}' not in registry") };
        return finish_provider_run(sink, run_id, status, 0, OrganizationCandidates::default(),
            serde_json::json!({ "provider": provider_id, "runId": run_id, "state": "failed", "reason": "unknown_provider" }), Vec::new());
    };

    let mut candidates = OrganizationCandidates::default();
    let mut profile_entries: Vec<ProfileFieldEntry> = Vec::new();
    let mut request_evidence = Vec::new();
    for q in queries {
        let rendered = q.template.replace("{{company_name}}", company_name);
        let qt = parse_query_type(&q.query_type);
        match provider.query(qt, &rendered, &key).await {
            Ok(records) => {
                for rec in &records {
                    let (cand, profile) = bridge_record(&provider_id, rec);
                    profile_entries.extend(profile);
                    if let Some(c) = cand { candidates.targets.push(c); }
                }
                request_evidence.push(serde_json::json!({ "query": rendered, "records": records.len() }));
            }
            Err(e) => {
                request_evidence.push(serde_json::json!({ "query": rendered, "error": e.to_string() }));
                tracing::warn!(provider=%provider_id, query=%rendered, error=%e, "native provider query failed");
            }
        }
    }

    let _ = (project_root,); // native providers write no artifacts; evidence is the JSON summary
    let candidate_count = candidates.organizations.len() + candidates.targets.len();
    let record_count = candidate_count + profile_entries.len();
    let state = if record_count == 0 { AssetIntelProviderRunState::CheckedEmpty } else { AssetIntelProviderRunState::Completed };
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: format!("{provider_id} produced {record_count} record(s)"),
    };
    finish_provider_run(sink, run_id, status, candidate_count, candidates,
        serde_json::json!({ "provider": provider_id, "runId": run_id, "state": if record_count==0 {"checked_empty"} else {"completed"}, "candidateCount": candidate_count, "profileFieldCount": profile_entries.len(), "queries": request_evidence }),
        profile_entries)
}
```

> 注：`AssetIntelProviderRuntimeKind` 当前枚举仅有 `HttpJson`/`CliJson`（见 types.rs）。本任务先复用 `HttpJson` 作为 started 事件的 runtime 标签以避免扩 ts-rs 类型；如需精确区分，另起任务给 `AssetIntelProviderRuntimeKind` 加 `Native` 变体并走 ts-rs（I5）。`bridge_record` 的 profile 字段映射（domains/ip_ranges/certificates/<id>_http_titles/<id>_http_servers）对齐 `0-zone.json`/`quake.json` 现有 `profile_fields` 命名约定。

**步骤 3（红）：** 在 `asset_intel/tests.rs` 加 runner 单测（用一个不依赖网络的桥接纯函数测，避免真打 provider）。先测 `bridge_record`：把它改为 `pub(crate)` 并加：

```rust
#[test]
fn native_bridge_record_maps_surface_and_profile() {
    use golish_intel_providers::{ProviderRecord, QueryType};
    let mut fields = std::collections::HashMap::new();
    fields.insert("domain".to_string(), "api.example.com".to_string());
    fields.insert("ip".to_string(), "1.2.3.4".to_string());
    fields.insert("title".to_string(), "Hello".to_string());
    let rec = ProviderRecord::new("fofa", QueryType::Site, fields, serde_json::json!({}));
    let (cand, profile) = super::runtime::bridge_record("fofa", &rec);
    assert_eq!(cand.unwrap().value, "api.example.com");
    assert!(profile.iter().any(|p| p.target_field == "domains" && p.value == "api.example.com"));
    assert!(profile.iter().any(|p| p.target_field == "ip_ranges" && p.value == "1.2.3.4"));
    assert!(profile.iter().any(|p| p.target_field == "fofa_http_titles" && p.value == "Hello"));
}
```
（把 `bridge_record` 与 `parse_query_type` 标 `pub(crate)`，并在 native.rs 顶部 `#[cfg(test)] mod` 之外可见。）

**步骤 4：** 跑 `cargo test -p golish-recon-app native_bridge_record` → 红（函数未导出/编译错）→ 调整可见性 → 绿。

**验证：** `cargo test -p golish-recon-app native_bridge_record` exit 0。
**提交：** `feat(asset-intel): native_provider runner bridging intel registry`

---

## Task 4：分发接线 + lookup 守卫

**文件：** `service/hydrate.rs`、`runtime/lookup.rs`

**步骤 1：** `hydrate.rs:149` 的 `match &asset.runtime` 加第三臂（与 http_json 用同一并发闸 `http_limit`，因 native provider 也是出网 HTTP）：

```rust
                golish_pentest::models::AssetIntelRuntimeConfig::NativeProvider { .. } => {
                    let _permit = http_limit.acquire_owned().await.map_err(|error| {
                        GolishError::Internal(format!("asset intel HTTP concurrency limiter closed: {error}"))
                    })?;
                    run_native_provider(pool, &tool, project_root, run_id, company_name, config, sink).await
                }
```
（确认 `company_name` / `config` / `run_id` 等在该 match 作用域内的实际变量名，对齐 http_json 臂的传参。）

**步骤 2：** `lookup.rs` 的 `run_lookup_cli_provider` 已用 `if !matches!(asset.runtime, CliJson{..})` 提前返回（lookup.rs:54）——`NativeProvider` 天然落入「非 CliJson → 跳过」，**无需改**；确认该守卫对新变体仍返回非 lookup 状态即可（读一遍确认，不改则跳过）。

**步骤 3：** 跑 `cargo build -p golish-recon-app` → 预期 non-exhaustive match 报错消除，编译通过。

**验证：** `cargo build -p golish-recon-app` exit 0；`cargo clippy -p golish-recon-app --lib -- -D warnings` exit 0。
**提交：** `feat(asset-intel): dispatch native_provider in hydrate`

---

## Task 5：写三家 toolsconfig JSON

**文件：** 新建 `resources/toolsconfig/{fofa,hunter,shodan}.json`

**步骤 1：** 写 `fofa.json`（org 查询用 fofa DSL `org="..."`；capabilities 为纯富化，不含 subsidiaries，故 `select_enrichment_providers` 会选它）：

```json
{
  "tool": {
    "id": "fofa",
    "name": "FOFA（鹰图）",
    "description": "FOFA cyberspace mapping provider exposed to Asset Intel via the native golish-intel-providers registry.",
    "executable": "",
    "runtime": "native",
    "launchMode": "cli",
    "category": "recon",
    "subcategory": "osint",
    "tags": ["recon", "osint", "asm", "native-provider", "credential-required"],
    "pentestPhase": ["recon"],
    "tier": "recommended",
    "asset_intel": {
      "enabled": true,
      "provider_id": "fofa",
      "display_name": "FOFA（鹰图）",
      "capabilities": ["domains", "ips", "services"],
      "requires_integration": { "tool_id": "fofa", "group_ids": ["default"] },
      "auto": { "default": true, "priority": 70 },
      "runtime": {
        "kind": "native_provider",
        "provider_id": "fofa",
        "queries": [
          { "query_type": "site", "template": "org=\"{{company_name}}\"" }
        ]
      }
    }
  }
}
```

**步骤 2：** 写 `hunter.json`（同结构，`provider_id`/`id` 改 `hunter`，priority `65`，org DSL 用 hunter 的单位/ICP 字段——实现时按 `hunter/mod.rs::render_query` 与 hunter DSL 文档确认确切语法；候选模板：`icp.name="{{company_name}}"`，若 hunter DSL 用别名则据实改）：

```json
{
  "tool": {
    "id": "hunter", "name": "奇安信 Hunter", "executable": "", "runtime": "native",
    "launchMode": "cli", "category": "recon", "subcategory": "osint",
    "tags": ["recon","osint","asm","native-provider","credential-required"],
    "pentestPhase": ["recon"], "tier": "recommended",
    "description": "Qianxin Hunter cyberspace mapping via native intel registry.",
    "asset_intel": {
      "enabled": true, "provider_id": "hunter", "display_name": "奇安信 Hunter",
      "capabilities": ["domains","ips","services"],
      "requires_integration": { "tool_id": "hunter", "group_ids": ["default"] },
      "auto": { "default": true, "priority": 65 },
      "runtime": { "kind": "native_provider", "provider_id": "hunter",
        "queries": [{ "query_type": "site", "template": "icp.name=\"{{company_name}}\"" }] }
    }
  }
}
```

**步骤 3：** 写 `shodan.json`（org DSL `org:"..."`，priority `60`）：

```json
{
  "tool": {
    "id": "shodan", "name": "Shodan", "executable": "", "runtime": "native",
    "launchMode": "cli", "category": "recon", "subcategory": "osint",
    "tags": ["recon","osint","asm","native-provider","credential-required"],
    "pentestPhase": ["recon"], "tier": "recommended",
    "description": "Shodan banner mapping via native intel registry.",
    "asset_intel": {
      "enabled": true, "provider_id": "shodan", "display_name": "Shodan",
      "capabilities": ["domains","ips","services"],
      "requires_integration": { "tool_id": "shodan", "group_ids": ["default"] },
      "auto": { "default": true, "priority": 60 },
      "runtime": { "kind": "native_provider", "provider_id": "shodan",
        "queries": [{ "query_type": "site", "template": "org:\"{{company_name}}\"" }] }
    }
  }
}
```

> **T0 已确认：三家无需 `integration` 块。** resolver 从各 `ProviderMeta.integration_schema` 合成 schema（in-code），`requires_integration{tool_id,group_ids:["default"]}` 即可被命中；用户已配的 key（任一页面）经 legacy 回退可被检测+读取。故本任务只写 `asset_intel` 块即可，不加 `integration`。

**步骤 4：** 校验 JSON 合法：`python -c "import json,glob; [json.load(open(f)) for f in ['resources/toolsconfig/fofa.json','resources/toolsconfig/hunter.json','resources/toolsconfig/shodan.json']]; print('ok')"`。

**验证：** 上述 python 打印 `ok`。
**提交：** `feat(toolsconfig): fofa/hunter/shodan native_provider asset_intel`

---

## Task 6：解析 + 选择单测

**文件：** `asset_intel/tests.rs`

**步骤 1：** 仿照现有 quake 解析测（tests.rs:175 附近）加一个测，断言 scan_toolsconfig 后 fofa 被解析成 `NativeProvider` 且 `provider_id="fofa"`、`auto.default=true`、capabilities 不含 `subsidiaries`：

```rust
#[test]
fn fofa_toolsconfig_parses_as_native_provider() {
    let json = include_str!("../../../../resources/toolsconfig/fofa.json");
    let tool: golish_pentest::models::ToolConfig =
        serde_json::from_value(serde_json::from_str::<serde_json::Value>(json).unwrap()["tool"].clone()).unwrap();
    let asset = tool.asset_intel.unwrap();
    assert_eq!(asset.provider_id, "fofa");
    assert!(asset.auto.default);
    assert!(!asset.capabilities.iter().any(|c| c == "subsidiaries"));
    match asset.runtime {
        golish_pentest::models::AssetIntelRuntimeConfig::NativeProvider { provider_id, queries } => {
            assert_eq!(provider_id, "fofa");
            assert_eq!(queries.len(), 1);
        }
        other => panic!("expected NativeProvider, got {other:?}"),
    }
}
```
（`include_str!` 路径相对 tests.rs 实际深度核对调整。）

**步骤 2：** 加一个 `select_enrichment_providers` 测：构造含 fofa native + enscan(subsidiaries) 的 tools，断言 `select_enrichment_providers(tools, &[])` 含 fofa、不含 enscan。

**验证：** `cargo test -p golish-recon-app fofa_toolsconfig_parses` + `cargo test -p golish-recon-app select_enrichment` exit 0。
**提交：** `test(asset-intel): native provider parse + selection`

---

## Task 7：全量验证

**步骤：**
1. `cd backend && cargo nextest run -p golish-recon-app -p golish-pentest-domain` → 全绿、0 failed。
2. `cargo clippy -p golish-recon-app -p golish-pentest-domain --all-targets -- -D warnings` → exit 0。
3. `cargo fmt -p golish-recon-app -p golish-pentest-domain --check` → clean。
4. 跨 crate 引用核查：`rg "provider_registry|run_native_provider|NativeProvider" backend/crates --type rust` 确认无悬空引用。
5. （活体，需用户授权 + 各家 key）`just dev` → 进 target_intel → `recon_list_providers` 应返回 fofa/hunter/shodan（配 key 后 `available:true`）→ `recon_enrich_assets` 日志出现 native provider 跑动 + 候选/profile 落库。

**验证：** 1-4 全 exit 0，证据贴入 `agent-progress.md`。
**提交：** （无代码，验证步骤）

---

## Task 8：文档 + 收口

**步骤：**
1. `feature_list.json` 加条目 `native-intel-providers-auto-enrich`（状态随实现推进）。
2. `agent-progress.md` 记本轮：目标、改动文件、跑过的验证命令+输出、未跑项（活体）、下一步。
3. 更新模块卡：`docs/modules/backend/golish-recon-app/asset_intel.md`（新增 native_provider runtime）+ `docs/modules/backend/golish-intel-providers.md`（注册表被 asset_intel 复用）+ `docs/modules/INDEX.md` 状态列。
4. **code-audit 收口**（global-enforcement §0 收口规则）：复核 IDOR（org 归属，run_phase 已校验）、凭据不外泄（key 不进 tracing）、I8（无 provider/key 记 unavailable 不伪造）、回滚点（删 3 JSON + 变体）。
5. `just precommit`（前后端全量）→ 全绿后方可 commit 收尾；**push 需用户单独授权**（AGENTS.md §2.7）。

**验证：** `just precommit` exit 0，输出 `✓ All checks passed!`。
**提交：** `docs: native intel provider auto-enrich progress + module cards`

---

## 自检（规格覆盖 / 占位符 / 类型一致）

- **覆盖**：runtime 变体(T1) + 注册表暴露(T2) + runner+桥接(T3) + 分发(T4) + 三 JSON(T5) + 解析/选择测(T6) + 验证(T7) + 文档收口(T8)，覆盖方案 B 全部改面（golish-pentest-domain / golish-recon-app / toolsconfig）。
- **占位符**：Task 0 的「调查结论」与 hunter org DSL 为「实现时据真实代码/文档确认」——非占位逃避，而是必须实读校准的精确点（已标注读哪个文件/函数）。
- **类型一致**：`NativeProvider{provider_id,queries:Vec<NativeProviderQuery{query_type,template}>}`、`bridge_record(&str,&ProviderRecord)->(Option<OrganizationCandidate>,Vec<ProfileFieldEntry>)`、`run_native_provider(...)->(AssetIntelProviderRunStatus,OrganizationCandidates,Value,Vec<ProfileFieldEntry>)` 在 T1/T3/T4/T6 间签名一致。
- **风险/回滚**：纯增量（新变体 + 新 runner + 新 JSON），0.zone/quake/ENScan 不动；回滚 = 删三 JSON + 还原枚举/分发。
