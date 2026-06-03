# Task 模式 Lead-Agent 前置思考/分诊 设计

> 目的：在 Task 模式「输入 → 规划器拆子任务 → 子 agent 执行」的链路前，补一层**主 agent（lead agent）用完整推理先思考再决定**的分诊层——决定「直接回答 / 反问澄清 / 拆成计划交付编排」，把决策权还给主 agent，而不是让每条输入都被 8-token 意图分类器一锤定音、硬塞进规划器。
>
> 上游背景：`docs/design/2026-06-02-golish-agent-engine-v2-design.md`（task_orchestrator 是「干活的 AI」成熟底座，零改动留用）。本设计是在该底座**入口处**加一层 triage，不动编排主干。
> 证据来源：本设计 §1 表中每条均为 2026-06-03 本会话亲自读真实代码核对。日期：2026-06-03。
> 关联 feature：`task-mode-lead-agent-triage-2026-06-03`（待加入 `feature_list.json`，状态 `not_started`）。

---

## 0. 决策（TL;DR）

- **问题**：Task 模式下主 agent 不参与思考。输入只经过一个极简意图分类器（问 LLM 一个词 `TASK`/`CHAT`），判 `TASK` 就**直接进规划器**硬拆子任务；分类器超时/失败默认 `TASK`，于是「你好」这类闲聊也被硬拆，规划器拿不到 plan → 报错。
- **方向（用户 2026-06-03 选定方案 1）**：在进规划器前加 **lead-agent 分诊**——用主 agent 的**完整推理能力**读输入，结构化产出三选一：`reply`（直接回答）/ `clarify`（反问）/ `decompose`（交规划器）。只有 `decompose` 才进现有 orchestrator。
- **非目标**：不重写 task_orchestrator / sub-agent / agentic_loop；不动 Chat 模式；不改 DB schema（首期）。
- **分期**：P0 分诊层 + 三分支接线（替代裸意图分类器）→ P1 分诊质量（few-shot、确定性兜底、可观测）→ P2 与 harness/checkpoint 对齐（分诊也可断点续跑）。
- **与近期前端缓解的关系**：本会话已落地的「error→warning severity 重分类 + 去重 + 持久化」是**治标**（让误判后的报错不吓人、可追溯）；本设计是**治本**（从源头让主 agent 决定要不要拆）。两者互补，不冲突。

---

## 1. 现状勘验（本会话亲自核对真实代码）

| 环节 | 现状 | 真实落点（已核） | 缺口 |
|---|---|---|---|
| 模式分发 | ✅ | `golish-agent-app/src/ai/commands/core/chat.rs:57-100`：`Chat→bridge.execute`；`Task→classify_user_intent→(Conversation→bridge.execute｜else→execute_task_mode)` | Task 分支只有「分类器」一道关 |
| 意图分类器 | ⚠️ 脆 | `bridge_executor/intent.rs:27-95`：一次性 LLM、`max_tokens=8`、`temp=0`、**15s 超时**；结果含 `CHAT`→Conversation，否则 Task；**失败/超时默认 Task**（`:88/:92`） | 单点 LLM，误判即硬拆；非推理、无澄清 |
| 分类 prompt | ✅ | `task_orchestrator/prompts/mod.rs:15-28`：示例里**已把「你好」列为 CHAT** | prompt 没问题，问题在「只信这一次调用」 |
| Task 编排入口 | ✅ | `chat.rs:109-204 execute_task_mode`：建 session 行 → `emit UserMessage` → `orchestrator.run(task_input, &executor)` → 成功后把报告作为一条 `Started/TextDelta/Completed` 发出 | **无主 agent 推理/直接回答这一层** |
| 规划器 | ✅ | `bridge_executor/trait_impl.rs:14-47 generate_subtasks`：planner LLM → `extract_json_from_response` → `serde_json::from_str::<GeneratorOutput>` → 失败走 `describe_plan_parse_failure` | 只会「拆」，不会「判断该不该拆/直接答」 |
| 非 plan 响应处理 | ⚠️ | `bridge_executor/mod.rs:328-359`：`looks_like_json_object`（看是否 `{`/`[` 开头）+ `describe_plan_parse_failure`（拒答散文 vs 坏 JSON 两分支） | `{"message":…}` 被判「坏 JSON」；无论哪种都成 `Err`→报错 |
| 主 agent（会思考的那个） | ✅ | Chat 路径 `bridge.execute`（`agentic_loop` 推理 + 工具 + 直接回答） | 仅 Chat 模式用到；Task 模式不调用它来「思考」 |

> **核心洞察**：会推理的主 agent（`bridge.execute`）**已经存在且成熟**，只是 Task 模式没在「该不该拆」这个决策点上用它。本设计 = 把这个已有能力接到 Task 入口的决策位，而非新造 agent。

### 1.1 复现链（你好 → 报错）

```
你好
 → chat.rs Task 分支
 → classify_user_intent  ──(超时/误判)──> Task
 → execute_task_mode → orchestrator.run → generate_subtasks(planner)
 → 模型用对话回： {"message":"你好！我是渗透测试任务规划专家…"}
 → serde 缺 `subtasks` → describe_plan_parse_failure → Err
 → "Generator failed: Failed to parse task planner JSON (missing field `subtasks`)"
```

---

## 2. 目标 / 非目标

**目标**
1. 主 agent 在 Task 模式下「先思考再决定」：直接回答 / 反问 / 拆解，三选一。
2. 闲聊、寒暄、知识问答、能力询问等**不进规划器**，由主 agent 正常回答（复用 `bridge.execute` 的事件流，与 Chat 体验一致）。
3. 决策**确定性可控**：不把「是否拆解」单点押在一次易超时的 LLM 调用上。
4. 决策**可观测**：分诊结果、理由、耗时进 trace；对用户透明（可选：UI 展示「主 agent 判断：需要拆成 N 步」）。

**非目标**
- 不重写 `task_orchestrator` / sub-agent / `agentic_loop`。
- 不动 Chat 模式行为。
- 首期不引入 DB schema 变更（分诊产物先走事件 + 内存；如需断点续跑再进 P2）。
- 不在本设计内做「规划器 schema 合法化 message 字段」（那是备选方案 3，见 §6）。

---

## 3. 提议设计：Lead-Agent 分诊层

### 3.1 决策位与流程

把 `chat.rs` Task 分支的「裸 `classify_user_intent`」升级为 **`triage`**：

```
Task 模式输入
 → lead_agent_triage(bridge, prompt)            # 主 agent 用完整推理
     ├─ reply      → 直接把 triage 产出的答复作为 assistant 消息发出（或转 bridge.execute 生成完整回答）
     ├─ clarify    → 发一条 ask-human / 普通追问消息，等用户补充
     └─ decompose  → execute_task_mode(...)（现有编排，零改动）
```

### 3.2 分诊产物（结构化）

```jsonc
{
  "decision": "reply" | "clarify" | "decompose",
  "reason": "一句话，进 trace",
  "reply": "decision=reply 时：给用户的话（可空，空则转 bridge.execute 生成）",
  "clarify": "decision=clarify 时：要问用户的问题",
  "confidence": 0.0
}
```

- **decision=reply**：寒暄/知识问答/能力询问/与渗透无关的闲聊。两种实现档位：
  - 轻：直接把 `reply` 文本作为 `Started/TextDelta/Completed` 发出（省 1 次调用）；
  - 重：忽略 `reply`，直接 `bridge.execute(&prompt)` 让主 agent 给完整带工具的回答（与 Chat 一致）。**首期建议「重」**——真正「主 agent 思考」，不要二段式割裂。
- **decision=clarify**：信息不足（如「帮我测一下」没给目标），主 agent 反问，不进规划器。
- **decision=decompose**：确属多步可执行任务 → 进现有 `execute_task_mode`，链路完全不变。

### 3.3 lead-agent triage 怎么实现（两条候选实现路径）

- **路径 A·复用主 agent + 结构化输出**（推荐）：用 `bridge` 现成的 completion，给一个「triage system prompt」+ 让模型走结构化输出（`output_schema`，`intent.rs` 里 `CompletionRequest` 已有该字段）产出 §3.2 JSON。比现分类器强在：用主模型、有理由、能反问、能给答复。
- **路径 B·让主 agent 自己带「decompose 工具」**：把「拆成计划」做成主 agent 的一个工具调用。主 agent 正常对话；当它判断需要多步时**主动调用 `decompose` 工具**触发 orchestrator。最贴近「主 agent 自己思考并决定委派」，但改面更大（要把 orchestrator 暴露成工具 + 处理嵌套）。**首期 A，B 作为 P2 演进**。

### 3.4 确定性兜底（不全押 LLM）

- 进 LLM triage 前，先过一层**确定性短路**：极短纯问候/致谢（命中保守白名单且无渗透动词/目标 token）直接 `reply`；空输入/纯标点直接 `reply`。
- LLM triage **超时/失败**时：不再「默认 decompose」，改为**默认 reply（转 bridge.execute）**——闲聊误答成正常对话，比把寒暄硬拆成扫描任务安全得多（且 `bridge.execute` 自己也会判断要不要用工具）。
  - ⚠️ 这是相对现状 `intent.rs` 默认 `Task` 的**行为反转**，需在 §7 风险里评估：会不会让「真任务」在 triage 失败时退化成普通对话？缓解：确定性层对「明显任务信号（URL/IP/scan/exploit/审计 等动词）」优先判 decompose，不依赖 LLM。

### 3.5 事件流（与现状对齐）

- `decompose`：完全沿用 `execute_task_mode` 现有事件（`UserMessage` echo → orchestrator `TaskProgress` → 最终报告 `Completed`）。
- `reply`（重档）：`bridge.execute` 自带 `Started/TextDelta/Completed`，与 Chat 模式一致。
- `clarify`：复用 `AiEvent::AskHumanRequest`（前端已有 `AskHumanInline`）或普通 `Completed` 追问消息。
- 可选透明度：分诊为 `decompose` 时，先发一条「主 agent 判断：这是多步任务，开始规划…」的轻量 `TaskProgress`，让用户看到「它确实想过」。

---

## 4. 影响面 / 受影响文件

| 文件 | 改动 | 并发风险 |
|---|---|---|
| `golish-agent-app/src/ai/commands/core/chat.rs` | Task 分支：`classify_user_intent` → `lead_agent_triage`；按 `decision` 三分支 | 低（不在当前改动列表） |
| `golish-agent-bridge/src/bridge_executor/intent.rs` | 升级/替换为 triage（结构化输出 + 确定性兜底）；或新增 `triage.rs`，保留 intent 兼容 | ⚠️ 中：`bridge_executor/mod.rs`/`trait_impl.rs` 正被别的会话改 harness；新增 `triage.rs` 比改 `intent.rs` 冲突面小 |
| `golish-agent-kit/src/task_orchestrator/prompts/mod.rs` | 新增 `lead_triage_prompt()`（与现 `intent_classifier_prompt` 并存或替换） | 低（task_orchestrator 不在改动列表） |
| 前端 | 基本零改：`reply`/`clarify` 走既有消息/AskHuman 渲染；可选加「分诊判断」轻量提示 | 低 |

> **并发策略**：新增 `triage.rs` 而非重写 `intent.rs`，把改动集中在 `chat.rs`（非热点）+ 新文件，最小化与 harness 会话的冲突面。

---

## 5. 备选方案（为什么选 1）

| 方案 | 做法 | 取舍 |
|---|---|---|
| **1 · lead-agent 分诊（选）** | 进规划器前主 agent 完整推理决定 reply/clarify/decompose | 治本、把决策权还主 agent；改面中等、需设计先行 |
| 2 · 轻量兜底 | 仅在「分类误判 + 规划器拿到对话响应」时回退 bridge.execute | 最快、改面小；**治标**，主 agent 仍不思考，且回退点在编排失败后、事件可能已脏 |
| 3 · 规划器可拒绝拆解 | planner schema 合法支持 `{decision, message}`；编排层据此走对话 | 比 2 干净；但「该不该拆」仍由规划器（一个面向拆解的 prompt）判，不是真正主 agent 思考 |

> 2 可作为 1 落地前的**临时缓解**（用户若想「你好」立刻不报错）；3 可作为 1 的实现细节被吸收（让 planner 也能回 decline，作为 triage 之外的第二道网）。

---

## 6. 风险 / 回滚

- **R1 行为反转（§3.4）**：triage 失败默认 `reply` 而非 `decompose`，可能让「真任务」在 LLM 失败时退化成普通对话。**缓解**：确定性层优先识别强任务信号（URL/IP/scan/exploit/审计动词）→ 直接 decompose，不经 LLM。
- **R2 延迟**：triage 多一次（主模型）调用。**缓解**：确定性短路覆盖常见寒暄；triage 用低 `max_tokens` + 结构化输出；与现 15s 超时同量级，不更差。
- **R3 误判把任务当闲聊**：渗透平台「漏判任务」比「误拆闲聊」后果轻（不会误触扫描/exploit），方向上是更安全的默认。仍以确定性任务信号兜底。
- **R4 并发冲突**：见 §4 策略（新文件 + 集中 chat.rs）。
- **回滚**：triage 是入口一层，feature-flag 包裹（如 `lead_triage_enabled`，默认可灰度）；关闭即回到现 `classify_user_intent` 行为。**首期实现必须带开关。**

---

## 7. 验证策略（DoD 摘要，细化进实现计划）

- 单测（Rust）：确定性短路（寒暄/空输入/任务信号）判定；triage JSON 解析与三分支映射；超时/失败兜底走 `reply`。
- 集成：Chat 模式不受影响（回归）；Task 模式「你好/谢谢/你能做什么」→ 正常回答（非报错）；「scan example.com」→ decompose 进编排。
- 证据：跑 `just precommit` 全绿；trace 里能看到 triage decision/reason/耗时。
- 不把「分类对了」当「完成」——以**实际跑过的命令 + 输出**为准（AGENTS.md §3）。

---

## 8. 开放问题（实现前需用户/团队拍板）

1. `reply` 用「轻档」（直接发 triage 文本）还是「重档」（转 `bridge.execute` 完整回答）？本设计倾向重档。
2. triage 失败默认 `reply` 还是保持现状默认 `decompose`？本设计倾向 `reply` + 强任务信号兜底（R1）。
3. 是否要 UI 透明展示「主 agent 分诊判断」？（提升信任，但增前端改面。）
4. 是否同时吸收方案 3（planner 也能 decline）作为第二道网？
5. feature-flag 命名与默认值（灰度策略）。

---

## 9. 分期与后续

- **P0**：`triage.rs`（路径 A，结构化输出 + 确定性短路）+ `chat.rs` 三分支接线 + feature-flag + 单测/集成。产出实现计划 `docs/superpowers/plans/2026-06-03-task-mode-lead-agent-triage-p0.md`（按 `.cursor/skills/writing-plans`）。
- **P1**：triage 质量（few-shot、置信度、可观测面板）、延迟优化。
- **P2**：路径 B（decompose 作为主 agent 工具）、triage 与 harness checkpoint 对齐（分诊也可断点续跑）。

> 下一步：若用户确认 §8 的开放问题，则进入 writing-plans 产出 P0 实现计划，再 executing-plans 落地。本设计文件不覆盖旧文档，新增独立 markdown（AGENTS.md §2.4 / I6）。
