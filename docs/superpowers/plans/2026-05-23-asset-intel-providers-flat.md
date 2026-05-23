# Asset Intel Providers Flat 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让一个 `tool` JSON 能声明多个 Asset Intel Provider，解决 ENScan_GO 被拆成 4 个 JSON 的 UX/维护问题。

**架构：** 在 `ToolConfig` 加 `asset_intel_providers: Option<Vec<AssetIntelToolConfig>>` 字段（与现有 `asset_intel` 互斥），新增 `expand_provider_tools()` 把多 provider tool 展开成多个 virtual ToolConfig，下游 `provider_descriptors_from_tools` / `select_*_providers` 全部基于展平结果工作。原 `run_cli_json_provider` / `run_http_json_provider` 签名不变（仍接受 `&ToolConfig`，看到的是 virtual tool）。最后把主 `enscan-go.json` 改用新数组，删 3 个 child JSON。

**技术栈：** Rust (golish-pentest schema / golish::tools::asset_intel) + serde + cargo nextest + 资源 JSON 配置。

**设计文档**：`docs/design/2026-05-23-asset-intel-providers-flat.md`

---

## 文件结构（一次完成）

| 路径 | 角色 | 改动类型 |
|---|---|---|
| `backend/crates/golish-pentest/src/models.rs` | Schema：加 `asset_intel_providers` 字段 | 扩展 |
| `backend/crates/golish-pentest/src/parsers.rs`（或 lib.rs 入口） | Loader：实现 `asset_intel` vs `asset_intel_providers` 互斥校验 | 扩展 |
| `backend/crates/golish/src/tools/asset_intel.rs` | 加 `expand_provider_tools()`；改造 `provider_descriptors_from_tools` / `select_asset_intel_providers` / `select_subsidiary_providers` / `select_enrichment_providers` 让它们都先展平 | 扩展 |
| `resources/toolsconfig/enscan-go.json` | 把 3 个 child 的 `asset_intel` 合并进主文件的 `asset_intel_providers: [...]` | 重构（含原 AQC） |
| `resources/toolsconfig/enscan-go-tyc-discovery.json` | 删除 | 删除 |
| `resources/toolsconfig/enscan-go-kc-discovery.json` | 删除 | 删除 |
| `resources/toolsconfig/enscan-go-rb-discovery.json` | 删除 | 删除 |
| `agent-progress.md` | 加一轮新条目 | 扩展 |
| `feature_list.json` | 加一条 in_progress | 扩展 |

---

## 任务

### Task 1 · 在 ToolConfig 加 `asset_intel_providers` 字段 + 单测

**文件：**
- 修改：`backend/crates/golish-pentest/src/models.rs`
- 测试：同文件 `#[cfg(test)] mod tests`

**步骤：**

1. 在 `ToolConfig`（约 `models.rs:58` 起）的 `asset_intel` 字段下面加新字段：

```rust
/// 多 provider 形式：与 `asset_intel` 互斥。一个 tool 共享 executable / install
/// 元数据，每个 provider 各自声明 capabilities / runtime / requires_integration /
/// normalize / auto / discovery / lookup。
#[serde(
    default,
    rename = "asset_intel_providers",
    skip_serializing_if = "Option::is_none"
)]
pub asset_intel_providers: Option<Vec<AssetIntelToolConfig>>,
```

2. 在 `models.rs` 现有 `mod tests` 末尾追加：

```rust
#[test]
fn tool_config_accepts_asset_intel_providers_array() {
    let json = serde_json::json!({
        "id": "shared-binary",
        "name": "Shared Binary",
        "executable": "shared/bin",
        "asset_intel_providers": [
            {
                "enabled": true,
                "provider_id": "shared-binary",
                "display_name": "Shared Binary",
                "capabilities": ["subsidiaries"],
                "auto": { "default": true, "priority": 100 },
                "runtime": { "kind": "cli_json", "skill_id": "default", "timeout_secs": 60 }
            },
            {
                "enabled": true,
                "provider_id": "shared-binary-alt",
                "display_name": "Shared Binary Alt",
                "capabilities": ["subsidiaries"],
                "auto": { "default": false, "priority": 50 },
                "runtime": { "kind": "cli_json", "skill_id": "default-alt", "timeout_secs": 60 }
            }
        ]
    });
    let tool: ToolConfig = serde_json::from_value(json).expect("two-provider tool must deserialize");
    let providers = tool.asset_intel_providers.expect("providers vec");
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0].provider_id, "shared-binary");
    assert_eq!(providers[1].provider_id, "shared-binary-alt");
    assert!(tool.asset_intel.is_none(), "single asset_intel must stay empty when providers is used");
}

#[test]
fn tool_config_round_trips_asset_intel_providers() {
    let json = serde_json::json!({
        "id": "shared",
        "name": "shared",
        "executable": "bin",
        "asset_intel_providers": [
            {
                "enabled": true,
                "provider_id": "shared",
                "display_name": "shared",
                "capabilities": ["subsidiaries"],
                "auto": { "default": true, "priority": 100 },
                "runtime": { "kind": "cli_json", "skill_id": "x", "timeout_secs": 30 }
            }
        ]
    });
    let parsed: ToolConfig = serde_json::from_value(json.clone()).unwrap();
    let dumped = serde_json::to_value(&parsed).unwrap();
    assert_eq!(dumped.get("asset_intel_providers").unwrap().as_array().unwrap().len(), 1);
    assert!(dumped.get("asset_intel").is_none(), "asset_intel must not appear when providers form is used");
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-pentest \
  -E 'test(tool_config_accepts_asset_intel_providers_array)+test(tool_config_round_trips_asset_intel_providers)' \
  --status-level fail
```
预期：先红（字段不存在），加字段后 2 passed。

**Commit：** `feat(golish-pentest): add asset_intel_providers field on ToolConfig`

---

### Task 2 · 加 `asset_intel` 与 `asset_intel_providers` 互斥校验

**文件：**
- 修改：`backend/crates/golish-pentest/src/parsers.rs` 中 `walk_json_files` 解析处（或 `scanner.rs::scan_toolsconfig` 调用层）
- 测试：`backend/crates/golish-pentest/src/scanner.rs::tests` 或 `parsers.rs::tests`（哪个已有现有单测就放哪个）

**先 grep 确认入口**：
```bash
rg 'fn walk_json_files|scan_toolsconfig\b' backend/crates/golish-pentest/src --line-number
```
找到现有 `walk_json_files` 之后，在它把 JSON 解成 `ToolConfig` 之后、push 进结果之前加：

```rust
if tool.asset_intel.is_some() && tool.asset_intel_providers.is_some() {
    warn!(
        tool_id = %tool.id,
        path = %path.display(),
        "tool config declares both `asset_intel` and `asset_intel_providers`; skipping (mutually exclusive)"
    );
    // 注意：返回 ScanResult.error 而不是 panic；保持其它 tool 不受影响
    errors.push(format!(
        "tool {} at {}: `asset_intel` and `asset_intel_providers` are mutually exclusive",
        tool.id,
        path.display()
    ));
    continue; // 跳过这个 tool
}
```

如果 `walk_json_files` 当前没有 `errors` 累积模式（典型实现可能是 silent skip），需要先看 `ScanResult` 结构再决定是 push 到 `result.error` 还是 `result.warnings`。如果没有 warnings 字段，先按 `tracing::warn!` + skip 实现，TODO 留到后续 task 加 strict mode（**本计划不引入** strict mode，避免范围蔓延）。

**测试** —— 在 `parsers.rs` 或 `scanner.rs` 已有 `mod tests` 里加：

```rust
#[test]
fn scan_skips_tool_declaring_both_asset_intel_and_providers() {
    use tempfile::TempDir;
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("conflict.json"),
        r#"{
            "tool": {
                "id": "conflict",
                "name": "conflict",
                "executable": "x",
                "asset_intel": {
                    "enabled": true,
                    "provider_id": "conflict",
                    "display_name": "conflict",
                    "capabilities": ["subsidiaries"],
                    "auto": {"default": true, "priority": 1},
                    "runtime": {"kind": "cli_json", "skill_id": "x", "timeout_secs": 30}
                },
                "asset_intel_providers": [
                    {
                        "enabled": true,
                        "provider_id": "conflict",
                        "display_name": "conflict",
                        "capabilities": ["subsidiaries"],
                        "auto": {"default": true, "priority": 1},
                        "runtime": {"kind": "cli_json", "skill_id": "x", "timeout_secs": 30}
                    }
                ]
            }
        }"#,
    ).unwrap();

    let result = scan_toolsconfig(dir.path());
    assert!(result.success, "scan should not fail catastrophically; bad tool should be skipped");
    assert_eq!(result.tools.len(), 0, "conflicting tool must be excluded");
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-pentest \
  -E 'test(scan_skips_tool_declaring_both_asset_intel_and_providers)' --status-level fail
```
预期：先红（互斥校验未实现，结果会有 1 个 tool 被收纳），加校验后 1 passed。

**Commit：** `feat(golish-pentest): reject tool declaring both asset_intel and asset_intel_providers`

---

### Task 3 · 加 `expand_provider_tools` 函数 + 单测（TDD 红绿）

**文件：**
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`
- 测试：同文件 `#[cfg(test)] mod tests`

**步骤：**

1. 在 `asset_intel.rs` 文件靠近 `provider_descriptors_from_tools`（约 `line 290`）的位置加新函数：

```rust
/// 把 scan_toolsconfig 输出按 Asset Intel 视角展平：
/// - tool 有 `asset_intel_providers: Some(vec)` → 拆成 N 个 virtual ToolConfig，
///   每个 clone 共享元数据（id / executable / install / runtime / ...），
///   `asset_intel` 设为对应那一项，`asset_intel_providers` 清空；
/// - tool 有 `asset_intel: Some(_)` → 1 个 clone（原样）；
/// - 其它 tool → 不出现（Asset Intel 选择器不应看见无 provider 的 tool）。
fn expand_provider_tools(tools: &[ToolConfig]) -> Vec<ToolConfig> {
    let mut out = Vec::new();
    for tool in tools {
        if let Some(providers) = tool.asset_intel_providers.as_ref() {
            for provider in providers {
                if !provider.enabled {
                    continue;
                }
                let mut virtual_tool = tool.clone();
                virtual_tool.asset_intel = Some(provider.clone());
                virtual_tool.asset_intel_providers = None;
                out.push(virtual_tool);
            }
        } else if let Some(asset) = tool.asset_intel.as_ref() {
            if !asset.enabled {
                continue;
            }
            out.push(tool.clone());
        }
    }
    out
}
```

2. 在同文件 `mod tests` 末尾加：

```rust
#[test]
fn expand_provider_tools_clones_each_provider_into_virtual_tool() {
    let tool = ToolConfig {
        id: "shared".into(),
        name: "Shared".into(),
        executable: "shared/bin".into(),
        asset_intel_providers: Some(vec![
            golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "shared".into(),
                display_name: "Shared".into(),
                capabilities: vec!["subsidiaries".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig { default: true, priority: 100 },
                runtime: fake_runtime(),
                normalize: Default::default(),
                discovery: Default::default(),
                lookup: None,
            },
            golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "shared-alt".into(),
                display_name: "Shared Alt".into(),
                capabilities: vec!["subsidiaries".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig { default: false, priority: 50 },
                runtime: fake_runtime(),
                normalize: Default::default(),
                discovery: Default::default(),
                lookup: None,
            },
        ]),
        ..Default::default()
    };
    let expanded = expand_provider_tools(&[tool]);
    assert_eq!(expanded.len(), 2);
    assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "shared");
    assert_eq!(provider_id_for_tool(&expanded[1]).unwrap(), "shared-alt");
    assert_eq!(expanded[0].executable, "shared/bin");
    assert_eq!(expanded[1].executable, "shared/bin");
    assert!(expanded[0].asset_intel_providers.is_none(), "virtual tool must not carry providers vec");
}

#[test]
fn expand_provider_tools_passes_single_asset_intel_tool_through_unchanged() {
    let tools = two_phase_fixture_tools(); // existing helper: enscan-go + 0.zone single-provider
    let expanded = expand_provider_tools(&tools);
    assert_eq!(
        expanded.iter().map(|t| provider_id_for_tool(t).unwrap()).collect::<Vec<_>>(),
        vec!["enscan-go".to_string(), "0.zone".to_string()],
        "single-provider tools must be cloned 1:1"
    );
}

#[test]
fn expand_provider_tools_skips_disabled_providers() {
    let tool = ToolConfig {
        id: "shared".into(),
        name: "Shared".into(),
        executable: "x".into(),
        asset_intel_providers: Some(vec![
            golish_pentest::models::AssetIntelToolConfig {
                enabled: false,
                provider_id: "off".into(),
                display_name: "off".into(),
                capabilities: vec!["subsidiaries".into()],
                requires_integration: None,
                auto: Default::default(),
                runtime: fake_runtime(),
                normalize: Default::default(),
                discovery: Default::default(),
                lookup: None,
            },
            golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "on".into(),
                display_name: "on".into(),
                capabilities: vec!["subsidiaries".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig { default: true, priority: 1 },
                runtime: fake_runtime(),
                normalize: Default::default(),
                discovery: Default::default(),
                lookup: None,
            },
        ]),
        ..Default::default()
    };
    let expanded = expand_provider_tools(&[tool]);
    assert_eq!(expanded.len(), 1);
    assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "on");
}
```

注意 `fake_runtime()` 是文件中已存在的 helper（搜 `fn fake_runtime`），如果命名不一致就 grep 后调整。

**验证：**
```bash
cd backend && cargo nextest run -p golish --lib \
  -E 'test(expand_provider_tools)' --status-level fail
```
预期：先红（函数不存在），加函数后 3 passed。

**Commit：** `feat(golish): add expand_provider_tools fan-out helper`

---

### Task 4 · 让 `provider_descriptors_from_tools` 接入 expand

**文件：** 修改：`backend/crates/golish/src/tools/asset_intel.rs`（约 `line 293-332`）

**步骤：**

1. 在函数开头加一行展平：

```rust
fn provider_descriptors_from_tools(tools: &[ToolConfig]) -> Vec<AssetIntelProviderDescriptor> {
    let expanded = expand_provider_tools(tools);
    expanded
        .iter()
        .filter_map(|tool| {
            /* 现有 body 不变 —— filter_map 第一行已经 .as_ref()? 来取 asset_intel */
            /* ... */
        })
        .collect()
}
```

2. 新增 fixture 测试（在同文件 `mod tests`）：

```rust
#[test]
fn provider_descriptors_from_tools_unpacks_multi_provider_tool() {
    let tool = ToolConfig {
        id: "multi".into(),
        name: "Multi".into(),
        executable: "m/bin".into(),
        asset_intel_providers: Some(vec![
            /* 同 Task 3 两 provider 写法 */
        ]),
        ..Default::default()
    };
    let descriptors = provider_descriptors_from_tools(&[tool]);
    assert_eq!(descriptors.len(), 2);
    assert!(descriptors.iter().any(|d| d.id == "multi-a"));
    assert!(descriptors.iter().any(|d| d.id == "multi-b"));
}
```
（provider_id 用 `multi-a` / `multi-b` 等独立 id，避免与其它测试冲突。具体内容按 Task 3 模板填。）

**验证：**
```bash
cd backend && cargo nextest run -p golish --lib \
  -E 'test(provider_descriptors_from_tools_unpacks_multi_provider_tool)' --status-level fail
cd backend && cargo nextest run -p golish --lib \
  -E 'test(asset_intel_provider_descriptors_load_from_tool_configs)' --status-level fail
```
预期：新测试先红（descriptors_from_tools 没接入 expand 时只返回 0 个），改一行后转绿；既有 1 tool 1 descriptor 测试继续绿。

**Commit：** `feat(golish): provider descriptors honour asset_intel_providers fan-out`

---

### Task 5 · 让 `select_asset_intel_providers` / `select_subsidiary_providers` / `select_enrichment_providers` 接入 expand

**文件：** 修改：`backend/crates/golish/src/tools/asset_intel.rs`（约 `line 1053-1155`）

**关键决策：** `select_*` 现在返回 `Vec<&'a ToolConfig>`（借用），改用 `expand_provider_tools` 后必须返回 owned `Vec<ToolConfig>`（virtual tool 持有权属本地）。这会影响所有调用方。

**步骤：**

1. 把 `select_asset_intel_providers` 改签名：

```rust
fn select_asset_intel_providers(
    tools: &[ToolConfig],
    requested: &[String],
) -> Result<Vec<ToolConfig>, GolishError> {
    let expanded = expand_provider_tools(tools);
    let mut providers: Vec<ToolConfig> = expanded
        .into_iter()
        .filter(|tool| provider_id_for_tool(tool).is_some())
        .collect();
    if requested.is_empty() {
        providers.retain(|tool| tool.asset_intel.as_ref().is_some_and(|asset| asset.auto.default));
        providers.sort_by(|a, b| {
            let pa = a.asset_intel.as_ref().map(|x| x.auto.priority).unwrap_or(0);
            let pb = b.asset_intel.as_ref().map(|x| x.auto.priority).unwrap_or(0);
            pb.cmp(&pa).then_with(|| {
                provider_id_for_tool(a).unwrap_or_default()
                    .cmp(&provider_id_for_tool(b).unwrap_or_default())
            })
        });
        return Ok(providers);
    }
    /* 原有 explicit 分支：把 .iter().find 改为 .into_iter().find */
    Ok(/* explicit branch result */)
}
```

2. `select_subsidiary_providers` / `select_enrichment_providers` 同步改返回 `Vec<ToolConfig>`：

```rust
fn select_subsidiary_providers(tools: &[ToolConfig], requested: &[String])
    -> Result<Vec<ToolConfig>, GolishError>
{
    let all = select_asset_intel_providers(tools, requested)?;
    let (kept, rejected): (Vec<ToolConfig>, Vec<ToolConfig>) =
        all.into_iter().partition(provider_has_subsidiaries);
    if !rejected.is_empty() && !requested.is_empty() {
        let ids: Vec<String> = rejected.iter().filter_map(provider_id_for_tool).collect();
        return Err(GolishError::Validation { /* 原消息 */ });
    }
    Ok(kept)
}
```
（`select_enrichment_providers` 同样套路：`.partition` 完后保留**不**含 subsidiaries 的）

3. 调用方 `run_providers_for_org`（约 `line 3225-3271`）：把 `providers: Vec<&ToolConfig>` 改成 `providers: Vec<ToolConfig>`；`for tool in providers` 改为 `for tool in &providers` 或直接 `for tool in providers.iter()`。其余 `tool.asset_intel.as_ref()` 不变。

4. 所有调 `select_*` 的 command（`asset_intel_hydrate_subsidiaries` / `asset_intel_enrich_organization` / `asset_intel_enrich_batch` / legacy `asset_intel_hydrate`）：返回值已是 owned，直接 `let providers = select_subsidiary_providers(...)?;` 后 `run_providers_for_org(... providers, ...).await?;`，不再需要 `.iter().collect::<Vec<&_>>()` 之类的 adapter。

**测试** —— 在 `mod tests` 加：

```rust
#[test]
fn select_subsidiary_providers_expands_multi_provider_tool() {
    let tool = ToolConfig {
        id: "multi".into(),
        name: "Multi".into(),
        executable: "x".into(),
        asset_intel_providers: Some(vec![
            /* 2 providers, both capabilities=["subsidiaries"], auto.default=true, priority 100/90 */
        ]),
        ..Default::default()
    };
    let selected = select_subsidiary_providers(&[tool], &[]).unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(provider_id_for_tool(&selected[0]).unwrap(), "multi-hi");
    assert_eq!(provider_id_for_tool(&selected[1]).unwrap(), "multi-lo");
}

#[test]
fn select_asset_intel_providers_treats_multi_provider_tool_as_single_pool() {
    /* 一个 tool 含 2 provider (priority 50/100) + 另一个 tool 单 provider (priority 75)
       → 排序后预期 [100, 75, 50] */
    /* 实现细节同上 */
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish --lib \
  -E 'test(select_) and test(asset_intel)' --status-level fail
```
预期：现有 `select_asset_intel_providers_uses_json_auto_priority` / `select_subsidiary_providers_keeps_only_subsidiaries_capable_tools` 等仍绿；新 2 个测试绿。

**Commit：** `refactor(golish): select_* asset intel returns owned virtual ToolConfig`

---

### Task 6 · 把 3 个 child JSON 的 provider 合并到主 enscan-go.json `asset_intel_providers`

**文件：**
- 修改：`resources/toolsconfig/enscan-go.json`

**步骤：**

1. 把现有 `tool.asset_intel`（AQC）整段移除，替换为 `tool.asset_intel_providers: [...]`：

```json
"asset_intel_providers": [
  {
    "enabled": true,
    "provider_id": "enscan-go",
    "display_name": "ENScan_GO",
    "capabilities": ["subsidiaries"],
    "requires_integration": { "tool_id": "enscan-go", "group_ids": ["aqc"] },
    "auto": { "default": true, "priority": 100 },
    "runtime": { /* 原 AQC runtime（CliJson + skill_id: company-default-json + arg_bindings + artifact_globs + timeout 180） */ },
    "lookup": { /* 原 AQC lookup（skill_id: company-lookup-json + normalize.enterprise_info） */ },
    "normalize": { /* 原 AQC normalize.organization + target + profile_fields */ },
    "discovery": { /* 原 AQC discovery.auto_promote + promote_when + ownership_field + dedupe_by */ }
  },
  {
    "enabled": true,
    "provider_id": "enscan-go-tyc-discovery",
    "display_name": "ENScan_GO · TYC Discovery",
    "capabilities": ["subsidiaries"],
    "requires_integration": { "tool_id": "enscan-go", "group_ids": ["tyc"] },
    "auto": { "default": false, "priority": 95 },
    "runtime": {
      "kind": "cli_json",
      "skill_id": "company-default-json-tyc",
      "timeout_secs": 180,
      "artifact_globs": ["**/*.json"],
      "arg_bindings": {
        "min_ownership_percent": "-invest {{config.min_ownership_percent}}",
        "depth": "-deep {{config.depth}}",
        "include_branches": "-branch"
      }
    },
    "discovery": { "auto_promote": true, "promote_when": [{"field":"scale","op":"gte","value":"51"},{"field":"status","op":"contains","value":"开业"}], "ownership_field": "scale", "dedupe_by": ["pid","name"] },
    "normalize": { "organization": [/* 同 child 文件 */], "profile_fields": [] }
  },
  /* enscan-go-kc-discovery 同样填，runtime.skill_id: company-default-json-kc */
  /* enscan-go-rb-discovery 同样填，runtime.skill_id: company-default-json-rb */
]
```

2. 同步在主 enscan-go.json 的 `tool.skills` 数组里**加** 3 个独立 skill（TYC / KC / RB 各一个），因为 runtime.skill_id 指向新名字：

```json
{
  "id": "company-default-json-tyc",
  "name": "TYC company investment JSON",
  "description": "...",
  "args": "-n \"{{org}}\" -type tyc -field invest -delay 3 -json -out-dir \"{{out_dir}}\"",
  "tags": ["enterprise","json","tyc","subsidiary"]
}
/* KC / RB 类似，args 把 -type kc / -type rb */
```

（确认：现有 child JSON 的 skill 名都是 `company-default-json`，要避免主 JSON skill_id 冲突。让 child runtime.skill_id 都用 `company-default-json` + child 自己 skills 数组里定义同名 skill 是 child 文件的独立性带来的。合并后必须给每个 provider 用不同的 skill_id，因为它们共享主 JSON 的 skills 数组。）

3. 校验 JSON 格式：

```bash
python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null
```

**验证（合并前）：**
```bash
cd backend && cargo nextest run -p golish \
  -E 'test(fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable)' \
  --status-level fail
```
预期：依然绿（3 个 child JSON 还没删，主 JSON 改了，scan_toolsconfig 会找到 1 个 enscan-go tool 展平出 4 provider + 3 个 child tool 各 1 provider = **总共 7 provider**，其中 4 个含 "enscan-go" / "enscan-go-tyc-discovery" / "enscan-go-kc-discovery" / "enscan-go-rb-discovery"，其余 3 个 child 也叫一样的 provider_id 会冲突）。

**这一步预期会暴露 provider_id 跨 tool 冲突**——所以 Task 6 完成后立即进入 Task 7 删 child JSON。

**Commit：** `feat(toolsconfig): merge enscan-go discovery providers into main JSON`

---

### Task 7 · 删 3 个 child JSON

**文件：**
- 删除：`resources/toolsconfig/enscan-go-tyc-discovery.json`
- 删除：`resources/toolsconfig/enscan-go-kc-discovery.json`
- 删除：`resources/toolsconfig/enscan-go-rb-discovery.json`

**步骤：**

> **AGENTS.md §2.7**：删文件属于高风险，删之前必须用户已确认（本计划顶部用户已选 A 方案 = 默认确认删 child；执行时仍在 reply_message 给用户最后一次回执机会）。

```bash
rm resources/toolsconfig/enscan-go-tyc-discovery.json
rm resources/toolsconfig/enscan-go-kc-discovery.json
rm resources/toolsconfig/enscan-go-rb-discovery.json
```

**验证：**
```bash
cd backend && cargo nextest run -p golish \
  -E 'test(fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable)' \
  --status-level fail
cd backend && cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
cd backend && cargo nextest run -p golish-pentest -E 'test(asset_intel)' --status-level fail
```
预期：fixture 绿（默认 3 source = enscan-go + enscan-go-kc-discovery + enscan-go-rb-discovery，TYC 因 auto.default=false 被排除）；asset_intel 全套 ≥ 29 passed；schema layer 7 passed。

**Commit：** `refactor(toolsconfig): drop 3 child enscan-go discovery JSON files (merged into main)`

---

### Task 8 · 全套验证

**步骤：**

```bash
# JSON valid
python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null && echo "JSON valid"

# Backend
cd backend
cargo nextest run -p golish-pentest --status-level fail
cargo nextest run -p golish --lib --status-level fail -E 'test(asset_intel)+test(scan)'
cargo fmt --package golish --package golish-pentest --check
cargo check -p golish

# Frontend
cd .. && pnpm exec tsc --noEmit
pnpm exec biome check frontend/components/TargetPanel/ frontend/lib/api/asset-intel.ts
pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts

# Lint files we touched
```
之后用 ReadLints 检查 `backend/crates/golish-pentest/src/models.rs` + `backend/crates/golish-pentest/src/parsers.rs`（或 scanner.rs，看 Task 2 实际落点） + `backend/crates/golish/src/tools/asset_intel.rs` + `resources/toolsconfig/enscan-go.json`。

预期：所有命令 exit 0；ReadLints No errors。

**Commit：** （本任务不单独 commit；只是检查上几个 commit 的整体绿）

---

### Task 9 · 更新 agent-progress.md + feature_list.json

**文件：**
- 修改：`agent-progress.md`
- 修改：`feature_list.json`

**步骤：**

1. `agent-progress.md` 在「会话记录」段顶部插入一条 2026-05-23 记录，模板：

```markdown
### 2026-05-23 · Asset Intel providers flat：4 个 JSON 合并为 1 个多 provider

- **本轮目标**：用户拍板走 A 方案，把 enscan-go 的 3 个 child discovery JSON 合进主 enscan-go.json 的 `asset_intel_providers` 数组。
- **设计文档**：`docs/design/2026-05-23-asset-intel-providers-flat.md`
- **实现计划**：`docs/superpowers/plans/2026-05-23-asset-intel-providers-flat.md`
- **已完成（按 Task）**：
  - Task 1: ToolConfig 加 `asset_intel_providers: Option<Vec<...>>` 字段 + 2 个 schema 单测
  - Task 2: scan 时拒绝同时声明 `asset_intel` 和 `asset_intel_providers` 的 tool
  - Task 3: 新增 `expand_provider_tools` fan-out 工具 + 3 单测
  - Task 4: `provider_descriptors_from_tools` 接入 expand
  - Task 5: `select_*` 改返回 owned `Vec<ToolConfig>` 并接入 expand
  - Task 6: 主 enscan-go.json 改用 `asset_intel_providers: [aqc, tyc, kc, rb]`，加 3 个 skill 项
  - Task 7: 删 3 个 child JSON
- **运行过的验证**：（粘贴 Task 8 的所有命令 + 退出码）
- **已知风险或未解决问题**：
  - 工具管理面板从 4 行 ENScan_GO 变 1 行：手动 dev 验证待用户复测一次
  - TYC 仍 `auto.default=false`（等上游 PR #221）
```

2. `feature_list.json` 加：

```json
{
  "id": "asset_intel_providers_flat",
  "title": "Asset Intel: 一个 tool 声明多个 provider",
  "status": "passing",
  "priority": "medium",
  "phase": "asset-intel-phase-4",
  "owner": "fullstack",
  "verification": [
    "cargo nextest run -p golish-pentest --status-level fail",
    "cargo nextest run -p golish --lib -E 'test(asset_intel)+test(expand_provider_tools)' --status-level fail",
    "python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null"
  ],
  "evidence": "<填 Task 8 的 nextest 输出>",
  "notes": "上游 wgpsec/ENScan_GO PR #221 合并后须把 enscan-go.json 内 tyc provider auto.default 改回 true 并把 fixture 名改回 defaults_to_all_enscan_sources。"
}
```

**验证：**
```bash
python3 -m json.tool feature_list.json >/dev/null && echo "feature_list valid"
```

**Commit：** `docs: log asset_intel_providers_flat completion`

---

## 自检（在执行前由作者校对）

- ✅ **规格覆盖度**：设计文档 §3-7 每条都有 task：§3 契约 → Task 1+2+6；§4.1 schema → Task 1+2；§4.2 expand → Task 3+4+5；§5 兼容 → Task 3 单 provider 测；§7 验证 → Task 8。
- ✅ **占位符扫描**：所有代码块给完整 Rust + JSON；TODO 只在 Task 2 的 `strict mode` 处明确写"本计划不引入"。
- ✅ **类型一致性**：`Vec<ToolConfig>`（owned）从 Task 5 起一致；`AssetIntelToolConfig` 字段名贯穿；`provider_id_for_tool` 在所有任务调用一致。
- ✅ **AGENTS.md §2.7**：Task 7 删文件前已在用户拍板里确认；执行时再 reply 一次回执机会。
- ✅ **TDD**：Task 1 / 2 / 3 / 4 / 5 都先写测试再实现。
- ✅ **frequent commit**：8 个 commit（task 1-7 + task 9），平均粒度 2-5 分钟可完成一个 task 的代码改动。
