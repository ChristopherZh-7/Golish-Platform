# golish-agent-runtime

> **一句话职责**：**高层 agent runtime**（Layer 4b）——流式 tool-call loop（`run_agentic_loop*`）、sub-agent 派发、上下文压缩（compaction）、evals 评测 harness、mock test_utils。

- **类型**：crate（Layer 4b · agent 高层）
- **路径**：`backend/crates/golish-agent-runtime/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agentic loop 主体（流式 tool-call 循环、turn 处理、stream_processor）时
- 改上下文压缩 / 摘要触发（`maybe_compact` / `apply_compaction`）、transcript/artifacts/summaries 目录解析时
- 改 `stage_run` / sub-agent 派发 / harness repair checkpoint 传递时
- 改 evals 评测 harness（`eval_support`）或共享 mock（`test_utils`）时

## 职责

承载约 6.5 KLOC 的流式 loop 主体（从 `golish-agentic-loop` 在 A2 改名）。从 `golish-agent-kit`（L4a）拆出，是为了把 rig-core 重泛型实例化的 loop 与底层基础设施分开编译，恢复增量编辑。下游（bridge / app / evals）直接从这里 import loop 入口。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `run_agentic_loop` / `_generic` / `_unified` | 流式 tool-call loop 入口 |
| `AgenticLoopConfig` / `AgenticLoopContext` | loop 配置与上下文 |
| `LoopLlmRefs` / `LoopEventRefs` / `LoopCaptureContext` / `LoopAccessControl` | loop 依赖注入引用束 |
| `maybe_compact` / `apply_compaction` / `CompactionResult` | 上下文压缩 |
| `get_transcript_dir(_for)` / `get_artifacts_dir(_for)` / `get_summaries_dir(_for)` | 产物目录解析 |
| `McpToolExecutor` / `OutputClassifier` / `PostShellHook` / `TerminalErrorEmitted` | 工具执行/分类/钩子 |
| `eval_support` / `test_utils`（feature `test-utils`） | 评测 harness / 共享 mock |

## 依赖

- **内部**：`golish-agent-kit`（核心下层）、`golish-core`、`golish-context`、`golish-events`、`golish-llm-providers`、`golish-settings`、`golish-tools`、`golish-sub-agents`、`golish-prompts`、`golish-indexer`、`golish-json-repair`
- **外部**：`rig-core`、`rig-anthropic-vertex`、`rig-openai-responses`、`tokenx-rs`

## 被谁依赖 / 改动影响面

`golish-agent-bridge`、`golish-agent-app`、`golish`。改 loop 入口签名会波及 bridge 与 app 层。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `agentic_loop/` | 流式 loop 主体（turn/stream_processor） | [→](golish-agent-runtime/agentic_loop.md) |
| `execution_mode/` | 执行模式策略框架（policy/registry） | [→](golish-agent-runtime/execution_mode.md) |
| `eval_support/` | evals 评测 harness | [→](golish-agent-runtime/eval_support.md) |
| `test_utils/` | 共享 mock（feature test-utils） | [→](golish-agent-runtime/test_utils.md) |

## 关键文件

- `agentic_loop/tool_execution/direct/stage_run_call.rs`：Task harness `stage_run` per-org fan-out、gate retry、worker resume chain、stage-refiner repair checkpoint 传递。
- `agentic_loop/tool_execution/direct/sub_agent_call.rs`：`sub_agent_*` 派发、sub-agent repair checkpoint 恢复、tool observer。
- `test_utils.rs` / `test_utils_tests.rs`：feature gate 下的 mock 与自测。

## 注意事项 / 坑

- 与 `golish-agent-kit` 是**有意分家**（A2，编译预算）：底层在 kit，loop 在此。
- `test-utils` feature 才会编 `test_utils`（并拉 `tempfile`、传递给 `golish-agent-kit/test-utils`）；普通 release 不付出成本。
- crate 级 `#![allow(too_many_arguments)]`：loop 主体按设计透传宽 context。
- main-agent tool execution 会把 `event_tx` 通过 `golish_core::with_agent_tool_output_sender` 绑定到当前 `AgentToolContext`，这样 `browser_collect_js_api` / `js_extract_apis` 这类 bridge tools 可以在执行中发 `tool_output_chunk`，前端 Output 面板实时显示工具动态。新增绕过 `single_tool_call` 的执行路径时，要同步包上 tool context + output sender。
- `stage_run` 和 `sub_agent_call` 共享同一个 per-org `agent_path` checkpoint；`submit_stage_deliverable needs_fix` 里的 `SubmitRepairMode.coverage_gap_actions` 必须被 `stage_run` 接住并继续持久化，否则取消/重跑后会退化成泛化的 “without StageDeliverable” BLOCK，repair mode 就丢失精确 target/technique 工具清单。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime
```
