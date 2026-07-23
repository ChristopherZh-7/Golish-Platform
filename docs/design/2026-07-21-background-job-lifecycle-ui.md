> Superseded by `docs/design/2026-07-23-codex-same-session-process-yield.md` for same-process initial/read yield semantics. This document remains the UI lifecycle history.

# 后台工具生命周期与事件驱动收口设计

日期：2026-07-21
关联功能：`background-job-lifecycle-ui-2026-07-21`

## 1. 问题

Golish 已经具备软超时转后台、实时输出、完成事件、原工具卡终态回填和取消入口，但当前体验仍有两个断层：

1. `ToolCallDetailView` 只有 `Backgrounded` 徽标和底部提示，缺少可读的后台任务生命周期；全局 `N running` 只列命令，不能跳回原工具，也没有最后活动、软/硬时间边界或一致的停止状态。
2. `submit_stage_deliverable` 默认立即返回 `needs_fix`，指示模型调用 `wait_for_background_jobs`，必要时再调用 `check_job`。因此模型承担了本应属于 runtime 的等待/轮询职责，时间线被控制面工具淹没。

现有完成链路本身是事件驱动的：后台 manager 广播 completion，bridge listener 完成 evidence/结构化落库并排入下一 Turn note，前端通过 `tool_background_completed` 回填原卡。因此本设计不新造第二套 job 系统，而是补齐读取模型，并让 closeout barrier 直接等待同一条 completion/reconciliation 事件。

## 2. 决策

- 后台任务始终属于发起它的原工具调用。UI 不创建“检查任务”的替代卡片。
- 原工具 Detail 显示专用后台面板：状态、job id、已运行时间、软超时原因、硬截止、最后输出、实时 stdout/stderr 和 Stop。
- 全局后台入口是导航索引：点击主 agent job 打开原 `tool-detail`；点击 sub-agent job 打开其父 `sub-agent-detail`。
- Stop 是请求状态：先显示 `Stopping…`，只有 completion event 到达后才显示最终 `killed/failed/done`；不能在信号发出时伪称“已杀死”。
- `submit_stage_deliverable` 的正常路径由 runtime 内部等待当前 session 的后台 job 终止且 completion side effects 已收口，再继续同一次 submit。模型不再收到“请轮询并重新提交”的正常修复提示。
- `check_job`、`wait_for_background_jobs`、`kill_job` 保留为恢复/诊断控制面；只有 reconciliation 超过其有界预算时才暴露给模型，不能成为正常调度循环。
- 不改 DB schema，不新增 Tauri command，不手改 generated types。

## 3. 前端读取模型

`BackgroundJob` 从三字段扩为会话内的轻量 origin/lifecycle 投影；同时把不会随 completion 覆盖而丢失的 `BackgroundRunMeta` 保存到原 main/sub-agent tool row：

```ts
interface BackgroundJob {
  jobId: string;
  command: string;
  toolName: string;
  origin:
    | { kind: "main_tool"; requestId: string }
    | { kind: "sub_agent_tool"; parentRequestId: string; requestId: string };
  startedAt: number;
  backgroundedAt: number;
  lastOutputAt?: number;
  softTimeoutMs?: number;
  hardTimeoutMs?: number;
  state: "running" | "stopping";
}

interface BackgroundRunMeta {
  jobId: string;
  backgroundedAt: number;
  softTimeoutMs?: number;
  hardTimeoutMs?: number;
}
```

`tool_result` 已携带 `request_id/tool_name/source`，sub-agent result 另携带 `parent_request_id`；后台结果再补 `hard_timeout_ms`。因此该读取模型完全由现有事件确定性构造，不需要新 IPC。

`tool_output_chunk` 按 `requestId` 更新 `lastOutputAt`。completion 仍删除全局 running 投影，并按 `job_id` 更新原卡，但保留原 row 的 `backgroundRun`，让历史 Detail 能说明它曾经进入后台。刷新后失去进程内 registry 的旧 backgrounded 卡继续按现有规则投影为 interrupted，不能凭 metadata 伪造仍在运行。

## 4. Detail 与全局入口

### 4.1 原工具 Detail

当 execution 的 `backgroundRun` 存在时，参数块后渲染后台面板：running 时合并当前 registry job；terminal 时使用 row 内保留的历史 metadata。

- `Running in background` / `Stopping…`
- `Background for`、`Last output`、`Deadline in`
- `Soft timeout after Ns` 与 job id
- Stop 按钮
- 原有 live output 区继续承载 stdout/stderr，不复制第二份输出 buffer

completion 到达后，同一 execution 进入既有 terminal Output/Error 视图；后台面板保留为 terminal 历史摘要，但不再显示 Stop、deadline countdown 或 live spinner。

### 4.2 Sub-agent Detail

按 `parentRequestId` 过滤属于当前 sub-agent 的 jobs。全局入口打开父 sub-agent Detail 后，另存 exact child tool request id；对应 tool row 自动展开、滚动到中间并短暂高亮。不能把 child tool id 塞进 parent detail stack，否则会落到 not-found。

### 4.3 全局入口

`BackgroundJobsBadge` 的每一行可点击。导航只使用事件携带的 exact request identity：

```text
main job      -> toolDetailRequestIds=[requestId], detailViewMode=tool-detail
sub-agent job -> toolDetailRequestIds=[parentRequestId], detailViewMode=sub-agent-detail
                 backgroundToolFocusRequestId=requestId
```

禁止按工具名或命令文本猜测归属。

## 5. 后端 reconciliation

后台 job 需要区分“进程已终止”和“completion side effects 已处理”。否则 submit 可能在 job terminal 后、evidence/业务表尚未落完时抢跑 gate。

`BackgroundJobManager` 为有 session 归属的 job 维护 reconciliation 状态，并提供 session 级事件等待：

```rust
pub async fn wait_for_session_reconciled(
    &self,
    session_id: &str,
    timeout: Duration,
) -> Vec<RunningJob>;

pub fn mark_reconciled(&self, job_id: &str);
```

manager 在 spawn/terminal 时发状态通知；bridge completion listener 完成 evidence、结构化输出、coverage outcome 和 background note 后调用 `mark_reconciled`。等待函数只有在该 session 不存在 `Running` 或 terminal-but-unreconciled job 时返回空列表。

`BackgroundJobsQuery` 暴露同一有界等待 seam。生产默认 reconciliation 预算覆盖普通后台 hard deadline，并允许环境变量收紧；若预算耗尽，返回 `needs_fix` 作为异常恢复，提示只检查一次，而不是建立轮询循环。

## 6. 取消语义边界

本功能不把现有 `kill_job` 的信号请求升级为“已终止”声明。前端调用成功只进入 `stopping`，最终状态仍由 `tool_background_completed` 决定。进程组 TERM→grace→KILL、Windows Job Object 和 cancellation reason 拆分属于同一 supervisor 方向，但需要独立的跨平台安全改造与测试，不在本次 UI/调度收口中顺手混入。

## 7. 验证

- 前端 focused Vitest：origin 注册、output activity、主/sub-agent 导航、Stop→Stopping、Detail 生命周期。
- 前端受影响文件 Biome + TypeScript typecheck。
- Rust focused nextest：manager session reconciliation 等待、listener mark、submit 正常内部等待和超时恢复。
- Rust 受影响 crate scoped Clippy 与 rustfmt。
- 不运行未获授权的 `init.sh`、`just precommit`、全量前后端测试或真实外部扫描。
