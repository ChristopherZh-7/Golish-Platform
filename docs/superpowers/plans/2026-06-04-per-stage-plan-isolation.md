# Per-Stage Plan 隔离（PlanManager 完整重构）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。每个任务单独 commit，每步跑验证命令并贴证据。改 DB schema 前已与用户确认（§2.7）。

**目标：** 把后端 `PlanManager` 从"每 session 一份全局扁平 plan"改成"每 harness stage 一份独立 plan"，根除 AI 的 `update_plan` 内容随当前 stage 在前端 stage 卡之间漂移/重复的问题。

**架构：** `PlanManager` 内部 `plan: TaskPlan` → `plans: HashMap<stage_key, TaskPlan>`（`stage_key` = harness stage id，如 `scoping` / `target_intel`；chat 模式用空串 `""` 兜底，保持单卡旧行为）。新增 `current_stage` 记录"最后写入的 stage"，让无参的 `snapshot()` / `format_for_prompt()` / `is_empty()` 仍返回"当前 stage"的 plan（从而 `get_plan` command 与 prompt 注入处签名不变）。DB `execution_plans` 加 `stage_id` 列（nullable，向后兼容），每 stage 一行，按 `(session_id, stage_id)` upsert。前端 `misc-handlers.ts` 早已按 `event.stage_id` 分桶，**前端零改动**。

**技术栈：** Rust（golish-db / golish-agent-kit / golish-agent-bridge / golish-agent-app workspace）+ sqlx + cargo nextest。

---

## 背景：已定位的根因（证据见 2026-06-04 会话日志）

- AI 的 `update_plan` 走 `tool_executors/plan.rs::execute_plan_tool(plan_manager, event_tx, args, stage_id)`，内部 `plan_manager.update_plan(args)` 更新 **全局唯一** 的 `plan: Arc<RwLock<TaskPlan>>`，emit `PlanUpdated{ ..., stage_id }` 时把"当前 active stage"当标签贴上。
- 前端 `frontend/services/ai-events/misc-handlers.ts::handlePlanUpdated`：`if (event.stage_id) setStagePlan(sid, stage_id, plan)` —— 把**整份** plan 塞进该 stage 桶。
- 后果：同一份全局 plan 随 AI 所在 stage 在 `plansByStage` 各桶间漂移；scoping 性质的 step（如 "Document authorized scope and submit deliverable"）出现在 target_intel 卡。
- 后端 prompt（`task_orchestrator/prompts/mod.rs:193`）已要求"只列当前 stage 的 todo"，但弱模型不可靠 → 需后端结构性隔离。

## 已核实的关键事实（决定本计划接线）

1. **PlanManager 是 per-session 单例**：`agent_bridge/config.rs:82` `PlanManager::new().with_db_repo(session_uuid, project_path)`；结构见 `planner/manager/mod.rs:25-32`：`plan / db_repo / session_id / project_path / db_plan_id / event_emitter`。
2. **stage_id 在 plan 执行时已可得**：`agentic_loop/tool_execution/direct/mod.rs:58` 调 `execute_plan_tool(ctx.plan_manager, ctx.events.event_tx, tool_args, stage_id)`，`stage_id` 来自 loop ctx（= `harness_active_stage`）。
3. **update_plan 现签名**：`planner/manager/mutations.rs:17` `pub async fn update_plan(&self, args: UpdatePlanArgs) -> Result<TaskPlan, PlanError>`，末尾 `self.persist_async(&result)`。
4. **持久化**：`mod.rs:124 persist_async` 用 `db_plan_id`（单个）做 update-or-create；`persistence.rs:9 load_from_db` 取 `list_active()[0]` 单行恢复。
5. **读取/注入点**：`mod.rs:77 snapshot()`、`mod.rs:87 format_for_prompt()`、`prepare.rs:127/223` 注入 prompt、`agent-app/src/ai/commands/plan.rs:31` `bridge.plan_manager().snapshot()`。
6. **DB（已逐文件复核，链路比初稿长 → 见任务 2/3 修正）**：
   - schema：`migrations/20260412000003_execution_plans.sql` 定义 `execution_plans`（**无 stage_id**）。
   - golish-db 模型：`models/session.rs:159` `ExecutionPlan`（`FromRow`）+ `models/session.rs:191` `NewExecutionPlan`（**golish-db 自己的一份**，与 db_traits 同名结构是两份）。
   - golish-db repo：`repo/execution_plans.rs::create`（INSERT，**无 stage_id**）、`list_active`（`SELECT *`，加列后自动带）、`update_steps`。
   - db_traits（golish-agent-kit）：`db_traits/types.rs:119` `NewExecutionPlan`、`:206` `ExecutionPlanView`、`:102` `PlanStep`；trait `db_traits/repo.rs:200/210` `plan_list_active`/`plan_create`（签名不变，只是结构体加字段）。
   - **真正的映射桥 = `golish-agent-app/src/ai/db_bridge/orchestration.rs`**：`plan_list_active_impl:141`（golish-db `ExecutionPlan` → db_traits `ExecutionPlanView`）、`plan_create_impl:171/179`（db_traits `NewExecutionPlan` → golish-db `NewExecutionPlan`，再 → `ExecutionPlanView`）。**初稿误把这层写成 `db_shim.rs`；实际 `db_shim.rs` 是纯委托（`repo.plan_create(plan)` / `repo.plan_list_active(..)`），结构体加字段后零改动。**
   - 其它构造点：`golish-pentest-app/src/execution_plans.rs` 的 tauri `plan_create`（构造 golish-db `NewExecutionPlan`，非 harness，补 `stage_id: None`）；测试 stub `planner/tests/manager_tests.rs:729 make_demo_plan`（构造 `ExecutionPlanView`，补字段）。
7. **前端已就绪**：`misc-handlers.ts` 按 `stage_id` 分桶；`stagePlanPersistence.ts` 按 stage 存 localStorage。**前端不改**。

---

## 设计决定（已与用户拍板 · 2026-06-04）

- **D1 路径 = A 完整重构**（用户选）：PlanManager 真正持有 per-stage 多份 plan，DB 每 stage 一行。
- **D2 stage_key 约定**：harness 模式用真实 stage id 字符串；chat / 非 harness（`stage_id=None`）统一映射到空串 `""` 桶 → 单卡旧行为，向后兼容。
- **D3 无参读接口语义**：`snapshot()` / `format_for_prompt()` / `is_empty()` 返回 `current_stage` 桶（最后写入的 stage），使 `get_plan` command 与 `prepare.rs` 注入处**签名不变**；另加显式 `snapshot_for(stage)` / `snapshot_all()` 供需要时用。
- **D4 DB 向后兼容（I10）**：`stage_id` 列 nullable，旧行 NULL 视为 `""` 桶；先扩字段、再上新代码，无需回填。

---

## 文件结构（创建/修改 + 职责）

| 文件 | 改动职责 |
|---|---|
| `golish-db/migrations/20260604000001_execution_plans_stage_id.sql`（新建） | `ALTER TABLE execution_plans ADD COLUMN stage_id TEXT` + 复合索引 |
| `golish-db/src/models/session.rs` | `ExecutionPlan`（FromRow, :159）+ `NewExecutionPlan`（:191, golish-db 版）各加 `stage_id: Option<String>` |
| `golish-db/src/repo/execution_plans.rs` | `create` INSERT 带 `stage_id`（多 1 列 + bind）；`list_active` 已 `SELECT *` 自动带列，无需改 |
| `golish-agent-kit/src/db_traits/types.rs` | `NewExecutionPlan`（:119）加 `stage_id`；`ExecutionPlanView`（:206）加 `stage_id` |
| `golish-agent-app/src/ai/db_bridge/orchestration.rs`（**初稿遗漏 · 关键**） | `plan_create_impl` 构造 golish-db `NewExecutionPlan` 带 `stage_id` + 两处 `ExecutionPlanView` 带 `stage_id`；`plan_list_active_impl` 的 `ExecutionPlanView` 带 `stage_id` |
| `golish-pentest-app/src/execution_plans.rs` | tauri `plan_create` 构造 golish-db `NewExecutionPlan` 补 `stage_id: None`（用户态 CRUD，非 harness） |
| ~~`golish-agent-kit/src/db_shim.rs`~~ | **无需改动**：`execution_plans::create/list_active` 纯委托，结构体加字段即自动透传 |
| `golish-agent-kit/src/planner/manager/mod.rs` | `plan` → `plans: HashMap`，`db_plan_id` → `db_plan_ids: HashMap`，加 `current_stage`；`snapshot/is_empty/format_for_prompt/clear` 按 current_stage；`persist_async(stage, snap)` |
| `golish-agent-kit/src/planner/manager/mutations.rs` | `update_plan(args, stage_id)` 写对应桶（preserved-steps 逻辑按桶内）；`apply_patch_ops` 同步带 stage |
| `golish-agent-kit/src/planner/manager/persistence.rs` | `load_from_db` 恢复**所有** stage 行 → 填 plans map + db_plan_ids，逐 stage emit `PlanUpdated{stage_id}` |
| `golish-agent-kit/src/tool_executors/plan.rs` | `execute_plan_tool` 调 `plan_manager.update_plan(args, stage_id)` |
| `golish-agent-kit/src/planner/tests/*`、`tool_executors/plan.rs` 测试 | 适配新签名 + 新增 per-stage 隔离断言 |

> 前端：无改动（`misc-handlers.ts` 已按 stage_id 分桶）。`get_plan` command / `prepare.rs`：无签名改动（靠 D3 的 current_stage 语义）。

---

## 任务分解（逐步、可单测、频繁 commit）

### 任务 1 · DB migration：execution_plans 加 stage_id
- **文件**：`backend/crates/golish-db/migrations/20260604000001_execution_plans_stage_id.sql`（新建）
- **步骤**：
  ```sql
  -- Per-stage plan isolation: tag each plan row with the harness stage it
  -- belongs to. NULL = chat-mode / non-harness plan (legacy single-card).
  ALTER TABLE execution_plans ADD COLUMN stage_id TEXT;

  CREATE INDEX idx_plans_session_stage
      ON execution_plans(session_id, stage_id);

  COMMENT ON COLUMN execution_plans.stage_id IS
      'Harness stage id (scoping, target_intel, …) this plan belongs to; NULL = chat-mode';
  ```
- **验证**：`cd backend && cargo check -p golish-db`（迁移文件被 `sqlx::migrate!` 收录即编译期可见；如有 `sqlx-data`/offline 缓存则按项目惯例 `cargo sqlx prepare` 重生）。
- **提交**：`feat(db): add stage_id column to execution_plans (per-stage plan)`

### 任务 2 · ExecutionPlan/NewExecutionPlan model + db_traits 加 stage_id
- **文件**：`backend/crates/golish-db/src/models/session.rs`（`ExecutionPlan` :159 与 `NewExecutionPlan` :191，**两份都在这**）、`backend/crates/golish-agent-kit/src/db_traits/types.rs`
- **步骤**：
  1. golish-db `ExecutionPlan`（`:159`，`#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]`）加字段（FromRow 按列名匹配，位置无关）：
     ```rust
     pub stage_id: Option<String>,
     ```
  2. golish-db `NewExecutionPlan`（`:191`，写入 DTO）加字段：
     ```rust
     pub stage_id: Option<String>,
     ```
  3. db_traits `NewExecutionPlan`（`types.rs:119`）加：
     ```rust
     pub stage_id: Option<String>,
     ```
  4. db_traits `ExecutionPlanView`（`types.rs:206`）加：
     ```rust
     pub stage_id: Option<String>,
     ```
- **验证**：`cd backend && cargo check -p golish-db -p golish-agent-kit`（预期在 repo/orchestration/构造点报缺字段 → 任务 3 补）。
- **提交**：`feat(db): thread stage_id through ExecutionPlan/NewExecutionPlan + db_traits`

### 任务 3 · repo INSERT + orchestration 映射桥 + 其它构造点透传 stage_id
- **文件**：`backend/crates/golish-db/src/repo/execution_plans.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/orchestration.rs`、`backend/crates/golish-pentest-app/src/execution_plans.rs`
- **步骤**：
  1. `repo/execution_plans.rs::create` 改 SQL 与 bind（`list_active` 是 `SELECT *`，加列后自动带，**不改**）：
     ```rust
     let row = sqlx::query_as::<_, ExecutionPlan>(
         r#"INSERT INTO execution_plans (session_id, project_path, title, description, steps, stage_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *"#,
     )
     .bind(plan.session_id)
     .bind(&plan.project_path)
     .bind(&plan.title)
     .bind(&plan.description)
     .bind(&plan.steps)
     .bind(&plan.stage_id)
     .fetch_one(pool)
     .await?;
     ```
  2. `orchestration.rs::plan_create_impl`（:165）—— 构造 golish-db `NewExecutionPlan` 透传，并在**两处** `ExecutionPlanView` 构造里带 `stage_id`：
     ```rust
     .plan_create(golish_db::models::NewExecutionPlan {
         session_id: plan.session_id,
         project_path: plan.project_path,
         title: plan.title,
         description: plan.description,
         steps: plan.steps,
         stage_id: plan.stage_id.clone(),
     })
     .await?;
     Ok(ExecutionPlanView {
         id: created.id,
         title: created.title,
         description: created.description,
         steps: created.steps,
         status: convert_plan_status(created.status),
         current_step: created.current_step,
         stage_id: created.stage_id,
     })
     ```
  3. `orchestration.rs::plan_list_active_impl`（:134）—— map 里 `ExecutionPlanView` 补 `stage_id: p.stage_id`：
     ```rust
     .map(|p| ExecutionPlanView {
         id: p.id,
         title: p.title,
         description: p.description,
         steps: p.steps,
         status: convert_plan_status(p.status),
         current_step: p.current_step,
         stage_id: p.stage_id,
     })
     ```
  4. `golish-pentest-app/src/execution_plans.rs` 的 tauri `plan_create`（构造 golish-db `NewExecutionPlan` 处，约 :70）补 `stage_id: None`（用户态 plan CRUD，非 harness）。
  5. **`db_shim.rs` 不动**：`execution_plans::create/list_active` 是 `repo.plan_create(plan)` / `repo.plan_list_active(..)` 纯委托，结构体加字段后自动透传。
- **验证**：`cd backend && cargo check -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-pentest-app`。
- **提交**：`feat(db): persist + map stage_id across repo/orchestration/pentest-app`

### 任务 4 · PlanManager 结构改造（plans/db_plan_ids/current_stage + 读接口）
- **文件**：`golish-agent-kit/src/planner/manager/mod.rs`
- **步骤**：
  1. 结构改为：
     ```rust
     pub struct PlanManager {
         plans: Arc<RwLock<std::collections::HashMap<String, TaskPlan>>>,
         db_repo: Option<Arc<dyn crate::db_traits::DbRepoProvider>>,
         session_id: Option<uuid::Uuid>,
         project_path: Option<String>,
         db_plan_ids: Arc<RwLock<std::collections::HashMap<String, uuid::Uuid>>>,
         current_stage: Arc<RwLock<String>>, // last-written stage key; "" = chat-mode
         event_emitter: Option<super::SharedPlanEventEmitter>,
     }
     ```
     `new()` 用空 HashMap + `current_stage: Arc::new(RwLock::new(String::new()))`。
  2. 加内部 helper：
     ```rust
     async fn current_key(&self) -> String { self.current_stage.read().await.clone() }
     ```
  3. `snapshot()` / `is_empty()` / `format_for_prompt()` 改为读 `plans[current_key]`（缺省返回 `TaskPlan::default()` 等价）：
     ```rust
     pub async fn snapshot(&self) -> TaskPlan {
         let key = self.current_key().await;
         self.plans.read().await.get(&key).cloned().unwrap_or_default()
     }
     pub async fn is_empty(&self) -> bool {
         let key = self.current_key().await;
         self.plans.read().await.get(&key).map(|p| p.is_empty()).unwrap_or(true)
     }
     ```
     `format_for_prompt()` 同理：取 `plans[current_key]`，其余格式化逻辑不变。
  4. 加显式访问器（供恢复/调试）：
     ```rust
     pub async fn snapshot_for(&self, stage: &str) -> Option<TaskPlan> {
         self.plans.read().await.get(stage).cloned()
     }
     ```
  5. `clear()` 清当前 stage 桶：`self.plans.write().await.remove(&self.current_key().await);`
  6. `persist_async` 改签名 `fn persist_async(&self, stage_key: &str, snapshot: &TaskPlan)`：把 `self.db_plan_id`（单个）逻辑换成 `self.db_plan_ids` 按 `stage_key` 读/写；`NewExecutionPlan` 构造体加 `stage_id: (!stage_key.is_empty()).then(|| stage_key.to_string())`。其余 update-or-create 流程不变（existing_id 从 `db_plan_ids[stage_key]` 取，create 成功后写回 `db_plan_ids[stage_key]`）。
- **验证**：`cargo check -p golish-agent-kit`（mutations/persistence 会报 → 任务 5/6 修）。
- **提交**：`refactor(planner): PlanManager holds per-stage plans + db ids + current_stage`

### 任务 5 · update_plan / apply_patch_ops 带 stage
- **文件**：`golish-agent-kit/src/planner/manager/mutations.rs`、`tool_executors/plan.rs`
- **步骤**：
  1. `update_plan` 改签名并写对应桶：
     ```rust
     pub async fn update_plan(
         &self,
         args: UpdatePlanArgs,
         stage_id: Option<&str>,
     ) -> Result<TaskPlan, PlanError> {
         let key = stage_id.unwrap_or("").to_string();
         // ...（验证逻辑不变）...
         // 把所有 `self.plan.read()/write()` 改为对 plans 桶的读写：
         let mut plans = self.plans.write().await;
         let entry = plans.entry(key.clone()).or_default();
         // preserved_steps / existing_id_map 改为基于 `entry`（桶内现有 steps）
         // entry.explanation/steps/summary/version/updated_at 更新（version += 1）
         let result = entry.clone();
         drop(plans);
         *self.current_stage.write().await = key.clone();
         self.persist_async(&key, &result);
         Ok(result)
     }
     ```
     注意：原实现先 `read` existing 再 `write`；改为单次 `write` 锁内完成 preserved-steps 计算（桶内 self-contained），避免读写竞态。
  2. `apply_patch_ops` 同步加 `stage_id: Option<&str>` 参数，写对应桶 + `current_stage` + `persist_async(&key, ...)`。
  3. `tool_executors/plan.rs::execute_plan_tool`：`plan_manager.update_plan(update_args, stage_id).await`；`execute_plan_patch_tool` 若调 apply_patch_ops 也补 `stage_id`（该函数已有 `stage_id` 上下文则透传，否则传 `None`）。
- **验证**：`cargo check -p golish-agent-kit` 通过；`cargo nextest run -p golish-agent-kit -E 'test(planner)'`（任务 8 修测试后全绿）。
- **提交**：`refactor(planner): update_plan/apply_patch_ops write per-stage bucket`

### 任务 6 · load_from_db 恢复所有 stage
- **文件**：`golish-agent-kit/src/planner/manager/persistence.rs`
- **步骤**：把"取 `plans[0]` 单行"改为"遍历 `list_active` 全部行，按 `stage_id`（NULL→`""`）填 `self.plans` 与 `self.db_plan_ids`，对每行 emit 一次 `PlanUpdated`（带该行 stage_id）"：
  ```rust
  let rows = crate::db_shim::execution_plans::list_active(repo.as_ref(), project_path).await.ok()?;
  if rows.is_empty() { return false; }
  let mut plans = self.plans.write().await;
  let mut ids = self.db_plan_ids.write().await;
  for row in &rows {
      let key = row.stage_id.clone().unwrap_or_default();
      let plan_steps: Vec<PlanStep> = /* 现有 steps 解析逻辑 */;
      let summary = PlanSummary::from_steps(&plan_steps);
      let tp = TaskPlan { explanation: Some(row.description.clone()), steps: plan_steps,
                          summary, version: 1, ..Default::default() };
      if let Some(em) = &self.event_emitter {
          em.emit_plan_updated_for_stage(tp.version, tp.summary.clone(), tp.steps.clone(),
                                         tp.explanation.clone(), row.stage_id.clone());
      }
      plans.insert(key.clone(), tp);
      ids.insert(key, row.id);
  }
  true
  ```
  - `PlanEventEmitter` trait（`planner/mod.rs:40`）加一个带 stage_id 的方法 `emit_plan_updated_for_stage(...)`（或给现有 `emit_plan_updated` 加 `stage_id: Option<String>` 参数并更新其唯一实现）。其上层实现把 `stage_id` 透传进 `AiEvent::PlanUpdated{ stage_id }`。
- **验证**：`cargo check -p golish-agent-kit`；`cargo nextest run -p golish-agent-kit -E 'test(planner::tests::manager)'`（含 load_from_db 测试，任务 8 调整）。
- **提交**：`refactor(planner): restore all per-stage plans from DB on load`

### 任务 7 · 修编译断点 + emitter 实现/调用方
- **文件**：`rg "emit_plan_updated" backend/crates --type rust` 命中的实现处（higher-layer 把 trait 包成事件通道的地方）、以及任何直接 `PlanManager::update_plan(` 调用方
- **步骤**：按编译器报错逐个修：①给 emitter 实现补 `stage_id` 透传到 `AiEvent::PlanUpdated`；②所有 `update_plan(args)` 调用点补第二参（生产仅 `execute_plan_tool` 已在任务 5 改；其余为测试）。
- **验证**：`cd backend && cargo check --workspace --all-targets 2>&1 | tail -30`（零 error）。
- **提交**：`fix(planner): thread stage_id through plan event emitter + callers`

### 任务 8 · 测试更新 + 新增 per-stage 隔离断言
- **文件**：`golish-agent-kit/src/planner/tests/manager_tests.rs`、`tests/patch_tests.rs`、`tests/property_tests.rs`、`tool_executors/plan.rs`（`#[cfg(test)]`）
- **步骤**：
  1. 编译断点修复：① 现有 `manager.update_plan(args)` 调用全部补 stage 参（普通用例传 `None`）；② 测试 stub 构造 `ExecutionPlanView` 的字面量（如 `planner/tests/manager_tests.rs:729 make_demo_plan`）补 `stage_id: None`；③ 任何测试构造 db_traits / golish-db `NewExecutionPlan` 字面量补 `stage_id: None`。
  2. 新增隔离断言：
     ```rust
     #[tokio::test]
     async fn plans_are_isolated_per_stage() {
         let m = PlanManager::new();
         m.update_plan(args(&["scope step"]), Some("scoping")).await.unwrap();
         m.update_plan(args(&["recon step"]), Some("target_intel")).await.unwrap();
         assert_eq!(m.snapshot_for("scoping").await.unwrap().steps.len(), 1);
         assert_eq!(m.snapshot_for("target_intel").await.unwrap().steps.len(), 1);
         // current_stage = 最后写入的 target_intel
         assert_eq!(m.snapshot().await.steps[0].step, "recon step");
     }
     ```
     （`args(&[..])` 复用现有测试 helper；若无则内联构造 `UpdatePlanArgs`。）
  3. `execute_plan_tool` 测试：断言 emit 的 `PlanUpdated.stage_id == Some("scoping")` 当传入 stage。
- **验证**：`cd backend && cargo nextest run -p golish-agent-kit -E 'test(planner) | test(plan)'` 全绿。
- **提交**：`test(planner): per-stage isolation + stage_id tagging coverage`

### 任务 9 · 收口（全量验证 + 前端冒烟）
- **步骤**：
  1. `just precommit`（fmt + check-fe + test-fe + lint-rust + test-rust-all）全绿。
  2. `code-audit` 收口：核对 `get_plan` command 行为（chat 模式 current_stage="" → 旧单卡）、`prepare.rs` prompt 注入（当前 stage plan）、前端无改动仍正确（手动 E2E：`just dev` → task『搞一下 example.com』→ scoping 卡只含 scoping 的 todo、target_intel 卡只含 target_intel 的 todo，"Document authorized scope" 不再串台）。
  3. 把 backend.log 的 `entering stage` + per-stage `update_plan`（stage_id 各异）证据贴进 `agent-progress.md`。
- **验证**：`just precommit` 退出码 0；E2E 截图/日志为证。
- **提交**：`chore(planner): finalize per-stage plan isolation + audit`

---

## 自检

- **规格覆盖**：DB schema→任务 1；model+db_traits→任务 2；repo INSERT + orchestration 映射桥 + pentest-app + db_shim(不动)→任务 3；PlanManager 结构→任务 4；写路径→任务 5；恢复路径→任务 6；编译/事件透传→任务 7；测试→任务 8；收口→任务 9。前端"无改动"由 D3（无参读接口语义不变）+ 既有 `misc-handlers` 按 stage_id 分桶保证。
- **类型一致**：`stage_id: Option<String>` 贯穿 migration→golish-db `ExecutionPlan`/`NewExecutionPlan`→repo INSERT bind→db_traits `NewExecutionPlan`/`ExecutionPlanView`→orchestration 双向映射→`persist_async(stage_key)`；`db_shim` 纯委托不出现该字段。`update_plan(args, stage_id: Option<&str>)` 在任务 5 定义、任务 5/7 调用方、任务 8 测试三处一致；`current_stage`/`db_plan_ids`/`plans` 三个新字段在任务 4 定义、任务 5/6 使用。
- **占位符扫描**：无 TODO/待定；每个代码步骤含真实片段与真实文件路径。
- **风险**：① `load_from_db` 旧逻辑只取 `[0]`，改全量遍历后需确认 `list_active` 排序不影响（按 stage 入 map，顺序无关）；② chat 模式空串桶与 harness 桶共存于同一 session 的 DB（stage_id NULL vs 具体）——`current_stage` 决定 `snapshot()` 返回哪桶，get_plan 在 chat 模式恒为 ""，符合预期；③ `persist_async` 由单 id 变 per-stage id map，首次某 stage 写入会 create 新行（每 stage 一行 execution_plans），需在任务 9 E2E 确认 DB 行数符合预期；④ `NewExecutionPlan` 有**两份**（golish-db `models/session.rs` 与 db_traits `types.rs`），两份都要加 `stage_id`，且 `orchestration.rs` 的 db_traits→golish-db 映射要显式带上——这是初稿最易漏的点（初稿误标 `db_shim`，实测真正映射桥在 `orchestration.rs`）。
