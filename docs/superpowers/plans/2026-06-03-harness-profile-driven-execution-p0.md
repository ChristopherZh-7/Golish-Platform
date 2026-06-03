# 实现计划 · P0：profile/DAG 真正驱动 Task 执行（A1 治本 + B gate fail-closed）

> 配套设计：`docs/design/2026-06-03-harness-profile-driven-execution.md`。
> 本计划只覆盖 P0（A1+B）。P1（深度上限/工具裁剪/韧性）、P2（分诊/可观测）另列。
> 执行规范：`.cursor/skills/executing-plans` + TDD（`.cursor/skills/test-driven-development`）。
> 日期：2026-06-03。

---

## 实现状态（2026-06-03 · 已落地，测试阶段无 flag）

用户指示：测试阶段，**不要 flag**——P0 行为做成**默认无条件生效**（仅保留 harness 既有的
`stage_mode` / `graph_flow` 主开关，二者默认 ON）。原计划的 `GOLISH_HARNESS_DAG_STRICT`
已**删除**。

- **P0 全部完成（无条件生效）**：S0 可观测 / S1+S2 空 stage 合成执行不再真空 PASS /
  S3 run() 补 stage 标签 + 剔除 forbidden 子任务 / S4 gate fail-closed + 统一提交文案。
  落点：`harness/mod.rs`、`task_orchestrator/subtask_phases/execute.rs`、
  `task_orchestrator/orchestrator.rs`。验证：`golish-agent-kit` **443 nextest 全过** +
  `clippy -D` + fmt 净 + 下游 `golish-agent-app/-runtime/-bridge` 编译过。
- **P1-C 子 agent 深度上限**：`MAX_AGENT_DEPTH 5→2`（一处常量，三个执行点同步生效：
  tool 暴露 / bridge 硬上限 / 委派 shim）。**单层嵌套**（主→子，子不能再生子）。
  ⚠️ 顺带禁用了 pentester→coder 分级委派；要保留一层委派改 `3`。验证：
  `golish-sub-agents`+`golish-agent-runtime` **281/654 nextest 全过** + clippy/fmt 净。
- **P1-D 阶段工具治理**：D1 `tool_list` 对「零扫描 stage（scoping/target_intel/reporting，
  allowed_tool_types 为空）」从清单里**裁掉扫描工具**；D2 `stage_tool_guard` 拦截信息
  **列出当前 stage 允许的工具类型**并提示别重试（治 26 次撞墙）。落点：
  `harness/tool_taxonomy.rs`(+`is_scan_tool_name`)、`agentic_loop/tool_list.rs`、
  `tool_execution/direct/sub_agent_call.rs`。验证：**654 nextest 全过** + clippy/fmt 净。
- **P1-E provider 韧性**：核查发现**核心已存在**——`agentic_loop/stream_retry.rs` 已对
  5xx/timeout/429 做分类 + 指数退避 + 3 次重试 + 终态错误（有单测）；重复文本已被
  `detect_repetitive_text` 检测并**截停**（防失控）。**净新增的剩余项**（重复后 re-prompt
  恢复 / mid-stream 错误重试 / 失败转移到备用模型）是 streaming 热路径上的较大改造，建议
  独立任务做（可活体测时再上），本批未动。

**未做（诚实边界）**：① 活体复跑验证（`just dev` + Task `example.com` 看 scoping 真跑 /
每 stage 过 gate / vuln_triage 不执行 / 嵌套≤1 / 撞墙=0）；② 全量 `just precommit`；
③ 未 commit；④ E 的三个增强项。

---

## （以下为原始计划，flag 部分已废弃，保留作设计记录）

---

## 精确根因（本会话读真码 + 实测确认）

DAG 驱动**骨架已存在且按 DAG 顺序跑**（`run_executor_driven`：`dag = base_operation_graph().project(profile.allowed_stage_set())` → metalcraft Executor 按序访问 scoping→target_intel→eas→enumeration→reporting，每个 stage 发一个 `StageRunRequest`）。真正的洞是：

1. **空 stage 组 = 真空 PASS**：`execute.rs` 收到 `StageRunRequest{stage}` 时 `groups.get(&stage).unwrap_or_default()`；planner 没产 scoping/target_intel 子任务 → `indices` 空 → `run_stage_subtasks` 的 `for &idx in indices` 一次不跑 → `stage_outcome_acc=None` → 末尾 `unwrap_or_else(StageFlowOutcome::pass_with_progress)` → **自动 PASS、无 gate、无日志**。这就是 scoping/target_intel「0 次出现」的真相：被访问但真空放行。
2. **forbidden stage 子任务被孤儿化**：DAG 已按 `allowed_stage_set` 投影，forbidden(vuln_triage) 不在图里 → 引擎从不发它的 StageRunRequest → planner 给它建的 subtask 行永不执行（created 但 dead）。
3. **gate fail-open**（B）：见 `apply_harness_gate_hook`/`parse_deliverable_from_content`，叙述无 JSON → skip 透传。

> 修正后的方向：**不是"让 DAG 驱动"（已经在驱动），而是"让每个被投影进 DAG 的 stage 真正干活 + 真正过 gate，禁止真空 PASS"**，并在计划期清理 forbidden 孤儿。

---

## Flag

新增 `GOLISH_HARNESS_DAG_STRICT`（默认 OFF）。ON 时启用：空 stage 合成执行 + 真空 PASS 改 fail-closed。OFF = 现行为（真空 PASS），零回归、可回滚。`harness/mod.rs` 加 `dag_strict_enabled()`（仿 `two_level_enabled`）。

---

## 步骤（TDD · 每步红→绿）

### S0 · 可观测先行（纯 additive，无 flag）
- 落点：`run_executor_driven` 收 req 处 + `run`。
- 改：INFO 日志打出 ① 选定 profile id；② 投影后 DAG stage 有序列表；③ 每个 StageRunRequest 的 stage + 该 stage 的 indices.len()（=0 即真空 stage）；④ 真空 PASS 时显式 WARN「stage X projected but has no subtasks → (current) vacuous pass」。
- 验证：复跑 example.com，日志能看出 scoping/target_intel 被真空放行。
- 风险：极低。先合这步，立即可观测。

### S1 · 空 stage 不再真空 PASS（B 的核心 · flag ON）
- 落点：`run_stage_subtasks`（空 indices 分支）+ 末尾 `unwrap_or_else`。
- 改：`dag_strict_enabled()` 且 `indices` 为空时，**不返回 pass_with_progress**；走 S2 的「合成 stage 子任务并执行」。S2 落地前，临时返回 `StageFlowOutcome::blocked()`（让引擎在该 stage Interrupt，不真空跨越）——保证"宁可停，不可假过"。
- TDD：`run_stage_subtasks` 空组 + flag ON → 不再 PASS（返回 blocked 或触发合成）。
- 风险：中（改流转语义，flag 包裹）。

### S2 · 合成 stage-scoped 子任务（A1 核心 · flag ON）
- 落点：新增 `AgentExecutor::generate_stage_subtask(stage, task_input, upstream_evidence) -> PlannedSubtask`（或复用 `generate_subtasks` 加 stage 约束）。`bridge_executor` 实现：prompt = 该 stage 的 charter（已有 `prompts::stage_inherited_evidence` + `render_inherited_handoff`）+ "只产出本阶段(scoping/target_intel/...)的一个可执行子任务"。
- 改：`run_stage_subtasks` 空组 + flag ON → 调 `generate_stage_subtask` 合成一个，按正常路径 `execute_single_subtask` 执行 + gate。
- TDD：mock executor，空 scoping 组 → 合成并执行一个 scoping 子任务 → 进 gate。
- 风险：中（新增 executor 方法；下游 bridge 实现）。

### S3 · 计划期清理 forbidden 孤儿（A2 子集 · flag ON）
- 落点：`run()` 建 queue 前（`generate_subtasks` 之后）。
- 改：把 tag 落在 `profile.allowed_stage_set()` 之外（forbidden / 不可达）的 planned subtask **剔除**（不建 DB 行、不 emit SubtaskCreated），打一条 INFO「dropped forbidden-stage subtask: <title> (<stage>)」。
- TDD：assessment + 含 vuln_triage 的假 plan → 该子任务被剔除，不出现在 queue。
- 风险：低。

### S4 · gate fail-closed + 统一提交（B 收口 · flag ON）
- 落点：`apply_harness_gate_hook`（fail-open `return (content, None)`）+ `parse_deliverable_from_content` + `harness_submit_tool.rs` + `build_gate_correction` 矛盾文案。
- 改：flag ON 时 stage-tagged 子任务结尾无合法 deliverable → 返回「阻塞 + 纠正」让 reflector 重试补 deliverable（有重试上限，到顶降级 blocked 收尾，避免死卡）；删 "there is no submit tool" 文案，提交契约统一指向 `submit_stage_deliverable`。
- TDD：stage-tagged + 无 deliverable + flag ON → 不透传，返回阻塞/纠正。
- 风险：中（retry 上限要稳，防死卡——见设计 R2）。

### S5 · 验证（DoD）
- 复跑 example.com + assessment（flag ON）：① 日志首个真正执行的 stage = scoping；② 每个 stage 都过 gate（无真空 PASS、无 silent skip）；③ vuln_triage 永不执行；④ flag OFF 回归：行为同现状。
- `just precommit` 全绿；证据贴 `agent-progress.md`。

---

## 影响面 / 回滚
- 改动集中 `golish-agent-kit`（execute.rs / orchestrator.rs / prompts）+ `bridge_executor`（generate_stage_subtask 实现）。
- 全程 `GOLISH_HARNESS_DAG_STRICT` 包裹，OFF 即回退。S1 先用 blocked 占位、S2 再补合成 —— 任何中间态都是「宁停不假过」，不会比现状更糟。

## 开放问题
1. 空 stage 合成：用 LLM 合成（更聪明）还是每 stage 模板默认子任务（更确定）？建议先模板兜底 + LLM 增强。
2. S4 retry 上限取值（建议 2）+ 到顶降级策略（blocked 收尾 vs 标记 stage skipped 继续）。
3. flag 名最终定 `GOLISH_HARNESS_DAG_STRICT`？是否并入已有 stage_mode flag 体系。
