# stage_run Phase 2（闸 1·A-lite）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现；改 Rust 后跑 `cargo nextest -p golish-agent-runtime stage_run` + `cargo clippy -p golish-agent-runtime -- -D warnings`，收尾 `cargo check --workspace`。每个 Task 单独可验证。

**目标：** 让 chat `stage_run` 的 per-org 扇出在某 org 没过权威 gate 时，**带着 gate 的 BLOCK 理由把同一个 specialist 重跑（有上限）**，过了才算通过/写账本；上限内仍不过才进 `gaps`。即「专家过关才能收工」的低风险实现（A-lite）。

**架构：** 复用已提交的 Phase 1 per-org gate（`evaluate_org_stage_gate` + `decide_org_verdict`，commit `15f88c3a`）。**只改 1 个文件** `stage_run_call.rs`：把现有「dispatch → gate → verdict → Pass/Block」单次流程包进一个**有界重试循环**。重试决策抽成纯函数 `next_org_action`（TDD 单测），异步编排薄薄一层。无 DB（pure-eval/headless）路径用 `max_attempts=1` 退化为「不重试」，保持回归/eval 确定性。

**技术栈：** Rust 2021（golish-agent-runtime），`cargo nextest`，`serde_json`。

**不做（明确排除，YAGNI）：** 不动 `golish-sub-agents` 执行器 / barrier / `submit_result` / `submit_stage_deliverable` 工具（那是 A-full，blast radius 大）。不用 sub-agent `resume`（fresh 重跑 + DB 累积证据已足够；resume 作为未来优化）。

---

## 关键事实（实读确认）

- 扇出循环：`stage_run_call.rs` `execute_stage_run` 的 `for unit in &units`（L313-477）。每个 org：resume-skip（L325，已通过则跳过，保留）→ emit running（L346）→ `build_org_objective`（L359）→ `execute_sub_agent_call`（L366）→ 取 sink 交付 + 清 sink（L380-392）→ `verdict`（L398-417：有 repo+交付走 `evaluate_org_stage_gate`+`decide_org_verdict`；否则回退 `sub_ok`）→ `match verdict { Pass→passed_count++/record_org_stage_completion/emit passed；Block{reasons}→detail/emit blocked/gaps.push }`（L419-476）。
- `OrgVerdict`（`golish-agent-kit::harness::org_gate`）：`Pass` | `Block { reasons: Vec<String> }`，已 import（L31 区）。
- `build_org_objective(stage, unit, &spec.expected_techniques, &spec.allowed_tool_types) -> String`（同文件纯函数，已有单测）。
- `emit_org_progress(ctx, stage, unit, &org_request_id, status, note: Option<String>, _n, &stage_label, &role_label, &coverage_axis)`：status 用过 `queued`/`running`/`passed`/`blocked`。
- 重试天然收敛：gate 读 DB 真值（累积），attempt-1 收集的证据仍在账本，attempt-2 只需补 BLOCK 点 → fresh 重跑不浪费已得证据。
- 测试模块：文件尾 `#[cfg(test)] mod tests`（已有 `parse_org_units_*`/`build_org_objective_*`/`completion_freshness_*` 等纯函数测）。

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs` | 新增纯函数 `next_org_action` + `gate_retry_feedback` + `OrgAttemptOutcome` + 单测；把 per-org 单次流程改成有界重试循环 | 改 |

---

## Task 1 — 纯重试决策函数 + 单测（TDD 先红）

**步骤 1.1** 在 `stage_run_call.rs`（`build_org_objective` 附近、`execute_stage_run` 之外）加常量 + 类型 + **故意错误的 stub**（先红）：

```rust
/// Phase 2 闸1·A-lite：每个 org 过权威 gate 的最大尝试次数（1 次初投 + 2 次带反馈重跑）。
/// 超过仍 BLOCK 才记 gap（交回主 agent 的 gap-closure 循环）。
const MAX_ORG_GATE_ATTEMPTS: usize = 3;

/// 一个 org 某次尝试拿到 gate 裁决后的下一步动作（纯控制流，单测覆盖）。
#[derive(Debug, PartialEq, Eq)]
enum OrgAttemptOutcome {
    /// gate 通过 → 记通过 + 写账本。
    Passed,
    /// gate BLOCK 且还有尝试次数 → 带 `feedback` 重投 specialist。
    Retry { feedback: String },
    /// gate BLOCK 且尝试用尽 → 记 gap（`reasons` 供汇报）。
    Exhausted { reasons: Vec<String> },
}

/// 纯函数：给定一次尝试的 `verdict`、1-based `attempt`、`max_attempts`，决定下一步。
/// `max_attempts==1`（无 DB gate 的回退路径）下 BLOCK 直接 Exhausted（不重试）。
fn next_org_action(
    _verdict: &OrgVerdict,
    _attempt: usize,
    _max_attempts: usize,
) -> OrgAttemptOutcome {
    OrgAttemptOutcome::Passed // STUB（故意错，待 Task 2 实现）
}

/// 重试时追加到 specialist objective 末尾的反馈块，点名 gate 的 BLOCK 理由。
/// `attempt` 是即将发起的（1-based）下一次尝试号。
fn gate_retry_feedback(attempt: usize, max_attempts: usize, reasons: &[String]) -> String {
    let reasons_block = if reasons.is_empty() {
        "the per-org stage gate did not pass (no specific reasons returned)".to_string()
    } else {
        reasons
            .iter()
            .map(|r| format!("- {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "## GATE FEEDBACK — retry {attempt}/{max_attempts}\n\n\
         Your previous deliverable for THIS organization did NOT pass the per-org \
         stage gate. The evidence you already collected is saved in the ledger — do \
         NOT redo it; focus only on closing these specific gaps, then submit the \
         StageDeliverable again:\n\n{reasons_block}"
    )
}
```

**步骤 1.2** 在 `#[cfg(test)] mod tests` 内追加单测：

```rust
#[test]
fn next_org_action_pass_is_passed() {
    assert_eq!(next_org_action(&OrgVerdict::Pass, 1, 3), OrgAttemptOutcome::Passed);
    assert_eq!(next_org_action(&OrgVerdict::Pass, 3, 3), OrgAttemptOutcome::Passed);
}

#[test]
fn next_org_action_block_with_attempts_left_retries_with_named_reasons() {
    let v = OrgVerdict::Block { reasons: vec!["missing GOLISH-INTEL-DNS on a.com".to_string()] };
    match next_org_action(&v, 1, 3) {
        OrgAttemptOutcome::Retry { feedback } => {
            assert!(feedback.contains("missing GOLISH-INTEL-DNS on a.com"), "names the gap: {feedback}");
            assert!(feedback.contains("retry 2/3"), "names attempt: {feedback}");
        }
        other => panic!("expected Retry, got {other:?}"),
    }
}

#[test]
fn next_org_action_block_on_last_attempt_is_exhausted() {
    let v = OrgVerdict::Block { reasons: vec!["coverage incomplete".to_string()] };
    assert_eq!(
        next_org_action(&v, 3, 3),
        OrgAttemptOutcome::Exhausted { reasons: vec!["coverage incomplete".to_string()] }
    );
}

#[test]
fn next_org_action_no_db_fallback_does_not_retry() {
    // max_attempts==1 (no-repo fallback path): a BLOCK is terminal, never retried.
    let v = OrgVerdict::Block { reasons: vec!["sub-agent did not complete".to_string()] };
    assert_eq!(
        next_org_action(&v, 1, 1),
        OrgAttemptOutcome::Exhausted { reasons: vec!["sub-agent did not complete".to_string()] }
    );
}
```

**步骤 1.3** 跑测确认 **红**（stub 恒返回 Passed → 后 3 个测失败）：

```bash
cd backend && cargo nextest run -p golish-agent-runtime next_org_action
```
预期：`next_org_action_pass_is_passed` 过；其余 3 个 **失败**（got Passed, expected Retry/Exhausted）。

---

## Task 2 — 实现 `next_org_action`（绿）

**步骤 2.1** 用真实逻辑替换 stub：

```rust
fn next_org_action(
    verdict: &OrgVerdict,
    attempt: usize,
    max_attempts: usize,
) -> OrgAttemptOutcome {
    match verdict {
        OrgVerdict::Pass => OrgAttemptOutcome::Passed,
        OrgVerdict::Block { reasons } => {
            if attempt < max_attempts {
                OrgAttemptOutcome::Retry {
                    feedback: gate_retry_feedback(attempt + 1, max_attempts, reasons),
                }
            } else {
                OrgAttemptOutcome::Exhausted { reasons: reasons.clone() }
            }
        }
    }
}
```

**步骤 2.2** 跑测确认 **绿**：

```bash
cd backend && cargo nextest run -p golish-agent-runtime next_org_action
```
预期：4 passed。

**步骤 2.3** Commit：`feat(stage_run): pure per-org gate retry decision (next_org_action)`。

---

## Task 3 — 接线：per-org 有界重试循环

**步骤 3.1** 把 `stage_run_call.rs` L346-476（emit running → … → `match verdict {…}`）整块替换为重试循环。新逻辑：

```rust
        // Phase 2 闸1·A-lite：把这个 org 的「投递→过 gate」放进有界重试循环。
        // 没过就带 gate 的 BLOCK 理由把同一个 specialist 重投，过了才算；上限内仍
        // 不过才记 gap。无 DB gate 的回退路径用 max_attempts=1 → 不重试（保 eval 确定性）。
        let mut attempt = 0usize;
        let mut feedback: Option<String> = None;
        loop {
            attempt += 1;
            emit_org_progress(
                ctx,
                stage,
                unit,
                &org_request_id,
                "running",
                Some(if attempt == 1 {
                    format!("dispatching {role_label}")
                } else {
                    format!("retry {attempt}/{MAX_ORG_GATE_ATTEMPTS}: closing gate gaps")
                }),
                0,
                &stage_label,
                &role_label,
                &coverage_axis,
            );

            let objective = {
                let base = build_org_objective(
                    stage,
                    unit,
                    &spec.expected_techniques,
                    &spec.allowed_tool_types,
                );
                match &feedback {
                    Some(fb) => format!("{base}\n\n{fb}"),
                    None => base,
                }
            };
            let sub_args = json!({ "task": objective });
            let result = execute_sub_agent_call(
                &sub_agent_tool,
                &sub_args,
                ctx,
                model,
                context,
                &org_request_id,
            )
            .await;

            let sub_ok = matches!(&result, Ok(r) if r.success);

            // Take THIS org's own deliverable (serial: sink holds this org's last submit).
            let org_deliverable: Option<StageDeliverable> =
                match ctx.harness_deliverable_sink.as_ref() {
                    Some(sink) => sink
                        .read()
                        .await
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<StageDeliverable>(s).ok()),
                    None => None,
                };
            if let Some(sink) = ctx.harness_deliverable_sink.as_ref() {
                *sink.write().await = None;
            }

            // Authoritative verdict + whether it came from the real DB gate.
            let (verdict, from_gate) = match (
                ctx.events.db_tracker.and_then(|t| t.repo()),
                org_deliverable.as_ref(),
            ) {
                (Some(repo), Some(deliv)) => {
                    let org_uuid = uuid::Uuid::parse_str(&unit.id).ok();
                    let session = ctx.events.session_id.unwrap_or("");
                    let gate =
                        evaluate_org_stage_gate(repo, org_uuid, session, stage, deliv).await;
                    (decide_org_verdict(&gate), true)
                }
                _ => {
                    let v = if sub_ok {
                        OrgVerdict::Pass
                    } else {
                        OrgVerdict::Block {
                            reasons: vec!["sub-agent did not complete".to_string()],
                        }
                    };
                    (v, false)
                }
            };

            // No-DB fallback never retries (max_attempts=1); real gate gets up to MAX.
            let max_attempts = if from_gate { MAX_ORG_GATE_ATTEMPTS } else { 1 };
            match next_org_action(&verdict, attempt, max_attempts) {
                OrgAttemptOutcome::Passed => {
                    passed_count += 1;
                    if let (Some(tracker), Ok(org_id)) =
                        (ctx.events.db_tracker, uuid::Uuid::parse_str(&unit.id))
                    {
                        tracker
                            .record_org_stage_completion(
                                org_id,
                                stage.as_str(),
                                Some(&org_request_id),
                            )
                            .await;
                    }
                    emit_org_progress(
                        ctx, stage, unit, &org_request_id, "passed", None, 0, &stage_label,
                        &role_label, &coverage_axis,
                    );
                    break;
                }
                OrgAttemptOutcome::Retry { feedback: fb } => {
                    feedback = Some(fb);
                    continue;
                }
                OrgAttemptOutcome::Exhausted { reasons } => {
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
                        ctx, stage, unit, &org_request_id, "blocked", None, 0, &stage_label,
                        &role_label, &coverage_axis,
                    );
                    gaps.push(
                        json!({ "org_id": unit.id, "org_name": unit.name, "detail": detail }),
                    );
                    break;
                }
            }
        }
```

**步骤 3.2** 编译 + 既有 stage_run 测不回归 + clippy：

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_run && cargo clippy -p golish-agent-runtime -- -D warnings
```
预期：`parse_org_units_*`/`build_org_objective_*`/`completion_freshness_*`/`next_org_action_*`/`tool_definition_*` 全 passed；clippy exit 0。

**步骤 3.3** 收尾编译：`cargo check --workspace`（exit 0）。

**步骤 3.4** Commit：`feat(stage_run): re-run specialist with gate feedback until org passes (Phase 2 闸1)`。

---

## 自检

- **规格覆盖**：「专家过关才能收工」→ 重试循环只在 Passed/Exhausted 才 break；BLOCK 且有次数 → 带反馈重投。「对不上重问」→ feedback 点名 reasons。「不浪费已得证据」→ fresh 重跑 + gate 读累积 DB 真值。
- **不变量**：I7/I8——仍只认 gate 对账本/证据的 PASS（重试不改判定，只给更多机会）；I2——org_id 透传不变；无 schema 改动。
- **回归安全**：无 DB 路径 `max_attempts=1` → 行为同改前（Pass 或单 Block→gap），eval/headless 不受影响。
- **有界**：每 org ≤ `MAX_ORG_GATE_ATTEMPTS` 次；resume-skip 仍先跳过已通过 org。
- **占位符扫描**：无 TODO；`next_org_action`/`gate_retry_feedback`/`OrgAttemptOutcome` 均在 Task 1 定义、Task 3 一致使用。
