# Engine v2 · P2 信任+质量（验证 gate + eval + 护栏）实现计划

> **面向 AI 代理的工作者：** 必需子技能：superpowers:executing-plans 逐任务实现。commit/push 按 AGENTS.md §2.7。

**目标：** 把 gate 从「证据存在/新鲜」升到「**结论可信**」——critical/high finding 必须有**真实利用验证**才放行（借 XBOW per-finding 验证理念）；加 eval 能判 doer 输出质量（借 Heartbit 内置 eval）；加 guardrail 拦危险工具 I/O（借 AutoAgents Block/Sanitize/Audit + OpenFang 16 安全层）。

**架构（留-搓-借）：** **留** 现有 gate 框架（`validate_stage_gate` + check 模块 + `required_checks` 命名选跑）。**搓** `verification_check` + eval + guardrail 接缝。**借** XBOW（每 finding 真实利用验证）/ Heartbit（`heartbit-ai/heartbit`，已 clone：AgentRunner/Orchestrator + eval + guardrail hook）/ AutoAgents（`liquidos-ai/autoagents`，已 clone：`EnforcementPolicy::Sanitize` 等）/ OpenFang（`RightNow-AI/openfang`，已 clone：16 安全层）。

**现状（本会话亲核）：** gate 框架 = `gate/mod.rs::validate_stage_gate`（结构 check 恒跑 + 语义 check 按 `spec.required_checks` 选跑，名→id 映射 scope/surface_coverage/min_invocations）。`HarnessFinding{finding_id,kind,subject,severity:FindingSeverity(Info..Critical),evidence_refs:Vec<EvidenceAuditId>}`。`IntentAxis::ExploitValidation` 已存在。P0 已让 evidence 真落库 + 存在性回查（`enforce_evidence_existence`）。

---

## 增量划分（建议按序，P2-a 先做）

### P2-a · 验证 gate（结构层）—— 本计划主交付，最高价值
**新增** `gate/verification_check.rs`：`run(deliverable) -> GateCheckOutcome`。规则：每个 `severity ∈ {High, Critical}` 的 finding **必须**有非空 `evidence_refs`，否则 BLOCK（reason 列出违规 finding_id + recovery.missing_evidence_kinds 提示补利用验证）。
**接线** `gate/mod.rs`：`required_checks` 名映射加 `"findings_verified" => "verification"`，spec.required_checks 含它的 stage（verification/vuln_triage）触发；去重逻辑同现有。
**stage spec**：给 `resources/harness/stages/{verification,vuln_triage}.json` 的 required_checks 加 `"findings_verified"`（确认点：读这些 json 现有 required_checks 数组）。
**测试**：`verification_check` 单测——critical 无 evidence→Block；critical 有 evidence→Pass；info/low 无 evidence→Pass（不强制）；空 findings→Pass。
**验证**：`cargo nextest -p golish-agent-kit -E 'test(verification_check)'` + `-E 'test(harness::gate)'` 全绿。

### P2-b · 验证 gate（ledger 层）—— 强化「真实利用」
caller 侧（execute.rs，仿 P0 `enforce_evidence_existence`）：对 critical/high finding 的 evidence_refs 查 ledger，要求至少一条 evidence 的 `detail->>'kind'` ∈ 验证类集合（`exploit_verified` / `poc` / `vuln_validated`）。无 → BLOCK。需 `DbRepoProvider` 加 `evidence_kinds_for(ids) -> Map<i64,String>`（默认空）。**确认点**：evidence kind 当前 MVP=工具名（P0 Task 5），验证类 kind 需在工具产出阶段打标——本增量先把 gate 准备好，kind 标注随 P2 工具包接入。

### P2-c · eval 框架（借 Heartbit）
**新增** `harness/eval/`：`EvalCase{name, input, expect}` + `EvalOutcome{passed, score, notes}` + 一个 deterministic evaluator（先做 rule-based：gate 通过率 / evidence 计数 / finding 验证率），跑历史 stage_runs 算 doer 质量分。LLM-judge eval 留 P2-c2。**确认点**：读 Heartbit `crates/heartbit/src/**` eval 模块的 AgentRunner/eval 写法（已 clone /tmp/refs/heartbit）。

### P2-d · 护栏（借 AutoAgents + OpenFang）
在 `tool_policy` / `pre_action_authorizer` 加 guardrail 层：`GuardrailAction{Allow, Sanitize(reason), Block(reason), Audit}`——拦 SSRF/prompt-injection/危险 shell（借 OpenFang 16 层的 capability gate / taint / SSRF 写法 + AutoAgents EnforcementPolicy）。**确认点**：读 OpenFang `crates/openfang-runtime/src/**` 安全层 + AutoAgents guardrails（已 clone）。

---

## 类型/依赖一致
- P2-a 纯 deliverable 结构 check，零新依赖、零 DB、零 schema。
- P2-b 复用 P0 的 DbRepoProvider 桥接模式（默认 no-op 不破 mock）。
- P2-c/d 借已 clone 的 Heartbit/AutoAgents/OpenFang，照抄设计不引依赖。

## 自检（writing-plans）
- 规格覆盖设计 §5 P2（验证 gate + eval + guardrail）+ 验收「无 PoC 的 critical 过不了」（P2-a 结构层即满足最小版，P2-b 强化）。
- P2-a 是独立可测最小增量，建议先做；b/c/d 依次叠加。
