# Operation Harness · 对标主流 + 差距分析（2026）

> 目的：把 Golish 的 harness/引擎 对标 **2026 主流 agentic 架构 + AI 渗透框架**，回答三件事：①你的架构对吗 ②现在主流逻辑是什么 ③有什么更好的框架/实现可借鉴。作为后续决策与改造的输入。
>
> 证据来源：① 本仓库代码（前 5 份 harness 文档已逐一核对）② 2026 公开资料（见 §6）。
> 配套：引擎底座 `2026-06-02-pentagi-engine-substrate-reference.md`；执行层 `2026-06-02-harness-execution-layer-reference.md`；拓扑层 `2026-06-02-harness-topology-reference.md`；节点层 `2026-06-02-harness-stage-spec-reference.md`；总览 `2026-06-01-harness-explainer-and-decisions.md`。日期：2026-06-02。

---

## 0. 结论（TL;DR）

- **架构方向正确**，与 2026 主流主干一致：orchestrator-worker + plan-execute + reflection + evaluator-optimizer + HITL + 有状态图编排。
- **超前点**：你有「确定性 gate + evidence ledger」**治理层**，多数开源（含 PentAGI）没有——这正是业界往「可信 / 可验证」走的方向。
- **弱点在成熟度**：治理的执行偏 skeleton——gate 多数 check 近似、evidence ledger 没建、分支策略写死取第一候选、引擎工具面与 harness stage 工具面两套过滤没打通。
- **一句话**：**设计对，缺的是把治理做「实」。**

---

## 1. 你的架构 ↔ 主流模式对照

| 你的组件 | 对应主流模式 | 你的成熟度 |
|---|---|---|
| PentAGI 引擎（Generator→primary→subtask→sub-agents） | orchestrator-worker + plan-execute | ✅ 成熟 |
| agentic loop + reflector 重试 | ReAct + reflection | ✅ 成熟 |
| Gate（确定性校验 deliverable，PASS/BLOCK） | evaluator-optimizer（doer vs judge）+ guardrails | ⚠️ 骨架（多 check skeleton/近似） |
| Profile × DAG 投影 + 游标 | plan-execute + stateful graph | ⚠️ 有图无全量检查点 |
| human_approval hold→wait→resume | HITL interrupt | ✅ 机制有，默认流程少触发 |
| operation_state 游标 | checkpointer persistence | ⚠️ 只游标；stage_runs 空 |
| evidence_refs / EvidenceAuditId | guardrails 的证据底座 | ❌ evidence_audit 表未建 |

> 重点：你**不是走偏了，是走在主流主干上**。差距集中在右列「成熟度」。

---

## 2. 现在主流的逻辑

### 2.1 通用 agentic（2026 共识，LangGraph 是事实标准）

- **有状态图编排**：节点=函数、状态显式、条件边路由、checkpointer（Postgres）持久化、HITL interrupt、subgraph 分层。
- **5 个核心模式**：
  - **ReAct**：想→做→看→重复（探索型任务）。
  - **plan-execute**：planner 出多步计划/DAG，executor 跑，**失败才 replan**（明确多步任务，省 LLM 调用）。
  - **orchestrator-worker**：supervisor 动态派子任务给专家。
  - **reflection**：自我批评→迭代（通常 2–3 轮收敛）。
  - **evaluator-optimizer**：doer 与 judge（rubric / LLM-as-judge）分离 = 给 agent 做 TDD。
- **上生产三件套**：strict guardrails（schema 校验 / PII 脱敏）+ observability（结构化 trace / LLM-as-judge eval）+ MCP（版本化、可发现的工具目录）。

### 2.2 AI 渗透专门（2026）

- **XBOW**（闭源标杆）：全自治、**每个发现用真实利用独立验证**（消除误报、给可复现证明）、深度优先、持续化、no human in loop。
- **PentAGI**（开源，你已对标）：多 agent（researcher / developer / executor）、Docker 沙箱内 20+ 专业工具、Graphiti 长期记忆/知识图、10+ LLM provider。
- **Pentest Agent Suite**（开源）：50 个专家 agent、MCP server、FAISS 语义 writeup 检索（测前查先验）、所有破坏性操作 `--execute` 显式门控。
- **共性赢家逻辑**：沙箱隔离 + RAG 先验检索 + **真实利用验证** + 持续化。

---

## 3. 框架 / 标杆速览

| 框架 | 类型 | 关键特征 | 对你的价值 |
|---|---|---|---|
| **LangGraph** | 编排框架（Py/TS） | StateGraph + checkpointer + interrupt + subgraph | 借「显式状态图 + 检查点 + 中断」范式（你是 Rust 手搓，借模式不直接用） |
| **XBOW** | 闭源 AI 渗透平台 | 每发现真实利用验证、深度、持续 | 抓「validate-by-exploitation」放进你的 gate |
| **PentAGI** | 开源多 agent 渗透 | 沙箱 + Graphiti 记忆 + 多 LLM | 对齐沙箱隔离 + 记忆/知识图 |
| **Pentest Agent Suite** | 开源 bug bounty 框架 | 50 专家 + RAG writeup + `--execute` 门控 | 借 RAG 先验检索 + 破坏性操作门控 |

---

## 4. 差距 + 借鉴清单（4 个最该补，均对应已有文档里的缺口）

1. **gate 升级成「真实利用验证」**（XBOW 式 / evaluator-optimizer 精髓）
   - 现状：`verification` 阶段没有「必须有 exploit_proof」强校验（见执行层文档 §4、节点层 §6.4）。
   - 借鉴：critical 发现必须带可复现 PoC 证据，gate 才放行——这是渗透的信任分水岭。
2. **真正的 checkpointer / persistence**（LangGraph 式）
   - 现状：`operation_state` 只是游标，`stage_runs` 空、`evidence_audit` 没建（执行层 §6）。
   - 借鉴：全量状态检查点 → 容错 + time-travel 调试 + 跨 resume 持久化。
3. **分支用条件路由**（plan-execute 的 replan）
   - 现状：DAG「永远取第一候选」，4 条早退边形同虚设（拓扑层 B5）。
   - 借鉴：按 LLM / rubric / 阶段结果决定分支（如没找到攻击面就 bail 到 reporting）。
4. **RAG 先验检索**（Pentest Agent Suite / PentAGI Graphiti）
   - 现状：有 knowledge / graph 工具，但「测漏洞前先查历史 exploit/writeup」未成强约束。
   - 借鉴：把先验检索做成每个漏洞类测试前的标准前置。

---

## 5. 优先级建议

| 优先级 | 动作 | 为什么 |
|---|---|---|
| **P0** | 落 **Evidence Ledger**（evidence_audit 表 + 工具产出入账） | 是「gate 真校验」和「checkpoint」的共同前置，解锁 1 和 2 |
| **P1** | `verification` 真实利用验证强校验 | 信任分水岭；evaluator-optimizer 落地 |
| **P2** | 分支条件路由 + 打通两套工具过滤 | 让 DAG 早退边真正可用；防幽灵工具 |
| **P3** | RAG 先验检索 + 持续化运行 | 提质 + 对齐 continuous 主流 |

> P0 一通，gate 的多数 skeleton check（contract 真计数 / freshness 真 age / min_invocations 真计数 / scope 真 label）就能从「信自报」升到「可交叉验证」。

---

## 6. 框架决策（2026-06-02 定）

**结论：不采用任何现成框架整体替换；保持自研 Rust 引擎（rig + task_orchestrator + harness），从开源项目借鉴设计与代码来补成熟度。**

依据（本轮调研了约 12 个 Rust agent 项目）：

- **Rust 没有「既成熟又合适」的多 agent 框架。** 高星的（OpenFANG 17.7k★ / IronClaw 11.8k★ / ZeroClaw 31.5k★）是 OpenClaw 式个人助理**成品**，不是可嵌入库；可嵌入的多 agent 库全 < 1k★、pre-1.0。
- **可嵌入候选对比**：

| 库 | ★ | 贴你需求吗 | 成熟吗 | 拦路 |
|---|---|---|---|---|
| AutoAgents | 663 | 原生工具好 | v0.3 | 无 HITL、持久化弱 |
| GraphBit | 529 | 理念同你 | pre-1.0 | Python 壳 |
| Heartbit | 较少 | 架构最像你 | 较成熟但早 | **flat 不能嵌套** + 无沙箱 |
| metalcraft | 0 | **图式·同栈·支持嵌套** | v0.3 / 1 人 | **bus-factor 1** |

- **嵌套委派只有「图式」框架天然支持**；orchestrator-worker 式（Heartbit / AutoAgents）有 flat 限制（你 pentester→coder 用不了）。
- **安全产品**把核心引擎压在 bus-factor-1 / 0 用户的外部 crate 上 = 供应链 + 延续风险。

→ 自研引擎留着（合适、可控、已支持嵌套），**借设计而非引依赖**。

## 7. 借鉴清单

| 从哪借 | 借什么 | 补你哪个缺口 |
|---|---|---|
| **OpenFANG** | Merkle 哈希链审计 / 16 安全层（taint / loop-guard / capability gate / SSRF / prompt-injection）/ subprocess sandbox 跑原生工具 | **P0 evidence ledger** + 安全 |
| **metalcraft** | 图执行器 + Checkpointer trait + interrupt/resume + 确定性并行合并 + 怎么接 rig（代码已读·质量好·MIT·~2k 行，可 vendor / 照抄，不引为依赖；**逐行 review 详见附录 A**） | **P2 checkpointer + 分支条件路由** |
| **Heartbit** | 引擎架构写法（AgentRunner / Orchestrator 分离 · 4-hook guardrails）+ 内置 eval 框架 + Postgres 记忆 | 引擎清晰度 + **gate/eval** |
| **AutoAgents** | `#[tool]` 派生宏工具人体工学 + guardrails(Block/Sanitize/Audit) + 设计模式示例 | 工具定义 + 护栏 |
| **GraphBit** | 「LLM 只推理 · 编排交确定性引擎」理念背书（arxiv 2605.13848） | 验证 harness 方向正确 |
| **XBOW**（概念） | 每个 finding 真实利用验证 | **P1 verification gate** |

落地方式：metalcraft 小 + MIT + 同栈 → **vendor 一份你掌控的内部模块 / 照抄其设计**，把「图 + checkpointer + interrupt」实现进你自己代码；其余按上表逐项借。**核心引擎不引入 bus-factor-1 / pre-1.0 外部 crate 作硬依赖。**

## 8. 来源

公开资料（2026）：

- LangGraph + MCP 2026 生产构建指南（ailearningguides.com）
- arxiv 2602.10479 —— From Prompt–Response to Goal-Directed Systems: The Evolution of Agentic AI Software Architecture
- The Definitive Guide to Agentic Design Patterns in 2026（sitepoint.com）
- Plan-and-Execute Agents（langchain.com/blog/planning-agents）
- XBOW —— "We Ran 1,060 Autonomous Attacks" + 平台页（xbow.com）
- PentAGI（github.com/vxcontrol/pentagi，v2.1.0）
- Pentest Agent Suite（cybersecuritynews.com）

本地证据：本仓库 harness/引擎代码 + 前 5 份 `docs/design/2026-06-0*-harness-*` / `pentagi-engine-*` 参考文档。

---

## 附录 A · metalcraft 源码深挖（2026-06-02 调研落盘）

> 来源：2026-06-02 框架调研会话对 GitHub `rust4ai/metalcraft` 的 README / docs.rs + `executor.rs` / `checkpoint.rs` 源码逐行 review。本附录把当时只存在于对话里的研究笔记落盘，**未在本次会话重新拉取源码复验**；真要 vendor / 照抄落地前，建议再核一遍最新 commit。

### A.1 项目身份

- 仓库：`rust4ai/metalcraft`
- 星标 / 维护：**0 ★、1 人（ethereumdegen）**、2026-05-01 新建、版本 **v0.3**
- 一句话：一个人一个月前刚开的高质量小项目——「高质量但未被发现」，不是烂项目。

### A.2 设计契合度（几乎是「梦想中的」那套，README / docs.rs 实证）

- LangGraph 式**有状态图** + typed state + reducer + **循环图**
- **Checkpointer trait + MemoryCheckpointer**（正好补你缺的持久化）
- **HITL interrupt + resume**
- **嵌套委派**：图式 + 条件边天然支持，正是 flat 框架（Heartbit / AutoAgents）做不到的

### A.3 源码质量（逐行看 `executor.rs` / `checkpoint.rs`，写得好）

| 维度 | 实证 |
|---|---|
| 错误处理有思考 | 节点失败用 `RunOutcome::Failed { state, node, error }` **保留部分状态**；注释还解释「以前会丢状态，现在保住」——成熟设计 |
| 并行合并确定性 | `execute_parallel` 用 `FuturesUnordered` 跑，但应用前 `sort_by(name)` 保证顺序**可复现** |
| 扩展性 | builder + trait 模式，结构清晰 |

### A.4 为什么「写得好」≠「能当核心依赖」

- **bus-factor 1**：1 个人维护，他一停你就得自己接。
- **v0.3 会有 breaking change**、**0 生产用户验证**。
- **范围小**：它只是「图执行器 + checkpointer」核心；工具 / 记忆 / eval / harness 你仍得自己堆。
- **安全产品**把核心引擎压在 1 人 / 0 星 crate 上 = 供应链 + 延续风险。

### A.5 结论 · 最佳用法（刚好完美）

metalcraft **MIT + 小（~2k 行）+ 同栈（rig）+ 写得好** → **不当外部依赖，而是 vendor 一份你掌控的内部模块 / 照抄其设计**，把「图 + Checkpointer trait + interrupt/resume + 确定性并行合并」实现进你自研代码。

→ 对应补 §5 的 **P2（checkpointer + 分支条件路由）**，与 §7 借鉴清单 metalcraft 行一致。

---

## 附录 B · 讨论过的项目逐项落盘（框架 / 标杆全集）

> **provenance（诚实标注）**：本附录正文（B.1–B.7）最初把 §3 / §6 / §7 散落的项目研究**集中落盘**，星标 / 内部细节来自 **2026-06-02（MCP-4）那轮调研**，**写入当下本会话并未亲自复验**——此点被用户当场质疑，合理。**故于 2026-06-02 当场补做 web 核对**（README / docs.rs / crates.io），结果见 B.0.0：**项目均真实存在、关键特征大体属实**，仅星标随来源 / 日期浮动、个别拼写需更正；**唯 metalcraft 源码行级断言（附录 A 的 `executor.rs` / `checkpoint.rs`）仍未亲自逐行 clone 复验，vendor 前必须补。**

### B.0.0 · web 核对结果（2026-06-02 当场补验）

| 项目 | 真实 repo | ★（核对） | 关键特征核对 | 与原文出入 |
|---|---|---|---|---|
| LangGraph | `langchain-ai/langgraph` | 高 | StateGraph + checkpointer + interrupt 属实 | 无 |
| OpenClaw（被引为「风格」基准） | `openclaw/openclaw` | ~374k | TS 个人助理成品属实 | 原文未单列，仅作风格代称 |
| OpenFang | `RightNow-AI/openfang` | ~17.6k | Agent OS · Rust · 16 安全层 · 137k LOC/14 crate/1767+ test 属实 | 拼写 OpenFANG→**OpenFang** |
| ZeroClaw | `zeroclaw-labs/zeroclaw` | ~26–31.5k | Rust · OpenClaw 重写 · 22+ LLM provider 属实 | 星标随来源/日期浮动 |
| IronClaw | `nearai/ironclaw` | ~12.3k | Rust · WASM 沙箱 · 加密验证 | 原文 11.8k → 实测 **~12.3k** |
| **metalcraft** | `rust4ai/metalcraft` | 低（新） | LangGraph 式 · Checkpointer trait · MemoryCheckpointer · HITL interrupt · 并行 · RunOutcome · rig · step guards 均属实 | ⚠️ 附录 A **源码行级**断言仍未亲自复验 |
| Heartbit | `heartbit-ai/heartbit` | — | AgentRunner/Orchestrator 分离 · **flat（子 agent 不再派）** · Guardrail hook · Postgres · eval 属实 | 「flat 不能嵌套」✓ 证实 |
| AutoAgents | `liquidos-ai/autoagents` | — | ReAct · **`#[tool]` 派生宏** · WASM 沙箱 · Guardrails · Ractor pub/sub 属实 | 持久化=滑窗记忆（偏弱）✓ |
| GraphBit | `InfinitiBit/graphbit` | — | **Rust 核 + Python 壳** · 确定性 DAG · 防 LLM 路由幻觉 属实 | 「Python 壳」✓ |

> 核对方式：web 搜索 README / docs.rs / crates.io（**非亲自 clone 源码**）。**选型结论不变**——metalcraft 仍是唯一适合 vendor 的图式底座（C.1），web 进一步证实其 Checkpointer / RunOutcome / rig / HITL 特征属实；但 **vendor 落地前必须真 clone 读 `executor.rs` / `checkpoint.rs`** 复验附录 A 的行级细节（`RunOutcome::Failed` 保状态、`FuturesUnordered`+`sort_by` 确定性合并）。

metalcraft 已单列**附录 A**，本附录不重复，只在对比表（B.8）保留一行。

### B.0 先归三类（11 个项目一眼分清）

| 类 | 项目 | 能不能直接拿来用 | 我们怎么用 |
|---|---|---|---|
| **范式来源**（Python，跨不过语言边界） | LangGraph | ❌ 不引本体 | 借「图 + checkpointer + interrupt」**设计范式** |
| **成品类**（高星，是 App 不是库） | OpenFANG / IronClaw / ZeroClaw | ❌ 接不进来 | 仅借**单点设计**（OpenFANG 的安全/审计），另两个仅登记 |
| **可嵌入库**（Rust，<1k★ / pre-1.0） | metalcraft / Heartbit / AutoAgents / GraphBit | ⚠️ 可 vendor / 借写法，不引硬依赖 | **metalcraft 做范式底座**，其余借局部 |
| **渗透标杆**（对标信任/能力，非通用框架） | XBOW / PentAGI / Pentest Agent Suite | — | 借**渗透专门逻辑**（利用验证 / 沙箱记忆 / RAG 先验） |

### B.1 LangGraph（范式来源 · Python/TS）

- **身份**：2026 通用 agentic 事实标准；StateGraph + Postgres checkpointer + interrupt + subgraph，MCP adapter 一等公民、生产级成熟。
- **借什么**：**显式状态图 + 全量检查点 + 中断/恢复**这套范式（不是代码）。
- **为什么不直接用**（与正文 §6 / 转移摘要一致）：① 后端纯 Rust，Python 运行时打包进 Tauri = 体积 / 签名 / 跨平台死结；② harness 的 gate / evidence / DAG / approval 全在 Rust，流程挪到 Python 要么重写治理、要么每步跨 IPC 当裁判；③ 双语维护破 `ts-rs` 单一真相源。
- **结论**：**管子（MCP）留用，本体不引**——用 Rust 把它的范式手搓出来（即 metalcraft 路线）。

### B.2 OpenFANG（成品 · 17.7k★ · 借安全层 + 审计链）

- **身份**：OpenClaw 式个人助理**成品 App**，不是可嵌入库 → 整体接不进来。
- **借什么**（单点设计，对应 §7）：**Merkle 哈希链审计**、**16 层安全**（taint / loop-guard / capability gate / SSRF / prompt-injection 等）、**subprocess sandbox 跑原生工具**。
- **补我们哪**：**P0 evidence ledger 的防篡改思路** + 引擎安全护栏。哈希链尤其值得抄进 evidence 入账（让证据链可验证、不可悄改）。

### B.3 Heartbit（可嵌入 · 架构最像你 · 借引擎写法 + eval）

- **身份**：可嵌入多 agent 库，**架构与 Golish 最接近**，较成熟但仍早期。
- **拦路**：**flat 编排，不能嵌套**（你的 `pentester → coder/researcher` 嵌套委派做不了）+ 无沙箱。
- **借什么**：**引擎架构写法**（AgentRunner / Orchestrator 分离、4-hook guardrails）+ **内置 eval 框架** + **Postgres 记忆**。
- **补我们哪**：引擎清晰度 + **gate/eval**（evaluator-optimizer 落地的参考实现）。

### B.4 AutoAgents（可嵌入 · 663★ · 借工具人体工学 + 护栏）

- **身份**：原生工具体验好的 Rust agent 库，v0.3。
- **拦路**：**无 HITL、持久化弱**（正是渗透平台刚需的两块）。
- **借什么**：`#[tool]` **派生宏**的工具定义人体工学、guardrails(Block/Sanitize/Audit) 三态、设计模式示例。
- **补我们哪**：工具定义工效 + 护栏写法（可对照现有 `tool_definitions.rs` / `tool_policy.rs`）。

### B.5 GraphBit（可嵌入 · 529★ · 借理念背书）

- **身份**：理念与你高度一致——**「LLM 只推理，编排交确定性引擎」**（arxiv 2605.13848），pre-1.0、Python 壳。
- **借什么**：**理念背书**——验证「确定性 harness 外壳 + LLM 引擎底座」方向正确。
- **补我们哪**：不借代码；作为**架构方向的外部佐证**（与 §0 结论互相印证）。

### B.6 IronClaw（11.8k★）/ ZeroClaw（31.5k★）— 成品类 · 仅登记

- **身份**：均为 OpenClaw 式个人助理**成品 App**，非可嵌入库。
- **借什么**：作为库**无可借**；若需要可参考其**交互 UX / 工具调用编排的产品形态**，但与「嵌入式 Rust 引擎」目标无直接关系。
- **结论**：**仅登记，不纳入选型**（高星 ≠ 对你有用，star 数会误导）。

### B.7 渗透标杆三项（XBOW / PentAGI / Pentest Agent Suite）

> 这三个不是「通用 agent 框架」，是「**渗透专门标杆**」——对标的是**信任分水岭与能力闭环**，不是拿来当底座。

| 标杆 | 类型 | 借什么 | 补我们哪 |
|---|---|---|---|
| **XBOW**（闭源） | AI 渗透平台 | **每个 finding 用真实利用独立验证**（消误报、给可复现证明）、深度优先、持续化 | **P1 verification gate**（validate-by-exploitation） |
| **PentAGI**（开源 · 你已对标） | 多 agent 渗透 | Docker 沙箱内 20+ 工具、**Graphiti 长期记忆 / 知识图**、10+ LLM provider | 沙箱隔离 + 记忆/知识图（你引擎血统就是它，`orchestrator.rs` 对标 `NewTaskWorker`） |
| **Pentest Agent Suite**（开源） | bug bounty 框架 | 50 专家 agent、MCP server、**FAISS 语义 writeup 检索**（测前查先验）、破坏性操作 `--execute` 显式门控 | **P3 RAG 先验检索** + 破坏性操作门控 |

### B.8 横向总表（含 metalcraft，一眼看完）

| 项目 | 类型 | ★ | 可嵌入 | 支持嵌套 | HITL | 持久化 | 同栈(rig/Rust) | 能当底座 | 借鉴点 |
|---|---|---|---|---|---|---|---|---|---|
| LangGraph | 范式(Py) | 高 | ❌ | ✅ | ✅ | ✅(PG) | ❌ | ❌ | 图+checkpoint+interrupt 范式 |
| OpenFANG | 成品 | 17.7k | ❌ | — | — | — | ❌ | ❌ | Merkle 审计链 / 16 安全层 / sandbox |
| IronClaw | 成品 | 11.8k | ❌ | — | — | — | ❌ | ❌ | （仅登记） |
| ZeroClaw | 成品 | 31.5k | ❌ | — | — | — | ❌ | ❌ | （仅登记） |
| **metalcraft** | 图式库 | 0 | ✅ | ✅ | ✅ | ✅(trait) | ✅ | **✅ vendor** | 图执行器+Checkpointer+并行合并（附录 A） |
| Heartbit | 编排库 | 较少 | ✅ | ❌flat | 部分 | ✅(PG) | ✅ | ⚠️ 部分 | 引擎写法 + eval + 记忆 |
| AutoAgents | 编排库 | 663 | ✅ | ⚠️ | ❌ | 弱 | ✅ | ❌ | #[tool] 宏 + guardrails |
| GraphBit | 编排库 | 529 | ⚠️Py壳 | — | — | — | ⚠️ | ❌ | 理念背书 |
| XBOW | 渗透标杆 | 闭源 | — | — | ❌ | — | — | ❌ | 真实利用验证 |
| PentAGI | 渗透标杆 | 开源 | — | ✅ | — | ✅ | ❌(Go) | ❌ | 沙箱+Graphiti 记忆 |
| Pentest Agent Suite | 渗透标杆 | 开源 | — | — | — | — | — | ❌ | RAG 先验+`--execute` 门控 |

> 一句话读表：**能当底座的只有 metalcraft 那一行（且是 vendor、不是引依赖）**；其余要么跨语言（LangGraph/PentAGI）、要么是成品 App（OpenFANG/IronClaw/ZeroClaw）、要么有硬伤（Heartbit flat / AutoAgents 无 HITL）。

---

## 附录 C · 底座选型 + 怎么自己手搓

### C.1 底座结论：自研 Rust 引擎（保留）+ vendor metalcraft 图范式（新增）

**一句话**：**底座 = 你现有的 PentAGI 式自研引擎（`golish-agent-kit/src/task_orchestrator` + `golish-agent-runtime/src/agentic_loop`），不换**；**新增一层 metalcraft 式「图 + Checkpointer」做状态/分支/恢复**，照抄进自研代码，**不引任何外部 crate 作硬依赖**。

为什么是这个组合（逐个排除）：

| 候选做底座 | 判决 | 理由 |
|---|---|---|
| LangGraph / PentAGI | ❌ | 跨语言（Py / Go），打包进 Tauri 纯 Rust 后端 = 死结 |
| OpenFANG / IronClaw / ZeroClaw | ❌ | 是成品 App 不是库，接不进来 |
| Heartbit | ❌ 当底座 | flat 不能嵌套（你的核心需求做不了）；可借写法 |
| AutoAgents | ❌ 当底座 | 无 HITL + 持久化弱（渗透刚需缺两块）；可借工具宏 |
| **metalcraft（vendor）** | ✅ **范式底座** | 图式·支持嵌套·同栈(rig)·MIT·~2k 行·写得好；唯一硬伤 bus-factor 1 → **vendor 化解** |
| **你的自研引擎** | ✅ **执行底座** | 已成熟（agentic loop / 13 sub-agent / 嵌套委派 / plan-execute-refine），可控、已支持嵌套 |

> 关键区分：**「执行底座」是你的引擎**（真正跑渗透的 LLM 多 agent），**「状态/治理底座」是 metalcraft 范式 + 你的 harness 三层**（图 + checkpointer + gate）。两者不冲突，是上下层关系（引擎是 substrate，harness 寄生其上——见 `pentagi-engine-substrate-reference.md` §0）。

### C.2 手搓总策略（三句话）

1. **留引擎**：`task_orchestrator` + `agentic_loop` 不动主干，只在它的 hook 点（`tool_dispatch` / 交付点 / 切阶段）继续挂治理。
2. **vendor 图范式**：把 metalcraft 的 `executor.rs` / `checkpoint.rs`（图执行器 + `Checkpointer` trait + interrupt/resume + 确定性并行合并）**照抄成你掌控的内部模块**（如 `golish-agent-kit/src/harness/graph_engine/`），按你的 `StageKind` / `operation_graph.json` 适配。
3. **借表逐项**：安全/审计抄 OpenFANG，工具宏/护栏抄 AutoAgents，引擎写法/eval 抄 Heartbit，验证逻辑对标 XBOW，RAG 先验对标 Pentest Agent Suite。

### C.3 分阶段手搓步骤（落到 crate / 表 / 函数 / hook，对齐 P0–P3）

> 标注现状用本次会话**已核对的真实代码**（见附录 D.0 勘误）：✅ 已有 / ⚠️ 半成品 / ❌ 待建。

**P0 · 闭合 Evidence Ledger 写入闭环**（最高优先，是其余的共同前置）
- ✅ schema：`golish-db/migrations/20260601000001_evidence_ledger.sql`（audit_log.audit_role + evidence_classifications + operation_state + stage_runs + sprint_contracts）已落。
- ✅ 读路径：`golish-pentest-app/src/evidence.rs::evidence_read`（sanitize + 三态 freshness + scope_label + IDOR）已实现且有单测。
- ✅ 域层：`golish-pentest/src/evidence_ledger/{mod,types}.rs`（类型 + `ScopeService` trait + `InMemoryScopeService`）已有。
- ❌ **写路径（要手搓的核心）**：实现 `EvidenceLedger::append()`（现仅注释/类型，mod.rs §Phase 1b 推迟）→ 在引擎 `agentic_loop/turn/phases/tool_dispatch.rs` 工具执行**后置 hook**里，把工具产出写成 `audit_log(audit_role='evidence')` 行 + 同步写 `evidence_classifications`（落生产版 `ScopeService`，查 `organizations.scope_rules`）。
- ❌ **gate 回查**：把 `harness/gate/{contract_check,freshness_check,min_invocations_check,scope_check}.rs` 从「信 AI 自报 `evidence_refs`」改为「查 ledger 真计数 / 真 age / 真 InScope label」（代码注释已标 `推 Phase 2 接 EvidenceLedger`，见 `vacuous_check.rs:36`）。
- 抄谁：**OpenFANG Merkle 哈希链**做证据防篡改。

**P1 · verification 真实利用验证强校验**（信任分水岭）
- ⚠️ 现状：`stages/verification.json` 没有「必须有 `exploit_proof` 证据」的强 check。
- 手搓：加 evidence kind `exploit_proof` + 在 verification 的 `required_checks` 强制其非空，gate 缺证据→BLOCK。
- 抄谁：**XBOW**（每个 critical finding 带可复现 PoC 才放行）。

**P2 · checkpointer + 分支条件路由**（vendor metalcraft 主战场）
- ⚠️ 分支：`harness/stage_transition.rs::decide_transition` 已能判 `Branch`，但 `advance_target()` 对 `Branch` **直接取 `candidates.first()`** → 4 条早退边形同虚设（拓扑层 B5）。手搓：让 agent/规则按阶段结果选分支（如没攻击面就 bail 到 reporting）。
- ❌ checkpoint：`stage_runs` 表在但**从没写过行**（空）；`operation_state` 只当游标。手搓：vendor metalcraft `Checkpointer` trait → 每个 stage 起止写 `stage_runs` 行 + 全量状态进 `operation_state.state_blob`（容错 + time-travel + 跨 resume）。
- 抄谁：**metalcraft**（附录 A 的 `RunOutcome::Failed` 保状态 + `FuturesUnordered`+`sort_by` 确定性合并）。

**P3 · RAG 先验检索 + 持续化运行**
- ✅ 底子：`researcher` / `memorist` 已有 `knowledge` / `graph_*` / `search_exploits`。
- ❌ 强约束：把「测某漏洞类前先查历史 exploit/writeup」做成 `vuln_triage` 的前置 required check。
- 抄谁：**Pentest Agent Suite**（FAISS writeup 检索）+ **PentAGI**（Graphiti 知识图）。

> 另有一条贯穿性收口（非新功能、但 P0/P2 都依赖）：**打通两套工具过滤** —— 引擎角色工具面（`execution_mode/modes/task.rs`）与 harness 的 per-stage `allowed_tools`（`stages/*.json`）目前独立，`stage.allowed_tools` 里写的工具名必须真的在某 sub-agent 的 `with_tools` 里存在（防「幽灵工具」）。

---

## 附录 D · 现阶段需要的功能（按当前已验证的真实状态）

### D.0 ⚠️ 现状勘误（重要，先读）

正文 §1 / §5 / §6 与 5 份参考文档写于 evidence ledger **migration 落地之前**，多处说「evidence ledger ❌ 未实现 / `evidence_audit` 表未建」。**本次会话核对真实代码后更正**：

| 说法（旧） | 实情（2026-06-02 核对） | 证据 |
|---|---|---|
| 「evidence_audit 表未建」 | 设计上**本就没有**叫 `evidence_audit` 的表——证据是 `audit_log` 行 + `audit_role='evidence'`；该 schema **已随 migration 落地** | `migrations/20260601000001_evidence_ledger.sql` |
| 「evidence ledger 没建」 | **建了一半**：schema ✅ + 读路径 ✅ + 分类层 ✅ + 域类型 ✅；**缺写入入账（append）+ gate 回查** | `golish-pentest-app/src/evidence.rs`、`evidence_ledger/mod.rs` |
| 「stage_mode 默认 OFF」 | 现在**默认 ON**（显式 =false 才关） | `task_orchestrator/subtask_phases/execute.rs:477` |
| 「stage_runs 空」 | 仍准确：表在、**无写路径**（没写过行） | grep 无 `INSERT ... stage_runs` |

> 结论：P0 不是「从零建」，而是「**补上写入闭环 + gate 回查**」——已完成度比正文高，剩余工作更聚焦。

### D.1 现阶段功能清单（按优先级 · 每条含落点 + 验收 + 现状）

| 优先级 | 现阶段要的功能 | 落在哪 | 验收证据（done 的标准） | 现状 |
|---|---|---|---|---|
| **P0-a** | **证据自动入账**：工具一产出就写 `audit_log(audit_role='evidence')` + 分类行 | `tool_dispatch.rs` 后置 hook + `EvidenceLedger::append()` + 生产版 `ScopeService` | 跑一次 eas 阶段后，DB 里有对应 evidence 行 + `evidence_read` 能读回 | ❌ 待建 |
| **P0-b** | **gate 回查真证据**：contract 真计数 / freshness 真 age / min_invocations 真计数 / scope 真 label | `harness/gate/*.rs` 四个 check | 自报假 evidence_refs 会被 BLOCK（不再信自报） | ❌ 待建 |
| **P1** | **verification 必须带 exploit_proof** | `stages/verification.json` + gate | 无 PoC 证据的 critical finding 过不了 gate | ⚠️ 占位 |
| **P2-a** | **分支条件路由**：让早退边真正可用 | `stage_transition.rs` / `operation_graph.rs` | 没攻击面时能 bail→reporting（不再永远走第一候选） | ⚠️ 取第一候选 |
| **P2-b** | **全量检查点**：写 `stage_runs` + `state_blob`（vendor metalcraft） | `harness/graph_engine/`(新) + `golish-db` repo | 杀进程后能从上次 stage 恢复；stage_runs 有行 | ❌ 空表 |
| **P3** | **RAG 先验检索强约束** + 持续化运行 | `vuln_triage` 前置 check + researcher/memorist | 测漏洞前自动检索 writeup 并入交付 | ⚠️ 有工具无约束 |
| **贯穿** | **两套工具过滤打通**（防幽灵工具） | `task.rs` ↔ `stages/*.json` | `stage.allowed_tools` 工具名都能在 sub-agent `with_tools` 里找到 | ⚠️ 独立未通 |

### D.2 一句话下一步

> **先做 P0-a/P0-b（证据写入闭环 + gate 回查）**——它一通，gate 从「信自报」升到「可交叉验证」，P1/P2/P3 才有真证据可依赖；写入 hook 抄 OpenFANG 的哈希链，状态/分支抄 metalcraft（附录 A），落地方式 vendor 不引依赖。

---

> **正式化（2026-06-02）**：本文 §6 + 附录 C/D 的路线已落成正式设计与计划——
> - 设计：`docs/design/2026-06-02-golish-agent-engine-v2-design.md`（方案 B 嵌接+借鉴）
> - P0 实现计划：`docs/superpowers/plans/2026-06-02-engine-v2-p0-evidence-loop.md`（writing-plans · Task 1-7）
> - 功能登记：`feature_list.json` 的 `engine-v2-graft-2026-06-02`（not_started）
