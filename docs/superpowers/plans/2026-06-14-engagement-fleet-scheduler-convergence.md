# 2026-06-14 · Engagement fleet scheduler convergence (CLI + chat → one per-org driver)

> 目标：让 **CLI `--stage-run`** 和 **chat/GUI 多 org 扇出**走**同一条**「每 org 一个完整 `run_stage` + 独立 gate」的调度路径，使 CLI（测试面）真正验证生产逻辑。
>
> 选型：用户在 2026-06-14 会话选定**方案 C：调度层统一**（不是把 CLI 降级成 chat 的 sub-agent 扇出）。
>
> 对齐既有设计：`docs/design/2026-06-12-engagement-fleet-orchestration.md`、`docs/design/2026-06-13-engagement-scoping-fanout-redesign.md §6.4`、`docs/design/2026-06-13-stage-run-fanout-design.md`。本计划即这些设计里的 **Phase B**（把 `run_fleet_scheduler` 内核接线进生产）。

## 0. 现状：四套并存的 per-org 机制（mid-migration）

| # | 机制 | 位置 | per-org 原语 | 谁驱动 | 状态 |
|---|---|---|---|---|---|
| 1 | CLI `--stage-run` Phase 3 | `golish/src/stage_run/mod.rs` step 6.5 | 完整 `orchestrate()`→`run_stage` | 后端 Rust 循环（串行） | 在用 |
| 2 | chat `stage_run` 工具 | `golish-agent-runtime/.../direct/stage_run_call.rs` | `sub_agent_<specialist>` | agent 在 stage 内调工具 | 在用（MCP-2 新建） |
| 3 | chat engagement worker pool | `engagement_scope.rs` + `chat.rs` `execute_task_mode` | 完整 `run_stage`（每会话 1 org） | **前端**逐 org 设 `engagement_worker_scope` + 发 prompt | 在用 |
| 4 | `run_fleet_scheduler` 内核 | `golish/src/engagement/scheduler.rs` | 注入 `OrgRunExecutor::run_org` | —（**未接线**，仅 Phase A 搬回备用） | 死代码 |

已就位的 trait 实现：`DbWeaknessScorer`（`engagement/weakness.rs`）。缺：`OrgRunExecutor`、`OrgCompletionOracle` 的生产实现。

## 1. 目标架构（统一后）

`run_fleet_scheduler` 成为**唯一**的多 org 扇出驱动：

- **per-org 原语统一**为 `OrgRunExecutor::run_org(task)` = 「为该 org 跑一个完整 `run_stage` 切片（独立 gate、独立 org 轴）」。
- **排序/并发/续跑**统一进内核：checklist/funnel 排序、`buffer_unordered(concurrency)`、`OrgCompletionOracle` 续跑跳过、失败隔离、`FleetReport` 聚合。
- CLI 与 chat 只提供各自的 `OrgRunExecutor`（怎么起一个 org-run）与上下文，**不再各写一套扇出循环**。
- 机制 2（`stage_run` sub-agent 工具）→ 评估废弃或仅保留为「单会话内轻量并采」别名，不再作为多 org 的权威路径（见开放问题 Q2）。

## 2. 开放设计问题（动手前需用户拍板）

- **Q1（chat 扇出归属）**：chat 多 org 由谁调度？
  - (a) **后端 scheduler 主导**：前端只「启动 engagement」，后端 `run_fleet_scheduler` 在一个驱动里跑完所有 org（每 org 一个 `run_stage`）。最符合「调度层统一」，但要改前端 worker pool 形态 + 单卡事件来源。
  - (b) **保留前端 worker pool**：前端继续逐 org 拉起 worker 会话，但每个 worker 内部与 CLI 共用同一 `OrgRunExecutor`/oracle 语义（共享内核语义，不共享同一次 scheduler 调用）。改动小，但「调度」仍分散在前端。
  - 倾向 (a)（真正的调度层统一）；(b) 是过渡。
- **Q2（`stage_run` 工具去留）**：统一后是否废弃机制 2？若前端/agent 不再需要 in-stage sub-agent 扇出，建议标记 deprecated 并从 `selection_apply` 摘除注入。

## 3. 任务分解（每步独立可验证 + 可回滚）

- **T1（fork 无关 · 先做）· CLI children → `run_fleet_scheduler`**
  - 在 `stage_run/mod.rs`：父 run（step 6）后，发现 children → 构造 `OrgRunTask`（entry=`child_entry`、allowlist=`child_allowlist`、objective=`build_child_objective`）→ 调 `run_fleet_scheduler(FleetConfig{concurrency:1, mode:Checklist}, tasks, &CliOrgRunExecutor, &oracle, &DbWeaknessScorer)` 替换 step 6.5 手写循环。
  - 新增 `CliOrgRunExecutor`（持 `Arc<AgentBridge>`/`Arc<PgPool>`/session/profile，`run_org` 包 `orchestrate()`）。
  - oracle：T1 先用 always-incomplete（行为保持：照跑所有 children）；DB 续跑 oracle 留 T3。
  - 报告：`FleetReport.render()` 替 `subsidiary_summary`；engagement 成败聚合改读 `FleetReport`。
  - 验证：`cargo nextest -p golish`（stage_run + scheduler 单测）+ clippy + ReadLints。**行为等价**（串行、跑全部 children、失败隔离、全过才算成功）。
- **T2 · 抽 `OrgRunExecutor` 到可共享位置**（若 chat 选 (a)）：把 executor/oracle 语义放到 chat 后端也能用的层（可能 `golish/src/engagement/` 下新模块），CLI 与 chat 共用。
- **T3 · DB 续跑 oracle**：实现 `DbCompletionOracle`（用 `engagement/weakness.rs::org_stage_has_truth`），CLI/chat 都接；CLI 可加 `--no-resume` 关掉。
- **T4 · chat 接 scheduler**（依赖 Q1）：
  - (a)：新增后端 engagement-run 入口，跑 `run_fleet_scheduler`；前端改为「启动+看单卡」；`StageRunOrgProgress` 事件由 executor 内 emit。
  - (b)：worker pool 内部统一到共享 executor 语义。
- **T5 · 处理 `stage_run` 工具**：**决定 = deprecate now, hard-remove in T4.4**。已在 `selection_apply` 注入点 + `stage_run_call` handler 标 DEPRECATED（超集为 `run_engagement_fleet`）；注入**暂留**以兼容旧前端 worker-pool 回滚路径（recon_family 靠它收子公司），T4.4 活体验证新路径后，连 injection + `direct/mod.rs` 路由 + 文件一并摘除。
- **T6 · 前端**：单卡（`StageRunView`）事件来源对齐统一路径；去掉重复/失效路径。
- **T7 · 收口**：`just precommit` 全绿；更新 `agent-progress.md` / `feature_list.json` / 模块卡 / 本计划状态。

## 4. 验证策略

- 每个 T 后：受影响 crate 的 `cargo nextest` + `clippy -D` + `ReadLints`。
- 阶段性：`just precommit` 全量（需停 dev 跑干净）。
- 活体：CLI `golish --stage-run --include-subsidiaries`（小米 key，非 deepseek）确认 children 经 scheduler 跑、每 org 独立 gate；对比统一前后报告。

## 5. 回滚

- 每个 T 独立 commit；T1 是纯 CLI 内部重构，回滚只 revert 一个 commit。
- chat 改动（T4）走 feature flag / 灰度：scheduler 路径与现有 worker pool 并存一版，确认后再摘旧路径。

## 6. 状态

- [ ] T1 CLI → scheduler
- [ ] T2 共享 executor
- [ ] T3 DB 续跑 oracle
- [ ] T4 chat 接 scheduler（待 Q1）
- [~] T5 stage_run 工具去留（已 deprecate；硬移除待 T4.4 活体验证后）
- [ ] T6 前端
- [ ] T7 收口
