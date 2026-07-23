# Codex 同会话受管进程与可调 yield 实现计划

**目标：** 把 EAS 局部修复提升为所有 AI 本地命令进程的统一合同：bounded yield 只交还控制权，同一 managed process 继续运行，AI/用户依据活动显式决定是否 kill。

**架构：** `golish-app-core` 的 `BackgroundJobManager` 是主/子 agent 命令进程的单一生命周期 owner；PTY 与 pentest tools只选择一次 initial yield；`check_job`对同一个handle执行可调 output-sensitive poll；sub-agent通过共享 registry进入同一路径；EAS typed reconciler只保留业务授权和终态证据，不再携 transport wait。

## Task 1：恢复任务状态并锁定全局合同

- 新增本设计/计划，旧 EAS-scoped 文档标 superseded。
- 将 feature 从 `passing` 恢复为 `in_progress`，记录先前证据只覆盖局部合同。
- 清点所有 AI command execution 路径；区分 process lifetime 与 HTTP/socket/DB/stream-start timeout。

## Task 2：把 PTY timeout 重命名为 initial yield

- `run_pty_cmd` 新 schema 使用 `yield_time_ms`（10s default，250..30s）。
- 兼容读取旧 `timeout`，但不再输出或描述 process timeout。
- manager从 spawn 起持有同一个 child；initial yield后返回同一 `job_id`，不重启、不 kill。
- 结果使用 `initial_yield_ms` 与 `automatic_kill=false`；移除固定 30 秒环境/文案合同。

## Task 3：让进程读取本身也可调 yield

- `check_job`增加 `yield_time_ms`：0 immediate，默认10s，最大5m。
- terminal、新 stdout/stderr 或本次 yield结束即返回，并报告 `poll_reason`。
- 增加本地进程测试证明每次poll后仍是同一live handle且只有显式kill改变寿命。

## Task 4：统一 pentest 与 sub-agent 路由

- `pentest_run`优先读取 `yield_time_ms`，legacy `timeout_secs`只映射为initial yield。
- 普通pentest命令默认走managed-yield；必须同步authority的private wrapper不使用elapsed kill。
- sub-agent删除对 `execute_streaming` timeout executor的特殊绕行，让`run_pty_cmd/run_command`经过共享registry和agent tool context。
- 所有拥有raw shell/pentest权限的默认角色补齐`check_job/kill_job`，相关prompt说明quiet/activity判断。

## Task 5：从 EAS 业务真相移除 transport wait

- `PortScanPlan`、run args、coverage、attestation移除`inline_wait_secs=30`。
- EAS使用runner默认initial yield，不将它写进manifest或evidence。
- v3 Gate要求transport wait字段不存在，同时继续严格验证recipe/manifest/receipt/full-output/自然退出。
- 更新producer/read-side focused tests。

## Task 6：更新 UI、提示词和模块卡

- store兼容解析`initial_yield_ms`，不再展示“30秒后转后台”。
- panel显示受管进程、activity、不会自动停止和显式stop。
- 更新stage prompts/methodology、受影响backend/frontend模块卡及INDEX。

## Task 7：定向验证与收尾

- 每次Cargo前运行`just space-guard`。
- focused nextest覆盖app-core、pentest、agent Gate、sub-agent route/tools与runtime prompts。
- 运行受影响crate scoped Clippy `--all-targets -- -D warnings`、rustfmt check、focused Vitest、tsc、Biome、JSON和scoped diff checks。
- 不运行未获授权的init/precommit/全workspace门禁，不发真实外部扫描。
- 证据写入progress/feature，全部focused行为覆盖后才恢复`passing`。
