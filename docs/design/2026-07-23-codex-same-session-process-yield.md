# Codex 同会话受管进程与可调 yield 设计

> Extended by `2026-08-10-codex-style-durable-continuation.md`: generic jobs
> still have no elapsed watchdog, while an exact security producer may install
> a versioned server-owned policy deadline and durable launch admission.

## 决策

这不是 EAS 特例。所有由 AI 直接启动、可能长时间运行的本地命令进程统一采用一个生命周期合同：进程从创建起就由 server 进程管理器持有；一次工具调用只在可调 `yield_time_ms` 窗口内等待输出或退出；窗口结束而进程仍存活时，返回同一个 `job_id` 并把控制权交还 AI。它不是重新启动、detach 或“30 秒后搬到后台”，也不会因等待窗口结束而 kill。

AI 随后可以对同一个 `job_id` 再次进行有界读取，查看进程是否存活、累计 stdout/stderr、最近一次输出和 tail；也可以显式 `kill_job`。只有自然退出、AI/用户显式取消、会话取消、spawn/wait/output 基础设施失败能够改变进程寿命。

当前 Codex 产品语义与此一致：后台 terminal 可以被查看和显式停止；空 `write_stdin` 的最大等待配置限制一次 poll，而不是进程寿命。Golish 对齐的是这个控制模型，不复制某个固定秒数。

## 适用边界

统一合同覆盖：

- 主 agent 与 sub-agent 的 `run_pty_cmd` / `run_command`；
- 普通 `pentest_run`；
- 使用 server-owned typed reconciler 的 EAS 等安全 wrapper 子进程；
- `check_job` / `wait_for_background_jobs` 对这些进程的后续读取。

以下 timeout 保留，因为它们不是进程墙钟寿命：HTTP connect/request timeout、scanner 单 socket timeout、DB lock/acquire timeout、LLM stream-start timeout、输出 pipe drain 安全上限，以及必须同步取得不可重放 receipt 的原子动作。保留项必须在代码和文档中明确命名为 I/O、协议、资源或事务边界，不能再叫 process timeout。

不能安全 detached 的 Candidate action / cleanup side effect 仍保持同步 authority；它们同样不得因通用 elapsed watchdog 自动 kill，但不会伪造一个缺少 durable completion authority 的后台 handle。

## 调用合同

### 首次执行

- 首选参数为 `yield_time_ms`，默认 10,000ms，范围 250..30,000ms；这与 Codex `exec_command` 的 bounded yield 语义一致。
- 旧 `timeout` / `timeout_secs` 仅作输入兼容并映射为一次 initial yield；结果和新文档不再称其为 timeout。
- 旧 `background:true` 只作兼容的短 startup yield，不创建第二种进程类型。所有命令从创建起都是同一种 managed process。
- 到点仍存活返回兼容状态 `backgrounded` 与 `job_id`，同时返回 `initial_yield_ms`、activity 和 `automatic_kill=false`。`backgrounded` 只是 UI/事件兼容标签，不表示发生了重新派生进程。

### 后续读取

- `check_job(job_id, yield_time_ms?)` 对同一进程做一次有界读取；默认 10,000ms，0 表示立即 snapshot，最大 300,000ms。
- 当进程终止、产生新输出或本次 yield 结束时返回，包含 `poll_reason=terminal|output|yield_elapsed|snapshot`。
- `wait_for_background_jobs` 是聚合恢复工具；它的 timeout/idle 也只结束一次读取，不终止任何进程。
- quiet 不等于 hung。模型结合工作量、存活、累计字节、最近输出和业务 slow-SLO 判断；只有它或用户显式调用 `kill_job` 才取消。

## 全局路由

sub-agent 不再绕过共享 tool registry 调用旧的 `golish-shell-exec::execute_streaming` timeout/kill 路径。`run_pty_cmd` 必须经过已安装的 `VisibleRunPtyCmdTool`，从而与主 agent 共用 manager、session attribution、输出流、completion reconciliation 与 controls。

任何允许 `run_pty_cmd` 或 `pentest_run` 的 agent 角色必须同时拥有 `check_job` 和 `kill_job`；需要聚合等待的 stage 角色再拥有 `wait_for_background_jobs`。权限仍由原 stage/candidate boundary约束，process controls 不扩大 target scope。

## EAS 与证据隔离

EAS v3 的 `PortScanPlan`、coverage、manifest、attestation 和 Gate 不再包含或验证固定 30 秒 initial wait。transport yield 不属于端口覆盖事实，也不能成为业务 profile identity。

v3 仍严格绑定 server recipe、exact authorized hosts/ports、完整 spool hash、receipt、`termination_reason=exited` 与 `automatic_kill=false`。历史 v1/v2 evidence 保持原 strict read compatibility。slow-SLO 只用于提示 AI 评估活动，不自动 kill。

## UI 与文案

- 删除“30 秒后转后台”“inline handoff”“soft timeout”等产品承诺。
- running card 表达为“受管进程仍在运行；不会自动停止”，显示 elapsed、最近输出和累计字节。
- initial yield 是 transport metadata，不在 EAS coverage/Gate 展示；需要诊断时可从 tool result/run log 读取。
- stop 按钮和 `kill_job` 都是显式 operator cancellation，并显示 server-authored termination reason。

## 验证

使用本地短进程和 fixtures，不对外部目标扫描：

- 任意 initial/read yield 结束后 PID/job_id 不变且进程仍存活；
- `check_job` 在 output/terminal/yield 三种条件正确返回且从不 kill；
- sub-agent shell route 经过 registry，不再调用旧 timeout executor；
- 所有 raw shell/pentest 角色具有 read/kill controls；
- EAS profile/attestation/Gate 不含固定 wait，并仍严格拒绝不完整、取消或篡改输出；
- UI 不再显示固定后台化秒数。

跨 app crash 恢复 OS child 仍不在本期；进程内 manager 消失后 DB terminal truth保持 pending，由原 worklist 安全重试。
