# Asset Intel Provider Abstraction

> Superseded by `docs/design/2026-05-22-asset-intel-json-driven-providers.md`.

> 日期：2026-05-22
> 状态：Draft
> 背景：`Engagement Workspace` 的 `Discover Assets` 模式需要企业情报数据源来生成子公司、域名、ICP、APP、小程序、社交账号等候选资产。当前 ENScan_GO 可以作为第一批数据源，但 Target / Engagement UI 不应与 ENScan_GO 的生命周期绑定。
> 相关文档：`docs/design/2026-05-22-engagement-workspace.md`、`docs/design/2026-05-21-integrations.md`

---

## 1. 目标

建立一个 `Asset Intel Provider` 抽象层，让 `Discover Assets` 只消费统一候选结果，不直接认识 ENScan_GO、0.zone、AQC、TYC 或其它具体工具。

核心目标：

- `TargetPanel` / `Engagement Workspace` 保持业务语义：hydrate intel、review candidates、promote to targets。
- ENScan_GO 降级为 provider adapter。以后 ENScan_GO 不维护时，只替换 adapter，不重做 Target UI。
- 所有 provider 输出统一写入 organization candidates，并保留 evidence/source。
- 主动扫描仍只看已确认 scope / targets，不因自动发现结果直接扩大授权边界。

非目标：

- 本阶段不新增 `engagements` 独立表。
- 本阶段不直接把所有发现结果写入 `targets`。
- 本阶段不在 Target UI 做 ENScan 专属表单或 ENScan 专属页面。

---

## 2. 分层边界

```text
Engagement Workspace
  └─ Asset Intel Service
       ├─ Provider Registry
       ├─ ENScan_GO Provider Adapter
       ├─ 0.zone Provider Adapter
       └─ Future Provider Adapters

Integrations
  └─ credentials / cookies / test connection / auto-capture
```

### Engagement Workspace

只负责用户流程：

- 创建 `discover_assets` engagement。
- 触发 `Hydrate intel`。
- 展示 candidates。
- 用户确认后 promote 到 child organizations 或 targets。

### Asset Intel Service

负责把“公司名 + 发现参数”编排成一个或多个 provider run，并把 provider 输出规范化为统一候选。

### Provider Adapter

负责对接具体数据源。Adapter 可以是本地 CLI、HTTP API、内置 provider、文件导入或未来自研采集器。

### Integrations

只负责凭据和连接能力。它可以告诉 provider 凭据是否存在、测试是否健康，但不决定 engagement 业务流程。

---

## 3. 核心数据契约

### Hydrate Request

```json
{
  "organization_id": "uuid",
  "company_name": "小米",
  "provider_ids": ["enscan-go"],
  "config": {
    "min_ownership_percent": "51",
    "depth": "2",
    "include_branches": true,
    "create_candidates": true
  }
}
```

`provider_ids` 为空时表示 auto mode，由后端按可用 credentials、provider capability 和项目策略选择。

### Hydrate Result

```json
{
  "run_id": "uuid",
  "status": "completed",
  "provider_status": [
    {
      "provider_id": "enscan-go",
      "status": "completed",
      "message": "42 candidates normalized"
    }
  ],
  "candidates": {
    "organizations": [],
    "targets": []
  },
  "evidence": []
}
```

第一版可以同步返回结果；若 CLI / API 运行较久，再升级为 `start/status/cancel` job 模型。

### Candidate

复用现有 `OrganizationCandidate`，但约定字段含义：

- `kind`: `organization` 或 `target`
- `label`: UI 展示名
- `value`: 去重主键候选值，例如公司名、域名、IP、URL
- `source`: provider id，例如 `enscan-go`
- `confidence`: 0 到 1
- `status`: `needs_review` / `approved` / `rejected`
- `evidence`: provider 原始记录、字段来源、run id、时间戳

候选写入当前已有位置：`organizations.intel.engagement.candidates`。

---

## 4. Provider Capability

每个 provider 需要声明能力，避免前端和服务层猜测某个工具能做什么。

```text
AssetIntelProvider
- id
- display_name
- requires_integration?: { tool_id, group_ids[] }
- capabilities:
  - subsidiaries
  - domains
  - icp
  - apps
  - mini_programs
  - social_accounts
  - contacts
- run(request) -> provider output
- normalize(output) -> candidates + evidence
```

ENScan_GO 的声明示例：

```text
id: enscan-go
display_name: ENScan_GO
requires_integration:
  tool_id: enscan-go
  group_ids: [aqc, tyc, kc, rb, miit]
capabilities:
  subsidiaries, domains, icp, apps, mini_programs, social_accounts
```

如果 ENScan_GO 不可用，registry 可以把它标记为 `unavailable` 或 `deprecated`，但 `Discover Assets` UI 仍然存在。

---

## 5. Backend API

建议新增命令：

```text
asset_intel_list_providers(project_path?) -> ProviderDescriptor[]
asset_intel_hydrate_start(args) -> AssetIntelRun
asset_intel_hydrate_status(run_id) -> AssetIntelRun
asset_intel_hydrate_cancel(run_id) -> void
```

第一阶段如果先做同步 MVP，可以只实现：

```text
asset_intel_hydrate(args) -> AssetIntelRun
```

但命名和返回结构应预留 run/job 语义，避免后续 CLI 长任务改接口。

写入路径：

1. 后端读取 organization 和 engagement config。
2. registry 选择 provider。
3. provider 执行并解析原始输出。
4. service normalize 为 `OrganizationCandidate[]`。
5. 调用现有候选 upsert 逻辑写入 `intel.engagement.candidates`。
6. 返回 run status 和候选计数。

---

## 6. Frontend Contract

`TargetPanel` 不直接调用 ENScan_GO。Discover Assets 右侧工作区只依赖：

- provider list：用于显示可选数据源和健康状态。
- hydrate command：用于触发企业情报补全。
- organization candidates API：用于展示候选。
- promote API：用于把 approved candidates 变成 child orgs 或 targets。

推荐 UI：

```text
Scope & Intel
- Hydrate intel
- Providers: Auto / ENScan_GO / 0.zone / AQC Direct
- Last run status
- Candidate organizations
- Candidate targets
- Promote selected
```

ENScan_GO 只应该出现在 provider 选择器和 Settings / Integrations 中，不应成为工作区名称或页面结构。

---

## 7. Evidence 与授权边界

自动发现结果默认是 candidate，不是 scope。

规则：

- Provider 输出必须带 source 和 evidence。
- Candidate 默认 `needs_review`。
- Promote 到 target 时才进入可测试资产集合。
- Active scan 必须读取 target scope / organization scope rules，不能只因为 `discover_assets` mode 自动放行。
- “已检查为空”和“未检查”必须区分：provider run 成功但无结果，应记录为 checked-empty evidence；provider 未运行则保持 unchecked。

---

## 8. ENScan_GO Adapter 第一版

ENScan_GO adapter 是第一个 provider，用于验证抽象层。

输入：

- company name
- selected source groups
- discovery depth / ownership hints

执行：

- 通过 integrations 找到 ENScan_GO executable 和 config。
- 执行只读企业情报命令。
- 解析 stdout / exported artifact。
- 规范化为 organization candidates 和 target candidates。

输出：

- 子公司 / 分支机构 -> `kind: organization`
- 域名 / ICP / app domain / mini-program URL -> `kind: target`
- 原始 ENScan 字段 -> `evidence.raw`

限制：

- ENScan 输出字段不稳定时，adapter 内部兜底，不能把不稳定字段泄漏到 Target UI。
- Cookie 失效只让 provider run failed，不影响 Engagement 页面渲染。

---

## 9. 实施阶段

### Phase 1 · 服务抽象

- 新增 provider descriptor 和 hydrate request/result 类型。
- 新增 provider registry。
- 新增 ENScan_GO provider skeleton。
- 新增 `asset_intel_hydrate` 后端命令。

验收：给定 mock provider，可生成 candidates 并写入现有 `organization_candidates_upsert` 路径。

### Phase 2 · ENScan_GO 接入

- 调用 ENScan_GO CLI。
- 解析输出并 normalize。
- provider status 体现 missing credential / expired cookie / command failed。

验收：真实公司名 hydrate 后出现候选组织和候选 targets；失败时 UI 可解释原因。

### Phase 3 · Frontend Discover Assets 闭环

- Target workspace 接 `Hydrate intel`。
- 显示 provider status 和候选列表。
- 支持 candidate approve/reject。

验收：未 approve 的候选不会进入 active scan；approved candidate 可 promote。

### Phase 4 · 多 provider

- 接入第二个 provider，例如 0.zone 或 direct AQC。
- auto mode 合并多个 provider 输出并去重。

验收：Target UI 不变，只多出 provider 来源和 evidence。

---

## 10. 设计原则

- UI 绑定业务流程，不绑定工具。
- Provider 输出必须 normalize 后才能进入 engagement。
- 候选先复核，再变成 scope。
- 凭据归 Integrations，发现编排归 Asset Intel Service。
- 工具可替换，evidence 和 candidate contract 保持稳定。
