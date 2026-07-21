# Stage Team Rolling Refill 锁收敛实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 保证 rolling StageTeam scheduler 在 refill claim等待 child completion 所持数据库锁时仍持续 poll该 child，从而消除 Vuln Triage及其他 Company阶段的调度自锁。

**架构：** 在现有 `drain_rolling_stage_team_work` 内保留一个持久 pending-claim队列，并用偏向 child completion的 `tokio::select!` 同时推进 child与claim。`None`只暂停到下一 child进展；错误、取消和landing边界保持原合同。

**技术栈：** Rust 2021、Tokio、`futures::stream::FuturesUnordered`、Cargo nextest。

## 文件结构

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：RED/GREEN回归测试与最小 scheduler状态机修复。
- 更新 `docs/modules/backend/golish-agent-runtime.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/INDEX.md`：记录 claim/completion并发轮询合同。
- 更新 `feature_list.json`、`agent-progress.md`：唯一 active feature与可重放证据。

共享工作树已有未提交改动，本计划不自动 stage/commit，避免把其它任务混入提交；只有用户另行要求时才提交。

## 任务 1：写出稳定复现自锁的 RED 测试

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 在现有 rolling-drain Tokio测试组增加 `rolling_stage_team_child_drain_polls_landing_while_refill_claim_waits`。
2. 测试使用 `Semaphore(1)` 作为 operation row lock，并用 `Notify` 固定顺序：work 1持锁；work 2确认后完成；work 3 claim先通知work 1再等待锁。
3. 核心断言如下：

```rust
let completed = timeout(
    Duration::from_secs(1),
    drain_rolling_stage_team_work(2, claim, execute, || false),
)
.await
.expect("refill waiting on a child-held lock must not stall child polling")
.expect("all fake children should land");
assert_eq!(completed, 3);
assert_eq!(*claimed.lock().unwrap(), vec![1, 2, 3]);
assert_eq!(operation_lock.available_permits(), 1);
```

4. 运行 RED：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(rolling_stage_team_child_drain_polls_landing_while_refill_claim_waits)' --status-level fail
```

预期旧算法 exit 100，测试因1秒 timeout失败；失败必须发生在 refill等待锁且持锁 child未再被 poll，不接受编译错误作为RED证据。

## 任务 2：最小实现持久 claim与并发轮询

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 在 driver中新增：

```rust
let mut pending_claims = FuturesUnordered::new();
let mut claim_paused_after_none = false;
```

2. 仅当无terminal error、未取消、pending claim为空、未因`None`暂停且child未到cap时，push一个 `claim(claim_sequence)`；claim不得spawn或并行。
3. child与claim同时存在时使用：

```rust
tokio::select! {
    biased;
    child = in_flight.next(), if !in_flight.is_empty() => { /* record + unpause */ }
    claimed = pending_claims.next(), if !pending_claims.is_empty() => { /* Some/None/Err */ }
}
```

4. `Some(work)`才递增sequence并push execute；`None`暂停到child completion；claim error只记录首个terminal error。取消后不再创建claim，但pending claim若已成功claim worker仍须执行，所有started child排空后才返回。
5. 重跑任务1命令，预期1/1 GREEN。

## 任务 3：回归合同与静态检查

**文件：** 只验证 `stage_run_call.rs` 及受影响 crate。

1. 运行全部rolling-drain与关联worklist回归：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(rolling_stage_team_child_drain_) | test(stage_team_child_batch_preserves_error_without_skipping_recoverable_results) | test(persisted_vuln_worklist_distinguishes_in_flight_from_exhausted)'
```

2. 运行 scoped静态验证：

```bash
just space-guard
cd backend && cargo clippy -p golish-agent-runtime --lib --tests -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml -p golish-agent-runtime -- --check
```

预期全部 exit 0、零warning、格式无diff。不得运行未获授权的init/precommit/全workspace门禁。

## 任务 4：模块卡和证据收尾

**文件：** 更新三处模块文档、`feature_list.json`、`agent-progress.md`。

1. 记录“child completion与至多一个pending claim必须并发poll；`None`只能暂停至下一child进展”的合同。
2. 把RED/GREEN run id、测试数、Clippy/rustfmt和JSON/diff证据写入progress与feature evidence。
3. 运行：

```bash
jq empty feature_list.json
test "$(jq '[.features[] | select(.status == \"in_progress\")] | length' feature_list.json)" -eq 0
git diff --check
```

预期全部 exit 0。只声明代码路径的定向修复；未重启真实GUI时明确保留live recovery未执行。
