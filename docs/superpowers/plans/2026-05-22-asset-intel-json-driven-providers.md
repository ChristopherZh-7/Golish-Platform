# Asset Intel JSON-Driven Providers 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划；实现代码前遵守 TDD，先写失败测试。

**目标：** 把 Asset Intel provider registry、runtime 和 normalize 从 Rust 硬编码改为 toolsconfig JSON 驱动，让新增 CLI provider 只需要新增或修改 JSON。  
**架构：** `asset_intel` 保留 IPC 和 candidate 写入业务契约，但 provider 描述从 `tool.asset_intel` 加载。Rust 提供 `cli_json` runtime、descriptor validator、candidate normalizer 和 auto mode selector。  
**技术栈：** Rust Tauri command、serde JSON schema、existing `golish_pentest::scan_toolsconfig`、existing `OrganizationCandidate` upsert、Vitest/TypeScript frontend wrapper 回归。

## 文件结构

- `backend/crates/golish-pentest/src/models.rs`：给 `ToolConfig` 增加 `asset_intel: Option<AssetIntelToolConfig>`，定义可反序列化的 descriptor 类型。
- `backend/crates/golish/src/tools/asset_intel.rs`：删除 provider 专属常量和分支，改为 descriptor loader、selector、generic runtime、generic normalizer。
- `resources/toolsconfig/enscan-go.json`：新增 `tool.asset_intel`，把 provider metadata、auto mode、CLI skill、artifact globs、normalize mappings 放入 JSON。
- `docs/design/2026-05-22-asset-intel-provider-abstraction.md`：标记被新设计取代。
- 旧 `docs/superpowers/plans/2026-05-22-asset-intel-provider-abstraction.md`：后续已删除，避免后续 agent 继续执行硬编码 Phase 4。
- `frontend/lib/api/asset-intel.ts`：原则上不变；只在 Rust IPC 类型变更时同步。
- `frontend/components/TargetPanel/TargetGroupedView.tsx`：原则上不变；只跑回归验证。

## 任务 1：给 toolsconfig schema 加 Asset Intel descriptor

**文件：** `backend/crates/golish-pentest/src/models.rs`

1. 先写反序列化测试，放在 `models.rs` 现有 tests 模块中，覆盖最小 descriptor：

```rust
#[test]
fn tool_config_accepts_asset_intel_descriptor() {
    let raw = r#"{
        "id": "fake-intel",
        "name": "Fake Intel",
        "executable": "fake",
        "runtime": "native",
        "launchMode": "cli",
        "asset_intel": {
            "enabled": true,
            "provider_id": "fake-intel",
            "display_name": "Fake Intel",
            "capabilities": ["domains"],
            "requires_integration": {
                "tool_id": "fake-intel",
                "group_ids": ["default"]
            },
            "auto": { "default": true, "priority": 50 },
            "runtime": {
                "kind": "cli_json",
                "skill_id": "company-default-json",
                "timeout_secs": 30,
                "artifact_globs": ["**/*.json"],
                "arg_bindings": { "org": "{{company_name}}" }
            },
            "normalize": {
                "target": [
                    { "path": "$..domains[*]", "label": "domain", "value": "domain", "confidence": 0.8 }
                ]
            }
        }
    }"#;
    let tool: ToolConfig = serde_json::from_str(raw).unwrap();
    let asset = tool.asset_intel.expect("asset_intel descriptor");
    assert_eq!(asset.provider_id, "fake-intel");
    assert_eq!(asset.capabilities, vec!["domains"]);
    assert_eq!(asset.auto.priority, 50);
}
```

2. 运行：

```bash
cargo test -p golish-pentest tool_config_accepts_asset_intel_descriptor --lib
```

预期先红灯，原因是 `ToolConfig` 没有 `asset_intel` 字段。

3. 增加类型：

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIntelToolConfig {
    #[serde(default = "default_true_asset_intel")]
    pub enabled: bool,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub requires_integration: Option<AssetIntelIntegrationRequirement>,
    #[serde(default)]
    pub auto: AssetIntelAutoConfig,
    pub runtime: AssetIntelRuntimeConfig,
    #[serde(default)]
    pub normalize: AssetIntelNormalizeConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIntelIntegrationRequirement {
    pub tool_id: String,
    #[serde(default)]
    pub group_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIntelAutoConfig {
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssetIntelRuntimeConfig {
    CliJson {
        skill_id: String,
        #[serde(default = "default_asset_intel_timeout_secs")]
        timeout_secs: u64,
        #[serde(default)]
        artifact_globs: Vec<String>,
        #[serde(default)]
        arg_bindings: std::collections::HashMap<String, String>,
    },
}
```

4. 增加 `AssetIntelNormalizeConfig` 和 `AssetIntelNormalizeRule`，字段为 `path`, `label`, `value`, `confidence`。`label` 和 `value` 用 untagged enum 支持 string 或 string array。

5. `ToolConfig` 增加字段：

```rust
#[serde(default, rename = "asset_intel", skip_serializing_if = "Option::is_none")]
pub asset_intel: Option<AssetIntelToolConfig>,
```

6. 再运行同一测试，预期通过。

## 任务 2：把 ENScan provider 描述移入 JSON

**文件：** `resources/toolsconfig/enscan-go.json`

1. 在 `tool` 对象下增加 `asset_intel`，复用已有 `skills.company-default-json` 作为 runtime 起点。

2. Descriptor 内容必须表达当前行为：

```json
"asset_intel": {
  "enabled": true,
  "provider_id": "enscan-go",
  "display_name": "ENScan_GO",
  "capabilities": ["subsidiaries", "domains", "icp", "apps", "mini_programs", "social_accounts"],
  "requires_integration": {
    "tool_id": "enscan-go",
    "group_ids": ["aqc", "tyc", "kc", "rb", "miit"]
  },
  "auto": { "default": true, "priority": 100 },
  "runtime": {
    "kind": "cli_json",
    "skill_id": "company-default-json",
    "timeout_secs": 180,
    "artifact_globs": ["**/*.json"],
    "arg_bindings": {
      "org": "{{company_name}}"
    }
  },
  "normalize": {
    "organization": [
      { "path": "$..invest[*]", "label": "name", "value": "name", "confidence": 0.82 },
      { "path": "$..holds[*]", "label": "name", "value": "name", "confidence": 0.82 },
      { "path": "$..branch[*]", "label": "name", "value": "name", "confidence": 0.78 }
    ],
    "target": [
      { "path": "$..icp[*]", "label": "domain", "value": "domain", "confidence": 0.78 },
      { "path": "$..app[*]", "label": "name", "value": ["link", "app_url", "name"], "confidence": 0.68 },
      { "path": "$..wx_app[*]", "label": "name", "value": ["app_id", "name"], "confidence": 0.68 },
      { "path": "$..wechat[*]", "label": "name", "value": ["wechat_id", "id", "name"], "confidence": 0.62 },
      { "path": "$..weibo[*]", "label": "name", "value": ["weibo_id", "id", "name"], "confidence": 0.62 }
    ]
  }
}
```

3. 运行：

```bash
python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null
```

预期 exit 0。

## 任务 3：descriptor loader 替换硬编码 provider list

**文件：** `backend/crates/golish/src/tools/asset_intel.rs`

1. 先写测试：用临时 toolsconfig dir 写一个只有 `asset_intel` 的 fake JSON，调用 loader 后返回一个 `AssetIntelProviderDescriptor`。

2. 新增纯函数：

```rust
fn provider_descriptors_from_tools(tools: &[golish_pentest::models::ToolConfig]) -> Vec<AssetIntelProviderDescriptor>
```

映射规则：

- `asset_intel.enabled == false` 跳过。
- `provider_id` 为空时使用 `tool.id`。
- `display_name` 为空时使用 `tool.name`。
- capabilities string 转成现有 `AssetIntelCapability`，未知值跳过并记录 validation evidence。
- requires integration 从 JSON 直接映射。
- status 初始为 `Available`。

3. `asset_intel_list_providers` 改为读取 `PentestState.config_manager` 的 `toolsconfig_dir`，调用 `scan_toolsconfig`，再用 loader 返回 descriptor。

4. 运行：

```bash
cargo test -p golish asset_intel_provider_descriptors --lib
cargo check -p golish
```

预期 provider descriptor 测试通过，`cargo check` exit 0。

## 任务 4：generic JSON normalizer 替换 ENScan/0.zone 字段分支

**文件：** `backend/crates/golish/src/tools/asset_intel.rs`

1. 先写测试：给 fake descriptor normalize mapping 和 raw JSON，输出 organization + target candidates。

2. 实现：

```rust
fn normalize_json_with_descriptor(
    provider_id: &str,
    run_id: &str,
    fetched_at: u64,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    raw: &serde_json::Value,
) -> OrganizationCandidates
```

3. JSON path 第一版支持当前 ENScan 需要的形态：

- `$..field[*]`：递归找任意对象属性 `field` 下的数组。
- `field`：从当前 object 取 string。
- `["field_a", "field_b"]`：按顺序取第一个非空 string。

4. 删除或停止调用 `collect_enscan_records`、`zone_records_to_provider_records`、`parse_enscan_json_records` 这一类 provider 专属 normalize 函数。

5. 运行：

```bash
cargo test -p golish asset_intel --lib
```

预期 normalize 相关测试通过。

## 任务 5：generic `cli_json` runtime 替换 ENScan 命令构建

**文件：** `backend/crates/golish/src/tools/asset_intel.rs`

1. 先写测试：fake tool 带 skill args `-n "{{org}}" -json -out-dir "{{out_dir}}"`，runtime 渲染后包含公司名和 temp output dir。

2. 实现：

```rust
fn render_asset_intel_skill_args(
    skill_args: &str,
    company_name: &str,
    out_dir: &std::path::Path,
    config: &AssetIntelHydrateConfig,
) -> String
```

支持 token：

- `{{org}}` 和 `{{company_name}}`
- `{{out_dir}}`
- `{{config.min_ownership_percent}}`
- `{{config.depth}}`
- `{{config.include_branches}}`

3. `run_cli_json_provider` 流程：

- 找 tool descriptor。
- 找 `runtime.skill_id` 对应 `tool.skills[].args`。
- resolve executable。
- 渲染 args。
- 执行命令并收集 stdout、stderr、artifact JSON。
- 对每个 JSON document 调 `normalize_json_with_descriptor`。

4. 删除 `build_enscan_command_plan` 和 `run_enscan_go_provider` 的专属逻辑。

5. 运行：

```bash
cargo test -p golish asset_intel --lib
cargo check -p golish
```

预期 exit 0。

## 任务 6：auto mode 从 JSON 选择 provider

**文件：** `backend/crates/golish/src/tools/asset_intel.rs`

1. 先写测试：三个 fake providers，两个 `auto.default=true`，验证按 `priority` 降序或升序稳定选择。项目采用高 priority 先执行。

2. 实现：

```rust
fn select_asset_intel_providers<'a>(
    tools: &'a [ToolConfig],
    requested: &[String],
) -> Result<Vec<&'a ToolConfig>, GolishError>
```

规则：

- `requested` 非空：只返回 provider_id 命中的 descriptor；未命中返回 NotFound。
- `requested` 为空：返回 `auto.default == true` 的 descriptor，按 `auto.priority` 从高到低排序，再按 provider id 稳定排序。

3. 删除 `vec![ENSCAN_PROVIDER_ID, ZONE_PROVIDER_ID]`。

4. 运行：

```bash
cargo test -p golish asset_intel --lib
```

预期 provider selector 测试通过。

## 任务 7：移除 0.zone Rust 专属 Asset Intel 分支

**文件：** `backend/crates/golish/src/tools/asset_intel.rs`

1. 删除 Asset Intel 模块中对 `ZoneProvider`, `IntelProvider`, `ProviderRecord`, `QueryType` 的直接 import。

2. 删除 `run_zone_provider` 和 `read_provider_api_key`，Asset Intel 不再直接查询 `vault_entries`。

3. 保留 `golish-intel-providers` crate 给 Integrations Test Connection 使用，不再从 Asset Intel 直接调用它。

4. 如果没有 `0.zone` JSON descriptor，本轮 `asset_intel_list_providers` 不返回 0.zone。等 `http_json` runtime 实现后再通过 JSON 接入。

5. 运行：

```bash
cargo check -p golish
```

预期 exit 0，且 `asset_intel.rs` 内搜索不到 `ZoneProvider`。

## 任务 8：前端回归验证

**文件：** `frontend/lib/api/asset-intel.ts`、`frontend/components/TargetPanel/TargetGroupedView.tsx`

1. 不改前端 API，确认 Rust serde 输出仍匹配当前 TS 类型。

2. 运行：

```bash
pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts
pnpm exec tsc --noEmit
pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts
```

预期 vitest 全绿、tsc exit 0、biome no fixes。

## 任务 9：文档和进度收尾

**文件：** `agent-progress.md`、`docs/design/2026-05-22-asset-intel-provider-abstraction.md`

1. 旧设计文档顶部增加：

```markdown
> Superseded by `docs/design/2026-05-22-asset-intel-json-driven-providers.md`.
```

2. 旧计划文档后续已删除，避免后续 agent 继续执行硬编码 Phase 4。

3. `agent-progress.md` 新增一条记录，说明本次发现的硬编码问题、替代方案、运行过的验证命令。

4. 运行 scoped 验证：

```bash
python3 -m json.tool resources/toolsconfig/enscan-go.json >/dev/null
cargo test -p golish-pentest tool_config_accepts_asset_intel_descriptor --lib
cargo test -p golish asset_intel --lib
cargo check -p golish
pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts
pnpm exec tsc --noEmit
```

## 总体验收

- `backend/crates/golish/src/tools/asset_intel.rs` 不再包含 ENScan_GO 或 0.zone 的 provider id 常量。
- `asset_intel_list_providers` 来自 toolsconfig JSON。
- `asset_intel_hydrate` 的 auto mode 来自 JSON `auto.default` 和 `auto.priority`。
- ENScan 的 CLI 参数和 normalize mapping 存在于 `resources/toolsconfig/enscan-go.json`。
- 增加 fake JSON-only provider 测试，证明新增 provider 不改 Rust。
- Target UI 无行为回退。
