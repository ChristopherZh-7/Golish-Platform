# Stage Team 重叠派工、begin fence 与 child drain 收敛实现计划

**目标：** 修复实体 Vuln Triage run 中重复 whole-company assignment、begin fence deadlock误判 lease loss、以及
child drain fail-fast 留下 queued retry 的组合停机。

**架构：** runtime 在 durable write 前拒绝 batch-local exact assignment overlap；golish-db 让 begin/finish 共用
SQLSTATE 闭集的有界事务 retry；runtime 只对 typed LeaseLost污染 begin bound；Controller drain先耗尽可领取 child
再传播未收敛错误。

**技术栈：** Rust 2021、Tokio、SQLx/PostgreSQL、cargo-nextest。

---

## Task 1：建立 RED tests

**文件：**

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/worker_tool_lifecycle.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

**步骤：**

1. 新增 router test：两个不同 dedupe key、相同 role/kind/objective/空 subjects 必须整体拒绝，repository requests=0。
2. 新增 retry-runner test：retryable 两次后成功，non-retryable一次即返回。
3. 新增 begin lease classifier test：`Storage` 不失 lease、`LeaseLost` 失 lease。
4. 新增 generic begin wrapper test：concrete lifecycle未报告 lease loss时保持 bound可落。
5. 新增 drain collector test：`Err + RetryScheduled` 同时保留错误与已处理计数。
6. 每次 Cargo 前执行 `just space-guard`，运行精确测试名并确认旧实现不满足新断言。

## Task 2：实现 assignment overlap 与 begin fence 收敛

1. canonicalization 后计算排序 subjects + normalized objective 的 batch-local identity；重复时返回
   `STAGE_TEAM_DISPATCH_ASSIGNMENT_OVERLAP`。
2. 抽出 worker-tool transaction retry runner，让 begin/finish 都从 `*_once` 闭包进入三次有界 retry。
3. `RuntimeWorkerToolLifecycle::begin` 只在 typed `LeaseLost` 时调用 `mark_lease_lost()`；finish 保持原行为。
4. generic sub-agent begin wrapper信任 concrete lifecycle的 typed lease state，不再二次无条件标记。
5. 运行 Task 1 精确测试确认 GREEN。

## Task 3：实现 non-fail-fast drain

1. 提取逐批结果 collector，记录首个 error但处理所有 sibling。
2. drain 持有跨 round 的 first error；每轮完成后继续 claim，直到无可领取 child。
3. 无可领取 child 时：有 error则返回它，无 error则返回 completed count。
4. 运行 stage-team child/drain 聚焦回归。

## Task 4：文档与定向验证

1. 同步 `golish-agent-runtime/agentic_loop`、`golish-db/repo` 模块卡和 INDEX 状态。
2. 更新 active parent feature 的 design/plan/evidence addendum；保持唯一 `in_progress`。
3. 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(stage_team_dispatch_rejects_semantically_overlapping_assignments) | test(begin_worker_tool_error_marks_only_typed_lease_loss) | test(stage_team_child_batch_preserves_error_without_skipping_recoverable_results)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(worker_tool_transaction_retry_runner)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(stage_team_dispatch) | test(stage_team_child)' --status-level fail
cd backend && cargo clippy -p golish-agent-runtime -p golish-db --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
jq empty feature_list.json
git diff --check
```

4. 不运行 `./init.sh`、`just precommit`、全 workspace tests；不触发 provider、扫描器或 live DB。
