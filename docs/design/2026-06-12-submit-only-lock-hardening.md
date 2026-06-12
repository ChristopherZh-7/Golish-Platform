# Submit-only 锁加固：dispatch 层闭锁 + prompt 级硬约束

> 状态：实现中（2026-06-12）。承接 `2026-06-12` session_id 统一修复后的活体观察。

## 1. 背景

统一 refiner（`2026-06-12-unified-refiner`）的 A 类 submit-only 锁：当某 stage 的扫描工作**已落账**（账本有真实 evidence id）但 agent 漏交 / 交了无法解析的 StageDeliverable 时，gate BLOCK → refiner 判 `SubmitOnly` → 下一轮把 turn 的 `tool_choice` 锁到 `submit_stage_deliverable`，并把「别重做、用这些真实 id 提交」的纠正文本回灌。

session_id 修复后首次在 headless 活体观察到该锁真触发（xiaomi/mimo 跑默安 moresec.cn）。但 stage 仍 BLOCK，transcript 实证暴露**两个锁的逃逸缝**：

1. **dispatch 层未闭锁**：submit-only 当前只在 `llm_stream_start` 给 LLM 请求带 `tool_choice`（API 层强制）。MiMo 不走原生 function-call，而是把工具调用写进普通文本（XML 风格），被 `tool-adapter` 事后恢复成结构化调用——**这条路径绕过 API 层 tool_choice**。实证：锁定轮 MiMo 吐出的是 `update_plan`（textual-tool-call-1-0），照样被 dispatch 执行。正门锁了、侧门没锁。

2. **prompt 层无硬约束**：对 xiaomi/MiMo，API 层 `tool_choice`（含 named tool_choice via `additional_params`）形同虚设（模型不理）。系统从未在模型**看得见的文字**里下达「只准提交」的硬指令。

（最终该轮 MiMo 在 5 个 iteration 烧 80k+ thinking token、输出反刍被截断三次、零产出——纯模型能力问题，非本设计能根治；但上述两缝是系统侧能且应该堵死的。）

## 2. 目标

把 submit-only 的强制力从「单点 API tool_choice」升级为**三道防御**：
- A（既有）API 层 `tool_choice` → submit_stage_deliverable。
- B（本设计①）**dispatch 层闭锁**：submit-only 锁生效期间，任何非 `submit_stage_deliverable` 的工具调用（含 textual-adapter 恢复的）在真正执行前被拒绝 + 回灌定向纠正，不下达执行器。
- C（本设计②）**prompt 层硬约束**：submit-only 轮在系统提示里追加强指令「你这一轮的全部输出必须且只能是一个 submit_stage_deliverable 调用」，对忽略 API tool_choice 的 provider 兜底。

## 3. 设计

### ① dispatch 层闭锁（`turn/phases/tool_dispatch.rs` + `turn/executor.rs`）

- `executor.rs`：在 ToolDispatch 之前算 `submit_only_lock = ctx.harness_submit_only && !turn_state.stage_deliverable_submitted`，**在** line 270「批次含 submit 则置 stage_deliverable_submitted=true」**之前**取值（保证一个批次里 submit+update_plan 仍能放过 submit、拒掉 update_plan）。作为新参数传入 `tool_dispatch_phase::run`。
- `tool_dispatch.rs`：`run` 开头，当 `submit_only_lock` 为真，先把批次按 `name == submit_stage_deliverable` 切成 (submit, blocked)；blocked 全部走 `push_submit_only_rejections`（每个 blocked call 配一条 ToolResult，保持 assistant tool_call ↔ tool_result 配对不破，避免 provider 报错），只把 submit 部分往下走原有 gate/allow-list/dispatch。
- 纯函数 `split_for_submit_only` + 消息 helper，便于单测。

### ② prompt 层硬约束（`llm_stream_start.rs`）

- `start_completion_stream` 已持有 `submit_only`。新增纯函数 `compose_system_prompt(system_prompt, submit_only)`：submit_only 为真时在系统提示末尾追加 `SUBMIT_ONLY_PROMPT_DIRECTIVE`（强指令），否则原样返回。
- 普通路径（preamble）与 NVIDIA 首条 user message 路径都用它，确保所有 provider 一致注入。对遵从 API tool_choice 的 provider 是无害冗余。

## 4. 不变量 / 红线

- 锁只在 `harness_submit_only` 且未提交时生效；非 submit-only 轮零行为变化。
- blocked 调用必须回灌 ToolResult（配对完整性）。
- 不改 refiner 文本（agent-kit，GUI 共用）；本设计纯 runtime 层加固。
- 不放松 gate：dispatch 闭锁只是更早地阻止「锁定期跑别的工具」，gate 终判不变。

## 5. 验证

- 单测：`split_for_submit_only` 切分；`compose_system_prompt` 注入与否；dispatch `run` 在 submit_only_lock 下拒非 submit（含本应在 allow-list 的 update_plan）+ 放过 submit。
- 静态门禁：`cargo check / clippy -D / nextest -p golish-agent-runtime`。
- 活体（best-effort）：换工具调用能力正常的模型重跑，确认锁定轮非 submit 调用被拒、submit 走通。
