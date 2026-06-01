# Golish Agent Engine v2 设计（方案 B：嵌接 + 借鉴）

> 目的：把「换掉 AI 流程这一块」落成可执行设计——**不推倒重写**，而是在成熟的 PentAGI 式自研引擎上 **graft 一层你掌控的 metalcraft 式「图 + 检查点」骨架**，并从已调研的开源项目**逐项借设计**补成熟度。
>
> 上游输入：`docs/design/2026-06-02-harness-vs-mainstream-gap-analysis.md`（§6 框架决策 + 附录 A metalcraft 深挖 + 附录 B 项目 web 核对 + 附录 C/D 路线）。本设计是其 §6 + 附录 C 的正式化。
> 证据来源：本设计 §1 表中每条均为 **2026-06-02 本会话亲自读真实代码/文件**核对。配套：拓扑/节点/执行/引擎 4 份 `2026-06-0*-harness-*` / `pentagi-engine-*` 参考。日期：2026-06-02。

---

## 0. 决策（TL;DR）

- 选 **方案 B（嵌接 + 借鉴）**——用户 2026-06-02 拍板（A 推倒重写 / C 最小补丁均否）。
- **留**成熟底座 · **搓**你掌控的图/检查点新骨架（vendor metalcraft，不引硬依赖）· **借**各家长处焊到具体接缝。
- 分 4 期：**P0 证据闭环 → P1 图骨架+断点 → P2 信任+质量 → P3 知识+持续**。**P0 先行**（是「gate 真校验」和「checkpoint」的共同前置）。
- 对应 feature `engine-v2-graft-2026-06-02`；P0 实现计划见 `docs/superpowers/plans/2026-06-02-engine-v2-p0-evidence-loop.md`。

---

## 1. 现状勘验（本会话亲自核对真实代码）

> 这张表更正了 gap 文档/旧参考里「evidence ledger 没建 / stage_mode 默认 OFF」等**已过时**的说法（migration `20260601000001` 已落地后口径变了）。

| 能力 | 现状 | 真实落点（已核） | 缺口 |
|---|---|---|---|
| 多 agent | ✅ 有 | `golish-agent-kit/src/task_orchestrator`（`orchestrator.rs::run`/`resume`）+ 13 sub-agent（`golish-sub-agents/.../registry.rs`）+ 嵌套委派 | 无 |
| 上下文压缩 | ✅ 有 | `golish-agent-runtime/src/agentic_loop/compaction.rs` + `compaction_loop.rs` + `turn/phases/token_estimate.rs` | 无 |
| 长/短期记忆 | ✅ 有 | 长期 `golish-agent-kit/src/db_tracking/memory/{store,search,fetch}.rs`（`memories` 表 + pgvector 语义检索）+ `memorist`/`enricher`；短期 per-turn 历史 + compaction | 调优级 |
| 暂停/继续 | ⚠️ 半 | `orchestrator.rs::resume`（从上个完成 subtask 续）+ HITL approval hold→wait→resume（`execute.rs`，`execute_harness_loop_tests.rs` 有测试） | 全量状态、stage 中途断点 |
| 持久化 | ✅ 底子 | 内嵌 PG + sqlx + migrations + repos（`operation_state.rs` / `evidence_classifications.rs`） | `stage_runs` 空表、`state_blob` 未写 |
| Evidence Ledger | ⚠️ 半 | schema `migrations/20260601000001_evidence_ledger.sql` ✅；读路径 `golish-pentest-app/src/evidence.rs::evidence_read`（sanitize+三态 freshness+scope_label+IDOR）✅；域类型+`ScopeService` trait+`InMemoryScopeService`（`golish-pentest/src/evidence_ledger/`）✅；分类层 ✅ | **写入 `append()` ❌**（`mod.rs` 注/类型，Phase 1b 推迟）+ **gate 回查 ❌** |
| Gate | ✅ 有 | `harness/gate/mod.rs::validate_stage_gate`（4 结构 + 3 语义 check）；`freshness_check.rs` 已含 `run_with_freshness()` 真 max_age | mod 当前调 `run()` sanity 版，**未喂 ledger** |
| 分支路由 | ⚠️ | `harness/stage_transition.rs::decide_transition` 能判 `Branch`，但运行时 `advance_target()` 取 `candidates.first()` | 条件选分支 / 早退边 |
| stage_mode flag | ✅ 默认 **ON** | `task_orchestrator/subtask_phases/execute.rs:477`（显式 =false 才关） | — |

> **核心洞察**：5 项能力里 **4.5 项已现成且成熟**；唯一真缺口 = **全量检查点续跑 + evidence 写入闭环**。所以 B 是「补缺口」而非「造引擎」。

---

## 2. 架构：留 / 搓 / 借

### 2.1 留（成熟底座，不动主干）

`task_orchestrator`（Generator→primary→subtask→sub-agents）+ `agentic_loop`（per-turn 状态机 / compaction / loop 检测 / extended thinking）+ 13 sub-agent 嵌套委派 + 工具层（`tool_definitions.rs` / `tool_policy.rs`）+ 记忆（`db_tracking/memory` + pgvector）。**这些是 PentAGI 血统的「干活的 AI」，已验证成熟，零改动。**

### 2.2 搓（你掌控的新骨架 = `golish-agent-kit/src/harness/graph_engine/`）

vendor metalcraft 范式，**照抄进内部模块、不引为外部依赖**（bus-factor 1 / v0.3 风险，见 gap 附录 A.4）。单元职责：

| 单元 | 职责 | 接口（草案） |
|---|---|---|
| `StateGraph<S>` | 可执行状态图（over `StageKind` 节点 + 条件边），替「JSON 投影 + cursor」 | `add_node` / `add_conditional` / `compile` |
| `Checkpointer` trait + `PgCheckpointer` | 全量状态写 `operation_state.state_blob` + `stage_runs` 行 | `save(thread_id, snapshot)` / `load(thread_id)` |
| `RunOutcome` | `Completed{state}` / `Interrupted{reason,state}` / `Failed{state,node,error}`（**失败保状态**） | enum |
| `Executor` | 跑图 + 每步后 checkpoint + step guard（loop/error-spiral） | `run(state, thread_id)` |
| 条件分支 | 替 `advance_target()` 的 `first()`，按阶段结果/规则选边 | `decide_transition` 升级 |

> 接口直接照搬 metalcraft 的 `Graph/Executor/Checkpointer/RunOutcome`（gap 附录 A.2/A.3），用 Golish 的 `StageKind` + `operation_graph.json` 适配。**vendor 前必须真 clone 读 `executor.rs`/`checkpoint.rs` 复验附录 A 行级断言（`FuturesUnordered`+`sort_by` 确定性合并、`RunOutcome::Failed` 保状态）。**

### 2.3 借（各家长处 → 接缝映射）

| 从哪借 | 借什么 | 焊到哪 | 哪期 |
|---|---|---|---|
| **OpenFang**（`RightNow-AI/openfang`）| Merkle 哈希链审计 + 16 安全层（capability gate / taint / SSRF / prompt-injection）+ subprocess sandbox | evidence `append()` 加 prev_hash/hash；`tool_policy.rs` 加安全层 | P0（哈希链）/ P2（安全层）|
| **metalcraft**（`rust4ai/metalcraft`）| 图执行器 + Checkpointer trait + interrupt/resume + 确定性并行合并 | `harness/graph_engine/`（新）| P1 |
| **Heartbit**（`heartbit-ai/heartbit`）| AgentRunner/Orchestrator 分离 + 4-hook guardrails + 内置 eval 框架 | eval 新模块；guardrail hook | P2 |
| **AutoAgents**（`liquidos-ai/autoagents`）| `#[tool]` 派生宏 + guardrail 三态（Block/Sanitize/Audit）| `tool_definitions.rs` 工效；`tool_policy.rs` | P2 |
| **XBOW**（概念）| 每个 finding 真实利用验证 | `stages/verification.json` + gate `exploit_proof` check | P2 |
| **Pentest Agent Suite** | RAG writeup 先验检索 + `--execute` 破坏性门控 | `vuln_triage` 前置 check；破坏性工具门控 | P3 |
| **PentAGI** | 沙箱隔离 + Graphiti 知识图 | sandbox runner；knowledge graph 深化 | P3 |
| **GraphBit / LangGraph** | 理念背书 + 概念模型（StateGraph+checkpointer+interrupt+subgraph）| 验证方向（已对齐）| — |

---

## 3. 分期路线图

每期 = 一个 spec→plan→实现 周期（AGENTS.md §1.3），同一时间一个 `in_progress`。

| 期 | 目标 | 关键落点 | 验收（done 标准）|
|---|---|---|---|
| **P0 证据闭环**（先做）| 工具产出自动入账 + gate 回查 | `tool_dispatch` 后置 hook + `EvidenceLedger::append()`（OpenFang 哈希链）+ `gate/*.rs` 读 ledger | 跑 eas 阶段→DB 有 `audit_role='evidence'` 行；自报假 `evidence_refs` 被 BLOCK |
| **P1 图骨架+断点** | vendor metalcraft 全量检查点 + 续跑 + 条件分支 | `harness/graph_engine/`（新）+ `stage_transition.rs` + `stage_runs` repo | 杀进程→原位恢复；没攻击面能 bail→reporting |
| **P2 信任+质量** | 真实利用验证 gate + eval 框架 + 护栏 | `verification.json`+gate(XBOW) + eval(Heartbit) + guardrail(AutoAgents/OpenFang) | 无 PoC 的 critical 过不了；eval 能判 doer |
| **P3 知识+持续** | RAG 先验 + 知识图 + 持续运行 | `vuln_triage` 前置 check + Graphiti(PentAGI) | 测漏洞前自动检索 writeup 并入交付 |

---

## 4. P0 设计细节（Evidence 写入闭环）

### 4.1 为什么 P0 先行

证据是两件事的共同前置：① **gate 真校验**（contract 真计数 / freshness 真 age / min_invocations 真计数 / scope 真 label，现都「信自报」）；② **checkpoint**（evidence 是可恢复状态的一部分）。P0 一通，gate 从「信自报」升「可交叉验证」，P1/P2/P3 才有真证据可依赖。

### 4.2 数据流

```
工具执行(tool_dispatch 后置 hook)
   → EvidenceLedger::append(kind, subject, raw_output, stage_run_id)
        → 写 audit_log(audit_role='evidence', detail={kind,subject,raw_output}, prev_hash, hash)  // OpenFang 哈希链
        → ScopeService.classify_subject(subject) → 写 evidence_classifications(valid_to=NULL)
        → 返回 EvidenceAuditId
   → deliverable.evidence_refs 收集真实 id
阶段末 gate: validate_stage_gate_with_ledger(deliverable, spec, contract, evidence_index)
   → freshness_check::run_with_freshness(已存在) + contract/min_invocations/scope 读 ledger 真计数
   → 假/缺/过期 evidence → BLOCK + recovery
```

### 4.3 组件（落点 + 现状）

| 组件 | 落点 | 现状 |
|---|---|---|
| `EvidenceLedger::append()` | `golish-pentest/src/evidence_ledger/`（新 struct + DB writer）| ❌ 仅注释/类型 |
| `log_evidence`（DB writer）| `golish-db/src/repo/audit/`（仿 `log_operation`，set `audit_role='evidence'` 返 id）| ❌ 待加 |
| 生产版 `ScopeService` | `golish-pentest`（查 `organizations.scope_rules` JSONB，替 `InMemoryScopeService`）| ❌ 仅 in-memory |
| 哈希链 | evidence 行加 `prev_hash`/`hash`（链上一条 evidence id）| ❌ 待加（borrow OpenFang）|
| tool_dispatch hook | `golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs` 工具执行后调 append | ❌ 待接 |
| `validate_stage_gate_with_ledger` | `harness/gate/mod.rs`（新入口，复用现有 `freshness_check::run_with_freshness`）| ⚠️ run() sanity 版已在用 |

### 4.4 OpenFang 哈希链（tamper-evident）

每条 evidence 行存 `hash = H(prev_hash ‖ canonical(detail) ‖ created_at)`，`prev_hash` = 同 operation 上一条 evidence 的 `hash`。gate/审计可重算校验链不被悄改。`H` 用 sha256（已在依赖树）。**MVP 存在 `audit_log.detail` JSON 里**（不改表结构，向后兼容；真要列化推 P2）。

### 4.5 P0 验收

1. `cargo nextest run -p golish-pentest -E 'test(evidence_ledger)'` 含 append + 哈希链单测全绿。
2. `cargo nextest run -p golish-agent-kit -E 'test(harness::gate)'` gate-with-ledger 新测全绿（假 refs 被 BLOCK）。
3. 活体（用户）：`GOLISH_HARNESS_STAGE_MODE=true just dev` 跑 eas → DB `SELECT count(*) FROM audit_log WHERE audit_role='evidence'` > 0 + `evidence_read` 能读回 + 哈希链可重算。
4. `just precommit` 全绿。

---

## 5. 不变量与风险

**不变量（AGENTS.md §5，必须守）**：I7 安全交付有 evidence（本设计正是强化它）；I2 IDOR（evidence_read 已做，append 同样验 operation 归属）；I5 ts-rs（新增跨 IPC 类型走 `#[derive(ts_rs::TS)]`）；I1 错误码 `code` 字段；I9 事务内不调外部 HTTP（append 写 DB 与工具执行解耦）。

**风险**：
- P1 在现有引擎动骨架——`stage_mode` flag 兜底（默认 ON 但可 =false 回退旧路径），分期 + 每期单测 gate。
- metalcraft 行级未复验——**vendor 前必读 `executor.rs`/`checkpoint.rs`**（gap 附录 A.5 已标）。
- 哈希链 MVP 存 JSON——可接受，列化推后。

---

## 6. 决策记录

- **2026-06-02**：用户在 gap 文档 §6 + 附录 C 基础上，明确选 **B（嵌接+借鉴）**。否 A（推倒重写=扔成熟资产、数月、高风险）、否 C（最小补丁=拿不到统一引擎掌控感）。
- **为什么 vendor metalcraft 不引依赖**：MIT + 小(~2k 行) + 同栈(rig) + 写得好，但 bus-factor 1 / v0.3 / 0 生产验证 → 照抄进掌控的内部模块（gap §6 + 附录 A.5）。
- **为什么 P0 打头**：证据是 gate 真校验 + checkpoint 的共同前置（gap §5 P0）。
