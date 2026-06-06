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
| `ai-events/session-sequence.ts` | 按 seq 有序处理 |
| `terminal-events.ts` | 终端事件服务 |

## 依赖

- 消费 `store`（写状态）、`lib`（AI 事件类型/generated）；被 `hooks`（useAiEvents）调用

## 注意事项 / 坑

- **wire 契约对齐**：handler 处理的事件类型对应后端 `golish-core::events::AiEvent`（ts-rs 生成）；后端加事件变体要在此加 handler。
- `session-sequence` 依赖后端 envelope 的 seq 保证有序——别绕过它直接处理乱序事件。

## 测试入口

```bash
just check-fe
just test-fe   # vitest（含 ai-events registry 测试）
```
