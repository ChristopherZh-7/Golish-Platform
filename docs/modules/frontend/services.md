# frontend / services

> **一句话职责**：前端事件服务层——`ai-events`（AI 事件处理器注册表：core/context/tool/task/workflow/sub-agent/misc handlers + session-sequence 排序）+ `terminal-events`（终端事件服务）。

- **类型**：前端子系统
- **路径**：`frontend/services/`（~14 ts）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 AI 事件 → store 的分发处理器（按事件类别）、事件序号/顺序处理时
- 改终端事件服务时

## 职责

把后端 `AiEvent` 流分发到 store 的处理层。`ai-events` 是处理器注册表（`eventHandlerRegistry` + `dispatchEvent`），按类别拆 handler（core/context/tool/task/workflow/sub-agent/misc）+ `session-sequence`（按 seq 有序处理，配合后端 `AiEventEnvelope`）。`terminal-events` 是终端事件服务工厂。

## 公开接口

| 符号 | 说明 |
|---|---|
| `eventHandlerRegistry` / `dispatchEvent` | AI 事件处理器注册表 + 分发 |
| `EventHandler` / `EventHandlerContext` / `EventHandlerRegistry`（类型） | 处理器契约 |
| `createTerminalEventService` / `TerminalEventService` | 终端事件服务 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `ai-events/registry.ts` | 处理器注册表 + dispatch |
| `ai-events/{core,context,tool,task,workflow,sub-agent,misc}-handlers.ts` | 按类别的事件处理器 |
| `ai-events/harness-handlers.ts` | harness/event → store；`stage_run_org_progress`解析所属requestId，Candidate/Reporting trace与`investigation_projection_changed`只写refresh hint |
| `ai-events/session-sequence.ts` | 按 seq 有序处理 |
| `terminal-events.ts` | 终端事件服务 |

## 依赖

- 消费 `store`（写状态）、`lib`（AI 事件类型/generated）；被 `hooks`（useAiEvents）调用

## 注意事项 / 坑

- **wire 契约对齐**：handler 处理的事件类型对应后端 `golish-core::events::AiEvent`（ts-rs 生成）；后端加事件变体要在此加 handler。
- `candidate_review_required/resumed`、`candidate_attempt_terminalized`、`attack_wave_consolidated` 都不是审批/Attempt/Wave authority；handler 只能更新已存在的 `Session.candidateReviewHint`，组件随后重读 DB API。terminal trace 用自己的 `wave_run_id`，consolidation trace 用 `source_wave_run_id` 做 exact match；operation/wave 不匹配或 hint 尚不存在时不得创建假 cursor，匹配时也必须保留现有 `resumeVersion`。
- Reporting 的 `gate_decision` / `deliverable_submitted` 也不是报告 authority；仅当 outer `stage=reporting` 时更新 `Session.reportingReadModelHint`，报告内容、CAS 与 finalization 一律由 scoped IPC/DB 重验。
- `investigation_projection_changed`必须在production registry显式注册；handler只转发operation/execution/request/change-seq四字段给store的monotonic exact-identity setter。event payload不是summary/control/transcript authority，duplicate/out-of-order/foreign hint不得触发latest read；有效gap也只能要求direct route做fresh no-seq bootstrap。
- `session-sequence` 依赖后端 envelope 的 seq 保证有序——别绕过它直接处理乱序事件。
- `tool_result.result.status === "backgrounded"`要走live/non-terminal路由：登记带exact origin（main requestId或sub-agent parentRequestId+child requestId）的`backgroundJobs`，timeline/streaming block标`backgrounded`并保留诊断用`initialYieldMs`与`automaticKill:false`；该状态表示initial yield后同一受管进程仍存活，不表示detach/respawn，也绝不解析或展示legacy hard deadline。每个output chunk刷新`lastOutputAt`，等`tool_background_completed`再按`job_id`翻终态。
- 高频流式 handler 不要直接写 store：`reasoning`、`sub_agent_reasoning`、`tool_output_chunk` 走 `EventHandlerContext` 的 batch 方法，由 `lib/ai/streaming-buffer.ts` 合并后写入，避免 detail thinking/output 同时刷新时卡顿。

## 测试入口

```bash
just check-fe
just test-fe   # vitest（含 ai-events registry 测试）
```
