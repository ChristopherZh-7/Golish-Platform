# Asset Map 直接落 Target 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 `recon_map_assets` 将本轮 provider 发现的合法 domain/IP 去重后直接落为组织 Target，并删除 TargetPanel 的人工资产候选工作区。
**架构：** 保留 current-run candidate DTO 作为 provider 归一化中间态，新增纯 planner 生成 domain/IP landing plan，复用 org/type/exact-value scoped upsert 写 Target、DNS 和 service/subdomain 关系。Enrich 不再持久化 target candidate JSON；subsidiary organization review 通道保持兼容。
**技术栈：** Rust 2021、sqlx/Postgres、React 19、TypeScript 6、Vitest、Biome、cargo nextest。

## 文件结构

- 修改 `backend/crates/golish-recon-app/src/asset_intel/landing.rs`：current-run domain/IP planner、实际 landing summary、DNS/upsert 顺序与单元测试。
- 修改 `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`：调用 direct landing、调整顺序/计数/零写入错误、强制 enrich 不写 candidate queue。
- 修改 `backend/crates/golish-recon-app/src/asset_intel/commands.rs`：单组织 enrich 接入 `enrichment_hydrate_config`。
- 修改 `backend/crates/golish-recon-app/src/organization_recon/runner.rs`：asset stage 强制关闭 target candidate persistence。
- 修改 `backend/crates/golish-recon-app/src/agent_tools/mod.rs`：`recon_map_assets` phase-aware 关闭 candidate queue，保留 subsidiaries review。
- 修改 `backend/crates/golish-agent-kit/src/harness/org_gate.rs` 与 `task_orchestrator/subtask_phases/execute.rs`：Target Intel 资产轴按 stage-start cutoff 冻结。
- 修改 `backend/crates/golish-agent-app/src/ai/{harness_submit_tool.rs,commands/stage_coverage.rs}`：submit preview 与 coverage read model 使用同一冻结轴。
- 修改/删除 `frontend/components/TargetPanel/*`、`frontend/lib/target-panel/*` 和 i18n：删除候选 tab、人工 promote 状态和陈旧产品文案。
- 修改 `resources/harness/stages/target_intel/{methodology.md,spec.json}` 与模块卡：同步 direct handoff 合同。

## 任务 1：先锁住 current-run landing 合同

**文件：** `backend/crates/golish-recon-app/src/asset_intel/landing.rs`

**步骤：**

1. 新增失败测试 `plan_current_run_targets_promotes_domains_and_ips_without_authorized_roots`，输入重复大小写域名、URL host、IP candidate 和一对多 `HostIpPair`，断言得到 canonical domain/IP exact identities，domain primary IP 稳定且 IP 全部独立存在。
2. 新增失败测试 `plan_current_run_targets_rejects_wildcard_and_malformed_values`，断言 wildcard/空值/无效 host/CIDR 不落。
3. 运行红灯：

```bash
cd backend && cargo nextest run -p golish-recon-app plan_current_run_targets --status-level fail
```

预期：测试因 planner 行为缺失失败，exit 101。

4. 实现 `CurrentRunTarget` 与 `plan_current_run_targets(...)`，只接 current-run 三类输入，使用 canonical exact `(type,value)` 去重。
5. 复用 `upsert_target` 实现 `land_current_run_targets(...) -> TargetLandingSummary`；先写 domain/IP target，再写所有 provider DNS edges。已有 `scope=out` 不翻转。
6. 重新运行上面的 nextest，预期相关测试通过。

## 任务 2：接入 agent landing 并修正真实计数

**文件：** `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`

**步骤：**

1. 新增失败测试 `summary_distinguishes_observed_and_landed_targets`：JSON 中 `observedTargets=45`、`targets=0` 必须同时可见，`targets` 不再代表 observation count。
2. 将 Enrich 执行顺序改为：抽 pairs/observations → direct target+DNS landing → subdomain relation landing → service landing。
3. 使用 current-run domain 集合构造 relation-only organization view，使 subdomain/service 校验只消费本轮发现，不依赖 pre-existing root。
4. `PassiveIntelSummary` 增加 `observedTargets`、`landedDomains`、`landedIps`、`dnsRecords`、`serviceAssets`、`subdomainAssets`；`targets` 改为实际 Target upsert/reuse 数。
5. 只有实际业务行大于 0 才增加 `golish-landing found`；有 observations/pairs 但实际 landing 为 0 时设置 `Partial + error`。
6. 验证：

```bash
cd backend && cargo nextest run -p golish-recon-app -E 'test(plan_current_run_targets) | test(summary_distinguishes_observed_and_landed_targets) | test(pairs_from_candidates) | test(service_assets)' --status-level fail
```

预期：全部通过。

## 任务 3：停止 durable asset target candidate 写入

**文件：**

- `backend/crates/golish-recon-app/src/agent_tools/mod.rs`
- `backend/crates/golish-recon-app/src/asset_intel/commands.rs`
- `backend/crates/golish-recon-app/src/organization_recon/runner.rs`
- `backend/crates/golish-recon-app/src/asset_intel/tests.rs`

**步骤：**

1. 先补 phase/config 接线测试：Enrich 固定 `create_candidates=false`，Subsidiaries 的 organization review 输入仍可保留。
2. `recon_map_assets`、single-org enrich 和 organization-recon asset stage 在调用 provider runner 前统一应用 `enrichment_hydrate_config`。
3. 保留 `AssetIntelRun.candidates` 返回值、OrganizationCandidate DTO/commands 和 subsidiary organizations bucket。
4. 验证：

```bash
cd backend && cargo nextest run -p golish-recon-app -E 'test(enrichment_config_disables_candidate_queue_writes) | test(enrich_organization_config_disables_candidate_queue_writes)' --status-level fail
```

预期：配置与实际入口测试通过。

## 任务 4：删除 TargetPanel 人工候选产品路径

**文件：**

- 删除 `frontend/components/TargetPanel/CandidateReviewList.tsx`
- 修改 `frontend/components/TargetPanel/OrgWorkspacePanel.tsx`
- 修改 `frontend/components/TargetPanel/TargetGroupedView.tsx`
- 修改 `frontend/components/TargetPanel/OrgTreeSidebar.tsx`
- 修改 `frontend/components/TargetPanel/NewEngagementDialog.tsx`
- 修改 `frontend/lib/target-panel/{types.ts,asset-intel.ts,engagement.ts}`
- 修改 `frontend/components/TargetPanel/{TargetGroupedView.actions.test.ts,NewEngagementDialog.test.tsx}`
- 修改 `frontend/lib/i18n/{zh-CN.json,en.json}`

**步骤：**

1. 先把自动路由测试改为成功后留在 `activity`，运行并确认旧实现返回 `candidates` 的红灯。
2. 删除 `WorkspaceTab="candidates"`、固定 tab、CandidateReviewList 渲染、approve/reject/promote state/handlers 与 `review_scope -> candidates` 假入口。
3. 删除 New Engagement 的 `createCandidates` state/checkbox；hydrate config 固定不创建 target candidates。
4. 清理只服务该 UI 的 helper 和中英文文案；保留 `listOrganizationCandidates` 及 AskHuman subsidiary reader。
5. 验证：

```bash
pnpm exec vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/components/TargetPanel/NewEngagementDialog.test.tsx frontend/components/AIChatPanel/AskHumanInline.test.tsx frontend/components/AIChatPanel/ScopeReviewTable.test.tsx
pnpm typecheck
pnpm exec biome check frontend/components/TargetPanel frontend/lib/target-panel frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json
```

预期：TargetPanel 与 subsidiary ask_human 回归全部通过，类型与 Biome 无错误。

## 任务 5：同步合同并做定向回归

**文件：**

- `resources/harness/stages/target_intel/methodology.md`
- `resources/harness/stages/target_intel/spec.json`
- `docs/modules/backend/golish-recon-app/asset_intel.md`
- `docs/modules/backend/golish-recon-app/organization_recon.md`
- `docs/modules/frontend/components.md`
- `docs/modules/INDEX.md`
- `feature_list.json`
- `agent-progress.md`

**步骤：**

1. 把“provider observation 只进候选/不得成为 IP target”改成 current-run direct domain/IP handoff；保留 active-scan approval、scope-out 和 cross-org 隔离说明。
2. 先补 `target_intel_freezes_its_asset_axis_at_stage_start` 与 `target_intel_coverage_excludes_targets_created_by_the_current_stage` 红灯；随后让 final gate、submit preview、subtask gate 与 coverage UI 都使用 stage-start cutoff，新落 Targets 只进入 EAS handoff。
3. 记录红灯/绿灯命令、实际退出码和未运行 `init/precommit` 的原因，feature 保持 `in_progress`。
4. 运行定向后端回归与 Clippy：

```bash
cd backend && cargo fmt -p golish-recon-app -- --check
cd backend && cargo nextest run -p golish-recon-app asset_intel --status-level fail
cd backend && cargo clippy -p golish-recon-app --all-targets -- -D warnings
python3 -m json.tool resources/harness/stages/target_intel/spec.json
python3 -m json.tool feature_list.json
git diff --check
```

预期：全部 exit 0。按用户要求不运行 `./init.sh` 或全量 `just precommit`，也不发起真实 provider/API 请求。
