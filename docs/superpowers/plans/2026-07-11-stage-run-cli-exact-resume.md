# Headless stage-run exact resume 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 增加一个 fail-closed 的 `--stage-run-resume` CLI 入口，在新进程中复用旧 session/task/operation 和 exact worker chain。

**架构：** `golish` CLI 解析 selector 后仍使用现有 stage-run 后端启动流程，但恢复分支从 DB 解析唯一 waiting operation，或在显式 orphan-running 授权和 expected ids 全匹配时接受残留 running operation。它取得 operation-scoped PostgreSQL advisory lock；若首 stage 中断只留下 flat HarnessResumeState，则在显式 repair flag 下 CAS 合成最小 graph_flow 且保留所有 sibling keys；锁后重新读取校验 checkpoint 与 chain 所有权，然后调用 `TaskOrchestrator::resume`。恢复复用原 chat key、DB session UUID、profile、org、stage freshness 和 transcript，不创建 task/operation。

**技术栈：** Rust 2021、clap、sqlx、Tokio、现有 `TaskOrchestrator`/`BridgeAgentExecutor`。

## 文件结构

- `backend/crates/golish/src/cli/args.rs`：声明并测试恢复参数及冲突关系。
- `backend/crates/golish/src/main.rs`：把恢复参数路由到 stage-run headless bootstrap。
- `backend/crates/golish/src/stage_run/mod.rs`：解析恢复目标、纯 fail-closed 校验、chain DB 所有权检查、恢复编排。
- `docs/modules/backend/golish/cli.md`：记录 CLI 命令契约。
- `docs/modules/backend/golish/stage_run.md`：记录恢复身份和安全边界。
- `docs/modules/INDEX.md`：同步模块卡状态说明。

## 任务 1：CLI 参数 RED→GREEN

**文件：** `backend/crates/golish/src/cli/args.rs`、`backend/crates/golish/src/main.rs`

**步骤 1：编写失败测试**

加入测试，期望下面的解析 API 存在：

```rust
let args = Args::parse_from([
    "golish",
    "--stage-run-resume",
    "stage-run-abc",
    "-e",
    "继续",
    "/tmp/workspace",
]);
assert_eq!(args.stage_run_resume.as_deref(), Some("stage-run-abc"));
```

再加入冲突测试：`--stage-run-resume` 必须拒绝 `--stage-run`、`--only` 和
`--ephemeral-db`。

加入孤儿恢复参数测试：

```rust
let args = Args::parse_from([
    "golish", "--stage-run-resume", "stage-run-abc",
    "--allow-orphan-running",
    "--repair-missing-graph-flow",
    "--expect-session", SESSION_UUID,
    "--expect-task", OPERATION_UUID,
    "--expect-operation", OPERATION_UUID,
    "--expect-org", ORG_UUID,
    "--expect-stage", "enumeration",
]);
assert!(args.allow_orphan_running);
```

**步骤 2：验证 RED**

```bash
cd backend && cargo test -p golish cli::args::tests::test_args_stage_run_resume --lib
```

预期：编译失败，`Args` 尚无 `stage_run_resume` 字段。

**步骤 3：最小实现**

在 `Args` 中加入：

```rust
#[arg(
    long,
    value_name = "SESSION_OR_OPERATION",
    conflicts_with_all = [
        "stage_run", "ephemeral_db", "keep_ephemeral_db", "profile", "from",
        "to", "only", "org", "target", "include_subsidiaries"
    ]
)]
pub stage_run_resume: Option<String>,

#[arg(long, requires = "stage_run_resume")]
pub allow_orphan_running: bool,
#[arg(long, requires = "stage_run_resume")]
pub repair_missing_graph_flow: bool,
#[arg(long, requires = "stage_run_resume")]
pub repair_reaped_task: bool,
#[arg(long, requires = "stage_run_resume")]
pub expect_session: Option<uuid::Uuid>,
#[arg(long, requires = "stage_run_resume")]
pub expect_task: Option<uuid::Uuid>,
#[arg(long, requires = "stage_run_resume")]
pub expect_operation: Option<uuid::Uuid>,
#[arg(long, requires = "stage_run_resume")]
pub expect_org: Option<uuid::Uuid>,
#[arg(long, requires = "stage_run_resume")]
pub expect_stage: Option<String>,
```

`main.rs` 的 stage-run 分支在 `args.stage_run || args.stage_run_resume.is_some()`
时进入同一个 headless runtime。

**步骤 4：验证 GREEN**

```bash
cd backend && cargo test -p golish cli::args::tests::test_args_stage_run_resume --lib
```

预期：恢复参数与冲突测试通过。

## 任务 2：恢复候选校验 RED→GREEN

**文件：** `backend/crates/golish/src/stage_run/mod.rs`

**步骤 1：编写失败测试**

声明测试期望的纯函数接口：

```rust
let validated = validate_resume_candidate(&candidate).expect("valid waiting resume");
assert_eq!(validated.operation_id, candidate.task_id);
assert_eq!(validated.stage, StageKind::Enumeration);
```

分别断言未授权的 `running` task、缺任一 expected identity 的 orphan-running、expected
identity 不匹配、session/task 不匹配、operation/task 不匹配、
superseded operation、缺 graph checkpoint、`next_node != current_stage`、缺
worker chain、worker session/task/agent 不匹配均返回错误。

为首 stage mid-node 中断增加独立测试：缺 `graph_flow` 默认拒绝；显式 repair +
全量 expected identities 时返回 `needs_graph_repair=true`；flat profile/stage/run id
任一缺失或漂移都拒绝。

**步骤 2：验证 RED**

```bash
cd backend && cargo test -p golish stage_run::tests::resume_candidate --lib
```

预期：编译失败，恢复候选类型与校验函数尚不存在。

**步骤 3：最小实现**

增加私有 `ResumeCandidate`、`ResumeWorkerRef`、`ValidatedResumeTarget`，以及：

```rust
fn validate_resume_candidate(candidate: &ResumeCandidate) -> Result<ValidatedResumeTarget>
```

函数只做确定性字段校验，不访问 DB，不放宽任何缺失状态。

**步骤 4：验证 GREEN**

```bash
cd backend && cargo test -p golish stage_run::tests::resume_candidate --lib
```

预期：所有接受/拒绝矩阵通过。

## 任务 3：DB selector 与 chain scope RED→GREEN

**文件：** `backend/crates/golish/src/stage_run/mod.rs`

**步骤 1：编写失败测试**

为 selector 分类和 SQL 形状增加无 DB 测试：

```rust
assert_eq!(classify_resume_selector("stage-run-abc"), ResumeSelector::ChatKey);
assert_eq!(classify_resume_selector(OP_UUID), ResumeSelector::Uuid(_));
assert!(EXACT_RESUME_CHAIN_SQL.contains("session_id = $2"));
assert!(EXACT_RESUME_CHAIN_SQL.contains("task_id IS NULL OR task_id = $3"));
assert!(EXACT_RESUME_CHAIN_SQL.contains("agent = $4::agent_type"));
```

**步骤 2：验证 RED**

```bash
cd backend && cargo test -p golish stage_run::tests::resume_selector --lib
```

预期：编译失败，selector 与 SQL 尚不存在。

**步骤 3：最小实现**

实现 selector 解析、锁前候选读取和锁后同 identity 重读：

```rust
async fn resolve_stage_run_resume_target(
    pool: &sqlx::PgPool,
    selector: &str,
) -> Result<ResolvedResumeTarget>
```

chat key/DB session 仅接受唯一 waiting checkpoint；operation UUID 直接绑定
task。显式 orphan-running 仅在三项 expected UUID 全匹配时接受。解析
`stage_run_workers[current_stage]` 后，对每个 chain 执行 exact
session+agent 查询并要求 chain body 非空；`task_id=Some` 时还必须等于 operation，
兼容旧 stage-run 的 `task_id=NULL`（由 operation_state 内的 exact map 提供 operation
绑定，不手工回填），再交给纯校验函数。

增加 operation-scoped claim：

```rust
async fn try_claim_stage_run_resume(
    pool: &sqlx::PgPool,
    operation_id: uuid::Uuid,
) -> Result<StageRunResumeClaim>
```

从 pool acquire 后 `detach()` 专用 `PgConnection`，执行
`pg_try_advisory_lock(key_hi, key_lo)`；失败立即返回 busy。claim 持有整段 resume，
显式 unlock 后关闭连接；异常 drop 关闭 detached connection，Postgres 自动释放锁。

增加纯 JSON helper 和 CAS SQL：

```rust
fn synthesize_graph_flow_checkpoint(
    state_blob: serde_json::Value,
    current_stage: StageKind,
) -> Result<serde_json::Value>
```

测试输入含 `stage_run_workers`、`route_probe_checkpoints` 和未知 sibling；输出只新增
`graph_flow.state`/`graph_flow.next_node`，所有 sibling 深度相等。SQL 必须包含
`jsonb_set`、`graph_flow IS NULL`、operation/current_stage/superseded guards。

**步骤 4：验证 GREEN**

```bash
cd backend && cargo test -p golish stage_run::tests::resume_selector --lib
```

预期：selector、SQL scope、orphan expected-id 与 advisory-lock key 测试通过。

## 任务 4：恢复编排 RED→GREEN

**文件：** `backend/crates/golish/src/stage_run/mod.rs`

**步骤 1：编写失败测试**

把恢复动作选择抽成纯枚举，测试恢复分支只能选择 `Resume`：

```rust
assert_eq!(stage_run_action(Some("stage-run-abc")), StageRunAction::Resume);
assert_eq!(stage_run_action(None), StageRunAction::Fresh);
```

并以源代码级断言保护恢复函数包含 `.resume(` 且不包含 `.run_stage(`。

**步骤 2：验证 RED**

```bash
cd backend && cargo test -p golish stage_run::tests::resume_action --lib
```

预期：编译失败，恢复动作/编排函数尚不存在。

**步骤 3：最小实现**

在公共 bootstrap 中：

```rust
match resume_target {
    Some(target) => orchestrate_resume(
        &bridge,
        &db_pool,
        &target,
        args.execute.as_deref().unwrap_or("继续"),
        args.auto_approve,
    ).await,
    None => orchestrate(/* existing fresh path */).await,
}
```

`orchestrate_resume` 获取新 top-level request，设置旧 profile/chat session/org、
当前 stage 单阶段 allowlist、`set_force_stage_run_on_resume_once(true)`，然后只调用：

```rust
orchestrator.resume(target.operation_id, continuation, &executor).await
```

**步骤 4：验证 GREEN**

```bash
cd backend && cargo test -p golish stage_run::tests::resume_action --lib
```

预期：恢复分支选择和调用边界测试通过。

## 任务 5：文档与定向验证

**文件：** `docs/modules/backend/golish/cli.md`、`docs/modules/backend/golish/stage_run.md`、`docs/modules/INDEX.md`

**步骤 1：同步模块卡**

记录命令示例、selector 规则、waiting-only、exact chain scope、同 stage
freshness 保持，以及 `--replay` 仍然只读。

**步骤 2：格式化并跑定向测试**

```bash
cd backend && cargo fmt --all -- --check
cd backend && cargo test -p golish cli::args::tests --lib
cd backend && cargo test -p golish stage_run::tests --lib
```

预期：全部通过且无 warning。

**步骤 3：scoped clippy**

```bash
cd backend && cargo clippy -p golish --all-targets -- -D warnings
```

预期：退出码 0。

**步骤 4：安全复核**

确认验证过程没有执行 `--stage-run-resume`，没有连接/写入 live DB，没有修改
`feature_list.json` 或 `agent-progress.md`，并记录最终可执行命令：

```bash
./target/debug/golish \
  --stage-run-resume stage-run-476558c3-c22a-4009-a82e-17e086a005de \
  --allow-orphan-running \
  --repair-missing-graph-flow \
  --repair-reaped-task \
  --expect-session a15c0b0f-23ff-42f9-b950-7dcaf25de860 \
  --expect-task 462b6c9f-2a0d-48af-8ff0-8b5c08416196 \
  --expect-operation 462b6c9f-2a0d-48af-8ff0-8b5c08416196 \
  --expect-org 0a431390-7726-48e5-b0a8-e692a9070e33 \
  --expect-stage enumeration \
  -e "继续" \
  /Users/christopherzheng/golish-platform/Test1
```

**提交：** 本任务不自动 commit；由主会话统一确认工作树范围后提交。

## 任务 6：首 stage flat checkpoint 与 startup reaper 兼容

**文件：** `backend/crates/golish-db/src/repo/tasks.rs`、
`backend/crates/golish/src/{cli/args.rs,stage_run/mod.rs}`

**步骤 1：补 RED**

断言 task reaper 的 recoverable predicate 不只识别完整 `graph_flow`，还要求 flat
checkpoint 的 profile/current_stage 一致、非 nil `current_stage_run_id`、
`completed_count=0` 和当前 stage 的非空 worker map；不完整 flat JSON 仍由 fail
reaper 处理。另断言普通 failed task 即使传 repair flag 也拒绝，只有固定
startup-reaper abandoned marker + 完整 expected identities 返回
`needs_task_repair=true`。

**步骤 2：实现 guarded repair**

startup reaper 将完整 flat first-stage orphan pause 为 `waiting`。对已被旧二进制
标记 failed 的历史行，新增显式 `--repair-reaped-task`；取得 operation advisory
lock 后以 task/session/status/result/update-time + operation profile/stage/org/state
全等 CAS 恢复为 `waiting`，清除 synthetic abandoned result，再执行 graph CAS 并
全量重读。任何 provider/tool/普通 failed result 不得恢复。

**步骤 3：验证**

```bash
CARGO_INCREMENTAL=0 cargo test -p golish-db repo::tasks --lib
CARGO_INCREMENTAL=0 cargo test -p golish stage_run::tests::resume_candidate --lib
CARGO_INCREMENTAL=0 cargo clippy -p golish-db -p golish --all-targets -- -D warnings
```
