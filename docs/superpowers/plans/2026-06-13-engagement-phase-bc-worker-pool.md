# Engagement Phase B+C — 会话工人池 fan-out + 总览 UI 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
> 设计来源：`docs/design/2026-06-13-engagement-scoping-fanout-redesign.md` §5/§6.1/§6.3/§12 Phase B+C。
> 前置：Phase A 已落（`2026-06-13-engagement-phase-a-scoping.md`，纠名工具链 + engagement snapshot 读模型 + scheduler 内核搬回）。
> 验证节奏（用户指定）：**每期整期实现 → 末尾统一门禁 → 按报错修**。

**目标：** scoping 锁定范围后，前端按 K 受控并发把工作 fan-out 成真·独立 AI 会话（recon 按母公司家族、attack 按 org）；scoping 对话升级成 engagement 总览（org 树 + 活跃/队列 + per-org 状态 + 钻入）。
**架构：** ① 后端给 chat task 路径接上「工人范围」（org_id + 阶段切片 + 子公司策略），复用 CLI stage_run 已验证的 `resolve_slice`/`set_harness_org_id`/`set_stage_allowlist`/`run_stage` 链；② 前端纯逻辑池（单元构造/调度循环）+ Zustand 池 slice + 程序化 spawn（复用 conv/terminal/init/send 既有原语）；③ 总览组件内嵌 chat 流，订阅池状态 + DB 快照双源渲染。
**技术栈：** Rust（golish-agent-kit / golish-agent-bridge / golish-agent-app / golish bin）+ React/Zustand/TS。

---

## 实施期勘验结论（实读）

1. **task 模式无 lead turn（D1=B）**：`golish-agent-app/ai/commands/core/chat.rs::execute_task_mode` 直接 `TaskOrchestrator::new` → `set_profile_override(bridge.get_harness_profile())` → `run(task_input)`/`resume`。`sendPromptSession` 在 task 模式 resolve = 整个 operation 跑完；Err = BLOCK 终态/失败 → invoke reject。**这就是工位占用/释放信号**。
2. **org/切片接线缺口**：chat 路径 `harness_org_id`/`stage_allowlist`/`subsidiary_scope` 均未设置（orchestrator.rs:64 注释「until org wiring lands」）；CLI `stage_run/mod.rs::resolve_slice`（profile DAG 投影 + slice + entry_points）只存在于 golish bin，其依赖全是 agent-kit API → 可下沉共用。
3. **execution mode id**：`set_execution_mode(sessionId, "<profile_id>")` = Task 引擎 + profile（`red_team` 等，EMBEDDED_PROFILE_IDS 校验）。
4. **red_team 阶段链**：`scoping → target_intel → external_attack_surface → enumeration → vuln_triage → verification → access_validation → internal_discovery → objective_pathing → objective_simulation → cleanup → reporting`。工人切片：recon 家族=`target_intel..=enumeration`（org=母 + include_subsidiaries，Phase 3 母先收→子逐个收复用）；attack org=`vuln_triage..=reporting`（单 org）。
5. **spawn 可纯函数化**：`useCreateTerminalTab` 内部全走 `useStore.getState()` + `ptyCreate`，无 React 状态依赖 → spawn 序列（createNewConversation→addConversation→ptyCreate/addSession→addTerminalToConversation→initAiSession→setAgentMode("auto-approve")→setExecutionMode(profile)→set worker scope→seed 消息→sendPromptSession）可放 `lib/engagement/` 纯模块。
6. **provider 配置**：`buildProviderConfig(provider, model, workspace, settings)`（AIChatPanel 的 selectedModel useState 持有 model/provider）→ 池启动参数传入。
7. **AgentBridge config**：`harness_profile` 的 set/get 模式（`agent_bridge/config.rs:376`）是 worker scope 字段的样板；bridge crate 已依赖 agent-kit（待核 Cargo.toml，若无则 scope 存字符串、chat.rs 解析）。
8. **完成复核**：`engagement_get_snapshot`（Phase A）的 per-org status（DB 真值 org_stage_has_truth）做 spawn 前续跑判定 + 完成后兜底复核。

## Phase B 文件清单

| # | 文件 | 动作 | 职责 |
|---|---|---|---|
| B1 | `backend/crates/golish-agent-kit/src/harness/slice.rs`（新）+ `harness/mod.rs` | 下沉 | `resolve_slice(profile_id, from, to) -> (entry, allowlist)`（从 stage_run 平移）|
| B2 | `backend/crates/golish/src/stage_run/mod.rs` | 改 | 删本地 resolve_slice，改调 agent-kit 版 |
| B3 | `backend/crates/golish-agent-bridge/src/agent_bridge/{config.rs,mod.rs}` | 改 | `EngagementWorkerScope{org_id, from, to, include_subsidiaries, threshold_pct}` 字段 + set/get |
| B4 | `backend/crates/golish-agent-app/src/ai/commands/engagement_scope.rs`（新）+ `commands/mod.rs` + `ai/mod.rs` | 新 | Tauri `engagement_set_worker_scope` / `engagement_get_worker_scope`（session 绑定，IDOR：org 必须属当前 project） |
| B5 | `backend/crates/golish/src/commands_facade/engagement.rs` + `commands_registry.rs` | 改 | facade 转发 + 注册 2 命令 |
| B6 | `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs` | 改 | task 路径读 scope → set_harness_org_id/set_subsidiary_scope/set_stage_allowlist + `run_stage(entry)`（run 与 resume 分支都设）|
| B7 | `frontend/lib/engagement/pool.ts`（新） | 新 | 纯逻辑：WorkerUnit/STAGE_SLICES/buildReconUnits/buildAttackUnits/buildWorkerPrompt/nextSpawnable（K 并发判定）— 全单测 |
| B8 | `frontend/store/slices/engagementPool.ts`（新）+ `store/index.ts` | 新 | 池状态 slice（workers/queue/concurrency/running 计数 + 动作）|
| B9 | `frontend/lib/engagement/spawn.ts`（新） | 新 | spawnWorkerConversation（程序化 conv+terminal+init+mode+scope+seed+send）|
| B10 | `frontend/lib/engagement/runPool.ts`（新） | 新 | 池循环：续跑跳过 → K 并发 spawn → await send → PASS/BLOCK/FAILED → recon 完成把该家族 attack 单元入队 → 收官汇总 |
| B11 | `frontend/lib/api/engagement.ts` | 改 | + setWorkerScope/getWorkerScope wrapper |
| B12 | `frontend/lib/engagement/pool.test.ts`（新） | 新 | B7 纯函数 + B8 slice 动作单测 |

## Phase C 文件清单

| # | 文件 | 动作 | 职责 |
|---|---|---|---|
| C1 | `frontend/components/Engagement/EngagementOverview.tsx`（新，参考 stash 旧版改造） | 新 | 总览卡：org 树折叠表（家族分组）+ 状态徽章（pending/running/passed/blocked/failed/skipped）+ 活跃 K/队列剩余 + K 设置 + 「开始 fan-out」/「停止」 + 点行钻入（setActiveConversation）|
| C2 | `frontend/components/Engagement/engagementOverview.utils.ts`（新） | 新 | 快照×池状态合并（运行时态覆盖 DB 真值）、行模型构造 — 纯函数全单测 |
| C3 | `frontend/components/AIChatPanel/AIChatPanel.tsx`（或 MessageBlock 区域） | 改 | scoping 会话内嵌总览：conversation 标记 `engagementOverview: true` 时在消息流上方渲染 C1 |
| C4 | `frontend/store/slices/conversation.ts` | 改 | ChatConversation 加可选 `engagementRole?: "overview" \| "worker"` + worker 元数据（orgName/unitKind），ConversationTabs 徽标用 |
| C5 | `frontend/components/AIChatPanel/ConversationTabs.tsx` | 改 | worker 会话 tab 显示单元徽标（家族/org + 状态点）；engagement 分组折叠（≥N 个 worker 时收纳成组）|
| C6 | `frontend/components/Engagement/EngagementOverview.test.tsx` + utils 测试 | 新 | 合并函数 + 渲染冒烟 |

## 任务（Phase B）

### B-T1 · resolve_slice 下沉 agent-kit
`harness/slice.rs`：平移 stage_run 的 `resolve_slice`（参数 `(profile_id: &str, from: Option<StageKind>, to: StageKind)`，内部 `base_operation_graph` + `load_embedded_profile` + `project` + `slice` + entry_points，错误用 `Result<_, String>` 或 anyhow——agent-kit 错误风格按现有 harness 模块（先看 graph.rs 用什么）。stage_run 原 4 个调用点 + 单测改 import。harness/mod.rs `pub use slice::resolve_slice;`。

### B-T2 · bridge worker scope
config.rs 加：
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct EngagementWorkerScope {
    pub org_id: uuid::Uuid,
    pub from: Option<String>,   // stage id 字符串（bridge 不依赖 agent-kit 时的中立表示）
    pub to: String,
    pub include_subsidiaries: bool,
    pub subsidiary_threshold_pct: u8,
}
```
字段 `engagement_worker_scope: RwLock<Option<EngagementWorkerScope>>` + `set_engagement_worker_scope/get_engagement_worker_scope`（mirror harness_profile）。若 bridge 已依赖 agent-kit 则 from/to 直接 `StageKind`。

### B-T3 · Tauri 命令（agent-app）
`engagement_scope.rs`：
- `engagement_set_worker_scope(session_id, org_id: String, to: String, from: Option<String>, include_subsidiaries: Option<bool>, threshold_pct: Option<u8>, state)`：校验 session 已 init、org_id 是 UUID、from/to 是合法 stage id（StageKind::try_parse）、to 必填；scope 写 bridge。传 `org_id=""` 或新增 `engagement_clear_worker_scope` 清除（取一种：用 set 传 null org → 清除，简化为 `engagement_clear_worker_scope(session_id)`）。
- `engagement_get_worker_scope(session_id)` 回显（调试/恢复用）。
命令域名是 engagement_*（I4），但实现住 agent-app（State<AgentState>）；golish facade `commands_facade/engagement.rs` 加 `pub use golish_agent_app::ai::commands::engagement_scope::*;` + registry 注册。

### B-T4 · chat.rs task 路径接线
`execute_task_mode` 在 `orchestrator.set_approval_coordinator(...)` 后：
```rust
let worker_scope = bridge.get_engagement_worker_scope().await;
if let Some(ws) = &worker_scope {
    orchestrator.set_harness_org_id(Some(ws.org_id));
    orchestrator.set_subsidiary_scope(ws.include_subsidiaries, ws.subsidiary_threshold_pct);
}
```
run/resume 选择处：scope 有值时解析切片（profile id 取 `bridge.get_harness_profile()` 或 active 默认）：
```rust
let slice = match &worker_scope {
    Some(ws) => Some(golish_agent_kit::harness::resolve_slice(&profile_id, ws.from_kind()?, ws.to_kind()?)?),
    None => None,
};
match (resumable, slice) {
    (Some(task), _) => orchestrator.resume(...),            // resume 已带 allowlist?（resume 不重投影 → 仍 set_stage_allowlist）
    (None, Some((entry, allowlist))) => { orchestrator.set_stage_allowlist(Some(allowlist)); orchestrator.run_stage(entry, ...) }
    (None, None) => orchestrator.run(...),
}
```
resume 分支同样 `set_stage_allowlist`（实例级配置）。tracing 打 worker scope 摘要。

### B-T5 · 前端纯逻辑 pool.ts
```ts
export type WorkerUnitKind = "recon_family" | "attack_org";
export interface WorkerUnit { id: string; kind: WorkerUnitKind; orgId: string; orgName: string; familyRootId: string; }
export const STAGE_SLICES = { recon_family: { from: "target_intel", to: "enumeration" }, attack_org: { from: "vuln_triage", to: "reporting" } } as const;
export function buildReconUnits(tree: OrgTreeNode[]): WorkerUnit[]            // 每根一个（家族）
export function buildAttackUnits(tree: OrgTreeNode[], familyRootId: string): WorkerUnit[]  // 母+全部子，各一个
export function buildWorkerPrompt(unit, opts: { includeSubsidiaries: boolean; thresholdPct: number }): string  // 中文任务书：目标 org、阶段范围、纪律（只打该 org、过 gate 即收）
export function unitAlreadyCovered(unit, snapshot): boolean                    // status==passed/skipped（recon 看母 org 的 to-stage 真值；attack 看该 org reporting——快照 toStage 参数化，B-T7 池里分别查）
```

### B-T6 · 池 store slice
`engagementPool.ts`：state `{ pool: { running: Record<unitId, WorkerRuntime>, queue: WorkerUnit[], done: Record<unitId, WorkerOutcome>, concurrency: number, phase: "idle"|"running"|"stopped"|"complete", projectPath: string|null } }` + actions（configurePool/enqueueUnits/markSpawning(convId 绑定)/markRunning/markOutcome/setConcurrency/stopPool/resetPool）。WorkerRuntime 含 convId（钻入用）。注册进 store/index.ts（照其它 slice 模式）。

### B-T7 · spawn.ts + runPool.ts
spawn（参数：unit、profileId、model/provider、projectPath、scopeOpts）：
1. `const conv = createNewConversation(); conv.title = workerTitle(unit);` + store.addConversation + `engagementRole:"worker"` 元数据（C4 字段，B 期就加上）
2. `ptyCreate` 走 `useCreateTerminalTab` 等价序列（直接 import ptyCreate + addSession + addTerminalToConversation；不复用 hook，逻辑等价、加注释链接源 hook）
3. `initAiSession(conv.aiSessionId, buildProviderConfig(...))`
4. `setAgentMode(conv.aiSessionId, "auto-approve")`（工人无人值守）
5. `setExecutionMode(conv.aiSessionId, profileId)`
6. `engagementSetWorkerScope(conv.aiSessionId, { orgId: unit.orgId, from/to: STAGE_SLICES[unit.kind], includeSubsidiaries: unit.kind==="recon_family", thresholdPct })`
7. seed 用户消息进 conv（addConversationMessage role:user）+ `setConversationStreaming(true)`
8. `await sendPromptSession(conv.aiSessionId, buildWorkerPrompt(unit, ...))` → resolve=PASS 候选 / reject=BLOCK 或 FAILED（错误文本含 "block" → blocked，else failed——镜像 scheduler::classify_run_error）
9. finally `finalizeStreamingMessage`
runPool（参数：units、K、snapshotFetcher）：
- 主循环：while queue 非空 or running 非空：续跑判定（unitAlreadyCovered → markOutcome(skipped) 出队）；可 spawn（running<K）→ 取队首 spawn（不 await——push 到 inflight Promise 池）；`await Promise.race(inflight)` 收一个 → markOutcome → **recon_family passed → buildAttackUnits(该家族) 入队**；stopPool 标志位中断（不杀正在跑的，等其收口；不再出队）。
- 收官：phase=complete + 汇总（覆盖 x/n、blocked/failed 列表）写池状态。

### B-T8 · 前端 API wrapper
`lib/api/engagement.ts` + `engagementSetWorkerScope/engagementGetWorkerScope/engagementClearWorkerScope`。

### B-T9 · Phase B 统一验证
```bash
cd backend && cargo check -p golish-agent-kit -p golish-agent-bridge -p golish-agent-app -p golish
cargo nextest run -p golish-agent-kit -p golish-agent-bridge -p golish-agent-app -p golish --status-level fail
cargo clippy（同四 crate）--all-targets -- -D warnings && cargo fmt（同）-- --check
just check-fe && cd frontend && pnpm --silent test
python3 scripts/check_repo_ownership.py && python3 scripts/check_dag.py
```

## 任务（Phase C）

### C-T1 · 合并纯函数 utils
`engagementOverview.utils.ts`：`mergePoolOntoSnapshot(snapshot, pool): OverviewRow[]`（DB 真值底 + running/queued/outcome 覆盖；树→带深度行用 Phase A 的 flattenTree）+ `summarize(rows, pool)`（活跃 K/队列/covered/blocked/failed）。

### C-T2 · EngagementOverview 组件
卡片式（嵌在 chat 流上方）：摘要条（root/total/covered/活跃/队列）+ K 数字输入 + 开始/停止按钮（调 runPool/stopPool，参数 model/provider 从 props）+ org 树表（家族分组折叠、状态徽章、weakness total、ownership%）+ 行点击 → worker convId 存在则 setActiveConversation。三态：加载快照 spinner / 空 org 树（提示先跑 scoping）/ 错误重试。i18n 跟现有 key 风格（en + zh-CN）。

### C-T3 · chat 内嵌 + 会话标记
- conversation.ts：`engagementRole?: "overview" | "worker"`、`workerMeta?: { unitId; unitKind; orgName }`（持久化随 conv DB 同步走现有序列化——核对 conversation-db 是否白名单字段，若是则补）。
- AIChatPanel：active conv `engagementRole==="overview"` → 消息流顶部渲染 `<EngagementOverview/>`；提供「将本会话设为 engagement 总览」入口（scoping 会话内一个轻按钮 / 或 snapshot rootCount>0 时自动提示条）。
- ConversationTabs：worker 会话 tab 加状态点（池状态色）+ orgName 截断徽标；worker 数 > 6 时折叠为「Engagement ×N」组 tab（点开下拉列表切换）。

### C-T4 · Phase C 统一验证
`just check-fe` + `pnpm --silent test`（新组件/utils 测试）+ 全栈 `cargo check -p golish`（C 期不动 Rust，跑一遍兜底）。

## 风险与对策
- **resume × allowlist**：恢复路径的 operation cursor 已在 DB；set_stage_allowlist 是实例级，resume 分支必须重设否则投影回整张 DAG（B-T4 显式覆盖 + 单测如可行）。
- **conversation DB 持久化新字段**：engagementRole/workerMeta 若 conv 序列化是显式字段表则要登记，否则刷新后 worker 标记丢失（C-T3 核对 conversation-db-sync）。
- **K>1 并发 bridge 隔离**：每 worker 独立 aiSessionId = 独立 bridge（init_ai_session per session）→ 天然隔离；LLM 限流风险由 K 默认 3 + 用户可调缓解（spec §11 风险 2）。
- **弱模型工人跑偏**：worker scope 的 org/切片是硬约束（gate 轴 + DAG 投影），prompt 只是辅助——比纯 prompt 方案强一档。
- **池循环生命周期**：runPool 是长 Promise，挂在模块级（非组件内）；刷新页面 = 池态丢失但 DB 真值在（续跑判定恢复）——spec §10 已接受。
