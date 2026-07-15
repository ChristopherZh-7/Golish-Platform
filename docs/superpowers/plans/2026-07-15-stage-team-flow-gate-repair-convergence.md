# Stage Team 运行流、Gate truth 与 Repair 收敛实现计划

> 对应设计：`docs/design/2026-07-15-stage-team-flow-gate-repair-convergence.md`

## Task 1：锁定前端回归（RED）

- 修改 `StageTeamRunView.test.tsx`：断言 Producer 未 Gate PASS 时只显示“已返回，待 Gate”，
  Aggregator 独立显示，并可按 WorkerRun 打开运行流。
- 修改 `StageRunOrgRows.test.tsx`：断言 worker request map 与旧 Aggregator fallback 正确透传。
- 修改 `harness-handlers.test.ts`：断言 `::team::` 和带 Worker 后缀的 request id 都绑定原始
  `stage_run` tool request。

## Task 2：实现 Team UI 与事件绑定

- `StageTeamRunView.tsx` 分离 Producer returned、Aggregator、Stage Gate 三层状态；每个 WorkItem
  渲染 exact flow button。
- `StageRunOrgRows.tsx` 不再用组织级 pointer 作为通用 flow；接收 WorkerRun→parentRequestId map，
  仅为旧 Aggregator保留 Team pointer fallback。
- `ToolCallDetailView.tsx` 从当前 session `activeSubAgents` 构建 WorkerRun identity map。
- `harness-handlers.ts` 支持 `::team::` parser，阻止 retry progress串到旧 card。

## Task 3：锁定 Producer authority fence（RED）

- 在 `stage_team_scheduler.rs` 为 snapshot validator 增加纯函数测试：
  - ASN 自报 `checked_empty`、snapshot仍 pending/error → reject；
  - exact technique全部 terminal checked_empty → accept；
  - found 但仍有 pending applicable cell → reject。

## Task 4：实现 Producer authority fence

- 在 Producer完成路径读取 exact authoritative coverage snapshot。
- 将 axis stable key映射到 `spec.expected_techniques` 的 exact technique，并在 immutable output
  landing 前验证。
- mismatch 复用 `retry_stage_worker`，DB read failure fail closed。

## Task 5：锁定并修复 Aggregator 首次 BLOCK handoff（RED→GREEN）

- 为 SubAgent dispatcher增加测试：host-owned terminal stage submission 的 `needs_fix +
  deliverable_submission_id` 必须形成 barrier response；普通 Worker仍保留既有 repair loop。
- 给 `BoundWorkerChainContext` 增加内部 terminal policy，Stage Team Aggregator独占启用。
- Aggregator parent request id改为 `::aggregator:<worker_run_id>`；scheduler从第一次 durable
  submission执行 Gate并进入现有 repair-generation分支。

## Task 6：聚焦验证与文档同步

- 前端：相关 3 个 Vitest 文件、scoped Biome、TypeScript typecheck。
- Rust：先 `just space-guard`，再运行 `golish-sub-agents` dispatcher聚焦测试与
  `golish-agent-runtime` Stage Team聚焦测试；执行 scoped rustfmt/check。
- 更新 `docs/modules/frontend/components.md`、
  `docs/modules/backend/golish-agent-runtime/agentic_loop.md`、相关 sub-agent executor模块卡与
  `docs/modules/INDEX.md`。
- 更新 `agent-progress.md` 和 `feature_list.json` 证据；没有 full precommit/live rerun则功能仍为
  `in_progress`。
