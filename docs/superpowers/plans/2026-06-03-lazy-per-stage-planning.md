# 懒生成逐阶段计划（Lazy per-stage planning）实现计划

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任务实现此计划，TDD（`.cursor/skills/test-driven-development`），每个 Task 单独 commit。

**目标：** Task 模式改为「进到某阶段才懒生成该阶段 plan，末步固定 submit+验证，过 gate 才进下一阶段」，取代「一次性产扁平全程 plan」。
**架构：** 给 `AgentExecutor` 加 `generate_stage_plan`（默认空实现）；bridge 用 LLM 按阶段产 2–4 步并强制 submit 末步；`run_stage_subtasks` 的空阶段分支改调它（失败回落单 synth）；`run()` 不再上游一次性产 plan。**测试阶段无 flag，行为无条件替换**（保留 harness 既有 `stage_mode`/`graph_flow`）。
**技术栈：** Rust（golish-agent-kit / golish-agent-bridge）、cargo nextest、cargo clippy。

> 配套设计：`docs/design/2026-06-03-lazy-per-stage-planning.md`。

---

## 文件结构（创建/修改 + 职责）

| 文件 | 职责 | 变更 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs` | `AgentExecutor` 契约 | 加 `generate_stage_plan`（默认 `Ok(vec![])`） |
| `backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs` | bridge 执行器实现 | 实现 `generate_stage_plan` + 私有 `ensure_submit_terminal` + 单测 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | DAG 驱动 + 进阶段执行 | `run_stage_subtasks` 空分支改懒产 + emit `plan_updated` |
| `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs` | `run()` 主流程 | 跳过上游扁平 `generate_subtasks` |

---

## Task 1 · trait 加 `generate_stage_plan`（默认空实现）

**文件：** `golish-agent-kit/src/task_orchestrator/types.rs`

**步骤 1.1** 在 `trait AgentExecutor`（约 `:280`）的 `refine_plan` 之后加方法：

```rust
    /// Lazily plan ONE stage's tactical subtasks (A1 · lazy per-stage). The
    /// returned list MUST end with a "Submit & verify <stage>" subtask. Default
    /// returns empty so existing executors (test mocks) need no change; the
    /// orchestrator falls back to the single-subtask synthesizer on empty.
    async fn generate_stage_plan(
        &self,
        stage: crate::harness::StageKind,
        task_input: &str,
        upstream_evidence: &str,
    ) -> anyhow::Result<Vec<PlannedSubtask>> {
        let _ = (stage, task_input, upstream_evidence);
        Ok(Vec::new())
    }
```

**验证：** `cd backend && cargo check -p golish-agent-kit` → exit 0（默认实现，无既有 impl 需改）。
**提交：** `feat(harness): add AgentExecutor::generate_stage_plan (default no-op) for lazy per-stage planning`

---

## Task 2 · bridge 实现 `generate_stage_plan` + 强制 submit 末步（TDD）

**文件：** `golish-agent-bridge/src/bridge_executor/trait_impl.rs`

**步骤 2.1（红）** 在文件末尾的 `#[cfg(test)] mod tests` 中（若无则新建）加测试，覆盖「末步非 submit → 自动补」与「已含 submit 末步 → 不重复补」：

```rust
#[cfg(test)]
mod stage_plan_tests {
    use super::ensure_submit_terminal;
    use golish_agent_kit::task_orchestrator::PlannedSubtask;

    fn pt(title: &str) -> PlannedSubtask {
        PlannedSubtask {
            title: title.into(),
            description: title.into(),
            agent: Some("pentester".into()),
            harness_stage: None,
            nl_slice: None,
            acceptance_criteria: vec![],
        }
    }

    #[test]
    fn appends_submit_when_missing() {
        let mut v = vec![pt("Enumerate subdomains")];
        ensure_submit_terminal(&mut v, "enumeration", "pentester");
        assert_eq!(v.len(), 2);
        assert!(v.last().unwrap().title.to_lowercase().contains("submit"));
        assert!(v.last().unwrap().description.contains("submit_stage_deliverable"));
    }

    #[test]
    fn keeps_existing_submit_terminal() {
        let mut v = vec![pt("recon"), pt("Submit & verify the enumeration StageDeliverable")];
        ensure_submit_terminal(&mut v, "enumeration", "pentester");
        assert_eq!(v.len(), 2, "must not double-append");
    }
}
```

**步骤 2.2** 在 `impl AgentExecutor for BridgeAgentExecutor` 块内（紧跟 `generate_subtasks` 之后）加实现：

```rust
    async fn generate_stage_plan(
        &self,
        stage: golish_agent_kit::harness::StageKind,
        task_input: &str,
        upstream_evidence: &str,
    ) -> Result<Vec<PlannedSubtask>> {
        let stage_id = stage.as_str();
        tracing::info!("[TaskMode/StagePlan] Lazily planning stage '{stage_id}'");
        let user = format!(
            "Target: {task_input}\n\nUpstream evidence so far:\n{upstream_evidence}\n\n\
             Plan ONLY the current stage: `{stage_id}`. Produce 2-4 concrete, executable \
             subtasks that advance THIS stage. The FINAL subtask MUST be \
             \"Submit & verify the {stage_id} StageDeliverable\" and instruct the agent to call \
             the submit_stage_deliverable tool. Do not plan other stages."
        );
        let response = self
            .simple_completion_for_phase(prompts::generator_prompt(), &user, Some("pipeline_generator"))
            .await
            .context("Stage-plan LLM call failed")?;
        let json_str = extract_json_from_response(&response);
        let mut output: GeneratorOutput = match serde_json::from_str(json_str) {
            Ok(o) => o,
            // Refusal / non-JSON → empty so the orchestrator falls back to the
            // single-subtask synthesizer (never spin, never vacuous pass).
            Err(_) => return Ok(Vec::new()),
        };
        if output.subtasks.is_empty() {
            return Ok(Vec::new());
        }
        // Force the stage tag onto every subtask + a deterministic submit terminal.
        let agent = if matches!(stage, golish_agent_kit::harness::StageKind::Reporting) {
            "analyzer"
        } else {
            "pentester"
        };
        for st in output.subtasks.iter_mut() {
            st.harness_stage = Some(golish_agent_kit::harness::HarnessStageHint::from(stage));
        }
        ensure_submit_terminal(&mut output.subtasks, stage_id, agent);
        Ok(output.subtasks)
    }
```

> 注：`HarnessStageHint::from(stage)` 的确切构造请对照 `execute.rs:1562 synthesize_stage_subtask` 里 `PlannedSubtask.harness_stage` 的赋值方式（同源），如该 `From` 不存在则用其使用的同款转换。

**步骤 2.3** 在 `trait_impl.rs` 顶层（impl 块外）加私有 helper：

```rust
/// Ensure a lazily-generated stage plan ends with a submit-and-verify step
/// (deterministic — the model may forget). No-op if the last step already
/// references `submit_stage_deliverable` / contains "submit".
fn ensure_submit_terminal(subtasks: &mut Vec<PlannedSubtask>, stage_id: &str, agent: &str) {
    let last_is_submit = subtasks.last().is_some_and(|s| {
        let t = s.title.to_lowercase();
        t.contains("submit") || s.description.contains("submit_stage_deliverable")
    });
    if last_is_submit {
        return;
    }
    subtasks.push(PlannedSubtask {
        title: format!("Submit & verify the {stage_id} StageDeliverable"),
        description: format!(
            "Compile this stage's findings and call the submit_stage_deliverable tool with the \
             structured StageDeliverable (stage_id, stage_run_id, claims, evidence_refs, findings) \
             for the '{stage_id}' stage. Cite only real evidence-ledger ids produced this run."
        ),
        agent: Some(agent.to_string()),
        harness_stage: None, // caller sets the stage tag on the whole list
        nl_slice: None,
        acceptance_criteria: vec![],
    });
}
```

> `PlannedSubtask` 完整字段见 `types.rs:21`：`title/description/agent/harness_stage/nl_slice/acceptance_criteria`。

**验证：**
- `cargo nextest -p golish-agent-bridge -E 'test(stage_plan_tests)'` → 2 passed。
- `cargo check -p golish-agent-bridge` → exit 0；`cargo clippy -p golish-agent-bridge -- -D warnings` → exit 0。
**提交：** `feat(harness): bridge generate_stage_plan with deterministic submit-terminal`

---

## Task 3 · `run_stage_subtasks` 空分支改懒产（execute.rs）

**文件：** `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**背景（现状已读）：** `run_stage_subtasks` 的空 `indices` 分支（约 `:596–637`）当前做 `let synth = synthesize_stage_subtask(stage, &exec_ctx.task_input);` 然后 `execute_single_subtask(&synth, …)` 跑这一个合成 subtask。

**步骤 3.1** 把该空分支从「合成 1 个」改为「懒产 N 个（失败回落 1 个 synth）」。将 `let synth = synthesize_stage_subtask(stage, &exec_ctx.task_input);`（及其后仅执行该单个 `synth` 的代码）替换为：

```rust
            // A1 · lazy per-stage planning: ask the executor for THIS stage's plan
            // (2-4 tactical steps ending in submit+verify). Empty / failure →
            // fall back to a single synthesized subtask so the stage still
            // executes + gets gated (never a vacuous pass — S1 fail-closed).
            let lazy = executor
                .generate_stage_plan(stage, &exec_ctx.task_input, &exec_ctx.upstream_evidence_for_stage())
                .await
                .unwrap_or_default();
            let stage_plan: Vec<PlannedSubtask> = if lazy.is_empty() {
                vec![synthesize_stage_subtask(stage, &exec_ctx.task_input)]
            } else {
                lazy
            };
            tracing::info!(
                target: "harness::hook",
                stage = %stage.as_str(),
                steps = stage_plan.len(),
                "lazy per-stage plan generated"
            );
            // Surface the current stage's plan to the UI (replaces the flat plan card).
            self.emit_stage_plan_update(task_id, &stage_plan);
            for pt in &stage_plan {
                exec_ctx.harness_stage = Some(stage);
                exec_ctx.harness_authz = op_max_authz.map(|max_authorization| {
                    let intent = crate::harness::IntentClassifier::with_default_keywords()
                        .classify(&pt.description, stage);
                    crate::harness::HarnessAuthz { max_authorization, intent }
                });
                exec_ctx.current_subtask = Some(super::super::types::CurrentSubtask {
                    title: pt.title.clone(),
                    description: pt.description.clone(),
                    agent: pt.agent.clone(),
                });
                let (result_text, _usage) = self
                    .execute_single_subtask(pt, exec_ctx, executor, &None, task_id)
                    .await;
                exec_ctx.completed_results.push(super::super::types::SubtaskResult {
                    title: pt.title.clone(),
                    result: result_text,
                    token_usage: None,
                });
            }
```

> 对照现状：上面 `exec_ctx.harness_stage/_authz/current_subtask` 设置与 `execute_single_subtask` 调用，**完全镜像**现有 synth 分支（`execute.rs:605–636`）的同款写法，只是从「单个 synth」改成「`for pt in &stage_plan`」。`upstream_evidence_for_stage()` 若 `ExecutionContext` 无此方法，用其已有的上游结果摘要（如 `completed_results` 拼接）替代——见 §开放问题。

**步骤 3.2** 加一个轻量 emit helper（若已有 `emit_plan_update` 可直接复用，跳过本步）。在 `impl` 内加：

```rust
    /// Emit a `plan_updated`-style view of the CURRENT stage's lazily-planned
    /// steps so the chat plan card shows the active stage's plan.
    fn emit_stage_plan_update(&self, task_id: uuid::Uuid, stage_plan: &[PlannedSubtask]) {
        // Reuse the existing plan event shape; map each PlannedSubtask.title to a step.
        // (If `emit_plan_update`/PlanStep already exists, delegate to it instead.)
        let _ = (task_id, stage_plan);
        // Implementation: build the plan-step list from titles + emit AiEvent::PlanUpdated
        // exactly like the existing `emit_plan_update` does (see its current call sites).
    }
```

> 实现 `emit_stage_plan_update` 时**复用现有 `emit_plan_update` / `AiEvent::PlanUpdated` 的真实构造**（本文件已多处调用 `self.emit_plan_update(queue, idx, StepStatus::…)`）；把每个 `PlannedSubtask.title` 映成一个 plan step。

**验证：**
- `cargo check -p golish-agent-kit` → exit 0。
- `cargo nextest -p golish-agent-kit -E 'test(task_orchestrator)'` → 全过（既有用例无回归；mock executor 默认 `generate_stage_plan` 返空 → 走 synth 回落，行为同旧）。
- `cargo clippy -p golish-agent-kit -- -D warnings` → exit 0。
**提交：** `feat(harness): lazily generate per-stage plan in run_stage_subtasks (fallback to synth)`

---

## Task 4 · `run()` 不再上游一次性产 plan（orchestrator.rs）

**文件：** `golish-agent-kit/src/task_orchestrator/orchestrator.rs`

**背景（现状已读）：** `run()` 约 `:156` 调 `executor.generate_subtasks(task_input)` 产扁平全程 plan。

**步骤 4.1** 让 graph-flow 路径起始 queue 为空（懒产驱动）。把 `:156` 的 `let generator_output = match executor.generate_subtasks(task_input).await { … }` 改为直接用空输出：

```rust
        // A1 · lazy per-stage planning: do NOT pre-generate a flat whole-run plan.
        // The graph-flow executor enters each stage and `run_stage_subtasks` lazily
        // plans that stage on entry (see execute.rs). Start with an empty queue.
        let generator_output = GeneratorOutput { subtasks: Vec::new() };
```

> 保留其后对 `generator_output` 的 queue 构造 / 持久化逻辑不动（空列表 → 不建 DB 行、不 emit 一次性 SubtaskCreated）。`run_executor_driven` 据此对每个投影阶段走空分支 → 懒产。

**步骤 4.2** 若 `run()` 在 `generator_output.subtasks.is_empty()` 时有「直接判失败/早退」的旧逻辑，需放行空列表进入 `run_executor_driven`（lazy 下空是正常起点）。检查并移除/放宽该早退。

**验证：**
- `cargo check -p golish-agent-kit` → exit 0；`cargo clippy -p golish-agent-kit -- -D warnings` → exit 0。
- `cargo nextest -p golish-agent-kit` → 全过。
**提交：** `feat(harness): skip upfront flat plan; drive lazy per-stage planning from run()`

---

## Task 5 · 下游编译 + 全量验证（DoD）

**步骤 5.1** 下游编译：
```
cd backend && cargo check -p golish-agent-app -p golish-agent-runtime -p golish-agent-bridge
```
→ exit 0。

**步骤 5.2** 跨 crate 测试：
```
cargo nextest run -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime --status-level fail
```
→ 全 passed。

**步骤 5.3（活体 · 用户跑）** `just dev` 输 `搞一下 example.com`：
- 期望：进每个阶段**先出该阶段 2–4 步 plan**（非一次性 6 条扁平）；末步 submit → 过 gate → 进下一阶段；UI plan 卡随阶段切换；`~/.golish/backend.log` 中 `entering stage` 后紧跟「lazy per-stage plan generated steps=N」+ 该阶段 subtask 执行。

**步骤 5.4** `just precommit` → 全绿后方可合并。
**提交：** `test(harness): verify lazy per-stage planning end-to-end`

---

## 自检

1. **规格覆盖度：** 设计 §3.1→T1；§3.2→T2；§3.3→T3；§3.4→T4；§6 验证→T2/T3/T5。✓
2. **占位符扫描：** `emit_stage_plan_update`（T3.2）与 `HarnessStageHint::from`（T2.2）/`upstream_evidence_for_stage`（T3.1）标注为「复用现有同款构造，对照 file:line」——非 TODO，是带锚点的精确指令（实现者照现有 `emit_plan_update` / `synthesize_stage_subtask` 写）。其余步骤均含完整代码。
3. **类型一致性：** `generate_stage_plan(stage,task_input,upstream_evidence)->Vec<PlannedSubtask>` 在 T1/T2/T3 三处签名一致；`ensure_submit_terminal(&mut Vec<PlannedSubtask>, &str, &str)` 在 T2.2 调用、T2.3 定义、T2.1 测试一致；`PlannedSubtask` 字段全程同 `types.rs:21`。✓
