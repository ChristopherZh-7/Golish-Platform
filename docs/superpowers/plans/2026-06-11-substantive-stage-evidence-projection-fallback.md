# Substantive 阶段证据投影兜底 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。

**目标：** substantive 情报阶段 agent 没交 StageDeliverable 但账本有真实 evidence facts 时，harness 从账本投影合成 deliverable（findings 永远空）走原 gate，替代 missing-deliverable 死锁 BLOCK。
**架构：** `StageSpec` 加 opt-in 布尔（serde default false）→ `execute.rs` stage-close 的 substantive BLOCK 分支改为「门控开 + facts 非空 → `synthesize_from_evidence` 纯函数投影；否则原 BLOCK」→ `target_intel.json` 单阶段灰度开关。投影 coverage 留空靠 gate `derive_from_evidence`（PR3）补，claims 只对 Found facts 各产一条（D3）。
**技术栈：** Rust（golish-agent-kit），cargo nextest。

> 设计：`docs/design/2026-06-11-substantive-stage-evidence-projection-fallback.md`（D1-D5 已拍板：opt-in spec 字段 / coverage 留空靠 gate / 每 Found fact 一 claim / 不做熔断 / 先只 target_intel）。
> 执行模式：用户 2026-06-11 指示「全部搞完再编译检测错误，然后自己跑测试看看结果」——任务 1-4 批量写完（含测试），任务 5 统一 `cargo check` + `cargo nextest`（替代逐任务先红后绿）。

---

## 文件清单

| 文件 | 职责 | 改动 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/stage_spec.rs` | StageSpec DTO | 加 `synthesize_from_evidence_when_missing: bool`（serde default）+ 默认值/解析守卫测试 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | stage-close gate hook | 新增 `synthesize_from_evidence` 纯函数；改 `apply_harness_gate_hook` 的 substantive missing-deliverable 分支；新增单测 |
| `resources/harness/stages/target_intel.json` | target_intel spec | 加 `"synthesize_from_evidence_when_missing": true` |
| `feature_list.json` | harness 台账 | 登记本功能条目 |

## 任务 1 · StageSpec 字段

**文件：** `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`

在 `expected_techniques` 字段后加：

```rust
    /// 设计 2026-06-11-substantive-stage-evidence-projection-fallback：substantive
    /// 阶段 agent 没交 deliverable 但账本有真实 evidence facts 时，允许 harness 从
    /// 账本投影合成 deliverable（findings 永远空）走原 gate。缺省 false = 原
    /// fail-closed BLOCK 行为逐字节不变。漏洞类阶段（vuln_triage 等）永不开。
    #[serde(default)]
    pub synthesize_from_evidence_when_missing: bool,
```

tests mod 加：默认 false（minimal inline spec 不写字段 → false）、显式 true 可解析、`target_intel.json` 解析出 true（任务 4 之后才绿）、`vuln_triage.json` 解析出 false。

## 任务 2 · `synthesize_from_evidence` 纯函数

**文件：** `execute.rs`（紧跟 `synthesize_confirm_only_deliverable` 之后）

```rust
fn synthesize_from_evidence(
    stage: crate::harness::StageKind,
    facts: &[crate::harness::gate::rule_engine::EvidenceFact],
) -> crate::harness::StageDeliverable {
    use crate::harness::gate::rule_engine::EvidenceOutcome;
    let claims = facts.iter()
        .filter(|f| f.outcome == EvidenceOutcome::Found)
        .map(|f| crate::harness::StageClaim {
            kind: format!("{}_evidence", stage.as_str()),
            subject: f.asset.clone(),
            summary: format!("Backend-projected from ledger evidence #{} ({}): the agent ran this technique but submitted no parseable StageDeliverable; findings are NEVER projected.", f.evidence_id, f.technique),
            evidence_ids: vec![EvidenceAuditId::new(f.evidence_id)],
            technique: Some(f.technique.clone()),
        })
        .collect();
    let mut ids: Vec<i64> = facts.iter().map(|f| f.evidence_id).collect();
    ids.sort_unstable(); ids.dedup();
    crate::harness::StageDeliverable {
        stage_id: stage.as_str().to_string(),
        stage_run_id: uuid::Uuid::new_v4(),
        claims,
        evidence_refs: ids.into_iter().map(EvidenceAuditId::new).collect(),
        skipped_checks: vec![], findings: vec![],
        required_checks_done: vec![], coverage: vec![],
    }
}
```

要点：claims 只产 Found（D3；Empty 行的 CheckedEmpty 格由 gate `derive_from_evidence` 投影）；evidence_refs 含全部 facts id（Found+Empty）去重排序；coverage 留空（D2）；findings/skipped/required 全空（红线）。technique 原样来自账本不过滤——脏值由 schema_check fail-closed 暴露。

单测（execute.rs tests mod）：① findings/coverage 恒空；② Found fact → claim（kind/subject/technique/evidence_ids 对齐）；③ Empty fact 不产 claim 但 id 进 evidence_refs；④ 重复 id 去重；⑤ stage_id = stage.as_str() 且 stage_run_id 非 nil。

## 任务 3 · substantive 分支改造

**文件：** `execute.rs` `apply_harness_gate_hook` 的 `None` 分支 else 臂（原 ~L1850）：

```rust
} else {
    let projection_enabled =
        crate::harness::load_embedded_stage_spec(stage_hint.stage_kind)
            .map(|s| s.synthesize_from_evidence_when_missing)
            .unwrap_or(false);
    let ledger_facts = evidence_facts.as_deref().filter(|f| !f.is_empty());
    match (projection_enabled, ledger_facts) {
        (true, Some(facts)) => {
            tracing::warn!(target: "harness::hook", /* stage/facts 计数字段 */
                "substantive stage produced no parseable StageDeliverable — synthesizing from ledger evidence facts (projection fallback), findings stay empty");
            synthesize_from_evidence(stage_hint.stage_kind, facts)
        }
        _ => { /* 原 warn + return missing_deliverable_gate_outcome 不动 */ }
    }
}
```

借用安全：`as_deref()` 只借用，后续 `gate_ctx` 仍可 move `evidence_facts`。投影产物落入原 gate 流（schema/contract/vacuous/freshness + gate_rules + 后续 `enforce_evidence_existence`），outcome 的 `missing_deliverable=false`（gate 正常跑了）。

单测：① 门控 off（EAS spec 无字段）+ 有 facts → 仍 BLOCK 且 `missing_deliverable=true`；② vuln_triage（off）同①；③ 门控 on（target_intel）+ facts → outcome `missing_deliverable=false` 且 evidence_refs 来自投影；④ 门控 on + facts None/空 → 原 BLOCK。

## 任务 4 · target_intel.json 灰度开关

`"max_other_skips": 2` 之前加一行：

```json
  "synthesize_from_evidence_when_missing": true,
```

## 任务 5 · 统一验证 + 登记

1. `cargo check -p golish-agent-kit` → exit 0
2. `cargo clippy -p golish-agent-kit --lib -- -D warnings` → exit 0
3. `cargo nextest run -p golish-agent-kit` → 全绿（新增测试全过、零回归）
4. `cargo fmt -p golish-agent-kit -- --check` → exit 0
5. ReadLints 改动文件
6. `feature_list.json` 登记条目（design/plan/verification/evidence）
7. 证据写 `agent-progress.md`（会话收尾时）

## 自检

- 规格覆盖：设计 §5.1 触发条件=任务 3；§5.2 投影函数=任务 2；§7 门控=任务 1+4；§11 单测=任务 2/3 测试；§11 活体=后续会话（需 key 跑活体，本轮 scope 是代码+测试）。
- 类型一致：`EvidenceFact{asset,technique,outcome,evidence_id:i64}`、`EvidenceAuditId::new(i64)`、`StageClaim.technique:Option<String>` 均与现源码核对过。
- 占位符：无。
