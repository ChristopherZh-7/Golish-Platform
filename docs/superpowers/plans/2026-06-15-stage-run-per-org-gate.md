# stage_run 每个 org 各过各的 gate 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。每个 Task 单独 commit；改 Rust 后跑 `just test-rust`/`just lint-rust`，收尾 `just precommit`。

**目标：** 让 chat `stage_run` 的 per-org 扇出真正做到「每个 org 各过各的权威 gate」——一个 org 只有它自己的 StageDeliverable **通过 DB 真值 gate** 才记 `passed`/写完成账本；否则进 `gaps`、不写账本、可被「只重跑缺口 org」闭环再测。

**架构：** 当前 `execute_stage_run` 用子 agent 的 `result.success`（= 子 agent 循环结束，与 gate 无关，见 `golish-sub-agents/src/executor/inner.rs:400`）当作 org「通过」，并据此写 7 天 resume 账本（`stage_run_call.rs:386-398`）。本计划新增一个**可复用的 per-org 权威 gate 评估器** `evaluate_org_stage_gate`（复用 orchestrator 已有的 DB 真值注入能力：`in_scope_assets(org_id)` / `in_scope_typed_assets(org_id)` / `evidence_facts_for_session` / `db_truth_facts(org_id, assets)` + `validate_stage_gate_with_context`），在 `stage_run` 里**每个 org 串行跑完后立刻**对该 org 的交付跑一次，用 PASS/BLOCK 取代 `result.success` 决定 `passed_count`/`gaps`/是否写账本。这是「闸 2 · 确定性裁判」，不依赖模型自觉。「闸 1 · 逼子 agent 过关才能收工」作为 Phase 2 单列（更动 sub-agent 执行器，blast radius 更大）。

**技术栈：** Rust 2021（golish-agent-kit / golish-agent-runtime），`cargo nextest`，`sqlx`（运行时绑定），`async_trait`。

---

## 关键事实（实读确认，动手前先核）

- `execute_stage_run`（`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`）串行 fan-out，每个 org 调 `execute_sub_agent_call`，`let ok = matches!(&result, Ok(r) if r.success)`（L386）→ `passed_count++` + `tracker.record_org_stage_completion(...)`（L392-398）。
- 子 agent `success`（`golish-sub-agents/src/executor/inner.rs:400-407`）在「达 max_iterations / 命中 barrier `submit_result` / 不再调工具」时**都 = true**，与交付是否过 gate 无关。
- 子 agent 的交付被写进**单槽** side-channel `harness_last_deliverable: Arc<RwLock<Option<String>>>`（`agent-bridge/.../mod.rs:250`），`ctx.harness_deliverable_sink`（`agentic_loop/context.rs:194`）是其写句柄；`sub_agent_call.rs:380-383` 在子 agent response 含 `stage_run_id` 时写入。**串行**执行下，每个 org 结束后该槽即是「这个 org 的交付」（下一个 org 才会覆盖）。
- DB 真值能力可由 `ctx.events.db_tracker` 取到：`DbTracker::repo() -> Option<&dyn DbRepoProvider>`（`golish-agent-kit/src/db_tracking/mod.rs:66`）。`DbRepoProvider`（`golish-agent-kit/src/db_traits/repo.rs`）已有：
  - `in_scope_assets(org_id: Option<Uuid>) -> Vec<String>`（L156）
  - `in_scope_typed_assets(org_id) -> Vec<(String,String)>`（L176，asset→type）
  - `evidence_facts_for_session(session_id) -> Vec<(String,String,String,i64)>`（L343，asset/technique/outcome/evidence_id）
  - `db_truth_facts(org_id, &[String]) -> Vec<(String,String)>`（L362，Found 语义）
  - `evidence_existing_ids(&[i64]) -> HashSet<i64>`（L377）
- 权威 gate 入口：`harness::gate::validate_stage_gate_with_context(deliverable, spec, contract: Option<&SprintContract>, skeleton: Option<&StageSkeleton>, ctx: &GateContext) -> GateResult { allowed, reasons, recovery_actions }`（`harness/gate/mod.rs:128`）。`GateContext { in_scope_assets, asset_types, expected_techniques, evidence_facts }`（`harness/gate/rule_engine.rs:194`）。`EvidenceFact { asset, technique, outcome: EvidenceOutcome::{Found,Empty}, evidence_id }`。
- orchestrator 已在 `subtask_phases/execute.rs` 用上面同一套 repo 方法做 gate 注入（L151/L1222/L1254/L1339），本计划的 `evaluate_org_stage_gate` 是把这套注入抽成「不依赖 `self`、可被 stage_run 复用」的自由函数（execute.rs 暂不改，blast radius 控制在 stage_run）。

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/org_gate.rs` | 新增。`evaluate_org_stage_gate`（async，repo+org_id+session+stage+deliverable → `GateResult`）+ 纯函数 `facts_from_rows`（行→`EvidenceFact`）+ 纯函数 `decide_org_verdict`（`&GateResult` → `OrgVerdict`）+ 单测 | 创建 |
| `backend/crates/golish-agent-kit/src/harness/mod.rs` | `pub mod org_gate;` + re-export `evaluate_org_stage_gate / OrgVerdict` | 改 |
| `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs` | 用 per-org gate 结果取代 `result.success` 决定 `passed/gaps/账本`；每 org 结束后从 sink 取该 org 交付过 gate | 改 |

Phase 2（单列，见末尾）：`golish-sub-agents/src/executor/inner.rs` + recon 专家定义（`defaults/builder/registry.rs`）——逼 stage 专家「过关才能收工」。

---

## Phase 1 · 闸 2：stage_run 里确定性 per-org gate（必做）

### Task 1 — 新建 `org_gate.rs`：纯函数 + 单测（先红）

**文件：** `backend/crates/golish-agent-kit/src/harness/org_gate.rs`（新建）

**步骤 1.1** 写纯函数 + 单测骨架（先只放纯函数，TDD 先红）：

```rust
//! Per-org 权威 gate 评估器（chat `stage_run` 扇出用）。
//!
//! `stage_run` 串行对每个 org 跑完专家后，用本模块对**该 org 自己的** StageDeliverable
//! 跑一次注入了该 org DB 真值的 gate（与 orchestrator stage-close gate 同一套
//! `validate_stage_gate_with_context` + 同一批 repo 查询），用 PASS/BLOCK 决定该 org
//! 是否算通过——取代旧的「子 agent 跑完即通过」。纯函数部分单测覆盖。

use uuid::Uuid;

use super::gate::rule_engine::{EvidenceFact, EvidenceOutcome, GateContext};
use super::gate::{validate_stage_gate_with_context, GateResult};
use super::stage_spec::StageSpec;
use super::types::StageDeliverable;
use super::{load_embedded_stage_spec, StageKind};
use crate::db_traits::DbRepoProvider;

/// 一个 org 在某 stage 的裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgVerdict {
    /// gate 通过：可计入 passed_count + 写完成账本。
    Pass,
    /// gate 未过：进 gaps，**不**写账本；`reasons` 供汇报 + 闭环回灌。
    Block { reasons: Vec<String> },
}

/// `GateResult` → `OrgVerdict`（纯函数，单测）。
pub fn decide_org_verdict(gate: &GateResult) -> OrgVerdict {
    if gate.allowed {
        OrgVerdict::Pass
    } else {
        OrgVerdict::Block {
            reasons: gate.reasons.clone(),
        }
    }
}

/// `evidence_facts_for_session` 的 `(asset, technique, outcome, id)` 行 →
/// `EvidenceFact`（纯函数，单测）。`outcome` 文本：`"found"` → Found，其余 → Empty
/// （I8：只有显式 found 才算 Found，绝不把别的当 Found）。
pub fn facts_from_rows(rows: Vec<(String, String, String, i64)>) -> Vec<EvidenceFact> {
    rows.into_iter()
        .map(|(asset, technique, outcome, id)| EvidenceFact {
            asset,
            technique,
            outcome: if outcome.eq_ignore_ascii_case("found") {
                EvidenceOutcome::Found
            } else {
                EvidenceOutcome::Empty
            },
            evidence_id: id,
        })
        .collect()
}

/// db_truth `(asset, technique)` → Found `EvidenceFact`（哨兵 id=0，与 execute.rs
/// `DB_TRUTH_EVIDENCE_ID` 同义：投影只看 asset/technique/outcome，与 id 无关）。
fn db_truth_to_facts(rows: Vec<(String, String)>) -> Vec<EvidenceFact> {
    rows.into_iter()
        .map(|(asset, technique)| EvidenceFact {
            asset,
            technique,
            outcome: EvidenceOutcome::Found,
            evidence_id: 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::gate::HarnessRecoveryActions;

    fn pass() -> GateResult {
        GateResult { allowed: true, reasons: vec![], recovery_actions: None, gate_result_id: None }
    }
    fn block(r: &str) -> GateResult {
        GateResult {
            allowed: false,
            reasons: vec![r.to_string()],
            recovery_actions: Some(HarnessRecoveryActions::default()),
            gate_result_id: None,
        }
    }

    #[test]
    fn verdict_pass_on_allowed() {
        assert_eq!(decide_org_verdict(&pass()), OrgVerdict::Pass);
    }

    #[test]
    fn verdict_block_carries_reasons() {
        assert_eq!(
            decide_org_verdict(&block("coverage incomplete")),
            OrgVerdict::Block { reasons: vec!["coverage incomplete".to_string()] }
        );
    }

    #[test]
    fn facts_only_found_outcome_maps_found() {
        let f = facts_from_rows(vec![
            ("a.com".into(), "GOLISH-INTEL-DNS".into(), "found".into(), 7),
            ("a.com".into(), "GOLISH-INTEL-WHOIS".into(), "empty".into(), 8),
        ]);
        assert_eq!(f[0].outcome, EvidenceOutcome::Found);
        assert_eq!(f[1].outcome, EvidenceOutcome::Empty);
        assert_eq!(f[0].evidence_id, 7);
    }

    #[test]
    fn db_truth_rows_are_found_sentinel() {
        let f = db_truth_to_facts(vec![("a.com".into(), "GOLISH-INTEL-ASN".into())]);
        assert_eq!(f[0].outcome, EvidenceOutcome::Found);
        assert_eq!(f[0].evidence_id, 0);
    }
}
```

**步骤 1.2** 在 `harness/mod.rs` 注册模块 + re-export：

```rust
pub mod org_gate;
pub use org_gate::{evaluate_org_stage_gate, OrgVerdict};
```

> 注：`evaluate_org_stage_gate` 在步骤 1.3 才定义；本步骤先只 `pub mod org_gate;`，待 1.3 后再补 re-export 那行，避免「引用未定义」编译错。

**步骤 1.3** 运行单测确认纯函数通过（此时尚无 async 评估器）：

```bash
cd backend && cargo nextest run -p golish-agent-kit org_gate
```
预期：`verdict_pass_on_allowed` / `verdict_block_carries_reasons` / `facts_only_found_outcome_maps_found` / `db_truth_rows_are_found_sentinel` 4 passed。

**步骤 1.4** Commit：`feat(harness): org_gate pure helpers (verdict + fact mapping)`。

---

### Task 2 — `org_gate.rs`：async `evaluate_org_stage_gate`

**文件：** `backend/crates/golish-agent-kit/src/harness/org_gate.rs`

**步骤 2.1** 追加 async 评估器（放在纯函数下方、`#[cfg(test)]` 上方）：

```rust
/// 对 `org_id` 的某 stage 交付跑一次注入了该 org DB 真值的权威 gate。
///
/// 复用 orchestrator stage-close 的同一批 repo 查询（`in_scope_assets` /
/// `in_scope_typed_assets` / `evidence_facts_for_session` / `db_truth_facts`）+ 同一个
/// `validate_stage_gate_with_context`，把判定按 org 隔离。先做一次 fabricated-ref 存在性
/// 兜底（与 execute.rs `enforce_evidence_existence` 同义；scoping 例外——它不产账本证据）。
///
/// 失败回退：spec 加载失败 → 直接 Block（fail-closed，配置坏不该放行）。注意 repo 缺失/
/// DB 错由调用方（stage_run）决定回退策略，本函数要求传入可用 repo。
pub async fn evaluate_org_stage_gate(
    repo: &dyn DbRepoProvider,
    org_id: Option<Uuid>,
    session_id: &str,
    stage: StageKind,
    deliverable: &StageDeliverable,
) -> GateResult {
    let spec: StageSpec = match load_embedded_stage_spec(stage) {
        Ok(s) => s,
        Err(e) => {
            return GateResult::block(
                vec![format!("could not load stage spec for {}: {e}", stage.as_str())],
                Default::default(),
            )
        }
    };

    // 1) fabricated-ref 兜底（scoping 不要求账本证据，跳过）。
    if stage != StageKind::Scoping {
        let cited: Vec<i64> = deliverable.evidence_refs.iter().map(|e| e.as_i64()).collect();
        if !cited.is_empty() {
            if let Ok(existing) = repo.evidence_existing_ids(&cited).await {
                let fabricated: Vec<i64> =
                    cited.into_iter().filter(|id| !existing.contains(id)).collect();
                if !fabricated.is_empty() {
                    return GateResult::block(
                        vec![format!(
                            "cited evidence ids {fabricated:?} do not exist in the evidence ledger"
                        )],
                        Default::default(),
                    );
                }
            }
            // infra error → 不在这兜底 BLOCK（与 execute.rs fail-open 一致），交给覆盖 gate。
        }
    }

    // 2) 资产轴 + 类型（org 隔离）。空资产集 → 不注入（gate 回退自报，coverage_complete
    //    自带「空矩阵但声明了期望技术 → BLOCK」保护，见 rule_engine.rs:373）。
    let in_scope_assets = repo.in_scope_assets(org_id).await.unwrap_or_default();
    let asset_types: Option<std::collections::HashMap<String, String>> = {
        let typed = repo.in_scope_typed_assets(org_id).await.unwrap_or_default();
        (!typed.is_empty()).then(|| typed.into_iter().collect())
    };

    // 3) 证据事实：账本投影 + DB 业务表真值（Found）合并。
    let mut facts: Vec<EvidenceFact> = repo
        .evidence_facts_for_session(session_id)
        .await
        .map(facts_from_rows)
        .unwrap_or_default();
    if !in_scope_assets.is_empty() {
        if let Ok(truth) = repo.db_truth_facts(org_id, &in_scope_assets).await {
            facts.extend(db_truth_to_facts(truth));
        }
    }

    let ctx = GateContext {
        in_scope_assets: (!in_scope_assets.is_empty()).then_some(in_scope_assets),
        asset_types,
        expected_techniques: None, // 回退 spec.expected_techniques（target_intel 已声明）
        evidence_facts: (!facts.is_empty()).then_some(facts),
    };

    validate_stage_gate_with_context(deliverable, &spec, None, None, &ctx)
}
```

**步骤 2.2** 补 `harness/mod.rs` re-export（步骤 1.2 留的那行）：

```rust
pub use org_gate::{evaluate_org_stage_gate, OrgVerdict};
```

**步骤 2.3** 编译 + clippy：

```bash
cd backend && cargo check -p golish-agent-kit && cargo clippy -p golish-agent-kit -- -D warnings
```
预期：exit 0（若 `as_i64()` 名称与 `EvidenceAuditId` 实际不符——核 `harness/types.rs` 中 evidence_refs 元素类型的取值方法，按真实方法名改）。

**步骤 2.4** Commit：`feat(harness): evaluate_org_stage_gate per-org authoritative gate`。

---

### Task 3 — stage_run 用 per-org gate 取代 `result.success`

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤 3.1** 在文件顶部 import 区加：

```rust
use golish_agent_kit::harness::org_gate::decide_org_verdict;
use golish_agent_kit::harness::{evaluate_org_stage_gate, OrgVerdict, StageDeliverable};
```

**步骤 3.2** 把 L386 起的「`let ok = matches!(...success); if ok { passed_count++; record... } else { gaps.push... }`」整块替换为「取该 org 交付 → 跑 per-org gate → 按裁决处理」。新逻辑：

```rust
        let sub_ok = matches!(&result, Ok(r) if r.success);

        // 取**这个 org** 自己的交付：串行执行下，sink 此刻即本 org 的最后一次 submit。
        let org_deliverable: Option<StageDeliverable> = match ctx.harness_deliverable_sink.as_ref() {
            Some(sink) => sink
                .read()
                .await
                .as_deref()
                .and_then(|s| serde_json::from_str::<StageDeliverable>(s).ok()),
            None => None,
        };
        // 取完即清，避免下一个 org 误用上一个 org 的残留交付。
        if let Some(sink) = ctx.harness_deliverable_sink.as_ref() {
            *sink.write().await = None;
        }

        // 权威裁决：必须有 repo + 该 org 的可解析交付，且 gate PASS 才算过。
        let verdict = match (
            ctx.events.db_tracker.and_then(|t| t.repo()),
            org_deliverable.as_ref(),
            uuid::Uuid::parse_str(&unit.id).ok(),
        ) {
            (Some(repo), Some(deliv), org_uuid) => {
                let session = ctx.events.session_id.unwrap_or("");
                let gate =
                    evaluate_org_stage_gate(repo, org_uuid, session, stage, deliv).await;
                decide_org_verdict(&gate)
            }
            // 没有 repo（纯 eval/无 DB 上下文）→ 退回旧的「子 agent 成功即过」以免回归测试/
            // headless 评测路径炸；真实运行恒有 DB。
            _ => {
                if sub_ok {
                    OrgVerdict::Pass
                } else {
                    OrgVerdict::Block {
                        reasons: vec!["sub-agent did not complete".to_string()],
                    }
                }
            }
        };

        match verdict {
            OrgVerdict::Pass => {
                passed_count += 1;
                if let (Some(tracker), Ok(org_id)) =
                    (ctx.events.db_tracker, uuid::Uuid::parse_str(&unit.id))
                {
                    tracker
                        .record_org_stage_completion(org_id, stage.as_str(), Some(&org_request_id))
                        .await;
                }
                emit_org_progress(
                    ctx, stage, unit, &org_request_id, "passed", None, 0,
                    &stage_label, &role_label, &coverage_axis,
                );
            }
            OrgVerdict::Block { reasons } => {
                let detail = if reasons.is_empty() {
                    match &result {
                        Ok(r) => r
                            .value
                            .get("response")
                            .or_else(|| r.value.get("error"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().take(300).collect::<String>())
                            .unwrap_or_default(),
                        Err(e) => e.to_string(),
                    }
                } else {
                    reasons.join("; ").chars().take(300).collect::<String>()
                };
                emit_org_progress(
                    ctx, stage, unit, &org_request_id, "blocked", None, 0,
                    &stage_label, &role_label, &coverage_axis,
                );
                gaps.push(json!({ "org_id": unit.id, "org_name": unit.name, "detail": detail }));
            }
        }
```

> `decide_org_verdict` 已在步骤 3.1 import（`use golish_agent_kit::harness::org_gate::decide_org_verdict;`），直接用 `decide_org_verdict(&gate)` 把 `GateResult` 转成 `OrgVerdict`。

**步骤 3.3** 删除现在已无用的 import / 变量（如旧 `let ok = ...` 残留）；`evaluate_org_stage_gate` 已含 spec 加载，stage_run 顶部原有的 `load_embedded_stage_spec` 仍用于 specialist/coverage_axis，保留。

**步骤 3.4** 编译 + 现有 stage_run 单测（纯解析测不受影响）：

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_run && cargo clippy -p golish-agent-runtime -- -D warnings
```
预期：`parse_org_units_*` / `build_org_objective_*` / `completion_freshness_*` / `tool_definition_requires_orgs` 全 passed；clippy exit 0。

**步骤 3.5** Commit：`fix(stage_run): record org pass on authoritative per-org gate, not sub-agent success`。

---

### Task 4 — 收尾门禁

```bash
just precommit
```
预期：`fmt` + `check-fe` + `test-fe` + `lint-rust` + `test-rust-all` 全绿。把命令 + 关键输出贴进 `agent-progress.md`「已记录证据」。

---

## Phase 1.5 · 阶段过门：fan-out 阶段「跑完 stage_run 即过」（hash 令牌 · 用户拍板）

> **目的**：specialist（fan-out）阶段的**阶段收尾**不再让主 agent 重交一份「整阶段」`StageDeliverable`、再跑一遍整阶段 coverage——这份「重交」既冗余（每个 org 在 Phase 1 已各过各 gate）、单槽 side-channel 又存不下 N 个 org、且整库资产轴 `org_id=None` 会分母爆炸（设计 2026-06-09 的老坑）。改为：stage_run 全过（11/11，Phase 1 后是**真** gate PASS）时生成一个**令牌 hash**，主 agent 把它带到收尾 gate；gate 拿 **per-org PASS 账本（`org_stage_completions`）当唯一真值**重算同一个 hash，**对上 + 全 org 新鲜 PASS** 才放行；对不上 / 缺 org → BLOCK，提示「只重跑缺口 org 的 stage_run」。**scoping 等非 specialist 阶段（主 agent 自交自过）走原 gate 不变**。

> **为什么 hash 而非「盖章」（回应用户担心 agent 直接盖章过）**：令牌不是 agent 能写的「通过章」——它是 stage_run **确定性代码**对 per-org 账本算出的摘要；gate **不信 agent 报的值本身**，而是自己从账本**重算**再比对。账本里每行（Phase 1 后）只有真过 gate 的 org → agent 造不出能对上的 hash。比「盖章」多一道「DB 真值重算」；比「approach A（gate 直接读账本）」多一道「逼 agent 必须真调过 stage_run 才拿得到能对上的令牌」。

### 关键事实（实读确认）

- 收尾 gate 在 `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` 的 gate hook：`parse_deliverable_from_content` → 建 `GateContext` → `harness.validate_gate_with_context(&deliverable, None, &gate_ctx)` → `decision` → 渲染 + `HarnessGateOutcome`（L1950-2094）。org/资产上下文取自 `self.harness_org_id`（chat 整阶段收尾 = `None` → 整库资产，正是要绕开的爆炸源 L1336/L2008）。
- specialist 判定：`load_embedded_stage_spec(stage).specialist.is_some()`（stage_run 同款判定见 `stage_run_call.rs:265`）。
- per-org PASS 真值已落 `org_stage_completions(organization_id, stage_kind, passed_at, stage_run_id, UNIQUE(org,stage))`（migration `20260615000001`；repo `golish_db::repo::org_stage_completions::{upsert,get}`；tracking trait `record/recent_org_stage_completion`；app 层 `tracking_bridge` 覆写）。**Phase 1 后该表只在真 gate PASS 时写** → 它就是阶段过门的真值源。
- **缺口 1**：`DbRepoProvider`（`db_traits/repo.rs`）没有「列出本 engagement 在-scope org 列表」的方法（grep 0）。要核「全 org 都过」必须新增 org 列表查询。
- **缺口 2**：收尾 gate 用 `self.repo: &dyn DbRepoProvider`，而 `recent_org_stage_completion` 在 **tracking** trait（`db_traits/tracking.rs`）上，gate 取不到。要让 gate 读账本，需把「批量读 completions」也加到 `DbRepoProvider`。
- 令牌载体：`StageDeliverable`（`harness/types.rs:229`）字段固定（`claims/evidence_refs/findings/coverage…`）。为**零 ts-rs 改动（I5）**，令牌走一条 **claim**：`{kind:"stage_run_pass_token", subject:<stage_id>, summary:<token>}`，不动类型。
- `GateResult.{gate_result_id,blocking_reason_id}`（`gate/mod.rs:54`）是 Observability 预留位（Phase 1=None），本期不依赖。

### 设计抉择（这一点请你拍板 → 决定 Task 7 写法）

令牌「对上」有两种实现，安全性差很多：

| | B-store（你原话「存 DB + 对上」） | **B-recompute（推荐）** |
|---|---|---|
| gate 真值源 | `stage_pass_tokens` 表里存的 hash | 当场从 `org_stage_completions` 重算 |
| 比什么 | agent 报的 == 表里存的 | 「全 org 在 TTL 内 PASS」+「重算==agent 带的」 |
| 弱点 | agent 的 hash 本就来自同一次 stage_run，必然相等；org 状态后来变了也不重查 → 只验「忠实带回」，不验「现在是否还全过」 | 无（对上 = agent 见过的账本态 == 当前账本态 == 全过）|
| 改动 | 多一张表 + migration | 无新表（可另存令牌仅供审计，不当真值）|

**推荐 B-recompute**：既满足你「runstage 过了生成 hash、对上才过、对不上重问」，又把真值钉死在 per-org 账本上，避免「盖章」与「存了就信」的盲点。下面 Task 按 B-recompute 写；若你坚持 B-store，Task 7 改成「读 `stage_pass_tokens` 表比对」即可（更短，但请知悉上表弱点）。

### 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `golish-agent-kit/src/harness/org_gate.rs` | 追加纯函数 `stage_pass_token(session,&stage,&[(Uuid,DateTime<Utc>)]) -> String`（规范化排序 + sha256 hex）+ `extract_pass_token(&StageDeliverable)->Option<String>` + 单测 | 改 |
| `golish-agent-kit/src/db_traits/repo.rs` | 新增 `in_scope_org_ids(operation_id:Option<Uuid>)->Vec<Uuid>` + `org_stage_completions_get(stage:&str,&[Uuid])->Vec<(Uuid,DateTime<Utc>)>`（均 default 空） | 改 |
| `golish-agent-app/src/ai/tracking_bridge/mod.rs` | 覆写上面两个方法：`organizations` 查在-scope org（**SQL 待按 organizations schema 核**）、`org_stage_completions` 批量取行 | 改 |
| `golish-agent-runtime/.../stage_run_call.rs` | 全 in-scope org 都 fresh PASS 时，**回读账本**算 token 放进返回 JSON `pass_token`，并在 summary 提示主 agent 收尾带上 | 改 |
| `golish-agent-kit/.../subtask_phases/execute.rs` | gate hook：specialist 阶段早分支——抽 token + 重算 + 校验，PASS/BLOCK，**跳过**整阶段 coverage | 改 |

> sha256 依赖：evidence_ledger 已用「OpenFang-style hash chain」，`sha2` 大概率在树内。Task 5 步骤 5.0 先核 `golish-agent-kit` 是否已依赖 `sha2`，缺则 `cargo add sha2 -p golish-agent-kit`。

---

### Task 5 — token 纯函数 + 账本读 / org 列表 repo 方法

**步骤 5.0** 核 sha2 依赖：`rg "^sha2" backend/crates/golish-agent-kit/Cargo.toml`，无则加。

**步骤 5.1** `org_gate.rs` 追加（纯函数，可单测；放在 Phase 1 纯函数旁）：

```rust
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::types::StageDeliverable;

/// 令牌 claim 的保留 kind（主 agent 收尾时带这条 claim 承载 stage_run 的 pass_token）。
pub const STAGE_RUN_PASS_TOKEN_KIND: &str = "stage_run_pass_token";

/// 对 (session, stage, 全 org 的 PASS 行) 算确定性摘要。**规范化**：org 行按
/// org_id 升序、用 stage_run 写库后**回读**的 passed_at（RFC3339）入摘要，保证
/// stage_run 端与 gate 端对同一账本态算出同一串。空行集 → 空串（调用方按「无 PASS」处理）。
pub fn stage_pass_token(
    session_id: &str,
    stage: StageKind,
    rows: &[(Uuid, DateTime<Utc>)],
) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&(Uuid, DateTime<Utc>)> = rows.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    h.update(session_id.as_bytes());
    h.update(b"|");
    h.update(stage.as_str().as_bytes());
    for (org, at) in sorted {
        h.update(b"|");
        h.update(org.as_bytes());
        h.update(b"@");
        h.update(at.to_rfc3339().as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// 从 deliverable 抽主 agent 带回的 stage_run pass_token（保留 claim 的 summary）。
pub fn extract_pass_token(deliverable: &StageDeliverable) -> Option<String> {
    deliverable
        .claims
        .iter()
        .find(|c| c.kind == STAGE_RUN_PASS_TOKEN_KIND)
        .map(|c| c.summary.trim().to_string())
        .filter(|s| !s.is_empty())
}
```

**步骤 5.2** `org_gate.rs` 单测追加：

```rust
#[test]
fn token_is_order_independent_and_binds_inputs() {
    let now = Utc::now();
    let a = Uuid::from_u128(1);
    let b = Uuid::from_u128(2);
    let t1 = stage_pass_token("s1", StageKind::TargetIntel, &[(a, now), (b, now)]);
    let t2 = stage_pass_token("s1", StageKind::TargetIntel, &[(b, now), (a, now)]);
    assert_eq!(t1, t2, "org 顺序不影响令牌");
    assert_ne!(t1, stage_pass_token("s2", StageKind::TargetIntel, &[(a, now), (b, now)]), "session 变 → 令牌变");
    assert_ne!(t1, stage_pass_token("s1", StageKind::Enumeration, &[(a, now), (b, now)]), "stage 变 → 令牌变");
    assert_ne!(t1, stage_pass_token("s1", StageKind::TargetIntel, &[(a, now)]), "少一个 org → 令牌变");
    assert!(stage_pass_token("s1", StageKind::TargetIntel, &[]).is_empty());
}

#[test]
fn extract_pass_token_reads_reserved_claim() {
    use super::types::{StageClaim, StageDeliverable};
    let d = StageDeliverable {
        stage_id: "target_intel".into(),
        stage_run_id: Uuid::new_v4(),
        claims: vec![StageClaim {
            kind: STAGE_RUN_PASS_TOKEN_KIND.into(),
            subject: "target_intel".into(),
            summary: "deadbeef".into(),
            evidence_ids: vec![],
            technique: None,
        }],
        evidence_refs: vec![],
        skipped_checks: vec![],
        findings: vec![],
        required_checks_done: vec![],
        coverage: vec![],
    };
    assert_eq!(extract_pass_token(&d).as_deref(), Some("deadbeef"));
}
```

**步骤 5.3** `db_traits/repo.rs` 加两个 default-空方法（test double 零改动）：

```rust
/// 本 engagement 在-scope 的 organization id 列表（主 agent scoping 建的 org 树）。
/// 阶段过门用它核「全 org 都过」。默认空 ⇒ 阶段过门退回「无法核 → 不放行」（fail-closed）。
async fn in_scope_org_ids(&self, operation_id: Option<Uuid>) -> anyhow::Result<Vec<Uuid>> {
    let _ = operation_id;
    Ok(Vec::new())
}

/// 批量取 `org_stage_completions` 行 `(organization_id, passed_at)`（gate 走 repo
/// 通道，拿不到 tracking trait 的 recent_org_stage_completion）。默认空。
async fn org_stage_completions_get(
    &self,
    stage_kind: &str,
    org_ids: &[Uuid],
) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> {
    let _ = (stage_kind, org_ids);
    Ok(Vec::new())
}
```

**步骤 5.4** `tracking_bridge/mod.rs` 覆写两方法。`org_stage_completions_get` 复用 `golish_db::repo::org_stage_completions::get` 逐 id 取（或加一个批量 SELECT，**SQL 按 `org_stage_completions.rs` 既有写法核**）。`in_scope_org_ids` 查 `organizations` 取本 operation 在-scope org——**动手前先读 `golish-db/src/repo/organizations.rs` 核：operation/engagement 与 organization 的关联列 + 在-scope 过滤口径**（避免编造 SQL）。

```rust
async fn org_stage_completions_get(
    &self,
    stage_kind: &str,
    org_ids: &[Uuid],
) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> {
    let mut out = Vec::new();
    for &org in org_ids {
        if let Ok(Some(row)) =
            golish_db::repo::org_stage_completions::get(self.pool.as_ref(), org, stage_kind).await
        {
            out.push((row.organization_id, row.passed_at));
        }
    }
    Ok(out)
}
// in_scope_org_ids：待核 organizations schema 后实现（select 本 operation 在-scope org id）。
```

**步骤 5.5** 编译 + 单测：

```bash
cd backend && cargo nextest run -p golish-agent-kit org_gate && cargo check -p golish-agent-app
```
预期：5.2 两个新测 + Phase 1 四个纯函数测 passed；app 层编译过。

**步骤 5.6** Commit：`feat(harness): stage_pass_token + org-completion/org-list repo reads`。

---

### Task 6 — stage_run 全过时回读账本算 token、随返回带回主 agent

**文件：** `stage_run_call.rs`

**步骤 6.1** 在 `execute_stage_run` 末尾「聚合」处（L438 `let passed = gaps.is_empty();` 之后、构造返回 JSON 之前）：当 `passed` 时，取**全 in-scope org**（不只本次 `units`，兼容 D11 只重跑缺口 org 的场景）的 fresh PASS 行回读，算 token。

```rust
// 阶段过门令牌（Phase 1.5）：仅当本阶段全 in-scope org 都已 fresh PASS 时生成。
// 用「全 in-scope org」而非本次 units——gap 重跑只传缺口 org，但阶段过门要看
// 累积账本是否齐（D11）。token 由账本回读值算（与收尾 gate 同源）。
let pass_token: Option<String> = if passed {
    match ctx.events.db_tracker.and_then(|t| t.repo()) {
        Some(repo) => {
            let op_id = ctx.events.operation_id; // Option<Uuid>，见下「待核」
            let org_ids = repo.in_scope_org_ids(op_id).await.unwrap_or_default();
            if org_ids.is_empty() {
                None // 无法核全集 → 不发令牌（收尾 gate 会 fail-closed 引导）
            } else {
                let mut rows = repo
                    .org_stage_completions_get(stage.as_str(), &org_ids)
                    .await
                    .unwrap_or_default();
                // 必须「全 org 都有 fresh 行」才算阶段过——少一个就不发令牌。
                let fresh: Vec<_> = rows
                    .drain(..)
                    .filter(|(_, at)| completion_is_fresh(*at, chrono::Utc::now(), STAGE_COMPLETION_TTL_SECS))
                    .collect();
                let have: std::collections::HashSet<uuid::Uuid> = fresh.iter().map(|(o, _)| *o).collect();
                if org_ids.iter().all(|o| have.contains(o)) {
                    let sid = ctx.events.session_id.unwrap_or("");
                    Some(golish_agent_kit::harness::org_gate::stage_pass_token(sid, stage, &fresh))
                } else {
                    None
                }
            }
        }
        None => None,
    }
} else {
    None
};
```

> **待核**：`ctx.events.operation_id` 的真实存在与类型——若 `AgenticLoopContext.events` 无 `operation_id`，用 `session_id`/task id 推导（`in_scope_org_ids` 的入参与 `organizations` 关联口径在 Task 5.4 一并核定，两处必须用同一 engagement 维度）。

**步骤 6.2** 把 token 放进返回 JSON，并在 `summary`（passed 分支）追加一句让主 agent 收尾带上：

```rust
"pass_token": pass_token,
```
summary passed 分支补：`format!("{} — 收尾请提交一条 claim {{kind:\"stage_run_pass_token\", subject:\"{}\", summary:<pass_token>}} 以过阶段门。", base, stage.as_str())`（仅当 `pass_token.is_some()`）。

**步骤 6.3** 编译 + stage_run 既有测：

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_run && cargo clippy -p golish-agent-runtime -- -D warnings
```
预期：解析类测全 passed（不碰 DB）；clippy 0。

**步骤 6.4** Commit：`feat(stage_run): emit stage pass_token when all in-scope orgs passed`。

---

### Task 7 — 收尾 gate：specialist 阶段改判 token（跳过整阶段 coverage）

**文件：** `subtask_phases/execute.rs`

**步骤 7.1** 在 gate hook 解析出 `deliverable` 后（L1972 之后）、建 `gate_ctx`/`validate` 之前，加 specialist 早分支：

```rust
// Phase 1.5：specialist（fan-out）阶段不跑整阶段 coverage，改判 stage_run 令牌。
let is_fanout = crate::harness::load_embedded_stage_spec(stage_hint.stage_kind)
    .map(|s| s.specialist.is_some())
    .unwrap_or(false);
if is_fanout {
    return verify_stage_run_pass_token(self, &content, &deliverable, stage_hint.stage_kind, confirm_only).await;
}
```

**步骤 7.2** 新增 async 自由函数 / 方法 `verify_stage_run_pass_token`（B-recompute）：

```rust
async fn verify_stage_run_pass_token(
    this: &Self,                      // 或拆 self.repo / self.harness_org_id 入参
    content: &str,
    deliverable: &crate::harness::StageDeliverable,
    stage: crate::harness::StageKind,
    confirm_only: bool,
) -> (String, Option<HarnessGateOutcome>) {
    use crate::harness::org_gate::{extract_pass_token, stage_pass_token};
    let sid = /* 同 fetch_evidence_facts_for_gate 用的 chat session id（待核 accessor）*/;
    let op_id = /* 同 Task 6 的 engagement 维度（待核）*/;

    let org_ids = this.repo.in_scope_org_ids(op_id).await.unwrap_or_default();
    // fail-closed：核不到 org 全集就不放行（specialist 阶段必须能核全过）。
    if org_ids.is_empty() {
        return block_outcome(content, stage, confirm_only,
            vec!["cannot verify stage completion: no in-scope organizations resolved — run scoping first".into()]);
    }
    let rows = this.repo.org_stage_completions_get(stage.as_str(), &org_ids).await.unwrap_or_default();
    let fresh: Vec<_> = rows.into_iter()
        .filter(|(_, at)| /* completion_is_fresh 同款 TTL（把 stage_run 的 TTL 提到共享常量复用）*/ true)
        .collect();
    let have: std::collections::HashSet<uuid::Uuid> = fresh.iter().map(|(o, _)| *o).collect();
    let missing: Vec<uuid::Uuid> = org_ids.iter().copied().filter(|o| !have.contains(o)).collect();
    if !missing.is_empty() {
        return block_outcome(content, stage, confirm_only, vec![format!(
            "stage not complete: {} of {} orgs have not passed this stage's gate — re-run stage_run with orgs={:?}",
            missing.len(), org_ids.len(), missing)]);
    }
    let expected = stage_pass_token(sid, stage, &fresh);
    match extract_pass_token(deliverable) {
        Some(tok) if tok == expected => pass_outcome(content, stage, confirm_only, deliverable),
        Some(_) => block_outcome(content, stage, confirm_only,
            vec!["stage_run pass_token mismatch (stale or wrong stage) — re-run stage_run and submit the new pass_token".into()]),
        None => block_outcome(content, stage, confirm_only,
            vec!["missing stage_run pass_token claim — run stage_run for this stage, then submit its pass_token".into()]),
    }
}
```

> `pass_outcome`/`block_outcome` 是本文件内两个小构造器，复刻现有 L2060-2093 `HarnessGateOutcome` 形状（`gated_stage=stage`、`gate_allowed`、`evidence_summary=summarize_deliverable`、`gate_reasons`、`confirm_only_stage=confirm_only`，其余 `..Default`/空）。把现有 PASS/BLOCK 渲染（`## Harness Gate Decision` JSON 追加）抽成一个 helper 两边共用，避免重复。

**步骤 7.3** 把 stage_run 的 `STAGE_COMPLETION_TTL_SECS` + `completion_is_fresh` 提升为 `golish-agent-kit` 内共享（如 `harness::org_gate` 里 `pub const STAGE_COMPLETION_TTL_SECS` + `pub fn completion_is_fresh`），stage_run 改 `use` 它，gate 也 `use` 它——**两端 TTL/新鲜度判定必须同源**，否则 stage_run 发了令牌、gate 因 TTL 略差判过期，造成假 BLOCK。

**步骤 7.4** 编译 + execute 既有测 + gate 测：

```bash
cd backend && cargo nextest run -p golish-agent-kit -- execute gate && cargo clippy -p golish-agent-kit -- -D warnings
```
预期：现有 `parse_deliverable_*` / gate 测全 passed；clippy 0。新增 specialist 分支建议补 1 个集成测（mock repo：全 org PASS + 正确 token → allowed；缺 org / token 错 → block）。

**步骤 7.5** Commit：`feat(harness): fan-out stage closes on stage_run pass_token, not whole-stage coverage`。

---

### Task 8 — 收尾门禁

```bash
just precommit
```
全绿后把命令 + 关键输出贴进 `agent-progress.md`「已记录证据」。

---

### Phase 1.5 自检

- **规格覆盖**：「跑完 stage_run 即过」→ Task 7 specialist 分支只判令牌、跳过整阶段 coverage。「对上才过」→ `tok == expected`（B-recompute）。「对不上 / 缺 org → 重问」→ block_outcome 的两条 reason 明确指「re-run stage_run」。「hash 存 DB」→ 真值即 `org_stage_completions`（令牌由它重算）；如需另存审计令牌，B-store 变体加 `stage_pass_tokens` 表。
- **不变量**：I2/IDOR——`in_scope_org_ids` 按 engagement 维度取（Task 5.4 核关联列）；I7——令牌仍出自主 agent 提交的 claim（deliverable 不变量保留）；I8——只认账本 PASS 行，无行 = 未过（绝不当过）；I10——B-recompute 无 schema 改动（只读 `org_stage_completions` + `organizations`）。
- **待核点（动手即核，勿编造）**：① `organizations` 与 operation/engagement 关联列 + 在-scope 过滤（Task 5.4）；② chat session id / operation_id 在 stage_run（`ctx.events.*`）与 gate hook（`self`/`exec_ctx`）两侧的真实 accessor，**两侧必须同维度**否则令牌对不上；③ `sha2` 是否已在 `golish-agent-kit` 依赖（Task 5.0）。

---

## Phase 2 · 闸 1（可选、单列）：逼 stage 专家「过关才能收工」

> 目的：让 per-org 子 agent 不能靠 `submit_result` 随便收工——把「过 gate」变成它结束的前置。Phase 1 已能堵住「没真过也算过」（确定性裁判），Phase 2 是省 token / 减空转的体验优化。**blast radius 大（动 sub-agent 执行器），与 Phase 1 分开 commit、分开评审。**

**思路（择一，brainstorming 后再定）：**
- 方案 A（推荐）：stage 专家（`recon` 等）**移除** `submit_result` barrier，唯一收工方式是 `submit_stage_deliverable`；并让该工具在 gate PASS 时才置「完成」信号、`needs_fix/BLOCK` 时把 reasons 喂回继续循环。需改 `golish-sub-agents/src/executor/{inner.rs,response_parsing.rs,tool_setup.rs}` 的 barrier 语义 + recon 定义（`defaults/builder/registry.rs`）。
- 方案 B（更轻）：保留 `submit_result`，但在 `inner.rs` 收尾处——若处于 harness stage 且本 org 没有「被接受的交付」——把 `SubAgentResult.success` 置 `false`。Phase 1 已不看 `success`，故 B 仅影响 UI/日志语义，价值有限；优先 A。

**风险：** 改 barrier 语义影响所有 stage 专家；需补 sub-agent 执行器测试（barrier 移除后 max_iterations 收尾路径、needs_fix 回灌不死循环）。先 `writing-plans` 单独出 Phase 2 计划再动。

---

## 自检

**1. 规格覆盖度：**
- 「org 通过=真过 gate」→ Task 2（评估器）+ Task 3（接线）。
- 「BLOCK 不写账本、可重跑」→ Task 3 `OrgVerdict::Block` 分支不调 `record_org_stage_completion`。
- 「per-org 交付不串槽」→ Task 3 取完即清 sink。
- 「DB 真值按 org 注入」→ Task 2 `in_scope_assets(org_id)`/`db_truth_facts(org_id,…)`/`evidence_facts_for_session`。
- 「fan-out 阶段跑完 stage_run 即过（hash 令牌、对不上重跑）」→ Phase 1.5（Task 5-7）。
- 「闸 1」→ Phase 2（单列）。

**2. 占位符扫描：** 无 TODO/待定；每步有代码或精确命令。唯一「按真实方法名改」处：`evidence_refs` 元素的取 i64 方法（步骤 2.3 已标注核对点 `harness/types.rs`）。

**3. 类型一致性：** `OrgVerdict`（Task 1 定义）在 Task 3 match 使用一致；`evaluate_org_stage_gate` 返回 `GateResult`，经 `decide_org_verdict` → `OrgVerdict`；`GateContext`/`EvidenceFact` 字段名与 `rule_engine.rs:194/210` 一致；repo 方法签名与 `db_traits/repo.rs` 一致。

**4. 不变量（AGENTS.md §5）：** I2/IDOR——gate 按 org 隔离（`org_id` 透传）；I7/I8——只认账本/DB 真值 Found，Empty 绝不当 Found（`facts_from_rows`）；I10——无 schema 改动（仅读 `org_stage_completions`，写入时机改为 PASS 后）。
