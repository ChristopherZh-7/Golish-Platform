# Stage Run 后端引擎 实现计划

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任务实现；每个任务单独 commit；改 Rust 后 `cd backend && cargo nextest run --status-level fail`，改前端后 `just check-fe`。
> 关联规格：`docs/design/2026-06-13-stage-run-fanout-design.md`。集成点勘察见该会话的 backend map。

**目标：** 新增 agent 工具 `stage_run`：主 agent 调用它 → 派出「当前阶段管理者」→ 按 org 并行起「该阶段专家」收集 → 每 org 各过各的 gate → 聚合 → 进度事件实时推前端（替掉 `__mockStageRun`）。intel 作首个接入阶段，专家=Recon。

**架构：** 复用 CLI `stage_run/mod.rs` 已验证的 per-org 逻辑（抽到 `golish-agent-kit/harness`）；`stage_run` 是 agent 工具（仿 `start_operation_tool.rs`），经 `ToolRegistry`+`BridgeToolSelection` 暴露给 task 主 agent；进度走 `AiEvent::HarnessTrace` 新增 `HarnessTraceKind::StageRunOrgProgress`；per-org gate 复用现有 `validate_stage_gate_with_context` + `coverage_truth.rs`。

**技术栈：** Rust（golish-agent-kit / golish-agent-app / golish-agent-runtime / golish-sub-agents / golish-core）+ ts-rs + React/Zustand。

---

## 文件清单（创建 C / 修改 M）

| 文件 | C/M | 职责 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/stage_fanout.rs` | C | 抽取的纯/可复用 per-org fan-out：`filter_child_orgs` / `build_child_objective` / 单元构造（CLI + 工具共用） |
| `backend/crates/golish/src/stage_run/mod.rs` | M | 改为调用 `harness::stage_fanout` 的共享逻辑（行为不变，去重） |
| `backend/crates/golish-core/src/events/harness_trace.rs` | M | 新增 `HarnessTraceKind::StageRunOrgProgress` |
| `backend/crates/golish-sub-agents/src/defaults/builder/mod.rs` + `registry.rs` | M | 新增 `recon` 专家 SubAgentDefinition |
| `backend/crates/golish-sub-agents/src/defaults/prompts/mod.rs`(+新 prompt) | M/C | `build_recon_prompt()` |
| `backend/crates/golish-sub-agents/src/defaults/tests.rs` | M | 锁 recon 工具清单 |
| `backend/crates/golish-agent-app/src/ai/stage_run_tool.rs` | C | `StageRunTool`（fan-out 编排 + 进度 emit + 聚合 + 缺口回灌） |
| `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs` | M | 注册 `StageRunTool` |
| `backend/crates/golish-agent-runtime/src/execution_mode/policy.rs` | M | `BridgeToolSelection.stage_run: bool` |
| `backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs` | M | task 主 agent 启用 `stage_run` |
| `backend/crates/golish-agent-runtime/src/execution_mode/selection_apply.rs` | M | bridge 工具白名单放行 `stage_run` |
| `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs` | M | charter 文案：让主 agent 调 `stage_run` |
| `resources/harness/stages/target_intel.json` + 阶段 spec 结构 | M | 加 `specialist` / `coverage_axis` 字段 |
| `frontend/services/ai-events/*` + `useAiChatEvents.ts` | M | 新事件 → `setSessionStageRun` 合并 per-org 行 |
| `frontend/store/dev-mock.ts` | M | 保留 `__mockStageRun`（标注：真事件接好后仅作离线 demo） |

---

## Task 1 — 抽取共享 fan-out 到 agent-kit（去重，CLI 行为不变）

**文件：** 新 `backend/crates/golish-agent-kit/src/harness/stage_fanout.rs`；改 `backend/crates/golish/src/stage_run/mod.rs`、`backend/crates/golish-agent-kit/src/harness/mod.rs`（pub mod）。

**步骤：**
1. 在 `stage_fanout.rs` 定义纯函数（从 `stage_run/mod.rs` L598-634 搬，签名不变）：
```rust
use golish_db::models::Organization;
use crate::harness::StageKind;

/// 仅保留 parent 的直接子 org（与 CLI Phase 3 一致）。
pub fn filter_child_orgs(orgs: Vec<Organization>, parent: uuid::Uuid) -> Vec<Organization> {
    orgs.into_iter().filter(|o| o.parent_id == Some(parent)).collect()
}

/// 单个子 org 的 objective（钉死 organization_id + 只收本子公司）。
pub fn build_child_objective(child: &Organization, parent_name: &str, to: StageKind) -> String {
    format!(
        "Run the {} stage for this engagement. Organization: {} (organization_id: {}). \
         This organization is a subsidiary of {} (already landed during scoping); \
         collect for THIS subsidiary only — discover its own assets and register them \
         as in-scope targets bound to this organization_id.",
        to.as_str(), child.name, child.id, parent_name,
    )
}
```
2. `harness/mod.rs` 加 `pub mod stage_fanout;`。
3. `stage_run/mod.rs`：删除本地 `filter_child_orgs`/`build_child_objective`，改 `use golish_agent_kit::harness::stage_fanout::{filter_child_orgs, build_child_objective};`，把对应 `child_slice` 处调用更新。把 `stage_run/mod.rs` 里这两个函数的单测（L1042-1066）迁到 `stage_fanout.rs` 的 `#[cfg(test)]`。

**验证：** `cd backend && cargo nextest run -p golish-agent-kit -p golish --status-level fail`（迁移的单测 + 原 stage_run 测试全过）。
**提交：** `refactor(harness): extract per-org stage fan-out helpers into agent-kit`

---

## Task 2 — 新增 StageRunOrgProgress 进度事件

**文件：** 改 `backend/crates/golish-core/src/events/harness_trace.rs`；生成 `frontend/lib/generated/`。

**步骤：**
1. 在 `HarnessTraceKind` 枚举加变体（字段对齐前端 `SessionStageRun` 的行）：
```rust
/// stage_run 工具：某个 org 的实时进度（前端 StageRunView 用）。
StageRunOrgProgress {
    org_id: String,
    org_name: String,
    /// "passed" | "running" | "queued" | "blocked" | "pending"
    status: String,
    /// [(technique, "found"|"checked_empty"|"blocked"|"pending")]
    coverage: Vec<(String, String)>,
    evidence_count: u32,
    activity: Option<String>,
    /// stage label + role label，供前端首帧建卡
    stage_label: String,
    role_label: String,
},
```
2. 确认 `HarnessTraceKind` 已 `#[derive(ts_rs::TS)]` 且随 `AiEvent` 导出（event.rs L17-24）。
3. `just gen-types`，确认 `frontend/lib/generated/GeneratedAiEvent.ts` 出现新变体。

**验证：** `cd backend && cargo build -p golish-core` 成功；`just gen-types` 无错；`pnpm typecheck` 通过。
**提交：** `feat(events): add StageRunOrgProgress harness trace for per-org stage run`

---

## Task 3 — Recon 专家 sub-agent（从 Pentester 拆出收集角色）

**文件：** 改 `backend/crates/golish-sub-agents/src/defaults/builder/mod.rs` + `registry.rs`；`defaults/prompts/mod.rs`（+ 可选 `prompts/recon.rs`）；`defaults/tests.rs`。

**步骤：**
1. `builder/mod.rs`（仿 pentester 块 L82-100）新增：
```rust
SubAgentDefinition::new(
    "recon",
    "Recon",
    "Passive intelligence collector for the target_intel stage. Enriches org \
     assets via providers (0.zone/quake/ENScan), passive subdomain + URL history. \
     ZERO-TOUCH: no live probing/exploitation — that's the Pentester's job.",
    build_recon_prompt(),
)
.with_tools(vec![
    "recon_list_providers".into(),
    "recon_discover_subsidiaries".into(),
    "recon_enrich_assets".into(),
    "manage_targets".into(),
    "pentest_run".into(),            // 仅被动：subfinder/amass/gau
    "submit_stage_deliverable".into(),
    "record_finding".into(),
    "search_knowledge_base".into(),
    "read_knowledge".into(),
])
```
   在 `registry.rs` 同步同一份定义（live prompts 路径）。
2. `defaults/prompts/mod.rs` 加 `pub(super) fn build_recon_prompt() -> String`（参考 pentester prompt，强调 zero-touch + methodology 红线：每 root 一次、不逐子域 dig、不 live 探测）。
3. `defaults/tests.rs` 加 `test_recon_has_passive_tools_only`：断言含 `recon_enrich_assets`/`submit_stage_deliverable`、**不含** 攻击工具。

**验证：** `cd backend && cargo nextest run -p golish-sub-agents --status-level fail`。
**提交：** `feat(sub-agents): add Recon specialist (passive collection split from Pentester)`

---

## Task 4 — `stage_run` agent 工具（核心引擎）

**文件：** 新 `backend/crates/golish-agent-app/src/ai/stage_run_tool.rs`；改 `bridge_config.rs`、`execution_mode/policy.rs`、`modes/task.rs`、`selection_apply.rs`、`prompts/mod.rs`。

**步骤：**
1. 实现 `Tool`（仿 `start_operation_tool.rs` L20-38 的结构 + `harness_submit_tool.rs` 的依赖注入）。工具持有：`db_repo`、`event_tx`、`sub_agent` 派发句柄。`call(args)` 内：
   - 读「当前阶段」(from active_stage side-channel，仿 submit 工具) + engagement org 树（`db_truth`/recon repo）。
   - 用 `harness::stage_fanout::filter_child_orgs` 构造 per-org 单元（母 + 直接子）。
   - 以并发 K（默认 3，`tokio::task::JoinSet`）对每个 org：起一个绑 `set_harness_org_id(org_id)` 的 per-org 执行（复用 `golish/src/stage_run/mod.rs::orchestrate` 同款 `TaskOrchestrator` 构造；如该函数在 golish bin，把可复用部分也下沉到 agent-kit 或在工具内重建等价序列），跑该阶段切片，分派 `sub_agent_recon`（由 stage config `specialist` 决定）。
   - 每个 org 状态变化 → `event_tx.send(AiEvent::HarnessTrace { trace: StageRunOrgProgress { .. } })`。
   - 聚合「EVERY org must pass」（仿 stage_run/mod.rs L345-359）；有缺口则在返回值里结构化列出（org_id + 未过技术），供主 agent 门禁闭环回灌。
   - 工具名：
```rust
impl Tool for StageRunTool {
    fn name(&self) -> &'static str { "stage_run" }
    // params: { "concurrency"?: number }（其余从当前阶段+org树推断）
}
```
2. `bridge_config.rs`（仿 L407-468）`registry.register_tool(Arc::new(StageRunTool::new(...)))`。
3. `policy.rs` `BridgeToolSelection` 加 `pub stage_run: bool`（`none()` 里默认 false）。
4. `modes/task.rs`（L56-78）主 agent `bridge_tools` 里 `stage_run: true`。
5. `selection_apply.rs`（L72-108）把 `stage_run` 加进 `bridge_allowed` 放行逻辑。
6. `prompts/mod.rs`（L121 附近 charter）：target_intel charter 增一句「进入本阶段后，调用 `stage_run` 把收集按 org 并行铺开；不要逐个 `sub_agent_*`」。

**验证：** `cd backend && cargo nextest run -p golish-agent-app -p golish-agent-runtime --status-level fail`；新增 `tool_list` 守卫测试断言 task 主 agent 工具集含 `stage_run`。
**提交：** `feat(stage-run): stage_run agent tool — per-org parallel fan-out with progress + per-org gate`

---

## Task 5 — 阶段配置（specialist / coverage_axis）

**文件：** 改 `resources/harness/stages/target_intel.json` + 阶段 spec 反序列化结构（`golish-agent-kit/src/harness/` 下 stage spec struct）。

**步骤：**
1. `target_intel.json` 加：
```json
"specialist": "recon",
"coverage_axis": ["DNS", "WHOIS", "ASN", "CT", "SUBDOMAIN", "OSINT"]
```
2. stage spec struct 加对应 `Option<String> specialist` + `Vec<String> coverage_axis`（serde default 空）。
3. `stage_run_tool` 读 `spec.specialist`（决定派哪个 sub-agent）+ `coverage_axis`（进度事件 + 首帧）。

**验证：** `cd backend && cargo nextest run -p golish-agent-kit --status-level fail`（spec 解析测试）。
**提交：** `feat(harness): target_intel specialist=recon + coverage_axis config`

---

## Task 6 — 前端接真事件（替 mock）

**文件：** 改 `frontend/services/ai-events/registry.ts`（+ 新 handler）、`frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`、`frontend/store/slices/session.ts`（行合并）。

**步骤：**
1. 在 ai-events 注册 `harness_trace` 的 `StageRunOrgProgress` 处理：按 `org_id` 把该行 upsert 进 `session.stageRun.rows`（无则首帧用 `stage_label/role_label/coverage_axis` 建 `SessionStageRun`），重算 `summary`。
2. `useAiChatEvents.ts` 把事件路由到该 handler（解析到 terminal session id，与 SubAgentInlineCard 同套）。
3. 加 store action `upsertStageRunRow(sessionId, row)` 或在 handler 内 `setSessionStageRun` 合并。
4. `dev-mock.ts` 的 `__mockStageRun` 注释标注「真事件接好后仅离线 demo 用」。

**验证：** `just check-fe`（biome + tsc）；真应用跑 intel，左详情/卡片随后端事件实时更新。
**提交：** `feat(frontend): wire StageRunOrgProgress events into the stage-run view`

---

## Task 7 — 门禁闭环（主 agent ↔ stage_run 缺口回灌）

**文件：** 改 `stage_run_tool.rs`（返回缺口结构）、`prompts/mod.rs`（charter 说明闭环）。

**步骤：**
1. `stage_run` 返回值在有缺口时形如：`{ "passed": false, "gaps": [{"org_id": "...", "missing": ["GOLISH-INTEL-CT"]}] }`。
2. charter：主 agent 收到 gaps → 再次 `stage_run`（工具内只重跑 gaps 里的 org/technique，跳过已过的）。`stage_run_tool` 支持可选 `only_orgs?: string[]` 入参用于重跑。
3. 全过返回 `{ "passed": true }`，主 agent 进下一阶段。

**验证：** `cd backend && cargo nextest run -p golish-agent-app --status-level fail`（缺口聚合 + only_orgs 重跑单测）。
**提交：** `feat(stage-run): gap feedback loop — main agent re-runs only failed orgs`

---

## Task 8 — 收口

**步骤：** `just precommit` 全绿；更新 `agent-progress.md` 证据；`feature_list.json` 状态；若实现与 spec 有出入回填 `docs/design/2026-06-13-stage-run-fanout-design.md`。
**提交：** `chore(stage-run): precommit green + progress/feature_list update`

---

## 自检（规格覆盖度）

- 一个通用工具按阶段参数化 → Task 4（读 spec.specialist）+ Task 5（config）✓
- stage 管理者 sub-agent + 门禁闭环 → Task 4（fan-out + per-org gate）+ Task 7（闭环）✓
- 每 org 各过各的 gate（DB 真值隔离）→ 复用现有 `coverage_truth.rs` + per-org `set_harness_org_id`（Task 4）✓
- Pentester 退出收集 / Recon 拆出 → Task 3 ✓
- 进度实时到前端 StageRunView → Task 2 + Task 6 ✓
- 通用性（12 阶段）→ specialist/coverage_axis 配置化（Task 5），后续阶段只填配置（计划外，本计划只接 intel）✓
- 风险（并行机制/成本/缺口数据形状）→ Task 4（JoinSet K 并发）/ Task 7（gaps 结构）✓
