# golish-agent-runtime / agentic_loop

> **一句话职责**：每-turn 状态机的公开入口（薄表面 ≤150 LOC）——`run_agentic_loop*`；真正的 phase 调度在 `turn::run_turn_loop`，子模块托管各 phase 调用的 helper 与工具集成（HITL 审批 / loop detection / 上下文窗口 / 消息历史 / extended thinking 流式）。

- **类型**：目录模块（属于 crate [`golish-agent-runtime`](../golish-agent-runtime.md)）
- **路径**：`backend/crates/golish-agent-runtime/src/agentic_loop/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改流式 tool-call loop 主体（turn 调度、stream_processor、tool_execution、sub_agent_dispatch、compaction）时
- 改 loop 入口签名（`run_agentic_loop_unified` 等）或产物目录解析时

## 职责

agentic loop 的公开表面 + 子模块实现。`turn/` 是 phase 调度本体；`stream_processor/` 处理流式；`tool_execution/` loop 内工具；`sub_agent_dispatch` 派发 sub-agent；compaction 触发上下文压缩。

## 公开接口

| 符号 | 说明 |
|---|---|
| `run_agentic_loop` / `_generic` / `_unified` | loop 入口 |
| `AgenticLoopConfig` / `AgenticLoopContext` / `LoopLlmRefs` / `LoopEventRefs` / `LoopCaptureContext` / `LoopAccessControl` | 配置 + 依赖注入束 |
| `maybe_compact` / `apply_compaction` / `CompactionResult` | 压缩 |
| `get_transcript_dir(_for)` / `get_artifacts_dir(_for)` / `get_summaries_dir(_for)` | 产物目录 |
| `McpToolExecutor` / `OutputClassifier` / `PostShellHook` | 工具/分类/钩子 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 薄公开表面（≤150 LOC） |
| `turn/` | phase 调度本体（`run_turn_loop`） |
| `stream_processor/` / `tool_execution/` | 流式处理 / loop 内工具 |

## 依赖

- crate 内（`golish-agent-kit` 经 crate 根）；`rig`（completion/streaming）

## 注意事项 / 坑

- `mod.rs` 是**薄表面**，逻辑在 `turn::*`——改 loop 行为去 turn，别把实现堆回 mod.rs。
- 透传宽 context（crate 级 `allow(too_many_arguments)`）是有意为之。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime agentic_loop
```
