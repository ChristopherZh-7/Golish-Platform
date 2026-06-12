# 红队 Phase 2：子公司发现进 scoping 实现计划

> **面向 AI 代理的工作者：** 使用 `executing-plans` 逐任务实现。每个任务先写失败测试（TDD），看它失败，再写最小实现，再验证，再 commit。
> 设计：`docs/design/2026-06-12-redteam-phase2-subsidiary-scoping.md` + 总纲 `docs/design/2026-06-12-redteam-db-truth-master.md`。

**目标：** 让红队 engagement 在 scoping 阶段确定性地完成「子公司发现 → 按持股比例阈值筛 → 母+合格子 org 落库成权威 org 树」，且 scoping gate 在 org 树真落库前不放行；`include_subsidiaries=false` 时 scoping 行为逐字节不变（零回归）。

**架构：** 底层筛选/落库能力已全部就绪（实读勘查 §0），本期只补「接线」——(B) 让 harness 子公司发现路径复用 GUI 已验证的 auto-promote（落 child org）；(C) scoping 允许 osint 工具；(A+D) engagement 参数 + scoping gate 的 `GOLISH-INTEL-SUBSIDIARY` 技术维度（照 Phase 0 DB 真值 gate 模式）。engagement 参数只进 **gate 决策路径**，不透传到工具（工具行为由 toolsconfig 静态控制）。

**技术栈：** Rust 2021、`golish-recon-app`（asset_intel）、`golish-db`（coverage_truth/evidence_facts）、`golish-agent-kit`（harness gate / orchestrator / technique_resolver）、`golish`（CLI / stage_run）、`resources/harness/stages/scoping.json`、`cargo nextest` / `clippy -D warnings`。

---

## 0. 现状勘查（2026-06-12 实读，动手前已核实）

**底层能力已就绪（复用，不重写）：**
- `golish-recon-app/src/asset_intel/promote.rs::auto_promote_child_decisions`（纯函数）：按 `ownership_percent`（ENScan `invest[].scale`）+ 阈值 + active status（开业/存续）+ dedupe 产出 promote/skip 决策。阈值取自 `policy.promote_when` 的 `>=/>/==` filter（`discovery_policy_threshold`）。`AutoPromoteSkipReason` 区分 `OwnershipBelowThreshold / MissingOwnership / InactiveStatus / Duplicate / PolicyFilterFailed`——**天然支持 I8「跑了→筛掉」vs「没跑」**。
- `golish-recon-app/src/asset_intel/service/hydrate.rs::auto_promote_discovered_children`：把 promote 的候选 `organizations::create(..., Some(parent.id), ...)` 落成 **child org**（写 `intel.asset_intel_discovery.ownershipPercent`），并 `clear_engagement_candidates_for_org`。
- `organizations.parent_id`（`migrations/20260517194500_organizations_table.sql:14`，自引用 FK `ON DELETE CASCADE`）——**org 树 schema 已存在，无需新 migration**。
- `resources/toolsconfig/enscan-go.json`：5 个 discovery provider（aqc/tyc/kc/rb）全部 `discovery.auto_promote=true`、`promote_when:[{scale gte 51},{status contains 开业}]`、`ownership_field:scale`、`dedupe_by:[pid,name]`；`output.fields` 映射 `invest.scale→ownership_percent`、`invest.name→subsidiary`。`-invest {{config.min_ownership_percent}}` arg_binding 已存在。
- `tool_taxonomy.rs:52`：`enscan_go|enscan|0.zone → ("recon","osint")`——子公司发现工具的 tool_type 是 `recon/osint`。
- expected_techniques 动态注入 seam **已 live**：`execute.rs::gate_expected_techniques(stage, target_types)` → `GateContext.expected_techniques`；`technique_resolver::stage_baseline(Scoping)` 当前返回**空**（scoping coverage_complete no-op）。
- gate DB 真值注入模式（Phase 0/1）：`execute.rs::fetch_evidence_facts_for_gate` → `repo.db_truth_facts(org_id, assets)` 注入 `Found` 哨兵 fact（id=0）；资产轴 `fetch_in_scope_assets_for_gate` → `repo.in_scope_assets(org_id)`（targets.scope='in'）。
- 透传脚手架模板：`orchestrator.rs` 的 `harness_org_id: Option<Uuid>`（pub(super) 字段 + `set_harness_org_id` setter + `new()` None）；`stage_run/mod.rs:349` `orchestrator.set_harness_org_id(org_id)` 灌入。

**真实缺口（本期补）：**
- **G1**：`asset_intel/agent_intel.rs::run_passive_intel(Subsidiaries)` 跑完**从不调** `auto_promote_discovered_children`（只 `run_providers_for_org` 产 candidates）。对比 GUI `commands.rs::asset_intel_hydrate_subsidiaries:259` 有 `if discovery_policy.auto_promote { auto_promote_discovered_children(...) }`。→ harness 路径子公司不落 child org。
- **G2**：harness 路径用 `AssetIntelHydrateConfig::default()`，不传 `min_ownership_percent`（本期用 toolsconfig 默认 51，engagement 覆盖列后续增强）。
- **G3**：`scoping.json` `allowed_tool_types:[]`、`gate_rules:[]` → 无门槛驱动子公司发现。
- **G5**：scoping 无 `GOLISH-INTEL-SUBSIDIARY` coverage 维度 → 不跑子公司发现也能 PASS。

**关键架构简化（实读结论）：** auto-promote 开关在 toolsconfig `discovery.auto_promote`（已 true），阈值在 `promote_when`（已 scale>=51）。所以 **engagement 参数（include_subsidiaries / threshold）只需进 gate 决策（要不要求子公司发现），不需要透传到工具**——工具拿不到 orchestrator 上下文（签名只有 args+workspace），也不需要拿。

---

## 1. 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `golish-recon-app/src/asset_intel/agent_intel.rs` | `run_passive_intel` 在 Subsidiaries phase 后复用 discovery policy 调 auto_promote（G1/G2） | 改 |
| `golish-recon-app/src/asset_intel/mod.rs`（或 service mod） | 确保 `auto_promote_discovered_children` 在 crate 内可达（已 pub(crate)） | 只读核对 |
| `resources/harness/stages/scoping.json` | `allowed_tool_types += recon/osint`（G3）；条件化 `gate_rules` 含 coverage_complete + authoritative（G5）；`expected_techniques` 留空靠 hook 动态注入 | 改 |
| `golish-db/src/repo/coverage_truth.rs` | `TECH_SUBSIDIARY` 常量 + org 级 `has_subsidiary_children` 查询 + `TruthInputs` 加字段 + `assemble_truth_facts` 投影（G5） | 改 |
| `golish-db/src/repo/coverage_truth/tests` | SUBSIDIARY 维度 assemble 单测 | 改 |
| `golish-agent-kit/src/harness/evidence_facts.rs` | enscan/subsidiary 命令 outcome → `GOLISH-INTEL-SUBSIDIARY` 的 Empty 派生（跑了→0 合格，I8） | 改 |
| `golish-agent-kit/src/harness/technique_resolver.rs` | `stage_baseline(Scoping)` 在「要求子公司」时含 `GOLISH-INTEL-SUBSIDIARY`（经签名/ctx 传 flag） | 改 |
| `golish-agent-kit/src/task_orchestrator/orchestrator.rs` | `harness_subsidiary_policy: Option<SubsidiaryScopePolicy>` 字段 + setter（A，照 harness_org_id） | 改 |
| `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | gate hook：`include_subsidiaries` 时给 scoping 注入 SUBSIDIARY expected technique + 资产轴（A+D） | 改 |
| `golish-agent-kit/src/harness/stage_spec.rs` | 守卫断言：scoping 在 include 时核 SUBSIDIARY；零回归断言 | 改 |
| `golish/src/cli/args.rs` | `--include-subsidiaries` / `--subsidiary-threshold <pct>`（默认 50） | 改 |
| `golish/src/stage_run/mod.rs` | 解析 flag → `orchestrator.set_subsidiary_policy(...)` | 改 |

---

## 2. 任务分解（按「独立可过 clippy + 可单测」排序：B → C → A+D → E）

### Task B · harness 子公司发现落 child org（G1/G2，最核心、最独立）

**文件：** `golish-recon-app/src/asset_intel/agent_intel.rs`

**做法（复用 GUI 路径已验证逻辑）：** `run_passive_intel` 在 `phase == Subsidiaries` 且选中 provider 有 `discovery.auto_promote=true` 时，run 后调 `auto_promote_discovered_children(pool, &org, &run.candidates, &policy)`，policy 取自选中 provider 的 `asset_intel.discovery`（照 `commands.rs:242-265`）。Summary 加 `promoted_children: usize`。

**步骤 B.1 — 失败测试**（`agent_intel.rs` tests 或 service 层纯函数测）：抽一个纯函数 `select_discovery_policy(&[ToolConfig]) -> Option<AssetIntelDiscoveryConfig>`（找第一个 `discovery.auto_promote` 的 provider policy），断言：选中含 auto_promote 的 provider → Some(policy)；无 → None。
**步骤 B.2 — 实现** `select_discovery_policy` + 在 `run_passive_intel` Subsidiaries 分支接 `auto_promote_discovered_children`。
**步骤 B.3 — 验证：** `cargo nextest run -p golish-recon-app asset_intel` 绿；`clippy -p golish-recon-app -- -D warnings` 零告警。
**步骤 B.4 — commit。**

> 注意：`run_passive_intel` 现也被 target_intel 的 `recon_discover_subsidiaries` 调用。接 auto_promote 后，target_intel 调它也会落 child org——这是**期望行为**（子公司本就该落库）；零回归核对：GUI/target_intel 路径行为与现 GUI hydrate 对齐即可。

### Task C · scoping 允许 osint 工具（G3）

**文件：** `resources/harness/stages/scoping.json`
**步骤 C.1：** `allowed_tool_types: []` → `["recon/osint"]`，加 `$comment` 说明（子公司发现是定义范围的 OSINT，非 probing，风险 low）。
**步骤 C.2 — 验证：** `python3 -c "import json;json.load(open('resources/harness/stages/scoping.json'))"` 合法；`cargo nextest run -p golish-agent-kit stage_spec` 绿（scoping spec 解析）。
**步骤 C.3 — commit。**

### Task A · engagement 参数 + orchestrator 透传 + CLI（gate 决策用）

**文件：** `orchestrator.rs` / `cli/args.rs` / `stage_run/mod.rs`

**步骤 A.1 — 类型 + setter**（`orchestrator.rs`，照 `harness_org_id`）：
```rust
/// Phase 2: when set, scoping gate requires subsidiary discovery (org-tree
/// landed) before PASS. `None` = legacy scoping (no subsidiary gate, zero回归).
#[derive(Debug, Clone, Copy)]
pub struct SubsidiaryScopePolicy { pub threshold_pct: u8 }

// 字段（在 harness_org_id 旁，pub(super)）
pub(super) harness_subsidiary_policy: Option<SubsidiaryScopePolicy>,
// new(): harness_subsidiary_policy: None,
pub fn set_subsidiary_policy(&mut self, p: Option<SubsidiaryScopePolicy>) {
    self.harness_subsidiary_policy = p;
}
```
**步骤 A.2 — CLI**（`cli/args.rs`）：加 `--include-subsidiaries`（bool）+ `--subsidiary-threshold <u8>`（默认 50）。
**步骤 A.3 — 透传**（`stage_run/mod.rs`，紧邻 `set_harness_org_id`）：`orchestrator.set_subsidiary_policy(include.then(|| SubsidiaryScopePolicy{threshold_pct}))`。
**步骤 A.4 — 验证：** orchestrator setter 单测 + `golish --help` 含新 flag；`cargo check -p golish` exit 0。
> A 的字段消费点在 Task D（execute.rs hook），A+D 连做避免 dead_code。

### Task D · scoping gate 的 GOLISH-INTEL-SUBSIDIARY 维度（G5，安全语义）

**文件：** `coverage_truth.rs` / `evidence_facts.rs` / `technique_resolver.rs` / `execute.rs` / `scoping.json`

**步骤 D.1 — coverage_truth SUBSIDIARY 投影**（`coverage_truth.rs`）：加 `pub const TECH_SUBSIDIARY = "GOLISH-INTEL-SUBSIDIARY";`；org 级 bool `has_subsidiary_children`（`EXISTS(SELECT 1 FROM organizations WHERE parent_id = $org)`）；`TruthInputs` 加 `has_subsidiary: bool`；`assemble_truth_facts` 在 `has_subsidiary` 时对每个 in-scope asset 产 `(asset, TECH_SUBSIDIARY)` Found。单测：有 child org → 每 asset 有 SUBSIDIARY fact；无 → 无。
**步骤 D.2 — evidence_facts Empty 派生**（`evidence_facts.rs`）：enscan/子公司发现命令 outcome 为「跑了→0 合格子」时派生 `GOLISH-INTEL-SUBSIDIARY` 的 `Empty` 事实（I8：跑了为空 ≠ 没跑）。单测：enscan 命令空输出 → Empty fact。
**步骤 D.3 — technique_resolver**（`technique_resolver.rs`）：`stage_baseline` / `resolve_expected_techniques` 增加「scoping + 要求子公司」时含 `GOLISH-INTEL-SUBSIDIARY`。机制：经 `execute.rs` hook 在 `subsidiary_policy.is_some()` 时为 scoping 注入该 expected technique（不改纯 resolver 的无条件行为，条件在 hook 层）。
**步骤 D.4 — execute.rs hook**：`gate_expected_techniques` / gate ctx 组装处，当 `self.harness_subsidiary_policy.is_some() && stage==Scoping` 时把 `GOLISH-INTEL-SUBSIDIARY` 并入 expected_techniques，并确保 scoping 也触发 `fetch_in_scope_assets_for_gate` + `fetch_evidence_facts_for_gate`（含 SUBSIDIARY 投影）。
**步骤 D.5 — scoping.json**：加 `coverage_complete` gate_rule（`derive_from_evidence:true`, `authoritative_found:true`, `authoritative_techniques:["GOLISH-INTEL-SUBSIDIARY"]`）+ found/checked_empty 证据规则。`expected_techniques` 仍留空（靠 hook 动态注入 → `include_subsidiaries=false` 时 hook 不注入 → coverage_complete no-op → 零回归）。
**步骤 D.6 — 验证：** `nextest -p golish-db -p golish-agent-kit` 绿；scoping 无 flag 时 gate 行为不变（断言测试）。
**步骤 D.7 — commit。**

### Task E · 守卫 + 静态门禁 + 活体

**步骤 E.1 — 守卫**（`stage_spec.rs`）：断言 scoping spec 在无 SUBSIDIARY expected technique 时 coverage_complete no-op（零回归护栏）。
**步骤 E.2 — 静态门禁：** `cargo fmt --check` / `clippy --all-targets -D warnings`（golish-recon-app + golish-db + golish-agent-kit + golish）/ `nextest`（同 4 crate）/ `cargo check -p golish` exit 0 / 改动的 stage JSON + toolsconfig `json.load` 合法 / `check_repo_ownership.py` 0 新违规。
**步骤 E.3 — 活体（需凭据 + 用户在场）：** `golish --stage-run --profile red_team --to scoping --include-subsidiaries --subsidiary-threshold 51 --org <有子公司母公司>` → `organizations` 出现母+合格子（parent_id）；scoping gate 在 org 树落库后才 PASS；不带 flag 时 scoping 立即 PASS（零回归）。
**步骤 E.4 — 更新 progress / feature_list / 模块卡 → commit。**

---

## 3. 决策点（master §10 + 本期）

1. **阈值默认**：CLI `--subsidiary-threshold` 默认 50；toolsconfig `promote_when` 现为 scale>=51（保持）。engagement 覆盖 toolsconfig 阈值列入后续增强（G2 增强），本期 gate 门槛只看「有没有 child org」，不在 gate 层重算阈值（筛选在 promote.rs 已做）。
2. **SUBSIDIARY 做成技术维度**：是（走 Phase 0 DB 真值 gate，最干净）。
3. **资产轴**：复用 target.scope='in'（母 org 根域名），org 级 SUBSIDIARY 事实投影到每个 in-scope asset；多 org 轴是 Phase 3。
4. **是否强制 auto-promote 落 child org（vs 留 candidates 人工 review）**：本期 harness 路径复用 toolsconfig `auto_promote=true`（与 GUI 一致）→ 自动落 child org。授权边界由 scoping `human_approval.required_before:["scope_expansion"]`（已存在）兜底。

---

## 4. DoD（完成定义）

- Task B-E 各自单测先红后绿；`just precommit` 全绿（或模块级 nextest+clippy+fmt+check 全绿并记录）。
- 零回归：`include_subsidiaries=false`（不带 flag）时 scoping `gate_rules` 动态注入为空 → coverage_complete no-op → 行为逐字节不变（断言测试 + 活体）。
- I7（child org 来自真实 ENScan 产物）/ I8（SUBSIDIARY checked_empty 需 Empty 账本事实）/ I10（无新 migration）守住。
- 证据写 `agent-progress.md`。

---

## 5. 红线对齐（AGENTS.md）

- **I2**：子公司落库经 `organizations::create(project_path, ..., Some(parent.id))` 限本 project；scoping `scope_expansion` 审批兜底授权边界。
- **I7**：findings 链路不碰；SUBSIDIARY Found 哨兵 id=0 不进 evidence_refs。
- **I8**：SUBSIDIARY「跑了→0 合格子」由 evidence_facts Empty 派生，DB 无 child org 且无账本 → not_attempted BLOCK，绝不推 checked_empty。
- **I10**：复用现有 `organizations.parent_id`，**无新 migration**。
- **§2.5/§2.7**：scoping gate 安全语义变更——用户 2026-06-12「全推 Phase 2（含 gate）」已确认授权。
- **零回归**：所有收紧门控在 `include_subsidiaries` flag（默认 false）后；不带 flag 逐字节回退旧 scoping。
