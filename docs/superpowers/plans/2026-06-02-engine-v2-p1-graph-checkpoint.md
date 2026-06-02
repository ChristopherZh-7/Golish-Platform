# Engine v2 · P1 图骨架 + 全量检查点/断点续跑 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
>
> **执行约束**：commit / push 按 AGENTS.md §2.7（未经用户授权不 commit、不 push）。Rust 编译慢，按 Task 末尾指定命令局部编译。

**目标：** 把「杀进程能原位恢复」+「操作有图可视/可驱动」补上——P0 已让证据持久化，P1 让**整条 operation 的执行状态**持久化且可续跑，并 vendor metalcraft 的图执行器范式作为底座。

**架构（留-搓-借）：** **借** metalcraft（`rust4ai/metalcraft`，MIT，本会话已 clone 深读 `graph.rs`/`executor.rs`/`checkpoint.rs`）的 `Graph/CompiledGraph/Executor/RunOutcome/Checkpointer/to_mermaid`，**照抄进** `golish-agent-kit/src/harness/graph_engine/`（不引外部依赖，bus-factor-1/v0.3 风险见 gap 附录 A.4）。**留** 现有 `operation_graph.rs`(base_operation_graph+project+decide_transition) 作 stage 路由真相源。**搓** 一个 `DbCheckpointer` + stage_runs/state_blob 写入接缝。

**技术栈：** Rust 2021（golish-agent-kit harness + orchestrator、golish-db repo、golish-agent-app 桥接）、sqlx、嵌入式 Postgres。Feature flag `GOLISH_HARNESS_STAGE_MODE`（默认 ON）。

---

## 0. 现状（2026-06-02 本会话亲核）

| 件 | 文件 | 状态 |
|---|---|---|
| metalcraft 三件套 | `/tmp/refs/metalcraft/src/{graph,executor,checkpoint}.rs` | ✅ clone 深读 + 附录 A 行级断言坐实 |
| `stage_runs` 表 + repo | `golish-db/src/repo/stage_runs.rs`（insert/get/list_for_operation/mark_terminal/set_active_sprint_contract）| ✅ schema+repo 在 / ❌ **golish-agent-kit 零调用 → 空表** |
| `operation_state.state_blob` + `write_state_blob` | `golish-db/src/repo/operation_state.rs`（含 state_blob 列 + advance_cursor + write_state_blob）| ✅ 在 / ❌ **零调用 → 未用** |
| stage 路由 DAG + 转移 | `harness/{operation_graph,stage_transition}.rs`（base_operation_graph/project/decide_transition）| ✅ 在用（gate 过 → drive_stage_transition 推 operation_state.current_stage）|
| orchestrator run/resume | `task_orchestrator/orchestrator.rs` | ⚠️ run() 插 operation_state(Scoping)+推游标; **resume() 重载 subtask 但 `harness_stage=None`（丢 harness 上下文，走旧路径）** |
| DbRepoProvider operation_state_* | `db_traits/repo.rs` + `db_bridge/mod.rs` | ✅ operation_state_{insert,get,advance_stage} 已是 trait 方法 / ❌ 无 stage_run_* / 无 write_state_blob |

> 核心：DB 原语齐了（stage_runs + state_blob），缺**编排层写入接缝 + metalcraft vendor + resume 恢复 harness 上下文**。

---

## 1. 关键设计决策（执行前请用户确认）

**D-1 · 集成形态：Shape B（vendor + 增强），不做 Shape A（全替换）。**
- **Shape A（不选）**：把 orchestrator subtask 循环重写成 metalcraft `Graph<OperationState>`（stage=节点）。推倒现有可用的 operation_graph + drive_stage_transition，风险高、收益与 B 重叠。
- **Shape B（选）**：vendor metalcraft 为 `harness/graph_engine/`（库 + 自带测试），用其 **Checkpointer/RunOutcome/to_mermaid** 三件：① `DbCheckpointer` 持久化到 stage_runs+state_blob；② 编排层在 stage 起止写 stage_runs；③ resume 读 operation_state 恢复 harness_stage；④ operation DAG 导出 Mermaid 供「图」UI。**保留** decide_transition 作 stage 路由真相源。低风险拿到「断点续跑 + 图」，且为将来 Shape A 迁移留好底座。

**D-2 · state_blob 内容：** 定义 `HarnessResumeState { profile, current_stage, queue_titles: Vec<String>, completed_count, schema_v }`（serde → operation_state.state_blob JSONB）。MVP 只存重建 queue + 定位所需，不存完整 LLM 历史（那走 message_chains）。

**D-3 · DB 访问：** 沿用 P0 的 Option-A 桥接——给 `DbRepoProvider` + `db_shim` + `GolishDbRepoProvider` 加 stage_run_* / operation_state_write_state_blob（镜像现有 operation_state_* 方法；默认 no-op 不破 mock）。**不**把 PgPool 泄进 kit。

**D-4 · metalcraft vendor 边界：** 照抄 graph/executor/checkpoint 三文件 + 其单测；改 `crate::error::{GraphError,Result}` 为 graph_engine 本地 error 模块；头部加 MIT 出处注释（`rust4ai/metalcraft` + commit）。**不**引为 Cargo 依赖。

---

## 2. 文件结构（创建/修改一览）

- **创建** `golish-agent-kit/src/harness/graph_engine/{mod,graph,executor,checkpoint,error}.rs`：vendor metalcraft（含其测试）。
- **创建** `golish-agent-kit/src/harness/operation_mermaid.rs`：把 base_operation_graph 投影后导出 Mermaid（借 to_mermaid 写法，适配 StageKind）。
- **修改** `golish-db/src/repo/stage_runs.rs`：确认现有签名够用（够）。
- **修改** `golish-agent-kit/src/db_traits/repo.rs`：加 `stage_run_insert` / `stage_run_mark_terminal` / `operation_state_write_state_blob`（默认 no-op）。
- **修改** `golish-agent-kit/src/db_shim.rs`：加 `stage_runs` + `operation_state::write_state_blob` 包装。
- **修改** `golish-agent-app/src/ai/db_bridge/{mod,orchestration}.rs`：实现上述方法（调 golish_db::repo::{stage_runs,operation_state}）。
- **修改** `task_orchestrator/orchestrator.rs`：① run() 在 operation_state insert 后写首个 stage_run + state_blob；② resume() 读 operation_state 恢复 current_stage → 重建 queue 的 harness_stage（不再恒 None）。
- **修改** `task_orchestrator/subtask_phases/execute.rs::drive_stage_transition`：stage 推进时 mark_terminal(旧 stage_run) + insert(新 stage_run) + write_state_blob。

---

## 3. Tasks

### Task 1 · Vendor metalcraft 图引擎
**文件：** 创建 `harness/graph_engine/{mod,error,graph,executor,checkpoint}.rs`
**步骤：** 复制 `/tmp/refs/metalcraft/src/{graph,executor,checkpoint}.rs` 内容；新建本地 `error.rs`（`GraphError`(NoEntryPoint/NodeNotFound/NoEdge/Node{node,message}/Checkpoint/StepLimitExceeded) + `pub type Result`）；`mod.rs` 头加 MIT 出处注释 + `pub mod` 四子模块 + re-export；改 `use crate::error::*` → `use super::error::*`；保留三文件自带 `#[cfg(test)]`。
**确认点：** metalcraft `executor.rs` 引 `futures`/`tokio`/`tracing`/`async-trait`/`tokio-stream`——确认 golish-agent-kit Cargo.toml 已有（futures/tokio/tracing/async-trait 有；`tokio-stream` 若无则加 workspace 版或删 `stream()` 方法）。
**验证：** `cargo nextest run -p golish-agent-kit -E 'test(graph_engine)'` → metalcraft 自带测试全绿（确定性并行/篡改/resume）。

### Task 2 · Operation DAG → Mermaid（「图」可视）
**文件：** 创建 `harness/operation_mermaid.rs`
**步骤：** 写 `pub fn operation_dag_mermaid(profile: &Profile) -> String`：取 `base_operation_graph().project(profile.allowed_stage_set())`，遍历节点/边输出 `flowchart TD`（借 metalcraft to_mermaid 写法）。供 Tauri command / UI 展示。
**确认点：** `base_operation_graph()` 返回类型 + `project()` 签名（看 `operation_graph.rs`）；边/分支结构如何枚举。
**验证：** `cargo nextest -p golish-agent-kit -E 'test(operation_mermaid)'` → assessment DAG 的 mermaid 含 `external_attack_surface --> enumeration` 等预期边。

### Task 3 · DbRepoProvider + db_shim + app 实现：stage_runs / state_blob
**文件：** `db_traits/repo.rs` + `db_shim.rs` + `db_bridge/{mod,orchestration}.rs`
**步骤：** trait 加（默认 no-op）：
```rust
async fn stage_run_insert(&self, id: Uuid, operation_id: Uuid, stage_kind: &str) -> anyhow::Result<()> { let _=(id,operation_id,stage_kind); Ok(()) }
async fn stage_run_mark_terminal(&self, id: Uuid, status: &str) -> anyhow::Result<()> { let _=(id,status); Ok(()) }
async fn operation_state_write_state_blob(&self, operation_id: Uuid, blob: serde_json::Value) -> anyhow::Result<()> { let _=(operation_id,blob); Ok(()) }
```
db_shim 加 `stage_runs::{insert,mark_terminal}` + `operation_state::write_state_blob` 包装；GolishDbRepoProvider 实现调 `golish_db::repo::{stage_runs,operation_state}`。
**确认点：** 镜像现有 `operation_state_insert` 在 db_bridge/orchestration.rs 的 `_impl` 写法。
**验证：** `cargo check -p golish-agent-kit -p golish-agent-app` → exit 0。

### Task 4 · HarnessResumeState + 写检查点
**文件：** `task_orchestrator/` 新 `harness_resume.rs`（或 types.rs）+ orchestrator/execute 接线
**步骤：** 定义 `#[derive(Serialize,Deserialize)] HarnessResumeState { profile:String, current_stage:String, queue_titles:Vec<String>, completed_count:usize, schema_v:u32 }`；在 run() 生成 queue 后 + drive_stage_transition 推进后，`repo.operation_state_write_state_blob(task_id, serde_json::to_value(state)?)`。stage 起止写 stage_run（insert(uuid,op,stage) / mark_terminal(uuid,"completed")）。
**确认点：** stage_run 的 uuid 生命周期——存进 state_blob 或 operation_state 以便 mark_terminal 找回；MVP 可每 stage 即时 insert+随 transition mark。
**验证：** serde round-trip 单测 + MemRepo 断言 write_state_blob/stage_run_insert 被调。

### Task 5 · resume 恢复 harness 上下文
**文件：** `orchestrator.rs::resume`
**步骤：** resume() 读 `operation_state_get(task_id)`；若存在且 stage_mode：从 state_blob 反序列化 `HarnessResumeState`，按 `queue_titles` 重建 queue 并用 `infer_harness_stage(title+desc)`（harness_backfill）回填 `harness_stage`（替代现在的 `None`），从 `completed_count` 续跑。
**确认点：** `infer_harness_stage` 路径（harness_backfill.rs，本会话已读）；resume 当前 `harness_stage=None` 行（orchestrator.rs:226）。
**验证：** MemRepo 集成测试——预置 operation_state(stage,state_blob) → resume → 断言重建 queue 的 harness_stage 非 None + 从正确位置续。

### Task 6 · 集中编译 + 测试 + 验收
**步骤：**
1. `cargo check -p golish-db -p golish-agent-kit -p golish-agent-app` → 全 exit 0
2. `cargo nextest -p golish-agent-kit -E 'test(graph_engine)'` + `-E 'test(harness)'` → 全绿
3. `cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings` → exit 0
4. `cargo fmt --check` → 净
5. **活体（用户）**：`GOLISH_HARNESS_STAGE_MODE=true just dev` 跑多 stage task → 中途 `just kill` → 重启 + resume → 从上次 stage 续（stage_runs 有行 / operation_state.state_blob 非空 / current_stage 正确）。
**验收（全满足才算 P1 done）：** 1-4 全绿 + 5 活体证据进 agent-progress.md。

---

## 4. 自检（writing-plans）
- **规格覆盖**：覆盖设计 §2.2 P1（vendor metalcraft + checkpointer + 分支条件路由 + 断点续跑）+ §5 P1（stage_runs 写行 + state_blob + resume）。Shape A 全替换标为远期。
- **类型一致**：`HarnessResumeState`（Task 4）= resume 反序列化对象（Task 5）；stage_run uuid 跨 insert/mark_terminal 一致（Task 4 确认点）。
- **确认点**：每 Task 标了实现前必读的真实签名（base_operation_graph/project、operation_state _impl 写法、infer_harness_stage、resume None 行）。
- **依赖顺序**：T1(vendor) 独立；T3(桥接) 是 T4/T5 前置；T2(mermaid) 独立可并。
