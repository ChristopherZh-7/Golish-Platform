# 实现计划 · P1-E（provider 韧性 3 增强）+ P2-G（可观测）收尾

> 配套设计：`docs/design/2026-06-03-harness-profile-driven-execution.md`（§3 E/G）。
> 父计划：`docs/superpowers/plans/2026-06-03-harness-profile-driven-execution-p0.md`（P0 + P1-C/D 已完成）。
> 执行规范：`.cursor/skills/executing-plans` + TDD（`.cursor/skills/test-driven-development`）。
> 日期：2026-06-03。

---

## 状态总览（2026-06-03）

| 项 | 设计 | 状态 |
|---|---|---|
| P1-C 子 agent 深度上限 | §3.C | ✅ 已完成（父计划） |
| P1-D 阶段工具裁剪 | §3.D | ✅ 已完成（父计划） |
| P1-E core（5xx/timeout/429 分类 + 退避 + 重试 + 终态错误 + 重复检测截停） | §3.E | ✅ 已存在（`agentic_loop/stream_retry.rs` + `stream_processor`） |
| **P1-E1 重复后 re-prompt 恢复** | §3.E② | ✅ 已完成（2026-06-03，TDD 3 测） |
| **P1-E2 mid-stream 错误重试** | §3.E① | ✅ 已完成（2026-06-03，同上测试套件） |
| **P1-E3 失败转移到备用模型** | §3.E① | ✅ 已完成（2026-06-03，bridge factory rebuild + re-dispatch，默认 OFF，7 决策单测） |
| P2-F 入口分诊 harness 化 | §3.F | ✅ 经独立 feature `task-mode-lead-agent-triage-2026-06-03` 落地（仅 blocked 于 precommit/E2E/commit） |
| **P2-G 可观测** | §3.G | ✅ 已完成（gate 决策 + graph-flow 游标推进 INFO 日志） |

---

## 实现记录（2026-06-03 · 用户「E1-E3 全做完」）

- **E1/E2**（`golish-agent-runtime/agentic_loop`）：`StreamProcessOutcome` 加
  `repetition_detected: bool` + `mid_stream_error: Option<String>`；`process_stream` 在
  重复 break 处置位、收尾把**可重试**的 mid-stream chunk error 透出（不再吞）；`TurnState`
  加 `repetition_recoveries` / `mid_stream_retries`（各上限 2，`MAX_*` 常量）；executor 在
  assistant_push 与 reflector 之间插入有界恢复块（注入纠正 re-prompt + `continue`），到顶
  接受 partial（不无限 spin）。仅 `!has_tool_calls` 时触发。
- **E3**（`golish-agent-bridge`）：核查发现 `CompletionRequest.model` 被各 rig fork 忽略
  （model 在 client 构造期固化，`rig-zai-sdk/conversion.rs:187`），故 request.model override
  方案作废；改在 bridge `execute_with_context_inner` 实现：主模型 run 返回**可恢复**错误（非
  取消/认证/上下文溢出）且配置了 `GOLISH_LLM_FALLBACK_MODEL`（默认空=OFF）且有
  `model_factory` 时，经 `LlmClientFactory::get_or_create(provider, fallback)` 重建客户端，对其
  再 `dispatch_llm_client_split!` 跑一次；主失败未 finalize history → 重试从同一状态起，不重发
  Started/UserMessage。决策逻辑抽到 `failover.rs`（纯函数，7 单测）。
- **验证**：`golish-agent-runtime` nextest **230/230**（含 3 E1/E2 集成测）；
  `golish-agent-bridge` `test(failover)` **7/7**；`clippy -p ...runtime ...bridge -D` 净；
  `fmt --check` 净；下游 `cargo check -p golish-agent-app` exit 0。
- **诚实边界（未做）**：活体复跑（真触发重复/断流/失败转移看恢复）需 runtime + LLM key；
  E3 仅覆盖主文本路径（`execute_with_context_inner`），多模态 vision 路径未接（niche）；
  全量 `just precommit` 未跑；未 commit。

本计划聚焦剩余的 **P1-E1/E2/E3**（均在 streaming 热路径，影响每一次 agent 迭代，按风险从低到高排序执行）。

---

## 现状精确落点（本会话读真码核对）

- **重复检测（已存在，截停但不恢复）**：`stream_processor/chunks.rs::handle_text_chunk` 每累计 200 字符调一次
  `sub_agent_dispatch::detect_repetitive_text`，命中即 `warn!("Repetitive text detected, stopping generation")`
  并返回 `true`；`stream_processor/mod.rs:163-175` 据此 `break` 流循环。**问题**：`StreamProcessOutcome`
  里没有"本次因重复而中断"的信号，外层 turn 循环把那段带重复的 partial 文本当正常结果用，**不重试**。
- **mid-stream 错误（已存在，吞掉）**：`stream_processor/mod.rs:313-316` 把流中途的 chunk error 仅
  `warn!` 并存进 `last_stream_chunk_error`；仅当**整段无任何可用内容**时（:357-368）才 surface 成 `Error` 并
  `BreakAgentLoop`。**问题**：partial 文本 + 中途错误 → 错误被静默吞掉，不重试本次 stream。
- **stream start 重试（已存在）**：`stream_retry.rs` 对**启动期**错误做分类 + 退避 + 3 次重试，但只重试
  **同一模型**，无失败转移。
- **turn 编排**：`agentic_loop/turn/phases/completion.rs`（跑 stream → `CompletionOutcome::Continue{outcome}`）
  + `reflector_or_break.rs`（无 tool 调用时的 reflector 纠偏，已有"注入 User 纠正消息 + continue"范式）。
  E1 的恢复注入应复用此范式。

---

## 步骤（TDD · 每步红→绿；全部 streaming 热路径，单 crate `golish-agent-runtime`）

### E1 · 重复后 re-prompt 恢复（风险中，最高价值，无开放问题 → 先做）
- 落点：`stream_processor/{mod.rs,chunks.rs}` + turn 编排（`completion.rs` 或 `reflector_or_break.rs`）。
- 改：
  1. `StreamProcessOutcome` 加 `repetition_detected: bool`；`process_stream` 在"因 `handle_text_chunk`
     返回 true 而 break"的分支置位（与正常 `chunk == None` 流结束区分）。
  2. turn 循环：`repetition_detected && recovery_budget 未耗尽` 时，**注入一条纠正 User 消息**
     （"你上一段在重复复述，别复述、直接给结论/下一步动作"）并 `continue`（重跑一次 iteration）；
     预算上限（建议 2 次/run，仿 reflector 的 `total_reflector_nudges<3`），到顶 → 保留现行为
     （用 partial 收尾，不再无限重试）。
- TDD：脚本化 model 连发"重复文本"chunk → 断言注入了恢复 re-prompt + 重跑一次；到预算上限后不再重试。
- 风险：中（热路径，但加预算上限 + 复用 reflector 注入范式，可控）。

### E2 · mid-stream 错误重试（风险中）
- 落点：`stream_processor/mod.rs`（流循环 `Err(e)` 分支 + 收尾判定）+ 复用 `stream_retry` 分类/退避。
- 改：流中途出现**可重试**错误（`classify_stream_start_error(err).retriable`）且本次尚无可用 tool_call
  时，结束本段后由 turn 层**重试整段 stream**（与 start 重试共享 `STREAM_START_MAX_ATTEMPTS` 预算）；
  partial 文本 + 中途错误不再静默吞掉——要么重试，要么 surface。不可重试错误维持现状（surface + break）。
- TDD：脚本化 model 在第 N 个 chunk 后吐可重试错误 → 断言重试；不可重试 → 断言 surface 不重试。
- 风险：中（要小心"已 emit 的 TextDelta 不要重复 emit 给前端"——重试段需重置 accumulated 或标记去重）。

### E3 · 失败转移到备用模型（风险高 · 有开放问题，最后做）
- 落点：`stream_retry.rs` 重试循环 + LLM client 选择层。
- 改：start 重试预算耗尽（或命中 `model_unavailable`）→ 切换到**备用模型/通道**重试一次；默认
  **OFF**（无配置 = 现行为，零回归），配置存在才启用。
- TDD：主模型连续失败 + 配置了 backup → 断言切到 backup 重试；无 backup 配置 → 现行为。
- 风险：高（跨模型客户端构造；需 settings/env 配置面）。**先拍板开放问题再实现。**

### 收尾
- 每步：`cargo nextest run -p golish-agent-runtime` 全绿 + `cargo clippy -p golish-agent-runtime -- -D warnings`
  + `cargo fmt -p golish-agent-runtime --check` + 下游 `cargo check -p golish-agent-app`。
- 全部完成后：`just precommit` + 活体复跑（重复 → 恢复 / 断流 → 重试）证据贴 `agent-progress.md`。

---

## 影响面 / 回滚
- 改动集中 `golish-agent-runtime/src/agentic_loop`（stream_processor + turn phases + stream_retry）。
- E1/E2 加恢复**预算上限**，到顶即回退现行为（partial 收尾 / surface error）——"宁停不假过"。
- E3 默认 OFF，无配置零回归。

## 开放问题（实现前需拍板）
1. **E3 备用模型来源**：用户设置项（settings UI）/ env 降级链 / 内置同 provider 次选模型？建议先
   **env 降级链**（`GOLISH_LLM_FALLBACK_MODEL`，默认空=OFF），settings UI 后补。
2. E1/E2 恢复预算上限取值（建议各 2 次/run）。
3. E2 重试时已 emit 的 `TextDelta` 去重策略（重置 accumulated vs 前端去重标记）。
