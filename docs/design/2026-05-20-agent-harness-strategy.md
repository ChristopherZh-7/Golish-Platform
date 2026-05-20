# Golish Agent Harness 改造策略文档

- **作者**: MCP-1（全栈工程师，BaJie-MCP）
- **日期**: 2026-05-20
- **状态**: Draft（策略级，非实现规范）
- **读者**: Golish 平台后续的任何工程师 / AI agent
- **目的**: 把当前 task 模式 agent 自动化从「抄了 PentAGI 但不知道下一步」转型为「Anthropic harness 思想下的渗透测试专用流水线」。本文档自包含——读完不需要再做调研。

---

## 0. 一句话先说清

> Golish 现在已经有 PentAGI 风格的通用 orchestrator；缺的是**渗透测试领域的"流水线 + 阶段 gate + 阶段 harness"**。改造方向是**不重写 orchestrator，在它之上叠一层领域骨架**。第一刀切在 Recon 阶段。

---

## 1. 背景

### 1.1 项目现状（一句话）

Golish 的目标是把"测试一个站点 / IP / 资产"这件事自动化，让 AI agent 跑完一条完整的渗透测试流水线、最后产出 PoC + 报告。

### 1.2 当前实现

后端核心模块：`backend/crates/golish-agent-kit/src/task_orchestrator/`

它是按 PentAGI 的 `Flow → Task → Subtask → Tool` 骨架实现的通用任务编排器，已有：

- `Generator`：把用户输入拆 ≤13 个 subtask
- `Primary Agent Loop`：enrich → plan → execute → reflector retry → user input pause
- `Refiner`：用 patch-style（add/remove/modify/reorder）调整剩余 plan
- `Reporter`：末端总结
- `[NEEDS_USER_INPUT]` HITL 暂停 / `TaskCostTracker` token 账本 / `SubtaskStatus` 持久化

### 1.3 真痛点（不是代码 bug，是骨架缺失）

**痛点 1：判定"完成"只靠两招，都不靠**
- 招一：`looks_like_text_only_response` 只检测"是不是只说话没动工具"
- 招二：Reflector 仅在招一命中后重试

**只要 agent 动了工具**（哪怕是 `echo hi`）**就被判完成**。这是跨二十个项目"假完成"的根源。

**痛点 2：subtask 是自然语言三字段**

```rust
pub struct PlannedSubtask {
    pub title: String,
    pub description: String,
    pub agent: Option<String>,
}
```

没有 `acceptance_criteria`，没有"什么算做完"的契约。

**痛点 3：`SubtaskResult` 是裸文本**

Refiner 拿到的是字符串，只能"读上下文语义反思"去改 plan。Anthropic 同期对照物是 `feature_list.json`——其它 agent 看见的是结构化清单，"哪些件未完成"一眼看得出来。

### 1.4 用户的认知卡点

用户原话："我抄了 PentAGI 所以现在很乱、整个逻辑不太清晰、不知道怎么继续、渗透测试的流程要怎么定义、每个 agent 要怎么实现 harness 工程我现在都一脸懵。"

这是**领域骨架空心**问题。代码骨架（PentAGI）已经在了，但渗透测试这件事在 harness 视角下的画面没建立起来。本文档就是要建立这张画面。

---

## 2. 核心理论：什么是 agent harness

### 2.1 一句话定义

> **Agent = LLM + Harness**。LLM 负责"想 / 说"；harness 是 LLM 之外、把它接到真实世界并约束它怎么行动的所有工程外壳（工具白名单、产出 schema、gate 校验、handoff 介质、错误恢复等）。

### 2.2 Anthropic 三层认知

| 层 | 含义 | 例子 |
|---|---|---|
| Workflow | 人预定代码路径，LLM 填空 | prompt chaining / routing / parallelization / orchestrator-workers / evaluator-optimizer |
| Agent | LLM 自决步骤，环境给 ground truth | 通用聊天 agent + tools |
| Harness | 上两者的 scaffold：工具/环境/handoff/checkpoint/error recovery | Claude Agent SDK / PentAGI / Golish 想做的事 |

Anthropic 反复强调：**find the simplest solution possible, only add complexity when needed.**

### 2.3 Anthropic 三篇必读

1. **[Building Effective Agents](https://www.anthropic.com/engineering/building-effective-agents)**（2024-12）
   - workflow vs agent 的根本区分
   - 5 个经典 workflow pattern
   - 反复强调：不必要复杂的话，单 LLM 调用 + retrieval 就够

2. **[Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)**（2025-11）
   - Initializer agent + Coding agent 二段式
   - `feature_list.json`：agent 只能 toggle `passes` 字段，不能动结构
   - `claude-progress.txt + git` 作 handoff
   - 每个 session 开头跑 sanity check（`init.sh` + Playwright 验关键路径）
   - **context reset 优于 compaction**

3. **[Harness design for long-running application development](https://www.anthropic.com/engineering/harness-design-long-running-apps)**（2026-03）
   - GAN-inspired 三 agent：Planner / Generator / Evaluator
   - **Sprint contract**：写代码前 generator 和 evaluator 谈好"done 长啥样"
   - Evaluator 用 Playwright **实际跑 app** 验收
   - Evaluator 默认会偏袒 LLM 输出，必须独立调教成 skeptical
   - 量化：solo 20 min/$9 → harness 6 hr/$200，质量跨级别

### 2.4 PentAGI 在这套理论中的位置

PentAGI 是个**通用 orchestrator** ≈ Anthropic 的 orchestrator-workers + evaluator-optimizer 混合骨架。它实现了任务级的"拆-执行-反思-总结"循环，**但它没有渗透测试领域知识**——它不知道什么是 Recon、什么是 done。

所以借鉴 PentAGI 的方式应是：
- **保留**：Flow / Task / Subtask / Tool 这四级抽象
- **替换**：把"任意领域的 done(result)"替换为"渗透测试每个阶段专用的 `submit_*_deliverable + validate_*_gate`"

---

## 3. 概念框架：渗透测试 + harness 的三层模型

这是本文档最重要的章节。用户后续做的所有事都应能放进这张图。

### 3.1 三层独立、自上而下

```text
┌─────────────────────────────────────────────────────────────┐
│ 第一层：渗透测试流水线（领域阶段 DAG）                       │
│   what 在做哪一行                                            │
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│ 第二层：每阶段的 harness 4 件套                              │
│   how 让 LLM 不作弊                                          │
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│ 第三层：PentAGI orchestrator + LLM（已有代码）              │
│   runner 真正跑 subtask、调工具                             │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 第一层：渗透测试 7 阶段 DAG

| # | 阶段 | 目标 | 风险等级 |
|---|---|---|---|
| 1 | Scoping | 确认范围 / 授权 / 禁区 | 无 |
| 2 | Recon | 信息收集（DNS / 子域 / 指纹 / 被动） | 低 |
| 3 | Enumeration | 服务枚举（端口 / 服务 / HTTP / 版本） | 中 |
| 4 | Vuln ID | 漏洞识别（CVE 匹配 / 弱口令 / 配置错误） | 中 |
| 5 | Exploitation | 漏洞验证（PoC 跑起来 / 低风险 / 不提权） | 高 |
| 6 | Post-Exploit | 后渗透（默认跳过，需明示授权） | 极高 |
| 7 | Reporting | 报告输出（按严重程度排、附修复建议） | 无 |

每个阶段必须能回答 5 个问题（这是 harness 合同模板）：

1. **输入是什么 schema**
2. **输出是什么 schema**
3. **允许什么工具**
4. **done 怎么判定**
5. **失败 / 越界 / 跳过怎么处理**

### 3.3 第二层：每阶段的 harness 4 件套

| 件 | 作用 | Anthropic 对应物 | PentAGI 对应物 | Golish 现状 |
|---|---|---|---|---|
| **Phase Charter** | 一份 Markdown 告诉 LLM：身份、输入、输出、白名单、黑名单、done 条件 | sprint contract / planner spec | `pentester.tmpl` | 部分有（静态 system prompt） |
| **Tool Belt** | 该阶段允许使用的工具子集（白名单） | ACI 设计 | `Tool Registry` | ✅ 有 `tool_executors/`，但未按阶段切子集 |
| **Deliverable + Gate** | 结构化产物 + 硬验收规则 | evaluator + criteria | `done(result)` 仅文本 | ❌ 待新增（即 Recon harness） |
| **Handoff Packet** | 阶段过关后交给下阶段的结构化负载 | `progress.txt + feature_list.json` | DB / subtask result 文本 | ❌ 现在是裸文本 |

### 3.4 第三层：PentAGI orchestrator 的落位

```text
用户输入 "测下 example.com"
   ↓
渗透测试流水线（你定义的 7 阶段 DAG）
   ↓ 指令 "现在进入 Recon 阶段"
阶段 harness（注入 Phase Charter、压缩 allowed_tools、准备接收 deliverable）
   ↓
PentAGI orchestrator 拆这个阶段为 N 个 subtask
   ↓
每个 subtask 走：generate → enrich → plan → execute → refine（已有代码）
   ↓ 产出 deliverable
阶段 gate 验收
   ↓ 过 → handoff 给下阶段；不过 → 退回让 refiner 加补救 subtask
```

**关键洞察**：PentAGI 只在中间"拆 subtask + 跑 subtask"那一步干活。它上面需要"阶段调度器 + 阶段 harness"，下面需要"工具集"。这两端是用户需要新增的。

---

## 4. 首选样板：为什么是 Recon

不要 7 个阶段同时上。**选 1 个阶段把 harness 完整做一遍，提炼出模板后其他 6 个阶段复制粘贴**。

### 4.1 选样板的评分表

| 阶段 | 适合样板吗 | 理由 |
|---|---|---|
| Scoping | ❌ | 重交互 / 重人工确认，几乎不需要 LLM |
| **Recon** | ✅ **最适合** | 工具多、输出能结构化、风险低、可纯被动跑 |
| Enumeration | 次选 | 需要主动扫描、授权门槛高 |
| Vuln ID | ❌ | 依赖前两阶段产出 |
| Exploitation | ❌ | 需要隔离沙箱、风险高 |
| Post-Exploit | ❌ | 同上 |
| Reporting | ❌ | 依赖前面全部阶段产出 |

### 4.2 Recon 阶段的 5 问答模板

| # | 问 | 答 |
|---|---|---|
| 1 | 输入 schema | `{ scope: [...], known_assets: [...] }` |
| 2 | 输出 schema | `ReconDeliverable`（见 [`docs/design/harness-recon-mvp.md`](harness-recon-mvp.md) §5） |
| 3 | 允许工具 | `dns_resolve, subdomain_enum_passive, whois_lookup, http_fingerprint_passive, shodan_query, fofa_query` 等被动 / 半被动工具 |
| 4 | done 判定 | `validate_recon_gate(deliverable).allowed == true`（硬规则函数，见同上文件 §6） |
| 5 | 失败 / 越界 | 发现 out-of-scope target 写 `skipped_checks` 且转人工确认；工具失败必须留 evidence trace |

---

## 5. 行动路线图（D-0 到 D+30）

这是用户后续可以直接按这个清单往下走的路线。每一阶段独立可交付。

### 阶段 0 — 量化痛点（D-0，1 天，不写代码）

**目标**：用 3-5 个真实跑过的 task 看清"假完成"分布。

**交付物**：一张表格 `docs/design/task-failure-audit-2026-05.md`，列出：
- task 名称 / 输入
- 哪些 subtask 假完成（agent 动了工具但产出无意义）
- 哪些 refiner 该拦没拦
- report 里几条幻觉

**为什么先做这个**：Anthropic 那篇 multi-agent research 反复说 *"Think like your agents"*。看清失败模式后，后续每一刀都该优先打这些模式。

### 阶段 1 — 写 Recon 的 Phase Charter（D+1～D+2，纯文档）

**目标**：把 Recon 阶段的 5 问答固化成 Markdown，作为后续 system prompt 注入源。

**交付物**：`backend/crates/golish-agent-kit/src/harness/phase_charters/recon.md`（或类似路径）

**结构**：
```markdown
# Recon Phase Charter

## Identity
- Agent type: pentester_recon
- Phase index: 2 / 7

## Inputs
- scope: [...]
- known_assets: [...]

## Outputs (REQUIRED)
You MUST call `submit_recon_deliverable(json)` with the following schema: ...

## Allowed Tools
- dns_resolve
- subdomain_enum_passive
- whois_lookup
- ...

## Forbidden Actions
- Do NOT scan IPs outside scope.
- Do NOT attempt exploitation.
- Do NOT skip submitting the deliverable.

## Done Criteria
You are done ONLY when:
1. submit_recon_deliverable was called and parsed successfully
2. validate_recon_gate returned allowed=true
```

**为什么不写代码**：领域骨架的清晰度决定一切。Charter 写完后用户自己会发现"哪些是必须的、哪些可以推后"。

### 阶段 2 — 列 Recon Tool Belt 白名单（D+2，半天，表格）

**目标**：盘点现有 `tool_executors/` 能在 Recon 用什么、缺什么。

**交付物**：`docs/design/recon-tool-belt-2026-05.md`，每一项含：
- 工具名 / 现状（已有 / 待开发 / 调外部 API）
- 风险等级（被动 / 半被动 / 主动）
- 输出格式（能不能直接喂给 `ReconDeliverable`）

### 阶段 3 — 加 Evaluator 钩子（D+3～D+4，最小起点代码）

**目标**：不动 orchestrator 主流程，给 `AgentExecutor` trait 加一个 default-Option 的 `evaluate_subtask` 钩子。

**实现要点**：
- trait 加方法，默认 `Ok(None)`
- `execute_single_subtask` 在 return 前调一次
- 不 pass 时把结果改写为 `[EVALUATOR_FAILED] ...` 喂给 refiner
- feature flag 控制（默认关）
- Evaluator LLM 调用：独立 system prompt（强调 skeptical）、推荐用与 executor 不同的模型供应商

**交付物**：
- 改 `task_orchestrator/types.rs`
- 改 `task_orchestrator/subtask_phases/execute.rs`
- 在 `golish` crate 提供具体 `evaluate_subtask` 实现
- 单测覆盖 pass / fail / parse-error

### 阶段 4 — 给 `PlannedSubtask` 加 `acceptance_criteria`（D+5～D+7）

**目标**：把"sprint contract"装进去。Generator 输出每个 subtask 时附带 3-5 条机器可读验收条件。

**实现要点**：
- 改 `PlannedSubtask`、加 `acceptance_criteria: Vec<String>`（`#[serde(default)]` 保后向兼容）
- 改 generator prompt，要求输出该字段
- Evaluator 从"自由心证"变成"逐条打勾"

### 阶段 5 — 实现 Recon harness 完整版（D+8～D+15）

**目标**：把 Recon 这个阶段的 4 件套全部装上。

**交付物**：基本对应上一份计划 [`docs/superpowers/plans/2026-05-20-golish-agent-harness.md`](../superpowers/plans/2026-05-20-golish-agent-harness.md) 中的 9 个 Task：
1. `harness::recon` 模块骨架
2. `ReconDeliverable` 等 DTO
3. Gate 失败测试（TDD 红）
4. `validate_recon_gate` 实现（TDD 绿）
5. Barrier JSON 解析
6. `PlannedSubtask.harness_phase` 标记
7. `execute_single_subtask` 接入 gate
8. `ReconGateEvaluated` 事件
9. 更新 `harness-recon-mvp.md` 文档

但 **加上前 4 个阶段做铺垫后再做这件事，难度会显著降低**——因为你已经手上有 Charter、Tool Belt、Evaluator 钩子、acceptance criteria。

### 阶段 6 — 复制到其他 6 个阶段（D+16～D+30+）

**目标**：用 Recon 提炼出的模板把 Enumeration / Vuln ID / Reporting 等阶段也装上 harness。

每个阶段重复阶段 1 / 2 / 5 的产物，复用阶段 3 / 4 已经加好的通用钩子。

---

## 6. 关键技术决策（待用户拍板）

| # | 决策点 | 选项 | 建议 |
|---|---|---|---|
| D1 | Evaluator 用什么模型 | 同 executor / 不同供应商 / Haiku-class 小模型 | **不同供应商**：避免同模型自说自话 |
| D2 | Gate 用硬规则 vs LLM-as-judge | 纯规则 / 纯 LLM / 混合 | **混合**：硬规则一道门 + LLM 复核作二道门。安全场景必须有硬规则 |
| D3 | 持久化用 DB vs JSON 文件 | sqlite migration / JSON 落地 | **先 sqlite migration**：Golish 已有 repo trait，沿用即可 |
| D4 | acceptance_criteria 用谁生成 | generator 一并产出 / 单独 LLM 调用 | **generator 一并**：减少调用次数 |
| D5 | 跨 session resume | 无 / 文件 / DB | **DB**：现有 `subtasks` 表加几列即可 |
| D6 | 阶段 charter 注入方式 | 拼到 system prompt / 单独 message | **拼到 system prompt**：Anthropic Building Effective Agents 推荐 |

---

## 7. 风险与回滚

| 风险 | 影响 | 缓解 |
|---|---|---|
| Evaluator 过严，所有 subtask 都被打回 | 用户体验暴跌 | feature flag + A/B 比较 + skip 列表 |
| LLM 不输出合法 JSON | barrier parse 失败 | 复用 `golish-json-repair` |
| Recon gate 误杀正常空结果 | 例如目标无开放端口 | `skipped_checks + skip_reason` 显式申报 |
| Generator 不按 acceptance_criteria schema 输出 | 字段缺失 | `#[serde(default)]` + Rust 侧 fallback 推断 |
| 改动破坏现有 task 执行 | 现网用户感知 | 全部新增字段 / 钩子默认关闭 |

回滚策略：每个改动**默认 feature flag off**，单独验证后再开。

---

## 8. 关联文档

| 文件 | 作用 |
|---|---|
| `docs/design/harness-recon-mvp.md` | Recon 阶段的设计草案（Phase Charter 的雏形） |
| `docs/superpowers/plans/2026-05-20-golish-agent-harness.md` | Recon harness 的 9-Task 实现计划（对应本文阶段 5） |
| `docs/design/2026-05-20-agent-harness-strategy.md` | **本文档**——总策略 |

---

## 9. 外部参考

| 资料 | 链接 | 关键信息 |
|---|---|---|
| Anthropic - Building Effective Agents | https://www.anthropic.com/engineering/building-effective-agents | workflow vs agent；5 个 workflow patterns；ACI 设计 |
| Anthropic - Effective harnesses for long-running agents | https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents | Initializer / Coding 二段式；feature_list.json；context reset > compaction |
| Anthropic - Harness design for long-running application development | https://www.anthropic.com/engineering/harness-design-long-running-apps | Planner / Generator / Evaluator 三角；sprint contract；skeptical evaluator |
| Anthropic - How we built our multi-agent research system | https://www.anthropic.com/engineering/multi-agent-research-system | orchestrator-worker；scale effort to query complexity；start wide narrow down |
| LangChain - The Anatomy of an Agent Harness | https://www.langchain.com/blog/the-anatomy-of-an-agent-harness | harness 通用解剖 |
| Sajal Sharma - Agents Have Outgrown Workflows | https://sajalsharma.com/posts/agentic-workflows-to-agent-harnesses | workflow → harness 演进 |
| PentAGI 源码 | 本机 `~/Downloads/pentagi-main/` | `backend/pkg/controller/task.go`、`subtask.go`、`templates/prompts/` |

---

## 10. AI 接手指南（重要）

如果你是后续接手本话题的 AI，请按以下顺序读：

1. 本文档（全策略图景）
2. `docs/design/harness-recon-mvp.md`（Recon 阶段细节）
3. `docs/superpowers/plans/2026-05-20-golish-agent-harness.md`（Recon 阶段 9 Task 计划）
4. 代码：
   - `backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs`
   - `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs`（看 `AgentExecutor` trait）
   - `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**不要重新调研 Anthropic 资料**——上面 §9 已经把链接列全，相关结论也写在 §2.3 和 §3 里。

**不要重新讨论"该不该改 orchestrator"**——结论是不改，叠 harness。已在 §1 / §3.4 论证。

**接下来应该做的事**：参考 §5 路线图，根据用户当前所处阶段往下走。

---

## 11. 变更日志

| 日期 | 作者 | 变更 |
|---|---|---|
| 2026-05-20 | MCP-1 全栈工程师 | 初稿——总策略 + 三层模型 + 6 阶段路线图 |
