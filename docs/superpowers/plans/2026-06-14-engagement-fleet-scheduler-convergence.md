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
  - **T1 UX 回归补回（2026-06-14）**：T1 用 `run_fleet_scheduler` 替换手写循环后，丢了旧循环里的 `── subsidiary i/N: 名 ──` 逐子 eprintln（调度内核 IO-free，只在最后出 FleetReport）→ headless 跑 6 子时 CLI 中途看不到「第几个子」。修复：调度器加第 4 个注入 trait `FleetProgress`（`on_org_start`/`on_org_done`，副作用外置故内核仍零 IO），CLI 传 `CliFleetProgress{label:"subsidiary"}` 打逐子进度（含终态 PASS/BLOCK/FAIL/SKIP）、GUI 传 `NoopProgress`（走单卡）。i/N 由调度器静态 org 序给（scheduler 知 total）。验证：`nextest -p golish engagement stage_run` 40/0（含新增 `progress_reports_index_total_and_skips_dont_start` 验 index/total + 跳过不 start）+ `clippy -p golish --lib --tests -D` 0 告警。
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

## 附录 · target_intel 阶段缺口（2026-06-14 活体观察 → follow-up，按价值排）

> 本会话用小米 MiMo 活体跑 `--stage-run --to target_intel --include-subsidiaries`（默安科技 + 6 子，经 run_fleet_scheduler 串行）时观察到的 **target_intel 情报质量缺口**，与 fleet 收敛正交，单独记为 follow-up。

1. **基本工商信息不进 gate（最该先补）**：`gate` 只判 6 维（ASN/CT/WHOIS/OSINT/SUBDOMAIN/DNS），但 enrich 落库的 ICP 备案 / `credit_code` / 行业 / App / 小程序 / `email_domains` / `ip_ranges` 这些「收了不强制、漏了不挡」。建议把关键几项纳入 gate / deliverable 必填，或报告强制列「查了/没查」。
2. **`checked_empty` 比 `found` 弱（最该先补）**：`found` 已锁 DB 真值（本会话修的 db_truth）；但「没数据→checked_empty」仍靠弱模型诚实跑工具+如实报（MiMo 可能空报蒙混）。需用 tool-invocation evidence 硬绑 checked_empty（I8「查过为空 ≠ 没查」要可证）。
3. **OSINT 维度太粗**：一个 bool 囊括 `intel.records`/`contacts`/`social_accounts`/`business_systems`；邮箱/泄露/业务系统/社交价值不同却合并成「有 OSINT 就过」，子类不可见、不可分别要求。
4. **DNS/SUBDOMAIN 维度太粗**：DNS 只判「有记录」，不分 MX/NS/TXT(SPF/DMARC)/CNAME；SUBDOMAIN 不分被动来源、不去重 CT 来源。
5. **per-org 确定性 gate + DB 续跑 oracle（= 计划内 T3）**：现 `AlwaysRunOracle` 每次重跑全部 org（无续跑跳过，浪费 LLM）；per-org「过」目前 = run_stage PASS，可加按 org_id 读 DB 真值的确定性 gate（`org_stage_has_truth`）。
6. **母子「家族关联」缺失**：扁平 per-org 跑，子公司发现有了，但母子间资产关联图（共享域/品牌/信任链/同 IP 段）没显式建模；cross-org 撞同一资产的归属/去重无策略。

> 另记：上游 `scoping` 的 red_team `unit-candidate / organization-creation flow` 检查（`evaluate_red_team_scoping_flow`，要求 `unit_review_invoked && organization_created`，且 create 撞重名不算）对弱模型 + 持久 PG（org 已存在→create 重名）很脆，活体常卡 scoping。非本 scope，但影响活体推进。
>
> **[2026-06-14 已修一半]** 上面这条 `organization_created` 脆点已修：`golish-pentest-app/manage_organizations.rs` 单个 `create` 改 **get-or-create**（撞重名 → parent-scoped 查出已存在 org 返回其 id 作 success，而非吞成 `{"error"}` 无 id），持久 PG 重跑不再因「母公司已存在→create 重名→organization_created=false」卡 scoping。防作弊不破（仍须真调 create + `ask_human(unit_review)`；gate 仍查表确认 id 真存在）。验证：`nextest -p golish-pentest-app manage_organizations` 4/4（含 `existing_org_create_result_is_gate_countable`）+ clippy -D 0。**未修的另一半**：`unit_review_invoked` 仍要求本 session 真发 `ask_human(input_type="unit_review")`——弱模型若跳过人审仍会卡（这是有意的防偷懒，非 bug）。
