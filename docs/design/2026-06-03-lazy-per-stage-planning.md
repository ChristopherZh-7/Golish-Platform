# 懒生成逐阶段计划（Lazy per-stage planning）设计

> 目的：把 Task 模式从「一次性产全程扁平 plan + 事后打 stage 标签」改成
> **「进到某阶段才懒生成该阶段的 plan，阶段最后一步固定为 submit + 验证，过 gate 才进下一阶段」**。
>
> 用户拍板（2026-06-03 brainstorming）：
> - **A 懒生成逐阶段**（进阶段才产该阶段 plan）。
> - 中途更新 plan：**先 D（主链）后 A（reflector 增量更新）**——本设计只覆盖 D。
> - **测试阶段不加 flag**：行为无条件替换（仅保留 harness 既有 `stage_mode` / `graph_flow` 主开关，二者默认 ON）。
>
> 证据来源：2026-06-03 本会话亲核真实代码（file:line 见 §1）。状态：Approved（用户已逐点拍板）。

---

## 0. 决策（TL;DR）

- **现状**：`run()` 先调 `executor.generate_subtasks(task_input)` 一次性产 N 个**扁平** subtask（用户截图 6 条）；`run_executor_driven` 按 profile 投影后的 DAG 顺序进每个阶段，对每个 `StageRunRequest` 取 `groups.get(&stage)`（扁平 plan 里 tag 给该阶段的 subtask 下标）调 `run_stage_subtasks`；该阶段没有 tagged subtask 时 `synthesize_stage_subtask` 合成 **1 个模板 subtask**。
- **改为**：取消上游一次性产；**每进一个阶段**调新增 `generate_stage_plan(stage, …)` 懒产该阶段 **2–4 个战术 subtask + 末步固定「Submit & verify {stage} deliverable」**；逐个执行 → 末步 submit → 现有 gate → PASS 推进下一阶段 → 再懒产。
- **无 flag**：测试阶段直接替换（不引入 `GOLISH_HARNESS_*` 新开关）。
- **分期**：本期 **D**（主链：进阶段→懒产→末步 submit→过 gate→进下一阶段）；**后续 A**（reflector 每步后增量改本阶段剩余 task，复用 `refine_plan`）。

---

## 1. 现状勘验（file:line · 已核）

| 环节 | 落点 | 说明 |
|---|---|---|
| Executor 契约 | `golish-agent-kit/src/task_orchestrator/types.rs:280` `trait AgentExecutor` | `generate_subtasks` / `execute_subtask` / `refine_plan` |
| 扁平产 plan | `golish-agent-kit/.../orchestrator.rs:156` `executor.generate_subtasks(task_input)` | run() 上游一次性产全程 |
| bridge 实现 | `golish-agent-bridge/src/bridge_executor/trait_impl.rs:14` | `simple_completion_for_phase(generator_prompt())` + `extract_json_from_response` + `backfill_harness_stage` |
| DAG 驱动 + 进阶段 | `.../subtask_phases/execute.rs:526`（`Some(req)=rx.recv()` 臂） | `indices = groups.get(&req.stage)` → `run_stage_subtasks(req.stage, &indices, queue, …)` |
| 空阶段合成 | `execute.rs:604` `synthesize_stage_subtask(stage, &exec_ctx.task_input)`（定义 `execute.rs:1562`） | 空 → 合成 1 个模板 subtask（per-stage charter 文案） |
| 缺 deliverable fail-closed | `execute.rs:1535` `missing_deliverable_gate_outcome` | 末端无 StageDeliverable → BLOCK + repair（reflector 重试补） |
| 数据结构 | `types.rs:21` `PlannedSubtask{title,description,agent:Option<String>,harness_stage:Option<HarnessStageHint>,nl_slice,acceptance_criteria}`；`types.rs:43` `GeneratorOutput{subtasks}` | — |

**关键洞察**：DAG 驱动骨架与「空阶段合成」接缝**已存在**（`run_stage_subtasks` 的空 `indices` 分支）。懒生成只需把这一分支从「合成 1 个模板 subtask」升级为「调 `generate_stage_plan` 产该阶段 N 个 subtask（含末步 submit）」，并让 `run()` 不再上游一次性产 plan（使每个阶段都走空分支 → 懒产）。

---

## 2. 目标 / 非目标

**目标**
1. 每阶段的 plan **懒生成**：实际跑某阶段时才产该阶段的 subtask 列表（不再一次性产全程）。
2. 每阶段 plan **末步固定 = Submit & verify**（调 `submit_stage_deliverable`，过 gate 才算本阶段 done）。
3. 过 gate **才推进**下一阶段（沿用现有 DAG 推进 + fail-closed）。
4. **plan 事件/UI**：进阶段 emit `plan_updated` = 当前阶段的 task 列表，聊天里那张 plan 卡显示「当前阶段的 plan」（取代扁平全程列表）。

**非目标**
- 不动 gate 内核（`validate_stage_gate` / vacuous / 证据账本）。
- 不动 Chat 模式、不动意图分诊。
- **不加 flag**（测试阶段）。
- 中途更新（reflector 增量 = A）本期不做。
- legacy `execute_subtask_loop`（`graph_flow` OFF）路径不纳入懒生成（懒生成假定 `graph_flow` ON，默认即 ON）。

---

## 3. 方案

### 3.1 新增 trait 方法 `generate_stage_plan`（带默认实现，零侵入 mock）

`AgentExecutor`（types.rs）新增：

```rust
/// Lazily plan ONE stage's tactical subtasks (A1 · lazy per-stage). The
/// returned list MUST end with a "Submit & verify <stage>" subtask. Default
/// returns empty → caller falls back to the single-subtask synthesizer, so
/// existing executors (mocks) need no change; only the bridge overrides it.
async fn generate_stage_plan(
    &self,
    stage: crate::harness::StageKind,
    task_input: &str,
    upstream_evidence: &str,
) -> Result<Vec<PlannedSubtask>> {
    let _ = (stage, task_input, upstream_evidence);
    Ok(Vec::new())
}
```

- 默认 `Ok(vec![])` → 所有现有 impl（含测试 mock）零改动；只有 bridge 覆盖。

### 3.2 bridge 实现 `generate_stage_plan`（trait_impl.rs）

- prompt = 「本阶段 charter」（复用 `synthesize_stage_subtask` 的 per-stage 文案 + `prompts::*`）+ 明确约束：「只产**本阶段** 2–4 个可执行战术 subtask；**最后一个**必须是『Submit & verify the {stage} StageDeliverable』并调 `submit_stage_deliverable`」。
- LLM → `extract_json_from_response` → `serde_json::from_str::<GeneratorOutput>`（复用 `describe_plan_parse_failure` 干净报错）。
- **确定性兜底（不靠模型自觉）**：① 若解析出的列表为空 → 返回 `vec![]`（调用方走单 synth 兜底）；② 若**末步不是 submit 步** → 代码**追加**一个 submit 末步（`title:"Submit & verify {stage} deliverable"`，`agent:Some("pentester"|"analyzer")`）；③ 所有 subtask 的 `harness_stage` 强制设为该 stage（镜像 `synthesize_stage_subtask` 的构造）。

### 3.3 `run_stage_subtasks` 空分支改懒产（execute.rs）

- 现「空 `indices` + dag_strict → `synthesize_stage_subtask` 单步」改为：
  1. `let lazy = executor.generate_stage_plan(stage, &exec_ctx.task_input, &exec_ctx.upstream_evidence_summary()).await.unwrap_or_default();`
  2. `let stage_plan = if lazy.is_empty() { vec![synthesize_stage_subtask(stage, &exec_ctx.task_input)] } else { lazy };`（generator 失败/空 → 单 synth 兜底，绝不空转/真空 PASS，沿用 S1 fail-closed）
  3. emit `plan_updated`（当前阶段 `stage_plan` 的 title 列表）。
  4. 逐个 `execute_single_subtask(&pt, …)`（含末步 submit → gate hook 累积 outcome）。

### 3.4 `run()` 不再一次性产全程（orchestrator.rs）

- run() 上游不再调 `generate_subtasks` 产扁平 plan（改用空 `GeneratorOutput{subtasks:vec![]}`），使 queue/groups 起始为空 → 每个被投影进 DAG 的阶段都走 §3.3 懒产分支。
- 旧扁平 emit（"Generating subtasks…" / 一次性 SubtaskCreated）随之省略；plan 可见性改由 §3.3 的 per-stage `plan_updated` 承载。

---

## 4. 影响面 / 受影响文件

| 文件 | 改动 | 风险 |
|---|---|---|
| `golish-agent-kit/src/task_orchestrator/types.rs` | trait + `generate_stage_plan`（默认实现） | 低 |
| `golish-agent-bridge/src/bridge_executor/trait_impl.rs` | bridge 实现 + 强制 submit 末步 + 单测 | 中 |
| `golish-agent-kit/.../subtask_phases/execute.rs` | `run_stage_subtasks` 空分支改懒产 + emit plan_updated | ⚠ 高（执行主干） |
| `golish-agent-kit/.../orchestrator.rs` | `run()` 跳过上游扁平 generate_subtasks | ⚠ 高 |

---

## 5. 风险 / 回滚

- **R1（执行主干重排）**：最高风险；**无 flag** → 测试阶段直接替换，回滚 = `git revert`。靠充分单测 + 活体复跑兜底。
- **R2（LLM 末步不产 submit）**：§3.2 代码**确定性追加** submit 末步，不靠模型自觉。
- **R3（generate_stage_plan 失败/空）**：§3.3 回落到 `synthesize_stage_subtask` 单步 → 绝不空转 / 真空 PASS（沿用现有 S1 fail-closed + `missing_deliverable_gate_outcome`）。
- **R4（legacy graph_flow OFF 路径）**：空 queue 下 legacy loop 无事可跑；懒生成假定 graph_flow ON（默认），legacy 路径本期不支持（测试阶段可接受）。

---

## 6. 验证（DoD）

- **单测**：① bridge `generate_stage_plan` 解析 + 「末步非 submit → 自动补 submit 末步」+「空/失败 → 空列表」；② `run_stage_subtasks` 懒产路径（mock executor 覆写 `generate_stage_plan` 产多步含 submit → 全部执行 + 末步进 gate；mock 返空 → 回落单 synth）。
- **活体/集成**：`just dev` 跑 `example.com` → 每进一个阶段**先出该阶段 plan**（非一次性 6 条扁平）→ 末步 submit → 过 gate → 进下一阶段；UI plan 卡随阶段切换；日志 `entering stage` 后紧跟该阶段 plan 的 subtask 执行。
- `just precommit` 全绿后才合并；以**实测复跑日志**为准（AGENTS.md §3 / I7）。

---

## 7. 开放问题（实现前）

1. `generate_stage_plan` 的 `upstream_evidence` 取值口径：用 `exec_ctx` 已有的「上游阶段 deliverable / completed_results 摘要」即可（实现时按 `ExecutionContext` 真实可用字段定，不新增持久化）。
2. 每阶段 task 数上限（建议 hard cap 4，防 LLM 产过多）。
3. submit 末步的 `agent`：复用 `synthesize_stage_subtask` 的 per-stage agent 选择（reporting→analyzer，其余→pentester）。
