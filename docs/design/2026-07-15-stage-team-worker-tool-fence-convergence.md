# Stage Team Worker 工具围栏与运行流收敛

> 状态：Implemented with focused validation（2026-07-15）；fresh live rerun / full precommit pending

## 问题

最新 `target_intel` Team run 中，六个 sibling producer 共用同一个前端
`parent_request_id`。并发 reasoning 因而被拼进同一条 SubAgent 时间线；批处理只保留
flush 时刻，又把零宽片段渲染成 `0.001s`。

同一次 run 的 DNS producer 已写入 DNS 业务事实，但在
`recon_list_providers` 返回后，`finish_worker_tool` 遇到 PostgreSQL deadlock。当前实现把
这类可重试事务失败立即视为 lease 丢失；通用 tool row 随后已 terminal failed，而
Worker 仍保留 active-tool pointer，最终进入无法领取的 `recovery_required`。

## 决策

1. 每个 Stage Team WorkerRun 使用独立的 UI `parent_request_id`；组织级 Team progress
   pointer 继续保持组织级，不把 sibling reasoning 合并。
2. SubAgent reasoning 在 tool request/result/completed/error 边界前同步 flush，并把 batch
   首末到达时间传给 store。零宽时间不伪装为 `0.001s`，小于 100ms 显示 `<0.1s`。
3. `finish_worker_tool` 对 PostgreSQL `40P01`（deadlock）和 `40001`
   （serialization failure）做有限次整事务重试；其他存储错误和任何 fence/CAS 错误仍
   fail closed。
4. 对已形成的 split-brain row 只开放一个窄自动修复：active tool 已 terminal failed、
   exact worker fence 仍匹配、工具是本地只读 `recon_list_providers`、attempt/lifetime budget
   仍可用时，旧 Worker supersede，稳定 WorkItem 重新入队。任何网络/副作用工具继续要求
   `manual_required`，绝不自动 replay。
5. 不删除历史 Worker、tool row 或旧运行组件；历史行保留审计，修复只追加新的 Worker
   attempt。

## 验收

- 同一 Team 的两个 WorkerRun 产生两个不同的 SubAgent UI identity。
- reasoning 在工具边界前落入时间线；零宽片段不再显示 `0.001s`。
- transient SQL transaction error 只触发有界 retry，不直接污染 lease 状态。
- terminal-failed `recon_list_providers` split row 可在下一次 producer claim 时重新入队；
  非 allowlist active tool 仍保持人工恢复。
