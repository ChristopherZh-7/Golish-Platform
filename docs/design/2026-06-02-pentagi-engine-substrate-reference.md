# Operation Harness · 引擎底座参考（PentAGI 式 AI 渗透引擎）

> 目的：讲清楚 harness 三层**之下**的「引擎底座」——真正干渗透的自治 AI 引擎：它是什么、由哪些部件组成、一次任务怎么自治地跑、13 个 sub-agent 是谁、工具面怎么按角色/深度分、以及它和 harness 三层在哪接缝。这是一份**现状参考**。
>
> **定位**：harness 三层（拓扑/节点/执行）是**确定性治理外壳**；本文讲的引擎是它们共同的**底座（substrate）**——LLM 驱动、非确定性、真正干活。harness「寄生」在它上面。
>
> 证据来源（均已逐一核对真实文件）：
> - `task_orchestrator/orchestrator.rs` + `subtask_phases/` + `execute.rs`（任务编排 + 子任务执行）
> - `golish-agent-runtime/src/agentic_loop/`（per-turn 状态机）
> - `golish-sub-agents/src/defaults/builder/registry.rs`（13 个 sub-agent）
> - `golish-agent-runtime/src/execution_mode/modes/task.rs`（按角色/深度的工具面）
>
> 配套：拓扑层 `2026-06-02-harness-topology-reference.md`；节点层 `2026-06-02-harness-stage-spec-reference.md`；执行层 `2026-06-02-harness-execution-layer-reference.md`；总览 `2026-06-01-harness-explainer-and-decisions.md`。日期：2026-06-02。

---

## 0. 这是什么 + 和 harness 的关系

**引擎底座 = 真正干渗透的自治 AI（PentAGI 血统）。** harness 三层不干活、只把关；引擎才是拆任务、调工具、反思重试、出结果的那一套。

- 证据：`orchestrator.rs` 自注「This is the top-level entry point, equivalent to PentAGI's `NewTaskWorker + tw.Run()`」。
- 证据：feature_list「harness 是 task_orchestrator 的 flag 叠加层，非独立引擎，删 task 逻辑=删 harness」。

> 一句话：**引擎干活，harness 把关；引擎是底座，harness 是寄生在它几个 hook 点上的治理层。**

---

## 1. 部件总览（在哪）

| 部件 | 职责 | 在哪 |
|---|---|---|
| 任务编排 | Generator 拆 subtask + 串行推进 + plan/refine | `task_orchestrator/orchestrator.rs` + `subtask_phases/` |
| agentic loop | 单个 agent 的 per-turn 状态机（LLM→工具→反思） | `golish-agent-runtime/src/agentic_loop/` |
| sub-agents | 13 个「工种」定义（各自工具 + 能派谁） | `golish-sub-agents/.../registry.rs` |
| 工具面策略 | 按角色（primary/subtask）选工具 | `golish-agent-runtime/execution_mode/modes/task.rs` |
| 工具实现 | 真实渗透/文件/shell/web 工具 | `golish-agent-kit/tool_definitions.rs` + executors |

---

## 2. 一次任务怎么自治地跑（管线）

`task.rs` 一句话概括引擎：**「Auto: plan → execute → refine → report (multi-agent orchestration)」**。

1. **Generator**（`generate_subtasks`）把任务拆成 3–7 个有序 subtask（带 agent 指派 + 依赖；harness on 时给每个打 `harness_stage` tag）
2. **primary（depth==0）= 项目经理**：只能 `sub_agent_*` 派发 + `ask_human`，**不碰文件/shell/pentest 工具**（§5 实证）
3. 每个 subtask 派给一个 specialist（**depth>0**）→ 跑 **agentic loop**
4. **agentic loop**（turn 状态机）：LLM 流式产出 → 解析 tool calls → 过 tool_gate/dispatch → 执行 → 观察结果喂回 → 没产 tool call 时 reflector 纠偏 → 重复，直到完成或 `MAX_TOOL_ITERATIONS`
5. specialist 可**继续派子 agent**（如 `pentester → coder/researcher/browser/...`）
6. subtask 完 → **refiner** 评估并调整剩余 plan → 下一 subtask
7. 全部 subtask 跑完 → 汇总（reporter）

---

## 3. agentic loop 内部（per-turn 状态机）

入口 `run_agentic_loop_unified` → `turn::run_turn_loop`。它是引擎「单个 agent 一次对话回合」的核心。

- **能力**：tool execution + HITL 审批、loop detection（防重复输出）、上下文窗口管理（compaction）、消息历史管理、extended thinking（流式推理）。
- **phases**（`agentic_loop/turn/phases/`）：pre_flight → token_estimate → tool_dispatch → reflector_or_break → completion → compaction。
- **关键常量**：`MAX_TOOL_ITERATIONS = 100`、`APPROVAL_TIMEOUT_SECS = 1800`（30 分钟）。
- **`config.is_sub_agent`** 收紧工具 allow-list 到编排工具；`config.require_hitl` 把工具执行走人审批。

> harness 的执行层（authorizer / gate）就是挂在这个 loop 的 tool_dispatch 与交付点上的（§6）。

---

## 4. 13 个 sub-agent（registry.rs）

| agent | 角色 | 关键工具（节选） | 能派谁 | iter | 备注 |
|---|---|---|---|---|---|
| `pentester` | 渗透主力 | run_pipeline / flow_compose / manage_targets / record_finding / vault / auth_probe / pentest_run / graph_* / search_exploits | coder · researcher · memorist · installer · enricher · browser | 50 | 主战 worker |
| `coder` | 外科式代码编辑（unified diff） | read/list/grep_file · ast_grep(_replace) | — | 20 | |
| `researcher` | 查文档 / web / 知识库 / CVE | web_search/fetch · knowledge* · ingest_cve · save_poc | memorist | 25 | |
| `installer` | 装工具 / 配环境 | read/write_file · web_fetch · pentest_list_tools · pentest_run | researcher · memorist | 30 | |
| `browser` | JS / 浏览器侦察 | js_collect · js_extract_apis · web_* · record_finding | — | 20 | |
| `memorist` | 长期记忆 / 知识图 | search/store/list_memories · graph_* · knowledge | — | 10 | |
| `enricher` | 上下文补全 | memories · knowledge · graph_* · search_exploits | — | 10 | |
| `adviser` | 复杂发现的安全顾问 | web_* · read_file · memories · knowledge | researcher · memorist | 15 | |
| `reporter` | 出结构化报告 | read/write_file · memories · knowledge · poc_stats | memorist | 20 | |
| `planner` | 任务拆解（3–7 subtask） | search_memories | — | 5 | primary 不可派 |
| `refiner` | 每 subtask 完后评估调整 plan | search_memories · knowledge | — | 5 | **pipeline-only** |
| `reflector` | 没产 tool call 时自动纠偏 | （无工具） | — | 3 | **pipeline-only** |
| `orchestrator` | 主协调（拆解 + 派发） | update_plan · memories · knowledge · query_target_data | 9 个 worker | 50 | **pipeline-only** |

> `pipeline-only`（orchestrator / refiner / reflector）= 不作为可派工具暴露给 LLM，只在引擎内部流水线用。

---

## 5. 工具面怎么分（按角色/深度）—— task.rs 实证

| | primary（depth==0） | subtask（depth>0） |
|---|---|---|
| 定位 | 项目经理（只编排） | 干活的 specialist |
| 文件 / shell / pentest 工具 | ❌ 全无 | ✅ 全开（file_ops / bridge / pentest_runtime / tavily / run_command） |
| 能派的 sub-agent | 仅 9 个 worker（排除 orchestrator/planner/refiner/reflector） | 全部 + 可继续派（planner/refiner/reflector 也放开） |
| `ask_human` | ✅ | ❌ |
| `update_plan` | ✅（只有 primary 能改 plan） | ❌（deny_overrides 去掉） |

> 即「primary 只指挥、subtask 才动手」。这是引擎自身的工具过滤，**和 harness 的 per-stage `allowed_tools` 是两套独立过滤**（见 §6 接缝）。

---

## 6. 引擎 × harness 三层 在哪接缝

引擎是底座，harness 在几个 hook 点插进去（引擎本身不知道 harness 存在）：

```
[引擎]   Generator ─→ primary 编排 ─→ subtask agentic loop ─→ sub-agents ─→ 工具
            │                              │  (tool_dispatch)        │ (交付)
[harness] 建 operation_state          ① authorizer(工具前)      ② gate(交付后) / ③ approval(切阶段)
          (拓扑层: profile 投影 DAG)    (执行层)                    (执行层)
```

- **stage tag 是命门**：Generator 打 / `harness_backfill` 关键词补；没 tag 的 subtask → gate 不跑、游标不动。
- **两套工具过滤未完全打通**（explainer §5 警告）：引擎的角色工具面（task.rs）和 harness 的 per-stage `allowed_tools`（stages/*.json）是独立的；规划时要保证 `stage.allowed_tools` 里的工具名真的在某 sub-agent 的 `with_tools` 里存在。

---

## 7. PentAGI 血统 + 现状/缺口

- **血统**：多 agent 编排 + per-turn agentic loop + 真实工具调用 = PentAGI 式；`orchestrator.rs` 直接对标 PentAGI 的 `NewTaskWorker + tw.Run()`。
- **现状（成熟）**：agentic loop 完整（HITL / compaction / loop detection / extended thinking）、13 个 sub-agent、primary/subtask 工具面分层、plan→execute→refine→report 闭环。
- **缺口/接缝**：
  1. 引擎工具面 vs harness stage `allowed_tools` **两套独立过滤**，未完全打通。
  2. **stage tag** 没打 → harness 对该 subtask 完全失效（gate 不跑）。
  3. harness 侧的 **evidence ledger 未建**（见执行层文档），引擎产出的工具证据没有逐条入账可回查。

> 总结：引擎（PentAGI 式）是成熟的「干活的 AI」；harness 三层是后加的「治理外壳」。当前两者主要在执行层（authorizer/gate/approval）+ 拓扑层（operation_state 游标）接缝，接缝处的 stage tag 与 evidence ledger 是把治理真正落到引擎产出的两个关键前置。
