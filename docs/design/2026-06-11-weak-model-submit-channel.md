# 弱模型提交通道收紧：定向 repair + 锁死 submit_stage_deliverable

> 日期：2026-06-11
> 状态：已实现
> 关联：`docs/design/2026-06-09-active-stage-verify-first.md`（gate fail-closed）、
> `docs/design/2026-06-10-gate-integrity-closure.md`、阶段 playbook（execute.rs 2026-06-11 注入）

## 1. 问题

实测（MiMo `mimo-v2.5-pro`，moresec.cn，2026-06-11 stage-run）：target_intel 阶段
**工具都跑了、evidence 已进账本**，但 agent 结尾输出散文、没有调
`submit_stage_deliverable` → gate 正确 fail-closed BLOCK → reflector repair 重试。

坏在重试的形态：现有 `missing_deliverable_gate_outcome` 的纠正文案是
"Re-do the stage work as needed and resubmit"——弱模型会**把整个阶段重做一遍**
（重新跑工具、重新烧 token），实测多烧 ~6min（02:24 BLOCK → 02:30 PASS）。
而事实上活已经干完，**只差最后一个提交动作**。

## 2. 方案（两个确定性小改，不依赖模型变好，不伪造 finding）

### 改 1 · 定向 repair（evidence 注入纠正提示）

阶段因「无可解析 StageDeliverable」被 BLOCK 时，**查证据账本**
（`repo.recent_evidence_ids(chat_session_id, 25)` + `evidence_kinds_for`）：

- **账本里有真实 evidence**（说明活干了，只差提交）→ 纠正文案改为定向版：
  「你的扫描工作已记录为 evidence ids #1632(dns_a), #1634(http_probe)…，
  **不要重跑任何工具**，唯一剩余动作 = 调 `submit_stage_deliverable`
  把这些 id 填进 evidence_refs / claims」。弱模型只要 echo id，不用重做阶段。
- **账本为空**（真的没干活）→ 保留原"重做并提交"纠正，不误导。

同时把查到的 id 写进 `outcome.available_real_ids`，HarnessTrace GateDecision
事件可见（与 fabricated-evidence 路径同字段，观测一致）。

### 改 2 · 锁死提交通道（tool_choice → 指定 submit_stage_deliverable）

**仅**当改 1 判定「活已干完、只差提交」时，该次 repair 重试 pass 把
`tool_choice` 从笼统 `Required` 收紧为**指定工具**：

- rig-core 0.36 `message::ToolChoice::Specific { function_names }` 已支持；
- **Anthropic 协议**（RigXiaomiAnthropic）：rig 原生转 `{"type":"tool","name":…}`；
- **OpenAI 协议**（RigXiaomi）：rig-core 的 openai provider 对 Specific **硬报错**
  （`Provider doesn't support only using specific tools`）——绕道：请求体
  `tool_choice` 字段留空，改经 `additional_params`（serde flatten 顶层合并）注入
  OpenAI 线格式 `{"tool_choice":{"type":"function","function":{"name":…}}}`；
- 提交后立刻释放：`stage_deliverable_submitted` 置位后回落 `Required`，
  避免循环里被迫重复提交；
- 作用域与现有 Required 强制完全一致：仅 `FORCE_REQUIRED_TOOL_CHOICE_PROVIDERS`
  （xiaomi）× harness stage × depth-0 primary × 工具表含 `submit_stage_deliverable`。

### 端点实测（2026-06-11，区域 Cn，model=mimo-v2.5-pro）

| 端点 | tool_choice 线格式 | 结果 |
|---|---|---|
| `…/v1/chat/completions` | `{"type":"function","function":{"name":"submit_stage_deliverable"}}` | HTTP 200，`finish_reason=tool_calls`，恰好调了指定工具 ✅ |
| `…/anthropic/v1/messages` | `{"type":"tool","name":"submit_stage_deliverable"}` | HTTP 200，`stop_reason=tool_use`，恰好调了指定工具 ✅ |

## 3. 为什么不按「min_invocations 满足就锁」

若以 spec `min_invocations`（如 EAS 仅 `http_probe:1`）作为活体锁定触发器：
模型跑完一次 httpx 就被锁进 submit-only，**再也跑不了 naabu/nmap/gowitness**，
而 `coverage_complete` gate 又要求逐资产 PORT/SERVICE 终态 → 死锁
（锁着提交 → gate 拦覆盖不全 → 没法跑工具补覆盖）。

改用「missing-deliverable BLOCK + 账本有真实 evidence」做触发器是自愈的：
锁定 pass 提交后若 gate 再以**内容原因**（覆盖不全等）BLOCK，下一轮纠正
不再是 missing-deliverable 类型 → 不锁 → 模型可以正常跑工具补活。

## 4. 改动面

| crate | 文件 | 改动 |
|---|---|---|
| golish-agent-kit | `task_orchestrator/subtask_phases/execute.rs` | `HarnessGateOutcome.missing_deliverable` 标记；`refine_missing_deliverable_correction`（查账本→定向纠正→返回 submit-only 判定）；repair 重试 pass 携带 `harness_submit_only` 的 exec_ctx |
| golish-agent-kit | `task_orchestrator/types.rs` | `ExecutionContext.harness_submit_only: bool` |
| golish-agent-bridge | `agent_bridge/mod.rs` / `constructors/mod.rs` / `prepare.rs` / `bridge_executor/trait_impl.rs` | 侧通道 `harness_submit_only: Arc<RwLock<bool>>`（沿用 `harness_active_stage` 模式）发布→注入 loop context |
| golish-agent-runtime | `agentic_loop/context.rs` | `AgenticLoopContext.harness_submit_only: bool` |
| golish-agent-runtime | `agentic_loop/turn/phases/completion.rs` | 计算 `submit_only = ctx.harness_submit_only && !state.stage_deliverable_submitted` 传入 stream start |
| golish-agent-runtime | `agentic_loop/llm_stream_start.rs` | `resolve_stage_tool_choice` 增 submit-only 分支（Specific）；OpenAI 协议经 additional_params 注入；Anthropic 原生 Specific |

## 5. 不变量

- gate 仍 fail-closed；本设计**不放宽任何 gate 判定**，只缩短 BLOCK→PASS 的重试路径。
- 定向纠正只引用账本里真实存在的 evidence id（I7：阶段交付必须有 evidence）。
- 账本为空时绝不暗示"活已干完"（不伪造）。
- 非 xiaomi provider / 非 harness stage / sub-agent 行为零变化。
