# Stage Team producer retry 与 UI 收口实现计划

> 2026-07-15：步骤 1–5 的实现和聚焦验证已完成；fresh live rerun 与 `just precommit` 按用户边界延后。

1. 为唯一 fenced JSON、invalid checked-empty、dependency blocker 和 landing policy 补 Rust
   回归测试。
2. 将 producer 校验改为 `Result`，在 `execute_stage_team_producer` 中把校验失败和登记的
   dependency blocker 路由到现有 `retry_stage_worker`；保持 exhausted output 与 DB immutable
   约束不变。
3. 为 `StageRunOrgRows` 增加 Team/legacy 互斥测试并改为 exact-pointer 分支渲染。
4. 将 `StageTeamRunView` 改为业务摘要默认态 + 调度详情折叠态，补 output-over-worker 状态测试。
5. 运行 `just space-guard`、聚焦 Rust tests、聚焦 Vitest、Biome/typecheck；同步模块卡、
   `agent-progress.md` 与现有 in-progress feature evidence。按用户既有边界不跑 `init.sh`、
   `just precommit` 或全量测试。
