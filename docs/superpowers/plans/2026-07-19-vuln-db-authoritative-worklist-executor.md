# Vuln DB-authoritative Worklist Executor 实现计划

> **面向 AI 代理的工作者：** 使用 test-driven-development 与 executing-plans，逐任务保留 RED/GREEN 证据。

**目标：** 把 Vuln formulaic fan-out、有限降级重试、target transport breaker 与用户可见 coverage 从 LLM 自由循环迁移到服务端 DB-authoritative executor，同时让 Nuclei wrapper完整拥有 timeout/cancellation/landing。

**架构：** `golish-agent-runtime` 在 Company Controller claim 后读取 operation-scoped coverage，生成 exact origin × capability shards并通过现有 durable Stage Worker Request 落库；`golish-core` 提供 task-local tool cancellation，`golish-app-core` foreground runner kill+await，`golish-pentest-app` 分类并 evidence-land Nuclei failure；前端通过现有 coverage IPC 的 operation-aware参数读取 `terminal/total`。

**技术栈：** Rust 2021、Tokio、SQLx、Tauri 2、React 19、TypeScript、Vitest。

## 任务 1：锁定 deterministic Vuln shard 规划

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`

1. 先写纯函数测试：两个 origins 不能合并；pending baseline 五 cells 合并为一个 exact shard；partial baseline 五 cells 拆成五个 single-technique recovery shards；terminal cells 不生成；缺 target id、空/非 exact HTTP(S) origin、未知 technique fail closed。
2. 运行 RED：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(vuln_worklist_shard)'
```

3. 实现 `VulnWorklistShard`、capability mapping、stable key/objective/subject material。请求形状包含 exact `target_id`、`target_url`、tool、techniques、`primary|narrowed`，禁止空 subject 与自然语言 whole-company assignment。
4. 运行相同命令验证 GREEN。

## 任务 2：让 StageRun 服务端驱动 Vuln fan-out

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`、`stage_team_scheduler.rs`

1. 先写测试证明 Vuln leader objective 不再要求 LLM dispatch，服务端 request hash/subject/objective稳定，pending→partial 后 stable key 缩小；非 Vuln Company Controller 保持原行为。
2. 在 `execute_company_controller_unit` 的 leader claim 后加入 Vuln 分支：读取 exact operation/org coverage，持久化未完成 deterministic requests，park leader，drain children并重读；无 gaps 时走现有 close/bind/final submit。
3. Vuln plan 设置 `organization_scope_implicit=false`、`formulaic_worklist_executor=v1`、初始 group attempt=1、narrowed recovery attempt 有限。replayed exhausted narrow shard不得派生同形新请求，返回带 code 的 StageRun block。
4. 定向运行 runtime shard/Company Controller/child drain tests。

## 任务 3：让 wrapper 拥有 cancellation 并 kill+await

**文件：** 修改 `backend/crates/golish-core/src/agent_session.rs`、`backend/crates/golish-app-core/src/background_jobs.rs`、`backend/crates/golish-app-core/src/pty_interactive.rs`、`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

1. 先写 RED 测试：task-local cancellation 隔离；foreground job cancellation 等待 terminal child/reaper；self-bounded wrapper cancellation 不跳过 tool lifecycle landing。
2. 实现 task-local `AgentToolCancellation`；SubAgent cancellation 先 signal token，再等待 self-bounded wrapper future；普通工具保留即时取消。
3. foreground runner 保留 tool attribution但不进入 background close barrier；收到 token 后 kill，等待 manager terminal snapshot，remove 后返回 typed cancelled result。
4. wrapper结果完成 DB/evidence/tool fence landing 后，dispatch 返回 cancelled并停止后续模型迭代。
5. 定向运行 core/app-core/sub-agents tests。

## 任务 4：有限、稳定的 target transport terminalization

**文件：** 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`、`landing.rs`

1. 先写 RED 纯函数/SQL-shape tests：target refusal/reset/TLS/target timeout 可分类；wrapper deadline/exit 124/template/parser/DB属于 runtime；同 class attempt 1/2 partial、3 blocked；不同 class或成功重置；重复同 generation不增加。
2. 把 attempt generation/failure owner/class写进每条 Nuclei evidence raw payload；从严格 scoped evidence 尾序列计算连续 attempts。
3. 新增 target transport terminal completion，仅第三次同类 target failure写 evidence-backed blocked；scanner/runtime/cancelled继续写 partial并返回 `automatic_retry_allowed` 的有限语义。
4. 定向运行 Nuclei parser/landing/capability tests与 `golish-pentest-app` scoped Clippy。

## 任务 5：UI 分离 attempts/retry/recovery 并展示 cells

**文件：** 修改 `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`、`frontend/lib/api/stage-coverage.ts`、`frontend/components/Engagement/StageTeamRunView.tsx`、`StageTeamRunView.test.tsx`

1. 先写 Vitest RED：coverage 返回 340 terminal/360 total时显示剩余20；历史 failed、live generation retry、manual recovery分别归组；coverage loading/error/empty三态可见。
2. 给现有 coverage command增加可选 `operation_id` 原始参数；Vuln 必须验证并使用 exact operation，非 Vuln兼容原调用。不要修改 generated types。
3. StageTeamRunView 按 Unit读取并聚合 coverage cells；刷新 Stage Team 时同步刷新 coverage，保留独立错误态。
4. focused Vitest、受影响文件 Biome；类型链受影响时跑 `pnpm typecheck`。

## 任务 6：收尾与证据

**文件：** 更新 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-app-core.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/frontend/components.md`、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(vuln_worklist_shard) | test(company_controller_vuln)'
just space-guard
cd backend && cargo nextest run -p golish-core -p golish-app-core -p golish-sub-agents -E 'test(agent_tool_cancellation) | test(foreground.*cancel) | test(self_bounded.*cancel)'
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(nuclei.*transport) | test(nuclei.*landing)'
pnpm exec vitest run frontend/components/Engagement/StageTeamRunView.test.tsx
pnpm exec biome check frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx frontend/lib/api/stage-coverage.ts
pnpm typecheck
```

每个 Cargo 命令前重新运行 `just space-guard`。只运行受影响 crate/file 的定向验证；不运行 init/precommit/全仓测试，不启动真实 CLI/外部扫描。没有真实 CLI 证据时 feature 保持 `in_progress`，不得声称整条 Vuln 阶段闭环。

## 2026-07-19 实施状态

- 任务 1–5 已落代码并通过对应 focused Rust/Vitest/Biome/typecheck/Clippy 验证；Vuln exact Nuclei shard 现由 host executor 直调 wrapper，LLM 不再拥有公式化 fan-out、分页或 retry。
- DB replay 额外区分 claimable、in-flight 与 operator-recovery WorkItem，避免把已有 Claimed/Running shard误判为自动预算耗尽。
- 按用户本轮“先修、不用跑”的指令，没有启动 Golish CLI、provider 或真实 Nuclei 网络扫描；因此任务 6 只完成代码级收尾，feature 继续保持 `in_progress`，真实阶段 Gate PASS 仍是后续闭环门槛。
