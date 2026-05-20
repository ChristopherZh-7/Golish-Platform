# Golish Agent Harness 实现计划

> Superseded by `docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md`.
> Do not implement this recon-first plan until the generic harness runtime exists.

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 借鉴 PentAGI 的 `Flow -> Task -> Subtask -> Tool` 骨架，在 Golish 现有 `TaskOrchestrator` 旁边补上 Recon 阶段专用 harness，使 agent 不能只靠自然语言声称完成侦察，必须提交结构化证据并通过 gate。
**架构：** 保留现有 `backend/crates/golish-agent-kit/src/task_orchestrator/` 的 Generator、Primary Agent、Refiner、Reporter 体系；新增 `harness::recon` 模块承载安全阶段契约、证据模型、barrier 结果和 gate validator。第一版先用纯 Rust 类型和纯函数证明状态约束，再把 Recon gate 接入 task-mode 子任务执行之后。
**技术栈：** Rust、serde、tokio、anyhow、现有 `golish-agent-kit` task orchestrator、现有 `AiEvent` 事件流。

## Problem

Golish 现在已经有 PentAGI-style 的任务编排：

- `TaskOrchestrator` 执行 `Generator -> Primary Agent Loop -> Refiner -> Reporter`。
- `ExecutionContext` 已经能携带已完成 subtasks、当前 subtask、剩余计划。
- `execute_single_subtask` 已经有 enrichment、planning、reflector retry 和 user input pause。

缺口是：安全测试阶段仍然缺少“阶段专用验收”。例如 Recon 子任务结束后，系统目前主要拿到的是 agent 文本结果；它不能硬判断：

1. 是否真的执行过 DNS/端口/服务/HTTP/技术栈检查。
2. 空结果是“已检查为空”还是“未检查”。
3. open port 是否都有 service fingerprint。
4. HTTP service 是否都有 probe/tech evidence。
5. 是否允许进入 Vulnerability Matching。

因此，本计划不是重写 orchestrator，而是给现有 orchestrator 增加一个安全领域 harness 层。

## Goals

1. 新增 Recon 专用结构化交付物 `ReconDeliverable`。
2. 新增 gate 输出 `ReconGateDecision`，明确 `allowed`、阻断原因、警告和推荐补采动作。
3. 新增纯函数 `validate_recon_gate(&ReconDeliverable) -> ReconGateDecision`。
4. 新增 barrier 结果类型，让 Recon 子任务必须提交结构化 deliverable，而不是自然语言完成声明。
5. 将 Recon gate 接入 task-mode：Recon 子任务完成后，gate 未通过时把推荐补采动作反馈给 refiner 或生成后续 subtask。
6. 暂不接真实扫描工具也能测试：先用 mock observation / fixture 覆盖 gate 规则。

## Non-Goals

1. 不在第一版实现完整自动化渗透测试。
2. 不在第一版新增危险扫描能力。
3. 不替换现有 `TaskOrchestrator`。
4. 不一次性实现 Verify / Report / Exploit Planning 全部 harness。
5. 不把 UI 做复杂；第一版只需要能从事件和日志看见 gate 结果。

## File Map

| File | Responsibility |
|---|---|
| `backend/crates/golish-agent-kit/src/lib.rs` | 导出新的 `harness` 模块 |
| `backend/crates/golish-agent-kit/src/harness/mod.rs` | harness 模块入口 |
| `backend/crates/golish-agent-kit/src/harness/recon/mod.rs` | Recon harness 入口和公共导出 |
| `backend/crates/golish-agent-kit/src/harness/recon/types.rs` | `ReconDeliverable`、`EvidenceItem`、`ReconGateDecision` 等 DTO |
| `backend/crates/golish-agent-kit/src/harness/recon/gate.rs` | `validate_recon_gate` 硬规则 |
| `backend/crates/golish-agent-kit/src/harness/recon/barrier.rs` | `submit_recon_deliverable` 的内部结果模型与解析入口 |
| `backend/crates/golish-agent-kit/src/harness/recon/tests.rs` | gate 和 barrier 单元测试 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs` | 增加 subtask phase / harness hint 字段，标记 Recon 子任务 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 在子任务结果返回后识别 Recon deliverable 并运行 gate |
| `backend/crates/golish-core/src/events/event.rs` | 增加 Recon gate 事件，供前端展示 |
| `docs/design/harness-recon-mvp.md` | 更新设计文档，链接到本实现计划 |

## API Contract

第一版不是外部 HTTP API，而是 agent 内部 barrier 契约。

```rust
pub struct ReconBarrierResult {
    pub deliverable: ReconDeliverable,
}

pub fn parse_recon_barrier_result(raw: &str) -> anyhow::Result<ReconBarrierResult>;
```

LLM 输出必须能解析为：

```json
{
  "target": { "value": "example.com", "kind": "domain" },
  "scope": "in_scope",
  "dns_records": [],
  "resolved_ips": [],
  "open_ports": [],
  "services": [],
  "http_services": [],
  "technologies": [],
  "evidence_items": [],
  "skipped_checks": [],
  "gate_status": "pending"
}
```

Gate 输出必须稳定序列化为：

```json
{
  "allowed": false,
  "blocking_reasons": ["target is domain but no resolved IPs or DNS skip reason"],
  "warnings": [],
  "missing_checks": ["dns_resolve"],
  "recommended_next_actions": [
    { "kind": "run_check", "check": "dns_resolve", "reason": "Resolve domain before port scanning" }
  ]
}
```

## Tasks

### Task 1: 创建 Recon harness 模块骨架

**Files:** `lib.rs`, `harness/mod.rs`, `harness/recon/mod.rs`

**Steps:**

1. 在 `backend/crates/golish-agent-kit/src/lib.rs` 增加：

```rust
pub mod harness;
```

2. 新建 `backend/crates/golish-agent-kit/src/harness/mod.rs`：

```rust
pub mod recon;
```

3. 新建 `backend/crates/golish-agent-kit/src/harness/recon/mod.rs`：

```rust
mod barrier;
mod gate;
mod types;

#[cfg(test)]
mod tests;

pub use barrier::{parse_recon_barrier_result, ReconBarrierResult};
pub use gate::validate_recon_gate;
pub use types::*;
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness::recon
```

Expected: crate compiles; no Recon tests exist yet, so command reports zero or newly added tests depending on task order.

**Commit:** `Add recon harness module skeleton`

### Task 2: 定义 Recon 结构化类型

**Files:** `harness/recon/types.rs`

**Steps:**

1. 添加所有 DTO，统一使用 `serde` 的 snake_case，方便后续作为 tool/barrier JSON schema。

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Domain,
    Ip,
    Url,
    Cidr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeStatus {
    InScope,
    OutOfScope,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconCheck {
    ScopeConfirm,
    DnsResolve,
    PortScan,
    ServiceFingerprint,
    HttpProbe,
    TechDetect,
    EvidenceCapture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconGateStatus {
    Pending,
    Passed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconDeliverable {
    pub target: ReconTarget,
    pub scope: ScopeStatus,
    #[serde(default)]
    pub dns_records: Vec<DnsRecord>,
    #[serde(default)]
    pub resolved_ips: Vec<ResolvedIp>,
    #[serde(default)]
    pub open_ports: Vec<OpenPort>,
    #[serde(default)]
    pub services: Vec<ServiceFingerprint>,
    #[serde(default)]
    pub http_services: Vec<HttpService>,
    #[serde(default)]
    pub technologies: Vec<TechnologyFinding>,
    #[serde(default)]
    pub evidence_items: Vec<EvidenceItem>,
    #[serde(default)]
    pub skipped_checks: Vec<SkippedCheck>,
    pub gate_status: ReconGateStatus,
}
```

2. 在同文件继续定义 `ReconTarget`、`DnsRecord`、`ResolvedIp`、`OpenPort`、`ServiceFingerprint`、`HttpService`、`TechnologyFinding`、`EvidenceItem`、`SkippedCheck`、`RecommendedAction`、`ReconGateDecision`。字段以 `docs/design/harness-recon-mvp.md` 的契约为准，所有 list 字段加 `#[serde(default)]`。

**Verification:**

```bash
cargo test -p golish-agent-kit harness::recon
```

Expected: DTO 编译通过。

**Commit:** `Define recon deliverable DTOs`

### Task 3: 编写 gate 的失败测试

**Files:** `harness/recon/tests.rs`

**Steps:**

1. 添加 fixture helper：

```rust
fn base_deliverable() -> ReconDeliverable {
    ReconDeliverable {
        target: ReconTarget {
            value: "example.com".to_string(),
            kind: TargetKind::Domain,
        },
        scope: ScopeStatus::InScope,
        dns_records: vec![],
        resolved_ips: vec![],
        open_ports: vec![],
        services: vec![],
        http_services: vec![],
        technologies: vec![],
        evidence_items: vec![],
        skipped_checks: vec![],
        gate_status: ReconGateStatus::Pending,
    }
}
```

2. 添加测试：

```rust
#[test]
fn blocks_domain_without_dns_result_or_skip_reason() {
    let decision = validate_recon_gate(&base_deliverable());
    assert!(!decision.allowed);
    assert!(decision.missing_checks.contains(&ReconCheck::DnsResolve));
}

#[test]
fn blocks_open_port_without_service_fingerprint() {
    let mut d = base_deliverable();
    d.resolved_ips.push(ResolvedIp::new_for_test("93.184.216.34", "e1"));
    d.evidence_items.push(EvidenceItem::new_for_test("e1", "dns resolved"));
    d.open_ports.push(OpenPort::new_for_test("93.184.216.34", 443, "e2"));
    d.evidence_items.push(EvidenceItem::new_for_test("e2", "443 open"));

    let decision = validate_recon_gate(&d);
    assert!(!decision.allowed);
    assert!(decision.missing_checks.contains(&ReconCheck::ServiceFingerprint));
}
```

3. 为测试 helper 实现 `new_for_test`，只放在 `#[cfg(test)]` impl 中。

**Verification:**

```bash
cargo test -p golish-agent-kit harness::recon
```

Expected: 测试失败，因为 `validate_recon_gate` 还未实现。

**Commit:** `Add failing recon gate tests`

### Task 4: 实现 `validate_recon_gate`

**Files:** `harness/recon/gate.rs`

**Steps:**

1. 添加入口：

```rust
use super::types::*;

pub fn validate_recon_gate(deliverable: &ReconDeliverable) -> ReconGateDecision {
    let mut decision = ReconGateDecision::allowed();

    require_in_scope(deliverable, &mut decision);
    require_dns_for_domain(deliverable, &mut decision);
    require_port_scan_result_or_skip(deliverable, &mut decision);
    require_service_for_open_ports(deliverable, &mut decision);
    require_tech_for_http_services(deliverable, &mut decision);
    require_evidence(deliverable, &mut decision);

    decision.allowed = decision.blocking_reasons.is_empty();
    decision
}
```

2. 每个 helper 只处理一条规则。阻断时同时写入：

- `blocking_reasons`
- `missing_checks`
- `recommended_next_actions`

3. 对 `skipped_checks` 增加 helper：

```rust
fn has_skip(deliverable: &ReconDeliverable, check: ReconCheck) -> bool {
    deliverable.skipped_checks.iter().any(|skip| skip.check == check)
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness::recon
```

Expected: Task 3 的失败测试变绿。

**Commit:** `Implement recon gate validator`

### Task 5: 增加 barrier 解析

**Files:** `harness/recon/barrier.rs`, `harness/recon/tests.rs`

**Steps:**

1. 实现 JSON 解析：

```rust
use anyhow::{Context, Result};

use super::types::ReconDeliverable;

#[derive(Debug, Clone, PartialEq)]
pub struct ReconBarrierResult {
    pub deliverable: ReconDeliverable,
}

pub fn parse_recon_barrier_result(raw: &str) -> Result<ReconBarrierResult> {
    let deliverable: ReconDeliverable =
        serde_json::from_str(raw).context("failed to parse recon deliverable JSON")?;

    Ok(ReconBarrierResult { deliverable })
}
```

2. 添加测试：

```rust
#[test]
fn parses_recon_deliverable_barrier_json() {
    let raw = r#"{
      "target": { "value": "example.com", "kind": "domain" },
      "scope": "in_scope",
      "gate_status": "pending"
    }"#;

    let parsed = parse_recon_barrier_result(raw).expect("valid deliverable JSON");
    assert_eq!(parsed.deliverable.target.value, "example.com");
    assert_eq!(parsed.deliverable.scope, ScopeStatus::InScope);
}
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness::recon
```

Expected: parsing and gate tests pass.

**Commit:** `Parse recon barrier deliverables`

### Task 6: 标记 Recon 子任务

**Files:** `task_orchestrator/types.rs`

**Steps:**

1. 增加阶段枚举：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessPhase {
    Recon,
}
```

2. 给 `PlannedSubtask` 增加可选字段：

```rust
#[serde(default)]
pub harness_phase: Option<HarnessPhase>,
```

3. 给 `CurrentSubtask` 增加同名字段，并在构造 `ExecutionContext` 的地方透传。

4. 在 generator prompt 后续任务中要求 recon 相关子任务输出 `"harness_phase": "recon"`。第一版如果 prompt JSON 修复链暂不稳定，可以在 Rust 侧用标题/agent fallback：标题包含 `recon`、`information gathering`、`attack surface` 且 agent 为 `pentester` 时标记为 Recon。

**Verification:**

```bash
cargo test -p golish-agent-kit task_orchestrator
```

Expected: 现有 task orchestrator tests 编译通过。

**Commit:** `Mark recon subtasks for harness validation`

### Task 7: 在 subtask 执行后运行 Recon gate

**Files:** `task_orchestrator/subtask_phases/execute.rs`

**Steps:**

1. 在 `execute_single_subtask` 拿到 `agent_result` 后，判断 `planned.harness_phase == Some(HarnessPhase::Recon)`。

2. 对 Recon 子任务调用：

```rust
let barrier = parse_recon_barrier_result(&agent_result.content)?;
let gate = validate_recon_gate(&barrier.deliverable);
```

3. 如果解析失败，把结果改写为明确失败文本，让 reflector 或 refiner 能理解：

```rust
return (
    format!(
        "Recon deliverable parse failed: {err}. The agent must return submit_recon_deliverable JSON."
    ),
    agent_result.token_usage,
);
```

4. 如果 gate 阻断，把 `blocking_reasons` 和 `recommended_next_actions` 写入 subtask result；不要把 subtask 标记为完整 Recon 成果。

5. 如果 gate 放行，把 gate decision 附加到 result，供后续 Vulnerability Matching 使用。

**Verification:**

```bash
cargo test -p golish-agent-kit task_orchestrator
cargo test -p golish-agent-kit harness::recon
```

Expected: 现有 orchestrator 测试不回退；Recon gate 测试通过。

**Commit:** `Run recon gate after recon subtasks`

### Task 8: 增加 Recon gate 事件

**Files:** `backend/crates/golish-core/src/events/event.rs`, `task_orchestrator/subtask_phases/execute.rs`

**Steps:**

1. 在 `AiEvent` 的 task-mode 区域增加：

```rust
ReconGateEvaluated {
    task_id: String,
    subtask_id: String,
    allowed: bool,
    blocking_reasons: Vec<String>,
    warnings: Vec<String>,
    missing_checks: Vec<String>,
},
```

2. 在 `execute_single_subtask` gate 完成后 emit 该事件。

3. `missing_checks` 使用字符串，避免 frontend 必须立刻依赖 Rust enum。

**Verification:**

```bash
cargo test -p golish-core
cargo test -p golish-agent-kit task_orchestrator
```

Expected: event enum 序列化编译通过；agent-kit 能继续 emit events。

**Commit:** `Emit recon gate events`

### Task 9: 更新设计文档

**Files:** `docs/design/harness-recon-mvp.md`

**Steps:**

1. 在文档新增 `PentAGI Borrowed Skeleton` 小节：

```markdown
## PentAGI Borrowed Skeleton

Golish borrows PentAGI's orchestration skeleton, not its soft completion semantics:

- `Flow -> Task -> Subtask` maps to `Run -> Objective -> PhaseStep`.
- `done(result)` maps to `submit_recon_deliverable(ReconDeliverable)`.
- `Refiner` remains useful, but it must react to `ReconGateDecision`.
- Tool use remains LLM-guided, while phase completion is harness-validated.
```

2. 在 `MVP 实现顺序` 后链接本计划：

```markdown
Implementation plan: `docs/superpowers/plans/2026-05-20-golish-agent-harness.md`
```

**Verification:**

```bash
cargo test -p golish-agent-kit harness::recon
```

Expected: docs change does not affect tests; recon tests still pass.

**Commit:** `Document PentAGI-inspired Golish harness plan`

## End-to-End Validation

Run these before declaring the plan implemented:

```bash
cargo test -p golish-agent-kit harness::recon
cargo test -p golish-agent-kit task_orchestrator
cargo test -p golish-core
```

Expected result:

- Recon gate tests pass.
- Existing task orchestrator behavior does not regress.
- Core event enum compiles and serializes.

## Risks

1. **LLM output may not be strict JSON.** Mitigation: first version parses raw JSON only; later reuse `golish-json-repair` if needed.
2. **Generator may not reliably set `harness_phase`.** Mitigation: add Rust-side fallback classification for obvious recon subtasks.
3. **Gate could block too aggressively.** Mitigation: support `skipped_checks` and warnings so authorized skips are explicit.
4. **UI may lag backend semantics.** Mitigation: emit `ReconGateEvaluated` event even before adding polished UI.
5. **Existing orchestrator tests may assume `PlannedSubtask` shape.** Mitigation: new field is optional with `#[serde(default)]`.

## Success Criteria

1. A Recon subtask that returns prose-only output fails parsing and cannot silently advance as a valid Recon deliverable.
2. A Recon deliverable with an open port but no service fingerprint is blocked.
3. A Recon deliverable with complete evidence passes.
4. Gate output includes machine-readable missing checks and recommended next actions.
5. Existing PentAGI-style task orchestration remains intact.
