# Engagement Workspace · Target Manager 重设计

> 日期：2026-05-22
> 状态：Draft
> 背景：当前 `Target Manager` 已经同时承载 organization tree、target list、asset intel、security testing，但 UI 仍以“target 管理”为中心，导致客户给定目标、自动资产发现、纯组织建档三种流程在创建后缺少清晰差异。
> 相关文档：`docs/design/2026-05-20-pentest-fields-tool-mapping.md`、`docs/design/2026-05-20-asm-intel-providers.md`

---

## 0. 目标

把现有 `Target Manager` 升级为以 **Engagement / 客户项目** 为中心的工作区，让用户能清楚回答四个问题：

1. 当前项目的客户主体是谁？
2. 当前 scope 是客户给定的，还是系统发现后待确认的？
3. 哪些内容是组织情报，哪些才是真正可测的 target？
4. 当前下一步应该导入目标、补全情报、确认 scope，还是开始 recon？

第一版不要求重做数据库 schema。优先复用现有：

- `organizations`：客户主体、子公司、组织画像、scope rules、intel
- `targets`：域名 / IP / URL / CIDR 等可测试资产
- `organization_id`：target 到客户主体的归属
- `organizations.intel.engagement`：短期存储 engagement mode 和入口参数

---

## 1. 概念边界

### 1.1 Engagement

一次客户委托或一次测试项目。当前可以先不新增独立表，用 root organization + `intel.engagement` 表达。

长期如果需要多个客户、多阶段授权、多轮复测，再新增 `engagements` 表。

### 1.2 Organization

公司 / 客户 / 子公司 / 分支机构。它不是扫描对象，而是资产和授权的归属主体。

典型字段：

- Basic：别名、行业、信用代码、优先级
- Scope：授权规则、排除规则、测试窗口
- Intel：工商、ICP、APP、小程序、联系人、子公司、证据

### 1.3 Scope

授权边界。scope 可以来自：

- 客户直接给定目标清单
- 企业情报发现后的候选资产
- 手工添加

只有确认进入 scope 的资产才能转成可测试 target 或进入主动扫描。

### 1.4 Target

真正可测试对象：domain / ip / cidr / url / wildcard。

target 应该始终能追溯：

- 属于哪个 organization
- 来源是什么：`customer_provided` / `asm_discovered` / `manual` / `tool:<name>`
- 当前 scope 状态
- recon 字段补全状态

---

## 2. 三种入口模式

### 2.1 Customer Targets · 客户已提供目标

适用场景：渗透测试项目中，客户已经给了域名、IP、URL、CIDR 清单。

默认行为：

1. 创建 root organization。
2. 批量导入客户给定 targets。
3. targets 直接关联到 root organization。
4. target source 标记为 `customer_provided`。
5. 默认进入 `Targets & Testing` 工作区。

主操作：

- Import targets
- Review scope
- Start recon

不应默认做：

- 递归查子公司
- 自动扩大 scope
- 主动扫描未确认资产

### 2.2 Discover Assets · 自动发现资产

适用场景：客户只给公司名，或要求做攻击面梳理 / ASM。

默认行为：

1. 创建 root organization。
2. 保存发现参数：数据源、持股阈值、递归深度、是否包含分支机构。
3. 调用企业情报数据源，例如 ENScan_GO / AQC / TYC / MIIT / 0.zone。
4. 发现子 organization、域名、业务系统、社交账号、候选 targets。
5. 默认进入 `Scope & Intel` 工作区。

主操作：

- Hydrate intel
- Review candidates
- Promote to targets

不应默认做：

- 把所有发现结果直接变成 in-scope target
- 跳过候选确认直接主动扫描

### 2.3 Org Profile Only · 只创建组织档案

适用场景：先建档，后续再决定导入目标或发现资产。

默认行为：

1. 只创建 root organization。
2. 记录 `engagement.mode = profile_only`。
3. 默认进入 `Overview` 工作区。

主操作：

- Import targets
- Discover assets
- Edit profile

---

## 3. 前端信息架构

### 3.1 页面命名

短期仍可保留 `Target Manager` 名称，但 UI 文案应逐步转向：

```text
Engagement Workspace
  Targets
  Security Testing
```

如果保留顶部 tab，`Target Manager` tab 内部应显示 engagement summary，而不是只显示 target count。

### 3.2 推荐布局

```text
┌──────────────────────────────────────────────────────────────┐
│ Engagement Workspace                 New Engagement / Import │
├──────────────────────────────────────────────────────────────┤
│ Left Tree                      │ Right Workspace              │
│                                │                              │
│ ▾ 中国平安  customer scope      │ Overview / Scope & Intel /   │
│   ├─ Targets (12)              │ Targets & Testing            │
│   └─ Scope candidates (0)      │                              │
│                                │ Mode-aware summary            │
│ ▾ 某子公司  discovery           │ Next actions                  │
│   ├─ Targets (3)               │ Coverage / Tables / Drawers   │
│   └─ Candidates (8)            │                              │
└──────────────────────────────────────────────────────────────┘
```

左侧树负责定位主体，右侧工作区负责完成任务。

### 3.3 右侧工作区

右侧不是永远打开同一个 `OrgProfileDrawer`。它应按 mode 和用户选择展示不同默认内容。

#### Customer Targets 默认右侧

- Summary：客户给定 scope、target 数、in/out 比例
- CTA：Import more targets、Start recon、Review scope
- Table：当前 targets 列表，展示 type、value、scope、status、recon coverage

#### Discover Assets 默认右侧

- Summary：发现模式、持股阈值、递归深度、数据源、最近运行状态
- CTA：Hydrate intel、Review candidates、Promote selected to targets
- Panels：Organization tree candidates、Domains、Business systems、Field coverage

#### Profile Only 默认右侧

- Summary：空档案状态
- CTA：Import targets、Discover assets、Edit profile
- Empty checklist：还缺客户目标、scope、联系人、授权窗口等

---

## 4. 左侧树行设计

当前树行 hover actions 过于通用。应拆成两组：

1. Mode primary actions：跟当前 org 的工作流有关。
2. Management actions：Info / Edit / Delete 等通用管理。

### 4.1 Customer Targets 行

Badge：`customer scope`

Primary actions：

- Import targets
- Review scope

Management：

- Info
- Edit
- Delete

### 4.2 Discover Assets 行

Badge：`discovery`

Primary actions：

- Hydrate intel
- Add child org / Review candidates

Management：

- Info
- Edit
- Delete

### 4.3 Profile Only 行

Badge：`profile`

Primary actions：

- Choose next step
- Import targets

Management：

- Info
- Edit
- Delete

---

## 5. New Engagement 弹窗

当前前端 spike 的方向可以保留，但弹窗应明确是创建 engagement 的入口，而不是简单创建 org。

### 5.1 Step 1 · Workflow

三张选择卡：

- Customer Targets
- Discover Assets
- Org Profile Only

### 5.2 Step 2 · Organization

共同字段：

- Organization name
- Owner / contact
- Notes / engagement description

### 5.3 Step 3A · Customer Targets

字段：

- targets textarea
- mark as in-scope
- link to organization
- optional passive enrichment

提交结果：

- create organization
- batch create targets
- write `intel.engagement.mode = customer_targets`

### 5.4 Step 3B · Discover Assets

字段：

- data sources
- min ownership percent
- depth
- include branches
- create candidates

提交结果：

- create organization
- write `intel.engagement.mode = discover_assets`
- enqueue or prepare hydrate job（后续阶段）

### 5.5 Step 3C · Profile Only

提交结果：

- create organization
- write `intel.engagement.mode = profile_only`

---

## 6. 数据契约

短期用 `organizations.intel.engagement` 表达入口语义：

```json
{
  "engagement": {
    "mode": "customer_targets",
    "source": "customer_provided",
    "target_count": 12,
    "min_ownership_percent": "51",
    "depth": "2",
    "include_branches": true,
    "create_candidates": true,
    "created_at": 1779370000000
  }
}
```

约束：

- `mode` 只能是 `customer_targets` / `discover_assets` / `profile_only`。
- `target_count` 是 UI summary，不作为事实来源；真实数量仍由 targets 表计算。
- Discover 参数用于恢复 UI 和 hydrate job，不等价于授权。
- active scan 必须看 `scope_rules` / target scope，不看 engagement mode。

---

## 7. 后端能力缺口

### P0

- `target_batch_add` 支持 `organizationId` 和 source 标记，避免逐条 `target_add`。
- `organization_update_profile` 支持保留已有 `intel` 并 patch `intel.engagement`，避免覆盖 provider 写入的 intel records。
- 新增 `asset_intel_hydrate` 编排命令：输入 org_id + discovery config，输出 job_id / run status。

### P1

- scope candidate 数据结构：记录候选 target / subsidiary / evidence / confidence / decision。
- candidate promote API：把选中的候选变成 targets。
- field coverage API：返回 Basic / Domains / Network / Scope / Other 的填充情况。

### P2

- engagement 独立表：支持一个项目多个客户主体、多阶段授权、复测记录。
- evidence ledger：每个自动填充字段可追溯来源、原始记录、时间、置信度。

---

## 8. 实施计划

### Phase 1 · UI 语义修正

- New Engagement 弹窗三模式。
- 左侧 org 行 mode badge。
- 左侧 org 行 mode-aware primary actions。
- 右侧 workspace skeleton：Overview / Scope & Intel / Targets & Testing。

验收：

- 三种模式创建后树上可区分。
- 点击不同 mode 的 primary action 进入不同右侧默认视图。
- 不需要后端 hydrate 编排器即可完成客户目标导入。

### Phase 2 · Customer Targets 闭环

- 批量导入 targets 关联 organization。
- target source 标记 `customer_provided`。
- Customer Targets 默认右侧展示 targets table 和 recon CTA。

验收：

- 输入 org + 目标清单后，列表中出现 org 和 targets。
- targets 都带 organization_id。
- in/out scope 可编辑。

### Phase 3 · Discover Assets 闭环

- 接 ENScan_GO / Integrations 数据源。
- 生成 organization candidates 和 target candidates。
- 用户确认后 promote to targets。

验收：

- 输入公司名 + 阈值 + 深度后能得到候选列表。
- 未确认候选不会进入 active scan。
- 已确认候选可生成 targets。

### Phase 4 · Field Coverage

- 计算 organization 字段覆盖率。
- 计算 target recon 字段覆盖率。
- 在 Overview 和 OrgProfile 中显示缺口和可执行动作。

验收：

- 用户能看到哪些字段已填、哪些缺工具、哪些需人工确认。
- Hydrate / Recon 运行后 coverage 有变化。

---

## 9. 当前前端 spike 的处理

当前已做的 `NewEngagementDialog` 和 mode-aware tree action 可作为 Phase 1 spike 保留，但后续需要：

- 把硬编码英文文案迁移到 i18n。
- 把 `Discover Assets` 从“创建并打标”接到真实 hydrate job。
- 把 `Import targets` 从 inline 单条添加升级为批量导入。
- 把 `OrgProfileDrawer` 升级或替换为右侧 `EngagementWorkspacePanel`。

---

## 10. Open Questions

1. 是否需要立刻新增 `engagements` 表，还是继续用 root organization 代表 engagement？
2. 客户给定目标是否默认全部 in-scope，还是导入后仍需 review？
3. Discover Assets 生成的子公司是否默认创建 organization 行，还是先存在候选区？
4. 右侧 workspace 是替代 drawer，还是 drawer 继续作为“编辑详情”的子视图？
5. target source 是否需要 schema 迁移，还是短期写入 `notes` / `tags` / `intel`？

---

## 11. 推荐决策

短期推荐：

1. 暂不新增 `engagements` 表，先用 root organization + `intel.engagement` 表达入口。
2. Customer Targets 默认 in-scope，但 UI 明确显示 `customer_provided`。
3. Discover Assets 的子公司和资产先进入 candidate 区，用户确认后再创建 target。
4. 右侧新增 workspace panel，`OrgProfileDrawer` 保留为“编辑全部字段”的辅助入口。
5. 下一步先做 Phase 1 + Phase 2，不要先接自动发现后端。

