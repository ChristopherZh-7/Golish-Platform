# Stage Harness MVP · external_attack_surface

- **Author**: MCP-1 (代笔 MCP-2 owner 内容)
- **Date**: 2026-05-26
- **Status**: Implemented (Phase 1 · 见 commits 163f04e / 559416f / bb98f3e / 1bcdc52 / 52f70d4 / 1b0a23e / 10dd927 / b106d55)
- **Source of truth**: `docs/design/2026-05-26-operation-harness-profile-dag-lab.md` §21
- **Depends on**: Doc 1 (evidence ledger) + Doc 2 (mcp resource)

> 本文是 Doc 3·三份拆分中的第三份·Phase 0 design only。
>
> 仅为设计提案 / Profile / DAG / Stage Spec / NlSlice / Sprint Contract / Gate 草案。**不动** `backend/crates/golish-agent-kit/src/task_orchestrator/`。
>
> Phase 1 实施需获得用户 §AGENTS.md §2.7 明示授权。

---

## 1. 目标

把 chat panel 现有的 task 模式（PentAGI 风格的 Generator/Refiner/Reflector/Reporter）**重构成 harness**：

```text
现状 task 模式
  Generator → 拆 13 个 subtask → Refiner → Reflector → Reporter
  问题: agent 动了任意工具就被判完成（§13.3 真痛点 1）

目标 harness 模式
  user intent → Profile (assessment/pentest/red_team/...)
              → 投影 Operation DAG
              → 顺序进 Stage Harness
              → 每 stage 内部用 PentAGI-style inner loop
              → 提交结构化 deliverable
              → Gate 验证（确定性 + sprint contract）
              → 过则进下一 stage
```

MVP 范围严格限定：仅 `assessment` profile + L2 active_recon + 1 个 stage `external_attack_surface`。

---

## 2. Profile 定义

### 2.1 assessment profile JSON

```json
// resources/harness/profiles/assessment.json (Phase 1 新建)
{
  "id": "assessment",
  "display_name": "Security Assessment",
  "max_authorization": "active_recon",
  "allowed_stage_kinds": [
    "scoping",
    "target_intel",
    "external_attack_surface",
    "enumeration",
    "reporting"
  ],
  "forbidden_stage_kinds": [
    "vuln_triage",
    "verification",
    "access_validation",
    "internal_discovery",
    "objective_pathing",
    "objective_simulation",
    "cleanup"
  ],
  "approval_policy": {
    "before_active_scan": true,
    "before_scope_expansion": true
  },
  "cleanup_required": false,
  "evidence_required": true
}
```

### 2.2 Authorization Level 含义

```text
L0 observe_only    : 仅查现有数据，无任何探测
L1 passive_intel   : 仅被动收集（公开数据库 / passive DNS / CT log）
L2 active_recon    : 低风险探测（HTTP probe / DNS query / 主动子域枚举）  ← assessment MAX
L3 vuln_validation : 非破坏性漏洞验证 (pentest)
L4 controlled_exploit : 受控 exploit 验证 (pentest)
L5 post_exploit_red_team : 横移 / 后渗透 (red_team)
```

assessment max=L2 意味着所有 tool 调用必须 ≤ L2，超出走 approval。

### 2.3 Operation.user_intent_constraints (§14.1)

```rust
pub struct Operation {
    pub id: OperationId,
    pub profile: ProfileId,
    pub user_intent_constraints: Vec<IntentConstraint>,
}

pub enum IntentConstraint {
    PassiveOnly,
    NoActiveProbeOnDomain(DomainPattern),
    NoExploitValidation,
    RateLimitedPerHour { tool: ToolName, max_per_hour: u32 },
}

pub fn effective_tool_allow_set(op: &Operation, stage: &StageSpec) -> HashSet<ToolName> {
    let profile_allow = profile_max_tools(op.profile, stage.kind);
    let stage_allow: HashSet<_> = stage.allowed_tools.iter().cloned().collect();
    let intent_block: HashSet<_> = op.user_intent_constraints.iter()
        .flat_map(|c| c.implied_forbidden_tools())
        .collect();
    profile_allow.intersection(&stage_allow).cloned().collect::<HashSet<_>>()
        .difference(&intent_block).cloned().collect()
}
```

**关键**：`intent_axis` 不在 NlSlice 里，在 Operation 顶层。agent 看不到这个枚举·gate 验证时才读。

---

## 3. Operation DAG

### 3.1 Base Operation Graph

```text
            ┌─────────┐
            │ scoping │
            └────┬────┘
                 ▼
         ┌──────────────┐
         │ target_intel │
         └──────┬───────┘
                ▼
    ┌────────────────────────┐
    │ external_attack_surface │  ← MVP 专注于此
    └────────┬───────────────┘
             ▼
       ┌───────────┐
       │ enumeration│   ← assessment 可达
       └─────┬─────┘
             ▼
         ┌────────┐
         │ reporting │
         └─────────┘

(以下 stage assessment profile 不允许走 · 留作未来 profile 扩展)
   vuln_triage / verification / access_validation /
   internal_discovery / objective_pathing /
   objective_simulation / cleanup
```

### 3.2 Profile 投影 (assessment)

```text
assessment profile allowed DAG:
  scoping → target_intel → external_attack_surface → enumeration → reporting
```

forbidden_stage_kinds 的 7 个 stage assessment 路径上不会出现。

### 3.3 DAG 投影函数

```rust
pub fn project_dag(profile: &Profile, base: &BaseGraph) -> AllowedDag {
    let allowed_nodes: HashSet<_> = profile.allowed_stage_kinds.iter().collect();
    let allowed_edges = base.edges.iter()
        .filter(|(from, to)| allowed_nodes.contains(from) && allowed_nodes.contains(to))
        .cloned()
        .collect();
    AllowedDag { nodes: allowed_nodes, edges: allowed_edges }
}
```

---

## 4. Stage Spec (external_attack_surface)

### 4.1 Stage Spec JSON

```json
// resources/harness/stages/external_attack_surface.json (Phase 1 新建)
{
  "id": "external_attack_surface",
  "kind": "external_attack_surface",
  "risk_level": "medium",
  "requires_stages": ["scoping", "target_intel"],
  "allowed_next_stages": ["enumeration", "reporting"],
  "allowed_tools": [
    "query_target_data",
    "dns_resolve",
    "subdomain_enum_passive",
    "http_probe",
    "fingerprint_target",
    "shodan_query",
    "log_operation",
    "log_scan_result",
    "submit_external_attack_surface_deliverable"
  ],
  "forbidden_tools": [
    "metasploit",
    "sqlmap",
    "credential_attack",
    "destructive_action",
    "exploit_validation"
  ],
  "deliverable_schema": "ExternalAttackSurfaceDeliverable",
  "gate_validator": "validate_external_attack_surface_gate",
  "required_checks": [
    "scope_status_present",
    "evidence_non_empty",
    "unchecked_distinct_from_checked_empty",
    "out_of_scope_targets_excluded",
    "min_tool_invocations_per_check"
  ],
  "min_invocations": {
    "dns_resolve": 1,
    "http_probe": 1,
    "subdomain_enum_passive": 1
  },
  "human_approval": {
    "required_before": ["active_scan", "exploit_validation"]
  },
  "agent_continuity": "single_session",
  "inherits_evidence_from": [
    { "stage_kind": "target_intel", "evidence_kinds": ["dns_a", "asn", "whois"] }
  ]
}
```

### 4.2 5 个问题答案

| # | 问 | 答 |
|---|---|---|
| 1 | 输入是什么 schema | `{ scope: [...], known_assets: [...], operation_id: UUID }` |
| 2 | 输出是什么 schema | `ExternalAttackSurfaceDeliverable`（见 §4.3） |
| 3 | 允许什么工具 | `allowed_tools` 列出的 9 个·effective_tool_allow_set 投影 |
| 4 | done 判定 | `validate_external_attack_surface_gate(deliverable, evidence_ledger).allowed == true` |
| 5 | 失败 / 越界 | OOS target 写 `skipped_checks` 走 `audit_role='approval'`·gate 失败回灌 repair |

### 4.3 ExternalAttackSurfaceDeliverable

```rust
pub struct ExternalAttackSurfaceDeliverable {
    pub stage_id: StageId,
    pub stage_run_id: Uuid,
    pub claims: Vec<StageClaim>,                  // 每个 claim 必须有 evidence_refs
    pub evidence_refs: Vec<EvidenceAuditId>,      // 所有 evidence
    pub skipped_checks: Vec<SkippedCheckRecord>,
    pub findings: Vec<Finding>,                   // 结构化 finding
    pub required_checks_done: Vec<String>,        // app-level 仅 hint，gate 用 stage_spec 为准
}

pub struct StageClaim {
    pub kind: String,                             // 'http_service_observed' / 'subdomain_found' / ...
    pub subject: String,
    pub summary: String,
    pub evidence_ids: Vec<EvidenceAuditId>,       // 必须非空 + 全 InScope
}

pub struct SkippedCheckRecord {
    pub check: String,
    pub reason: SkipReason,                       // 强制枚举 见 Doc 1 §4.6
}

pub struct Finding {
    pub finding_id: Uuid,
    pub kind: String,
    pub subject: String,
    pub severity: FindingSeverity,
    pub evidence_refs: Vec<EvidenceAuditId>,
}
```

---

## 5. Inner Loop (PentAGI 风格被包进 Stage Harness)

### 5.1 Loop 结构

```text
Stage harness entry
  ↓
charter 注入 system prompt (allowed_tools / forbidden_tools / 期望 deliverable schema)
  ↓
Sprint Contract 生成 (cross-vendor LLM 填变量) → 锁定
  ↓
inner loop:
  generate subtasks (PentAGI's Generator)
    ↓
  for each subtask:
    NlSlice 推导 (intent_axis classifier + bounded_targets)
    ↓
    pre-action authorizer (per tool call)
      ↓ 通过 → execute tool
      ↓ deny → 抛 ToolCallBlocked
    ↓
    tool_result → EvidenceLedger.append() → got evidence_audit_id
    ↓
    LLM 仅看到 eid (不看 raw)·想看走 evidence_read(eid)
    ↓
    refiner: 看 completed result + gate feedback 决定下一步
    ↓
  agent 决定 ready to submit → call submit_external_attack_surface_deliverable
    ↓
gate validator
  ↓ allowed → handoff packet 给下一 stage
  ↓ blocked → recovery_actions → 喂给 refiner 加 repair subtasks
  ↓ needs_user → 走 audit_role='approval' pause
```

### 5.2 现有代码改造点

```text
backend/crates/golish-agent-kit/src/task_orchestrator/
├── types.rs
│   PlannedSubtask 加字段：
│     pub harness_stage: Option<HarnessStageHint>,
│     pub nl_slice: Option<NlSlice>,
│     pub acceptance_criteria: Vec<String>,
│
├── subtask_phases/execute.rs
│   execute_single_subtask 末端加：
│     if planned.harness_stage.is_some() {
│         let deliverable = parse_stage_deliverable(&result)?;
│         let decision = validate_stage_gate(&deliverable, &ledger, &spec)?;
│         if !decision.allowed {
│             return Err(GateBlocked { reasons, recovery });
│         }
│     }
│
└── (新模块) ../harness/
    ├── mod.rs
    ├── stage_harness.rs  // Stage charter 注入 + Sprint Contract 生成 + Gate 调度
    ├── nl_slice.rs       // IntentClassifier + NlSlice 构造
    ├── pre_action_authorizer.rs
    └── gate/
        ├── mod.rs
        ├── schema_check.rs
        ├── contract_check.rs
        ├── scope_check.rs
        └── vacuous_check.rs
```

详细的步骤化改造在 plan 文档（`docs/superpowers/plans/2026-05-26-task-mode-refactor-to-harness.md`）。

---

## 6. NlSlice (终态 4 字段)

```rust
pub struct NlSlice {
    pub subtask_id: SubtaskId,
    pub stage_kind: StageKind,
    pub sealed_origin_session: SessionId,
    pub deliverable_schema_id: SchemaId,
}
```

**禁止继续加字段**（§14.1 + §18 警告）。再加抽 SubtaskContext 新结构，不在 NlSlice 上扩。

### 6.1 IntentClassifier 规则化

```rust
pub struct IntentClassifier {
    pub passive_keywords: Vec<String>,         // "看看 / 调研 / 列举 / passive / observe"
    pub active_probe_keywords: Vec<String>,    // "扫描 / 探测 / 主动 / probe"
    pub exploit_keywords: Vec<String>,         // "验证 / payload / 利用 / exploit"
    pub vuln_validation_keywords: Vec<String>,
}

impl IntentClassifier {
    pub fn classify(&self, user_intent: &str, stage_kind: StageKind) -> IntentAxis {
        let lower = user_intent.to_lowercase();
        // 命中 exploit_keywords → ExploitValidation
        if self.exploit_keywords.iter().any(|kw| lower.contains(kw)) {
            return IntentAxis::ExploitValidation;
        }
        // 命中 vuln_validation_keywords → VulnValidation
        if self.vuln_validation_keywords.iter().any(|kw| lower.contains(kw)) {
            return IntentAxis::VulnValidation;
        }
        // 命中 active_probe_keywords → ActiveProbe
        if self.active_probe_keywords.iter().any(|kw| lower.contains(kw)) {
            return IntentAxis::ActiveProbe;
        }
        // 命中 passive_keywords → PassiveObserve
        if self.passive_keywords.iter().any(|kw| lower.contains(kw)) {
            return IntentAxis::PassiveObserve;
        }
        // 默认按 stage_kind
        match stage_kind {
            StageKind::Scoping | StageKind::TargetIntel => IntentAxis::PassiveObserve,
            StageKind::ExternalAttackSurface => IntentAxis::PassiveObserve,
            StageKind::Enumeration => IntentAxis::ActiveProbe,
            _ => IntentAxis::PassiveObserve,
        }
    }
}
```

**不用 LLM**（同源带偏）。词库查表 deterministic·agent 不可绕过。

---

## 7. Sprint Contract

### 7.1 Sprint Skeleton (profile-driven)

```json
// resources/harness/profiles/assessment.sprint_skeleton.json (Phase 1 新建)
{
  "external_attack_surface": {
    "expected_findings": [
      {
        "kind": "subdomain",
        "expected_count_range": [1, 200],
        "required_evidence_kinds": ["dns_a", "ct_log"]
      },
      {
        "kind": "http_service",
        "expected_count_range": [0, 50],
        "required_evidence_kinds": ["http_probe"]
      }
    ],
    "time_budget_minutes": 30,
    "min_tool_invocations": {
      "dns_resolve": 1,
      "http_probe": 1,
      "subdomain_enum_passive": 1
    }
  }
}
```

### 7.2 LLM 填变量

```text
profile.sprint_skeleton  (静态骨架)
    + planner LLM (跨厂商，必须 != stage_executor)
    填变量:
      - specific_target_context
      - expected_count_range (按用户 scope 调整)
      - time_budget_minutes (按 scope 大小)
    ↓
SprintContract (locked-at-stage-start)
    ↓
存 sprint_contracts 表 + 写 audit_log
```

**预算不够 fallback**：v0 用同厂商不同 temperature·但 Doc 3 明记该疑点·v1 强制 cross-vendor。

---

## 8. Gate Validator

### 8.1 Gate 调用链

```rust
pub fn validate_external_attack_surface_gate(
    deliverable: &ExternalAttackSurfaceDeliverable,
    ledger: &EvidenceLedger,
    spec: &StageSpec,
    contract: &SprintContract,
) -> GateResult {
    // 顺序很重要：从形式检查到内容检查
    schema_check(deliverable, &spec.deliverable_schema)?;
    scope_check(deliverable, ledger)?;
    contract_check(deliverable, contract, ledger)?;
    vacuous_check(deliverable, ledger, spec)?;
    freshness_check(deliverable, ledger, spec)?;
    aggregation_check(deliverable, ledger)?;  // 4b 轻量
    Ok(GateResult::Pass)
}
```

### 8.2 各 check 含义

| Check | 验证 | 失败时 |
|---|---|---|
| schema_check | deliverable 字段齐全 + JSON schema 合 | BLOCK + recovery=「补字段」 |
| scope_check | claim.evidence_refs[*] 当前 label = InScope | BLOCK + recovery=「替换 OOS evidence」 |
| contract_check | findings 数量在 Sprint Contract range 内 + min_tool_invocations 达标 | BLOCK + recovery=「补 finding」 |
| vacuous_check | 见 §8.3 vacuous detector | BLOCK + adversarial alert |
| freshness_check | evidence as_of_timestamp + max_age < NOW | warning（默认）或 BLOCK（hard_expired） |
| aggregation_check | findings 不涉及多个 OOS evidence 拼出的推论 | BLOCK + 走 user_approval |

### 8.3 Vacuous detector

```rust
fn detect_vacuous(
    d: &Deliverable,
    ledger: &EvidenceLedger,
    spec: &StageSpec,
) -> Option<VacuousKind> {
    // (a) no tool invocation
    if ledger.tool_call_count(d.stage_run_id) == 0 {
        return Some(VacuousKind::NoToolInvocation);
    }

    // (b) fake pattern: required_check 缺 tool 调用记录
    for check in &spec.required_checks {
        let min = spec.min_invocations.get(check).copied().unwrap_or(1);
        let actual = ledger.find_tool_calls_for_check(d.stage_run_id, check).len();
        if actual < min {
            return Some(VacuousKind::FakePattern {
                check: check.clone(),
                expected_min: min,
                actual,
            });
        }
    }

    // (c) skip pattern: Other 类型 skip 太多 OR Other 无 evidence_ref
    let other_count = d.skipped_checks.iter()
        .filter(|s| matches!(s.reason, SkipReason::Other { .. }))
        .count();
    if other_count > spec.max_other_skips.unwrap_or(2) {
        return Some(VacuousKind::SkipPattern { other_count });
    }
    for s in &d.skipped_checks {
        if let SkipReason::Other { evidence_ref, .. } = &s.reason {
            if !ledger.contains(*evidence_ref) {
                return Some(VacuousKind::FakePattern {
                    check: s.check.clone(),
                    expected_min: 1,
                    actual: 0,
                });
            }
        }
    }

    None
}
```

**关键**：detector 以 `StageSpec.required_checks` 为准绳·**不**读 `deliverable.required_checks_done`（agent 可清空该字段绕过）。

**一阶 Rust 规则**（无 LLM）：上面这套·必须在外部 LLM 离线时仍能 BLOCK。
**二阶 LLM detector**（识别 placeholder evidence 等）：Phase 2 加·不进 MVP。

---

## 9. Stage 间 evidence 可见性

### 9.1 默认 stage-scoped

Gate 默认查询：

```sql
SELECT ... FROM evidence_classifications
WHERE evidence_audit_id = $eid
  AND valid_to IS NULL
  AND producing_stage_run_id = $current_stage_run_id
LIMIT 1;
```

跨 stage 的 evidence 默认查不到 → 强制本 stage 内重 fetch。

### 9.2 explicit carry_over 白名单

```json
{
  "inherits_evidence_from": [
    { "stage_kind": "target_intel", "evidence_kinds": ["dns_a", "asn", "whois"] }
  ]
}
```

Gate 在 carry_over 时：

```sql
... AND (producing_stage_run_id = $current_stage_run_id
      OR (producing_stage_run_id IN (SELECT id FROM stage_runs WHERE kind='target_intel')
          AND kind IN ('dns_a', 'asn', 'whois')))
```

### 9.3 cross-profile transition

如 assessment 升 pentest：

```text
operation_state A (profile=assessment) 
  ↓ 用户审批 cross-profile expand
operation_state B (profile=pentest, superseded_by=A)
  ↓
B 的 stage 全部走新 ScopeService snapshot
B 从 A 走 carry_over 接手合规 evidence (按新 profile.inherits_evidence_from 白名单)
A 的 evidence 不重 classify (留作 history 引用)
```

---

## 10. agent_continuity

### 10.1 二值

```rust
pub enum AgentContinuity {
    SingleSession,
    MultiSessionRelay,
}
```

MVP `external_attack_surface` stage 默认 `single_session`（evidence 来源完整性）。

### 10.2 MultiSessionRelay 下重跑 classifier

详见 Doc 1 §5.4。Doc 3 这边的语义：

```text
Stage 内 inner loop 跨 wait_message TIMEOUT 后 resume:
  - 若 producer_session == current_session → label 复用·零成本
  - 若 producer_session != current_session
    → ScopeService.classify_subject(parent) 重跑
    → 若结果 = OutOfScope → child evidence 升 DerivedFromOutOfScope
    → gate 验证 evidence_refs[*].sealed_origin_session 一致性 → 否则警告
```

---

## 11. Max repair attempts + paused_needs_user

```text
gate.allowed = false → refiner 加 repair subtask
                    → re-execute
                    → re-submit deliverable
                    → re-gate
                       ↑
                       │ N 次失败
                       ↓
                    upgrade to paused_needs_user
                    ↓
                    operation_state.state_blob 存 stage 状态
                    audit_log 写 'stage_paused_for_user' status=started
                    ↓
                    user 看 UI banner / 看 deliverable / 看 gate 失败原因
                    user 点 Approve / Manual edit / Abort
                    ↓
                    resume 从 paused_needs_user 重启 stage（不 abort 全会话）
```

**N=3** · 复用 `task_orchestrator::types::MAX_REFLECTOR_RETRIES`·不造新常量。

---

## 12. Stage rollback compensation

```text
失败 stage = audit_log status='failed' (PentestAudit 自然语义)
不需 saga
  ↓
若用户决定 rollback compensate:
   写一行 audit_role='action' action='stage_compensate' status='completed'
   detail = {
     original_run_id,
     original_audit_id,
     compensation_reason,
     repair_attempt: N
   }
   ↓
   不删除原 failed audit，留 history
```

---

## 13. 接到 Doc 1 / Doc 2 的点

| Doc 3 需要 | 来源 |
|---|---|
| `EvidenceLedger` trait | Doc 1 §4.3 |
| `EvidenceScopeLabel` 三变体 | Doc 1 §4.1 |
| `validate_relabel` 函数 | Doc 1 §4.4 |
| `evidence_classifications.producing_stage_run_id` | Doc 1 §3.2 |
| `audit_role='approval'` audit_log 行 | Doc 1 §8 |
| `read_evidence(eid, summary_level)` Tauri command | Doc 2 §3.2 |
| `EvidenceSanitizer` | Doc 2 §4.1 |
| `stream_retry` evidence_read 频率拦截 | Doc 2 §5 |
| `evidence_kinds.json` 静态配置 | Doc 1 §6.1 |
| `operation_state` cursor | Doc 1 §3.4 |

---

## 14. 与 §21 的对应关系

| Doc 3 章节 | §21 章节 |
|---|---|
| §2 Profile | §21.6.2 (Operation + IntentConstraint) |
| §3 DAG | §5.2 (Codex 原) + §21.6.2 (effective_tool_allow_set) |
| §4 Stage Spec | §21.7.4 (carry_over) + §5.3 (Codex 原 5 问题) |
| §5 Inner Loop | §13.4 PentAGI 风格 + §17 v0/v1 |
| §6 NlSlice | §21.6.1 |
| §7 Sprint Contract | §21.7.1 |
| §8 Gate | §21.7.6 (并发 race) + §21.8.1 (vacuous detector) |
| §9 Stage 间 evidence | §21.7.4 |
| §10 agent_continuity | §21.6.6 |
| §11 Max repair | §21.7.3 |
| §12 Stage rollback | §21.7.3 (compensation) |

---

## 15. 实施前置依赖

Phase 1 实施 Doc 3 之前必须满足：

- **Doc 1 Phase 1 完成**（schema + EvidenceLedger trait 在线）
- **Doc 2 Phase 1 完成**（read_evidence Tauri command 在线）
- `just precommit` 切绿
- `asset-intel-hydrate-disambiguation` 切 passing
- 用户明示 §AGENTS.md §2.7 授权

---

## 16. 风险

| 风险 | 缓解 |
|---|---|
| 现有 task_orchestrator 改造破坏 PentAGI behavior | feature flag `harness.stage_mode_enabled` 默认关·与旧路径并行 |
| Sprint Contract cross-vendor LLM 调用增本 | < 10% stage 总成本·v0 fallback 同厂商 + temperature |
| Stage harness 注入太多 prompt → token 上涨 | charter 控在 1KB 内·NlSlice + IntentConstraint 不进 prompt（只 gate 用） |
| 起始 stage `external_attack_surface` MVP 实施后发现 5 个 stage 全部要改 | MVP 锁 1 stage·跑通后再扩 |
| 改 task_orchestrator/types::PlannedSubtask 导致 frontend ts-rs 类型链断 | 加新字段而非删·#[serde(default)] 保后向兼容 |

---

## 17. 不做（与 §21.3 不变量一致）

- 不实施 13 个 stage（仅 1 个 external_attack_surface）
- 不实施 6 个 profile（仅 assessment）
- 不实施 L3-L5 authz（仅 L2）
- 不实施 Harness Lab（Phase 4 才考虑）
- 不实施二阶 LLM vacuous detector（一阶 Rust 即可）
- 不引入新 crate
- 不重构 task_orchestrator 主循环（仅在 execute_single_subtask 末端加 gate 钩子）

---

## 18. 后续

- plan 会按 Doc 1 + Doc 2 + Doc 3 的设计·拆 Phase 1 实施步骤
- Phase 2 加 enumeration stage（assessment profile 下一个 stage）
- Phase 3 加 pentest profile + vuln_triage stage + L3 authz
- Phase 4 加 Harness Lab (AHE-style)

---

## 19. 状态

**Discussion Draft** · 待 user 明示 §2.7 授权 + Doc 1/2 Phase 1 完成后进入 Doc 3 Phase 1。
