# Stage Team Worker 工具围栏与运行流收敛实现计划

1. 先补 frontend store/handler/ThinkingBlock 回归测试，锁定 tool-boundary flush、batch 时间与
   零宽显示语义。
2. 为 Stage Team Worker UI identity 补纯函数测试，并让 producer execution 使用 WorkerRun
   级 parent request id。
3. 为 PostgreSQL transient transaction 分类补单测，在 `finish_worker_tool` 外围实现有限次
   整事务 retry。
4. 补 DB integration：构造 active `recon_list_providers` tool row 已 failed、Worker/WorkItem
   `recovery_required` 的 split state；下一次 claim 必须 supersede 旧 Worker 并领取新 attempt。
   既有 active external-tool manual recovery 测试必须继续通过。
5. 运行 `just space-guard`、聚焦 Vitest、golish-agent-runtime/golish-db 聚焦 nextest、Biome、
   TypeScript、fmt 与 `git diff --check`；按现有用户边界不跑 init/precommit/全量测试。

> 2026-07-15：步骤 1–5 的实现与聚焦验证已完成；fresh live rerun 与 `just precommit`
> 按现有用户边界延后。
