# 治根：stage 管线驱动 subtask（必经阶段不被跳过）

> 日期：2026-06-02
> 状态：proposed（待实现）
> 触发：用户观察「评估外部攻击面 → 直奔 pentest，scoping 没跑」。本设计修根因。
> 关联：`docs/superpowers/plans/2026-06-02-engine-v2-p2-metalcraft-graph-executor.md`（方案 C executor-driven 已落地）、`docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`（白名单强制）。

## 1. 现象（实锤）

最新一轮 `pentest-chat-1780414078320-1`（23:28–23:32）：

- Generator 把「评估 example.com 外部攻击面」拆成 **5 个子任务**，全部偏 recon/report，**没有 scoping / target_intel 子任务**（`harness::backfill … backfilled=1 total=5`，唯一 stage 标签 `ExternalAttackSurface`）。
- 第 1 个真正执行的子任务 charter = `external_attack_surface`，编排器按 `mandatory_routing_rules`（安全/侦察 → 永远 `sub_agent_pentester`）派给 pentester。
- 全程 **无 `submit_stage_deliverable`、无 gate decision、无 cursor 推进**——卡在 recon 派发循环（叠加 mimo 文本式工具调用导致 pentester 连派 3 次）。

## 2. 根因（直接读代码确认）

栈：`stage_mode_enabled()` 默认 ON + `graph_flow_enabled()` 默认 ON，`just dev` 不设覆盖 → live 走 **`run_executor_driven`**（`subtask_phases/execute.rs`）。

1. **Generator 自由拆解**（`prompts/mod.rs::generator_prompt`）：只「能匹配就打 `harness_stage` 标签，匹配不上就省略」，**从不强制产出 scoping / target_intel**。用户任务窄（只问外部攻击面）→ LLM 不产 scoping。
2. **executor-driven 图按 DAG 顺序走**（scoping→target_intel→eas→…），但每个 stage 只填 Generator 子任务按 tag 分组的结果（`stage_execution::group_subtasks_by_stage`）。
3. **关键 bug**：某 DAG stage 没有 Generator 子任务时，`run_stage_subtasks` 的 `indices` 为空 → 循环跑零次 → 默认返回 `StageFlowOutcome::pass_with_progress()`（`execute.rs:601`）→ **该 stage 不跑任何活、不跑 gate，直接 auto-pass** → 游标推进。
4. 于是 scoping + target_intel 是**空的 auto-pass 空操作**，第一个有子任务的 stage 是 eas → pentester。这就是「直奔 pentest，scoping 被跳过」。

> 一句话：harness 是 **generator-driven**（有什么子任务就跑什么 stage），不是 **stage-driven**（profile DAG 要求哪些 stage 就必须跑）。空的必经 stage 被静默 auto-pass = gate 旁路。

## 3. 目标

让 harness 真正 **stage-driven**：executor 图走到的**每个必经 stage**都必须实跑其 charter 并过 gate，**不能因为 Generator 没产子任务就空过**。Generator 的子任务降级为「某 stage 的执行细节」，而非「决定哪些 stage 存在」。

## 4. 方案

### 方案 A（推荐）· stage-task 合成 + 取消空阶段 auto-pass
在 `run_stage_subtasks` 里：当某个被图访问到的 stage `indices` 为空时，**合成一个该 stage 的 stage-task**（title/description 从 stage charter + operation 目标派生）并实跑 + 过 gate，而不是返回 `pass_with_progress`。

- 改点集中在 `run_stage_subtasks`（`execute.rs`）+ 一个 `synthesize_stage_task(stage, task_input)` 纯函数（可单测）。
- 复用现有 charter 注入（`stage_charter`）+ gate 回查（existence/kinds/freshness）。
- **flag 闸**：新 `GOLISH_HARNESS_STAGE_SYNTHESIS`（默认 **OFF** → 逐字节回滚安全）。ON 时空必经 stage 被合成实跑。
- 合成范围：只对**图实际访问到**的 stage 生效（bail-to-reporting 已防止跑无关下游 stage）。scoping/target_intel 在线性前缀必经 → 必被合成。

优点：最小、集中、确定性；Generator 不动；必经 stage 永远有 charter 实跑 + gate；空阶段不再旁路 gate。
缺点：合成 task 的描述质量依赖模板（但 charter 本就强约束）；若 profile 含下游 stage，可能比用户「只问外部攻击面」多跑（见 §5 决策点）。

### 方案 B · 强制 Generator 产出有序阶段序列
改 `generator_prompt` 要求「先 scoping 再 target_intel 再 …」，并在 `backfill` 后**确定性注入**缺失的必经 stage 子任务。
- 优点：legacy flat loop 也受益。
- 缺点：仍依赖 LLM 顺从；注入逻辑等价于方案 A 的合成但更分散；**不修**空阶段 auto-pass 这个 gate 旁路。

### 方案 C · A + B
A 修 executor-driven 路径的 gate 旁路；B 兜底 legacy 路径 + 改善 Generator 输出。最全但最大。

## 5. 决策点（需用户拍）

1. **合成范围**：
   - (a) 只补**必经前缀**（scoping、target_intel 等 eas 之前的线性必经 stage）——最贴合「别跳过 scoping」，不扩大跑动范围。
   - (b) 补**所有图访问到的空 stage**——完整 stage-driven，但「评估外部攻击面」可能连带跑 enumeration/vuln_triage（若有进展）。
   - 推荐先做 (a)，把范围风险降到最低。
2. **flag 默认**：建议默认 OFF，真机验证后再翻 ON（同 skeleton/freshness 灰度惯例）。

## 6. 实现增量（flag OFF 时行为逐字节不变）

1. `synthesize_stage_task(stage, task_input) -> PlannedSubtask`（纯函数 + 单测：scoping/target_intel 文案、tag 正确）。
2. `harness/mod.rs` 加 `stage_synthesis_enabled()`（`GOLISH_HARNESS_STAGE_SYNTHESIS`，默认 OFF）+ 导出 + parser 单测。
3. `run_stage_subtasks`：flag ON 且 `indices` 为空且 stage 属必经范围 → 跑合成 task（经 `execute_single_subtask` + gate），否则维持现状（空→pass）。
4. 单测：executor-driven 下「Generator 只产 eas 子任务」→ flag ON 时 scoping/target_intel 被合成实跑（断言 visited + 各自过 gate），flag OFF 时维持空 auto-pass（回滚证明）。
5. `just test-harness` 全绿 + 真机：开 flag 跑 example.com，日志应见 scoping/target_intel 实跑 + gate PASS 后才到 eas。

## 7. 风险与回滚

- 触及 **最高风险核心循环**（executor-driven run）。靠 flag 默认 OFF + 旧空阶段行为保留实现零回归回滚。
- 真机前不翻默认；`GOLISH_HARNESS_STAGE_SYNTHESIS=0` 即回滚。
- 不改 schema、不改 IPC 类型、不改 Generator（方案 A）。

## 8. 与「表层」问题的关系（不在本设计内）

mimo 发文本式工具调用导致 pentester 连派 3 次空跑 = 独立的「表层」问题（强制结构化 / 压住 textual tool-call）。本设计只修「必经 stage 被跳过」的根因；两者可独立推进。
