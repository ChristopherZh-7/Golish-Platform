# Asset Intel Providers Flat

> 日期：2026-05-23
> 状态：Draft
> Supersedes 部分: `docs/design/2026-05-22-asset-intel-json-driven-providers.md` §4（保留其余 normalize/runtime/lookup 契约）

## 1. 问题

`docs/design/2026-05-22-asset-intel-json-driven-providers.md` 把 Asset Intel Provider 设计成「每个 `tool.asset_intel` descriptor = 1 个 provider」。当一个 tool（如 ENScan_GO）的同一可执行文件需要按不同凭据组（AQC / TYC / KC / RB）暴露成多个独立 provider 时，目前的实现要求拆成多个 `tool` JSON 文件：

```
resources/toolsconfig/enscan-go.json                    (AQC)
resources/toolsconfig/enscan-go-tyc-discovery.json      (TYC)
resources/toolsconfig/enscan-go-kc-discovery.json       (KC)
resources/toolsconfig/enscan-go-rb-discovery.json       (RB)
```

带来的副作用：

| 问题 | 现象 |
|---|---|
| 工具管理面板 UX 误导 | 同一个 `enscan-v2.0.5-darwin-amd64` 二进制显示成 4 个独立 Tool entry；install / uninstall / version 检测都按 4 个跑 |
| JSON 字段大量重复 | 3 个 child JSON 重复 `executable / runtime.kind / discovery.promote_when / normalize.organization` |
| 凭据 vs provider 分散 | cookies 在主 `enscan-go.json` 的 `integration.groups`，provider 行为在 child JSON；定位 TYC 问题要翻两个文件 |
| Tool 模型概念混淆 | `Tool` 概念本应代表"可执行文件 + 安装方式"，被滥用为"provider 注册条目" |

## 2. 目标

让**一个** `tool` 能在自身 JSON 里声明**多个** Asset Intel Provider，保持现有「每个 provider 独立 capabilities / requires_integration / auto / runtime / normalize」语义。

不改：

- Provider Adapter 抽象（CLI / HTTP）契约本身
- 候选规范化输出（OrganizationCandidate / target candidate / profile_entry）
- 前端业务流（hydrate_subsidiaries / enrich_organization / enrich_batch）
- Engagement Workspace 业务面板（仍按 provider_id 渲染来源徽章）

## 3. JSON 契约（新增字段）

`tool.asset_intel` 的现有定义保留（单 provider 形式向后兼容），**新增** `tool.asset_intel_providers` 数组：

```json
{
  "tool": {
    "id": "enscan-go",
    "executable": "ENScan_GO/enscan-v2.0.5-darwin-amd64",
    "integration": { "groups": [/* aqc, tyc, kc, rb, miit */] },
    "asset_intel_providers": [
      {
        "enabled": true,
        "provider_id": "enscan-go",
        "display_name": "ENScan_GO",
        "capabilities": ["subsidiaries"],
        "requires_integration": { "tool_id": "enscan-go", "group_ids": ["aqc"] },
        "auto": { "default": true, "priority": 100 },
        "runtime": { "kind": "cli_json", "skill_id": "company-default-json", ... },
        "discovery": { "auto_promote": true, ... },
        "normalize": { "organization": [...] },
        "lookup": { ... }
      },
      {
        "provider_id": "enscan-go-tyc-discovery",
        "display_name": "ENScan_GO · TYC Discovery",
        "capabilities": ["subsidiaries"],
        "requires_integration": { "tool_id": "enscan-go", "group_ids": ["tyc"] },
        "auto": { "default": false, "priority": 95 },
        "runtime": { "kind": "cli_json", "skill_id": "company-default-json-tyc", ... },
        "discovery": { "auto_promote": true, ... },
        "normalize": { "organization": [...] }
      },
      /* kc, rb 同样 */
    ]
  }
}
```

### 互斥性

同一个 tool **不允许**同时声明 `asset_intel` 和 `asset_intel_providers`，descriptor loader 必须拒绝（返回带 tool.id 的明确错误）。

### 校验规则

| 规则 | 错误码（serde 层） |
|---|---|
| 同一 tool 内 `providers[*].provider_id` 必须唯一 | "duplicate provider_id within tool {tool.id}" |
| 跨 tool 的 `provider_id` 全局唯一 | "duplicate provider_id across tools: {provider_id}" |
| `providers[]` 为空 = 等价于无 `asset_intel`（loader 忽略） | （无错，treat as None） |
| `provider_id` 非空字符串 | 走原 `provider_id_for_tool` fallback：若空则用 `{tool.id}::{index}` 作 unique key（仅给单 provider 形式留路） |

## 4. Rust 改造点（按层）

### 4.1 Schema (`backend/crates/golish-pentest/src/models.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolConfig {
    /* ... existing fields ... */
    #[serde(default, rename = "asset_intel", skip_serializing_if = "Option::is_none")]
    pub asset_intel: Option<AssetIntelToolConfig>,

    /// 多 provider 形式：一个 tool 声明 N 个 Asset Intel Provider，每个 provider
    /// 共享 tool 的 executable / install 元数据但拥有独立的 capabilities / runtime /
    /// requires_integration / normalize / auto / discovery / lookup。与 `asset_intel`
    /// 互斥。
    #[serde(
        default,
        rename = "asset_intel_providers",
        skip_serializing_if = "Option::is_none"
    )]
    pub asset_intel_providers: Option<Vec<AssetIntelToolConfig>>,
}
```

互斥校验在 `golish-pentest::parsers` 里 `walk_json_files` 解析后做（loader 层而非 serde 层，便于给出文件级错误）。

### 4.2 Provider 展平 (`backend/crates/golish/src/tools/asset_intel.rs`)

新增工具函数：

```rust
/// 把 scan_toolsconfig 输出按 Asset Intel 视角展平：
/// - tool 有 `asset_intel_providers` → 拆成 N 个 virtual ToolConfig，每个 clone 共享元数据，
///   `asset_intel` 设为对应那一项，`asset_intel_providers` 清空；
/// - tool 有 `asset_intel`（单 provider） → 1 个 virtual tool（原样 clone）；
/// - 其它 tool → 不出现在结果中（避免 select_* 误判）。
fn expand_provider_tools(tools: &[ToolConfig]) -> Vec<ToolConfig>;
```

所有 Asset Intel 选择/调用入口都先 `expand_provider_tools` 再走：

| 入口 | 现状 | 新行为 |
|---|---|---|
| `provider_descriptors_from_tools` | iter `tools` | iter `expand_provider_tools(tools)` |
| `select_asset_intel_providers` | iter `tools` | iter `expand_provider_tools(tools).iter()` |
| `select_subsidiary_providers` | 同上 | 同上 |
| `select_enrichment_providers` | 同上 | 同上 |
| `run_providers_for_org` 调用前 | `select_*` 返回 `Vec<&ToolConfig>` | `select_*` 返回 owned `Vec<ToolConfig>`（virtual 拷贝持有所有权） |

**关键**：`run_cli_json_provider` / `run_http_json_provider` 的签名不变（仍收 `&ToolConfig`），它们看到的就是 virtual tool，`tool.asset_intel.as_ref()` 拿到的就是对应那个 provider 的 descriptor。

### 4.3 工具管理面板

无影响：工具管理面板用的是 `scan_toolsconfig` 的原始输出，看到的还是 1 个 `enscan-go` tool entry。Asset Intel 展平只发生在 Asset Intel 自己的入口里。

## 5. 向后兼容

| 旧 tool | 现状 | 新行为 |
|---|---|---|
| 0.zone（单 provider, 用 `asset_intel`） | 1 provider | 1 provider（`expand_provider_tools` 直接 clone） |
| `enscan-go-tyc-discovery.json` 等 child 文件（迁移期暂留） | 1 provider | 1 provider（同上） |
| 主 `enscan-go.json` 旧用 `asset_intel` | 1 provider | 1 provider |
| 主 `enscan-go.json` 改用 `asset_intel_providers: [...]` | n/a | N providers |

迁移策略：先加 schema 字段 + expand 函数 + fixture（红）→ 改主 JSON（绿）→ **最后一步**删 3 个 child JSON（避免中间态测试断）。

## 6. 影响面

| 影响域 | 是否动 | 备注 |
|---|---|---|
| 后端 schema | ✅ | 加 1 字段 + 互斥校验 |
| 后端 provider 展平 | ✅ | 加 1 函数 + 改 4 个入口调用 |
| 后端 provider 调用入口 | ❌ | run_cli/http_json_provider 签名不变 |
| 前端 IPC | ❌ | provider_id 字段语义不变 |
| 前端 UI | ❌ | provider 列表渲染照旧 |
| 工具管理面板 | ❌ | scan_toolsconfig 输出不变 |
| 凭据/integration | ❌ | requires_integration 仍按 group_ids 走 |
| 已写入 DB 的 evidence/run 记录 | ❌ | provider_id 保持稳定 |

## 7. 验证

### 7.1 单元层

- `expand_provider_tools` 单测：含/不含 `asset_intel_providers` 都正确展平
- `provider_descriptors_from_tools` 单测：多 provider tool 展开成 N 个 descriptor
- `select_asset_intel_providers` 单测：跨 tool 与 tool 内多 provider 都按 priority 排序

### 7.2 Fixture 层

- `fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable`：断言不变（默认仍是 enscan-go + kc + rb），只是 expand 后才能拿到这 3 个
- 新增 `fixture_enscan_main_json_declares_multiple_providers`：断言主 enscan-go.json 的 expand 结果包含 4 个 provider_id

### 7.3 集成层

- JSON 格式：`python3 -m json.tool resources/toolsconfig/enscan-go.json`
- ts-rs 类型同步：本改动**不涉及**前端 IPC schema，无需 ts-rs（确认 `provider_descriptors_from_tools` 输出的 `AssetIntelProviderDescriptor` 字段不变）
- 端到端：手动跑 just dev → 工具管理面板显示 1 个 ENScan_GO + Asset Intel 配置面板显示 4 个 provider

## 8. 后续清理（计划文件外）

- 3 个 child JSON 删除（计划最后一个 task）
- `agent-progress.md` 把"当前最高优先级"切到本计划的 in_progress
- `feature_list.json` 加一条 in_progress
