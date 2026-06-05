# Unified AI + Harness Observability — P1 Implementation Plan

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。每个任务独立 commit，按 TDD（先红测后实现）。

**目标：** 让一个 AI 仅凭 `operation_id` 就能自助还原一次 task 运行的完整跨 agent 决策时间线（agent + sub-agent 串成一条），无需用户指路。
**架构：** 在现有 `AiEvent` + JSONL transcript 基础上，新增 ①一条关联主线（`operation_id` + `agent_path`）②harness 决策一等事件 `HarnessTrace` ③按 operation 归档的合并时间线 + manifest ④自助检索工具 `harness_trace` 与 `just replay`。
**技术栈：** Rust workspace（`golish-core` / `golish-events` / `golish-agent-kit` / `golish-agent-app` / `golish-sub-agents` / `golish` CLI）+ ts-rs 生成前端类型。
**设计文档：** [`docs/design/2026-06-05-unified-ai-harness-observability.md`](../../design/2026-06-05-unified-ai-harness-observability.md)（先读 §4 组件 + §5 决策表）。

---

## 范围说明（仅 P1）

本计划只实现设计文档 §8 的 **P1**：组件 A/B/C(b)/D/E。P2（单一 choke-point 重构、evidence operation_id 完整核验、`list_evidence`）与 P3（DB substrate / replay / diff / metrics / UI）**不在本计划**，留作后续计划。

**执行前置（必须先确认的设计决策，对应设计 §5）**：D1=operation_id 为主键、D2=单 `HarnessTrace{kind}` 变体、D3=JSONL 优先、D4=P1 用 post-hoc 合并、D5=id 放 record wrapper 不放 51 个变体、D6=本计划仅做「读侧用 operation_id」，evidence 写侧 `set_task_context` 的改动**移到 P2**（因涉及 evidence hash-chain，风险高，需单独 gate）。若用户对任一决策有异议，先改设计再执行。

---

## 文件结构（P1 将创建/修改）

**新建**
- `backend/crates/golish-core/src/events/harness_trace.rs` — `HarnessTraceKind` 枚举。
- `backend/crates/golish-events/src/op_trace/mod.rs` — `OperationManifest`、`TraceRecord`、合并写入器 `OpTraceWriter`、`op_trace_dir()`。
- `backend/crates/golish-events/src/op_trace/merge.rs` — 把 main `transcript.json` + `subagents/*/transcript.json` 按时间合并成 `timeline.jsonl` 的 post-hoc 合并器。
- `backend/crates/golish-agent-app/src/ai/harness_trace_tool.rs` — `harness_trace` agent 工具。
- `backend/crates/golish/src/cli/commands/replay.rs`（或就近 CLI 模块）— `just replay` 后端。

**修改**
- `backend/crates/golish-core/src/events/event.rs` — `AiEvent::HarnessTrace` 单变体 + `mod harness_trace`。
- `backend/crates/golish-core/src/events/event_dispatch.rs` — `event_type()` 加一臂。
- `backend/crates/golish-core/src/events/mod.rs` — `pub use harness_trace::*`。
- `backend/crates/golish-events/src/transcript/tests/should_transcript_tests.rs` — 断言 `HarnessTrace` 入档。
- `backend/crates/golish-events/src/lib.rs` — `pub mod op_trace`。
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` — `consume_gate_outcome` + `enforce_evidence_existence` 发 `HarnessTrace`。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs` — evidence sync append 发 `EvidenceBooked`。
- `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs` — background listener 发 `EvidenceBooked`；注册 `harness_trace` 工具。
- `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs` — 返回前发 `DeliverableSubmitted`。
- `backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs` — note 注入发 `BackgroundNotesInjected`。
- `backend/crates/golish-sub-agents/src/definition/mod.rs` — `SubAgentContext` 加 `agent_path()` 派生。
- `justfile` — `replay` recipe。
- `docs/development.md` — 文档 `harness=debug` profile + op-dir 布局。
- `frontend/lib/generated/` — ts-rs 重新生成（由 `just check` 驱动，勿手改）。
- `frontend/components/...` 任何对 `GeneratedAiEvent` 做 exhaustive switch 的地方 — 加 `harness_trace` 分支（Task 2 末查找）。

---

## Phase P1.0 — 新事件类型（无副作用，先打地基）

### 任务 1：定义 `HarnessTraceKind`（先红测）

**文件：** 新建 `backend/crates/golish-core/src/events/harness_trace.rs`

**步骤 1（写失败测试）：** 在文件底部加序列化测试，先不写类型 → 编译失败（红）。

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_decision_serializes_with_kind_tag() {
        let k = HarnessTraceKind::GateDecision {
            gate: "BLOCK".into(),
            findings: 0,
            fabricated_evidence_refs: vec![1, 2, 3],
            available_real_ids: vec![86, 88, 90],
            first_blocking_reason: Some("fabricated evidence ids".into()),
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "gate_decision");
        assert_eq!(v["gate"], "BLOCK");
        assert_eq!(v["fabricated_evidence_refs"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn evidence_booked_roundtrips() {
        let k = HarnessTraceKind::EvidenceBooked {
            tool: "run_pty_cmd".into(),
            evidence_id: 88,
            source: "background".into(),
        };
        let back: HarnessTraceKind =
            serde_json::from_value(serde_json::to_value(&k).unwrap()).unwrap();
        assert!(matches!(back, HarnessTraceKind::EvidenceBooked { evidence_id: 88, .. }));
    }
}
```

**步骤 2（实现到绿）：** 在文件顶部写类型（与设计 §4.B 一致）：

```rust
//! Harness decision sub-events carried by `AiEvent::HarnessTrace`.
//!
//! One variant per harness decision *kind*; adding a kind extends this enum
//! only (no new `AiEvent` arm, no exhaustive-match churn across the codebase).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum HarnessTraceKind {
    /// Stage-close gate decision. Emitted for both PASS and BLOCK at the single
    /// chokepoint in `consume_gate_outcome`.
    GateDecision {
        gate: String, // "PASS" | "BLOCK"
        #[ts(type = "number")]
        findings: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(type = "number[]")]
        fabricated_evidence_refs: Vec<i64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(type = "number[]")]
        available_real_ids: Vec<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first_blocking_reason: Option<String>,
    },
    /// An evidence row was appended to the ledger.
    EvidenceBooked {
        tool: String,
        #[ts(type = "number")]
        evidence_id: i64,
        source: String, // "sync" | "background"
    },
    /// `submit_stage_deliverable` produced an outcome.
    DeliverableSubmitted {
        status: String, // "accepted" | "needs_fix" | "rejected"
        #[serde(default)]
        #[ts(type = "number[]")]
        cited_evidence_refs: Vec<i64>,
        #[serde(default)]
        #[ts(type = "number[]")]
        available_real_ids: Vec<i64>,
    },
    /// Background-job completion notes were drained into the next turn's prompt.
    BackgroundNotesInjected {
        #[ts(type = "number")]
        count: u32,
        #[ts(type = "number[]")]
        evidence_ids: Vec<i64>,
    },
}
```

**步骤 3：** 在 `backend/crates/golish-core/src/events/mod.rs` 加 `pub mod harness_trace;` 与 `pub use harness_trace::HarnessTraceKind;`（按该文件现有 `pub use` 风格）。

**验证：**
```bash
cd backend && cargo nextest run -p golish-core -E 'test(harness_trace)' --status-level fail
# 预期：2 passed
```

**提交：** `feat(core): add HarnessTraceKind harness-decision sub-events`

---

### 任务 2：加 `AiEvent::HarnessTrace` 单变体 + event_type

**文件：** `backend/crates/golish-core/src/events/event.rs`、`event_dispatch.rs`

**步骤 1：** `event.rs` 顶部 `use` 区加 `use super::harness_trace::HarnessTraceKind;`。

**步骤 2：** 在 `AiEvent` 的 task-mode 段之后（`ToolBackgroundCompleted` 之后、enum 结束 `}` 之前，约 `event.rs:539`）加单变体：

```rust
    /// First-class harness decision record (gate / evidence / submit / notes).
    /// The correlation spine: `operation_id` + `agent_path` let an AI thread
    /// the main agent and every sub-agent into one timeline. See
    /// design 2026-06-05-unified-ai-harness-observability §4.B.
    HarnessTrace {
        operation_id: String,
        stage: String,
        #[serde(default)]
        agent_path: String,
        #[serde(flatten)]
        trace: HarnessTraceKind,
    },
```

**步骤 3：** `event_dispatch.rs` 的 `event_type()` match 加一臂（紧邻其它臂，返回稳定名）：

```rust
            AiEvent::HarnessTrace { .. } => "harness_trace",
```

> 注意：`event_dispatch.rs:12` 顶部注释写「46-arm」/「51-arm」已是 stale，可顺手把数字改对（非必须）。

**步骤 4（找前端 exhaustive switch）：** 运行查找，确认是否有对 `GeneratedAiEvent.type` 做穷尽 switch 的前端处理器需要加分支：
```bash
rg -n "case \"task_progress\"|case \"tool_background_completed\"" frontend/
```
若命中 handler（如 `frontend/services/ai-events/*`），加 `case "harness_trace":`（P1 可先 no-op 处理，仅防 TS 穷尽报错）。

**验证：**
```bash
cd backend && cargo check -p golish-core
cd backend && cargo nextest run -p golish-core --status-level fail   # 无回归
just check-types   # ts-rs 重新生成 GeneratedAiEvent / HarnessTraceKind 到 frontend/lib/generated
pnpm typecheck     # 前端类型不漂移（如有穷尽 switch，已加分支）
```

**提交：** `feat(core): add AiEvent::HarnessTrace variant + ts-rs binding`

---

### 任务 3：`should_transcript` 保留 `HarnessTrace`（断言而非改逻辑）

**文件：** `backend/crates/golish-events/src/transcript/tests/should_transcript_tests.rs`

**说明：** `should_transcript`（`transcript/mod.rs:23-34`）是「黑名单」语义——不在排除列表里的一律入档。`HarnessTrace` 不是流式，**无需改逻辑**；只加一条断言锁定行为，防回归。

**步骤：** 在测试文件加：

```rust
#[test]
fn harness_trace_is_transcripted() {
    use golish_core::events::harness_trace::HarnessTraceKind;
    let ev = AiEvent::HarnessTrace {
        operation_id: "op-1".into(),
        stage: "target_intel".into(),
        agent_path: "main>pentester".into(),
        trace: HarnessTraceKind::GateDecision {
            gate: "BLOCK".into(),
            findings: 0,
            fabricated_evidence_refs: vec![1],
            available_real_ids: vec![86],
            first_blocking_reason: None,
        },
    };
    assert!(should_transcript(&ev));
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-events -E 'test(should_transcript)' --status-level fail
```

**提交：** `test(events): assert HarnessTrace is persisted to transcript`

---

## Phase P1.1 — agent_path 派生（sub-agent 串联的关键）

### 任务 4：`SubAgentContext::agent_path()`（先红测）

**文件：** `backend/crates/golish-sub-agents/src/definition/mod.rs`（`SubAgentContext` 在 `:29-58`，含 `parent_agent` / `depth` / `agent_id` 等字段——执行前先 Read 确认确切字段名）。

**步骤 1（红测）：** 加 `#[cfg(test)]` 测试，断言：
- 顶层（无 parent）→ `"main"`；
- parent_agent=`"main"` + 当前名 `"pentester"` → `"main>pentester"`；
- 嵌套 parent_agent=`"main>pentester"` + 当前 `"reporter"` → `"main>pentester>reporter"`。

**步骤 2（实现）：** 加方法（字段名以实际为准）：

```rust
impl SubAgentContext {
    /// `>`-joined lineage, e.g. `main>pentester>reporter`. Falls back to
    /// `agent_id` when a human name is absent. The top-level agent is `main`.
    pub fn agent_path(&self) -> String {
        let name = if self.agent_name.is_empty() { &self.agent_id } else { &self.agent_name };
        match self.parent_agent.as_deref() {
            None | Some("") => name.clone(),
            Some(parent) if parent == "main" || parent.contains('>') => format!("{parent}>{name}"),
            Some(parent) => format!("main>{parent}>{name}"),
        }
    }
}
```

> 若 `SubAgentContext` 实际无 `agent_name` 字段，则用 `agent_id`；执行者读 `:29-58` 后定字段。顶层主 agent 的 `agent_path` = `"main"` 由调用方（非 SubAgentContext）提供常量。

**验证：**
```bash
cd backend && cargo nextest run -p golish-sub-agents -E 'test(agent_path)' --status-level fail
```

**提交：** `feat(sub-agents): derive agent_path lineage from SubAgentContext`

---

## Phase P1.2 — 在既有 chokepoint 发 HarnessTrace（additive，零行为变更）

> 每个任务都是「在已存在的 `tracing` 行旁边」加一次 `emit`，不改控制流。先确认该处能拿到 `operation_id`（= `task_id`/`task.id`）；主 agent 路径 `agent_path` 传 `"main"`，sub-agent 路径传 `ctx.agent_path()`。

### 任务 5：gate 决策（PASS+BLOCK）

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`，`consume_gate_outcome`（`:360-401`）。

**步骤：** 在现有 `tracing::info!(target:"harness::hook", … "gate decision")`（`:365-372`）之后、`if outcome.gate_allowed` 之前，加：

```rust
        self.emit(AiEvent::HarnessTrace {
            operation_id: task_id.to_string(),
            stage: outcome.gated_stage.as_str().to_string(),
            agent_path: "main".to_string(),
            trace: HarnessTraceKind::GateDecision {
                gate: if outcome.gate_allowed { "PASS" } else { "BLOCK" }.to_string(),
                findings: outcome.findings_count as u32,
                fabricated_evidence_refs: Vec::new(), // 填充见任务 6
                available_real_ids: Vec::new(),
                first_blocking_reason: outcome.first_block_reason.clone(), // 字段名以 HarnessGateOutcome 实际为准
            },
        });
```

> 保留既有 `TaskProgress{stage_passed}`（前端里程碑仍依赖它，见 `execute.rs:378-388` 注释）。`HarnessTrace` 是新增的、更结构化的并行信号。`self.emit` / `HarnessTraceKind` 需在文件顶部 `use`。

**红测：** 若 `consume_gate_outcome` 已有/可加测试桩（捕获 emit 的 mock event sink），断言一次 BLOCK 产生一条 `HarnessTrace::GateDecision{gate:"BLOCK"}`。若该结构难以单测，则在任务 6 的合并器层做集成断言，并在本任务说明「行为由集成测试覆盖」。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(consume_gate) or test(execute_harness_loop)' --status-level fail
cd backend && cargo clippy -p golish-agent-kit --all-targets -- -D warnings
```

**提交：** `feat(harness): emit HarnessTrace gate decision at consume_gate_outcome`

---

### 任务 6：BLOCK 的 fabricated / available_real_ids

**文件：** 同 `execute.rs`，`enforce_evidence_existence` 的 BLOCK 分支（现有 `tracing::warn!("gate BLOCK: deliverable cites evidence ids absent…")` 在 `:931-936`，已带 `fabricated` + `available_real_ids` 两个本地变量）。

**步骤：** 让该路径把 `fabricated` 与 `available_real_ids` 透传到任务 5 的 `GateDecision`。两种接法（执行者按 `HarnessGateOutcome` 实际结构择一）：
- (a) 在 `HarnessGateOutcome` 加两个字段 `fabricated_evidence_refs: Vec<i64>` / `available_real_ids: Vec<i64>`，由 `enforce_evidence_existence` 填，`consume_gate_outcome` 读出塞进 `GateDecision`（推荐，单一发射点）。
- (b) 在 `:931-936` 处直接 `self.emit(HarnessTrace::GateDecision{…})`（两个发射点，需去重）。

推荐 (a)。

**红测：** 单测 `enforce_evidence_existence`（已有同名测试，见 agent-progress 2026-06-04 记录）扩断言：BLOCK 时 outcome 携带 `fabricated=[1]` / `available_real_ids=[86,88,90]`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(enforce_evidence) or test(block_outcome)' --status-level fail
```

**提交：** `feat(harness): thread fabricated/real evidence ids into gate HarnessTrace`

---

### 任务 7：EvidenceBooked（sync + background）

**文件：**
- sync：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`（现有 `tracing::info!(target:"harness::evidence", … "evidence appended…")` 在 `:291-296`、`:386-391`）。
- background：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`（`:200-205`）。

**步骤：** 在每处既有 `info!` 旁 `emit` 一条：

```rust
// sync (direct/mod.rs)，operation_id 取当前 tool 上下文的 task/op id，agent_path 取当前 agent
emit_fn(AiEvent::HarnessTrace {
    operation_id: op_id.to_string(),
    stage: stage.clone(),               // 该 tool 调用所属 stage（若可得；否则 "")
    agent_path: agent_path.clone(),     // 主 agent="main"，sub-agent=ctx.agent_path()
    trace: HarnessTraceKind::EvidenceBooked { tool: tool_name.clone(), evidence_id, source: "sync".into() },
});
```

```rust
// background (bridge_config.rs listener)
emit_fn(AiEvent::HarnessTrace {
    operation_id: op_id.to_string(),
    stage: String::new(),               // 后台 job 未必知 stage；留空，由 timeline 时序定位
    agent_path: "main".into(),          // 后台监听挂在主 bridge
    trace: HarnessTraceKind::EvidenceBooked { tool: tool.clone(), evidence_id, source: "background".into() },
});
```

> `direct/mod.rs` 的事件发射通道：先确认该层用的是 `ctx.emit_event` / `event_tx`（见 `agentic_loop/context.rs:230-272`），按既有发射方式调用。`bridge_config.rs` listener 已经在发 `ToolBackgroundCompleted`（`:108-116`），照同一通道加发本事件。

**红测：** 给 listener 的 `maybe_append_background_evidence` 路径加单测，断言成功 append 后发了一条 `EvidenceBooked{source:"background"}`（可用已有 listener 测试的 mock）。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-runtime -p golish-agent-app -E 'test(evidence)' --status-level fail
cd backend && cargo clippy -p golish-agent-runtime -p golish-agent-app --all-targets -- -D warnings
```

**提交：** `feat(harness): emit EvidenceBooked HarnessTrace on ledger append`

---

### 任务 8：DeliverableSubmitted

**文件：** `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`（返回 `Value` 处 `:204-293`，已含 `status` / `fabricated_evidence_refs` / `available_evidence_ids` / 入参 `evidence_refs`）。

**步骤：** 在构造返回 `Value` 后、返回前，`emit` 一条（工具已有 `session_id` 字段——任务 by `with_session_id`，见 agent-progress 2026-06-04「乙」；`operation_id` 若工具层只有 session，可先用 session 串，P2 再统一为 op id）：

```rust
emit_fn(AiEvent::HarnessTrace {
    operation_id: self.operation_id_or_session(), // 工具层可得的最稳 id；优先 op id，退化 session 串
    stage: stage.clone(),
    agent_path: agent_path.clone(),               // 提交方可能是 reporter 子 agent → ctx.agent_path()
    trace: HarnessTraceKind::DeliverableSubmitted {
        status: status.clone(),
        cited_evidence_refs: cited.clone(),       // = 入参 evidence_refs
        available_real_ids: available.clone(),    // = available_evidence_ids（needs_fix 分支已算）
    },
});
```

> 关键价值：这条 + 任务 5/6 的 `GateDecision` 在 `timeline.jsonl` 里前后相邻，一眼看出「cited=[1,2,3] 但 available=[86,88,90] → BLOCK fabricated」。工具的事件发射通道需确认（submit tool 是否持有 event sink；若无，则把该事件经 tool 返回值带回上层由 `consume`/single_tool_call 发射）。

**红测：** 扩 `fabricated_needs_fix_lists_available_real_ids` 测试，断言同时发了 `DeliverableSubmitted{status:"needs_fix", cited:[…], available:[…]}`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(submit) or test(deliverable)' --status-level fail
```

**提交：** `feat(harness): emit DeliverableSubmitted HarnessTrace from submit tool`

---

### 任务 9：BackgroundNotesInjected

**文件：** `backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs`（`append_background_notes`，`:43-67`，现有 `tracing::info!("[agent] Injecting {} background-job completion note(s)")` 在 `:55-57`）。

**步骤：** 在该 `info!` 旁 emit：

```rust
self.emit_event(AiEvent::HarnessTrace {
    operation_id: self.current_operation_id(),  // bridge 侧可得的 op/session id
    stage: String::new(),
    agent_path: "main".into(),
    trace: HarnessTraceKind::BackgroundNotesInjected {
        count: notes.len() as u32,
        evidence_ids,   // 从 note 文本/队列里解析出的 evidence id（format_background_note 已带 evidence_id=）
    },
});
```

> `evidence_ids` 来源：`pending_background` 队列项里已有 evidence id（`bridge_config.rs:221-240` 的 `format_background_note` 带 `evidence_id={id}`）。若队列项未结构化保存 id，则在入队时（任务 7 background 分支同处）顺带把 id 存进队列项结构，这里读出。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-bridge -E 'test(background_note) or test(prepare)' --status-level fail
cd backend && cargo clippy -p golish-agent-bridge --all-targets -- -D warnings
```

**提交：** `feat(harness): emit BackgroundNotesInjected HarnessTrace on drain`

---

## Phase P1.3 — operation 归档：manifest + 合并 timeline

### 任务 10：`op_trace` 模块骨架 + 类型（先红测）

**文件：** 新建 `backend/crates/golish-events/src/op_trace/mod.rs`；`lib.rs` 加 `pub mod op_trace;`。

**步骤 1（类型）：**

```rust
//! Operation-scoped, self-discoverable trace: one directory per `operation_id`
//! containing `manifest.json` (entry point) + `timeline.jsonl` (merged main +
//! sub-agent + harness records). See design 2026-06-05 §4.C.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OperationManifest {
    pub operation_id: String,
    pub chat_session: String,
    pub title: String,
    pub status: String,                 // "running" | "blocked" | "passed" | "failed" | "waiting"
    pub current_stage: Option<String>,
    pub stages: Vec<String>,
    pub agent_paths: Vec<String>,
    pub last_decision: Option<serde_json::Value>, // a HarnessTraceKind value, or null
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRecord {
    pub ts: String,
    pub seq: u64,
    pub operation_id: String,
    pub agent_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub event: golish_core::events::AiEvent,
}

/// `{base}/op-<operation_id>/`
pub fn op_trace_dir(base: &Path, operation_id: &str) -> PathBuf {
    base.join(format!("op-{operation_id}"))
}
```

**步骤 2（红测）：** 断言 `op_trace_dir(Path::new("/t"), "abc")` == `/t/op-abc`；`OperationManifest` round-trips。

**验证：**
```bash
cd backend && cargo nextest run -p golish-events -E 'test(op_trace)' --status-level fail
just check-types   # 生成 OperationManifest 前端类型
```

**提交：** `feat(events): op_trace module — OperationManifest + TraceRecord + dir layout`

---

### 任务 11：post-hoc 合并器（main + subagents → timeline.jsonl）

**文件：** 新建 `backend/crates/golish-events/src/op_trace/merge.rs`。

**步骤（实现 + 红测）：** 写 `merge_session_into_timeline(base, chat_session, operation_id) -> Result<PathBuf>`：
1. 读 main `transcript_path(base, chat_session)`（复用 `read_transcript`）。
2. Glob `{base}/{chat_session}/subagents/*/transcript.json`，每个用 `read_transcript`-风格解析；从目录名 `{agent_id}-{parent_request_id}` 推 `agent_path`（P1 可先用 `agent_id`；agent_path 精确化依赖任务 4 在事件里带 path——见下注）。
3. 所有条目转 `TraceRecord`（main 的 `agent_path="main"`），按 `ts` 归并排序，分配 `seq`，逐行写 `op_trace_dir(base, operation_id)/timeline.jsonl`。

> agent_path 精确来源：最干净是事件本身带 `agent_path`（sub-agent 发事件时用任务 4 的 `ctx.agent_path()`）。P1 合并器对**没带 path 的旧条目**用目录名兜底。

**红测：** 造一个临时 base：写一个 main transcript（2 条）+ 一个 subagents 文件（1 条），调合并器，断言 `timeline.jsonl` 有 3 行、按时间序、main 行 `agent_path=="main"`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-events -E 'test(merge)' --status-level fail
```

**提交：** `feat(events): merge main + sub-agent transcripts into op timeline.jsonl`

---

### 任务 12：manifest 写入（原子）+ 在运行收尾/gate 时刷新

**文件：** `op_trace/mod.rs` 加 `write_manifest_atomic(base, &OperationManifest)`（temp 文件 + `rename`）；`build_manifest_from_timeline(base, operation_id, chat_session, title, status)` 扫 `timeline.jsonl` 汇总 `stages` / `agent_paths` / `last_decision`（取最后一条 `HarnessTrace`）。

**接线点（择一，低耦合优先）：** 在 `consume_gate_outcome`（每次 gate 决策后）或运行收尾处，调 `merge_session_into_timeline` + `build_manifest_from_timeline` + `write_manifest_atomic`。P1 可放在**运行收尾 + 每次 gate BLOCK**（卡住时最需要），避免每条事件都重算。

**红测：** 给 `build_manifest_from_timeline` 喂一个含 1 条 `GateDecision{BLOCK,fabricated:[1],available:[86]}` 的 timeline，断言 `manifest.status` 可设 `"blocked"`、`last_decision.kind=="gate_decision"`、`agent_paths` 去重含 `"main"`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-events -E 'test(manifest)' --status-level fail
cd backend && cargo clippy -p golish-events --all-targets -- -D warnings
```

**提交：** `feat(events): atomic op manifest + refresh on gate/finish`

---

## Phase P1.4 — 自助检索

### 任务 13：`harness_trace` agent 工具

**文件：** 新建 `backend/crates/golish-agent-app/src/ai/harness_trace_tool.rs`；注册参照 `submit_stage_deliverable` 的注册处 `bridge_config.rs:421-435`。

**步骤：**
- 工具入参：`{ operation_id?: string, last_n?: number=50, kinds?: string[] }`。
- 默认 `operation_id` = 当前运行 op（工具构造时由 `bridge_config` 注入 `with_operation_id(...)`，同「乙」`with_session_id` 模式）。
- 实现：定位 `op_trace_dir(base, operation_id)/timeline.jsonl`（不存在则先 `merge_session_into_timeline`），读末 `last_n` 条，按 `kinds` 过滤（默认只留 `harness_trace` + `tool_result` + `subtask_*`，**剔除** text/reasoning），返回紧凑 JSON 数组（每项 `{ts,agent_path,stage,event_summary}`）。
- 工具描述（给模型看）要写清：「卡住/要复盘时调我，我返回本次运行的跨 agent 决策时间线」。

**红测：** 单测工具 handler：临时 base 写一个 timeline → 调 handler（`last_n=10`, `kinds=["gate_decision"]`）→ 断言只返回 gate_decision 条目、含 `agent_path`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(harness_trace_tool)' --status-level fail
cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings
```

**提交：** `feat(agent): add harness_trace self-service introspection tool`

---

### 任务 14：`just replay <operation_id|session>`

**文件：** CLI 子命令（就近现有 CLI 结构，见 `backend/crates/golish/src/cli/`）+ `justfile` 加 recipe。

**步骤：**
- CLI：`golish replay <id>`：把 `<id>` 当 operation_id；找不到 op 目录则当 chat session，按 `sessions.chat_session_key`/索引解析出最近 operation_id（P1 可先要求传 operation_id，session 解析放注释 TODO/P2）。打印 `manifest.json` 摘要 + `timeline.jsonl`（人读格式：`[ts] agent_path | kind | 摘要`），`--with-backend-log` 时追加 `rg op-<id> ~/.golish/backend.log`。
- `justfile`：
```just
# Replay a harness operation's merged decision timeline (AI-debuggable)
replay id:
    cd backend && cargo run -q -p golish --bin golish -- replay {{id}}
```

**红测：** CLI 渲染函数（manifest+timeline → string）单测：喂结构断言输出含 `gate_decision BLOCK` 与 `main>pentester`。

**验证：**
```bash
cd backend && cargo nextest run -p golish -E 'test(replay)' --status-level fail
just replay op-<某真实运行 id>   # 手动冒烟（需先有一次运行产出 op 目录）
```

**提交：** `feat(cli): just replay <op|session> prints merged decision timeline`

---

## Phase P1.5 — 文档与收口

### 任务 15：文档 `docs/development.md`

**步骤：** 加一节「Debugging a harness run」：
- op 目录布局 `.golish/transcripts/op-<operation_id>/{manifest.json,timeline.jsonl}`。
- 自助三步：① `harness_trace()` 工具 / 读 `manifest.json` → ② `just replay <op>` → ③ 需要细节再 `golish=info,harness=debug` + `rg op-<id> ~/.golish/backend.log`。
- 记录日志 profile：`RUST_LOG="golish=info,harness=debug"` 或 settings `advanced.log_level`，「决策可见、token TRACE 关闭」。

**验证：** `rg -n "harness=debug" docs/development.md`（存在）。

**提交：** `docs(dev): document self-discoverable harness trace + log profile`

---

### 任务 16：全量收口

**步骤：**
```bash
just precommit   # fmt + check-fe + test-fe + lint-rust + test-rust-all + check-types
```
全绿后，把证据（命令 + 关键输出行）写入 `agent-progress.md`，`feature_list.json` 对应条目转 `passing` 并填 `evidence`。

**手动 E2E（需 `just dev` + LLM key）：** 跑一次 `target_intel`，制造一次 gate BLOCK → 确认：
- `.golish/transcripts/op-<id>/manifest.json` 出现，`last_decision` = gate BLOCK 带 fabricated/available；
- `just replay op-<id>` 一屏看清 `deliverable_submitted` → `gate_decision BLOCK`；
- `harness_trace()` 工具返回同一时间线；
- sub-agent 行带 `main>pentester…` 路径。

**提交：** `chore(harness): P1 observability — precommit green + progress/feature_list`

---

## 自检（writing-plans §自检）

1. **规格覆盖度**：设计 §4 A→任务 5-9+任务4(agent_path)+任务2(operation_id on record)；B→任务1-2+5-9；C→任务10-12；D→任务13-14；E→任务15 + 任务5-9 的 agent_path/operation_id 入 tracing（注：harness::* 行加 operation_id/agent_path 字段在任务5-9 顺带，若某行未带，执行者补）。✅
2. **占位符扫描**：无「TODO/待补」式步骤；少数「字段名以实际为准」处均给了确认指令（先 Read 指定 file:line）。可接受——这是诚实的「执行前确认签名」，非空洞占位。
3. **类型一致性**：`HarnessTraceKind`（任务1）→ `AiEvent::HarnessTrace.trace`（任务2）→ 发射（任务5-9）→ `TraceRecord.event`（任务10）→ 合并/工具/CLI（11-14）全程同名；`operation_id`/`agent_path` 命名贯穿一致。✅
4. **已知执行前需确认项（非阻塞）**：`HarnessGateOutcome` 是否有 `first_block_reason` 字段（任务5/6）、各发射点的 event sink 句柄（任务7-9）、`SubAgentContext` 字段名（任务4）——均在对应任务标注「先 Read 确认」。

> 风险与回滚见设计 §9。所有改动加性：回滚 = 删 emit 调用 + 删新文件，旧 transcript/backend.log 不受影响。
