# Task 断线恢复（state-driven resume）实现计划

> **面向 AI 代理的工作者：** 必需子技能：superpowers:executing-plans 逐任务实现；systematic-debugging 已完成根因调查（见下）。测试阶段，无 flag、无回滚分支，逐层 commit。

**目标：** 让 task 模式在断线/进程被杀后，用户再发一条消息（"继续" / 任意文本 / 空）能**恢复**上一个未完成的 operation（从 checkpoint 的 `next_node` 续跑），而不是新建任务从 scoping 重来。

**核心原则：** 恢复是**状态驱动**——由 DB 里"该 chat 会话是否有可恢复的 operation"决定，**不**特判"继续"关键字。chat 会话 = 一个 operation 的持久锚点。

**技术栈：** Rust（golish-db / golish-agent-kit / golish-agent-app）+ cargo nextest + 嵌入式 PG（sqlx `query_as` 运行时校验 + `migrate!` 自动应用）。

---

## 根因（systematic-debugging 第一阶段，证据已核）

1. **入口无恢复路径**：`chat.rs::execute_task_mode`（`chat.rs:113`）每条 task 消息都 `sessions::create`（`chat.rs:133`，注释"keeps task DB rows isolated per task invocation"）新建 DB session + `orchestrator.run()`（`chat.rs:171`）新建 task + `operation_state` 起点固定 `Scoping`（`orchestrator.rs:125-129`）。
2. **"继续" 路由**：`deterministic_intent("继续")` 返回 `None`（不在任务/闲聊词表，`chat.rs:240-316`）→ 落 `execute_task_mode` → 被当任务文本喂 scoping。
3. **恢复引擎已存在但没接线**：`Executor::resume(thread, inject)`（`graph_engine/executor.rs:276`）从 `DbFlowCheckpointer`（`stage_execution.rs:59`，键 = task_id，存 `operation_state.state_blob.graph_flow.{state,next_node}`）载入并续跑；但 `run_executor_driven` 永远 `.run(default)`（`execute.rs:515`），从不 `.resume()`。
4. **连带 bug A**：`run_executor_driven` 把引擎 `Interrupted`（暂停可恢复）也落 `Finished`（`execute.rs:603-616`）→ 暂停 op 变"已完成"不可恢复。
5. **连带 bug B**：startup reaper（`golish-db/lib.rs:89` → `repo/tasks.rs:fail_abandoned`）把超时非终态一律 `failed`，会误杀有 checkpoint 的可恢复 op。

---

## 已核实关键事实

- 迁移：`sqlx::migrate!("./migrations")`（`pool.rs:53`）启动自动跑；新增 `.sql` 即生效。
- repo 全用 `sqlx::query_as`（运行时校验）→ 加列**无需** `.sqlx` 离线缓存重生。
- `Session`（`models/session.rs:11`）派生 `FromRow`（按列名映射），**无** `ts_rs::TS` → 加列不破 IPC 类型链（不变量 I5）。
- PG 唯一索引默认 NULL 互不冲突 → 给已有行（chat_session_key 全 NULL）加唯一索引安全（不变量 I10 扩展式）。
- `operation_state.operation_id` = task_id（PK）；`tasks` 有 `session_id` FK + `status` + `created_at`。

---

## 任务分解（逐层、可单测、逐层 commit）

### 任务 1 · L1 锚点：schema + model + upsert
- **文件**：
  - 新建 `golish-db/migrations/20260604000002_sessions_chat_session_key.sql`：`ALTER TABLE sessions ADD COLUMN IF NOT EXISTS chat_session_key TEXT;` + `CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_chat_session_key ON sessions(chat_session_key);`
  - `golish-db/src/models/session.rs`：`Session` 加 `pub chat_session_key: Option<String>,`。
  - `golish-db/src/repo/sessions.rs`：加 `upsert_by_chat_key(pool, chat_key, NewSession) -> Session`（`INSERT ... ON CONFLICT (chat_session_key) DO UPDATE SET updated_at=NOW() RETURNING *`）。
- **验证**：`cargo check -p golish-db`；SQL 字符串单测（`upsert` 含 `ON CONFLICT`）。
- **提交**：`feat(db): anchor chat session to one DB session via chat_session_key upsert`

### 任务 2 · L1 入口接线
- **文件**：`golish-agent-app/src/ai/commands/core/chat.rs::execute_task_mode`：把 `sessions::create` 换成 `sessions::upsert_by_chat_key(pool, _session_id, NewSession{...})`；`uuid_session_id` 改取自 upsert 结果（同一 chat → 同一行）。
- **验证**：`cargo check -p golish-agent-app`。
- **提交**：`fix(task-mode): upsert one DB session per chat session (stop per-message row)`

### 任务 3 · L2 入口判恢复
- **文件**：
  - `golish-db/src/repo/tasks.rs`：加 `latest_resumable_by_session(pool, session_id) -> Option<Task>`：`SELECT t.* FROM tasks t JOIN operation_state os ON os.operation_id=t.id WHERE t.session_id=$1 AND t.status IN ('running','waiting') AND os.state_blob -> 'graph_flow' IS NOT NULL ORDER BY t.created_at DESC LIMIT 1`。
  - `chat.rs::execute_task_mode`：upsert 后查 resumable；`Some(task) => orchestrator.resume(task.id, task_input, executor)`，`None => orchestrator.run(task_input, executor)`。
- **验证**：`cargo check`；SQL 单测（含 `graph_flow`/`IN ('running', 'waiting')`）。
- **提交**：`feat(task-mode): resume-aware entry — branch resume vs new by DB state`

### 任务 4 · L3 orchestrator.resume + run_executor_driven(resume)
- **文件**：
  - `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：`run_executor_driven` 加 `resume: bool` 形参；`exec_fut` 处按 `resume` 选 `executor_obj.resume(&thread, None)` / `.run(default)`（类型注解为 `Pin<Box<dyn Future<...>+Send>>`）。
  - `golish-agent-kit/src/task_orchestrator/orchestrator.rs`：加 `pub async fn resume(&mut self, task_id, user_message: &str, executor) -> Result<String>`：set task Running → emit "resuming" → `run_executor_driven(task_id, &[], executor, true)`（空 queue：roadmap 由 DAG 重发）→ 失败 `fail_task_if_active`。
- **验证**：`cargo nextest run -p golish-agent-kit -E 'test(task_orchestrator)'`。
- **提交**：`feat(harness): TaskOrchestrator::resume wires Executor::resume from checkpoint`

### 任务 5 · L4a Interrupted→Waiting（不再误判 Finished）
- **文件**：`execute.rs::run_executor_driven` 收尾：捕获 `RunOutcome`；`Interrupted{reason,resume_from}` → `tasks::update_status(Waiting)` + emit paused TaskProgress + `return Ok(paused 摘要)`（**不**跑 reporter/不落 Finished）；`Completed`/其它走原 report+Finished。
- **验证**：`cargo nextest run -p golish-agent-kit -E 'test(execute_harness_loop)'`（新增：Interrupted → task 落 Waiting，不 Finished）。
- **提交**：`fix(harness): interrupted operation marked Waiting (resumable), not Finished`

### 任务 6 · L4b reaper carve-out（有 checkpoint 不误杀）
- **文件**：`golish-db/src/repo/tasks.rs`：`FAIL_ABANDONED_TASKS_SQL` WHERE 加 `AND NOT EXISTS (SELECT 1 FROM operation_state os WHERE os.operation_id=tasks.id AND os.state_blob -> 'graph_flow' IS NOT NULL)`（有 checkpoint 的不 fail）；新增 `pause_resumable_abandoned`（把超时 `running`+有 checkpoint 改 `waiting`）+ `lib.rs` 启动调用。
- **验证**：SQL 单测（fail 排除 checkpoint；pause 只动 running+checkpoint）。
- **提交**：`fix(db): reaper keeps checkpointed tasks resumable (waiting), only fails truly-dead`

### 任务 7 · 收口
- `cargo fmt` + `cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app --all-targets -D warnings`；`just precommit`；更新 `agent-progress.md` + `feature_list.json`。
- **提交**：`chore(task-resume): audit + evidence`

---

## 自检

- **规格覆盖**：根因 1→任务 2+3；根因 3→任务 4；连带 A→任务 5；连带 B→任务 6。
- **不靠关键字**：恢复由 `latest_resumable_by_session`（DB 状态）决定，"继续"无特判。
- **复用而非重造**：`Executor::resume`+`DbFlowCheckpointer`+`HarnessResumeState` 全复用。
- **类型一致**：`resume` 形参在任务 4 加、`run_executor_driven(resume)` 同处调。
- **风险**：steering（用户在恢复时给新指令）本期仅记录/续跑，不注入引擎 FlowUpdate（FlowUpdate 只管 stage 路由）；歧义"暂停 op + 新目标"本期按"输入态→续跑"，执行中遇新目标的二选一 UI 留后续。
