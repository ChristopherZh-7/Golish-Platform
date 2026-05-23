# Asset Intel · Two-Phase Hydrate（discovery + enrichment 拆分）

> 状态：accepted · 关联 feature `asset-intel-hydrate-disambiguation`（Phase D）
> 关联文档：
> - `docs/design/2026-05-22-asset-intel-json-driven-providers.md`
> - `docs/design/2026-05-22-asset-intel-provider-abstraction.md`
> - 实施计划：`docs/superpowers/plans/2026-05-22-asset-intel-two-phase-hydrate.md`

## 1. 背景与问题

当前 `asset_intel_hydrate` 是**单层 for 循环顺跑所有 provider**，每个 provider 输入同一个 `company_name`（主公司名）：

- enscan-go（capability `subsidiaries` + domains/icp/apps/...）拿主公司名查企查查/天眼查 → 抽出子公司、域名、app 等
- 0.zone（capability `domains` + `apps` + `contacts`，**没有 subsidiaries**）**同样**拿主公司名 POST 到 `0.zone/api/data/`

用户反馈：「0.zone 不应该跟 enscan-go 同步搜，0.zone 是第二步——给 enscan 找到的每家子公司补字段用的。」

## 2. 设计目标

把 hydrate intel 拆成两个语义清晰的阶段：

| 阶段 | 目的 | provider 选择 | 候选去向 |
|---|---|---|---|
| **Discovery** | 找出子公司 | provider.capabilities 含 `subsidiaries` | 主公司的 `organization_candidates` 表（用户审核 → Promote 为正式子公司） |
| **Enrichment** | 给单个 organization 补字段（域名/APP/联系方式/credit_code 等） | provider.capabilities **不含** `subsidiaries` | 该 organization 自己的 `organization_candidates` + master profile（profile_fields 自动写） |

并提供「主公司一键批量补字段」入口：对主公司 + 所有子公司（`parent_id = parent_organization_id` 的 organization）逐个 enrichment。

## 3. IPC 契约

```
asset_intel_hydrate_subsidiaries
Req: { organizationId: string, companyName?: string, providerIds?: string[], config?: AssetIntelHydrateConfig }
Res 200: AssetIntelRun  // candidates 主要是 kind=organization 的子公司候选
Res 404: organization 不存在 / 不属于 caller
Res 400: 没有任何 subsidiaries-capable provider 可用
权限: caller 必须能访问 organizationId (复用 organizations::get_one)
说明: 仅运行 provider.capabilities 含 'subsidiaries' 的 provider；profile_fields 仍按既有规则写入主公司主档案。
```

```
asset_intel_enrich_organization
Req: { organizationId: string, providerIds?: string[], config?: AssetIntelHydrateConfig }
Res 200: AssetIntelRun  // candidates 是该 org 的 domains/apps/contacts，profile_fields 写该 org 自己的 profile
Res 404: organization 不存在
Res 400: 没有 enrichment provider 可用
权限: 同上
说明: 仅运行 provider.capabilities 不含 'subsidiaries' 的 provider；company_name 取 org.name；不再用主公司名。
```

```
asset_intel_enrich_batch
Req: { parentOrganizationId: string, includeSelf?: boolean = true, providerIds?: string[], config?: AssetIntelHydrateConfig }
Res 200: { runs: AssetIntelRun[], skipped: { organizationId: string, reason: string }[] }
Res 404: parentOrganization 不存在
Res 400: 既无 self 也无子公司 / 没有 enrichment provider
权限: 父 org 必须能访问；子 org 通过 parent_id 过滤同 project 自动同权
说明: 顺序对 [parent (if includeSelf), ...children] 各跑一次 asset_intel_enrich_organization 路径；
     失败 / 部分失败不阻塞后续；汇总返回每条的状态。
```

错误码沿用现有 `GolishError`（Validation / NotFound / Internal）。

## 4. 数据模型

**不改 schema**。candidates 仍写入 `organization_candidates`（按 `organization_id` 关联）。
- discovery 候选写主公司的 organization_id
- enrichment 候选写**被 enrich 的 organization 自己的** id

## 5. 兼容性

- 原 `asset_intel_hydrate` **保留不删**（默认 unchanged 行为：跑所有 provider）。
  - 标 deprecated 注释；前端默认不再调用，但旧 evidence / 旧前端 cache 不会炸。
  - 后续 PR 可彻底移除。
- enscan-go 同时具备 `subsidiaries` + `domains/apps/...` 多 capability —— **discovery 阶段已经会拿这部分 candidate**，无需重复跑。Enrichment 阶段只跑非 subsidiaries provider（即 0.zone 等）。

## 6. 端到端流程

```
用户场景：刚创建「中国平安」discover_assets engagement

Step 1 · 用户点【查子公司】(hydrate_subsidiaries)
  → enscan-go 跑 -n 中国平安 → 抽出 invest/holds/branch
  → candidates 写主公司：org candidates = [平安银行, 平安证券, 平安人寿, ...]
                       target candidates = [www.pingan.com, ...]（enscan 顺便抓的）

Step 2 · 用户在 Candidates 面板逐条 Approve + Promote
  → 平安银行 / 平安证券 ... 成为主公司的 child organization（parent_id=主公司）

Step 3 · 用户点主公司【批量补字段】(enrich_batch, includeSelf=true)
  → 对主公司 + 每个子公司顺序跑 enrich_organization：
     · 主公司      → 0.zone (query=中国平安) → 补主公司的域名/APP/联系方式
     · 平安银行    → 0.zone (query=平安银行) → 补平安银行的域名/APP/联系方式
     · 平安证券    → 0.zone (query=平安证券) → 补平安证券的域名/APP/联系方式
     · ...
  → 每家 organization 自己的 candidate 列表 + profile 都被填充

Step 4 · 用户对单家 organization 点【补字段】(enrich_organization)
  → 单独再跑一次 enrichment，无需走 batch
```

## 7. UI 改动概要

`TargetGroupedView.tsx` OrgActionItem：
- 老：`hydrate_intel`（一键）
- 新：
  - `hydrate_subsidiaries`（labeled「查子公司」）— 主公司 + 任何 organization 行可见
  - `enrich_batch`（labeled「批量补字段」）— 只在主公司行可见
  - `enrich_organization`（labeled「补字段」）— 子公司行可见

streaming 事件 (`assetIntel.listenStream`) 仍复用现有的 `provider_started/progress/batch/completed`；前端 UI 只需要按 organizationId 隔离 activity 即可。

## 8. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 0.zone API quota 在批量 enrich 时被打爆 | enrich_batch 是用户主动触发；P1 给出 dry-run 预估调用次数（next PR） |
| 子公司还没 promote 就跑 enrich_batch → batch 为空 | UI 提示「先把子公司 Approve+Promote 后再跑批量补字段」 |
| enscan-go 自己也有 domains/apps capability —— 跑 discovery 时已抽 → 不应再在 enrichment 阶段重复跑 | 通过 capability 集合的 disjoint 划分自然解决（enscan-go 有 subsidiaries → 只在 discovery 跑；0.zone 无 subsidiaries → 只在 enrichment 跑） |
| 既有 `asset_intel_hydrate` 仍存在 → 用户/前端可能调错 | 前端默认 UI 不暴露老命令；后端打 deprecated 注释 + 一行 tracing::warn |

## 9. 验证策略

- 后端单测：
  1. `select_subsidiary_providers` 只返 capabilities 含 subsidiaries 的 provider
  2. `select_enrichment_providers` 只返 capabilities 不含 subsidiaries 的 provider
  3. enrich_batch 顺序 + 失败不阻塞 + skipped 汇总
- 前端 vitest：
  1. 主公司行渲染「查子公司」+「批量补字段」按钮
  2. 子公司行渲染「补字段」按钮（不显示「批量补字段」）
  3. 点击「批量补字段」调 enrichBatch 并传 parentOrganizationId + includeSelf=true
- 用户 E2E：
  1. just dev → 创建中国平安 → 点【查子公司】→ candidates 出来都是子公司名 + enscan 顺带的域名
  2. Approve+Promote 3 家子公司 → 子公司行显示
  3. 主公司点【批量补字段】→ 看 activity 流；查 organization profile / candidates
  4. 单家子公司点【补字段】→ 看候选只属于该子公司
