# 后台工具生命周期与事件驱动收口 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让后台命令在原工具 Detail 中拥有完整、可停止的生命周期视图，并让 stage closeout 由 runtime 等待 completion reconciliation，而不是让模型反复调用 check/wait 工具。

**架构：** 前端扩展现有 `BackgroundJob` 会话读取模型，复用 `tool_output_chunk` 与 `tool_background_completed` 驱动 UI；后端在现有 background manager 与 completion listener 之间增加 reconciliation ack，`submit_stage_deliverable` 等待该 ack 后继续同一次提交。控制面工具保留为异常恢复，不再是正常路径。

**技术栈：** Rust 2021、Tokio broadcast/Notify、React 19、TypeScript 6、Zustand、Vitest、Tailwind 4。

## 文件结构

- 修改 `backend/crates/golish-app-core/src/background_jobs.rs`：session reconciliation 状态、事件等待与 focused tests。
- 修改 `backend/crates/golish-app-core/src/pty_interactive.rs`：backgrounded result 暴露 hard deadline 元数据，更新非轮询提示。
- 修改 `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`：`BackgroundJobsQuery` 改为有界 event-driven reconciliation，正常 submit 不返回 wait repair。
- 修改 `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：completion side effects 后 ack manager；生产 wiring 与 focused tests。
- 新增 `frontend/store/types/background-job.ts`，修改 `frontend/store/types/{message,sub-agent,session,index}.ts`：共享 origin/lifecycle metadata、原 tool row 历史 metadata 与 sub-agent exact focus。
- 修改 `frontend/store/slices/{ai,session-core,workflow/sub-agent}.ts`：扩展 `BackgroundJob` origin/lifecycle 投影、保留 `backgroundRun`、activity/stopping actions 与 exact focus 生命周期。
- 修改 `frontend/services/ai-events/{tool-handlers,sub-agent-handlers}.ts`：从 exact event identity 注册 job，并在 output/completion 更新读取模型。
- 新增 `frontend/components/BackgroundJobPanel/BackgroundJobPanel.tsx`：共享后台生命周期面板。
- 新增 `frontend/components/BackgroundJobPanel/BackgroundJobPanel.test.tsx`：状态、deadline 与 Stop 测试。
- 修改 `frontend/components/UnifiedInput/{StatusBadges,InputStatusRow}.tsx` 与测试：可导航的全局 job 索引。
- 修改 `frontend/components/{ToolCallDetailView,SubAgentDetailView}/*.tsx` 与 focused tests：主 Detail 接入后台面板；sub-agent exact tool 自动展开、滚动和高亮。
- 修改 `frontend/lib/i18n/{en,zh-CN}.json`：后台生命周期文案。
- 修改相关 `docs/modules/` 卡片、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`：同步事实与证据。

## 任务 1：锁定前端读取模型与事件行为

**文件：**

- 修改：`frontend/store/slices/ai.ts`
- 新增：`frontend/store/types/background-job.ts`
- 修改：`frontend/store/types/message.ts`
- 修改：`frontend/store/types/sub-agent.ts`
- 修改：`frontend/store/types/session.ts`
- 修改：`frontend/store/types/index.ts`
- 修改：`frontend/store/slices/session-core.ts`
- 修改：`frontend/store/slices/workflow/sub-agent.ts`
- 修改：`frontend/services/ai-events/tool-handlers.ts`
- 修改：`frontend/services/ai-events/sub-agent-handlers.ts`
- 测试：`frontend/services/ai-events/registry.test.ts`

**步骤 1：编写失败测试**

增加 main/sub-agent background result 断言，要求 job 保存 exact `requestId/toolName/source/parentRequestId`、soft/hard timeout，同时原 tool row 保存 `backgroundRun`；增加 completion 后历史 metadata 不丢、output chunk 后 `lastOutputAt` 更新、Stop action 进入 `stopping` 的 store 断言。

**步骤 2：运行 RED**

```bash
pnpm vitest run frontend/services/ai-events/registry.test.ts
```

预期：新 origin/lifecycle 字段断言失败。

**步骤 3：实现最小读取模型**

按设计定义 `BackgroundJobOrigin` / `BackgroundRunMeta` / `BackgroundJob`，把注册函数签名改为：

```ts
registerBackgroundJobFromResult(state, sessionId, result, {
  requestId,
  toolName,
  source,
  parentRequestId,
});
```

新增 `markBackgroundJobOutput` 与 `setBackgroundJobState` actions；`handleToolOutputChunk` 在 batch output 外同步 touch exact request。main timeline 与 sub-agent tool 的 background transition 写入 `backgroundRun`，terminal 更新保留该字段。session 离开 detail 时清理 `backgroundToolFocusRequestId`。

**步骤 4：运行 GREEN**

重复 focused Vitest，预期全绿。

## 任务 2：实现共享后台生命周期面板与全局导航

**文件：**

- 新增：`frontend/components/BackgroundJobPanel/BackgroundJobPanel.tsx`
- 新增：`frontend/components/BackgroundJobPanel/BackgroundJobPanel.test.tsx`
- 修改：`frontend/components/UnifiedInput/StatusBadges.tsx`
- 修改：`frontend/components/UnifiedInput/StatusBadges.test.tsx`
- 修改：`frontend/components/UnifiedInput/InputStatusRow.tsx`
- 修改：`frontend/lib/i18n/en.json`
- 修改：`frontend/lib/i18n/zh-CN.json`

**步骤 1：编写失败测试**

覆盖：running/stopping 文案、deadline/last activity、Stop 成功后 store state、main job 点击进入 `tool-detail`、sub-agent job 点击进入 `sub-agent-detail`。

**步骤 2：运行 RED**

```bash
pnpm vitest run frontend/components/BackgroundJobPanel/BackgroundJobPanel.test.tsx frontend/components/UnifiedInput/StatusBadges.test.tsx
```

**步骤 3：实现组件**

面板只消费 store 投影和 `cancelBackgroundJob`；API 返回 true 后调用：

```ts
useStore.getState().setBackgroundJobState(sessionId, job.jobId, "stopping");
```

Badge 行点击按 exact origin 设置 detail stack；Stop 按钮必须 `stopPropagation()`，不能触发导航。

**步骤 4：运行 GREEN**

重复 focused Vitest，预期全绿。

## 任务 3：接入主工具与 Sub-agent Detail

**文件：**

- 修改：`frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- 修改：`frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts`
- 修改：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
- 修改：`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`

**步骤 1：编写失败测试**

主工具按 `requestId` 只显示自己的 job；completion 后仍显示历史 background lifecycle 且原 result 终态可见；sub-agent badge 导航打开 exact parent 后，child tool 自动展开、滚动并高亮，不能把 child id 当 parent id。

**步骤 2：运行 RED**

```bash
pnpm vitest run frontend/components/ToolCallDetailView/ToolCallDetailView.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

**步骤 3：实现接线**

在主 Detail metadata 后插入 `BackgroundJobPanel`，继续复用原 live output 区，不复制 stdout/stderr buffer。Sub-agent Detail 消费 `backgroundToolFocusRequestId` 聚焦 exact tool row；所有 `BackgroundJobsBadge` 传入 `sessionId` 以启用 exact navigation。

**步骤 4：运行 GREEN**

重复 focused Vitest，预期全绿。

## 任务 4：锁定 manager reconciliation 协议

**文件：**

- 修改：`backend/crates/golish-app-core/src/background_jobs.rs`
- 修改：`backend/crates/golish-app-core/src/pty_interactive.rs`

**步骤 1：运行空间守卫**

```bash
just space-guard
```

**步骤 2：编写失败测试**

真子进程测试要求：terminal 但未 ack 时 session wait 仍 pending；`mark_reconciled` 后立即返回；其它 session 不互相阻塞；backgrounded result 含 `hard_timeout_ms`。

**步骤 3：运行 RED**

```bash
cd backend && cargo nextest run -p golish-app-core -E 'test(session_reconciliation_) | test(background_command)'
```

**步骤 4：实现协议**

在 `JobState` 保存 reconciliation 标志，在 manager 保存 session 状态通知；实现：

```rust
pub async fn wait_for_session_reconciled(
    &self,
    session_id: &str,
    timeout: Duration,
) -> Vec<RunningJob>;
pub fn mark_reconciled(&self, job_id: &str);
```

状态检查与订阅必须避免“检查为空后才订阅”的 lost wakeup；超时返回仍 pending 的 exact jobs。

**步骤 5：运行 GREEN**

重复 focused nextest，预期全绿。

## 任务 5：让 submit 正常路径内部等待并在落库后继续

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`

**步骤 1：运行空间守卫**

```bash
just space-guard
```

**步骤 2：编写失败测试**

覆盖：默认预算不是 0；job settle+ack 后同一次 submit accepted；预算耗尽才返回 recovery `needs_fix`；completion listener 的 UI event/note 在 side effects 后发布并 ack manager。

**步骤 3：运行 RED**

```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(submit_reconcile_) | test(submit_proceeds_after_background_jobs_settle) | test(background_completion_)'
```

**步骤 4：实现 event-driven seam**

`BackgroundJobsQuery` 接受 `timeout` 并返回 exact pending jobs；生产 adapter 委派 manager。completion listener 在所有 evidence/structured/outcome/note 处理后发送 terminal UI event，最后调用 `mark_reconciled`。正常返回空列表时 submit 直接继续 gate preview，不生成 BackgroundJobs repair mode。

**步骤 5：运行 GREEN**

重复 focused nextest，预期全绿。

## 任务 6：定向检查与文档收口

**文件：**

- 修改：`docs/modules/backend/golish-app-core.md`
- 修改：`docs/modules/backend/golish-agent-app/ai.md`
- 修改：`docs/modules/frontend/{store,services,components}.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`feature_list.json`
- 修改：`agent-progress.md`

**步骤 1：前端检查**

```bash
pnpm biome check frontend/store/slices/ai.ts frontend/services/ai-events/tool-handlers.ts frontend/services/ai-events/sub-agent-handlers.ts frontend/components/BackgroundJobPanel frontend/components/UnifiedInput/StatusBadges.tsx frontend/components/UnifiedInput/InputStatusRow.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/SubAgentDetailView/SubAgentDetailView.tsx
pnpm typecheck
```

**步骤 2：Rust 检查**

```bash
just space-guard
cd backend && cargo clippy -p golish-app-core -p golish-agent-app --lib --tests -- -D warnings
cd backend && cargo fmt -p golish-app-core -p golish-agent-app -- --check
```

**步骤 3：元数据检查**

```bash
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json
git diff --check
```

**步骤 4：记录证据**

将每条实际命令、退出码、关键 passed 数写入 `agent-progress.md` 和 feature `evidence`。只有全部定向证据覆盖本次行为时才把功能改为 `passing`；未获授权的全量门禁如实记录未运行。
