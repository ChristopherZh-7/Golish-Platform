# Harness 改造方案：让 profile/DAG 真正驱动 Task 执行 + 收口本次 example.com 测试暴露的 harness 缺口

> 目的：把「profile + operation_graph(DAG)」从**只用于校验/打标签的旁路叠加层**，升级为**真正驱动 Task 执行的主干**；并顺带把同一次测试暴露的 gate 旁路、子 agent 无限套娃、阶段工具"挡而不藏"、provider 韧性、意图分诊脆弱等 harness 缺口一并收口。
>
> 证据来源：2026-06-03 本会话亲自核对真实代码 + 实测日志（`~/.golish/backend.log` UTC / `~/.golish/frontend.log` 本地时区 / 转写 `~/golish-platform/Test1/.golish/transcripts/<sid>/transcript.json`）。测试用例：Task 模式输入 `example.com`，profile=assessment(env 默认)，模型 mimo-v2.5-pro。
>
> 关联设计：`docs/design/2026-06-03-task-mode-lead-agent-triage.md`（入口分诊）、`docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`（阶段工具白名单）、`docs/design/2026-06-03-two-level-phase-stage-model.md`（两级阶段模型）。
> 状态：Proposed（待用户拍板分期）。日期：2026-06-03。

---

## 0. 决策（TL;DR）

- **根问题**：harness 有一套漂亮的 profile（`assessment` 等）+ DAG（`operation_graph.json`，scoping→…→reporting）+ 阶段 gate，但**运行时执行的是 PentAGI planner 自由生成的子任务列表**，profile/DAG 只在「打 stage 标签 / 提交 deliverable 时跑 gate / 按阶段裁工具」三处被读，**从不驱动"该跑哪些阶段、按什么顺序、是否允许"**。
- **实测后果**（example.com / assessment profile）：
  - `scoping`、`target_intel` 两个 profile 允许且 DAG 必经的前置阶段**从未运行**（0 次出现）。
  - planner 生成了 `Vulnerability Scanning`(=vuln_triage) 子任务——而 assessment **forbidden_stage_kinds 明令禁止** vuln_triage。
  - 执行顺序不按 DAG，直接从 external_attack_surface 起跳。
  - profile 的 `before_active_scan` 阶段审批没生效（只触发了通用逐工具 HITL；phase 审批默认 flag-off）。
  - gate 在前两个子任务**被静默跳过**（agent 叙述、无 StageDeliverable JSON → fail-open）。
  - 子 agent 递归套娃 3 层（depth 写死 0、无上限）。
  - 阶段工具"挡而不藏"：pentest_run 在 enumeration 被拦 26 次仍反复重试。
- **方向**：分两条主线 + 若干护栏：
  - **主线 A（首选）：DAG 驱动执行** —— 运行时游标从 profile 投影后的 DAG 起点（scoping）推进，planner 退化为"填充当前阶段战术"的下层，而非顶层规划者。
  - **主线 B（过渡/兜底）：约束 planner** —— 若暂不重排执行主干，至少把 profile.allowed/forbidden + DAG 顺序喂给 planner 并在生成后做**确定性校验+修复**（拒绝跳过必经阶段、拒绝 forbidden 阶段）。
  - 护栏：gate fail-closed + 统一提交通道、子 agent 深度上限、阶段工具从工具清单裁剪、provider 退避/失败转移、意图分诊受限输出、profile/stage/cursor 进日志。
- **非目标**：不重写 agentic_loop；不改 DB schema（首期）；不改 Chat 模式。

---

## 1. 现状勘验（实测 + 代码落点）

| 环节 | 现状 | 真实落点（已核） | 缺口 |
|---|---|---|---|
| profile 解析 | env 默认 `assessment` | `harness/mod.rs::read_env_profile`（默认 "assessment"）；legacy task 走 `mode.rs::set_harness_profile(None)` | profile id **从不进日志**，无法从日志确认实际 profile |
| profile→DAG 投影 | 静态存在 | `profiles/assessment.json::allowed_stage_kinds`(+forbidden) × `graph/operation_graph.json` | 投影结果**不驱动执行**，仅供 gate/工具裁剪查 |
| 顶层规划 | planner 自由生成 | `bridge_executor/trait_impl.rs::generate_subtasks`（LLM 一次性产 subtasks） | **不读 profile/DAG**，可跳过必经阶段、可产 forbidden 阶段 |
| 阶段标签 | 事后打标 | `subtask_phases/*`（Generator tag + keyword backfill，实测 `backfilled=0 total=6`） | 只给已有子任务打标，**不插入缺失的必经阶段** |
| gate 触发 | 两条路且都脆 | `subtask_phases/execute.rs::apply_harness_gate_hook` + `parse_deliverable_from_content`（叙述无 JSON→skip，fail-open）；`ai/harness_submit_tool.rs::submit_stage_deliverable`（显式工具） | hook fail-open；两条提交通道互相矛盾（gate 纠正语写 "there is no submit tool" 却又存在该工具） |
| 阶段审批 | 多套并存 | profile `approval_policy.before_active_scan`；two-level phase 审批 `harness::two_level_enabled()`（默认 off）；逐工具 HITL（实际生效的那个） | profile 的阶段审批语义未驱动；只有逐工具 HITL 真跑 |
| 阶段工具边界 | 挡而不藏 | `sub_agent_call.rs::stage_tool_guard`（调用时 deny-by-default 报错） | 工具仍留在模型 tool_list → 反复重试（实测 pentest_run 拦 26 次） |
| 子 agent 深度 | 无上限 | `sub_agent_call.rs` record_start 传 `depth=0`、`parent=None`（注释 "tracking deferred (P1)"） | 无深度护栏 → 实测套娃 3 层 |
| provider 韧性 | 弱 | `agentic_loop::stream_processor`（重复检测有；500 处理弱） | 无退避/失败转移；重复检测后不恢复 |
| 入口分诊 | 脆 | `intent.rs::classify_user_intent`（max_tokens=8，思考模型恒返空→默认 Task）；`chat.rs::deterministic_intent`（只认 http(s)://，不认裸域名/IP） | 见 triage 设计；本次 25/25 分类器返空 |

---

## 2. 目标 / 非目标

**目标**
1. profile/DAG 成为 Task 执行的**事实驱动**：实际跑的阶段集合 = profile.allowed 投影后的 DAG 可达子图；顺序 = DAG 拓扑；forbidden 阶段不可能被执行。
2. 必经前置阶段（scoping/target_intel）**不可被跳过**。
3. gate **fail-closed**：阶段产物缺失/不合法 → 阻塞推进或强制补，绝不静默放行。
4. 阶段审批按 profile.approval_policy 在**阶段边界**触发。
5. 弱模型也跑得动：深度上限、工具裁剪、provider 退避/失败转移、重复恢复。
6. 可观测：profile id / 当前 stage / 游标推进 / gate 决策全进日志。

**非目标**
- 不重写 agentic_loop / sub-agent 执行内核。
- 首期不动 DB schema（游标/状态已有 operation_state 承载）。
- 不动 Chat 模式。

---

## 3. 提议改造（分模块；每条含 落点 + 改法 + 验证）

### A. profile/DAG 当驱动（P0 · 核心）

**A1（首选）DAG 驱动执行循环**
- 落点：`subtask_phases/`（执行主干）+ `harness::operation_state` 游标 + `operation_graph` 投影。
- 改法：Task 启动时按 `profile.allowed_stage_kinds` 投影 DAG 得"可达阶段有序列表"，游标从入口（scoping）起；**每个阶段是一个执行单元**：进入阶段→（可选 planner 仅为"本阶段"产战术子步）→收 deliverable→gate→PASS 才按 DAG 边推进游标→下一阶段。planner 不再一次性产全程 subtasks。
- 验证：单测——assessment 投影后阶段序列 = [scoping,target_intel,eas,enumeration,reporting]；集成——example.com 跑出的第一个阶段必是 scoping，且 forbidden(vuln_triage) 永不进入。

**A2（过渡/兜底）约束 planner**（若 A1 暂不落地，先上 A2）
- 落点：`bridge_executor/trait_impl.rs::generate_subtasks` + `task_orchestrator/prompts/mod.rs`（planner prompt）+ 新增确定性校验器。
- 改法：① prompt 注入 profile.allowed/forbidden + DAG 顺序 + "必须覆盖必经前置阶段"；② 生成后做**确定性校验**：缺 scoping/target_intel→自动插桩或打回重规划；含 forbidden 阶段→剔除或打回；阶段顺序不符 DAG→重排。校验是确定性代码，不靠模型自觉。
- 验证：单测——给"只含 eas/enumeration/vuln_triage"的假 plan，校验器补 scoping/target_intel、剔 vuln_triage、按 DAG 重排。

### B. gate fail-closed + 统一提交通道（P0 · 与 A 强相关）
- 落点：`subtask_phases/execute.rs::apply_harness_gate_hook`（fail-open 的 `return (content, None)`）+ `parse_deliverable_from_content` + `ai/harness_submit_tool.rs` + gate 纠正语 `build_gate_correction`（"there is no submit tool" 矛盾文案）。
- 改法：① **唯一合法提交 = `submit_stage_deliverable` 工具**（结构化、确定性），废弃"解析叙述自由文本"这条脆路径或仅作兜底；② stage-tagged 子任务结尾无合法 deliverable → **不放行**：强制重试要求补 deliverable，或阻塞游标推进（与 A 的游标统一，杜绝"子任务完成但阶段游标没动"的脱节）；③ 删掉矛盾文案，提交契约全局一致。
- 验证：单测——stage-tagged + 无 deliverable → 返回"阻塞+纠正"而非透传；集成——叙述式收尾不再静默跳过 gate。

### C. 子 agent 深度上限（P1 · 一道硬护栏，最简）
- 落点：`sub_agent_call.rs`（现 `depth=0`/`parent=None` 写死）。
- 改法：把真实 depth/parent_dispatch_id 透传（record_start 参数已预留）；dispatch 入口判 `depth >= MAX_SUBAGENT_DEPTH(默认 1)` → 直接拒绝并回明确错误（"sub-agents may not spawn sub-agents"）。
- 验证：单测——depth=1 再调 sub_agent_* 被拒；实测回归——套娃层级 ≤1。

### D. 阶段工具"藏"而非"挡"（P1 · 对症 26 次撞墙）
- 落点：工具清单构建（`agentic_loop/tool_list.rs` / `execution_mode/selection_apply`）+ 现有 `stage_tool_guard`。
- 改法：① 按当前阶段 `allowed_tool_types` **从暴露给模型的 tool_list 里裁掉**不允许的工具（模型看不到→不会调）；② 万一仍调（guard 兜底），错误信息**列出当前可用工具**，让模型当场改用。
- 验证：单测——enumeration 阶段 tool_list 不含 exploit-only 工具；实测回归——pentest_run 撞墙次数→0。

### E. provider 韧性（P1）
- 落点：`agentic_loop::stream_processor` / llm 调用层。
- 改法：① 5xx **指数退避重试 + 重试预算**，超预算→失败转移到备用模型/通道；② 重复检测命中后**恢复**：重试一次并提示"别复述、直接给结论"+调高 repetition penalty，再不行跳过该步记一笔。
- 验证：单测——连续 500 触发退避/转移；重复后走恢复分支。

### F. 入口分诊 harness 化（P2 · 见 triage 设计）
- 落点：`chat.rs::deterministic_intent`（裸域名/IP）+ `intent.rs::classify_user_intent`（max_tokens/受限输出）。
- 改法：① 确定性层加裸域名/IP 识别（→ Task 或 clarify）；② LLM 分类改**受限/结构化输出**（强制吐 CHAT/TASK/CLARIFY 之一），空/非法→重试或安全兜底，绝不静默默认 Task；③（可选）落地 triage 设计的 clarify 三分支。
- 验证：单测——example.com→确定性命中；mimo 受限输出非空；空响应不再默认 Task。

### G. 可观测（P2 · 低成本高价值）
- 落点：Task 启动处 + 阶段推进处。
- 改法：profile id、投影后阶段序列、游标每次推进、gate 决策（pass/block/skip 原因）全部 INFO 进日志（`harness::*` target 已被 telemetry 收录）。
- 验证：实测——日志可直接看出"用了哪个 profile、跑了哪些阶段、卡在哪"。

---

## 4. 影响面 / 受影响文件

| 文件 | 改动 | 风险 |
|---|---|---|
| `golish-agent-kit/src/task_orchestrator/subtask_phases/*` | A1 执行循环重排 / B gate fail-closed | ⚠ 高（执行主干，需充分测试 + flag 包裹） |
| `golish-agent-bridge/src/bridge_executor/trait_impl.rs` + `task_orchestrator/prompts/mod.rs` | A2 planner 约束 + 校验器 | 中 |
| `golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs` | C 深度上限 | 低 |
| `golish-agent-runtime/src/agentic_loop/tool_list.rs` / `execution_mode/selection_apply` | D 工具裁剪 | 中 |
| `golish-agent-runtime/src/agentic_loop/stream_processor*` | E 退避/恢复 | 中 |
| `golish-agent-app/src/ai/commands/core/chat.rs` + `golish-agent-bridge/src/bridge_executor/intent.rs` | F 分诊 | 低 |
| `golish-agent-app/src/ai/harness_submit_tool.rs` | B 统一提交 | 中 |

---

## 5. 分期与优先级

- **P0**：A（profile/DAG 驱动，先 A2 约束 planner 兜底，再 A1 DAG 驱动）+ B（gate fail-closed + 统一提交）。这两条直接修"scoping 被跳 / forbidden 被规划 / gate 旁路"。
- **P1**：C（深度上限，最简先做）+ D（工具裁剪）+ E（provider 韧性）。
- **P2**：F（分诊 harness 化）+ G（可观测）。

每条均 **feature-flag 包裹**（A1 尤其，默认灰度），关闭即回退现行为。

---

## 6. 风险 / 回滚

- **R1（A1 执行主干重排）**：最高风险，必须 flag + 充分单测/集成；回滚=关 flag 回 planner 驱动。
- **R2（B fail-closed）**：可能让"模型死活不交合法 deliverable"的阶段卡死 → 配重试上限 + 到顶后降级（记 blocked 并收尾，而非无限卡）。
- **R3（A2 校验打回）**：planner 反复产非法 plan → 限重规划次数，到顶用确定性插桩兜底。
- 全部改动 flag 化、单 crate 内优先、`just precommit` 全绿前不 commit（AGENTS.md §3）。

---

## 7. 验证策略（DoD 摘要）

- 单测（Rust）：A 投影/校验器、B fail-closed 分支、C 深度拒绝、D 工具裁剪、E 退避/恢复、F 分诊。
- 集成：example.com + assessment → 第一个阶段 = scoping；forbidden(vuln_triage) 永不进入；gate 不再静默 skip；套娃 ≤1 层；pentest_run 撞墙=0。
- 证据：`just precommit` 全绿 + 复跑一次 example.com，日志可见 profile/stage/cursor/gate 全链路（G）。
- 不把"代码编译过"当完成——以**实测复跑日志**为准（AGENTS.md §3 / I7）。

---

## 8. 开放问题（实现前需拍板）

1. A 选 A1（DAG 驱动，治本但改面大）还是先 A2（约束 planner，过渡）？建议 A2 先上、A1 紧随。
2. B 的"唯一合法提交"是否彻底废弃自由文本解析？还是保留为兜底？
3. MAX_SUBAGENT_DEPTH 取值（建议 1）。
4. provider 失败转移的备用模型/通道由谁配置（用户设置 or 内置降级链）？
5. flag 命名与默认灰度策略。
