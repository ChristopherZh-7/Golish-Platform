# Stage Team 滚动补位实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让所有 Company Controller Stage Team 在任意 child完成后立即补领下一个 durable WorkItem，同时保持所有现有并发、安全、scope、retry与evidence上限不变。

**架构：** 在 `stage_run_call.rs` 内提取一个小型泛型 rolling drain driver，以 `FuturesUnordered`管理调用方拥有的 futures；生产 `drain_company_controller_children`只提供 durable claim与现有 child executor闭包。driver统一保留 first-error-after-drain、claim/cancel stop-refill和in-flight落稳语义。

**技术栈：** Rust 2021、Tokio、futures、Cargo nextest。

## 文件结构

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：rolling driver、共享生产接线与单元测试。
- 更新 `docs/modules/backend/golish-agent-runtime.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/INDEX.md`：记录全部 Stage Team共享的rolling refill契约。
- 更新 `agent-progress.md`、`feature_list.json`：记录RED/GREEN命令、证据与功能状态。

## 任务 1：用测试锁定即时补位

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 在现有 `tests` 模块新增 `rolling_stage_team_child_drain_refills_before_slow_sibling_finishes`。fake claim依次返回 `1,2,3,None`；child 1立即完成，child 2等待 `Notify`，child 3启动时发通知。测试必须在释放 child 2前观察到 child 3启动，最终完成数为3。
2. 运行 RED：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(rolling_stage_team_child_drain_refills_before_slow_sibling_finishes)'
```

预期旧实现缺少 rolling driver，测试编译失败或断言第三项未即时启动；不得通过修改等待时长掩盖批次行为。

## 任务 2：实现最小 rolling driver并接入共享 drain

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 把 import 从 `future::join_all` 改为 `stream::FuturesUnordered`。
2. 新增泛型 `drain_rolling_stage_team_work`，签名接受 `concurrency`、async claim闭包、async execute闭包与 cancellation predicate，返回完成数量或排空后的第一项错误。
3. driver循环必须实现以下核心顺序：

```rust
while terminal_error.is_none() && in_flight.len() < concurrency {
    match claim(claim_sequence).await {
        Ok(Some(work)) => in_flight.push(execute(work)),
        Ok(None) => break,
        Err(error) => terminal_error = Some(error),
    }
}
if in_flight.is_empty() {
    return terminal_error.or(first_execution_error).map_or(Ok(completed), Err);
}
record(in_flight.next().await.expect("non-empty rolling drain"));
```

4. `drain_company_controller_children`用现有 `claim_stage_work_item`/`bind_claimed_stage_team_worker`作为claim闭包，用现有provider permit + `execute_stage_team_child`作为execute闭包；lease owner改为单调 `:child:<claim_sequence>`，不改变任何durable业务identity。
5. 重跑任务1命令，预期1/1 GREEN。

## 任务 3：锁定上限与错误排空

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 新增 `rolling_stage_team_child_drain_never_exceeds_cap`，用atomic active/peak计数验证大量fake work下peak精确不超过2。
2. 新增 `rolling_stage_team_child_drain_records_error_and_finishes_siblings`，让一个child返回错误、后续child成功，断言所有claimable work都执行且最终返回第一项错误。
3. 新增 `rolling_stage_team_child_drain_stops_refill_on_cancel_but_awaits_started_work`，取消后断言未claim新work，已启动child全部完成后才返回cancel错误。
4. 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(rolling_stage_team_child_drain_) | test(stage_team_child_batch_preserves_error_without_skipping_recoverable_results)'
```

预期所有目标测试通过。

## 任务 4：定向静态验证与文档收尾

**文件：** 更新两张runtime模块卡、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`

1. 记录 rolling refill适用于 Target Intel/EAS/Enumeration/Vuln，且上限、provider semaphore与evidence边界不变。
2. 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(rolling_stage_team_child_drain_) | test(stage_team_child_batch_preserves_error_without_skipping_recoverable_results) | test(persisted_vuln_worklist_distinguishes_in_flight_from_exhausted)'
just space-guard
cd backend && cargo clippy -p golish-agent-runtime --lib --tests -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml -p golish-agent-runtime -- --check
jq empty feature_list.json && test "$(jq '[.features[] | select(.status == \"in_progress\")] | length' feature_list.json)" -eq 1 && git diff --check
```

3. 不运行init/precommit/全workspace测试，不重启或打断当前真实scan。未做真实新binary运行时，明确保留集成风险，不把Vuln整体CLI feature标为passing。
