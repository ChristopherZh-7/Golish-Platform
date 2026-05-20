# Golish Agent Harness Architecture 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 按 Anthropic 的 `gather context -> take action -> verify work -> repeat` 思路，在 Golish 现有 task orchestrator 旁边补上一层通用 agent harness runtime。  
**架构：** 不以 Recon 为地基。先新增 `harness` 通用模块，承载上下文、受控行动、证据记录、结构化完成、验证反馈和恢复循环；再接入 `TaskOrchestrator::execute_single_subtask` 的执行后流程；最后用一个 thin demo stage 验证 loop。  
**技术栈：** Rust、serde、serde_json、anyhow、现有 `golish-agent-kit`、现有 `tool_policy` / `tool_execution`、现有 `golish-core::events::AiEvent`。

## Problem

上一版计划把 harness 直接绑定到 Recon：`ReconDeliverable`、DNS、端口、服务、HTTP、技术栈、`validate_recon_gate`。这不够稳，因为当前 Recon DAG 还没被验证。

外部资料和 PentAGI 代码都说明 harness 的核心不是某个安全阶段，而是模型外部的运行系统：

- 执行循环：gather context -> take action -> verify -> repeat。
- 工具边界：工具注册、scope、权限、沙箱、approval。
- 状态持久化：长期任务、单次工作轮、事件流、timeline。
- 结构化完成：barrier tool 取代自然语言“完成”。
- 验证和恢复：gate/evaluator 判断是否放行，失败后生成 recovery。

因此本计划改为 harness-first。Recon 只保留为后续 demo 候选，不作为总架构前提。

## Goals

1. 新增通用 `harness` 模块，不引用 Recon 专有字段。
2. 先实现 Anthropic 式工作循环的工程边界：context、action、evidence、verification、recovery。
3. 定义候选 DTO：`StageDeliverable`、`GateDecision`、`EvidenceItem`、`RecoveryAction`。这些名字可以在实现前按 Golish 领域语言重命名。
4. 实现纯函数 gate：先验证 deliverable 结构和 evidence 引用完整性。
5. 在 task-mode 子任务执行后识别并解析 stage deliverable。
6. gate 失败时把 recovery actions 写回结果，供 refiner 调整后续计划。
7. 增加 timeline 事件，使前端和日志能看到 barrier/gate/recovery。
8. 实现一个 thin demo stage，证明 runtime 可约束 agent 行为，但不承诺 Recon DAG。

## Non-Goals

1. 不实现完整 Recon。
2. 不新增主动扫描或 exploit 工具。
3. 不替换 `TaskOrchestrator`。
4. 不在第一版做复杂 UI。
5. 不把 evaluator 设计成强 LLM judge；第一版先用确定性规则。

## File Map

| File | Responsibility |
|---|---|
| `backend/crates/golish-agent-kit/src/lib.rs` | 导出新的 `harness` 模块 |
| `backend/crates/golish-agent-kit/src/harness/mod.rs` | harness 模块入口 |
| `backend/crates/golish-agent-kit/src/harness/types.rs` | 通用 DTO：stage、item、evidence、deliverable、gate |
| `backend/crates/golish-agent-kit/src/harness/barrier.rs` | 解析 `StageDeliverable` barrier JSON |
| `backend/crates/golish-agent-kit/src/harness/gate.rs` | 通用 gate 规则 |
| `backend/crates/golish-agent-kit/src/harness/demo.rs` | thin demo stage helper |
| `backend/crates/golish-agent-kit/src/harness/tests.rs` | gate/barrier/demo 单元测试 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs` | 给 `PlannedSubtask` 增加 optional `harness_stage` |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 执行后解析 deliverable、运行 gate、写回 recovery |
| `backend/crates/golish-core/src/events/event.rs` | 增加 harness timeline 事件 |
| `docs/design/2026-05-20-agent-harness-architecture-mvp.md` | 架构设计 |

## API Contract

第一版使用 agent 内部 barrier JSON，不暴露 HTTP API。

Naming note: `StageDeliverable`、`GateDecision`、`EvidenceItem` 等是 Golish 候选类型名，不是 Claude、OpenAI、LangChain 或 PentAGI 的标准术语。实现前可以按 Golish 现有领域语言再命名一次；本计划更看重 Anthropic 式循环的边界和职责，而不是固定这些名字。

Agent 提交的最小 JSON：

```json
{
  "stage_id": "stage-1",
  "stage_kind": "attack_surface_snapshot",
  "claims": [
    {
      "kind": "target_observed",
      "subject": "https://example.com",
      "summary": "Target responded with HTTP 200",
      "evidence_ids": ["ev-1"]
    }
  ],
  "evidence_ids": ["ev-1"],
  "skipped_checks": []
}
```

Gate 输出：

```json
{
  "allowed": false,
  "status": "blocked",
  "blocking_reasons": ["claim target_observed references missing evidence ev-1"],
  "warnings": [],
  "recovery_actions": [
    {
      "kind": "collect_evidence",
      "reason": "Every claim must reference persisted evidence"
    }
  ]
}
```

## Tasks

### Task 1: 创建通用 harness 模块骨架

**Files:** `backend/crates/golish-agent-kit/src/lib.rs`, `backend/crates/golish-agent-kit/src/harness/mod.rs`

**Steps:**

1. 在 `lib.rs` 增加模块导出：

```rust
pub mod harness;
```

2. 新建 `harness/mod.rs`：

```rust
mod barrier;
mod demo;
mod gate;
mod types;

#[cfg(test)]
mod tests;

pub use barrier::{parse_stage_deliverable, StageBarrierResult};
pub use demo::is_demo_stage;
pub use gate::validate_stage_gate;
pub use types::*;
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness
```

Expected: crate compiles; tests may be zero until later tasks add them.

**Commit:** `Add generic harness module skeleton`

### Task 2: 定义通用 harness 类型

**Files:** `backend/crates/golish-agent-kit/src/harness/types.rs`

**Steps:**

1. 添加 stage 和 evidence 基础类型：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    AttackSurfaceSnapshot,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    ToolResult,
    UserInput,
    AgentObservation,
    Fixture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub id: String,
    pub source: EvidenceSource,
    pub subject: String,
    pub summary: String,
    #[serde(default)]
    pub raw_ref: Option<String>,
}
```

2. 添加 claim、deliverable 和 skip：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageClaim {
    pub kind: String,
    pub subject: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedCheck {
    pub check: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageDeliverable {
    pub stage_id: String,
    pub stage_kind: StageKind,
    #[serde(default)]
    pub claims: Vec<StageClaim>,
    #[serde(default)]
    pub evidence: Vec<EvidenceItem>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub skipped_checks: Vec<SkippedCheck>,
}
```

3. 添加 gate 输出：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Allowed,
    Blocked,
    NeedsUser,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryActionKind {
    CollectEvidence,
    AskUser,
    RefinePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryAction {
    pub kind: RecoveryActionKind,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateDecision {
    pub allowed: bool,
    pub status: GateStatus,
    #[serde(default)]
    pub blocking_reasons: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub recovery_actions: Vec<RecoveryAction>,
}
```

4. 给 `GateDecision` 增加 constructors：

```rust
impl GateDecision {
    pub fn allowed() -> Self {
        Self {
            allowed: true,
            status: GateStatus::Allowed,
            blocking_reasons: vec![],
            warnings: vec![],
            recovery_actions: vec![],
        }
    }

    pub fn block(&mut self, reason: impl Into<String>, recovery: RecoveryAction) {
        self.allowed = false;
        self.status = GateStatus::Blocked;
        self.blocking_reasons.push(reason.into());
        self.recovery_actions.push(recovery);
    }
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness
```

Expected: DTOs compile and serde derives succeed.

**Commit:** `Define generic harness DTOs`

### Task 3: 编写 gate 失败测试

**Files:** `backend/crates/golish-agent-kit/src/harness/tests.rs`

**Steps:**

1. 添加 fixture：

```rust
use super::*;

fn deliverable_without_evidence() -> StageDeliverable {
    StageDeliverable {
        stage_id: "stage-1".to_string(),
        stage_kind: StageKind::AttackSurfaceSnapshot,
        claims: vec![StageClaim {
            kind: "target_observed".to_string(),
            subject: "https://example.com".to_string(),
            summary: "Target responded".to_string(),
            evidence_ids: vec!["ev-1".to_string()],
        }],
        evidence: vec![],
        evidence_ids: vec!["ev-1".to_string()],
        skipped_checks: vec![],
    }
}
```

2. 添加失败测试：

```rust
#[test]
fn blocks_claims_that_reference_missing_evidence() {
    let decision = validate_stage_gate(&deliverable_without_evidence());

    assert!(!decision.allowed);
    assert_eq!(decision.status, GateStatus::Blocked);
    assert!(decision
        .blocking_reasons
        .iter()
        .any(|reason| reason.contains("missing evidence ev-1")));
}

#[test]
fn blocks_deliverable_with_no_claims_and_no_skips() {
    let deliverable = StageDeliverable {
        stage_id: "stage-1".to_string(),
        stage_kind: StageKind::AttackSurfaceSnapshot,
        claims: vec![],
        evidence: vec![],
        evidence_ids: vec![],
        skipped_checks: vec![],
    };

    let decision = validate_stage_gate(&deliverable);

    assert!(!decision.allowed);
    assert!(decision
        .blocking_reasons
        .iter()
        .any(|reason| reason.contains("no claims")));
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness
```

Expected: tests fail until Task 4 implements gate.

**Commit:** `Add failing generic harness gate tests`

### Task 4: 实现通用 gate

**Files:** `backend/crates/golish-agent-kit/src/harness/gate.rs`

**Steps:**

1. 实现 gate 入口：

```rust
use std::collections::HashSet;

use super::types::*;

pub fn validate_stage_gate(deliverable: &StageDeliverable) -> GateDecision {
    let mut decision = GateDecision::allowed();

    require_claim_or_skip(deliverable, &mut decision);
    require_claim_evidence(deliverable, &mut decision);
    require_declared_evidence_exists(deliverable, &mut decision);

    decision
}
```

2. 实现 helper：

```rust
fn require_claim_or_skip(deliverable: &StageDeliverable, decision: &mut GateDecision) {
    if deliverable.claims.is_empty() && deliverable.skipped_checks.is_empty() {
        decision.block(
            "stage deliverable has no claims and no skipped checks",
            RecoveryAction {
                kind: RecoveryActionKind::CollectEvidence,
                reason: "Submit at least one supported claim or explicitly record skipped checks".to_string(),
            },
        );
    }
}

fn require_claim_evidence(deliverable: &StageDeliverable, decision: &mut GateDecision) {
    let available = evidence_id_set(deliverable);

    for claim in &deliverable.claims {
        if claim.evidence_ids.is_empty() {
            decision.block(
                format!("claim {} has no evidence references", claim.kind),
                RecoveryAction {
                    kind: RecoveryActionKind::CollectEvidence,
                    reason: "Every claim must reference at least one evidence item".to_string(),
                },
            );
        }

        for id in &claim.evidence_ids {
            if !available.contains(id) {
                decision.block(
                    format!("claim {} references missing evidence {}", claim.kind, id),
                    RecoveryAction {
                        kind: RecoveryActionKind::CollectEvidence,
                        reason: "Persist the referenced evidence before submitting the claim".to_string(),
                    },
                );
            }
        }
    }
}

fn require_declared_evidence_exists(deliverable: &StageDeliverable, decision: &mut GateDecision) {
    let available = evidence_id_set(deliverable);

    for id in &deliverable.evidence_ids {
        if !available.contains(id) {
            decision.block(
                format!("deliverable references missing evidence {}", id),
                RecoveryAction {
                    kind: RecoveryActionKind::CollectEvidence,
                    reason: "Evidence ids must match entries in the evidence list".to_string(),
                },
            );
        }
    }
}

fn evidence_id_set(deliverable: &StageDeliverable) -> HashSet<String> {
    deliverable
        .evidence
        .iter()
        .map(|item| item.id.clone())
        .collect()
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness
```

Expected: Task 3 tests pass.

**Commit:** `Implement generic harness gate`

### Task 5: 解析 barrier deliverable

**Files:** `backend/crates/golish-agent-kit/src/harness/barrier.rs`, `backend/crates/golish-agent-kit/src/harness/tests.rs`

**Steps:**

1. 实现 JSON parser：

```rust
use anyhow::{Context, Result};

use super::types::StageDeliverable;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageBarrierResult {
    pub deliverable: StageDeliverable,
}

pub fn parse_stage_deliverable(raw: &str) -> Result<StageBarrierResult> {
    let deliverable: StageDeliverable =
        serde_json::from_str(raw).context("failed to parse stage deliverable JSON")?;

    Ok(StageBarrierResult { deliverable })
}
```

2. 添加解析测试：

```rust
#[test]
fn parses_stage_deliverable_json() {
    let raw = r#"{
      "stage_id": "stage-1",
      "stage_kind": "attack_surface_snapshot",
      "claims": [],
      "evidence": [],
      "evidence_ids": [],
      "skipped_checks": [{"check": "http_probe", "reason": "not authorized"}]
    }"#;

    let parsed = parse_stage_deliverable(raw).expect("valid stage deliverable");

    assert_eq!(parsed.deliverable.stage_id, "stage-1");
    assert_eq!(parsed.deliverable.stage_kind, StageKind::AttackSurfaceSnapshot);
    assert_eq!(parsed.deliverable.skipped_checks[0].check, "http_probe");
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness
```

Expected: parser and gate tests pass.

**Commit:** `Parse generic stage deliverables`

### Task 6: 增加 demo stage 判定

**Files:** `backend/crates/golish-agent-kit/src/harness/demo.rs`, `backend/crates/golish-agent-kit/src/harness/tests.rs`

**Steps:**

1. 实现 conservative 判定：

```rust
use crate::task_orchestrator::types::PlannedSubtask;

pub fn is_demo_stage(planned: &PlannedSubtask) -> bool {
    let title = planned.title.to_ascii_lowercase();
    let description = planned.description.to_ascii_lowercase();

    title.contains("attack surface snapshot")
        || description.contains("submit_stage_deliverable")
        || description.contains("stage deliverable")
}
```

2. 添加测试：

```rust
#[test]
fn detects_demo_stage_from_deliverable_instruction() {
    let planned = crate::task_orchestrator::types::PlannedSubtask {
        title: "Capture target observations".to_string(),
        description: "Return submit_stage_deliverable JSON with evidence ids.".to_string(),
        agent: Some("pentester".to_string()),
    };

    assert!(is_demo_stage(&planned));
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness
```

Expected: demo stage helper tests pass.

**Commit:** `Detect thin harness demo stages`

### Task 7: 给 planned subtask 增加 optional harness stage

**Files:** `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs`

**Steps:**

1. 添加 stage hint：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessStageHint {
    AttackSurfaceSnapshot,
}
```

2. 给 `PlannedSubtask` 增加字段：

```rust
/// Optional harness stage that requires structured deliverable validation.
#[serde(default)]
pub harness_stage: Option<HarnessStageHint>,
```

3. 给 `SubtaskModification` 增加同名字段，允许 refiner 修改：

```rust
#[serde(default)]
pub harness_stage: Option<HarnessStageHint>,
```

4. 给 `CurrentSubtask` 和 `PlannedSubtaskInfo` 增加可选字段，并在 `render_xml` 输出：

```rust
if let Some(ref stage) = current.harness_stage {
    out.push_str(&format!("<harness_stage>{:?}</harness_stage>\n", stage));
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit task_orchestrator
```

Expected: existing orchestrator tests compile. If fixtures construct `PlannedSubtask`, update them with `harness_stage: None`.

**Commit:** `Add optional harness stage hints to subtasks`

### Task 8: 子任务执行后运行 barrier/gate

**Files:** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**Steps:**

1. 在 imports 中加入：

```rust
use crate::harness::{is_demo_stage, parse_stage_deliverable, validate_stage_gate};
```

2. 在 `Ok(agent_result)` 分支、返回结果前插入：

```rust
if planned.harness_stage.is_some() || is_demo_stage(planned) {
    match parse_stage_deliverable(&agent_result.content) {
        Ok(barrier) => {
            let decision = validate_stage_gate(&barrier.deliverable);
            let decision_json = serde_json::to_string_pretty(&decision)
                .unwrap_or_else(|_| "{\"allowed\":false}".to_string());

            let content = format!(
                "{}\n\n## Harness Gate Decision\n\n```json\n{}\n```",
                agent_result.content, decision_json
            );

            return (content, agent_result.token_usage);
        }
        Err(err) => {
            let content = format!(
                "Harness stage deliverable parse failed: {err}. \
                 The agent must return valid StageDeliverable JSON through the barrier contract."
            );
            return (content, agent_result.token_usage);
        }
    }
}
```

3. 不要在这一任务里改变 refiner 逻辑；先让 result 文本携带 gate decision，现有 refiner 会读 completed results。

**Verification:**

```bash
cargo test -p golish-agent-kit harness
cargo test -p golish-agent-kit task_orchestrator
```

Expected: harness tests pass; task orchestrator tests do not regress.

**Commit:** `Validate harness deliverables after subtasks`

### Task 9: 增加 harness timeline 事件

**Files:** `backend/crates/golish-core/src/events/event.rs`, `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**Steps:**

1. 在 `AiEvent` task-mode 区域增加：

```rust
HarnessGateEvaluated {
    task_id: String,
    subtask_id: String,
    stage_id: String,
    stage_kind: String,
    allowed: bool,
    blocking_reasons: Vec<String>,
    warnings: Vec<String>,
},
```

2. gate 完成后 emit：

```rust
self.emit(AiEvent::HarnessGateEvaluated {
    task_id: task_id.to_string(),
    subtask_id: db_subtask
        .as_ref()
        .map(|s| s.id.to_string())
        .unwrap_or_default(),
    stage_id: barrier.deliverable.stage_id.clone(),
    stage_kind: format!("{:?}", barrier.deliverable.stage_kind),
    allowed: decision.allowed,
    blocking_reasons: decision.blocking_reasons.clone(),
    warnings: decision.warnings.clone(),
});
```

3. 如果 parse 失败，也 emit `AiEvent::Warning`，内容包含 stage deliverable parse failed。

**Verification:**

```bash
cargo test -p golish-core
cargo test -p golish-agent-kit task_orchestrator
```

Expected: event enum serializes; agent-kit compiles.

**Commit:** `Emit harness gate timeline events`

### Task 10: 更新 prompt 约束为 harness-first

**Files:** `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs`

**Steps:**

1. 在 primary/subtask 执行 prompt 中加入通用规则，不写 Recon 字段：

```text
When a subtask includes a harness_stage or asks for submit_stage_deliverable,
you must complete the stage by returning valid StageDeliverable JSON.
Natural language completion is not sufficient for harness stages.
Every claim must reference evidence ids.
If a check was not performed, record it in skipped_checks with a reason.
```

2. 在 generator prompt 中说明：只有当任务明确要求 harness demo 或 stage validation 时，才设置 `harness_stage`。不要把所有安全任务自动标记成 demo stage。

3. 在 refiner prompt 中说明：如果 completed result 包含 blocked gate decision，应根据 `recovery_actions` 生成补采或澄清 subtask。

**Verification:**

```bash
cargo test -p golish-agent-kit task_orchestrator
```

Expected: prompt changes compile; tests do not regress.

**Commit:** `Teach task prompts generic harness semantics`

### Task 11: 更新文档链接和旧 Recon 草案状态

**Files:** `docs/design/harness-recon-mvp.md`, `docs/superpowers/plans/2026-05-20-golish-agent-harness.md`

**Steps:**

1. 在 `docs/design/harness-recon-mvp.md` 顶部加状态提示：

```markdown
> Superseded as the primary architecture by `docs/design/2026-05-20-agent-harness-architecture-mvp.md`.
> This document is now a candidate Recon demo/gate draft, not the harness foundation.
```

2. 在旧实现计划顶部加状态提示：

```markdown
> Superseded by `docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md`.
> Do not implement the recon-first plan until the generic harness runtime exists.
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness
```

Expected: docs-only status changes do not affect tests.

**Commit:** `Mark recon-first harness docs as superseded`

## End-to-End Validation

Run these before declaring the implementation complete:

```bash
cargo test -p golish-agent-kit harness
cargo test -p golish-agent-kit task_orchestrator
cargo test -p golish-core
```

Expected result:

- Generic harness tests pass.
- Existing task orchestrator behavior does not regress.
- Core event enum compiles and serializes.
- A demo harness stage cannot pass with prose-only completion.
- A claim that references missing evidence is blocked with recovery actions.

## Risks

1. **Parser too strict for LLM output.** Mitigation: first version requires strict JSON; later can reuse `golish-json-repair`.
2. **Stage hints over-trigger.** Mitigation: `harness_stage` is optional; fallback detection only matches explicit deliverable language.
3. **Gate output hidden in text.** Mitigation: add `HarnessGateEvaluated` event early, even before polished UI.
4. **Evidence ledger duplicated later.** Mitigation: first DTOs stay small and generic; storage integration is a follow-up plan.
5. **Prompt changes cause generator drift.** Mitigation: only mention harness stage when explicitly needed; do not auto-label every pentest subtask.

## Success Criteria

1. New docs describe harness-first architecture without relying on Recon correctness.
2. Implementation creates generic harness DTOs and gate tests before any Recon-specific type.
3. Task execution can parse a stage deliverable and append/emit a gate decision.
4. Gate can block missing evidence and produce recovery actions.
5. Old recon-first docs are clearly marked as superseded, not silently treated as canonical.
