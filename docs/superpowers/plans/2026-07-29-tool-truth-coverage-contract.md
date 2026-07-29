# Tool Truth 与 Coverage Contract 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 为每次 capability 执行建立 operation-frozen、逐 input、可对账的 Tool Truth receipt，并在不改变旧 operation 或用户界面的前提下，消除当前 EAS、Enumeration、positive-partial 与 Nuclei no-match 的直接假阴性。

**架构：** 在 golish-pentest-domain 中定义纯状态机，在 golish-db 中以唯一 additive migration 持久化 frozen contract、denominator、receipt、reconciliation 与 shadow assessment；producer 通过 pentest bridge 执行 begin → encrypted raw-vault seal → staged typed closeout → vault-authenticated stable snapshot → atomic reconciliation/finalize 协议。新建 operation 仍冻结为 legacy_v1，不提供 promotion；shadow_v1 只写审计，receipt_v1 才启用 fail-safe producer 投影，而 Gate 的 control_decision/coverage_grade 在本计划内始终 shadow-write。

**技术栈：** Rust 2021、sqlx/PostgreSQL、serde/serde_json、sha2、tokio、tempfile、cargo-nextest、现有 Golish evidence ledger 与 stage Gate。

---

## 非协商边界

1. 只使用 migration <code>backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql</code>；不得拆成第二个 migration，也不得占用新的时间戳。
2. schema 实施前必须停下并取得用户对 DB schema/migration 的明确批准。书面设计批准不等于 schema 执行批准。
3. operation 创建时冻结 <code>legacy_v1 | shadow_v1 | receipt_v1</code>；已有 operation 回填 <code>legacy_v1</code>，deployment default 仍为 <code>legacy_v1</code>。
4. 本计划不提供 rollout promotion API、CLI、Tauri command 或配置热切换；same-operation resume/continuation 必须保留原 contract。
5. <code>shadow_v1</code> 写 receipt、reconciliation 和 Gate assessment，但保留 legacy producer/Gate authority；<code>receipt_v1</code> 才启用 fail-safe producer projection。
6. Plan A 的 <code>control_decision</code>/<code>coverage_grade</code> 只写 DB shadow assessment；不修改 <code>frontend/</code>、<code>frontend/lib/generated/</code>、Tauri IPC、report source 或 report renderer。
7. 不把外部执行与数据库事务伪装成原子操作：外部执行必须在 transaction 外，crash gap 由 raw witness、CAS closeout 和 reconciliation 显式表达。
8. Verification campaign、Prepared Action、完整 versioned negative oracle 和 Reporting cutover 属 Plan C/D；本计划只让 Nuclei no-match fail-safe。
9. 所有 Cargo 构建、测试、clippy 前先运行 <code>just space-guard</code>。本计划只列定向命令，不授权 <code>./init.sh</code>、<code>just check</code>、<code>just test</code>、<code>just precommit</code> 或全 workspace 测试。
10. 每个 Task 的 <code>Future Commit</code> 只是未来实施时的原子提交边界；当前轮次不执行其中任何实现、migration或产品测试。当前设计/计划/状态文档按用户要求单独commit，但不push。
11. 文件bytes reconciliation与目标状态时效是两条正交authority：hash一致不等于DNS/端口/权限/业务状态仍然成立。所有consumer必须同时通过operation-frozen `EvidenceTemporalValidityPolicyV1`；expired、mixed target epoch或超max skew只能revalidate/residual/HOLD。negative/refutation TTL不得长于positive TTL。
12. 本计划的`coverage complete`只表示frozen declared denominator内完成，不证明全局attack surface或Threat Coverage充分；UI/Reporting必须使用“已声明范围内完成”，在`ThreatCoverageProfileV1`上线前全局sufficiency为`not_assessed`。

## 状态语义

<code>checked_empty</code> 不落为 receipt 基础状态，只能从以下组合派生：

~~~text
attempt_state=succeeded
landing_state=committed
observation_state=no_match
coverage_extent=complete
coverage_gap_reason=none
reconciliation_state=consistent
所有 frozen input 均属于当前 authority 且 terminal
所有 observation 均在 EvidenceTemporalValidityPolicyV1 的 valid_until、target_state_epoch 与 max-skew 内
~~~

receipt 维度固定为：

- <code>attempt_state</code>: not_started, running, succeeded, failed, outcome_unknown, exhausted, superseded
- <code>landing_state</code>: not_attempted, partial, committed, failed
- <code>observation_state</code>: found, no_match, indeterminate, not_applicable
- <code>coverage_extent</code>: none, complete, partial, sampled, template_only
- <code>coverage_gap_reason</code>: none, transport, tool_failure, parser_reject, budget_exhausted, unsupported, policy_blocked, source_unavailable
- <code>reconciliation_state</code>: pending, consistent, orphaned, superseded
- <code>security_interpretation</code>: not_assessed, signal, proof, refutation, inconclusive；但Plan A producer receipt只允许not_assessed/signal/inconclusive，proof/refutation只属于Plan C typed oracle
- <code>control_decision</code>: allow, hold
- <code>coverage_grade</code>: complete, degraded, incomplete

合法 Gate 组合只有：

| control_decision | coverage_grade | 条件 |
|---|---|---|
| allow | complete | frozen denominator 全部 complete/consistent，且temporal validity全fresh |
| allow | degraded | bounded exhaustion 已稳定收口，存在 exact residual/owner/next action，且没有 unknown、authority drift 或 evidence corruption |
| hold | degraded | 存在已解释缺口，但仍未满足可继续条件 |
| hold | incomplete | denominator 未冻结、pending、unknown、orphan、expired/mixed-epoch/skew-exceeded 或 authority drift |

<code>allow + incomplete</code> 与 <code>hold + complete</code> 必须在 Rust reducer 和 DB CHECK 双重拒绝。

## 文件结构

### 新建

- <code>backend/crates/golish-pentest-domain/src/tool_truth.rs</code> — 纯 ontology、状态组合校验、checked-empty 派生与 Gate grade reducer。
- <code>backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql</code> — 唯一 schema migration。
- <code>backend/crates/golish-db/src/repo/tool_truth_rollout.rs</code> — 只读/锁定 deployment default；本计划不暴露 setter。
- <code>backend/crates/golish-db/src/repo/capability_execution_receipts.rs</code> — denominator、receipt、raw witness ref、closeout、reconciliation、shadow assessment repo。
- <code>backend/crates/golish-db/tests/capability_execution_receipts.rs</code> — migration/repository contract integration tests。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/tool_truth.rs</code> — producer lifecycle、typed landing和request governor。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/raw_witness_vault.rs</code> — per-operation envelope encryption、vault-owned sealed ref token、访问审计与retention/crypto-erasure port。
- <code>backend/crates/golish-agent-kit/src/harness/tool_truth.rs</code> — frozen denominator DTO 构建和 shadow Gate assessment orchestration。
- <code>backend/crates/golish-agent-app/src/ai/db_bridge/tool_truth.rs</code> — DbRepoProvider 的 Tool Truth 持久化实现。

### 修改

- <code>backend/crates/golish-pentest-domain/Cargo.toml</code> — 复用 workspace thiserror dependency。
- <code>backend/crates/golish-pentest-domain/src/lib.rs</code> — 导出 Tool Truth 类型。
- <code>backend/crates/golish-db/src/repo/mod.rs</code> — 导出新 repo。
- <code>backend/crates/golish-db/src/repo/operation_state.rs</code> — 在 operation row/insert/get 中冻结 tool_truth_contract。
- <code>backend/crates/golish-db/src/repo/runtime_memory_tx.rs</code> — operation 创建/fork 时锁定默认值或继承 source contract。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs</code> — 注册内部 helper module。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs</code> — EAS/WhatWeb empty stdout fail-safe。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/enum_preflight_web_origins.rs</code> — transport preflight 不再封闭四个 content axis。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs</code> — observation found 与 coverage partial 分离。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs</code> — positive sibling 不再掩盖 incomplete siblings。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/landing.rs</code> — stage Nuclei no-match 非 terminal。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs</code> — 传递 frozen contract/receipt closeout。
- <code>backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs</code> — exact replay no-match 默认 inconclusive。
- <code>backend/crates/golish-agent-kit/src/harness/mod.rs</code> — 导出 shadow evaluator。
- <code>backend/crates/golish-agent-kit/src/harness/org_gate.rs</code> — 移除 receipt 模式的 preflight 四轴权威并写 shadow assessment。
- <code>backend/crates/golish-agent-kit/src/db_traits/types.rs</code> — denominator/receipt/assessment 的无 sqlx DTO。
- <code>backend/crates/golish-agent-kit/src/db_traits/repo.rs</code> — 添加 Tool Truth repo trait 方法。
- <code>backend/crates/golish-agent-kit/src/tool_executors/security.rs</code> — 修正 Enumeration worklist/methodology 文案。
- <code>backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs</code> — 注册 Tool Truth bridge。
- <code>backend/crates/golish-agent-app/src/ai/db_bridge/orchestration.rs</code> — generic wave dispatch 前 seal denominator。
- <code>backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs</code> — Company stage team dispatch 前 seal per-unit denominator。
- <code>backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs</code> — TargetIntel receipt_v1只读current attempt exact receipts。
- <code>backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs</code> — TargetIntel current-attempt authority/receipt query。
- <code>backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs</code> — 旧source terminal row降为legacy/shadow projection。
- <code>backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs</code> — 把 trusted stage_execution_id 传入 generic wave sealer。
- <code>backend/crates/golish-recon-app/src/intel_providers.rs</code> — passive provider逐input begin/witness/close/reconcile接线。
- <code>resources/harness/stages/enumeration/spec.json</code> — 删除 preflight 可关闭四轴的契约。
- <code>resources/harness/stages/enumeration/methodology.md</code> — 把 preflight blocked 改为 transport prerequisite gap。
- <code>resources/harness/stages/vuln_triage/methodology.md</code> — 声明 scanner no-match 不等于 checked-empty/refutation。
- <code>docs/modules/backend/golish-pentest-domain.md</code>
- <code>docs/modules/backend/golish-db.md</code>
- <code>docs/modules/backend/golish-db/repo.md</code>
- <code>docs/modules/backend/golish-pentest-app/pentest_bridge.md</code>
- <code>docs/modules/backend/golish-agent-kit/harness.md</code>
- <code>docs/modules/backend/golish-agent-kit/db_traits.md</code>
- <code>docs/modules/backend/golish-agent-app/ai.md</code>
- <code>docs/modules/backend/golish-agent-runtime/agentic_loop.md</code>
- <code>docs/modules/backend/golish-recon-app.md</code>
- <code>docs/modules/INDEX.md</code>

### 明确不修改

- <code>frontend/</code>
- <code>frontend/lib/generated/</code>
- <code>backend/crates/golish/src/commands_registry.rs</code>
- report source、report renderer、Tauri command 与 API wrapper
- Plan B/C/D 的 Registry、Campaign、Prepared Action、oracle schema

---

## Task 1：建立纯 Tool Truth 状态机

**文件：**

- 创建：<code>backend/crates/golish-pentest-domain/src/tool_truth.rs</code>
- 修改：<code>backend/crates/golish-pentest-domain/Cargo.toml</code>
- 修改：<code>backend/crates/golish-pentest-domain/src/lib.rs</code>
- 测试：<code>backend/crates/golish-pentest-domain/src/tool_truth.rs</code> 内联单元测试

### Step 1：写 RED 状态组合测试

在新文件先写这些测试；测试直接固定跨层共享的类型名，后续任务不得改名：

~~~rust
#[cfg(test)]
mod tests {
    use super::*;

    fn complete_fact(input_key: &str) -> ReceiptCoverageFact {
        ReceiptCoverageFact {
            input_key: input_key.to_string(),
            attempt_state: AttemptState::Succeeded,
            landing_state: LandingState::Committed,
            observation_state: ObservationState::NoMatch,
            coverage_extent: CoverageExtent::Complete,
            coverage_gap_reason: CoverageGapReason::None,
            reconciliation_state: ReconciliationState::Consistent,
            security_interpretation: SecurityInterpretation::NotAssessed,
            authority_current: true,
            residual: None,
        }
    }

    fn noncritical_degraded_policy() -> CoverageContinuationPolicyV1 {
        CoverageContinuationPolicyV1 {
            mandatory_input_keys: BTreeSet::new(),
            max_degraded_input_count: 1,
            require_human_risk_acceptance: false,
            risk_acceptance_receipt_id: None,
        }
    }

    #[test]
    fn checked_empty_requires_every_axis_and_all_frozen_inputs() {
        let fact = complete_fact("origin:https://app.example.test");
        assert!(fact.is_checked_empty());

        let mut partial = fact.clone();
        partial.coverage_extent = CoverageExtent::Partial;
        partial.coverage_gap_reason = CoverageGapReason::ParserReject;
        assert!(!partial.is_checked_empty());

        let assessment = reduce_coverage(&CoverageReductionInput {
            denominator_sealed: true,
            temporal_validity_status: TemporalValidityStatus::Fresh,
            expected_input_keys: vec![fact.input_key.clone(), "origin:https://api.example.test".into()],
            receipt_facts: vec![fact],
            continuation_policy: noncritical_degraded_policy(),
        });
        assert_eq!(assessment.control_decision, ControlDecision::Hold);
        assert_eq!(assessment.coverage_grade, CoverageGrade::Incomplete);
    }

    #[test]
    fn positive_partial_is_not_complete() {
        let fact = ReceiptCoverageFact {
            input_key: "script:https://app.example.test/a.js".into(),
            attempt_state: AttemptState::Succeeded,
            landing_state: LandingState::Partial,
            observation_state: ObservationState::Found,
            coverage_extent: CoverageExtent::Partial,
            coverage_gap_reason: CoverageGapReason::ParserReject,
            reconciliation_state: ReconciliationState::Orphaned,
            security_interpretation: SecurityInterpretation::Signal,
            authority_current: true,
            residual: None,
        };
        let assessment = reduce_coverage(&CoverageReductionInput {
            denominator_sealed: true,
            temporal_validity_status: TemporalValidityStatus::Fresh,
            expected_input_keys: vec![fact.input_key.clone()],
            receipt_facts: vec![fact],
            continuation_policy: noncritical_degraded_policy(),
        });
        assert_eq!(assessment.control_decision, ControlDecision::Hold);
        assert_eq!(assessment.coverage_grade, CoverageGrade::Incomplete);
    }

    #[test]
    fn consistent_partial_without_exact_residual_is_incomplete() {
        let mut fact = complete_fact("origin:https://partial.example.test");
        fact.coverage_extent = CoverageExtent::Partial;
        fact.coverage_gap_reason = CoverageGapReason::BudgetExhausted;
        fact.security_interpretation = SecurityInterpretation::Inconclusive;
        fact.residual = None;

        let assessment = reduce_coverage(&CoverageReductionInput {
            denominator_sealed: true,
            temporal_validity_status: TemporalValidityStatus::Fresh,
            expected_input_keys: vec![fact.input_key.clone()],
            receipt_facts: vec![fact],
            continuation_policy: noncritical_degraded_policy(),
        });
        assert_eq!(assessment.control_decision, ControlDecision::Hold);
        assert_eq!(assessment.coverage_grade, CoverageGrade::Incomplete);
    }

    #[test]
    fn illegal_terminal_tuples_and_producer_verdicts_fail_closed() {
        for (attempt_state, landing_state) in [
            (AttemptState::Failed, LandingState::Failed),
            (AttemptState::Exhausted, LandingState::Committed),
        ] {
            let mut fact = complete_fact("origin:https://invalid.example.test");
            fact.attempt_state = attempt_state;
            fact.landing_state = landing_state;
            assert!(fact.validate_for_producer().is_err());
            let assessment = reduce_coverage(&CoverageReductionInput {
                denominator_sealed: true,
                temporal_validity_status: TemporalValidityStatus::Fresh,
                expected_input_keys: vec![fact.input_key.clone()],
                receipt_facts: vec![fact],
                continuation_policy: noncritical_degraded_policy(),
            });
            assert_eq!(assessment.control_decision, ControlDecision::Hold);
            assert_eq!(assessment.coverage_grade, CoverageGrade::Incomplete);
        }

        let mut forged = complete_fact("origin:https://oracle.example.test");
        forged.security_interpretation = SecurityInterpretation::Proof;
        assert_eq!(
            forged.validate_for_producer().expect_err("producer cannot prove").code(),
            "TOOL_TRUTH_ORACLE_AUTHORITY_REQUIRED",
        );
    }

    #[test]
    fn stable_noncritical_exhaustion_can_be_allow_degraded() {
        let fact = ReceiptCoverageFact {
            input_key: "origin:https://blocked.example.test".into(),
            attempt_state: AttemptState::Exhausted,
            landing_state: LandingState::Committed,
            observation_state: ObservationState::Indeterminate,
            coverage_extent: CoverageExtent::Partial,
            coverage_gap_reason: CoverageGapReason::Transport,
            reconciliation_state: ReconciliationState::Consistent,
            security_interpretation: SecurityInterpretation::Inconclusive,
            authority_current: true,
            residual: Some(CoverageResidual {
                reason_code: "ENUM_TRANSPORT_EXHAUSTED".into(),
                owner: "enumeration".into(),
                affected_input_keys: vec!["origin:https://blocked.example.test".into()],
                next_action: "route through an approved alternate transport".into(),
            }),
        };
        let assessment = reduce_coverage(&CoverageReductionInput {
            denominator_sealed: true,
            temporal_validity_status: TemporalValidityStatus::Fresh,
            expected_input_keys: vec![fact.input_key.clone()],
            receipt_facts: vec![fact],
            continuation_policy: noncritical_degraded_policy(),
        });
        assert_eq!(assessment.control_decision, ControlDecision::Allow);
        assert_eq!(assessment.coverage_grade, CoverageGrade::Degraded);
    }

    #[test]
    fn critical_or_unaccepted_exhaustion_holds_even_when_stable() {
        let mut fact = complete_fact("technique:authz-boundary");
        fact.attempt_state = AttemptState::Exhausted;
        fact.coverage_extent = CoverageExtent::Partial;
        fact.coverage_gap_reason = CoverageGapReason::BudgetExhausted;
        fact.observation_state = ObservationState::Indeterminate;
        fact.security_interpretation = SecurityInterpretation::Inconclusive;
        fact.residual = Some(CoverageResidual::exact_for(&fact.input_key));
        let assessment = reduce_coverage(&CoverageReductionInput {
            denominator_sealed: true,
            temporal_validity_status: TemporalValidityStatus::Fresh,
            expected_input_keys: vec![fact.input_key.clone()],
            receipt_facts: vec![fact],
            continuation_policy: CoverageContinuationPolicyV1 {
                mandatory_input_keys: BTreeSet::from(["technique:authz-boundary".into()]),
                max_degraded_input_count: 0,
                require_human_risk_acceptance: true,
                risk_acceptance_receipt_id: None,
            },
        });
        assert_eq!(assessment.control_decision, ControlDecision::Hold);
        assert_eq!(assessment.coverage_grade, CoverageGrade::Degraded);
    }

    #[test]
    fn byte_consistent_but_temporally_expired_fact_holds() {
        let fact = complete_fact("authz:user-a:resource-1");
        let assessment = reduce_coverage(&CoverageReductionInput {
            denominator_sealed: true,
            temporal_validity_status: TemporalValidityStatus::Expired,
            expected_input_keys: vec![fact.input_key.clone()],
            receipt_facts: vec![fact],
            continuation_policy: noncritical_degraded_policy(),
        });
        assert_eq!(assessment.control_decision, ControlDecision::Hold);
        assert_eq!(assessment.coverage_grade, CoverageGrade::Incomplete);
    }

    #[test]
    fn illegal_control_grade_pairs_are_rejected() {
        assert!(ToolTruthGateAssessment::new(
            ControlDecision::Allow,
            CoverageGrade::Incomplete,
            vec![],
        )
        .is_err());
        assert!(ToolTruthGateAssessment::new(
            ControlDecision::Hold,
            CoverageGrade::Complete,
            vec![],
        )
        .is_err());
        assert!(ToolTruthGateAssessment::new(
            ControlDecision::Allow,
            CoverageGrade::Degraded,
            vec![],
        )
        .is_err());
        assert!(ToolTruthGateAssessment::new(
            ControlDecision::Hold,
            CoverageGrade::Degraded,
            vec![],
        )
        .is_err());
    }
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-domain -E 'test(checked_empty_requires_every_axis_and_all_frozen_inputs) | test(positive_partial_is_not_complete) | test(consistent_partial_without_exact_residual_is_incomplete) | test(stable_noncritical_exhaustion_can_be_allow_degraded) | test(critical_or_unaccepted_exhaustion_holds_even_when_stable) | test(byte_consistent_but_temporally_expired_fact_holds) | test(illegal_control_grade_pairs_are_rejected)')
~~~

**Expected:** 编译失败，错误明确指出 <code>ReceiptCoverageFact</code>、状态枚举与 <code>reduce_coverage</code> 尚不存在；不是现有测试失败。

### Step 3：实现最小纯状态机

先在 domain crate dependencies 中复用 workspace error type：

~~~toml
thiserror = { workspace = true }
~~~

实现并从 <code>lib.rs</code> 导出以下类型；所有 enum 使用 <code>#[serde(rename_all = "snake_case")]</code>：

~~~rust
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTruthContract {
    LegacyV1,
    ShadowV1,
    ReceiptV1,
}

impl ToolTruthContract {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyV1 => "legacy_v1",
            Self::ShadowV1 => "shadow_v1",
            Self::ReceiptV1 => "receipt_v1",
        }
    }

    pub const fn writes_receipts(self) -> bool {
        !matches!(self, Self::LegacyV1)
    }

    pub const fn enforces_fail_safe_projection(self) -> bool {
        matches!(self, Self::ReceiptV1)
    }
}

impl TryFrom<&str> for ToolTruthContract {
    type Error = ToolTruthValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "legacy_v1" => Ok(Self::LegacyV1),
            "shadow_v1" => Ok(Self::ShadowV1),
            "receipt_v1" => Ok(Self::ReceiptV1),
            other => Err(ToolTruthValidationError::UnknownContract(other.to_string())),
        }
    }
}

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ToolTruthValidationError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(ToolTruthValidationError::UnknownStatus {
                        axis: stringify!($name),
                        value: other.to_string(),
                    }),
                }
            }
        }
    };
}

string_enum!(AttemptState {
    NotStarted => "not_started",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
    OutcomeUnknown => "outcome_unknown",
    Exhausted => "exhausted",
    Superseded => "superseded",
});
string_enum!(LandingState {
    NotAttempted => "not_attempted",
    Partial => "partial",
    Committed => "committed",
    Failed => "failed",
});
string_enum!(ObservationState {
    Found => "found",
    NoMatch => "no_match",
    Indeterminate => "indeterminate",
    NotApplicable => "not_applicable",
});
string_enum!(CoverageExtent {
    None => "none",
    Complete => "complete",
    Partial => "partial",
    Sampled => "sampled",
    TemplateOnly => "template_only",
});
string_enum!(CoverageGapReason {
    None => "none",
    Transport => "transport",
    ToolFailure => "tool_failure",
    ParserReject => "parser_reject",
    BudgetExhausted => "budget_exhausted",
    Unsupported => "unsupported",
    PolicyBlocked => "policy_blocked",
    SourceUnavailable => "source_unavailable",
});
string_enum!(ReconciliationState {
    Pending => "pending",
    Consistent => "consistent",
    Orphaned => "orphaned",
    Superseded => "superseded",
});
string_enum!(SecurityInterpretation {
    NotAssessed => "not_assessed",
    Signal => "signal",
    Proof => "proof",
    Refutation => "refutation",
    Inconclusive => "inconclusive",
});
string_enum!(ControlDecision { Allow => "allow", Hold => "hold" });
string_enum!(CoverageGrade {
    Complete => "complete",
    Degraded => "degraded",
    Incomplete => "incomplete",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageResidual {
    pub reason_code: String,
    pub owner: String,
    pub affected_input_keys: Vec<String>,
    pub next_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptCoverageFact {
    pub input_key: String,
    pub attempt_state: AttemptState,
    pub landing_state: LandingState,
    pub observation_state: ObservationState,
    pub coverage_extent: CoverageExtent,
    pub coverage_gap_reason: CoverageGapReason,
    pub reconciliation_state: ReconciliationState,
    pub security_interpretation: SecurityInterpretation,
    pub authority_current: bool,
    pub residual: Option<CoverageResidual>,
}

impl ReceiptCoverageFact {
    pub fn validate_for_producer(&self) -> Result<(), ToolTruthValidationError> {
        if matches!(
            self.security_interpretation,
            SecurityInterpretation::Proof | SecurityInterpretation::Refutation
        ) {
            return Err(ToolTruthValidationError::OracleAuthorityRequired);
        }
        if self.coverage_extent == CoverageExtent::Complete
            && !(self.authority_current
                && self.attempt_state == AttemptState::Succeeded
                && self.landing_state == LandingState::Committed
                && matches!(
                    self.observation_state,
                    ObservationState::Found | ObservationState::NoMatch
                )
                && self.coverage_gap_reason == CoverageGapReason::None
                && self.reconciliation_state == ReconciliationState::Consistent)
        {
            return Err(ToolTruthValidationError::IllegalReceiptTuple);
        }
        Ok(())
    }

    pub fn is_checked_empty(&self) -> bool {
        self.validate_for_producer().is_ok()
            && self.authority_current
            && self.attempt_state == AttemptState::Succeeded
            && self.landing_state == LandingState::Committed
            && self.observation_state == ObservationState::NoMatch
            && self.coverage_extent == CoverageExtent::Complete
            && self.coverage_gap_reason == CoverageGapReason::None
            && self.reconciliation_state == ReconciliationState::Consistent
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageReductionInput {
    pub denominator_sealed: bool,
    pub temporal_validity_status: TemporalValidityStatus,
    pub expected_input_keys: Vec<String>,
    pub receipt_facts: Vec<ReceiptCoverageFact>,
    pub continuation_policy: CoverageContinuationPolicyV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageContinuationPolicyV1 {
    pub mandatory_input_keys: BTreeSet<String>,
    pub max_degraded_input_count: usize,
    pub require_human_risk_acceptance: bool,
    pub risk_acceptance_receipt_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalValidityStatus {
    Fresh,
    Expired,
    MixedEpoch,
    SkewExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTruthGateAssessment {
    pub control_decision: ControlDecision,
    pub coverage_grade: CoverageGrade,
    pub residuals: Vec<CoverageResidual>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ToolTruthValidationError {
    #[error("unknown tool-truth contract: {0}")]
    UnknownContract(String),
    #[error("unknown tool-truth status for {axis}: {value}")]
    UnknownStatus { axis: &'static str, value: String },
    #[error("illegal tool-truth gate pair: {control}/{grade}")]
    IllegalGatePair { control: String, grade: String },
    #[error("degraded requires an exact residual for every non-complete input")]
    DegradedRequiresExactResidual,
    #[error("illegal capability receipt status tuple")]
    IllegalReceiptTuple,
    #[error("producer proof/refutation requires a versioned oracle authority")]
    OracleAuthorityRequired,
}

impl ToolTruthValidationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownContract(_) => "TOOL_TRUTH_CONTRACT_INVALID",
            Self::UnknownStatus { .. } => "TOOL_TRUTH_STATUS_INVALID",
            Self::IllegalGatePair { .. } => "TOOL_TRUTH_GATE_PAIR_INVALID",
            Self::DegradedRequiresExactResidual => "TOOL_TRUTH_RESIDUAL_REQUIRED",
            Self::IllegalReceiptTuple => "TOOL_TRUTH_STATUS_TUPLE_INVALID",
            Self::OracleAuthorityRequired => "TOOL_TRUTH_ORACLE_AUTHORITY_REQUIRED",
        }
    }
}

impl ToolTruthGateAssessment {
    pub fn new(
        control_decision: ControlDecision,
        coverage_grade: CoverageGrade,
        residuals: Vec<CoverageResidual>,
    ) -> Result<Self, ToolTruthValidationError> {
        if matches!(
            (control_decision, coverage_grade),
            (ControlDecision::Allow, CoverageGrade::Incomplete)
                | (ControlDecision::Hold, CoverageGrade::Complete)
        ) {
            return Err(ToolTruthValidationError::IllegalGatePair {
                control: control_decision.as_str().to_string(),
                grade: coverage_grade.as_str().to_string(),
            });
        }
        if coverage_grade == CoverageGrade::Degraded && residuals.is_empty() {
            return Err(ToolTruthValidationError::DegradedRequiresExactResidual);
        }
        Ok(Self { control_decision, coverage_grade, residuals })
    }
}

pub fn reduce_coverage(input: &CoverageReductionInput) -> ToolTruthGateAssessment {
    if !input.denominator_sealed
        || input.temporal_validity_status != TemporalValidityStatus::Fresh
    {
        return ToolTruthGateAssessment::new(
            ControlDecision::Hold,
            CoverageGrade::Incomplete,
            vec![],
        )
        .expect("hold/incomplete is legal");
    }
    let expected = input.expected_input_keys.iter().cloned().collect::<BTreeSet<_>>();
    let mut current = BTreeMap::new();
    let mut duplicate_current_input = false;
    for fact in input.receipt_facts.iter().filter(|fact| fact.authority_current) {
        duplicate_current_input |= current.insert(fact.input_key.clone(), fact).is_some();
    }
    if expected.is_empty()
        || duplicate_current_input
        || expected.iter().any(|key| !current.contains_key(key))
        || current.keys().any(|key| !expected.contains(key))
        || current.values().any(|fact| {
            fact.validate_for_producer().is_err()
                ||
            matches!(
                fact.attempt_state,
                AttemptState::NotStarted
                    | AttemptState::Running
                    | AttemptState::OutcomeUnknown
                    | AttemptState::Superseded
            ) || matches!(
                fact.reconciliation_state,
                ReconciliationState::Pending
                    | ReconciliationState::Orphaned
                    | ReconciliationState::Superseded
            )
        })
    {
        return ToolTruthGateAssessment::new(
            ControlDecision::Hold,
            CoverageGrade::Incomplete,
            vec![],
        )
        .expect("hold/incomplete is legal");
    }
    if expected.iter().all(|key| {
        let fact = current[key];
        fact.coverage_extent == CoverageExtent::Complete
            && fact.coverage_gap_reason == CoverageGapReason::None
            && fact.reconciliation_state == ReconciliationState::Consistent
    }) {
        return ToolTruthGateAssessment::new(
            ControlDecision::Allow,
            CoverageGrade::Complete,
            vec![],
        )
        .expect("allow/complete is legal");
    }
    let is_complete = |fact: &ReceiptCoverageFact| {
        fact.coverage_extent == CoverageExtent::Complete
            && fact.coverage_gap_reason == CoverageGapReason::None
            && fact.reconciliation_state == ReconciliationState::Consistent
    };
    let residual_is_exact = |fact: &ReceiptCoverageFact| {
        fact.residual.as_ref().is_some_and(|residual| {
            !residual.reason_code.trim().is_empty()
                && !residual.owner.trim().is_empty()
                && !residual.next_action.trim().is_empty()
                && residual.affected_input_keys == vec![fact.input_key.clone()]
        })
    };
    let non_complete = expected
        .iter()
        .map(|key| current[key])
        .filter(|fact| !is_complete(fact))
        .collect::<Vec<_>>();
    if non_complete.iter().any(|fact| !residual_is_exact(fact)) {
        return ToolTruthGateAssessment::new(
            ControlDecision::Hold,
            CoverageGrade::Incomplete,
            vec![],
        )
        .expect("unexplained partial truth is incomplete");
    }
    let residuals = non_complete
        .iter()
        .filter_map(|fact| fact.residual.clone())
        .collect::<Vec<_>>();
    let is_stable_exhaustion = |fact: &ReceiptCoverageFact| {
        fact.attempt_state == AttemptState::Exhausted
            && fact.reconciliation_state == ReconciliationState::Consistent
            && residual_is_exact(fact)
    };
    let stable_exhaustion = expected.iter().all(|key| {
        let fact = current[key];
        is_complete(fact) || is_stable_exhaustion(fact)
    }) && expected.iter().any(|key| is_stable_exhaustion(current[key]));
    let degraded_keys = non_complete
        .iter()
        .map(|fact| fact.input_key.clone())
        .collect::<BTreeSet<_>>();
    let continuation_allowed = stable_exhaustion
        && degraded_keys.len() <= input.continuation_policy.max_degraded_input_count
        && degraded_keys.is_disjoint(&input.continuation_policy.mandatory_input_keys)
        && (!input.continuation_policy.require_human_risk_acceptance
            || input.continuation_policy.risk_acceptance_receipt_id.is_some());
    ToolTruthGateAssessment::new(
        if continuation_allowed {
            ControlDecision::Allow
        } else {
            ControlDecision::Hold
        },
        CoverageGrade::Degraded,
        residuals,
    )
    .expect("degraded accepts allow or hold")
}
~~~

### Step 4：运行 GREEN

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-domain -E 'test(checked_empty_requires_every_axis_and_all_frozen_inputs) | test(positive_partial_is_not_complete) | test(consistent_partial_without_exact_residual_is_incomplete) | test(illegal_terminal_tuples_and_producer_verdicts_fail_closed) | test(stable_noncritical_exhaustion_can_be_allow_degraded) | test(critical_or_unaccepted_exhaustion_holds_even_when_stable) | test(byte_consistent_but_temporally_expired_fact_holds) | test(illegal_control_grade_pairs_are_rejected)')
~~~

**Expected:** 8 tests passed，exit code 0；failed/exhausted/landing-failed不能伪装成complete，producer不能直接写proof/refutation，任何没有逐input exact residual的partial都只能是hold/incomplete；稳定耗尽也必须经过operation-frozen continuation policy，关键轴、超比例gap或缺risk acceptance继续HOLD；bytes consistent但observation过期同样HOLD。

### Step 5：Future Commit

~~~bash
git add backend/crates/golish-pentest-domain/Cargo.toml backend/crates/golish-pentest-domain/src/tool_truth.rs backend/crates/golish-pentest-domain/src/lib.rs
git commit -m "feat(tool-truth): add coverage status ontology"
~~~

---

## Schema 授权暂停点

Task 1 完成后停止。向用户展示唯一 migration 路径、表清单、operation backfill/default 以及 rollback 边界，并明确询问是否批准修改 DB schema/migration。

**Expected:** 在用户明确回复批准前，不创建 migration、不执行 Task 2 及之后任何代码任务、不运行 migration test。批准只覆盖本计划列出的 additive schema；不覆盖 promotion、真实外部扫描或全量测试。

---

## Task 2：冻结 operation contract 并创建 additive schema

**文件：**

- 创建：<code>backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql</code>
- 创建：<code>backend/crates/golish-db/src/repo/tool_truth_rollout.rs</code>
- 修改：<code>backend/crates/golish-db/src/repo/mod.rs</code>
- 修改：<code>backend/crates/golish-db/src/repo/operation_state.rs</code>
- 修改：<code>backend/crates/golish-db/src/repo/runtime_memory_tx.rs</code>
- 测试：<code>backend/crates/golish-db/src/repo/operation_state.rs</code> 内联 SQL/contract tests
- 创建：<code>backend/crates/golish-db/tests/capability_execution_receipts.rs</code>（Task 3 起继续扩展同一文件）

### Step 1：写 RED operation-freeze 测试

加入测试，固定 default、source inheritance 和 SQL column：

~~~rust
#[test]
fn operation_insert_freezes_tool_truth_contract() {
    assert!(INSERT_OPERATION_SQL.contains("tool_truth_contract"));
    assert!(INSERT_OPERATION_WITH_EXECUTOR_SQL.contains("tool_truth_contract"));
    assert!(OPERATION_STATE_ROW_COLUMNS.contains("tool_truth_contract"));
}

#[test]
fn tool_truth_contract_does_not_fallback_on_unknown_value() {
    let error = parse_tool_truth_contract("future_contract")
        .expect_err("unknown persisted contract must fail closed");
    assert_eq!(error.to_string(), "unknown tool-truth contract: future_contract");
}

#[test]
fn runtime_operation_creation_locks_tool_truth_rollout() {
    let source = include_str!("runtime_memory_tx.rs");
    assert!(source.contains("tool_truth_rollout::get_for_share"));
    assert!(source.contains("source_tool_truth_contract"));
}

#[tokio::test]
async fn persisted_operation_tool_truth_contract_is_db_immutable() {
    let fixture = ToolTruthDbFixture::legacy_operation().await;
    let error = sqlx::query(
        "UPDATE operation_state SET tool_truth_contract='receipt_v1' WHERE operation_id=$1",
    )
    .bind(fixture.operation_id)
    .execute(&fixture.pool)
    .await
    .expect_err("operation-frozen contract must reject direct SQL UPDATE");
    assert!(error.to_string().contains("operation_tool_truth_contract_immutable"));
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-db -E 'test(operation_insert_freezes_tool_truth_contract) | test(tool_truth_contract_does_not_fallback_on_unknown_value) | test(runtime_operation_creation_locks_tool_truth_rollout) | test(persisted_operation_tool_truth_contract_is_db_immutable)')
~~~

**Expected:** 4 tests fail because column、parser、immutable trigger 和 rollout repo 尚未存在。

### Step 3：写唯一 migration

migration 使用 text + CHECK，避免增加不可回滚 Postgres enum；核心 DDL 必须等价于：

~~~sql
CREATE TABLE tool_truth_rollout (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    new_operation_contract TEXT NOT NULL
        CHECK (new_operation_contract IN ('legacy_v1', 'shadow_v1', 'receipt_v1')),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

INSERT INTO tool_truth_rollout (singleton, new_operation_contract)
VALUES (TRUE, 'legacy_v1')
ON CONFLICT (singleton) DO NOTHING;

ALTER TABLE operation_state
    ADD COLUMN tool_truth_contract TEXT;

UPDATE operation_state
SET tool_truth_contract = 'legacy_v1'
WHERE tool_truth_contract IS NULL;

ALTER TABLE operation_state
    ALTER COLUMN tool_truth_contract SET NOT NULL,
    ALTER COLUMN tool_truth_contract SET DEFAULT 'legacy_v1',
    ADD CONSTRAINT operation_state_tool_truth_contract_check
        CHECK (tool_truth_contract IN ('legacy_v1', 'shadow_v1', 'receipt_v1'));

CREATE FUNCTION reject_operation_tool_truth_contract_update()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tool_truth_contract IS DISTINCT FROM OLD.tool_truth_contract THEN
        RAISE EXCEPTION 'operation_tool_truth_contract_immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_state_tool_truth_contract_immutable
BEFORE UPDATE OF tool_truth_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION reject_operation_tool_truth_contract_update();

CREATE TABLE tool_truth_revalidation_dispatch_policies (
    operation_id UUID PRIMARY KEY REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    policy_contract_version TEXT NOT NULL DEFAULT 'tool_truth_revalidation_dispatch.v1'
        CHECK (policy_contract_version='tool_truth_revalidation_dispatch.v1'),
    mode TEXT NOT NULL DEFAULT 'manual_only'
        CHECK (mode IN ('manual_only','auto_passive_t0_t1')),
    max_automatic_risk_tier TEXT NOT NULL DEFAULT 't0'
        CHECK (max_automatic_risk_tier IN ('t0','t1')),
    policy_hash TEXT NOT NULL CHECK (length(policy_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE tool_truth_revalidation_dispatch_heads (
    operation_id UUID PRIMARY KEY REFERENCES tool_truth_revalidation_dispatch_policies(operation_id) ON DELETE RESTRICT,
    dispatch_held BOOLEAN NOT NULL DEFAULT TRUE,
    dispatch_generation BIGINT NOT NULL DEFAULT 0 CHECK (dispatch_generation >= 0),
    reason_code TEXT NOT NULL DEFAULT 'initial_hold' CHECK (BTRIM(reason_code) <> ''),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE tool_truth_revalidation_dispatch_events (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES tool_truth_revalidation_dispatch_policies(operation_id) ON DELETE RESTRICT,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal >= 0),
    event_kind TEXT NOT NULL CHECK (event_kind IN ('hold','release')),
    expected_generation BIGINT NOT NULL CHECK (expected_generation >= 0),
    new_generation BIGINT NOT NULL CHECK (new_generation > expected_generation),
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code) <> ''),
    event_hash TEXT NOT NULL CHECK (length(event_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,event_ordinal),
    UNIQUE(operation_id,new_generation)
);

CREATE TABLE coverage_denominators (
    id UUID PRIMARY KEY,
    stable_seal_request_id UUID NOT NULL,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    stage_execution_id UUID NOT NULL REFERENCES stage_runs(id) ON DELETE RESTRICT,
    stage_kind TEXT NOT NULL,
    stage_asset_wave_id UUID REFERENCES stage_asset_waves(id),
    unit_id UUID REFERENCES stage_run_units(id) ON DELETE RESTRICT,
    denominator_kind TEXT NOT NULL DEFAULT 'root'
        CHECK (denominator_kind IN ('root','derived_child')),
    derived_ordinal INTEGER,
    attempt_epoch TIMESTAMPTZ NOT NULL,
    contract TEXT NOT NULL
        CHECK (contract IN ('legacy_v1', 'shadow_v1', 'receipt_v1')),
    authority_hash TEXT NOT NULL CHECK (length(authority_hash) = 64),
    input_manifest_hash TEXT NOT NULL CHECK (length(input_manifest_hash) = 64),
    expected_input_count INTEGER NOT NULL CHECK (expected_input_count > 0),
    sealed_at TIMESTAMPTZ,
    UNIQUE(operation_id,stable_seal_request_id)
);

CREATE TABLE coverage_denominator_items (
    id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    input_key TEXT NOT NULL,
    target_id UUID,
    exact_asset TEXT NOT NULL,
    technique TEXT NOT NULL,
    expected_capability TEXT NOT NULL,
    item_hash TEXT NOT NULL CHECK (length(item_hash) = 64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE (denominator_id, input_key),
    UNIQUE (denominator_id, item_hash)
);

CREATE TABLE capability_execution_destination_policies (
    id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    stage_execution_id UUID NOT NULL REFERENCES stage_runs(id) ON DELETE RESTRICT,
    capability TEXT NOT NULL CHECK (BTRIM(capability) <> ''),
    attempt_epoch TIMESTAMPTZ NOT NULL,
    policy_contract_version TEXT NOT NULL DEFAULT 'tool_execution_destination.v1'
        CHECK (policy_contract_version='tool_execution_destination.v1'),
    execution_backend TEXT NOT NULL CHECK (
        execution_backend IN ('host_pinned_http','sandboxed_cli','fixed_provider_transport','none_blocked')
    ),
    governance_status TEXT NOT NULL CHECK (
        governance_status IN ('enforced','shadow_observed_uncontrolled','policy_blocked')
    ),
    redirect_mode TEXT NOT NULL CHECK (redirect_mode IN ('deny','exact_same_origin_allowlist')),
    max_redirect_hops INTEGER NOT NULL CHECK (max_redirect_hops >= 0),
    secondary_fetch_mode TEXT NOT NULL CHECK (secondary_fetch_mode='deny'),
    proxy_mode TEXT NOT NULL CHECK (proxy_mode='none'),
    tls_policy_hash TEXT NOT NULL CHECK (length(tls_policy_hash)=64),
    prohibited_range_policy_hash TEXT NOT NULL CHECK (length(prohibited_range_policy_hash)=64),
    scope_snapshot_id UUID NOT NULL,
    scope_snapshot_hash TEXT NOT NULL CHECK (length(scope_snapshot_hash)=64),
    destination_member_count BIGINT NOT NULL CHECK (destination_member_count >= 0),
    destination_member_set_hash TEXT NOT NULL CHECK (length(destination_member_set_hash)=64),
    policy_hash TEXT NOT NULL CHECK (length(policy_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(denominator_id,capability,attempt_epoch,policy_hash),
    UNIQUE(id,policy_hash,governance_status,denominator_id,operation_id,organization_id,stage_execution_id,capability,attempt_epoch),
    CHECK (
        (governance_status='enforced' AND execution_backend <> 'none_blocked')
        OR (governance_status='policy_blocked' AND execution_backend='none_blocked')
        OR governance_status='shadow_observed_uncontrolled'
    )
);

CREATE TABLE capability_execution_destination_policy_members (
    id UUID PRIMARY KEY,
    policy_id UUID NOT NULL
        REFERENCES capability_execution_destination_policies(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    destination_role TEXT NOT NULL CHECK (
        destination_role IN ('authorized_target','fixed_provider_endpoint','fixed_dns_resolver')
    ),
    scheme TEXT NOT NULL CHECK (BTRIM(scheme) <> ''),
    normalized_host TEXT NOT NULL CHECK (BTRIM(normalized_host) <> ''),
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    path_prefix TEXT NOT NULL,
    input_binding_mode TEXT NOT NULL CHECK (
        input_binding_mode IN ('destination_authority','escaped_parameter_only')
    ),
    exact_scope_exception_hash TEXT,
    member_hash TEXT NOT NULL CHECK (length(member_hash)=64),
    UNIQUE(policy_id,ordinal),
    UNIQUE(policy_id,member_hash),
    UNIQUE(id,policy_id),
    CHECK (
        (destination_role='authorized_target' AND input_binding_mode='destination_authority')
        OR (destination_role IN ('fixed_provider_endpoint','fixed_dns_resolver')
            AND input_binding_mode='escaped_parameter_only')
    )
);

CREATE TABLE evidence_temporal_validity_policies (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    stage_execution_id UUID NOT NULL REFERENCES stage_runs(id) ON DELETE RESTRICT,
    policy_contract_version TEXT NOT NULL DEFAULT 'evidence_temporal_validity.v1'
        CHECK (policy_contract_version='evidence_temporal_validity.v1'),
    max_cross_observation_skew_ms BIGINT NOT NULL CHECK (max_cross_observation_skew_ms >= 0),
    member_count BIGINT NOT NULL CHECK (member_count > 0),
    member_set_hash TEXT NOT NULL CHECK (length(member_set_hash)=64),
    policy_hash TEXT NOT NULL CHECK (length(policy_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(id,policy_hash,operation_id,organization_id,stage_execution_id)
);

CREATE TABLE evidence_temporal_validity_policy_members (
    policy_id UUID NOT NULL REFERENCES evidence_temporal_validity_policies(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    fact_class TEXT NOT NULL CHECK (BTRIM(fact_class) <> ''),
    positive_ttl_ms BIGINT NOT NULL CHECK (positive_ttl_ms > 0),
    negative_ttl_ms BIGINT NOT NULL CHECK (negative_ttl_ms > 0),
    refutation_ttl_ms BIGINT NOT NULL CHECK (refutation_ttl_ms > 0),
    require_same_target_state_epoch BOOLEAN NOT NULL,
    required_recheck_source TEXT NOT NULL CHECK (BTRIM(required_recheck_source) <> ''),
    member_hash TEXT NOT NULL CHECK (length(member_hash)=64),
    PRIMARY KEY(policy_id,ordinal),
    UNIQUE(policy_id,fact_class),
    CHECK (negative_ttl_ms < positive_ttl_ms),
    CHECK (refutation_ttl_ms < positive_ttl_ms)
);

CREATE TABLE tool_truth_target_state_epoch_events (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    target_scope_identity_hash TEXT NOT NULL CHECK (length(target_scope_identity_hash)=64),
    epoch BIGINT NOT NULL CHECK (epoch >= 0),
    predecessor_event_id UUID,
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'operation_initialized','scope_authority_changed','credential_authority_changed',
        'application_model_changed','canonical_target_change_observed','coordinated_revalidation_wave'
    )),
    source_authority_hash TEXT NOT NULL CHECK (length(source_authority_hash)=64),
    event_hash TEXT NOT NULL CHECK (length(event_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,organization_id,target_scope_identity_hash,epoch),
    UNIQUE(id,operation_id,organization_id,target_scope_identity_hash),
    UNIQUE(id,operation_id,organization_id,target_scope_identity_hash,epoch),
    FOREIGN KEY(predecessor_event_id,operation_id,organization_id,target_scope_identity_hash)
        REFERENCES tool_truth_target_state_epoch_events(
            id,operation_id,organization_id,target_scope_identity_hash
        ) ON DELETE RESTRICT
);

CREATE TABLE tool_truth_target_state_epoch_heads (
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    target_scope_identity_hash TEXT NOT NULL,
    current_epoch BIGINT NOT NULL CHECK (current_epoch >= 0),
    current_event_id UUID NOT NULL,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    PRIMARY KEY(operation_id,organization_id,target_scope_identity_hash),
    FOREIGN KEY(current_event_id,operation_id,organization_id,target_scope_identity_hash,current_epoch)
        REFERENCES tool_truth_target_state_epoch_events(
            id,operation_id,organization_id,target_scope_identity_hash,epoch
        ) ON DELETE RESTRICT
);

CREATE TABLE capability_execution_receipts (
    id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    stage_execution_id UUID NOT NULL REFERENCES stage_runs(id) ON DELETE RESTRICT,
    unit_id UUID REFERENCES stage_run_units(id) ON DELETE RESTRICT,
    capability TEXT NOT NULL,
    attempt_epoch TIMESTAMPTZ NOT NULL,
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal > 0),
    authority_hash TEXT NOT NULL CHECK (length(authority_hash) = 64),
    input_manifest_hash TEXT NOT NULL CHECK (length(input_manifest_hash) = 64),
    temporal_validity_policy_id UUID NOT NULL,
    temporal_validity_policy_hash TEXT NOT NULL CHECK (length(temporal_validity_policy_hash)=64),
    target_scope_identity_hash TEXT NOT NULL CHECK (length(target_scope_identity_hash)=64),
    target_state_epoch_event_id UUID NOT NULL,
    target_state_epoch BIGINT NOT NULL CHECK (target_state_epoch >= 0),
    observation_started_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    observation_completed_at TIMESTAMPTZ,
    valid_until TIMESTAMPTZ,
    budget_contract_hash TEXT NOT NULL CHECK (length(budget_contract_hash) = 64),
    destination_policy_id UUID NOT NULL
        REFERENCES capability_execution_destination_policies(id) ON DELETE RESTRICT,
    destination_policy_hash TEXT NOT NULL CHECK (length(destination_policy_hash)=64),
    destination_governance_status TEXT NOT NULL CHECK (
        destination_governance_status IN ('enforced','shadow_observed_uncontrolled','policy_blocked')
    ),
    attempt_state TEXT NOT NULL
        CHECK (attempt_state IN ('not_started','running','succeeded','failed','outcome_unknown','exhausted','superseded')),
    landing_state TEXT NOT NULL
        CHECK (landing_state IN ('not_attempted','partial','committed','failed')),
    observation_state TEXT NOT NULL
        CHECK (observation_state IN ('found','no_match','indeterminate','not_applicable')),
    coverage_extent TEXT NOT NULL
        CHECK (coverage_extent IN ('none','complete','partial','sampled','template_only')),
    coverage_gap_reason TEXT NOT NULL
        CHECK (coverage_gap_reason IN ('none','transport','tool_failure','parser_reject','budget_exhausted','unsupported','policy_blocked','source_unavailable')),
    reconciliation_state TEXT NOT NULL
        CHECK (reconciliation_state IN ('pending','consistent','orphaned','superseded')),
    security_interpretation TEXT NOT NULL
        CHECK (security_interpretation IN ('not_assessed','signal','inconclusive')),
    typed_landing_contract_version TEXT NOT NULL DEFAULT 'capability_landing.v1'
        CHECK (typed_landing_contract_version='capability_landing.v1'),
    typed_landing JSONB NOT NULL,
    residual JSONB,
    current_semantic_authority_version BIGINT NOT NULL DEFAULT 0
        CHECK (current_semantic_authority_version >= 0),
    current_semantic_reconciliation_id UUID,
    current_semantic_reconciliation_hash TEXT CHECK (
        current_semantic_reconciliation_hash IS NULL
        OR current_semantic_reconciliation_hash ~ '^[0-9a-f]{64}$'
    ),
    row_version BIGINT NOT NULL DEFAULT 0,
    started_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    finalized_at TIMESTAMPTZ,
    FOREIGN KEY(
        destination_policy_id,destination_policy_hash,destination_governance_status,denominator_id,
        operation_id,organization_id,stage_execution_id,capability,attempt_epoch
    ) REFERENCES capability_execution_destination_policies(
        id,policy_hash,governance_status,denominator_id,
        operation_id,organization_id,stage_execution_id,capability,attempt_epoch
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        temporal_validity_policy_id,temporal_validity_policy_hash,
        operation_id,organization_id,stage_execution_id
    ) REFERENCES evidence_temporal_validity_policies(
        id,policy_hash,operation_id,organization_id,stage_execution_id
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        target_state_epoch_event_id,operation_id,organization_id,
        target_scope_identity_hash,target_state_epoch
    ) REFERENCES tool_truth_target_state_epoch_events(
        id,operation_id,organization_id,target_scope_identity_hash,epoch
    ) ON DELETE RESTRICT,
    CHECK (
        coverage_gap_reason = 'none'
        OR coverage_extent <> 'complete'
    ),
    CHECK (
        coverage_extent <> 'complete'
        OR (
            attempt_state='succeeded'
            AND landing_state='committed'
            AND observation_state IN ('found','no_match')
            AND coverage_gap_reason='none'
            AND reconciliation_state='consistent'
            AND destination_governance_status='enforced'
            AND observation_completed_at IS NOT NULL
            AND valid_until IS NOT NULL
            AND valid_until > observation_completed_at
        )
    ),
    UNIQUE(id,destination_policy_id)
);

CREATE TABLE capability_raw_witness_artifacts (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL UNIQUE REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    content_key TEXT NOT NULL CHECK (content_key ~ '^sha256:[0-9a-f]{64}$'),
    vault_object_ref_token BYTEA NOT NULL CHECK (octet_length(vault_object_ref_token) BETWEEN 32 AND 4096),
    vault_object_ref_token_hash TEXT NOT NULL CHECK (vault_object_ref_token_hash ~ '^sha256:[0-9a-f]{64}$'),
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    ciphertext_sha256 TEXT NOT NULL CHECK (ciphertext_sha256 ~ '^[0-9a-f]{64}$'),
    encryption_contract_version TEXT NOT NULL DEFAULT 'raw_witness_envelope.v1'
        CHECK (encryption_contract_version='raw_witness_envelope.v1'),
    operation_key_ref_hash TEXT NOT NULL CHECK (operation_key_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    key_generation BIGINT NOT NULL CHECK (key_generation > 0),
    retention_policy_id UUID NOT NULL,
    retention_policy_hash TEXT NOT NULL CHECK (retention_policy_hash ~ '^sha256:[0-9a-f]{64}$'),
    sensitivity_disposition TEXT NOT NULL CHECK (
        sensitivity_disposition IN ('typed_derivative_ready','secret_or_pii_quarantined','raw_only_restricted')
    ),
    original_byte_count BIGINT NOT NULL CHECK (original_byte_count >= 0),
    stored_byte_count BIGINT NOT NULL CHECK (stored_byte_count >= 0),
    truncated BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (stored_byte_count <= original_byte_count)
);

CREATE TABLE capability_raw_witness_access_events (
    id UUID PRIMARY KEY,
    raw_witness_artifact_id UUID NOT NULL
        REFERENCES capability_raw_witness_artifacts(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    purpose_code TEXT NOT NULL CHECK (BTRIM(purpose_code) <> ''),
    decision TEXT NOT NULL CHECK (decision IN ('allowed','denied')),
    request_hash TEXT NOT NULL CHECK (request_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

CREATE TABLE capability_raw_witness_retention_events (
    id UUID PRIMARY KEY,
    raw_witness_artifact_id UUID NOT NULL
        REFERENCES capability_raw_witness_artifacts(id) ON DELETE RESTRICT,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('retention_extended','crypto_erased')),
    previous_policy_hash TEXT NOT NULL CHECK (previous_policy_hash ~ '^sha256:[0-9a-f]{64}$'),
    next_policy_hash TEXT CHECK (next_policy_hash IS NULL OR next_policy_hash ~ '^sha256:[0-9a-f]{64}$'),
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code) <> ''),
    principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    event_hash TEXT NOT NULL CHECK (event_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);

ALTER TABLE capability_execution_receipts
    ADD COLUMN raw_witness_artifact_id UUID UNIQUE
        REFERENCES capability_raw_witness_artifacts(id) ON DELETE RESTRICT;

CREATE TABLE capability_typed_landing_source_members (
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    input_key TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('raw_range','server_control')),
    raw_start BIGINT,
    raw_end BIGINT,
    normalized_observation_hash TEXT NOT NULL CHECK (length(normalized_observation_hash)=64),
    PRIMARY KEY(receipt_id,ordinal),
    CHECK (
        (source_kind='raw_range' AND raw_start IS NOT NULL AND raw_end IS NOT NULL AND raw_start>=0 AND raw_end>raw_start)
        OR (source_kind='server_control' AND raw_start IS NULL AND raw_end IS NULL)
    )
);

CREATE TABLE capability_execution_temporal_censuses (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL UNIQUE REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    temporal_validity_policy_id UUID NOT NULL REFERENCES evidence_temporal_validity_policies(id) ON DELETE RESTRICT,
    temporal_validity_policy_hash TEXT NOT NULL CHECK (length(temporal_validity_policy_hash)=64),
    target_state_epoch_event_id UUID NOT NULL REFERENCES tool_truth_target_state_epoch_events(id) ON DELETE RESTRICT,
    target_state_epoch BIGINT NOT NULL CHECK (target_state_epoch >= 0),
    observation_window_started_at TIMESTAMPTZ NOT NULL,
    observation_window_completed_at TIMESTAMPTZ NOT NULL,
    effective_valid_until TIMESTAMPTZ NOT NULL,
    member_count BIGINT NOT NULL CHECK (member_count > 0),
    member_set_hash TEXT NOT NULL CHECK (length(member_set_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(id,receipt_id),
    CHECK (observation_window_completed_at >= observation_window_started_at),
    CHECK (effective_valid_until > observation_window_completed_at)
);

CREATE TABLE capability_execution_temporal_census_members (
    census_id UUID NOT NULL,
    receipt_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    input_key TEXT NOT NULL,
    observation_identity_hash TEXT NOT NULL CHECK (length(observation_identity_hash)=64),
    temporal_fact_class TEXT NOT NULL CHECK (BTRIM(temporal_fact_class) <> ''),
    observation_polarity TEXT NOT NULL CHECK (
        observation_polarity IN ('positive','negative','inconclusive')
    ),
    mapping_rule_id TEXT NOT NULL CHECK (BTRIM(mapping_rule_id) <> ''),
    mapping_rule_version TEXT NOT NULL CHECK (BTRIM(mapping_rule_version) <> ''),
    mapping_rule_digest TEXT NOT NULL CHECK (length(mapping_rule_digest)=64),
    source_valid_until TIMESTAMPTZ,
    selected_ttl_ms BIGINT NOT NULL CHECK (selected_ttl_ms > 0),
    observed_at TIMESTAMPTZ NOT NULL,
    effective_valid_until TIMESTAMPTZ NOT NULL,
    member_hash TEXT NOT NULL CHECK (length(member_hash)=64),
    PRIMARY KEY(census_id,ordinal),
    UNIQUE(census_id,input_key,observation_identity_hash),
    FOREIGN KEY(census_id,receipt_id)
        REFERENCES capability_execution_temporal_censuses(id,receipt_id) ON DELETE RESTRICT,
    CHECK (effective_valid_until > observed_at),
    CHECK (source_valid_until IS NULL OR effective_valid_until <= source_valid_until)
);

ALTER TABLE capability_execution_receipts
    ADD COLUMN temporal_census_id UUID UNIQUE,
    ADD CONSTRAINT capability_execution_receipt_temporal_census_fk
        FOREIGN KEY(temporal_census_id,id)
        REFERENCES capability_execution_temporal_censuses(id,receipt_id)
        ON DELETE RESTRICT,
    ADD CONSTRAINT capability_execution_complete_requires_temporal_census CHECK (
        coverage_extent <> 'complete' OR temporal_census_id IS NOT NULL
    );

CREATE TABLE tool_truth_revalidation_obligations (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    source_root_denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    source_receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    input_key TEXT NOT NULL,
    temporal_fact_class TEXT NOT NULL CHECK (BTRIM(temporal_fact_class) <> ''),
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'observation_expired','target_epoch_changed','max_skew_exceeded','source_authority_invalid'
    )),
    temporal_validity_policy_hash TEXT NOT NULL CHECK (length(temporal_validity_policy_hash)=64),
    required_consumer_kind TEXT NOT NULL CHECK (BTRIM(required_consumer_kind) <> ''),
    stable_obligation_hash TEXT NOT NULL CHECK (length(stable_obligation_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(operation_id,organization_id,stable_obligation_hash)
);

CREATE TABLE tool_truth_revalidation_obligation_events (
    id UUID PRIMARY KEY,
    obligation_id UUID NOT NULL REFERENCES tool_truth_revalidation_obligations(id) ON DELETE RESTRICT,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal >= 0),
    predecessor_event_id UUID,
    event_kind TEXT NOT NULL CHECK (event_kind IN (
        'opened','claimed','attempt_started','succeeded','no_progress','exhausted','risk_accepted','released'
    )),
    worker_lease_id UUID,
    replacement_denominator_id UUID REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    replacement_receipt_id UUID REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    residual JSONB,
    event_hash TEXT NOT NULL CHECK (length(event_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(obligation_id,event_ordinal),
    UNIQUE(id,obligation_id),
    FOREIGN KEY(predecessor_event_id,obligation_id)
        REFERENCES tool_truth_revalidation_obligation_events(id,obligation_id) ON DELETE RESTRICT
);

CREATE TABLE tool_truth_revalidation_obligation_heads (
    obligation_id UUID PRIMARY KEY REFERENCES tool_truth_revalidation_obligations(id) ON DELETE RESTRICT,
    current_event_id UUID NOT NULL,
    current_event_ordinal BIGINT NOT NULL CHECK (current_event_ordinal >= 0),
    state TEXT NOT NULL CHECK (state IN (
        'open','claimed','running','succeeded','exhausted','risk_accepted'
    )),
    retry_count BIGINT NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    deadline_at TIMESTAMPTZ NOT NULL,
    no_progress_fingerprint TEXT,
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    FOREIGN KEY(current_event_id,obligation_id)
        REFERENCES tool_truth_revalidation_obligation_events(id,obligation_id) ON DELETE RESTRICT
);

CREATE TABLE capability_parser_censuses (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL UNIQUE REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    raw_witness_artifact_id UUID NOT NULL
        REFERENCES capability_raw_witness_artifacts(id) ON DELETE RESTRICT,
    framer_contract_id TEXT NOT NULL CHECK (BTRIM(framer_contract_id) <> ''),
    framer_contract_version TEXT NOT NULL CHECK (BTRIM(framer_contract_version) <> ''),
    framer_digest TEXT NOT NULL CHECK (length(framer_digest)=64),
    framing_manifest_hash TEXT NOT NULL CHECK (length(framing_manifest_hash)=64),
    parser_contract_id TEXT NOT NULL CHECK (BTRIM(parser_contract_id) <> ''),
    parser_contract_version TEXT NOT NULL CHECK (BTRIM(parser_contract_version) <> ''),
    parser_digest TEXT NOT NULL CHECK (length(parser_digest)=64),
    parse_domain_byte_count BIGINT NOT NULL CHECK (parse_domain_byte_count >= 0),
    framed_record_count BIGINT NOT NULL CHECK (framed_record_count >= 0),
    member_count BIGINT NOT NULL CHECK (member_count >= 0),
    member_set_hash TEXT NOT NULL CHECK (length(member_set_hash)=64),
    unaccounted_nonempty_record_count BIGINT NOT NULL
        CHECK (unaccounted_nonempty_record_count = 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(id,receipt_id),
    CHECK (member_count = framed_record_count)
);

CREATE TABLE capability_parser_census_members (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    census_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    stream_kind TEXT NOT NULL CHECK (stream_kind IN ('envelope','stdout','stderr')),
    raw_start BIGINT NOT NULL CHECK (raw_start >= 0),
    raw_end BIGINT NOT NULL CHECK (raw_end > raw_start),
    record_hash TEXT NOT NULL CHECK (length(record_hash)=64),
    disposition TEXT NOT NULL CHECK (
        disposition IN ('parsed_observation','ignored_versioned','control_framing')
    ),
    ignore_reason_code TEXT,
    ignore_rule_version TEXT,
    derived_child_identity_hash TEXT CHECK (
        derived_child_identity_hash IS NULL OR length(derived_child_identity_hash)=64
    ),
    UNIQUE(census_id,ordinal),
    UNIQUE(census_id,stream_kind,raw_start,raw_end),
    UNIQUE(id,receipt_id,raw_start,raw_end),
    FOREIGN KEY(census_id,receipt_id)
        REFERENCES capability_parser_censuses(id,receipt_id) ON DELETE RESTRICT,
    CHECK (
        (disposition='parsed_observation'
            AND ignore_reason_code IS NULL AND ignore_rule_version IS NULL)
        OR (disposition='ignored_versioned'
            AND BTRIM(ignore_reason_code) <> '' AND BTRIM(ignore_rule_version) <> ''
            AND derived_child_identity_hash IS NULL)
        OR (disposition='control_framing'
            AND ignore_reason_code IS NULL AND ignore_rule_version IS NULL
            AND derived_child_identity_hash IS NULL)
    )
);

ALTER TABLE capability_typed_landing_source_members
    ADD COLUMN parser_census_member_id UUID,
    ADD CONSTRAINT capability_typed_source_parser_member_fk
        FOREIGN KEY(parser_census_member_id,receipt_id,raw_start,raw_end)
        REFERENCES capability_parser_census_members(id,receipt_id,raw_start,raw_end)
        ON DELETE RESTRICT,
    ADD CONSTRAINT capability_typed_source_parser_shape_check CHECK (
        (source_kind='raw_range' AND parser_census_member_id IS NOT NULL)
        OR (source_kind='server_control' AND parser_census_member_id IS NULL)
    );

CREATE TABLE capability_execution_budget_contract_axes (
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    axis TEXT NOT NULL CHECK (axis IN (
        'requests','response_bytes','wall_clock_ms','retries','browser_steps','oast_tokens'
    )),
    required_for_complete BOOLEAN NOT NULL,
    planned_limit BIGINT CHECK (planned_limit IS NULL OR planned_limit >= 0),
    required_observation_source TEXT NOT NULL CHECK (required_observation_source IN (
        'host_governor','adapter_instrumentation','cli_unobserved'
    )),
    PRIMARY KEY (receipt_id,axis)
);

CREATE TABLE capability_execution_budget_observations (
    receipt_id UUID NOT NULL,
    axis TEXT NOT NULL,
    actual_value BIGINT CHECK (actual_value IS NULL OR actual_value >= 0),
    observed BOOLEAN NOT NULL DEFAULT FALSE,
    observation_source TEXT NOT NULL CHECK (observation_source IN (
        'host_governor','adapter_instrumentation','cli_unobserved'
    )),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    PRIMARY KEY (receipt_id,axis),
    FOREIGN KEY (receipt_id,axis)
        REFERENCES capability_execution_budget_contract_axes(receipt_id,axis)
        ON DELETE RESTRICT,
    CHECK (NOT observed OR actual_value IS NOT NULL)
);

CREATE TABLE capability_execution_network_hop_receipts (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    destination_policy_id UUID NOT NULL
        REFERENCES capability_execution_destination_policies(id) ON DELETE RESTRICT,
    destination_member_id UUID,
    hop_ordinal INTEGER NOT NULL CHECK (hop_ordinal >= 0),
    hop_kind TEXT NOT NULL CHECK (hop_kind IN ('initial','redirect','retry','cli_egress')),
    canonical_origin_hash TEXT NOT NULL CHECK (length(canonical_origin_hash)=64),
    normalized_path_hash TEXT NOT NULL CHECK (length(normalized_path_hash)=64),
    dns_answer_set_hash TEXT,
    selected_ip TEXT,
    host_sni_hash TEXT NOT NULL CHECK (length(host_sni_hash)=64),
    policy_decision TEXT NOT NULL CHECK (policy_decision IN ('allowed','blocked')),
    reason_code TEXT,
    request_budget_axis_value BIGINT NOT NULL CHECK (request_budget_axis_value >= 0),
    hop_receipt_hash TEXT NOT NULL CHECK (length(hop_receipt_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(receipt_id,hop_ordinal),
    FOREIGN KEY(receipt_id,destination_policy_id)
        REFERENCES capability_execution_receipts(id,destination_policy_id) ON DELETE RESTRICT,
    FOREIGN KEY(destination_member_id,destination_policy_id)
        REFERENCES capability_execution_destination_policy_members(id,policy_id) ON DELETE RESTRICT,
    CHECK (
        (policy_decision='allowed' AND destination_member_id IS NOT NULL
            AND selected_ip IS NOT NULL AND reason_code IS NULL)
        OR (policy_decision='blocked' AND BTRIM(reason_code) <> '')
    )
);

CREATE TABLE capability_execution_receipt_inputs (
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    denominator_item_id UUID NOT NULL REFERENCES coverage_denominator_items(id) ON DELETE RESTRICT,
    input_key TEXT NOT NULL,
    attempt_state TEXT NOT NULL
        CHECK (attempt_state IN ('not_started','running','succeeded','failed','outcome_unknown','exhausted','superseded')),
    landing_state TEXT NOT NULL
        CHECK (landing_state IN ('not_attempted','partial','committed','failed')),
    observation_state TEXT NOT NULL
        CHECK (observation_state IN ('found','no_match','indeterminate','not_applicable')),
    coverage_extent TEXT NOT NULL
        CHECK (coverage_extent IN ('none','complete','partial','sampled','template_only')),
    coverage_gap_reason TEXT NOT NULL
        CHECK (coverage_gap_reason IN ('none','transport','tool_failure','parser_reject','budget_exhausted','unsupported','policy_blocked','source_unavailable')),
    security_interpretation TEXT NOT NULL
        CHECK (security_interpretation IN ('not_assessed','signal','inconclusive')),
    evidence_member_count BIGINT CHECK (evidence_member_count IS NULL OR evidence_member_count >= 0),
    evidence_membership_hash TEXT CHECK (evidence_membership_hash IS NULL OR evidence_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    business_ref_member_count BIGINT CHECK (business_ref_member_count IS NULL OR business_ref_member_count >= 0),
    business_ref_membership_hash TEXT CHECK (business_ref_membership_hash IS NULL OR business_ref_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    lineage_sealed_at TIMESTAMPTZ,
    PRIMARY KEY (receipt_id, denominator_item_id),
    UNIQUE (receipt_id, input_key),
    CHECK (
        (lineage_sealed_at IS NULL AND evidence_member_count IS NULL AND evidence_membership_hash IS NULL
            AND business_ref_member_count IS NULL AND business_ref_membership_hash IS NULL)
        OR (lineage_sealed_at IS NOT NULL AND evidence_member_count IS NOT NULL AND evidence_membership_hash IS NOT NULL
            AND business_ref_member_count IS NOT NULL AND business_ref_membership_hash IS NOT NULL)
    )
);

CREATE TABLE capability_execution_input_evidence_members (
    receipt_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    evidence_id BIGINT NOT NULL,
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    operation_id UUID NOT NULL,
    project_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    PRIMARY KEY(receipt_id,denominator_item_id,ordinal),
    UNIQUE(receipt_id,denominator_item_id,evidence_id),
    FOREIGN KEY(receipt_id,denominator_item_id)
        REFERENCES capability_execution_receipt_inputs(receipt_id,denominator_item_id) ON DELETE RESTRICT
    -- actual migration also adds the existing evidence-ledger ownership/hash compound FK
);

CREATE TABLE capability_execution_input_business_ref_members (
    receipt_id UUID NOT NULL,
    denominator_item_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    ref_kind TEXT NOT NULL,
    ref_id TEXT NOT NULL CHECK (BTRIM(ref_id) <> ''),
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    operation_id UUID NOT NULL,
    project_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    PRIMARY KEY(receipt_id,denominator_item_id,ordinal),
    UNIQUE(receipt_id,denominator_item_id,ref_kind,ref_id),
    FOREIGN KEY(receipt_id,denominator_item_id)
        REFERENCES capability_execution_receipt_inputs(receipt_id,denominator_item_id) ON DELETE RESTRICT
    -- each closed ref_kind gets an exact compound FK to its canonical business table
);

CREATE TABLE capability_discovered_child_manifests (
    id UUID PRIMARY KEY,
    parent_receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    parent_denominator_item_id UUID NOT NULL REFERENCES coverage_denominator_items(id) ON DELETE RESTRICT,
    child_kind TEXT NOT NULL,
    expected_downstream_technique TEXT NOT NULL,
    expected_downstream_capability TEXT NOT NULL,
    child_manifest_hash TEXT NOT NULL CHECK (length(child_manifest_hash)=64),
    expected_child_count INTEGER NOT NULL CHECK (expected_child_count >= 0),
    sealed_empty BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(parent_receipt_id,parent_denominator_item_id,child_kind,expected_downstream_technique),
    CHECK (sealed_empty = (expected_child_count = 0))
);

CREATE TABLE capability_discovered_child_members (
    id UUID PRIMARY KEY,
    child_manifest_id UUID NOT NULL REFERENCES capability_discovered_child_manifests(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    child_key TEXT NOT NULL,
    exact_child_asset TEXT NOT NULL,
    child_hash TEXT NOT NULL CHECK (length(child_hash)=64),
    scope_snapshot_id UUID NOT NULL,
    scope_snapshot_hash TEXT NOT NULL CHECK (length(scope_snapshot_hash)=64),
    scope_classification TEXT NOT NULL CHECK (
        scope_classification IN ('in_scope','external_dependency','out_of_scope')
    ),
    scope_exception_hash TEXT CHECK (
        scope_exception_hash IS NULL OR length(scope_exception_hash)=64
    ),
    scope_decision_hash TEXT NOT NULL CHECK (length(scope_decision_hash)=64),
    UNIQUE(child_manifest_id,ordinal),
    UNIQUE(child_manifest_id,child_key)
);

CREATE TABLE capability_discovered_child_closures (
    id UUID PRIMARY KEY,
    child_member_id UUID NOT NULL UNIQUE REFERENCES capability_discovered_child_members(id) ON DELETE RESTRICT,
    closure_kind TEXT NOT NULL CHECK (closure_kind IN (
        'downstream_denominator_item','deduplicated_existing','not_applicable','blocked',
        'external_dependency','out_of_scope'
    )),
    downstream_denominator_item_id UUID REFERENCES coverage_denominator_items(id) ON DELETE RESTRICT,
    residual JSONB,
    closure_hash TEXT NOT NULL CHECK (length(closure_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    CHECK (
        (closure_kind IN ('downstream_denominator_item','deduplicated_existing')
            AND downstream_denominator_item_id IS NOT NULL AND residual IS NULL)
        OR (closure_kind IN (
                'not_applicable','blocked','external_dependency','out_of_scope'
            ) AND downstream_denominator_item_id IS NULL
            AND jsonb_typeof(residual)='object')
    )
);

CREATE TABLE tool_truth_discovery_operation_budget_contracts (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL UNIQUE
        REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    max_root_denominators BIGINT NOT NULL CHECK (max_root_denominators > 0),
    max_unique_children_total BIGINT NOT NULL CHECK (max_unique_children_total >= 0),
    max_derived_denominators BIGINT NOT NULL CHECK (max_derived_denominators >= 0),
    max_derived_receipts BIGINT NOT NULL CHECK (max_derived_receipts >= 0),
    max_raw_bytes_total BIGINT NOT NULL CHECK (max_raw_bytes_total >= 0),
    max_wall_clock_ms_total BIGINT NOT NULL CHECK (max_wall_clock_ms_total >= 0),
    contract_hash TEXT NOT NULL CHECK (length(contract_hash)=64),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(id,operation_id)
);

CREATE TABLE tool_truth_discovery_operation_budget_heads (
    operation_budget_contract_id UUID PRIMARY KEY
        REFERENCES tool_truth_discovery_operation_budget_contracts(id) ON DELETE RESTRICT,
    admitted_root_denominator_count BIGINT NOT NULL DEFAULT 0
        CHECK (admitted_root_denominator_count >= 0),
    admitted_unique_children BIGINT NOT NULL DEFAULT 0 CHECK (admitted_unique_children >= 0),
    derived_denominator_count BIGINT NOT NULL DEFAULT 0 CHECK (derived_denominator_count >= 0),
    derived_receipt_count BIGINT NOT NULL DEFAULT 0 CHECK (derived_receipt_count >= 0),
    raw_bytes_consumed BIGINT NOT NULL DEFAULT 0 CHECK (raw_bytes_consumed >= 0),
    wall_clock_ms_consumed BIGINT NOT NULL DEFAULT 0 CHECK (wall_clock_ms_consumed >= 0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0)
);

CREATE TABLE tool_truth_discovery_budget_contracts (
    id UUID PRIMARY KEY,
    operation_budget_contract_id UUID NOT NULL,
    root_denominator_id UUID NOT NULL UNIQUE
        REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    max_depth INTEGER NOT NULL CHECK (max_depth >= 0),
    max_unique_children_total BIGINT NOT NULL CHECK (max_unique_children_total >= 0),
    max_unique_children_per_parent_kind BIGINT NOT NULL
        CHECK (max_unique_children_per_parent_kind >= 0),
    max_derived_denominators BIGINT NOT NULL CHECK (max_derived_denominators >= 0),
    max_derived_receipts BIGINT NOT NULL CHECK (max_derived_receipts >= 0),
    max_raw_bytes_total BIGINT NOT NULL CHECK (max_raw_bytes_total >= 0),
    max_wall_clock_ms_total BIGINT NOT NULL CHECK (max_wall_clock_ms_total >= 0),
    contract_hash TEXT NOT NULL CHECK (length(contract_hash)=64),
    sealed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(operation_budget_contract_id,operation_id)
        REFERENCES tool_truth_discovery_operation_budget_contracts(id,operation_id)
        ON DELETE RESTRICT,
    UNIQUE(id,operation_budget_contract_id)
);

CREATE TABLE tool_truth_discovery_budget_heads (
    contract_id UUID PRIMARY KEY
        REFERENCES tool_truth_discovery_budget_contracts(id) ON DELETE RESTRICT,
    admitted_unique_children BIGINT NOT NULL DEFAULT 0 CHECK (admitted_unique_children >= 0),
    derived_denominator_count BIGINT NOT NULL DEFAULT 0 CHECK (derived_denominator_count >= 0),
    derived_receipt_count BIGINT NOT NULL DEFAULT 0 CHECK (derived_receipt_count >= 0),
    raw_bytes_consumed BIGINT NOT NULL DEFAULT 0 CHECK (raw_bytes_consumed >= 0),
    wall_clock_ms_consumed BIGINT NOT NULL DEFAULT 0 CHECK (wall_clock_ms_consumed >= 0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0)
);

CREATE TABLE tool_truth_discovery_nodes (
    id UUID PRIMARY KEY,
    contract_id UUID NOT NULL
        REFERENCES tool_truth_discovery_budget_contracts(id) ON DELETE RESTRICT,
    parent_node_id UUID REFERENCES tool_truth_discovery_nodes(id) ON DELETE RESTRICT,
    depth INTEGER NOT NULL CHECK (depth > 0),
    child_kind TEXT NOT NULL CHECK (BTRIM(child_kind) <> ''),
    canonical_child_identity_hash TEXT NOT NULL CHECK (length(canonical_child_identity_hash)=64),
    path_hash TEXT NOT NULL CHECK (length(path_hash)=64),
    admitted_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(contract_id,child_kind,canonical_child_identity_hash)
);

ALTER TABLE capability_discovered_child_members
    ADD COLUMN discovery_node_id UUID
        REFERENCES tool_truth_discovery_nodes(id) ON DELETE RESTRICT,
    ADD COLUMN discovery_depth INTEGER CHECK (discovery_depth IS NULL OR discovery_depth > 0);

CREATE TABLE tool_truth_discovery_overflow_manifests (
    id UUID PRIMARY KEY,
    contract_id UUID NOT NULL
        REFERENCES tool_truth_discovery_budget_contracts(id) ON DELETE RESTRICT,
    parent_manifest_id UUID NOT NULL
        REFERENCES capability_discovered_child_manifests(id) ON DELETE RESTRICT,
    depth INTEGER NOT NULL CHECK (depth > 0),
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'max_depth','parent_kind_limit','operation_child_limit',
        'derived_denominator_limit','derived_receipt_limit',
        'raw_byte_limit','wall_clock_limit','cycle_detected'
    )),
    member_count BIGINT NOT NULL CHECK (member_count > 0),
    member_set_hash TEXT NOT NULL CHECK (length(member_set_hash)=64),
    residual JSONB NOT NULL CHECK (jsonb_typeof(residual)='object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(parent_manifest_id,depth,reason_code)
);

CREATE TABLE tool_truth_discovery_overflow_members (
    overflow_manifest_id UUID NOT NULL
        REFERENCES tool_truth_discovery_overflow_manifests(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    child_kind TEXT NOT NULL,
    canonical_child_identity_hash TEXT NOT NULL CHECK (length(canonical_child_identity_hash)=64),
    source_parser_census_member_id UUID NOT NULL
        REFERENCES capability_parser_census_members(id) ON DELETE RESTRICT,
    PRIMARY KEY(overflow_manifest_id,ordinal),
    UNIQUE(overflow_manifest_id,child_kind,canonical_child_identity_hash)
);

CREATE TABLE tool_truth_discovery_budget_ledger_entries (
    id UUID PRIMARY KEY,
    contract_id UUID NOT NULL
        REFERENCES tool_truth_discovery_budget_contracts(id) ON DELETE RESTRICT,
    operation_budget_contract_id UUID NOT NULL
        REFERENCES tool_truth_discovery_operation_budget_contracts(id) ON DELETE RESTRICT,
    breadth_first_wave_depth INTEGER NOT NULL CHECK (breadth_first_wave_depth >= 0),
    entry_kind TEXT NOT NULL CHECK (entry_kind IN (
        'admit_children','admit_denominator','begin_receipt','consume_raw_bytes','consume_wall_clock','overflow'
    )),
    delta BIGINT NOT NULL CHECK (delta >= 0),
    resulting_head_hash TEXT NOT NULL CHECK (length(resulting_head_hash)=64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    FOREIGN KEY(contract_id,operation_budget_contract_id)
        REFERENCES tool_truth_discovery_budget_contracts(id,operation_budget_contract_id)
        ON DELETE RESTRICT
);

ALTER TABLE coverage_denominators
    ADD COLUMN parent_child_manifest_id UUID
        REFERENCES capability_discovered_child_manifests(id) ON DELETE RESTRICT,
    ADD CONSTRAINT coverage_denominator_kind_shape_check CHECK (
        (denominator_kind='root' AND parent_child_manifest_id IS NULL AND derived_ordinal IS NULL)
        OR (denominator_kind='derived_child' AND parent_child_manifest_id IS NOT NULL
            AND derived_ordinal IS NOT NULL AND derived_ordinal > 0)
    );

CREATE TABLE capability_execution_reconciliations (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    semantic_authority_version BIGINT NOT NULL CHECK (semantic_authority_version > 0),
    predecessor_reconciliation_id UUID,
    reconciliation_state TEXT NOT NULL
        CHECK (reconciliation_state IN ('pending','consistent','orphaned','superseded')),
    checked_evidence_member_count BIGINT CHECK (checked_evidence_member_count IS NULL OR checked_evidence_member_count >= 0),
    checked_evidence_membership_hash TEXT CHECK (checked_evidence_membership_hash IS NULL OR checked_evidence_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    checked_business_ref_member_count BIGINT CHECK (checked_business_ref_member_count IS NULL OR checked_business_ref_member_count >= 0),
    checked_business_ref_membership_hash TEXT CHECK (checked_business_ref_membership_hash IS NULL OR checked_business_ref_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    reason_code TEXT,
    observed_artifact_sha256 TEXT CHECK (observed_artifact_sha256 IS NULL OR length(observed_artifact_sha256)=64),
    observed_artifact_byte_count BIGINT CHECK (observed_artifact_byte_count IS NULL OR observed_artifact_byte_count>=0),
    authority_hash TEXT CHECK (authority_hash IS NULL OR length(authority_hash) = 64),
    semantic_reconciliation_hash TEXT CHECK (
        semantic_reconciliation_hash IS NULL OR length(semantic_reconciliation_hash) = 64
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE (receipt_id, semantic_authority_version),
    UNIQUE (receipt_id, id),
    UNIQUE (receipt_id,id,semantic_authority_version,semantic_reconciliation_hash),
    FOREIGN KEY (receipt_id, predecessor_reconciliation_id)
        REFERENCES capability_execution_reconciliations(receipt_id,id) ON DELETE RESTRICT,
    CHECK (
        (sealed_at IS NULL AND checked_evidence_member_count IS NULL AND checked_evidence_membership_hash IS NULL
            AND checked_business_ref_member_count IS NULL AND checked_business_ref_membership_hash IS NULL
            AND authority_hash IS NULL AND semantic_reconciliation_hash IS NULL)
        OR (sealed_at IS NOT NULL AND checked_evidence_member_count IS NOT NULL AND checked_evidence_membership_hash IS NOT NULL
            AND checked_business_ref_member_count IS NOT NULL AND checked_business_ref_membership_hash IS NOT NULL
            AND authority_hash IS NOT NULL AND semantic_reconciliation_hash IS NOT NULL)
    )
);

CREATE TABLE capability_execution_reconciliation_evidence_members (
    reconciliation_id UUID NOT NULL REFERENCES capability_execution_reconciliations(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    evidence_id BIGINT NOT NULL,
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    operation_id UUID NOT NULL,
    project_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    PRIMARY KEY(reconciliation_id,ordinal),
    UNIQUE(reconciliation_id,evidence_id)
    -- actual migration adds the evidence-ledger operation/project/org/hash compound FK
);

CREATE TABLE capability_execution_reconciliation_business_ref_members (
    reconciliation_id UUID NOT NULL REFERENCES capability_execution_reconciliations(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    ref_kind TEXT NOT NULL,
    ref_id TEXT NOT NULL CHECK (BTRIM(ref_id) <> ''),
    source_hash TEXT NOT NULL CHECK (source_hash ~ '^sha256:[0-9a-f]{64}$'),
    operation_id UUID NOT NULL,
    project_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    PRIMARY KEY(reconciliation_id,ordinal),
    UNIQUE(reconciliation_id,ref_kind,ref_id)
    -- actual migration adds one closed ref_kind -> canonical row compound FK per variant
);

ALTER TABLE capability_execution_receipts
    ADD CONSTRAINT capability_execution_receipts_current_semantic_reconciliation_fk
    FOREIGN KEY(
        id,current_semantic_reconciliation_id,
        current_semantic_authority_version,current_semantic_reconciliation_hash
    ) REFERENCES capability_execution_reconciliations(
        receipt_id,id,semantic_authority_version,semantic_reconciliation_hash
    ) ON DELETE RESTRICT,
    ADD CONSTRAINT capability_execution_receipts_current_semantic_shape_check CHECK (
        (current_semantic_authority_version=0
            AND current_semantic_reconciliation_id IS NULL
            AND current_semantic_reconciliation_hash IS NULL)
        OR (current_semantic_authority_version>0
            AND current_semantic_reconciliation_id IS NOT NULL
            AND current_semantic_reconciliation_hash IS NOT NULL)
    );

CREATE TABLE capability_execution_freshness_attestations (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    semantic_reconciliation_id UUID NOT NULL,
    semantic_authority_version BIGINT NOT NULL CHECK (semantic_authority_version > 0),
    semantic_reconciliation_hash TEXT NOT NULL CHECK (length(semantic_reconciliation_hash)=64),
    consumer_kind TEXT NOT NULL CHECK (BTRIM(consumer_kind) <> ''),
    stable_consumer_request_id UUID NOT NULL,
    artifact_object_identity_hash TEXT NOT NULL CHECK (length(artifact_object_identity_hash)=64),
    snapshot_sha256 TEXT NOT NULL CHECK (length(snapshot_sha256)=64),
    snapshot_byte_count BIGINT NOT NULL CHECK (snapshot_byte_count >= 0),
    freshness_status TEXT NOT NULL CHECK (freshness_status IN ('consistent','orphaned')),
    attestation_hash TEXT NOT NULL CHECK (length(attestation_hash)=64),
    checked_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE(stable_consumer_request_id,receipt_id),
    UNIQUE(id,receipt_id,semantic_reconciliation_id,semantic_authority_version,semantic_reconciliation_hash),
    FOREIGN KEY(
        receipt_id,semantic_reconciliation_id,
        semantic_authority_version,semantic_reconciliation_hash
    ) REFERENCES capability_execution_reconciliations(
        receipt_id,id,semantic_authority_version,semantic_reconciliation_hash
    ) ON DELETE RESTRICT
);

CREATE TABLE tool_truth_authority_set_seals (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    root_denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    consumer_kind TEXT NOT NULL CHECK (BTRIM(consumer_kind) <> ''),
    stable_consumer_request_id UUID NOT NULL,
    denominator_graph_hash TEXT NOT NULL CHECK (length(denominator_graph_hash)=64),
    member_count BIGINT NOT NULL CHECK (member_count >= 0),
    sealed_empty BOOLEAN NOT NULL,
    member_set_hash TEXT NOT NULL CHECK (length(member_set_hash)=64),
    semantic_authority_set_hash TEXT NOT NULL CHECK (length(semantic_authority_set_hash)=64),
    freshness_attestation_set_hash TEXT NOT NULL CHECK (length(freshness_attestation_set_hash)=64),
    temporal_validity_policy_hash TEXT NOT NULL CHECK (length(temporal_validity_policy_hash)=64),
    target_state_epoch_set_hash TEXT NOT NULL CHECK (length(target_state_epoch_set_hash)=64),
    observation_window_started_at TIMESTAMPTZ,
    observation_window_completed_at TIMESTAMPTZ,
    temporal_validity_status TEXT NOT NULL CHECK (
        temporal_validity_status IN ('fresh','expired','mixed_epoch','skew_exceeded')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(operation_id,consumer_kind,stable_consumer_request_id),
    CHECK (sealed_empty = (member_count = 0))
);

CREATE TABLE tool_truth_authority_set_members (
    seal_id UUID NOT NULL REFERENCES tool_truth_authority_set_seals(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    receipt_id UUID NOT NULL REFERENCES capability_execution_receipts(id) ON DELETE RESTRICT,
    semantic_reconciliation_id UUID NOT NULL,
    semantic_authority_version BIGINT NOT NULL CHECK (semantic_authority_version > 0),
    semantic_reconciliation_hash TEXT NOT NULL CHECK (length(semantic_reconciliation_hash)=64),
    freshness_attestation_id UUID NOT NULL,
    PRIMARY KEY(seal_id,ordinal),
    UNIQUE(seal_id,receipt_id),
    FOREIGN KEY(
        receipt_id,semantic_reconciliation_id,
        semantic_authority_version,semantic_reconciliation_hash
    ) REFERENCES capability_execution_reconciliations(
        receipt_id,id,semantic_authority_version,semantic_reconciliation_hash
    ) ON DELETE RESTRICT,
    FOREIGN KEY(
        freshness_attestation_id,receipt_id,semantic_reconciliation_id,
        semantic_authority_version,semantic_reconciliation_hash
    ) REFERENCES capability_execution_freshness_attestations(
        id,receipt_id,semantic_reconciliation_id,
        semantic_authority_version,semantic_reconciliation_hash
    ) ON DELETE RESTRICT
);

CREATE TABLE tool_truth_authority_bundle_seals (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    consumer_kind TEXT NOT NULL CHECK (BTRIM(consumer_kind) <> ''),
    stable_consumer_request_id UUID NOT NULL,
    relevant_root_count BIGINT NOT NULL CHECK (relevant_root_count > 0),
    relevant_root_set_hash TEXT NOT NULL CHECK (length(relevant_root_set_hash)=64),
    member_count BIGINT NOT NULL CHECK (member_count > 0),
    member_set_hash TEXT NOT NULL CHECK (length(member_set_hash)=64),
    semantic_authority_bundle_hash TEXT NOT NULL CHECK (length(semantic_authority_bundle_hash)=64),
    freshness_attestation_bundle_hash TEXT NOT NULL CHECK (length(freshness_attestation_bundle_hash)=64),
    temporal_validity_bundle_hash TEXT NOT NULL CHECK (length(temporal_validity_bundle_hash)=64),
    consistent_fresh_count BIGINT NOT NULL CHECK (consistent_fresh_count >= 0),
    stale_or_invalid_count BIGINT NOT NULL CHECK (stale_or_invalid_count >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(operation_id,organization_id,consumer_kind,stable_consumer_request_id),
    CHECK (consistent_fresh_count + stale_or_invalid_count = member_count)
);

CREATE TABLE tool_truth_authority_bundle_members (
    bundle_seal_id UUID NOT NULL REFERENCES tool_truth_authority_bundle_seals(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    root_denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    authority_set_seal_id UUID NOT NULL REFERENCES tool_truth_authority_set_seals(id) ON DELETE RESTRICT,
    authority_set_hash TEXT NOT NULL CHECK (length(authority_set_hash)=64),
    member_status TEXT NOT NULL CHECK (
        member_status IN ('consistent_fresh','semantic_invalid','expired','mixed_epoch','skew_exceeded')
    ),
    member_hash TEXT NOT NULL CHECK (length(member_hash)=64),
    PRIMARY KEY(bundle_seal_id,ordinal),
    UNIQUE(bundle_seal_id,root_denominator_id),
    UNIQUE(bundle_seal_id,authority_set_seal_id)
);

CREATE TABLE tool_truth_gate_assessments (
    id UUID PRIMARY KEY,
    denominator_id UUID NOT NULL REFERENCES coverage_denominators(id) ON DELETE RESTRICT,
    authority_set_seal_id UUID NOT NULL
        REFERENCES tool_truth_authority_set_seals(id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    stage_execution_id UUID NOT NULL REFERENCES stage_runs(id) ON DELETE RESTRICT,
    contract TEXT NOT NULL
        CHECK (contract IN ('shadow_v1', 'receipt_v1')),
    legacy_allowed BOOLEAN NOT NULL,
    control_decision TEXT NOT NULL CHECK (control_decision IN ('allow','hold')),
    coverage_grade TEXT NOT NULL CHECK (coverage_grade IN ('complete','degraded','incomplete')),
    divergence BOOLEAN NOT NULL,
    denominator_graph_hash TEXT NOT NULL CHECK (length(denominator_graph_hash) = 64),
    semantic_authority_set_hash TEXT NOT NULL CHECK (length(semantic_authority_set_hash) = 64),
    freshness_attestation_set_hash TEXT NOT NULL CHECK (length(freshness_attestation_set_hash) = 64),
    temporal_validity_policy_hash TEXT NOT NULL CHECK (length(temporal_validity_policy_hash)=64),
    target_state_epoch_set_hash TEXT NOT NULL CHECK (length(target_state_epoch_set_hash)=64),
    temporal_validity_status TEXT NOT NULL CHECK (
        temporal_validity_status IN ('fresh','expired','mixed_epoch','skew_exceeded')
    ),
    observation_window_started_at TIMESTAMPTZ,
    observation_window_completed_at TIMESTAMPTZ,
    decision_hash TEXT NOT NULL CHECK (length(decision_hash)=64),
    residual JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
    UNIQUE (authority_set_seal_id, contract),
    CHECK (NOT (control_decision = 'allow' AND coverage_grade = 'incomplete')),
    CHECK (NOT (control_decision = 'hold' AND coverage_grade = 'complete')),
    CHECK (
        coverage_grade <> 'degraded'
        OR (jsonb_typeof(residual) = 'array' AND jsonb_array_length(residual) > 0)
    )
);

CREATE UNIQUE INDEX coverage_denominators_execution_key_idx
    ON coverage_denominators (
        operation_id,
        organization_id,
        stage_execution_id,
        COALESCE(unit_id, '00000000-0000-0000-0000-000000000000'::uuid),
        attempt_epoch
    ) WHERE denominator_kind='root';

CREATE UNIQUE INDEX coverage_derived_denominators_manifest_idx
    ON coverage_denominators(parent_child_manifest_id)
    WHERE denominator_kind='derived_child';

CREATE UNIQUE INDEX capability_execution_receipts_execution_key_idx
    ON capability_execution_receipts (
        denominator_id,
        operation_id,
        organization_id,
        stage_execution_id,
        COALESCE(unit_id, '00000000-0000-0000-0000-000000000000'::uuid),
        attempt_epoch,
        capability,
        attempt_ordinal
    );

CREATE INDEX capability_execution_receipts_current_idx
    ON capability_execution_receipts
       (operation_id, organization_id, stage_execution_id, attempt_epoch);

CREATE INDEX capability_execution_reconciliations_receipt_idx
    ON capability_execution_reconciliations (receipt_id, created_at DESC);
~~~

实际DDL对所有重复authority列建立compound UNIQUE/FK，而不只做单列存在性：`coverage_denominator(id,operation,org,stage,attempt)`→receipt；`receipt(id,denominator,operation,org,stage,attempt)`→raw/parser/input/child manifest；parent receipt+parent denominator item必须属于同denominator；authority-set header的root denominator必须属于同operation/org，member的denominator必须处于该root的sealed derived graph且receipt必须属于该denominator/operation/org/attempt；freshness/semantic refs按`receipt + reconciliation id + semantic version + semantic hash`整体绑定。migration tests用direct SQL尝试跨org、跨attempt、跨denominator、跨child graph拼接并全部拒绝，不能只依赖repo查询。

`ToolExecutionDestinationPolicyV1`在任何receipt-v1外部dispatch前由host从frozen scope/target/provider registry生成并seal；Agent/tool args不能指定destination authority。V1 HTTP固定禁ambient/system proxy，默认deny redirect/secondary fetch，逐initial/redirect/retry强制重读policy、规范化scheme/host/port/path、解析并验证全部A/AAAA、拒绝混合合法+loopback/link-local/metadata/未授权private答案、pin validated IP socket并保留Host/SNI、按frozen TLS trust/cert/hostname policy验证；每次send先写/预留request budget，N+1在I/O前拒绝，并追加hop receipt。target私网只有exact scope snapshot exception可用；provider/WHOIS/CT/DNS等外部服务只能访问versioned fixed endpoint/resolver allowlist，用户target字符串只能成为escaped parameter/body/qname，永远不能决定URL host/port或redirect destination。

`EvidenceTemporalValidityPolicyV1`与destination policy一样由host从operation/stage policy registry生成open header/member exact set并seal，Agent/producer不能提交TTL、epoch、observed time或`valid_until`。每个canonical target/scope identity有monotonic epoch head+append-only event；scope/credential/application-model authority变化、观察到canonical target change或显式coordinated revalidation wave才CAS推进，外部世界未被观察到的变化仍由有限TTL兜底，不能声称epoch能证明目标绝对未变化。receipt `begin`只接受repo强读的current epoch event opaque authority。

closeout不能只给整张receipt挑一个有利TTL。host用versioned capability+typed-observation→temporal-fact-class mapping，对每个authoritative input/landing observation写`capability_execution_temporal_census_member`；unknown/unmapped class fail closed。`no_match`只能选negative TTL，Plan A producer不能写refutation polarity；positive/inconclusive按各自closed rule。每member observed time使用DB clock，`effective_valid_until = min(versioned source_valid_until, observed_at + selected class/polarity TTL)`由DB/repo私有规则派生；header/top receipt的window与valid-until取完整member exact set的min/max/最早expiry。caller传入远期时间、positive TTL冒充negative、漏member或旧epoch均拒绝。consumer guard用同一DB transaction clock重算所有member尚未过期、required same-epoch和max cross-observation skew；negative/refutation TTL严格短于positive TTL。过期不是artifact corruption：历史receipt仍可审计，但不能构造fresh/all-fresh authority、新Candidate authoritative snapshot/Campaign/report authority，只能写typed stale residual与revalidation obligation。

destination governance不是receipt可改的镜像：policy id/hash/status与receipt authority用compound FK整体绑定，complete硬要求`enforced`；每个hop member必须属于receipt的同一policy。closeout重读host governor的request observation，要求所有attempted sends（含blocked-before-I/O的hop decision）按ordinal exact census，实际allowed network sends与request budget actual计数一致，漏hop/额外hop/重复ordinal/status漂移一律orphan/partial。direct-SQL tests证明不能把shadow/policy-blocked policy伪装成enforced complete，也不能跨policy嫁接destination member。

unmanaged CLI无法证明逐hop DNS/redirect/secondary fetch与真实egress，因此不能靠“最终receipt partial”弥补已经发生的越界请求。`receipt_v1`只允许：host-pinned adapter，或带binary digest、exact literal-IP destination、no-redirect/no-secondary-fetch flags并由OS/process sandbox输出exact egress census的`sandboxed_cli`；缺任一控制时在spawn前写`policy_blocked + partial/residual`且process call count=0。`shadow_v1`可观察现有legacy CLI而不改变旧执行，但必须标`shadow_observed_uncontrolled`且永远不是complete/authoritative。WhatWeb/Nuclei/hostname CLI在首版默认不进入receipt-v1 authoritative执行；Nuclei保留legacy/shadow signal。future controlled proxy也必须有per-hop destination/DNS/policy/budget receipt，只有endpoint digest不够。

`original_byte_count`与`stored_byte_count`必须使用同一**加密前canonical plaintext**计量域：都统计canonical witness envelope（固定header + stdout + stderr）的总字节数；前者按未截断内容计算，后者按实际seal内容计算。typed source range也以该canonical plaintext artifact的byte offset为准；ciphertext长度另由vault attestation记录，不能混用。相同payload可以被同operation多个receipt引用同一个content key/vault-owned opaque object，因此`content_key/vault_object_ref_token_hash`不能全局唯一；receipt-owned artifact row仍由`receipt_id UNIQUE`保证exact-one。`vault_object_ref_token`是vault生成的sealed capability bytes，只允许module-private vault port解封；普通row/DTO/debug/audit API一律只返回其hash。

raw→typed landing还必须有独立parser completeness census。先由host-owned、与tool parser分离的versioned framer在sealed snapshot上把canonical envelope/stdout/stderr整个`[0,stored_byte_count)`确定性切成record/control ranges，冻结framer digest、record count与manifest hash；tool parser只能对这个既有frame exact set提交disposition，不能自报“总共只有9行”。repo按ordinal/stream/range/hash逐项重算exact-equal，deferred constraint trigger验证members对完整byte domain无重叠、无空洞、无越界，并验证每个`parsed_observation` member被同receipt、同raw range的typed source member exact-one反向引用，非parsed member零引用。每个非空record exact-one标为`parsed_observation`、`ignored_versioned`或`control_framing`。ignored只允许parser contract内versioned closed reason，且host child-discovery classifier必须证明该record不应派生port/script/endpoint等child；未知schema、未消费frame、range gap/overlap、只解析10行中的9行、或把可派生child的record标ignored，全部固定为`parser_reject + partial`。parser header的framing/parser manifests、raw byte domain、typed source set、derived child manifest hash一起进入semantic reconciliation；complete不能只证明“至少有一条解析成功”。property/fuzz tests随机插入unknown line、分隔符、重复/重叠range和partial parser输出，证明只有独立frame exact set全partition+exact disposition才能complete。

budget plan与actual observation必须分表并各自append-only：`begin`只插入`capability_execution_budget_contract_axes`并冻结`budget_contract_hash`；`stage_closeout`才对exact同一axis集合插入`capability_execution_budget_observations`。禁止用UPDATE把planned row变成actual row。

dynamic child manifest解决“外层asset×technique已complete，但执行中新发现的port/script/endpoint没有被继续处理”的假闭合。root denominator在provider dispatch前immutable seal，执行后绝不向它追加item。每个capability contract声明可能产生的`child_kind + downstream technique/capability` exact set；即使结果为空，也必须写`sealed_empty=true`的manifest。每个observed child先由host在当前scope snapshot下分类为`in_scope | external_dependency | out_of_scope`并把snapshot/hash/exception authority写入member；模型文本、redirect或provider label不能提升scope。只有`in_scope`或带精确scope exception authority的member能生成derived denominator item；第三方CDN/OAuth/analytics等external dependency和out-of-scope child只能写typed non-network closure+provenance+residual。非空in-scope set再生成一张`denominator_kind=derived_child`的独立immutable denominator（exact-one parent manifest + ordinal/hash），其items与eligible child members exact-equal；每个member必须exact-one地连接到该derived denominator item，或以`not_applicable/blocked/external_dependency/out_of_scope + exact residual`收口。不能因为缺整张manifest而把“未检查”解释成“没有child”，也不能把外部依赖误继承为target scope。Gate递归消费root及全部derived seals，但hash/identity都各自稳定。

递归发现不是无界任务生成器。operation创建时先seal唯一`tool_truth_discovery_operation_budget_contract`，冻结root数量、全operation unique child、derived denominator/receipt、raw bytes与wall-clock总上限；每个root在首个provider/tool dispatch前再seal自己的`tool_truth_discovery_budget_contract`，冻结max depth、每parent/kind unique child及root子上限，且每个root上限不得大于operation父上限。child identity由server按`root authority + normalized child kind/asset + downstream technique/capability`生成，`tool_truth_discovery_nodes`在root内全局唯一；parent edge必须同contract且`depth=parent+1`，DB deferred cycle trigger重算祖先并拒绝self/ancestor cycle。相同child被多个parser再次发现只形成deduplicated provenance/closure，不能重复计数、重复执行或绕过预算。所有admission按固定锁序`operation budget head -> root budget head`同时CAS两层head和ledger；多organization、多stage、多root并发都共享同一operation总ceiling，不能通过创建更多root把资源放大为`roots × limit`。

admission按breadth-first depth wave运行：depth d的parent exact set先冻结，允许并行执行；全部terminal/parser census到齐后，host先做scope classification，再按`parent node hash + child kind + canonical child identity`排序eligible候选，在一个短transaction依固定顺序锁两层budget head并确定depth d+1 admitted/overflow exact sets，再启动下一depth。provider完成顺序因此不影响admitted identity。任何上限命中都必须把**所有已看到但未admit**候选写入immutable overflow manifest/member并绑定source parser record与exact residual；raw本身被截断/无法穷举时另为source-unavailable。overflow/cycle/budget hit永远不能`sealed_empty`或complete，Gate只能在frozen `CoverageContinuationPolicyV1`允许时`allow/degraded + exact residual`，关键技术轴、超gap阈值或缺人工risk-acceptance一律HOLD；不能截断列表后把前N项冒充全部。initial operation/root contracts应取保守有限值，并由真实fixture调优，禁止`u64::MAX`式假上限。

所有header/member exact set统一使用同一个数据库seal状态机，范围至少包括root/derived denominator、destination policy、temporal validity policy/census、parser census、discovered-child manifest、overflow manifest、receipt-input evidence/business-ref lineage、reconciliation evidence/business-ref lineage、request-scoped authority set与multi-root authority bundle：repo在一个短transaction内插入`sealed_at=NULL` header、按canonical ordinal插入全部members、由host/DB重新计算count/hash/exact identity，再执行唯一允许的header转移`NULL -> statement_timestamp()`。任何authoritative reader、receipt `begin`、closeout、Gate与consumer callback都要求`sealed_at IS NOT NULL`。DB trigger拒绝sealed header的UPDATE/DELETE、拒绝member UPDATE/DELETE、并在header sealed后拒绝任何member INSERT；不能通过同时伪造header count/hash绕过deferred exact-set重算。child member的scope snapshot/classification/exception必须进入member、manifest与closure hash；deferred compound trigger只允许`in_scope`（含精确exception authority）连接`downstream_denominator_item/deduplicated_existing`，并强制`external_dependency/out_of_scope`使用同名non-network closure+residual，direct SQL不能拆开伪造。transaction中途失败整体回滚，不留下可见半seal。network hop receipts、target-state epoch events、freshness attestations与revalidation events是独立append-only event；semantic reconciliation自身是open→lineage members→seal header，不能在无exact evidence/business source的情况下成为authority。integration test必须逐类用direct SQL证明unsealed header不能被消费，seal后不能追加member、重seal、改hash或删除，并覆盖两个并发sealer只有exact replay可成功。

`typed_landing JSONB`只是一份closed tagged union的数据库编码，不是任意JSON扩展点。Rust唯一入口是`CapabilityLandingV1`（按capability variant绑定validated newtype）与`BusinessEvidenceRefV1`（闭集ref kind、canonical id、source hash、ownership）；unknown version/variant、额外字段、非canonical字符串、控制字符、超长字段或round-trip不稳定都fail closed。migration安装immutable validator/trigger对direct SQL执行同一golden corpus约束，并要求contract version、canonical reserialization hash和typed source member exact set一致。reconciler不接caller提供的`Vec<evidence_id>`或business JSON；它从sealed parser census、typed landing source members与canonical DB rows推导ordered evidence/business-ref members，逐条以operation/project/org/hash compound FK重读。任何跨租户拼接、孤儿、duplicate、omission或source hash漂移产生orphan/revalidation，而不是继续生成semantic authority。

在 migration comment 中记录：rollback 只允许回滚尚未被新 binary 写入的部署；存在 receipt 数据后使用向前修复，不删除 audit truth。

### Step 4：实现 rollout 只读 repo 与 operation freeze

<code>tool_truth_rollout.rs</code> 只提供：

~~~rust
use golish_pentest_domain::tool_truth::ToolTruthContract;
use sqlx::{Postgres, Transaction};

use crate::Result;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ToolTruthRolloutRow {
    pub new_operation_contract: String,
}

pub async fn get_for_share(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<ToolTruthContract> {
    let row = sqlx::query_as::<_, ToolTruthRolloutRow>(
        "SELECT new_operation_contract FROM tool_truth_rollout WHERE singleton=TRUE FOR SHARE",
    )
    .fetch_one(&mut **tx)
    .await?;
    ToolTruthContract::try_from(row.new_operation_contract.as_str())
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))
}
~~~

修改 <code>OperationStateRow</code>、全部 SELECT/RETURNING column list，并让 <code>insert_with_executor</code> 接受 <code>tool_truth_contract: ToolTruthContract</code>。普通 <code>insert</code> 在自身 transaction 里调用 <code>tool_truth_rollout::get_for_share</code>；<code>create_runtime_operation_inner</code> 与 fork 路径规则如下：

~~~rust
let source_tool_truth_contract = if let Some(source_operation_id) = input.source_operation_id {
    let persisted: String = sqlx::query_scalar(
        "SELECT tool_truth_contract FROM operation_state WHERE operation_id=$1 FOR SHARE",
    )
    .bind(source_operation_id)
    .fetch_one(&mut **tx)
    .await?;
    ToolTruthContract::try_from(persisted.as_str())
        .map_err(|_| RuntimeMemoryStoreError::Conflict {
            code: "unknown_tool_truth_contract",
        })?
} else {
    tool_truth_rollout::get_for_share(&mut tx).await?
};

let operation = operation_state::insert_with_executor(
    &mut *tx,
    input.operation_id,
    &input.profile,
    &input.entry_stage,
    &rollout.contract,
    input.project_scope_id,
    attack_contract,
    application_model_contract,
    source_tool_truth_contract,
)
.await?;
~~~

Plan A不得给<code>tool_truth_rollout</code>添加生产setter。冻结测试若需非legacy新operation，在A-only schema中只使用自动回滚transaction的<code>shadow_v1</code>；Plan B落地后该fixture必须走<code>operation_rollout</code>联合合法pair，禁止单独把Tool Truth切到<code>receipt_v1</code>并配<code>legacy_only</code>。receipt-v1 producer单元测试可构造typed contract fixture而不创建非法operation。Plan D才能在再次授权后增加唯一local-admin joint promotion路径，并在同一transaction推进Tool Truth + Investigation两个default；任何路径都不得UPDATE已冻结operation row。

### Step 5：运行 GREEN 与 migration smoke

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-db -E 'test(operation_insert_freezes_tool_truth_contract) | test(tool_truth_contract_does_not_fallback_on_unknown_value) | test(runtime_operation_creation_locks_tool_truth_rollout) | test(persisted_operation_tool_truth_contract_is_db_immutable) | test(operation_state_row_serde_roundtrip)')
~~~

**Expected:** 5 tests passed，exit code 0；direct SQL UPDATE 返回 immutable code；现有 operation fixture 补齐 <code>tool_truth_contract: "legacy_v1"</code>。

### Step 6：Future Commit

~~~bash
git add backend/crates/golish-db/migrations/20260729000005_capability_execution_receipts.sql backend/crates/golish-db/src/repo/tool_truth_rollout.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/tests/capability_execution_receipts.rs
git commit -m "feat(tool-truth): freeze receipt contract per operation"
~~~

---

## Task 3：实现 denominator、receipt closeout 与 reconciliation repo

**文件：**

- 创建：<code>backend/crates/golish-db/src/repo/capability_execution_receipts.rs</code>
- 修改：<code>backend/crates/golish-db/tests/capability_execution_receipts.rs</code>
- 修改：<code>backend/crates/golish-db/src/repo/mod.rs</code>

### Step 1：写 RED integration tests

测试必须用 project 现有 DB test harness 创建 operation/org/stage wave，然后固定以下行为：

~~~rust
#[tokio::test]
async fn manifest_is_immutable_and_begin_is_idempotent() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let request = fixture.seed_and_freeze_wave(&["origin:a", "origin:b"]).await;
    let denominator = fixture.seal_server_derived_denominator(&request).await;
    let command = fixture.begin_command(denominator.id, &["origin:a", "origin:b"]);
    let first = capability_execution_receipts::begin(&fixture.pool, &command)
        .await
        .expect("first begin");
    let replay = capability_execution_receipts::begin(&fixture.pool, &command)
        .await
        .expect("response-loss replay");
    assert_eq!(first.id, replay.id);

    let drifted = fixture.begin_command(denominator.id, &["origin:a"]);
    let error = capability_execution_receipts::begin(&fixture.pool, &drifted)
        .await
        .expect_err("same execution key with manifest drift must fail");
    assert!(error.to_string().contains("TOOL_TRUTH_MANIFEST_DRIFT"));
}

#[tokio::test]
async fn root_denominator_is_derived_from_the_locked_wave_not_a_caller_vec() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let request = fixture.seed_and_freeze_wave(&["origin:a", "origin:b"]).await;
    let denominator = fixture.seal_server_derived_denominator(&request).await;

    fixture
        .assert_denominator_exactly_matches_locked_wave_and_stage_spec(
            denominator.id,
            &["origin:a", "origin:b"],
        )
        .await;
    fixture
        .assert_public_seal_request_has_no_items_manifest_hash_or_authority_hash()
        .await;
    fixture
        .try_replay_request_against_a_different_wave(&request)
        .await
        .expect_err("stable request identity cannot be rebound to another source census");
}

#[tokio::test]
async fn sealed_root_and_derived_denominators_reject_direct_mutation() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let root = fixture.seal_root_denominator().await;
    fixture.assert_direct_item_insert_update_delete_rejected(root.id).await;
    fixture.assert_direct_header_reseal_hash_update_delete_rejected(root.id).await;

    let derived = fixture.seal_port_child_denominator(&root, &[80, 443]).await;
    fixture.assert_direct_item_insert_update_delete_rejected(derived.id).await;
    fixture.assert_direct_header_reseal_hash_update_delete_rejected(derived.id).await;
}

#[tokio::test]
async fn equal_downstream_capability_in_two_derived_denominators_has_distinct_receipts() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let left = fixture.seal_host_port_children("192.0.2.10", &[443]).await;
    let right = fixture.seal_host_port_children("192.0.2.11", &[443]).await;
    let left_receipt = fixture.begin_service_fingerprint(left.id, 1).await.expect("left");
    let right_receipt = fixture.begin_service_fingerprint(right.id, 1).await.expect("right");
    assert_ne!(left_receipt.id, right_receipt.id);
    assert_ne!(left_receipt.denominator_id, right_receipt.denominator_id);
}

#[tokio::test]
async fn late_prior_attempt_cannot_close_current_denominator() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let old = fixture.begin_old_attempt().await;
    let current = fixture.advance_epoch_and_begin().await;
    fixture.close_as_empty(old.id).await.expect("old closeout is retained");
    let old_row = capability_execution_receipts::get(&fixture.pool, old.id)
        .await
        .expect("read old")
        .expect("old exists");
    assert_eq!(old_row.reconciliation_state, "superseded");
    let current_row = capability_execution_receipts::get(&fixture.pool, current.id)
        .await
        .expect("read current")
        .expect("current exists");
    assert_eq!(current_row.attempt_state, "running");
}

#[tokio::test]
async fn terminal_publish_rejects_pending_or_orphan_inputs() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let receipt = fixture.begin_two_input_receipt().await;
    let error = fixture
        .close_only_first_input(receipt.id)
        .await
        .expect_err("unaccounted frozen input must fail closeout");
    assert!(error.to_string().contains("TOOL_TRUTH_INPUT_CENSUS_INCOMPLETE"));
}

#[tokio::test]
async fn staged_closeout_is_not_complete_until_reconciliation_finalizes_atomically() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let receipt = fixture.begin_one_input_receipt().await;
    fixture.stage_complete_closeout(receipt.id).await.expect("staged");
    let staged = fixture.receipt(receipt.id).await;
    assert_eq!(staged.reconciliation_state, "pending");
    assert_ne!(staged.coverage_extent, "complete");

    fixture.finalize_consistent_reconciliation(receipt.id).await.expect("finalize");
    let finalized = fixture.receipt(receipt.id).await;
    assert_eq!(finalized.reconciliation_state, "consistent");
    assert_eq!(finalized.coverage_extent, "complete");
}

#[tokio::test]
async fn raw_positive_parser_reject_is_partial_orphan() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let receipt = fixture.begin_one_input_receipt().await;
    fixture
        .close(
            receipt.id,
            ObservationState::Found,
            CoverageExtent::Partial,
            CoverageGapReason::ParserReject,
            ReconciliationState::Orphaned,
        )
        .await
        .expect("partial orphan is durable truth");
    let row = capability_execution_receipts::get(&fixture.pool, receipt.id)
        .await
        .expect("read")
        .expect("receipt exists");
    assert_eq!(row.observation_state, "found");
    assert_eq!(row.coverage_extent, "partial");
    assert_eq!(row.reconciliation_state, "orphaned");
}

#[tokio::test]
async fn partial_parser_cannot_silently_drop_one_of_ten_framed_records() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let receipt = fixture.begin_one_input_receipt().await;
    let artifact = fixture.stage_ten_record_witness(receipt.id).await;
    let error = fixture.close_with_only_nine_parser_members(receipt.id, artifact.id).await
        .expect_err("host framing exact set exposes dropped record");
    assert_eq!(error.code(), "TOOL_TRUTH_PARSER_CENSUS_INCOMPLETE");
    fixture.assert_partial_parser_reject_with_raw_preserved(receipt.id).await;
}

#[tokio::test]
async fn parser_ranges_and_typed_sources_are_an_exact_partition() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    for fault in [
        ParserCensusFault::Gap,
        ParserCensusFault::Overlap,
        ParserCensusFault::PastStoredBytes,
        ParserCensusFault::ParsedWithoutTypedSource,
        ParserCensusFault::TypedSourceRangeDrift,
        ParserCensusFault::IgnoredDiscoverableChild,
    ] {
        fixture.close_with_parser_fault(fault).await
            .expect_err("invalid partition must fail closed");
    }
    fixture.property_check_random_unknown_records_fail_partial().await;
}

#[tokio::test]
async fn witness_reconciliation_authenticates_server_owned_vault_object() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let receipt = fixture.begin_one_input_receipt().await;
    let artifact = fixture.stage_witness(receipt.id, b"typed-line\n").await;
    fixture.close_with_raw_range(receipt.id, artifact.id, 0..11).await.expect("close");
    fixture.reconcile(receipt.id).await.expect("consistent");

    fixture.tamper_vault_ciphertext(&artifact).await;
    let latest = fixture.reconcile(receipt.id).await.expect("tamper is durable orphan");
    assert_eq!(latest.semantic_authority_version, 2);
    assert_eq!(latest.reconciliation_state, "orphaned");
    assert_eq!(latest.reason_code.as_deref(), Some("TOOL_TRUTH_ARTIFACT_HASH_MISMATCH"));
}

#[tokio::test]
async fn concurrent_fresh_consumers_share_semantic_version_without_starvation() {
    let fixture = ToolTruthFixture::finalized_consistent().await;
    let before = fixture.receipt_authority_head().await;
    let (left, right) = tokio::join!(
        fixture.refresh_for_consumer("gate"),
        fixture.refresh_for_consumer("snapshot"),
    );
    let left = left.expect("gate authority set");
    let right = right.expect("snapshot authority set");
    assert_eq!(left.semantic_authority_set_hash, right.semantic_authority_set_hash);
    assert_ne!(left.freshness_attestation_set_hash, right.freshness_attestation_set_hash);
    assert_eq!(fixture.receipt_authority_head().await, before);
    fixture.assert_semantic_authority_versions(&[1]).await;
}

#[tokio::test]
async fn verified_vault_snapshot_closes_decrypt_consume_toc_tou_and_detects_later_tamper() {
    let fixture = ToolTruthFixture::finalized_consistent().await;
    let guarded = fixture.open_verified_vault_consumer_guard("gate").await.expect("verified snapshot");
    fixture.tamper_vault_ciphertext_after_snapshot().await;
    guarded.consume_asserting_original_bytes().await.expect("uses retained verified snapshot");

    let next = fixture.refresh_for_consumer("report").await.expect("fresh check");
    assert_eq!(next.current_state, ReconciliationState::Orphaned);
    assert_eq!(next.semantic_authority_version, 2);
    let replay = fixture.refresh_for_consumer("campaign").await.expect("same tamper");
    assert_eq!(replay.semantic_authority_version, 2);
    fixture.assert_semantic_authority_versions(&[1, 2]).await;
}

#[tokio::test]
async fn authority_set_seal_covers_root_and_all_derived_denominators_exactly() {
    let fixture = ToolTruthFixture::root_with_two_derived_denominators().await;
    let seal = fixture.refresh_for_consumer("campaign_admission").await.expect("sealed set");
    fixture.assert_authority_members_exact(&seal, &["root:a", "port:80", "port:443"]).await;
    fixture.forge_caller_reconciliation_hash(&seal).await
        .expect_err("caller hash is not authority");
    fixture.assert_no_public_snapshot_hash_or_member_vec_writer().await;
}

#[tokio::test]
async fn checked_bundle_covers_every_server_relevant_stage_root_exactly() {
    let fixture = ToolTruthFixture::operation_with_target_intel_eas_enum_and_vuln_roots().await;
    let bundle = fixture.checked_bundle_for_candidate_snapshot().await.expect("checked bundle");
    fixture.assert_bundle_roots_exact(&bundle, &["target_intel", "eas", "enumeration", "vuln"]).await;
    fixture.omit_or_cross_org_one_root_from_bundle()
        .await
        .expect_err("caller cannot define the relevant-root census");
}

#[tokio::test]
async fn stale_root_cannot_be_filtered_before_all_fresh_conversion() {
    let fixture = ToolTruthFixture::operation_with_one_expired_stage_root().await;
    let checked = fixture.checked_bundle_for_candidate_snapshot().await.expect("record stale census");
    fixture.assert_checked_bundle_has_exact_stale_member(&checked).await;
    fixture.try_all_fresh_bundle(&checked).expect_err("one stale root blocks authority");
    fixture.caller_filter_stale_root(&checked).expect_err("opaque exact bundle");
}

#[tokio::test]
async fn byte_fresh_but_temporally_expired_truth_cannot_authorize_a_consumer() {
    let fixture = ToolTruthFixture::consistent_checked_empty_with_short_negative_ttl().await;
    fixture.advance_server_clock_past_valid_until().await;
    let stale = fixture.refresh_for_consumer("candidate_snapshot").await
        .expect_err("byte integrity is not target-state freshness");
    assert_eq!(stale.code(), "TOOL_TRUTH_OBSERVATION_EXPIRED");
    fixture.assert_revalidation_obligation_and_exact_residual().await;
}

#[tokio::test]
async fn ttl_expiry_appends_hold_and_never_reuses_prior_allow() {
    let fixture = ToolTruthFixture::consistent_checked_empty_with_short_negative_ttl().await;
    let fresh = fixture.evaluate_tool_truth_gate().await.expect("fresh assessment");
    assert_eq!(fresh.control_decision, ControlDecision::Allow);
    fixture.advance_server_clock_past_valid_until().await;
    let expired = fixture.evaluate_tool_truth_gate().await.expect("expired assessment");
    assert_ne!(fresh.authority_set_seal_id, expired.authority_set_seal_id);
    assert_eq!(expired.control_decision, ControlDecision::Hold);
    assert_eq!(expired.temporal_validity_status, TemporalValidityStatus::Expired);
    fixture.assert_current_gate_assessment(expired.id).await;
}

#[tokio::test]
async fn mixed_target_epochs_or_excessive_skew_hold_the_exact_set() {
    let fixture = ToolTruthFixture::same_objective_observations_across_epochs().await;
    for fault in [TemporalFault::MixedTargetEpoch, TemporalFault::MaxSkewExceeded] {
        fixture.fresh_authority_set_with_fault(fault).await
            .expect_err("facts that never coexisted cannot be composed");
    }
}

#[tokio::test]
async fn every_exact_set_requires_open_members_then_seal() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    for kind in [
        ExactSetKind::RootDenominator,
        ExactSetKind::DerivedDenominator,
        ExactSetKind::DestinationPolicy,
        ExactSetKind::TemporalValidityPolicy,
        ExactSetKind::TemporalCensus,
        ExactSetKind::ParserCensus,
        ExactSetKind::DiscoveredChildManifest,
        ExactSetKind::DiscoveryOverflowManifest,
        ExactSetKind::AuthoritySet,
        ExactSetKind::AuthorityBundle,
    ] {
        fixture.assert_unsealed_cannot_be_consumed(kind).await;
        fixture.assert_direct_sql_add_modify_delete_after_seal_is_rejected(kind).await;
        fixture.assert_concurrent_exact_replay_or_drift_rejection(kind).await;
    }
}

#[tokio::test]
async fn operation_budget_cannot_be_oversubscribed_by_concurrent_roots() {
    let fixture = ToolTruthFixture::with_operation_and_root_discovery_limits(3, 2).await;
    let (left, right) = tokio::join!(
        fixture.admit_two_children_from_root("org-a/root-a"),
        fixture.admit_two_children_from_root("org-b/root-b"),
    );
    left.expect("one canonical admission may win");
    right.expect("the other is admitted plus overflow, not lost");
    fixture.assert_operation_unique_child_total(3).await;
    fixture.assert_every_unadmitted_child_has_overflow_member().await;
}

#[tokio::test]
async fn child_scope_classification_cannot_be_promoted_to_network_execution() {
    let fixture = ToolTruthFixture::mixed_first_and_third_party_children().await;
    fixture.classify_and_close_children().await.expect("scope classified");
    fixture.assert_only_in_scope_child_has_downstream_denominator().await;
    fixture.assert_external_and_out_of_scope_are_non_network_residuals().await;
    fixture.direct_sql_cross_scope_closure_mismatch()
        .await
        .expect_err("classification and closure are compound authority");
}

#[tokio::test]
async fn discovery_budget_records_overflow_instead_of_silently_truncating() {
    let fixture = ToolTruthFixture::with_discovery_limits(2, 4, 1).await;
    let manifest = fixture.discover_children_in_shuffled_completion_order(100).await;
    fixture.assert_admitted_children_are_canonical_and_bounded(&manifest, 2).await;
    fixture.assert_overflow_exactly_covers_remaining_parser_records(&manifest, 98).await;
    let assessment = fixture.evaluate_tool_truth().await;
    assert_ne!(assessment.coverage_grade, CoverageGrade::Complete);
    assert!(assessment.has_residual("TOOL_TRUTH_DISCOVERY_BUDGET_EXHAUSTED"));
}

#[tokio::test]
async fn duplicate_and_cyclic_discovery_cannot_expand_the_graph() {
    let fixture = ToolTruthFixture::with_discovery_limits(8, 32, 3).await;
    fixture.discover_cycle(&["a", "b", "a"]).await;
    fixture.assert_unique_nodes(&["a", "b"]).await;
    fixture.assert_cycle_overflow_member("a").await;
    fixture.assert_no_duplicate_denominator_or_receipt("a").await;
}

#[tokio::test]
async fn missing_artifact_or_wrong_raw_range_cannot_publish_complete() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    for fault in [WitnessFault::MissingVaultObject, WitnessFault::RangePastStoredBytes] {
        let receipt = fixture.begin_one_input_receipt().await;
        let error = fixture.close_and_reconcile_with_fault(receipt.id, fault).await
            .expect_err("witness fault must fail closed");
        assert!(error.to_string().contains("TOOL_TRUTH_RECONCILIATION_ORPHAN"));
    }
}

#[tokio::test]
async fn budget_plan_and_actual_are_separate_immutable_exact_sets() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let receipt = fixture.begin_http_receipt().await;
    fixture.assert_budget_plan_axes(receipt.id, &["requests", "response_bytes", "wall_clock_ms", "retries"]).await;
    fixture.assert_budget_plan_update_rejected(receipt.id).await;

    let first = fixture.close_with_observed_http_budget(receipt.id).await.expect("first close");
    let replay = fixture.close_with_observed_http_budget(receipt.id).await.expect("same close replay");
    assert_eq!(first.id, replay.id);
    fixture.close_with_missing_or_drifted_budget_axis(receipt.id).await
        .expect_err("required-axis drift must fail closed");
    fixture.assert_budget_observation_update_delete_rejected(receipt.id).await;
}

#[tokio::test]
async fn raw_artifact_and_source_members_are_append_only() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let receipt = fixture.begin_one_input_receipt().await;
    let artifact = fixture.stage_witness(receipt.id, b"line\n").await;
    fixture.assert_artifact_update_delete_rejected(artifact.id).await;
    fixture.assert_typed_source_update_delete_rejected(receipt.id).await;
}

#[tokio::test]
async fn witness_staging_is_idempotent_across_response_loss_and_shared_content() {
    let fixture = ToolTruthFixture::receipt_v1().await;
    let left = fixture.begin_one_input_receipt().await;
    let right = fixture.begin_one_input_receipt().await;
    let first = fixture.stage_witness(left.id, b"same bytes\n").await;
    fixture.simulate_artifact_row_response_loss(first.id).await;
    let replay = fixture.stage_witness(left.id, b"same bytes\n").await;
    assert_eq!(first.id, replay.id);

    let shared = fixture.stage_witness(right.id, b"same bytes\n").await;
    assert_eq!(first.content_key, shared.content_key);
    assert_eq!(first.vault_object_ref_token_hash, shared.vault_object_ref_token_hash);
    assert_ne!(first.id, shared.id);
    fixture.assert_artifact_receipt_authority(left.id, first.id).await;
    fixture.assert_artifact_receipt_authority(right.id, shared.id).await;
}
~~~

<code>ToolTruthFixture</code> 是该 integration test 文件中的完整 fixture helper，使用固定 UUID、local transaction 和当前 migration runner；不得访问真实目标或 provider。

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts)
~~~

**Expected:** test target 编译失败，因为新 repo command/row/API 尚不存在。

### Step 3：实现明确的 repo command 与 CAS

在新 repo 文件定义：

~~~rust
#[derive(Debug, Clone)]
pub struct SealCoverageDenominator {
    pub stable_seal_request_id: Uuid,
    pub stage_execution_id: Uuid,
    pub source: RootDenominatorSource,
}

#[derive(Debug, Clone)]
pub enum RootDenominatorSource {
    StageAssetWave { stage_asset_wave_id: Uuid },
    StageTeamUnit { stage_run_unit_id: Uuid },
}

#[derive(Debug, Clone)]
pub struct BeginCapabilityReceipt {
    pub id: Uuid,
    pub denominator_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub unit_id: Option<Uuid>,
    pub capability: String,
    pub attempt_epoch: DateTime<Utc>,
    pub attempt_ordinal: i32,
    pub temporal_validity_policy_id: Uuid,
    pub temporal_validity_policy_hash: String,
    pub target_scope_identity_hash: String,
    pub target_state_epoch_event_id: Uuid,
    pub target_state_epoch: i64,
    pub destination_policy_id: Uuid,
    pub destination_policy_hash: String,
    pub budget_axes: Vec<PlannedBudgetAxis>,
    pub input_keys: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PlannedBudgetAxis {
    pub axis: BudgetAxis,
    pub required_for_complete: bool,
    pub planned_limit: Option<i64>,
    pub observation_source: BudgetObservationSource,
}

#[derive(Debug, Clone)]
pub struct ActualBudgetAxis {
    pub axis: BudgetAxis,
    pub actual_value: Option<i64>,
    pub observed: bool,
    pub observation_source: BudgetObservationSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "capability", content = "landing", rename_all = "snake_case")]
pub enum CapabilityLandingV1 {
    ExternalAttackSurface(ExternalAttackSurfaceLandingV1),
    Enumeration(EnumerationLandingV1),
    JsApi(JsApiLandingV1),
    AnonymousAccess(AnonymousAccessLandingV1),
    NucleiSignal(NucleiSignalLandingV1),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusinessEvidenceRefV1 {
    Evidence { evidence_id: i64, evidence_hash: String },
    TargetAsset { asset_id: Uuid, source_hash: String },
    DnsRecord { record_id: Uuid, source_hash: String },
    HttpObservation { observation_id: Uuid, source_hash: String },
    Endpoint { endpoint_id: Uuid, source_hash: String },
}

#[derive(Debug, Clone)]
pub struct StageCapabilityCloseout {
    pub receipt_id: Uuid,
    pub expected_row_version: i64,
    pub raw_witness_artifact_id: Uuid,
    pub actual_budget_axes: Vec<ActualBudgetAxis>,
    pub status: ReceiptCoverageFact,
    pub typed_landing: CapabilityLandingV1,
    pub parser_census: HostFramedParserCensusInput,
    pub typed_source_members: Vec<TypedLandingSourceMember>,
    pub input_statuses: Vec<ReceiptInputCloseout>,
}

// All fields and constructors below are private to the raw-vault host module.
// None implements Clone/Serialize and each lifetime is created only while the
// vault retains the authenticated plaintext snapshot or sealed-write handle.
struct VerifiedVaultWrite<'guard> {
    _guard: PhantomData<&'guard ()>,
}

struct VerifiedVaultSnapshot<'guard> {
    _guard: PhantomData<&'guard ()>,
}

struct VerifiedVaultSnapshotCensus<'guard> {
    // Exact receipt membership is derived from the locked denominator graph;
    // callers cannot submit hashes, sizes, roots, or a Vec of observations.
    _guard: PhantomData<&'guard ()>,
}

struct ServerDerivedRootAuthorityCensus<'guard> {
    _guard: PhantomData<&'guard ()>,
}

struct ServerDerivedRelevantRootCensus<'guard> {
    _guard: PhantomData<&'guard ()>,
}

struct VerifiedVaultBundleSnapshotCensus<'guard> {
    // Created by walking every member of ServerDerivedRelevantRootCensus in
    // the same callback; its exact root/set/member mapping is private.
    _guard: PhantomData<&'guard ()>,
}

struct ReconciliationExpectation {
    reconciliation_id: Uuid,
    receipt_id: Uuid,
    expected_row_version: i64,
    expected_predecessor_reconciliation_id: Option<Uuid>,
    expected_semantic_authority_version: i64,
}

pub struct CheckedToolTruthAuthoritySet<'guard> {
    // exact single-root census; may contain stale/orphan member dispositions.
    _guard: PhantomData<&'guard ()>,
}

pub struct FreshToolTruthAuthoritySet<'guard> {
    // fields and constructor are private; the lifetime is tied to retained
    // verified vault snapshots and cannot cross requests or be cloned.
    _guard: PhantomData<&'guard ()>,
}

pub struct CheckedToolTruthAuthorityBundle<'guard> {
    // exact server-derived multi-root census; preserves fresh/stale/orphan
    // member dispositions and cannot be caller-filtered.
    _guard: PhantomData<&'guard ()>,
}

pub struct AllFreshToolTruthAuthorityBundle<'guard> {
    // private constructor succeeds only when every required root/set/member
    // in the checked bundle is semantic-consistent and temporally fresh.
    _guard: PhantomData<&'guard ()>,
}

pub struct StaleToolTruthAuthoritySetReceipt {
    pub authority_set_seal_id: Uuid,
    pub temporal_validity_status: TemporalValidityStatus,
    pub affected_receipt_ids: Vec<Uuid>,
    pub revalidation_obligation_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct SealDiscoveredChildManifest {
    pub id: Uuid,
    pub parent_receipt_id: Uuid,
    pub parent_denominator_item_id: Uuid,
    pub child_kind: String,
    pub expected_downstream_technique: String,
    pub expected_downstream_capability: String,
    pub child_manifest_hash: String,
    pub members: Vec<DiscoveredChildMemberInput>,
}

#[derive(Debug, Clone)]
pub struct CloseDiscoveredChildMember {
    pub id: Uuid,
    pub child_member_id: Uuid,
    pub closure: DiscoveredChildClosure,
    pub closure_hash: String,
}

#[derive(Debug, Clone)]
pub struct SealDerivedChildDenominator {
    pub stable_seal_request_id: Uuid,
    pub parent_child_manifest_id: Uuid,
}
~~~

`BudgetAxis`闭集为`requests/response_bytes/wall_clock_ms/retries/browser_steps/oast_tokens`；`BudgetObservationSource`闭集为`host_governor/adapter_instrumentation/cli_unobserved`。每个capability contract冻结required axis exact set与manifest hash；closeout必须逐轴exact match，不接受单个`observed=true`概括全部预算。HTTP adapter至少要求requests、response bytes、wall clock和retries；browser/OAST能力分别追加browser steps/OAST tokens。任一required axis未观测或超出已批准上界均不能得到complete coverage，并产生typed residual。

`TypedLandingSourceMember`逐observation绑定`input_key + raw byte range`或server-control authority。stage-closeout command只接收server staging返回的artifact ID，repo重读artifact row并验证其属于同一receipt；不能让caller自报path/hash。

public consumer API与sealed host submodule边界固定为：

~~~rust
pub async fn seal_denominator(
    pool: &PgPool,
    command: &SealCoverageDenominator,
) -> Result<CoverageDenominatorRow>;

pub async fn begin(
    pool: &PgPool,
    command: &BeginCapabilityReceipt,
) -> Result<CapabilityExecutionReceiptRow>;

pub async fn stage_closeout(
    pool: &PgPool,
    command: &StageCapabilityCloseout,
) -> Result<CapabilityExecutionReceiptRow>;

// The following functions live in the sealed raw-vault authority submodule.
// They are module-private and are callable only inside a vault callback that
// retains the matching write/snapshot guard for the full operation.
async fn stage_raw_witness_artifact_on(
    tx: &mut Transaction<'_, Postgres>,
    receipt_id: Uuid,
    verified: &VerifiedVaultWrite<'_>,
) -> Result<RawWitnessArtifactRow>;

async fn finalize_reconciliation_on(
    tx: &mut Transaction<'_, Postgres>,
    expected: &ReconciliationExpectation,
    snapshot: &VerifiedVaultSnapshot<'_>,
) -> Result<CapabilityExecutionReceiptRow>;

async fn attest_and_seal_authority_set_on<'guard>(
    tx: &mut Transaction<'_, Postgres>,
    root: &ServerDerivedRootAuthorityCensus<'guard>,
    snapshots: &VerifiedVaultSnapshotCensus<'guard>,
) -> Result<CheckedToolTruthAuthoritySet<'guard>>;

async fn attest_and_seal_authority_bundle_on<'guard>(
    tx: &mut Transaction<'_, Postgres>,
    checked_roots: &ServerDerivedRelevantRootCensus<'guard>,
    snapshots: &VerifiedVaultBundleSnapshotCensus<'guard>,
) -> Result<CheckedToolTruthAuthorityBundle<'guard>>;

pub async fn seal_discovered_child_manifest(
    pool: &PgPool,
    command: &SealDiscoveredChildManifest,
) -> Result<DiscoveredChildManifestRow>;

pub async fn close_discovered_child_member(
    pool: &PgPool,
    command: &CloseDiscoveredChildMember,
) -> Result<DiscoveredChildClosureRow>;

pub async fn seal_derived_child_denominator(
    pool: &PgPool,
    command: &SealDerivedChildDenominator,
) -> Result<CoverageDenominatorRow>;

pub async fn get_audit_only(
    pool: &PgPool,
    receipt_id: Uuid,
) -> Result<Option<AuditOnlyCapabilityExecutionReceiptRow>>;

async fn list_checked_facts_on(
    tx: &mut Transaction<'_, Postgres>,
    authority: &CheckedToolTruthAuthoritySet<'_>,
) -> Result<Vec<ReceiptCoverageFact>>;

async fn insert_gate_assessment_on(
    tx: &mut Transaction<'_, Postgres>,
    authority: &CheckedToolTruthAuthoritySet<'_>,
    decision: &ToolTruthGateDecision,
) -> Result<ToolTruthGateAssessmentRow>;
~~~

实现规则：

- <code>seal_denominator</code> 的调用方只提交`stable_seal_request_id + stage_execution_id + tagged frozen wave/unit identity`。repo在同一transaction按固定锁序锁住stage run、sealed wave/unit source census、operation/org/current attempt与版本化StageSpec/capability registry，重读authoritative assets，server-side计算完整`asset × applicable technique`集合、item ids、count、`input_manifest_hash`与`authority_hash`，再执行open→members→seal。public DTO没有items/count/hash/operation/org/stage/epoch字段；相同stable request与同一source census exact replay原row，request重绑、source seal漂移或registry漂移返回<code>TOOL_TRUTH_DENOMINATOR_DRIFT</code>。这样不存在“先读assets、caller组Vec、再seal”的TOCTOU或漏项旁路。derived denominator同理只接收stable request与sealed parent child-manifest id，repo从eligible manifest members和budget ledger自行推导ordinal、assignments与hash。
- <code>begin</code> 验证input keys与denominator exact set相等；`authority_hash/input_manifest_hash`只从locked sealed denominator复制，command不再携带这两个字段。execution/replay identity必须包含`denominator_id`，并重验该denominator的operation/org/stage/attempt scope。相同execution key/hash返回原receipt；两个parent manifest即使使用相同downstream capability和attempt ordinal也必须得到互不串线的receipt。
- raw artifact staging没有public command。`RawWitnessVaultPort`先把canonical plaintext在`Zeroizing`有界内存中做per-operation envelope encryption；持久化成功后只在同一vault callback产生不可构造、不可Clone/Serialize的`VerifiedVaultWrite<'guard>`。module-private `stage_raw_witness_artifact_on`从guard复制artifact/content/ciphertext/plaintext-hash/size/key-generation/retention attestation并锁定同一receipt、operation/org/current attempt；调用方不能提交vault ref token、hash、size、key或retention metadata。相同receipt/content attestation exact replay原row，任何vault attestation drift返回`TOOL_TRUTH_ARTIFACT_METADATA_DRIFT`。
- `stage_closeout`用`WHERE id=$1 AND row_version=$2` CAS写typed landing/source members/budget observations/input census，但top receipt仍保持nonterminal `reconciliation_state=pending`，不得先写`coverage_extent=complete`。逐input census必须exact-equal；旧epoch改写为superseded，不发布current terminal truth。它只能引用由`stage_witness`为同一receipt创建的server-owned artifact row，并校验typed source member的raw range不越过`stored_byte_count`。
- `stage_witness`是两层幂等：vault以`operation + content key`执行encrypted object noclobber；existing opaque object也必须AEAD验证、解密到有界`Zeroizing`buffer并重算plaintext hash/size后才能复用，不能因`AlreadyExists`直接成功。artifact row按deterministic `(receipt_id, content_key)` identity做`INSERT ... ON CONFLICT` exact replay。不同receipt可共享同一个encrypted opaque object/ref-token identity，但artifact row和close authority绝不共享；任何外部DTO只返回artifact id、content key、ref-token hash，内部后续操作只持不可解引用的sealed ref token，绝不返回vault object key。
- 每次finalize或consumer check都由vault按DB artifact id在内部解封`vault_object_ref_token`，验证ciphertext hash、AEAD tag、operation key generation、retention/erasure状态，再把plaintext解密到request-private bounded memory或无pathname sealed-memory handle；前后重验vault object version/etag与plaintext hash/size，成功后在callback lifetime内创建`VerifiedVaultSnapshot<'guard>`。snapshot字段与bytes均private，离开callback立即zeroize/关闭；任何普通filesystem path、workspace-relative object key、filesystem traversal流程或持久化plaintext临时文件都不属于本contract。module-private `finalize_reconciliation_on`只接受该guard与CAS identity，短transaction从guard和DB自行推导artifact/typed source/parser/budget/evidence/business/current authority tuple，创建semantic authority version并原子更新top receipt；caller没有raw hash/size/authority参数。只有consistent才能同事务写complete，其他路径写partial/orphaned。
- reconciliation被拆成**稳定semantic authority**与**request-scoped freshness attestation**。`capability_execution_reconciliations.semantic_authority_version/hash`只在`(state, observed artifact hash/size, authority hash, typed source/parser/budget/evidence/business membership)`发生语义变化时追加并CAS top receipt；同一tuple的再次检查复用原semantic reconciliation，不递增receipt row version、不推进projection entity version。每个consumer request仍追加自己的freshness attestation，但attestation id/time/file identity不进入receipt/report/projection semantic hash。
- consumer不再各自调用可复用`refresh_reconciliation` token。stage-local Gate使用host-owned `with_checked_tool_truth_authority_set`：host在同一transaction从locked root+derived graph推导`ServerDerivedRootAuthorityCensus<'guard>`，再由vault逐receipt产生exact `VerifiedVaultSnapshotCensus<'guard>`；module-private sealer不接收root id、member Vec、hash或size。它按receipt排序保留全部verified snapshots、用DB clock重验temporal policy并seal exact set，返回不可caller过滤的`CheckedToolTruthAuthoritySet<'guard>`；它可包含fresh/expired/orphan以便同事务写HOLD/residual。只有exact members全semantic-consistent且temporal-fresh时，private conversion才产生`FreshToolTruthAuthoritySet<'guard>`。
- Plan B snapshot、Plan C admission/closeout和Plan D current report跨多个stage/root，必须使用`with_checked_tool_truth_authority_bundle`：host先从consumer spec与operation facts生成不可caller指定的`ServerDerivedRelevantRootCensus<'guard>`，随后在**同一个vault callback、DB transaction与guard lifetime**由vault按root/receipt canonical order自行遍历全部成员并生成`VerifiedVaultBundleSnapshotCensus<'guard>`，再调用module-private bundle sealer。sealer API不接`&[CheckedSet]`、root/member Vec或单独hash，并重验两个opaque census exact绑定；漏一个set/member会让callback整体失败而不是得到较小bundle。`CheckedToolTruthAuthorityBundle<'guard>`始终保留所有root/member disposition，让B可以原子记录stale census与revalidation residual；caller不能先删掉坏root。只有bundle exact set全部fresh才private-convert为`AllFreshToolTruthAuthorityBundle<'guard>`，后者才可授权C action/revision verdict或D current finalization/reuse。相同stable request/hash exact replay原seal，不同root census/payload drift拒绝。
- 这些opaque guard均不可Clone/Serialize/由caller构造，不能跨request缓存；`VerifiedVaultWrite/Snapshot/SnapshotCensus`与root census的类型、字段、constructor及stage/finalize/attest函数都在同一个sealed host module内private，crate其他module也没有`pub(crate)`旁路。consumer只能使用与verified snapshot绑定的typed facts/bytes。`tool_truth_gate_assessments`没有public caller-write seam：只有checked-set callback内的module-private `insert_gate_assessment_on(tx, &CheckedToolTruthAuthoritySet, decision)`能写；repo复制seal/semantic/freshness/temporal/epoch/window hashes，caller不能传裸`reconciliation_state/hash/status/time`或自己拼assessment。
- repo不暴露裸`list_current_facts(pool, denominator_id)`权威旁路。`list_checked_facts_on(tx, &CheckedToolTruthAuthoritySet)`仅在guard callback内可见；C/D要求all-fresh bundle的constructor也只接受opaque guard。公开`get_audit_only`返回带`AuditOnly` marker且已移除`vault_object_ref_token`的历史DTO，只含token hash，不能转换成Candidate snapshot/Campaign/report authority。compile-time constructor tests与source usage scan必须证明receipt_v1 consumer没有pool+denominator裸读入口。
- 两个并发consumer观察相同tuple时都可成功：各有attestation/set seal，但引用同一semantic version，互不使token stale，也不产生projection churn。首次观察到tamper时exact-one追加新orphan semantic version并使top authority HOLD；随后仍观察相同tamper只复用该version，不重复生成C quarantine/correction。若内容/authority以后再次变化才创建下一semantic version。consistent之后的tamper必须被下一consumer发现；已依据旧version形成的Campaign/report由Plan C/D quarantine/supersession处理，不因文件恢复而自动复活。
- 这套guard证明“本次consumer实际使用的是刚刚完成AEAD验证与plaintext重算的稳定vault snapshot”，不是声称opaque object永久不可变。若未来改成可证明immutable object store，仍必须用新contract version与迁移明确替代，不得静默跳过fresh guard。
- `seal_discovered_child_manifest`只消费latest consistent parent receipt、host-framed parser census和typed landing；按capability contract验证expected child-kind exact set、member count/hash/ordinal且同值幂等。它先记录candidate manifest，不按provider完成时刻直接dispatch。breadth-first admission barrier在同depth全部parent terminal后统一canonical-sort、锁discovery budget head，原子写deduplicated nodes、admitted derived denominator/items、overflow manifests/members和budget ledger；非空admitted set必须在任何downstream provider dispatch前完成`seal_derived_child_denominator`。root denominator永不重seal/追加。`close_discovered_child_member`用于`downstream_denominator_item/not_applicable/blocked/deduplicated_existing` exact closure；repo验证operation/org/attempt/technique/capability/node/path/depth identity。跨attempt、漏/额外parser member、cycle、超深、重复execution、hash drift或把root item当child target均拒绝。
- coverage denominator、destination policy/member、temporal validity policy/census/member、parser census、dynamic child/overflow manifest、receipt-input与reconciliation evidence/business-ref lineage、authority-set与authority-bundle等header/member集合安装open→members→seal trigger；revalidation dispatch policy是operation-frozen immutable row；`capability_raw_witness_artifacts`、raw access/retention events、typed source、network-hop receipt、budget plan/observation、operation/root discovery budget contract与ledger、target-state epoch event、revalidation obligation/dispatch event、child closure及freshness attestation安装append-only trigger。两层discovery budget head、target-state epoch head、revalidation obligation/dispatch head与top receipt semantic head只允许受guard、expected-version和typed event/outbox约束的CAS；同tuple refresh不更新semantic head。新的sealed semantic reconciliation只能追加，不能覆写旧orphan事实。测试矩阵必须含policy/census/member post-seal append、typed landing unknown/extra/control/oversize与direct-SQL validator、lineage cross-org/orphan/duplicate/omission、raw plaintext absence/access audit/crypto-erasure、temporal TTL/class/epoch direct-SQL伪造、multi-root omission/filtering、read/inactive/T2-T3 zero-dispatch、hold-generation replay、hop UPDATE/DELETE、budget contract drift/ledger mutation、epoch/revalidation event-head mismatch以及authority set/bundle post-seal append。
- response-loss 重试返回已有 terminal receipt，不重新执行 capability。
- 错误 code 至少包含 <code>TOOL_TRUTH_CONTRACT_INVALID</code>、<code>TOOL_TRUTH_AUTHORITY_STALE</code>、<code>TOOL_TRUTH_MANIFEST_DRIFT</code>、<code>TOOL_TRUTH_INPUT_CENSUS_INCOMPLETE</code>、<code>TOOL_TRUTH_LANDING_PARTIAL</code>、<code>TOOL_TRUTH_RECONCILIATION_ORPHAN</code>。

### Step 4：运行 GREEN

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts)
~~~

**Expected:** integration test target全绿，exit code 0；除begin/CAS外，明确证明vault object missing/tamper/wrong offset只能追加orphan；同语义fresh检查不推进receipt/projection版本、并发consumer不starve；verified vault snapshot关闭AEAD-decrypt→consume TOCTOU；root+derived authority set exact封存；budget plan与actual分离且exact/immutable，同值重放幂等、漂移拒绝。

### Step 5：Future Commit

~~~bash
git add backend/crates/golish-db/src/repo/capability_execution_receipts.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/tests/capability_execution_receipts.rs
git commit -m "feat(tool-truth): persist execution receipts and reconciliation"
~~~

---

## Task 4：建立 producer lifecycle、durable raw witness 与 request governor

**文件：**

- 创建：<code>backend/crates/golish-pentest-app/src/pentest_bridge/tool_truth.rs</code>
- 创建：<code>backend/crates/golish-pentest-app/src/pentest_bridge/raw_witness_vault.rs</code>
- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs</code>
- 测试：<code>backend/crates/golish-pentest-app/src/pentest_bridge/tool_truth.rs</code> 内联测试

### Step 1：写 RED raw witness 与 N+1 测试

~~~rust
#[tokio::test]
async fn raw_witness_is_atomic_hash_bound_and_bounded() {
    let workspace = tempfile::tempdir().expect("workspace");
    let encrypted_vault = RawWitnessVaultFixture::for_workspace(workspace.path()).await;
    let receipt_id = Uuid::new_v4();
    let stdout = b"{\"matched\":\"CVE-TEST\"}\n";
    let stderr = b"parser rejected line 1\n";
    let witness = persist_raw_witness(
        encrypted_vault.host(),
        &encrypted_vault.operation_authority(),
        receipt_id,
        stdout,
        stderr,
    )
    .await
    .expect("persist witness");
    assert_eq!(
        witness.original_byte_count,
        RAW_WITNESS_ENVELOPE_BYTES + stdout.len() + stderr.len(),
    );
    assert_eq!(witness.stored_byte_count, witness.original_byte_count);
    assert_eq!(witness.sha256.len(), 64);
    assert_eq!(witness.encryption_contract_version, "raw_witness_envelope.v1");
    assert_eq!(witness.ciphertext_sha256.len(), 64);
    assert_eq!(witness.vault_object_ref_token_hash.len(), "sha256:".len() + 64);
    encrypted_vault.assert_external_ref_has_no_object_key_or_filesystem_path(&witness);
    encrypted_vault.assert_no_plaintext_bytes(stdout);
}

#[test]
fn raw_vault_authority_seam_exposes_no_object_key_or_raw_attestation_writer() {
    assert_public_type_has_no_field::<RawWitnessArtifactRef>("vault_object_ref_token");
    assert_public_api_has_no_function("stage_raw_witness_artifact");
    assert_public_api_has_no_function("finalize_reconciliation");
    assert_module_private_guard_has_no_clone_or_serialize::<VerifiedVaultSnapshot<'static>>();
    assert_bundle_sealer_accepts_only_server_root_and_vault_censuses();
}

#[tokio::test]
async fn raw_witness_access_is_separately_authorized_audited_and_crypto_erasable() {
    let fixture = RawWitnessVaultFixture::with_operation_retention_policy().await;
    let witness = fixture.persist_secret_and_pii_fixture().await;
    assert_eq!(witness.sensitivity_disposition, RawWitnessSensitivityDisposition::SecretOrPiiQuarantined);
    fixture.assert_normal_analysis_uses_typed_derivative_only().await;
    fixture.assert_unauthorized_view_denied_and_audited().await;
    fixture.crypto_erase_after_retention().await;
    fixture.assert_ciphertext_unrecoverable_but_hash_and_provenance_retained().await;
}

#[tokio::test]
async fn truncated_tail_signal_cannot_be_complete_or_proof() {
    let workspace = tempfile::tempdir().expect("workspace");
    let encrypted_vault = RawWitnessVaultFixture::for_workspace(workspace.path()).await;
    let mut stdout = vec![b'a'; MAX_RAW_WITNESS_BYTES + 64];
    stdout.extend_from_slice(b"TAIL_ONLY_SECURITY_MATCH");
    let witness = persist_raw_witness(
        encrypted_vault.host(),
        &encrypted_vault.operation_authority(),
        Uuid::new_v4(),
        &stdout,
        b"",
    )
    .await
    .expect("persist truncated witness");
    assert!(witness.truncated);

    let normalized = enforce_witness_safety_floor(
        &witness,
        attempted_complete_proof_closeout(),
    );
    assert_eq!(normalized.landing_state, LandingState::Partial);
    assert_eq!(normalized.coverage_extent, CoverageExtent::Partial);
    assert_eq!(normalized.coverage_gap_reason, CoverageGapReason::SourceUnavailable);
    assert_eq!(normalized.reconciliation_state, ReconciliationState::Orphaned);
    assert!(!matches!(
        normalized.security_interpretation,
        SecurityInterpretation::Proof | SecurityInterpretation::Refutation
    ));
}

#[test]
fn request_budget_guard_rejects_n_plus_one_before_send() {
    let budget = RequestBudgetGuard::new(2);
    assert_eq!(budget.reserve_before_send().expect("first"), 1);
    assert_eq!(budget.reserve_before_send().expect("second"), 2);
    let error = budget
        .reserve_before_send()
        .expect_err("third request must fail before transport");
    assert_eq!(error.code(), "TOOL_TRUTH_BUDGET_EXHAUSTED");
    assert_eq!(budget.actual_request_count(), 2);
}

#[test]
fn cli_internal_budget_is_never_claimed_as_observed() {
    let budget = ActualBudget::cli_process(&[
        BudgetAxis::Requests,
        BudgetAxis::ResponseBytes,
        BudgetAxis::WallClockMs,
        BudgetAxis::Retries,
    ]);
    assert_eq!(budget.axes.len(), 4);
    assert!(budget.axes.iter().all(|axis| !axis.observed));
    assert!(budget.axes.iter().all(|axis| axis.actual_value.is_none()));
}

#[tokio::test]
async fn host_transport_blocks_redirect_or_rebind_outside_exact_policy_before_send() {
    for fault in [
        DestinationFault::RedirectToMetadata,
        DestinationFault::MixedPublicAndPrivateDns,
        DestinationFault::RetryRebindToLoopback,
        DestinationFault::AmbientProxyPresent,
        DestinationFault::TlsHostnameMismatch,
    ] {
        let transport = ScriptedDestinationTransport::with_fault(fault);
        let result = transport.execute(exact_http_policy()).await;
        assert_eq!(result.trusted_send_count_after_block(), 0);
        assert!(result.has_policy_residual());
    }
}

#[test]
fn unmanaged_hostname_cli_is_blocked_before_spawn_in_receipt_v1() {
    let runner = CountingProcessRunner::default();
    let result = execute_receipt_v1_cli(
        &runner,
        unmanaged_nuclei_hostname_policy(),
    );
    assert_eq!(result.governance_status, DestinationGovernanceStatus::PolicyBlocked);
    assert_eq!(runner.spawn_count(), 0);
}

#[tokio::test]
async fn provider_target_input_cannot_choose_transport_destination() {
    let transport = ScriptedProviderTransport::fixed_allowlist();
    transport
        .execute(provider_request_with_target("http://169.254.169.254/latest/meta-data"))
        .await
        .expect("target is escaped provider parameter only");
    assert_eq!(transport.destination_hosts(), vec!["fixed-provider.example"]);
    assert_eq!(transport.redirect_send_count(), 0);
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(raw_witness_is_atomic_hash_bound_and_bounded) | test(raw_vault_authority_seam_exposes_no_object_key_or_raw_attestation_writer) | test(raw_witness_access_is_separately_authorized_audited_and_crypto_erasable) | test(truncated_tail_signal_cannot_be_complete_or_proof) | test(request_budget_guard_rejects_n_plus_one_before_send) | test(cli_internal_budget_is_never_claimed_as_observed) | test(host_transport_blocks_redirect_or_rebind_outside_exact_policy_before_send) | test(unmanaged_hostname_cli_is_blocked_before_spawn_in_receipt_v1) | test(provider_target_input_cannot_choose_transport_destination)')
~~~

**Expected:** 编译失败，缺少raw witness、budget guard、destination policy/transport与lifecycle类型。

### Step 3：实现 bounded raw witness

使用host-owned `RawWitnessVaultPort`。明文只在有界`Zeroizing`内存或无pathname sealed-memory handle中组装并立刻做per-operation envelope encryption；持久层只保存vault-owned sealed ref token、ciphertext和hash，不存在plaintext filesystem path：

~~~rust
pub const MAX_RAW_WITNESS_BYTES: usize = 1_048_576;
pub const STDOUT_HEADER: &[u8] = b"capability_raw_witness.v1\n--- stdout ---\n";
pub const STDERR_HEADER: &[u8] = b"\n--- stderr ---\n";
pub const RAW_WITNESS_ENVELOPE_BYTES: usize = STDOUT_HEADER.len() + STDERR_HEADER.len();

#[derive(Debug, PartialEq, Eq)]
pub struct RawWitnessArtifactRef {
    pub artifact_id: Uuid,
    pub content_key: String,
    pub vault_object_ref_token_hash: String,
    pub sha256: String,
    pub ciphertext_sha256: String,
    pub encryption_contract_version: String,
    pub operation_key_ref_hash: String,
    pub key_generation: u64,
    pub retention_policy_id: Uuid,
    pub retention_policy_hash: String,
    pub sensitivity_disposition: RawWitnessSensitivityDisposition,
    pub original_byte_count: usize,
    pub stored_byte_count: usize,
    pub truncated: bool,
}

pub async fn persist_raw_witness(
    host: &RawWitnessAuthorityHost,
    operation: &OperationRawWitnessAuthority,
    receipt_id: Uuid,
    stdout: &[u8],
    stderr: &[u8],
) -> anyhow::Result<RawWitnessArtifactRef> {
    let original_byte_count = RAW_WITNESS_ENVELOPE_BYTES + stdout.len() + stderr.len();
    let content_limit = MAX_RAW_WITNESS_BYTES.saturating_sub(RAW_WITNESS_ENVELOPE_BYTES);
    let stdout_limit = stdout.len().min(content_limit / 2);
    let stderr_limit = stderr
        .len()
        .min(content_limit.saturating_sub(stdout_limit));
    let mut payload = Vec::with_capacity(
        STDOUT_HEADER.len() + stdout_limit + STDERR_HEADER.len() + stderr_limit,
    );
    payload.extend_from_slice(STDOUT_HEADER);
    payload.extend_from_slice(&stdout[..stdout_limit]);
    payload.extend_from_slice(STDERR_HEADER);
    payload.extend_from_slice(&stderr[..stderr_limit]);
    let truncated = stdout_limit != stdout.len() || stderr_limit != stderr.len();
    let sha256 = sha2::Sha256::digest(&payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let stored_byte_count = payload.len();
    let content_key = format!("sha256:{sha256}");
    let artifact_id = uuid::Uuid::new_v5(&receipt_id, content_key.as_bytes());
    let sensitivity_disposition = classify_raw_witness_sensitivity(&payload);
    host
        .seal_and_stage_verified(RawWitnessSealRequest {
            operation_id: operation.operation_id,
            receipt_id,
            artifact_id,
            content_key: content_key.clone(),
            plaintext_sha256: sha256.clone(),
            plaintext: Zeroizing::new(payload),
            operation_key_ref: operation.key_ref.clone(),
            key_generation: operation.key_generation,
            retention_policy: operation.retention_policy.clone(),
            sensitivity_disposition,
            original_byte_count,
            stored_byte_count,
            truncated,
        })
        .await
}
~~~

`RawWitnessAuthorityHost::seal_and_stage_verified`是唯一write入口：它让vault在callback内产生`VerifiedVaultWrite<'guard>`，并在guard仍存活时调用module-private repo compound写`capability_raw_witness_artifacts`，最后才降级成上面的`RawWitnessArtifactRef`。外部ref没有object key、path、raw vault row或可伪造attestation，只保留ref-token hash；若内部流程需要继续引用vault对象，只能携带不可Serialize、不可Clone、不可解引用的private ref token。vault必须对新写与existing opaque-object exact replay都先AEAD验证、解密到`Zeroizing`有界buffer并重算plaintext hash/size，不能信token、object key或DB metadata；object-exists/no-row、row-exists/object-exists和并发noclobber走同一exact replay。每个operation冻结retention policy与独立envelope key generation，实际key只存系统keyring/KMS等secret authority，DB和DTO只保存key-ref hash。不得把raw body写入日志、tool result、普通Agent context、frontend、report或telemetry；adapter API只传播artifact ID、content key、ref-token hash、plaintext/ciphertext hash、同一canonical-artifact计量域的original/stored size、truncated与sensitivity disposition，adapter不能拼artifact row或拿到object key。

普通分析只消费由sealed parser census产生的typed/redacted derivative；raw viewer是独立local-operator权限面，每次allow/deny都写append-only access event且禁止把bytes放入DOM hidden field/clipboard telemetry。secret/PII classifier命中时raw保持quarantined，除显式审计查看外不可解密。retention到期不删除ledger row：host写`crypto_erased` event并销毁对应operation key generation，使ciphertext不可恢复，同时保留plaintext hash、typed derivative、provenance、count与erasure receipt。任何项目现有store只有在**持久层始终是ciphertext**、提供同等级的per-operation envelope encryption、访问审计和crypto-erasure时才可实现该port；private目录本身不是安全保证，也绝不允许保存plaintext witness、plaintext content-addressed object或带明文的稳定快照。

`enforce_witness_safety_floor`是close前的host-owned纯函数：只要`truncated=true`，无论完整stdout parser产生什么，都强制`landing=partial`、`coverage=partial`、`gap=source_unavailable`、`reconciliation=orphaned`，并把interpretation限制为`signal|inconclusive`；不得产生checked-empty、proof、refutation或terminal complete。typed tail observation可作为quarantined signal保留，但因canonical raw artifact不可重放而不能成为Finding/negative oracle authority。

### Step 4：实现 request governor 与 lifecycle

~~~rust
#[derive(Debug)]
pub struct RequestBudgetGuard {
    limit: u64,
    sent: std::sync::atomic::AtomicU64,
}

impl RequestBudgetGuard {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            sent: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn reserve_before_send(&self) -> Result<u64, ToolTruthExecutionError> {
        self.sent
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| (current < self.limit).then_some(current + 1),
            )
            .map(|previous| previous + 1)
            .map_err(|actual| ToolTruthExecutionError::BudgetExhausted {
                limit: self.limit,
                actual,
            })
    }

    pub fn actual_request_count(&self) -> u64 {
        self.sent.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActualBudget {
    pub axes: Vec<ActualBudgetAxis>,
}

impl ActualBudget {
    pub fn cli_process(required_axes: &[BudgetAxis]) -> Self {
        Self {
            axes: required_axes
                .iter()
                .copied()
                .map(|axis| ActualBudgetAxis {
                    axis,
                    actual_value: None,
                    observed: false,
                    observation_source: BudgetObservationSource::CliUnobserved,
                })
                .collect(),
        }
    }
}
~~~

定义 <code>ToolTruthExecution</code>，公开的内部方法固定为：

~~~rust
impl ToolTruthExecution {
    pub async fn begin(
        pool: Arc<PgPool>,
        raw_witness_host: Arc<RawWitnessAuthorityHost>,
        contract: ToolTruthContract,
        command: BeginCapabilityReceipt,
    ) -> anyhow::Result<Option<Self>>;

    pub fn request_budget(&self) -> Option<&RequestBudgetGuard>;

    pub async fn stage_witness(
        &mut self,
        stdout: &[u8],
        stderr: &[u8],
    ) -> anyhow::Result<RawWitnessArtifactRef>;

    pub async fn close_and_reconcile(
        self,
        status: ReceiptCoverageFact,
        typed_landing: CapabilityLandingV1,
        input_statuses: Vec<ReceiptInputCloseout>,
    ) -> anyhow::Result<CapabilityExecutionReceiptRow>;
}
~~~

行为：

- <code>legacy_v1</code> 的 <code>begin</code> 返回 <code>Ok(None)</code>。
- shadow/receipt都写begin/witness/stage-closeout/stable-snapshot/atomic semantic finalize；任何consumer再走fresh authority-set guard。shadow不改变legacy dispatch，但destination status固定uncontrolled；receipt-v1在spawn/send前必须有enforced policy，否则写policy-blocked receipt而零外部调用。
- response-loss 读到 terminal receipt 时直接返回旧 row，调用方不得再次启动外部工具。
- managed/background completion 必须持有 receipt id；不能靠 wrapper 名称重建 receipt。
- `close_and_reconcile`从sealed parser/typed-source census与`CapabilityLandingV1`自行derive evidence/business-ref exact members；caller没有裸ID/JSON参数。unknown tagged variant/version、field bound或canonical round-trip失败在写semantic authority前停止。
- direct HTTP/provider adapter必须只拿`ToolTruthPinnedTransportV1`，由host逐hop执行destination/DNS/pin/redirect/proxy/TLS policy并记录requests、response bytes、wall-clock与retries；每次<code>.send()</code>前预留request/retry axis，response body采用bounded stream逐chunk累计bytes，deadline由host monotonic clock记录。
- CLI wrapper使用<code>ActualBudget::cli_process</code>逐required axis写unknown，不因<code>-rl</code>、timeout或runner exit 0声称任一未观测axis完整。

### Step 5：运行 GREEN

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(raw_witness_is_atomic_hash_bound_and_bounded) | test(truncated_tail_signal_cannot_be_complete_or_proof) | test(request_budget_guard_rejects_n_plus_one_before_send) | test(cli_internal_budget_is_never_claimed_as_observed) | test(host_transport_blocks_redirect_or_rebind_outside_exact_policy_before_send) | test(unmanaged_hostname_cli_is_blocked_before_spawn_in_receipt_v1) | test(provider_target_input_cannot_choose_transport_destination)')
~~~

**Expected:** 8 tests passed，exit code 0；raw witness没有持久化明文、访问单独授权审计且retention后可crypto-erasure；truncated tail不能产生complete/proof，required budget缺轴不能被单一boolean洗白；redirect/rebind/proxy/TLS与unmanaged CLI均在越界I/O前阻断，provider target不能改destination；测试不发出任何真实网络请求。

### Step 6：Future Commit

~~~bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/tool_truth.rs backend/crates/golish-pentest-app/src/pentest_bridge/raw_witness_vault.rs backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs
git commit -m "feat(tool-truth): add producer receipt lifecycle"
~~~

---

## Task 5：修复 EAS exit 0 + empty stdout 假 checked-empty

**文件：**

- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs</code>
- 测试：同文件现有 test module

**现有落点：**

- <code>execute_wrapped</code>
- <code>finalize_wrapped_result</code>
- <code>land_authorized_output</code>
- <code>classify_whatweb_terminal_batch</code>
- <code>persist_guarded_eas_evidence_and_outcomes</code>
- <code>accept_full_empty_port_landing</code>
- <code>port_scan_manifest_complete</code>
- <code>apply_port_scan_result_contract</code>

### Step 1：把现有 WhatWeb 测试改成 RED fail-safe 测试

将 <code>whatweb_exit_zero_success_is_attributed_for_counter_reset</code> 重命名并改为：

~~~rust
#[test]
fn whatweb_exit_zero_without_per_origin_witness_is_nonterminal() {
    let authorization = authorized_origin("https://app.example.test");
    let result = serde_json::json!({
        "exit_code": 0,
        "stdout": "",
        "stderr": "",
        "error": null
    });
    let batch = classify_whatweb_terminal_batch_for_contract(
        &result,
        &[authorization],
        ToolTruthContract::ReceiptV1,
    )
    .expect("receipt mode returns an explicit batch truth");
    assert_eq!(batch.observation_state, ObservationState::Indeterminate);
    assert_eq!(batch.coverage_extent, CoverageExtent::Partial);
    assert_eq!(batch.coverage_gap_reason, CoverageGapReason::ParserReject);
    assert!(!batch.complete);
    assert!(batch.terminal_verdicts.is_empty());
}

#[test]
fn full_port_plan_and_exit_zero_without_trusted_probe_census_is_partial() {
    let plan = full_port_scan_plan(&["192.0.2.10"]);
    let result = completed_empty_port_runner_result(&plan);
    let status = full_empty_port_status(&plan, &result)
        .expect("fixed plan is retained but cannot self-prove execution");
    assert_eq!(status.observation_state, ObservationState::Indeterminate);
    assert_eq!(status.coverage_extent, CoverageExtent::Partial);
    assert_eq!(status.coverage_gap_reason, CoverageGapReason::SourceUnavailable);
    assert!(!status.is_checked_empty());
}

#[test]
fn generic_empty_stdout_never_proves_complete_landing() {
    let status = generic_empty_stdout_status(ToolTruthContract::ReceiptV1);
    assert_eq!(status.observation_state, ObservationState::Indeterminate);
    assert_eq!(status.coverage_extent, CoverageExtent::Partial);
    assert_eq!(status.coverage_gap_reason, CoverageGapReason::ParserReject);
    assert!(!status.is_checked_empty());
}
~~~

现有 test helper 名称不同之处就在同一 test module 内改名，不增加对真实 runner 的依赖。

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(whatweb_exit_zero_without_per_origin_witness_is_nonterminal) | test(full_port_plan_and_exit_zero_without_trusted_probe_census_is_partial) | test(generic_empty_stdout_never_proves_complete_landing)')
~~~

**Expected:** 新测试失败；当前实现仍把 empty stdout 视为 complete，并合成 WhatWeb empty verdict。

### Step 3：接入 receipt lifecycle，并拒绝 plan/exit-code 自证执行完整

为 <code>execute_wrapped</code> 和 managed <code>finalize_wrapped_result</code> 传递同一个 <code>receipt_id</code>。外部 runner 前调用 <code>ToolTruthExecution::begin</code>，parser 前调用 <code>stage_witness</code>，landing 后调用 <code>close_and_reconcile</code>。

新增纯 helper：

~~~rust
fn generic_empty_stdout_status(contract: ToolTruthContract) -> ReceiptCoverageFact {
    let fail_safe = contract.enforces_fail_safe_projection();
    ReceiptCoverageFact {
        input_key: "batch".to_string(),
        attempt_state: AttemptState::Succeeded,
        landing_state: if fail_safe {
            LandingState::Partial
        } else {
            LandingState::Committed
        },
        observation_state: if fail_safe {
            ObservationState::Indeterminate
        } else {
            ObservationState::NoMatch
        },
        coverage_extent: if fail_safe {
            CoverageExtent::Partial
        } else {
            CoverageExtent::Complete
        },
        coverage_gap_reason: if fail_safe {
            CoverageGapReason::ParserReject
        } else {
            CoverageGapReason::None
        },
        reconciliation_state: ReconciliationState::Pending,
        security_interpretation: SecurityInterpretation::NotAssessed,
        authority_current: true,
        residual: None,
    }
}
~~~

修改 <code>land_authorized_output</code>：

~~~rust
if runner_succeeded(&result) && stdout(&result).trim().is_empty() {
    let status = generic_empty_stdout_status(tool_truth_contract);
    return AuthorizedLandingSummary {
        complete: !tool_truth_contract.enforces_fail_safe_projection(),
        errors: tool_truth_contract
            .enforces_fail_safe_projection()
            .then(|| "exit 0 had no per-input completion witness".to_string())
            .into_iter()
            .collect(),
        receipt_status: Some(status),
        ..AuthorizedLandingSummary::default()
    };
}
~~~

修改 <code>classify_whatweb_terminal_batch</code>：

- legacy/shadow 保留旧 terminal batch projection。
- receipt_v1 不再为 expected origin 合成 <code>completed_without_fingerprints</code>。
- stdout 有不能解析的非空内容时为 <code>found/indeterminate + partial + parser_reject</code>，raw witness 保留原字节。
- stdout 真空且没有 per-origin completion witness 时为 <code>indeterminate + partial + parser_reject</code>。

删除`accept_full_empty_port_landing`仅凭`complete_for_gate + port_scan_manifest_complete + exit 0`形成checked-empty的分支。计划中的TCP 1–65535只是planned manifest，不是actual probe census；现有CLI wrapper无法由host观察逐probe dispatch，因此在Plan A内即使runner报告完成且stdout为空，也只能是`indeterminate/partial/source_unavailable`并附exact residual。未来若增加受信egress proxy或versioned instrumented transport，必须同时给出逐host/port actual census、tool/adapter digest、完整required budget observations和reconciliation，才能另行引入complete/no_match；不能复用当前纯status helper。

### Step 4：运行 GREEN 与现有相邻回归

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(whatweb_exit_zero_without_per_origin_witness_is_nonterminal) | test(full_port_plan_and_exit_zero_without_trusted_probe_census_is_partial) | test(generic_empty_stdout_never_proves_complete_landing) | test(full_empty_manifest_is_not_terminal_without_trusted_probe_census) | test(incomplete_port_profile_never_terminalizes_port_coverage) | test(whatweb_mixed_batch_preserves_success_and_keeps_first_failure_retryable)')
~~~

**Expected:** 6 tests passed，exit code 0；legacy/shadow compatibility assertion和receipt_v1 fail-safe assertion同时存在；固定plan或CLI exit 0均不能替代actual probe/budget authority。

### Step 5：Future Commit

~~~bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs
git commit -m "fix(eas): require per-input proof for empty output"
~~~

---

## Task 6：把 Enumeration preflight blocked 降为 prerequisite gap

**文件：**

- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/enum_preflight_web_origins.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/harness/org_gate.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/tool_executors/security.rs</code>
- 修改：<code>resources/harness/stages/enumeration/spec.json</code>
- 修改：<code>resources/harness/stages/enumeration/methodology.md</code>
- 测试：前两个 Rust 文件的现有 test modules

**现有落点：**

- <code>EnumPreflightWebOriginsTool::execute_authorized</code>
- <code>probe_origin</code>
- <code>ProbeDecision::Blocked</code>
- <code>ENUM_TECHNIQUES</code>
- <code>TRUSTED_ENUM_BLOCKED_SOURCE</code>
- <code>trusted_enumeration_blocked_source</code>
- <code>apply_technique_outcome_rows</code>

### Step 1：写 RED producer 与 Gate tests

~~~rust
#[test]
fn blocked_preflight_keeps_all_content_axes_partial() {
    let projection = preflight_failure_projection(
        ToolTruthContract::ReceiptV1,
        "https://app.example.test",
        "connect_timeout",
    );
    assert_eq!(projection.attempt_markers.len(), 4);
    assert!(projection
        .attempt_markers
        .iter()
        .all(|marker| marker.outcome == "partial"));
    assert!(projection.terminal_outcomes.is_empty());
    assert_eq!(projection.receipt.observation_state, ObservationState::Indeterminate);
    assert_eq!(projection.receipt.coverage_extent, CoverageExtent::Partial);
    assert_eq!(projection.receipt.coverage_gap_reason, CoverageGapReason::Transport);
}

#[test]
fn preflight_blocked_is_prerequisite_gap_not_content_coverage() {
    let rows = vec![technique_outcome_row(
        "https://app.example.test",
        "GOLISH-ENUM-JS",
        "blocked",
        "enum_preflight_web_origins",
    )];
    let facts = apply_technique_outcome_rows_for_contract(
        ToolTruthContract::ReceiptV1,
        rows,
    );
    assert!(facts.iter().all(|fact| !fact.is_terminal()));
}

#[test]
fn legacy_preflight_rows_keep_frozen_compatibility() {
    let rows = vec![technique_outcome_row(
        "https://legacy.example.test",
        "GOLISH-ENUM-JS",
        "blocked",
        "enum_preflight_web_origins",
    )];
    let facts = apply_technique_outcome_rows_for_contract(
        ToolTruthContract::LegacyV1,
        rows,
    );
    assert!(facts.iter().any(|fact| fact.is_terminal()));
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(blocked_preflight_keeps_all_content_axes_partial)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(preflight_blocked_is_prerequisite_gap_not_content_coverage) | test(legacy_preflight_rows_keep_frozen_compatibility)')
~~~

**Expected:** producer test 失败，因为当前 blocked 分支仍写四个 terminal rows；Gate test 失败，因为 source 仍被映射到四轴 terminal truth。

### Step 3：修改 producer projection

保留四个 attempt marker，用于清理 stale truth，但 receipt_v1 不再发布四个 blocked：

~~~rust
fn preflight_failure_projection(
    contract: ToolTruthContract,
    origin: &str,
    failure_class: &str,
) -> PreflightFailureProjection {
    let attempt_markers = ENUM_TECHNIQUES
        .iter()
        .map(|technique| TechniqueOutcomeWrite {
            asset: origin.to_string(),
            technique: (*technique).to_string(),
            outcome: "partial".to_string(),
            evidence_id: None,
            note: Some(format!("transport prerequisite failed: {failure_class}")),
        })
        .collect::<Vec<_>>();
    let terminal_outcomes = if contract.enforces_fail_safe_projection() {
        vec![]
    } else {
        legacy_preflight_blocked_outcomes(origin, failure_class)
    };
    PreflightFailureProjection {
        attempt_markers,
        terminal_outcomes,
        receipt: ReceiptCoverageFact {
            input_key: format!("origin:{origin}"),
            attempt_state: AttemptState::Failed,
            landing_state: LandingState::Committed,
            observation_state: ObservationState::Indeterminate,
            coverage_extent: CoverageExtent::Partial,
            coverage_gap_reason: CoverageGapReason::Transport,
            reconciliation_state: ReconciliationState::Pending,
            security_interpretation: SecurityInterpretation::Inconclusive,
            authority_current: true,
            residual: None,
        },
    }
}
~~~

<code>ProbeDecision::Blocked</code> 分支只追加一条 target/origin-bound prerequisite evidence，kind 固定为 <code>enumeration_transport_prerequisite_failed</code>。tool result 固定返回：

~~~json
{
  "terminal_coverage_written": false,
  "coverage_extent": "partial",
  "coverage_gap_reason": "transport",
  "affected_techniques": [
    "GOLISH-ENUM-JS",
    "GOLISH-ENUM-DIR",
    "GOLISH-ENUM-PARAM",
    "GOLISH-ENUM-JSAPI"
  ]
}
~~~

删除 receipt_v1 响应中的 <code>four_axis_atomic=true</code>。

### Step 4：修改 Gate compatibility projection 与 embedded resources

将 <code>trusted_enumeration_blocked_source</code> 改为接收 frozen contract：

~~~rust
fn trusted_enumeration_blocked_source(
    contract: ToolTruthContract,
    source: &str,
) -> Option<&'static [&'static str]> {
    if contract.enforces_fail_safe_projection() && source == TRUSTED_ENUM_BLOCKED_SOURCE {
        return None;
    }
    match source {
        "enum_preflight_web_origins" => Some(ENUM_CONTENT_TECHNIQUES),
        "route_probe_paths" => Some(&["GOLISH-ENUM-DIR"]),
        "browser_collect_js_api" => {
            Some(&["GOLISH-ENUM-JS", "GOLISH-ENUM-JSAPI", "GOLISH-ENUM-PARAM"])
        }
        _ => None,
    }
}
~~~

仅禁用 preflight 的四轴关闭；route/browser 自己拥有的 bounded recovery 语义保留。

同步以下现有文案：

- <code>security.rs</code> 中 <code>worklist_semantics</code>、<code>tool_boundary</code>、coverage preview/status 文案，不再出现 “preflight owns all-axis blocked”。
- <code>enumeration/spec.json</code> 的 <code>$comment_derive</code> 删除 “enum_preflight_web_origins may close JS/DIR/PARAM/JSAPI”。
- <code>enumeration/methodology.md</code> 明确 transport prerequisite failure 保留四轴 partial，producer 或后续批准的 alternate transport 才能关闭各轴。

### Step 5：运行 GREEN 与资源检查

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(blocked_preflight_keeps_all_content_axes_partial) | test(failed_strategy_setup_makes_transport_result_inconclusive) | test(operation_epoch_rejects_restart_advance_and_supersede)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(preflight_blocked_is_prerequisite_gap_not_content_coverage) | test(legacy_preflight_rows_keep_frozen_compatibility) | test(enumeration_terminal_outcome_requires_real_evidence_id) | test(enumeration_blocked_requires_matching_current_evidence_fact)')
pnpm exec biome check resources/harness/stages/enumeration/spec.json
~~~

**Expected:** Rust 7 tests passed；Biome 输出 checked 1 file、0 errors；exit code 均为 0。

### Step 6：Future Commit

~~~bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/enum_preflight_web_origins.rs backend/crates/golish-agent-kit/src/harness/org_gate.rs backend/crates/golish-agent-kit/src/tool_executors/security.rs resources/harness/stages/enumeration/spec.json resources/harness/stages/enumeration/methodology.md
git commit -m "fix(enumeration): keep preflight failures nonterminal"
~~~

---

## Task 7：分离 positive observation 与 partial coverage

**文件：**

- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/tool_truth.rs</code>
- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs</code>
- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs</code>
- 测试：上述三个文件现有 test modules

**现有 JS 落点：**

- <code>jsapi_outcome_from_extract</code>
- <code>js_extract_completion_state_with_persistence</code>
- <code>extract_outcome_with_completion</code>
- <code>upsert_param_outcome</code>
- <code>upsert_jsapi_outcome</code>

**现有 anonymous 落点：**

- <code>aggregate_observations</code>
- <code>anonymous_landing_outcome</code>
- <code>land_result</code>

### Step 1：写 RED projection tests

~~~rust
#[test]
fn positive_extract_preserves_found_observation_but_projects_partial_coverage() {
    let truth = producer_projection(
        ToolTruthContract::ReceiptV1,
        ObservationState::Found,
        CoverageExtent::Partial,
        CoverageGapReason::ParserReject,
    );
    assert_eq!(truth.observation_state, ObservationState::Found);
    assert_eq!(truth.technique_outcome_projection, "partial");
    assert_eq!(truth.security_interpretation, SecurityInterpretation::Signal);
}

#[test]
fn shadow_positive_partial_keeps_legacy_projection() {
    let truth = producer_projection(
        ToolTruthContract::ShadowV1,
        ObservationState::Found,
        CoverageExtent::Partial,
        CoverageGapReason::ParserReject,
    );
    assert_eq!(truth.observation_state, ObservationState::Found);
    assert_eq!(truth.technique_outcome_projection, "found");
}

#[test]
fn anonymous_positive_sibling_keeps_signal_but_projects_partial_coverage() {
    let observations = vec![
        suspicious_observation(),
        inconclusive_observation(),
    ];
    let aggregate = aggregate_observations_for_contract(
        ToolTruthContract::ReceiptV1,
        2,
        &observations,
        false,
    );
    assert_eq!(aggregate.observation_state, ObservationState::Found);
    assert_eq!(aggregate.coverage_extent, CoverageExtent::Partial);
    assert_eq!(aggregate.technique_outcome_projection, "partial");
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(positive_extract_preserves_found_observation_but_projects_partial_coverage) | test(shadow_positive_partial_keeps_legacy_projection) | test(anonymous_positive_sibling_keeps_signal_but_projects_partial_coverage)')
~~~

**Expected:** tests 失败，因为当前 JS/anonymous 聚合只返回单一 outcome，found 会覆盖 partial。

### Step 3：实现共享 projection

在 bridge helper 中加入：

~~~rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerProjection {
    pub observation_state: ObservationState,
    pub coverage_extent: CoverageExtent,
    pub coverage_gap_reason: CoverageGapReason,
    pub security_interpretation: SecurityInterpretation,
    pub technique_outcome_projection: &'static str,
}

pub fn producer_projection(
    contract: ToolTruthContract,
    observation_state: ObservationState,
    coverage_extent: CoverageExtent,
    coverage_gap_reason: CoverageGapReason,
) -> ProducerProjection {
    let legacy_observation = match observation_state {
        ObservationState::Found => "found",
        ObservationState::NoMatch => "empty",
        ObservationState::Indeterminate => "partial",
        ObservationState::NotApplicable => "not_applicable",
    };
    let technique_outcome_projection =
        if contract.enforces_fail_safe_projection() && coverage_extent != CoverageExtent::Complete {
            "partial"
        } else {
            legacy_observation
        };
    ProducerProjection {
        observation_state,
        coverage_extent,
        coverage_gap_reason,
        security_interpretation: match observation_state {
            ObservationState::Found => SecurityInterpretation::Signal,
            ObservationState::Indeterminate => SecurityInterpretation::Inconclusive,
            ObservationState::NoMatch | ObservationState::NotApplicable => {
                SecurityInterpretation::NotAssessed
            }
        },
        technique_outcome_projection,
    }
}
~~~

### Step 4：替换 JS 与 anonymous 单维聚合

JS：

- <code>jsapi_outcome_from_extract</code> 改为只算 observation。
- <code>js_extract_completion_state_with_persistence</code> 继续决定 complete/partial。
- 删除 <code>extract_outcome_with_completion</code> 的 “partial + found => found coverage” 规则，改为调用 <code>producer_projection</code>。
- evidence/business rows仍保留 positive facts；<code>TechniqueOutcomeWrite.outcome</code> 使用 <code>technique_outcome_projection</code>。

anonymous：

- <code>AggregateResult</code> 增加 <code>observation_state</code>、<code>coverage_extent</code>、<code>coverage_gap_reason</code> 和 <code>technique_outcome_projection</code>。
- 任一 suspicious 保留 found observation。
- selected count 不符、batch error、Inconclusive/Skipped、authority stale 任一成立即 partial coverage。
- <code>land_result</code> 的 evidence 使用 found signal，technique outcome 使用 projection。

raw positive 存在但 parser/persistence 失败时 receipt 固定为：

~~~rust
ReceiptCoverageFact {
    input_key,
    attempt_state: AttemptState::Succeeded,
    landing_state: LandingState::Partial,
    observation_state: ObservationState::Found,
    coverage_extent: CoverageExtent::Partial,
    coverage_gap_reason: CoverageGapReason::ParserReject,
    reconciliation_state: ReconciliationState::Orphaned,
    security_interpretation: SecurityInterpretation::Signal,
    authority_current: true,
    residual: None,
}
~~~

### Step 5：运行 GREEN 与相邻回归

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(positive_extract_preserves_found_observation_but_projects_partial_coverage) | test(shadow_positive_partial_keeps_legacy_projection) | test(anonymous_positive_sibling_keeps_signal_but_projects_partial_coverage) | test(partial_extract_never_publishes_terminal_found_or_empty) | test(only_complete_zero_script_manifest_can_prove_clean_empty_js) | test(all_read_errors_fall_through_as_partial_instead_of_clean_empty) | test(third_complete_anonymous_inconclusive_becomes_evidence_backed_blocked) | test(third_complete_anonymous_inconclusive_requires_current_network_clean_observations)')
~~~

**Expected:** 8 tests passed，exit code 0；receipt_v1 的 technique outcome 为 partial，同时 raw/business/evidence positive 不丢失。

### Step 6：Future Commit

~~~bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/tool_truth.rs backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs
git commit -m "fix(tool-truth): keep positive signals separate from coverage"
~~~

---

## Task 8：让 Nuclei no-match 默认 inconclusive

**文件：**

- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/landing.rs</code>
- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs</code>
- 修改：<code>backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs</code>
- 修改：<code>resources/harness/stages/vuln_triage/methodology.md</code>
- 测试：三个 Rust 文件现有 test modules

**现有落点：**

- <code>land_nuclei_report</code>
- <code>NucleiLandingInput</code>
- <code>nuclei_outcome</code>
- <code>VerifyExecuteCandidateActionTool</code>
- <code>execute_nuclei_replay</code>
- <code>parse_exact_nuclei_runner_result</code>
- <code>persist_exact_replay</code>

### Step 1：写 RED stage scanner 与 exact replay tests

~~~rust
#[test]
fn complete_nuclei_no_match_is_nonterminal_without_versioned_negative_oracle() {
    let projection = nuclei_projection(
        ToolTruthContract::ReceiptV1,
        NucleiCompletion::Complete,
        0,
        NucleiCoverageScope::FingerprintTargeted,
    );
    assert_eq!(projection.observation_state, ObservationState::NoMatch);
    assert_eq!(projection.coverage_extent, CoverageExtent::TemplateOnly);
    assert_eq!(projection.security_interpretation, SecurityInterpretation::Inconclusive);
    assert_eq!(projection.technique_outcome_projection, "partial");
}

#[test]
fn nuclei_complete_no_match_is_inconclusive_without_negative_oracle() {
    let interpretation = exact_nuclei_replay_interpretation(
        ToolTruthContract::ReceiptV1,
        NucleiCompletion::Complete,
        0,
    );
    assert_eq!(interpretation.role, "blocker");
    assert_eq!(interpretation.evidence_outcome, "partial");
    assert!(!interpretation.success);
    assert_eq!(
        interpretation.error_code,
        Some("NUCLEI_REPLAY_NO_MATCH_INCONCLUSIVE")
    );
}

#[test]
fn receipt_v1_nuclei_match_is_signal_not_oracle_proof() {
    let interpretation = exact_nuclei_replay_interpretation(
        ToolTruthContract::ReceiptV1,
        NucleiCompletion::Complete,
        1,
    );
    assert_eq!(interpretation.role, "supporter");
    assert_eq!(interpretation.evidence_outcome, "found");
    assert_eq!(interpretation.security_interpretation, SecurityInterpretation::Signal);
}

#[test]
fn legacy_nuclei_no_match_keeps_frozen_projection() {
    let projection = nuclei_projection(
        ToolTruthContract::LegacyV1,
        NucleiCompletion::Complete,
        0,
        NucleiCoverageScope::General,
    );
    assert_eq!(projection.technique_outcome_projection, "empty");
}

#[test]
fn nuclei_no_templates_is_unsupported_not_complete_not_applicable() {
    let projection = nuclei_projection(
        ToolTruthContract::ReceiptV1,
        NucleiCompletion::NoTemplates,
        0,
        NucleiCoverageScope::General,
    );
    assert_eq!(projection.observation_state, ObservationState::Indeterminate);
    assert_eq!(projection.coverage_extent, CoverageExtent::None);
    assert_eq!(projection.coverage_gap_reason, CoverageGapReason::Unsupported);
    assert_eq!(projection.security_interpretation, SecurityInterpretation::Inconclusive);
    assert_eq!(projection.technique_outcome_projection, "partial");
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(complete_nuclei_no_match_is_nonterminal_without_versioned_negative_oracle) | test(nuclei_complete_no_match_is_inconclusive_without_negative_oracle) | test(receipt_v1_nuclei_match_is_signal_not_oracle_proof) | test(legacy_nuclei_no_match_keeps_frozen_projection) | test(nuclei_no_templates_is_unsupported_not_complete_not_applicable)')
~~~

**Expected:** tests 失败；当前 <code>nuclei_outcome</code> 和 replay branch 都把 Complete + 0 matches 提升为 empty/refutation。

### Step 3：实现 stage scanner projection

给 server-owned <code>NucleiLandingInput</code> 增加 frozen contract；不得从模型 args 接收。实现：

~~~rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NucleiCoverageScope {
    General,
    FingerprintTargeted,
}

fn nuclei_projection(
    contract: ToolTruthContract,
    completion: NucleiCompletion,
    match_count: usize,
    scope: NucleiCoverageScope,
) -> ProducerProjection {
    if completion == NucleiCompletion::NoTemplates {
        let mut projection = producer_projection(
            contract,
            ObservationState::Indeterminate,
            CoverageExtent::None,
            CoverageGapReason::Unsupported,
        );
        projection.security_interpretation = SecurityInterpretation::Inconclusive;
        return projection;
    }
    if completion == NucleiCompletion::Complete && match_count > 0 {
        return producer_projection(
            contract,
            ObservationState::Found,
            CoverageExtent::Complete,
            CoverageGapReason::None,
        );
    }
    if completion == NucleiCompletion::Complete {
        let mut projection = producer_projection(
            contract,
            ObservationState::NoMatch,
            match scope {
                NucleiCoverageScope::General => CoverageExtent::Partial,
                NucleiCoverageScope::FingerprintTargeted => CoverageExtent::TemplateOnly,
            },
            CoverageGapReason::Unsupported,
        );
        projection.security_interpretation = SecurityInterpretation::Inconclusive;
        return projection;
    }
    producer_projection(
        contract,
        ObservationState::Indeterminate,
        CoverageExtent::Partial,
        match completion {
            NucleiCompletion::BudgetBlocked => CoverageGapReason::BudgetExhausted,
            NucleiCompletion::Blocked | NucleiCompletion::TransportBlocked => {
                CoverageGapReason::Transport
            }
            NucleiCompletion::Partial | NucleiCompletion::Error => {
                CoverageGapReason::ToolFailure
            }
            NucleiCompletion::Complete | NucleiCompletion::NoTemplates => {
                CoverageGapReason::Unsupported
            }
        },
    )
}
~~~

<code>nuclei_outcome</code> 改成消费 projection。matches 继续found；runtime `NoTemplates`只表示当前adapter没有可运行模板，不是server-proven applicability authority，因此在receipt_v1固定为`indeterminate/none/unsupported/inconclusive`并写residual。只有未来独立、versioned、server-owned applicability contract才能产生complete/not_applicable。receipt_v1 no-match写partial projection、scanner_no_match typed landing和inconclusive interpretation。

### Step 4：实现 exact replay fail-safe

抽出纯 helper：

~~~rust
struct ExactNucleiReplayInterpretation {
    role: &'static str,
    evidence_outcome: &'static str,
    success: bool,
    error_code: Option<&'static str>,
    security_interpretation: SecurityInterpretation,
}

fn exact_nuclei_replay_interpretation(
    contract: ToolTruthContract,
    completion: NucleiCompletion,
    exact_matches: usize,
) -> ExactNucleiReplayInterpretation {
    match (completion, exact_matches, contract.enforces_fail_safe_projection()) {
        (NucleiCompletion::Complete, count, true) if count > 0 => {
            ExactNucleiReplayInterpretation {
                role: "supporter",
                evidence_outcome: "found",
                success: true,
                error_code: None,
                security_interpretation: SecurityInterpretation::Signal,
            }
        }
        (NucleiCompletion::Complete, count, false) if count > 0 => {
            ExactNucleiReplayInterpretation {
                role: "proof",
                evidence_outcome: "found",
                success: true,
                error_code: None,
                security_interpretation: SecurityInterpretation::Proof,
            }
        }
        (NucleiCompletion::Complete, 0, true) => ExactNucleiReplayInterpretation {
            role: "blocker",
            evidence_outcome: "partial",
            success: false,
            error_code: Some("NUCLEI_REPLAY_NO_MATCH_INCONCLUSIVE"),
            security_interpretation: SecurityInterpretation::Inconclusive,
        },
        (NucleiCompletion::Complete, 0, false) => ExactNucleiReplayInterpretation {
            role: "refutation",
            evidence_outcome: "empty",
            success: true,
            error_code: None,
            security_interpretation: SecurityInterpretation::Refutation,
        },
        _ => ExactNucleiReplayInterpretation {
            role: "blocker",
            evidence_outcome: "partial",
            success: false,
            error_code: Some("NUCLEI_REPLAY_INCONCLUSIVE"),
            security_interpretation: SecurityInterpretation::Inconclusive,
        },
    }
}
~~~

receipt_v1 的result payload在零match时加<code>"scanner_no_match": true</code>；不得使用<code>refutation</code>或<code>empty</code>。positive match也只能是`signal/supporter`，不能在Plan A producer里铸造proof。methodology明确只有Plan C的exact、versioned、prerequisite-complete oracle才能proof/refute exact predicate。

### Step 5：运行 GREEN 与相邻回归

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(complete_nuclei_no_match_is_nonterminal_without_versioned_negative_oracle) | test(nuclei_complete_no_match_is_inconclusive_without_negative_oracle) | test(receipt_v1_nuclei_match_is_signal_not_oracle_proof) | test(legacy_nuclei_no_match_keeps_frozen_projection) | test(nuclei_no_templates_is_unsupported_not_complete_not_applicable) | test(nuclei_replay_uses_the_exact_frozen_template_and_url) | test(budget_blocked_maps_to_blocked_while_error_maps_to_partial)')
~~~

**Expected:** 7 tests passed，exit code 0；receipt_v1 positive只产生signal，no-match与NoTemplates都不再产生checked-empty/refutation/not-applicable complete，legacy fixture不变。

### Step 6：Future Commit

~~~bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/landing.rs backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs resources/harness/stages/vuln_triage/methodology.md
git commit -m "fix(nuclei): treat scanner no-match as inconclusive"
~~~

---

## Task 9：冻结 denominator 并 shadow-write Gate control/grade

**文件：**

- 创建：<code>backend/crates/golish-agent-kit/src/harness/tool_truth.rs</code>
- 创建：<code>backend/crates/golish-agent-app/src/ai/db_bridge/tool_truth.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/harness/mod.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/harness/org_gate.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/db_traits/types.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/db_traits/repo.rs</code>
- 修改：<code>backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs</code>
- 修改：<code>backend/crates/golish-agent-app/src/ai/db_bridge/orchestration.rs</code>
- 修改：<code>backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs</code>
- 修改：<code>backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs</code>
- 修改：<code>backend/crates/golish-db/src/repo/capability_execution_receipts.rs</code>
- 测试：agent-kit、agent-app、golish-db 对应 test modules

### Step 1：写 RED denominator 与 shadow tests

~~~rust
#[test]
fn denominator_items_are_asset_times_expected_technique() {
    let items = build_denominator_items(
        StageKind::Enumeration,
        &[LockedToolTruthAsset {
            target_id: Uuid::from_u128(1),
            exact_asset: "https://app.example.test".into(),
            asset_type: "web_origin".into(),
            web_capable: true,
        }],
    )
    .expect("enumeration denominator");
    assert_eq!(items.len(), 4);
    assert_eq!(
        items.iter().map(|item| item.technique.as_str()).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-JSAPI",
            "GOLISH-ENUM-PARAM",
        ])
    );
}

#[test]
fn denominator_catalog_is_exactly_the_embedded_spec_and_shared_applicability() {
    for stage in [
        StageKind::TargetIntel,
        StageKind::ExternalAttackSurface,
        StageKind::Enumeration,
        StageKind::VulnTriage,
    ] {
        let spec = load_embedded_stage_spec(stage).expect("embedded spec");
        let assets = golden_assets_for_stage(stage);
        let items = build_denominator_items(stage, &assets).expect("denominator");
        assert_denominator_exactly_matches_spec_and_shared_resolver(stage, &spec, &assets, &items);
    }
}

#[test]
fn new_spec_technique_without_registered_capability_fails_closed() {
    let mut spec = load_embedded_stage_spec(StageKind::Enumeration).expect("spec");
    spec.expected_techniques.push("GOLISH-ENUM-FUTURE".into());
    let error = build_denominator_items_from_spec(
        StageKind::Enumeration,
        &spec,
        &golden_assets_for_stage(StageKind::Enumeration),
    )
    .expect_err("spec growth must not silently disappear from denominator");
    assert_eq!(error.code(), "TOOL_TRUTH_CAPABILITY_MAPPING_MISSING");
}

#[tokio::test]
async fn public_denominator_request_cannot_omit_members_or_rebind_a_source() {
    let fixture = DenominatorRepoFixture::with_frozen_wave(&["origin:a", "origin:b"]).await;
    let request = SealToolTruthDenominatorRequest {
        stable_seal_request_id: fixture.stable_request_id,
        stage_execution_id: fixture.stage_execution_id,
        source: ToolTruthDenominatorSourceRef::StageAssetWave {
            stage_asset_wave_id: fixture.wave_id,
        },
    };
    let sealed = fixture.repo.tool_truth_seal_denominator(request.clone()).await
        .expect("repo derives exact members");
    fixture.assert_exact_server_members(sealed.id, &["origin:a", "origin:b"]).await;
    fixture.try_rebind_request_to_other_wave(request).await
        .expect_err("stable request cannot be rebound");
    fixture.assert_public_request_source_has_no_asset_item_count_or_hash_fields();
}

#[tokio::test]
async fn tool_truth_shadow_grade_does_not_change_legacy_gate_result() {
    let repo = ShadowRepoFixture::with_partial_receipt();
    let legacy = evaluate_org_stage_gate(
        &repo,
        Some(repo.operation_id),
        Some(repo.organization_id),
        "session-1",
        StageKind::Enumeration,
        &repo.legacy_passing_deliverable(),
        Some(repo.cutoff),
        Some(&repo.wave),
    )
    .await;
    assert!(legacy.allowed);
    let assessment = repo.last_assessment().expect("shadow assessment written");
    assert_eq!(assessment.control_decision, "hold");
    assert_eq!(assessment.coverage_grade, "incomplete");
    assert!(assessment.divergence);
}

#[tokio::test]
async fn missing_denominator_is_shadow_incomplete() {
    let repo = ShadowRepoFixture::without_denominator();
    let legacy = repo.evaluate().await;
    assert_eq!(
        repo.last_assessment().expect("assessment").coverage_grade,
        "incomplete"
    );
    assert_eq!(legacy.allowed, repo.expected_legacy_allowed);
}

#[tokio::test]
async fn three_discovered_ports_with_only_two_fingerprinted_hold_gate() {
    let repo = ShadowRepoFixture::with_port_children(&[80, 443, 8443]);
    repo.assert_root_denominator_append_and_reseal_rejected().await;
    repo.assert_derived_denominator_exact_members(&[80, 443, 8443]).await;
    repo.close_service_fingerprint_children(&[80, 443]).await;
    let assessment = repo.evaluate_tool_truth().await;
    assert_eq!(assessment.control_decision, "hold");
    assert_eq!(assessment.coverage_grade, "incomplete");
    assert_eq!(assessment.missing_dynamic_child_keys, vec!["tcp:8443"]);
}

#[tokio::test]
async fn five_discovered_scripts_with_four_parsed_hold_gate() {
    let repo = ShadowRepoFixture::with_script_children(5);
    repo.close_js_parse_children(4).await;
    let assessment = repo.evaluate_tool_truth().await;
    assert_eq!(assessment.control_decision, "hold");
    assert_eq!(assessment.coverage_grade, "incomplete");
}

#[tokio::test]
async fn explicit_sealed_empty_child_manifest_is_not_missing() {
    let repo = ShadowRepoFixture::with_explicit_empty_child_manifest("open_tcp_port");
    let assessment = repo.evaluate_tool_truth().await;
    assert!(assessment.missing_dynamic_child_keys.is_empty());
    assert_ne!(assessment.coverage_grade, "incomplete");
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(denominator_items_are_asset_times_expected_technique) | test(denominator_catalog_is_exactly_the_embedded_spec_and_shared_applicability) | test(new_spec_technique_without_registered_capability_fails_closed) | test(tool_truth_shadow_grade_does_not_change_legacy_gate_result) | test(missing_denominator_is_shadow_incomplete) | test(three_discovered_ports_with_only_two_fingerprinted_hold_gate) | test(five_discovered_scripts_with_four_parsed_hold_gate) | test(explicit_sealed_empty_child_manifest_is_not_missing)')
~~~

**Expected:** 编译失败；DTO、builder、repo methods 和 shadow persistence 尚不存在。

### Step 3：增加不承载denominator成员的窄DB trait DTO与方法

在 <code>db_traits/types.rs</code> 定义不依赖 sqlx 的类型：

~~~rust
#[derive(Debug, Clone)]
pub struct SealToolTruthDenominatorRequest {
    pub stable_seal_request_id: Uuid,
    pub stage_execution_id: Uuid,
    pub source: ToolTruthDenominatorSourceRef,
}

#[derive(Debug, Clone)]
pub enum ToolTruthDenominatorSourceRef {
    StageAssetWave { stage_asset_wave_id: Uuid },
    StageTeamUnit { stage_run_unit_id: Uuid },
}

#[derive(Debug, Clone)]
pub struct ToolTruthGateDecision {
    pub legacy_allowed: bool,
    pub control_decision: ControlDecision,
    pub coverage_grade: CoverageGrade,
    pub residuals: Vec<CoverageResidual>,
}
~~~

在 <code>DbRepoProvider</code> 增加：

~~~rust
async fn tool_truth_contract(
    &self,
    operation_id: Uuid,
) -> anyhow::Result<ToolTruthContract>;

async fn tool_truth_seal_denominator(
    &self,
    request: SealToolTruthDenominatorRequest,
) -> anyhow::Result<ToolTruthDenominatorView>;

async fn tool_truth_current_coverage(
    &self,
    operation_id: Uuid,
    organization_id: Uuid,
    stage: StageKind,
    stage_asset_wave_id: Option<Uuid>,
) -> anyhow::Result<Option<ToolTruthCoverageView>>;

async fn tool_truth_with_fresh_authority_set<R>(
    &self,
    scope: ToolTruthAuthorityScope,
    consumer: ToolTruthConsumerRequest,
    consume: FreshAuthorityConsumer<R>,
) -> anyhow::Result<R>;
~~~

agent-app bridge 使用 <code>golish_db::repo::capability_execution_receipts</code> 实现这些方法。`tool_truth_seal_denominator`不会先把asset列表返回给调用方：它在一个repo-owned transaction内锁住request指定的sealed wave/unit source，反查operation/org/stage/attempt，读取exact authoritative assets，调用共享StageSpec/applicability compiler，重算items/count/hash后直接open→members→seal。public trait/source DTO没有asset/item/count/hash字段，mock也必须走同一server-derived compiler contract。`tool_truth_with_fresh_authority_set`不是普通“取token”接口：它由bridge创建verified vault snapshots并在同一短transaction内调用纯reducer、把server-derived denominator/semantic/freshness hashes和`ToolTruthGateDecision`一起落assessment；调用方没有单独的write-assessment或caller-provided reconciliation hash路径。所有asset查询必须复用Gate当前exact-origin/typed-asset ownership与freshness条件；禁止从receipt行倒推denominator。

### Step 4：复用唯一 StageSpec / applicability source 构建 deterministic denominator

~~~rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct LockedToolTruthAsset {
    target_id: Uuid,
    exact_asset: String,
    asset_type: String,
    web_capable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerDerivedDenominatorItem {
    input_key: String,
    target_id: Uuid,
    exact_asset: String,
    technique: String,
    expected_capability: String,
    item_hash: String,
}

fn build_denominator_items(
    stage: StageKind,
    assets: &[LockedToolTruthAsset],
) -> Result<Vec<ServerDerivedDenominatorItem>, ToolTruthDenominatorError> {
    let spec = load_embedded_stage_spec(stage)
        .map_err(ToolTruthDenominatorError::StageSpec)?;
    build_denominator_items_from_spec(stage, &spec, assets)
}

fn build_denominator_items_from_spec(
    stage: StageKind,
    spec: &StageSpec,
    assets: &[LockedToolTruthAsset],
) -> Result<Vec<ServerDerivedDenominatorItem>, ToolTruthDenominatorError> {
    if spec.kind != stage {
        return Err(ToolTruthDenominatorError::StageSpecKindDrift);
    }
    let classes = assets
        .iter()
        .map(|asset| classify_stage_asset(stage, Some(&asset.asset_type), &asset.exact_asset))
        .collect::<Vec<_>>();
    let resolved = resolve_expected_techniques(stage, &classes);
    let techniques = if resolved.is_empty() {
        spec.expected_techniques.clone()
    } else {
        if resolved.iter().any(|technique| !spec.expected_techniques.contains(technique)) {
            return Err(ToolTruthDenominatorError::ResolverOutsideStageSpec);
        }
        resolved
    };
    if assets.is_empty() || techniques.is_empty() {
        return Err(ToolTruthDenominatorError::EmptyAuthoritativeInput);
    }
    let mut items = Vec::new();
    for asset in assets {
        let class = classify_stage_asset(stage, Some(&asset.asset_type), &asset.exact_asset);
        for technique in &techniques {
            if !technique_applies_web_aware(
                stage,
                class,
                &asset.exact_asset,
                technique,
                asset.web_capable,
            ) {
                continue;
            }
            let input_key = format!("{}\u{1f}{}\u{1f}{}", asset.target_id, asset.exact_asset, technique);
            items.push(ServerDerivedDenominatorItem {
                input_key: input_key.clone(),
                target_id: asset.target_id,
                exact_asset: asset.exact_asset.clone(),
                technique: technique.clone(),
                expected_capability: registered_capability_for(stage, technique)
                    .ok_or_else(|| ToolTruthDenominatorError::CapabilityMappingMissing(technique.clone()))?
                    .to_string(),
                item_hash: sha256_hex(input_key.as_bytes()),
            });
        }
    }
    items.sort_by(|left, right| left.input_key.cmp(&right.input_key));
    Ok(items)
}
~~~

`LockedToolTruthAsset`与`ServerDerivedDenominatorItem`都只存在于repo-owned transaction/compiler module，字段与constructor不出现在`DbRepoProvider`。这里禁止再实现`expected_techniques_for_stage`手写match。技术全集唯一来自`resources::load_embedded_stage_spec`，动态裁剪唯一来自`technique_resolver::{classify_stage_asset, resolve_expected_techniques, technique_applies_web_aware}`；VulnTriage resolver返回空时按既有Gate规则回退spec静态全集。`registered_capability_for`只做versioned technique→adapter capability registry，启动/contract test要求其domain精确等于四个stage spec的技术并集；spec新增一项而registry未登记必须失败，不能被denominator静默漏掉。Gate和sealer必须调用同一builder/共享resolver，不能复制第二套技术或applicability矩阵。

### Step 5：在 provider dispatch 前 seal

Generic wave：

- 给 <code>stage_asset_wave_current_or_create_initial_impl</code> 增加 trusted <code>stage_execution_id</code> 参数。
- <code>current_or_create_initial</code> 返回sealed wave identity后，只把stable request + stage execution + wave id交给上述单一repo compound；asset query、build items与seal都留在同一transaction，seal失败直接返回error，调用方不能启动provider。
- same wave retry 返回相同 denominator。

Company stage team：

- <code>seed_stage_team_runtime</code> durable seed 返回后、映射返回值前，按每个seeded unit的<code>stable request + stage_execution_id + unit_id</code>调用同一server-derived seal compound。
- 任一 unit seal 失败，bridge 返回 error；runtime 不 dispatch 任一 worker。
- 不使用 model-authored work item prose 作为 denominator。

核心顺序必须在代码中保持：

~~~rust
let wave = golish_db::repo::stage_asset_waves::current_or_create_initial(
    &self.pool,
    operation_id,
    organization_id,
    stage_kind,
    started_at,
    limit,
)
.await?;
let Some(wave) = wave else {
    return Ok(None);
};
let denominator = self
    .tool_truth_seal_denominator(SealToolTruthDenominatorRequest {
        stable_seal_request_id: stable_uuid_v5(wave.wave.id, stage_execution_id),
        stage_execution_id,
        source: ToolTruthDenominatorSourceRef::StageAssetWave {
            stage_asset_wave_id: wave.wave.id,
        },
    })
    .await?;
tracing::debug!(
    denominator_id = %denominator.id,
    wave_id = %wave.wave.id,
    "tool-truth denominator sealed before provider dispatch"
);
Ok(Some(stage_asset_wave_to_view(wave)))
~~~

Verification 不在 Task 9 根据 actions 临时构造 campaign denominator；Plan A 只覆盖 stage/org/wave Tool Truth 和 Task 8 的 no-match fail-safe。

### Step 6：把执行中发现的dynamic children纳入同一Gate closure

`tool_truth.rs`新增`evaluate_dynamic_child_closure`，输入只能来自repo重读的immutable parent receipt/manifest/member/closure，不能从deliverable prose或当前business table猜测：

1. 从每个current parent receipt的versioned capability contract取得应产生的child-manifest kind exact set；缺整张manifest是`hold/incomplete`，不是sealed empty。
2. 对每张manifest重算member count/hash；`expected_child_count=0`只接受显式`sealed_empty=true`。
3. 每个非空manifest必须exact-one derived denominator，每个member必须exact-one closure。`downstream_denominator_item`必须属于该derived denominator，且同operation/org/current attempt、technique/capability/child identity完全相等，并继续要求该item有current terminal receipt；`not_applicable/blocked`必须带只覆盖该child的exact residual。
4. 任一parent receipt superseded/orphaned、late旧attempt child、漏/额外closure或downstream item未终态，整体为`hold/incomplete`；已解释blocked child只能进入degraded，不得complete。
5. child集合不能由后续business row反推；例如“先seal root→发现三端口→seal独立derived fingerprint denominator→只处理两个”必须HOLD，五script只解析四个也必须HOLD。只有显式空manifest才表示“检查后没有child”；向root追加item或重seal必须由DB/repo拒绝。

DB bridge公开一次set-based read返回整个closure graph及manifest hash；禁止Gate逐member N+1查询。repo integration test同时验证manifest/member/closure append-only、同值重放、hash drift和跨attempt link拒绝。

### Step 7：在 Gate 后 shadow-write，不改变 GateResult

<code>evaluate_org_stage_gate</code> 的末尾先保存 legacy result，再计算/写 assessment：

~~~rust
let legacy_result = validate_stage_gate_with_context(
    deliverable,
    &spec,
    contract.as_ref(),
    skeleton.as_ref(),
    &gate_context,
);

if let Some(operation_id) = operation_id {
    if let Ok(tool_truth_contract) = repo.tool_truth_contract(operation_id).await {
        if tool_truth_contract.writes_receipts() {
            let shadow = evaluate_shadow_tool_truth(
                repo,
                operation_id,
                org_id.expect("receipt stages require organization"),
                stage,
                current_wave.map(|wave| wave.id),
                legacy_result.allowed,
            )
            .await;
            if let Err(error) = shadow {
                tracing::error!(
                    operation_id = %operation_id,
                    stage = stage.as_str(),
                    error = %error,
                    "tool-truth shadow assessment failed"
                );
            }
        }
    }
}

legacy_result
~~~

约束：

- shadow write failure 不修改 frozen legacy Gate authority。
- missing denominator 写 hold/incomplete assessment，不静默跳过。
- assessment 持久化 <code>legacy_allowed</code> 与新 control/grade，divergence 由两者确定。
- <code>GateResult</code> 结构不加 grade，避免现有 consumer/UI误读。
- receipt_v1 producer projection已经 fail-safe，因此危险 partial 仍会让旧 Gate 保守；新 grade 本计划内仍不成为用户可见 authority。

### Step 8：运行 GREEN 与 app bridge tests

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(denominator_items_are_asset_times_expected_technique) | test(denominator_catalog_is_exactly_the_embedded_spec_and_shared_applicability) | test(new_spec_technique_without_registered_capability_fails_closed) | test(tool_truth_shadow_grade_does_not_change_legacy_gate_result) | test(missing_denominator_is_shadow_incomplete) | test(three_discovered_ports_with_only_two_fingerprinted_hold_gate) | test(five_discovered_scripts_with_four_parsed_hold_gate) | test(explicit_sealed_empty_child_manifest_is_not_missing) | test(preflight_blocked_is_prerequisite_gap_not_content_coverage)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -E 'test(public_denominator_request_cannot_omit_members_or_rebind_a_source) | test(stage_asset_wave_seals_denominator_before_return) | test(stage_team_seed_seals_each_unit_before_return) | test(tool_truth_shadow_write_is_operation_and_org_scoped)')
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts -E 'test(denominator) | test(gate_assessment)')
~~~

**Expected:** 所列tests全绿，exit code 0；fixture明确证明public denominator seam只有stable source identity、repo在同一transaction server-derive exact members且request不能重绑，legacy result未因shadow divergence改变，四stage denominator与唯一spec/resolver一致，dynamic children没有漏闭合。

### Step 9：Future Commit

~~~bash
git add backend/crates/golish-agent-kit/src/harness/tool_truth.rs backend/crates/golish-agent-kit/src/harness/mod.rs backend/crates/golish-agent-kit/src/harness/org_gate.rs backend/crates/golish-agent-kit/src/db_traits/types.rs backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-app/src/ai/db_bridge/tool_truth.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs backend/crates/golish-agent-app/src/ai/db_bridge/orchestration.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-db/src/repo/capability_execution_receipts.rs
git commit -m "feat(tool-truth): seal coverage and shadow gate grades"
~~~

---

## Task 10：让 TargetIntel 逐 current attempt 写 exact receipt，禁止复用旧 source terminal row

**文件：**

- 修改：<code>backend/crates/golish-recon-app/src/intel_providers.rs</code>
- 修改：<code>backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs</code>
- 修改：<code>backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs</code>
- 修改：<code>backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/harness/tool_truth.rs</code>
- 测试：上述文件现有test modules与<code>backend/crates/golish-db/tests/capability_execution_receipts.rs</code>

### Step 1：写 RED attempt-isolation 与 producer lifecycle tests

~~~rust
#[tokio::test]
async fn prior_attempt_source_terminal_rows_cannot_close_current_target_intel() {
    let fixture = TargetIntelReceiptFixture::new().await;
    fixture.seed_attempt_n_legacy_source_terminals_for_all_applicable_axes().await;
    let current = fixture.start_attempt_n_plus_one().await;

    let assessment = fixture.evaluate_tool_truth(current.stage_execution_id).await;
    assert_eq!(assessment.control_decision, "hold");
    assert_eq!(assessment.coverage_grade, "incomplete");
    assert_eq!(assessment.current_terminal_receipt_count, 0);
}

#[tokio::test]
async fn late_attempt_n_receipt_is_superseded_not_current() {
    let fixture = TargetIntelReceiptFixture::new().await;
    let old = fixture.begin_attempt_n_dns_receipt().await;
    let current = fixture.start_attempt_n_plus_one().await;
    fixture.complete_dns_receipt(old.id).await;

    assert_eq!(fixture.receipt(old.id).await.reconciliation_state, "superseded");
    assert_eq!(fixture.current_receipts(current.stage_execution_id).await.len(), 0);
}

#[tokio::test]
async fn only_exact_current_target_intel_receipt_set_can_complete() {
    let fixture = TargetIntelReceiptFixture::new().await;
    let current = fixture.start_attempt_n_plus_one().await;
    fixture.complete_every_frozen_input_via_begin_witness_close_reconcile(&current).await;

    let assessment = fixture.evaluate_tool_truth(current.stage_execution_id).await;
    assert_eq!(assessment.control_decision, "allow");
    assert_eq!(assessment.coverage_grade, "complete");
    assert_eq!(assessment.receipt_manifest_hash, current.denominator_manifest_hash);
}

#[tokio::test]
async fn current_attempt_missing_one_target_intel_axis_stays_incomplete() {
    let fixture = TargetIntelReceiptFixture::new().await;
    let current = fixture.start_attempt_n_plus_one().await;
    fixture.complete_all_except(&current, "GOLISH-INTEL-WHOIS").await;
    let assessment = fixture.evaluate_tool_truth(current.stage_execution_id).await;
    assert_eq!(assessment.control_decision, "hold");
    assert_eq!(assessment.missing_techniques, vec!["GOLISH-INTEL-WHOIS"]);
}

#[test]
fn provider_transport_returns_host_observed_execution_envelope() {
    let transport = ScriptedToolTruthPinnedTransport::success(b"provider raw bytes");
    let result = run_provider_with_observed_transport(&transport, provider_request());
    assert_eq!(result.actual_budget.axis("requests"), Some(1));
    assert!(result.actual_budget.required_axes_all_observed());
    assert_eq!(result.raw_witness_bytes, b"provider raw bytes");
    assert!(result.destination_policy_sealed);
    assert_eq!(result.network_hops.len(), 1);
}

#[test]
fn provider_transport_cannot_turn_target_input_into_destination_authority() {
    let transport = ScriptedToolTruthPinnedTransport::new();
    let request = provider_request_with_target("https://169.254.169.254/latest/meta-data");
    let envelope = run_provider_with_observed_transport(&transport, request).expect("escaped query");
    assert_eq!(envelope.network_hops[0].normalized_host, "fixed.provider.example.test");
    assert!(!envelope.network_hops[0].url.contains("169.254.169.254"));
}

#[test]
fn provider_transport_blocks_mixed_dns_redirect_and_rebinding_before_send() {
    for fault in [
        ProviderEgressFault::MixedPublicAndPrivateDns,
        ProviderEgressFault::RedirectOutsideExactAllowlist,
        ProviderEgressFault::DnsRebindAfterPin,
    ] {
        let transport = ScriptedToolTruthPinnedTransport::with_fault(fault);
        let error = run_provider_with_observed_transport(&transport, provider_request())
            .expect_err("every hop is re-authorized and pinned");
        assert_eq!(error.code(), "TOOL_TRUTH_DESTINATION_POLICY_BLOCKED");
        assert_eq!(transport.unmanaged_send_count(), 0);
    }
}
~~~

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -E 'test(prior_attempt_source_terminal_rows_cannot_close_current_target_intel) | test(late_attempt_n_receipt_is_superseded_not_current) | test(only_exact_current_target_intel_receipt_set_can_complete) | test(current_attempt_missing_one_target_intel_axis_stays_incomplete)')
just space-guard
(cd backend && cargo nextest run -p golish-recon-app -E 'test(provider_transport_returns_host_observed_execution_envelope) | test(provider_transport_cannot_turn_target_input_into_destination_authority) | test(provider_transport_blocks_mixed_dns_redirect_and_rebinding_before_send)')
~~~

**Expected:** tests失败；当前TargetIntel仍可能从`source_query_log`/latest-session terminal projection取结果，也没有逐current attempt的begin→witness→close→reconcile接线。

### Step 3：在provider host seam观察执行，不让recon-app反向依赖DB

`golish-recon-app`定义纯`ObservedProviderExecutionEnvelope`，只返回server-owned provider id/version、exact request input key、raw witness bytes、typed observations、逐轴actual budget、destination policy id/hash和network-hop exact census。HTTP provider只能经Plan A的`ToolTruthPinnedTransportV1`；host从versioned provider registry冻结唯一endpoint/resolver allowlist并seal policy，target字符串只能成为escaped query/body/qname，不能决定scheme/host/port/path。transport逐initial/redirect/retry重验全量A/AAAA、scope/fixed-endpoint membership、proxy/TLS/redirect policy，pin validated IP并记录hop receipt；混合public+private DNS、redirect越界、rebind或N+1 send均在I/O前阻断。provider自报计数不能作为authority。timeout、redirect/retry budget耗尽、body truncation、parser reject都保留raw witness并产生partial/unknown信息。

`golish-recon-app`不得依赖`golish-db`或直接写receipt。`golish-agent-app`在dispatch前从已seal denominator加载`TargetIntelReceiptContext`，逐exact denominator item调用Plan A生命周期：

~~~text
load current operation/org/stage_execution/attempt_epoch/denominator item
  -> build open destination-policy header + exact fixed endpoint members, recompute and seal
  -> begin receipt with frozen capability + budget axes + sealed destination policy
  -> run provider outside DB transaction through ToolTruthPinnedTransportV1
  -> append exact network-hop decisions for every initial/redirect/retry attempt
  -> RawWitnessAuthorityHost::seal_and_stage_verified（vault callback内module-private staging）
  -> stage closeout with typed source ranges + exact input status + actual axes
  -> vault callback AEAD-verifies/decrypts a stable snapshot, then atomically appends reconciliation + finalizes current authority
  -> write legacy source_query_log projection only after canonical receipt finalize
~~~

同一个receipt id贯穿managed/background completion；response loss只exact replay，不重发provider。provider返回空数据不能仅凭HTTP 200变成no_match/complete：只有versioned provider contract明确声明该exact query是exhaustive、raw/typed/budget/reconciliation全部完整时才允许no_match；其余均为inconclusive + residual。

### Step 4：把current-attempt receipt设为receipt_v1唯一coverage authority

`stage_coverage.rs`、`db_bridge/recon.rs`和`db_bridge/evidence.rs`按operation-frozen contract分流：

- `legacy_v1`保持现有`source_query_log`/technique outcome行为；
- `shadow_v1`继续让旧projection决定legacy Gate，但Tool Truth assessment只读receipt exact set并记录divergence；
- `receipt_v1`的TargetIntel coverage只从`operation_id + organization_id + stage_execution_id + attempt_epoch + denominator_id` current receipts读取，要求latest reconciliation current/consistent；禁止latest session/run fallback；
- earlier attempt source rows和late receipt保留审计但标superseded，不能参与current manifest hash、checked-empty或Gate；
- source terminal compatibility rows必须带receipt id/attempt epoch projection metadata；缺该绑定的旧row在receipt_v1一律advisory；
- expected axis完全来自Task 9同一StageSpec/applicability builder，因此domain/IP/URL/wildcard只检查frozen applicable set，不硬编码“总是六项”。

### Step 5：运行 GREEN 与跨attempt DB回归

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-recon-app -E 'test(provider_transport_returns_host_observed_execution_envelope) | test(provider_transport_cannot_turn_target_input_into_destination_authority) | test(provider_transport_blocks_mixed_dns_redirect_and_rebinding_before_send) | test(provider_request_n_plus_one_is_rejected_before_send) | test(provider_empty_requires_exhaustive_contract)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -E 'test(prior_attempt_source_terminal_rows_cannot_close_current_target_intel) | test(late_attempt_n_receipt_is_superseded_not_current) | test(only_exact_current_target_intel_receipt_set_can_complete) | test(current_attempt_missing_one_target_intel_axis_stays_incomplete) | test(target_intel_receipt_projection_keeps_legacy_mode_unchanged)')
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts -E 'test(target_intel) | test(late_prior_attempt)')
~~~

**Expected:** 所列tests全绿，exit code 0；attempt N的DNS/WHOIS/ASN/CT/SUBDOMAIN/OSINT terminal rows不能关闭N+1，只有N+1 frozen applicable input exact receipts才能complete；provider destination固定来自registry，target不能成为destination authority，redirect/mixed-DNS/rebind均在send前阻断。所有测试使用scripted pinned transport且不访问真实provider。

### Step 6：Future Commit

~~~bash
git add backend/crates/golish-recon-app/src/intel_providers.rs backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs backend/crates/golish-agent-kit/src/harness/tool_truth.rs backend/crates/golish-db/tests/capability_execution_receipts.rs
git commit -m "fix(tool-truth): isolate target intel receipts by attempt"
~~~

---

## Task 10b：实现有界 Tool Truth revalidation orchestrator，避免TTL过期永久卡Gate

**文件：**

- 创建：<code>backend/crates/golish-db/src/repo/tool_truth_revalidation.rs</code>
- 修改：<code>backend/crates/golish-db/src/repo/mod.rs</code>
- 创建：<code>backend/crates/golish-agent-kit/src/task_orchestrator/tool_truth_revalidation.rs</code>
- 修改：<code>backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs</code>
- 创建：<code>backend/crates/golish-agent-app/src/ai/db_bridge/tool_truth_revalidation.rs</code>
- 修改：<code>backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs</code>
- 创建：<code>backend/crates/golish-agent-kit/tests/tool_truth_revalidation.rs</code>
- 继续测试：<code>backend/crates/golish-db/tests/capability_execution_receipts.rs</code>

### Step 1：写 RED obligation/liveness tests

覆盖：Candidate前TargetIntel negative TTL过期时checked bundle在同一transaction创建stable exact revalidation obligation并HOLD；重复B/C/D consumer合并到同一open obligation；final report download/打开UI可记录expired obligation但executor call=0；paused/finalized operation只有显式resume+授权后才可claim。scheduler claim必须有owner/lease/deadline/CAS和dispatch generation；scope/policy/budget不允许时零dispatch；active scan/T2/T3 refresh缺Prepared Action/JIT时零dispatch；成功只创建新attempt/denominator/receipt并关闭obligation，不修改旧receipt；下一次bundle选择新current authority后Candidate可继续。另覆盖worker crash reclaim、response loss exact replay、连续相同no-progress fingerprint、deadline/retry exhausted、critical axis不能靠普通risk acceptance放行。

所有producer使用scripted adapter/transport，panic mock证明Candidate/Campaign/Reporting本身不会因stale直接触网。

### Step 2：运行 RED

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app -p golish-db -E 'test(tool_truth_revalidation_)' --status-level fail)
~~~

**Expected:** tests因obligation repo、claim state machine与orchestrator尚不存在而失败；旧receipt仍可历史读取。

### Step 3：实现唯一owner的deterministic refresh loop

`with_checked_tool_truth_authority_bundle`发现expired/mixed-epoch/skew/invalid member时，在同一consumer transaction按`operation/org/source root/receipt/input/fact class/policy/reason`生成deterministic obligation id/hash并exact replay；B/C/D/UI/report download只拿obligation id/residual，不持有executor port，read操作本身绝不间接触网。后台`ToolTruthRevalidationOrchestrator`是唯一owner：canonical sort后claim，额外要求operation lifecycle仍active、explicit revalidation policy允许、revalidation dispatch hold关闭且generation匹配，再重验current scope、destination/temporal policy、operation/root discovery budget与continuation policy，按source capability contract开启新的revalidation attempt和immutable denominator，再复用Plan A begin→witness→close→reconcile生命周期。paused/finalized/inactive operation只保留open obligation；涉及active scan/T2/T3的refresh仍必须走Plan C Prepared Action/JIT authorization，不因“刷新TTL”降级。它不能UPDATE旧denominator/receipt或把旧raw bytes改时间。

operation创建事务同时冻结`tool_truth_revalidation_dispatch_policies`，历史与deployment default均为`manual_only`；`auto_passive_t0_t1`只允许显式选择且受max tier约束。独立dispatch head初始held，每次release/hold都CAS generation并写append-only event/outbox；本Plan不提供生产release setter。T2/T3无论policy值如何都不能自动dispatch，只能把obligation交给Plan C compiler/JIT。旧authorization不能跨hold on→off generation复活。

成功event精确绑定replacement denominator/receipt；下次consumer bundle由server current selector纳入新authority并关闭旧stale obligation。失败按bounded retry/deadline/no-progress进入`exhausted + exact residual`；只有frozen continuation policy允许且typed human risk-acceptance覆盖同一residual exact set时才能`risk_accepted`，mandatory axis继续HOLD。所有head transition与append-only event/typed outbox同事务，duplicate obligation不会生成并发refresh storm。

### Step 4：运行 GREEN

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app -p golish-db -E 'test(tool_truth_revalidation_)' --status-level fail)
~~~

**Expected:** stale→deduplicated obligation→bounded new attempt→fresh bundle路径全绿；失败路径稳定HOLD且无无限循环、无caller direct execution。

### Step 5：Future Commit

~~~bash
git add backend/crates/golish-db/src/repo/tool_truth_revalidation.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-agent-kit/src/task_orchestrator/tool_truth_revalidation.rs backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs backend/crates/golish-agent-app/src/ai/db_bridge/tool_truth_revalidation.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs backend/crates/golish-agent-kit/tests/tool_truth_revalidation.rs backend/crates/golish-db/tests/capability_execution_receipts.rs
git commit -m "feat(tool-truth): revalidate stale evidence with bounded obligations"
~~~

---

## Task 11：定向回归、模块卡与 evidence-ready 收尾

**文件：**

- 修改：<code>docs/modules/backend/golish-pentest-domain.md</code>
- 修改：<code>docs/modules/backend/golish-db.md</code>
- 修改：<code>docs/modules/backend/golish-db/repo.md</code>
- 修改：<code>docs/modules/backend/golish-pentest-app/pentest_bridge.md</code>
- 修改：<code>docs/modules/backend/golish-agent-kit/harness.md</code>
- 修改：<code>docs/modules/backend/golish-agent-kit/db_traits.md</code>
- 修改：<code>docs/modules/backend/golish-agent-app/ai.md</code>
- 修改：<code>docs/modules/backend/golish-agent-runtime/agentic_loop.md</code>
- 修改：<code>docs/modules/backend/golish-recon-app.md</code>
- 修改：<code>docs/modules/INDEX.md</code>

实现 agent 还必须按 AGENTS.md 在自己的会话收尾时更新 <code>agent-progress.md</code> 与 <code>feature_list.json</code>；这些状态文件不与功能 commit 混在一起，且没有新鲜证据时不得把功能标为 passing。

### Step 1：更新模块卡

每张卡记录：

- 新公开类型/API；
- operation-frozen contract 与默认 legacy；
- producer lifecycle 和 transaction boundary；
- denominator owner、receipt owner、Gate shadow owner；
- raw witness 敏感数据边界；
- focused test 入口；
- Plan A 不修改 frontend/report，也不提供 promotion。

<code>docs/modules/INDEX.md</code> 只更新上述模块的状态/摘要，不改变无关模块。

### Step 2：运行格式与 JSON/diff 检查

~~~bash
just space-guard
cargo fmt --check --manifest-path backend/Cargo.toml
pnpm exec biome check resources/harness/stages/enumeration/spec.json
git diff --check
git diff --name-only
~~~

**Expected:** fmt、Biome、diff-check exit code 0；name-only 只包含本计划“新建/修改”清单、AGENTS.md 要求的模块卡/状态文件，不包含 frontend/generated/report/design。

### Step 3：运行受影响 package 的 focused clippy

~~~bash
just space-guard
(cd backend && cargo clippy -p golish-pentest-domain --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-db --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-pentest-app --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-agent-kit --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-recon-app --all-targets -- -D warnings)
~~~

**Expected:** 每条命令 exit code 0，0 warnings。不运行全 workspace clippy。

### Step 4：重放最小验收矩阵

~~~bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-domain -E 'test(checked_empty_requires_every_axis_and_all_frozen_inputs) | test(positive_partial_is_not_complete) | test(consistent_partial_without_exact_residual_is_incomplete) | test(stable_noncritical_exhaustion_can_be_allow_degraded) | test(critical_or_unaccepted_exhaustion_holds_even_when_stable) | test(byte_consistent_but_temporally_expired_fact_holds) | test(illegal_terminal_tuples_and_producer_verdicts_fail_closed)')
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts)
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(whatweb_exit_zero_without_per_origin_witness_is_nonterminal) | test(full_port_plan_and_exit_zero_without_trusted_probe_census_is_partial) | test(blocked_preflight_keeps_all_content_axes_partial) | test(positive_extract_preserves_found_observation_but_projects_partial_coverage) | test(anonymous_positive_sibling_keeps_signal_but_projects_partial_coverage) | test(complete_nuclei_no_match_is_nonterminal_without_versioned_negative_oracle) | test(nuclei_complete_no_match_is_inconclusive_without_negative_oracle) | test(receipt_v1_nuclei_match_is_signal_not_oracle_proof) | test(nuclei_no_templates_is_unsupported_not_complete_not_applicable) | test(request_budget_guard_rejects_n_plus_one_before_send)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(preflight_blocked_is_prerequisite_gap_not_content_coverage) | test(denominator_catalog_is_exactly_the_embedded_spec_and_shared_applicability) | test(three_discovered_ports_with_only_two_fingerprinted_hold_gate) | test(five_discovered_scripts_with_four_parsed_hold_gate) | test(explicit_sealed_empty_child_manifest_is_not_missing) | test(tool_truth_shadow_grade_does_not_change_legacy_gate_result) | test(missing_denominator_is_shadow_incomplete)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -E 'test(stage_asset_wave_seals_denominator_before_return) | test(stage_team_seed_seals_each_unit_before_return) | test(tool_truth_shadow_write_is_operation_and_org_scoped) | test(prior_attempt_source_terminal_rows_cannot_close_current_target_intel) | test(only_exact_current_target_intel_receipt_set_can_complete)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-runtime -E 'test(stage_asset_wave_is_sealed_before_worker_dispatch)')
just space-guard
(cd backend && cargo nextest run -p golish-recon-app -E 'test(provider_transport_returns_host_observed_execution_envelope) | test(provider_transport_cannot_turn_target_input_into_destination_authority) | test(provider_transport_blocks_mixed_dns_redirect_and_rebinding_before_send) | test(provider_request_n_plus_one_is_rejected_before_send) | test(provider_empty_requires_exhaustive_contract)')
~~~

**Expected:** 所有命令 exit code 0，并提供以下逐条证据：

1. exit 0 + incomplete input 不能 complete/checked-empty；
2. raw positive + parser reject 是 partial/orphan；
3. previous-attempt late row不能关闭 current attempt；
4. 第 N+1 个 direct HTTP request 在 send 前拒绝；
5. Enumeration transport failure 不关闭四个 content axis；
6. positive observation 不掩盖 partial coverage；
7. Nuclei no-match 不产生 receipt_v1 checked-empty/refutation；
8. missing/pending/orphan denominator 为 hold/incomplete；
9. stable bounded exhaustion 才可能 allow/degraded；
10. shadow divergence 不改变 legacy Gate result；
11. TargetIntel previous-attempt/source-terminal事实不能关闭current attempt；
12. dynamic child manifest/derived denominator中的每个port/script/endpoint都exact-one收口；
13. raw artifact、budget plan/actual与denominator seal在direct SQL下不可篡改；
14. public denominator request没有asset/item/count/hash字段，repo在locked source transaction内server-derive exact set，request重绑与漏项尝试均失败；
15. raw persistent bytes全为ciphertext，adapter/audit DTO没有vault object key；snapshot/finalize/authority-set/bundle writer只能在同一vault callback持有不可构造guard时运行，漏root/member或伪造hash/size均失败。

### Step 5：记录证据与 rollout 状态

在 <code>agent-progress.md</code> 记录每条实际命令、exit code 和关键输出；在 <code>feature_list.json</code> 对应 verification/evidence 中记录：

- schema 获得批准的聊天证据；
- migration/repo focused tests；
- producer fault-injection tests；
- Gate shadow divergence tests；
- 大型门禁“按项目策略未运行”；
- deployment default 仍是 legacy_v1；
- 没有 promotion endpoint；
- 没有 frontend/generated/report 变更。

只有 AGENTS.md 完成定义全部满足时才标 passing；否则保留 in_progress 并写明剩余风险。

### Step 6：Future Commit

~~~bash
git add docs/modules/backend/golish-pentest-domain.md docs/modules/backend/golish-db.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/db_traits.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-recon-app.md docs/modules/INDEX.md agent-progress.md feature_list.json
git commit -m "docs(tool-truth): record contracts and focused evidence"
~~~

---

## 并行实施边界

Task 1 → schema 授权暂停点 → Task 2 → Task 3 → Task 4 必须串行。

Task 4 完成后可并行：

- Worker A：Task 5 EAS；
- Worker B：Task 6 Enumeration；
- Worker C：Task 7 JS + anonymous；

三个worker不得同时修改<code>pentest_bridge/tool_truth.rs</code>；Task 7对共享projection的修改由主agent先落，再并行分派具体producer文件。Task 8在共享projection落地后执行。Task 9等待Tasks 5–8的receipt facts稳定后执行；Task 10依赖Task 9已能为TargetIntel seal exact denominator。主agent负责合并、复核SubAgent结论并执行Task 11。

## Rollback 与 cutover 证据

- 默认仍为 legacy_v1，因此部署新 binary 后不改变新建或既有 operation 的行为。
- 本计划代码不提供任何生产切换接口，也不允许人工直接UPDATE已冻结operation；shadow_v1/receipt_v1只在自动回滚typed fixture中使用。Plan B migration存在时，任何创建operation的fixture都必须使用七态中的完整合法joint pair，不能只更新Tool Truth singleton。
- Plan D 的 joint promotion 获得单独授权后，才可以根据 shadow assessment、divergence、orphan、missing denominator、budget-unobserved 与 Investigation cohort evidence推进“之后创建的 operation”默认 pair；Campaign-authoritative pair必须是 receipt_v1。
- operation row冻结后不可原地切换。回退 binary 时保留 additive tables/rows；不删除 receipt audit truth。
- shadow writer失败只产生稳定日志和 missing assessment，不改变 legacy Gate；receipt_v1 producer closeout失败必须 fail closed 为 partial/outcome_unknown，不能回落到 legacy green。
- 若 migration 已应用但代码尚未部署，operation column default legacy_v1，旧 binary 不读取新列；additive tables无写入。
- 若代码已部署但 migration 不存在，应用启动/migration gate应按现有 sqlx流程失败，不能绕过 schema version检查。

## 本计划完成后仍明确不解决的事项

- versioned recipe negative oracle 与 campaign refutation authority；
- Verification campaign denominator、Prepared Action 和 action execution receipt；
- browser/CLI工具内部无法观测的真实request/probe ceiling；这些receipt必须让每个required budget axis分别保持`observed=false, actual_value=NULL, observation_source=cli_unobserved`，因此不能获得complete；
- user-visible PASS_WITH_GAPS、API/UI/report 映射与 rollout promotion；
- Registry/Hypothesis evolution 与 Reporting canonical-source cutover。

这些边界不是缺失实现，而是 Plan A 与 Plan C/D 的明确接口；不得用 legacy checked-empty/refutation 临时填补。
