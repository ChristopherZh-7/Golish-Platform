# Vuln 扫描预算耗尽终态实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 single-technique Nuclei 在最大预算与有界重试耗尽后落 evidence-backed `blocked`，同时为已经停在 attempt 3 的 durable operation提供一次真实 attempt-4恢复，消除 `partial + no shard` 永久死路。
**架构：** `golish-agent-runtime`只为 exact scan-budget exhaustion创建版本化 budget-recovery shard并固定600秒；`golish-pentest-app`严格认证 foreground timeout witness，在最终可信 deadline后由 wrapper先写 evidence、再 guarded发布 `BudgetBlocked` outcome。Gate不放宽，其他 scanner/runtime故障继续partial。
**技术栈：** Rust 2021、Tokio、SQLx、cargo-nextest。

## 文件结构

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`：增加 `budget_recovery` shard和旧 attempt-3精确迁移规则。
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：固定 primary/narrowed/budget-recovery的300/600/600秒server预算。
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`：认证 timeout witness、计算稳定预算耗尽与生成 BudgetBlocked。
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/{nuclei.rs,landing.rs}`：新增 completion variant并 evidence-first落 blocked。
- 更新 `docs/modules/backend/golish-agent-runtime{,/agentic_loop}.md`、`docs/modules/backend/golish-pentest-app{,/pentest_bridge}.md` 与 `docs/modules/INDEX.md`：记录公开行为和安全边界。
- 更新 `feature_list.json` 与 `agent-progress.md`：保存定向验证证据和剩余真实运行风险。

## 任务 1：锁定 runtime 的唯一 budget recovery

**文件：** 修改 `stage_team_scheduler.rs`、`stage_run_call.rs`。

1. 先增加失败测试：

```rust
#[test]
fn vuln_worklist_shard_reopens_one_legacy_scan_budget_exhaustion_at_max_budget() {
    // attempt=3 + retry=false + scanner_runtime/scan_budget_exhausted
    // => exactly one BudgetRecovery shard, recovery_attempt=4.
}

#[test]
fn vuln_worklist_shard_does_not_reopen_other_exhausted_runtime_failures() {
    // runner_failure / operator_cancelled / attempt 4 => no shard.
}

#[test]
fn server_vuln_formulaic_timeout_uses_full_budget_for_recovery_shapes() {
    assert_eq!(server_vuln_formulaic_timeout_secs("primary").unwrap(), 300);
    assert_eq!(server_vuln_formulaic_timeout_secs("narrowed").unwrap(), 600);
    assert_eq!(server_vuln_formulaic_timeout_secs("budget_recovery").unwrap(), 600);
}
```

2. 每次 Cargo前运行 `just space-guard`，再运行：

```bash
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(vuln_worklist_shard_) | test(server_vuln_formulaic_timeout_uses_full_budget_for_recovery_shapes)'
```

预期 RED：旧实现返回0个 shard、没有 `BudgetRecovery`/timeout helper或仍给 narrowed 180秒。

3. 最小实现：
   - 增加 `VulnShardShape::BudgetRecovery`；
   - 只在 exact details 为 attempt3、retry=false、owner=`scanner_runtime`、class=`scan_budget_exhausted` 时生成 attempt4；
   - attempt4、其他 failure与普通 cap保持停止；
   - shape进入 stable key/objective，解析只接受三个固定literal；预算helper返回300/600/600。
4. 重跑相同命令，预期GREEN。

## 任务 2：锁定可信 deadline 与 BudgetBlocked

**文件：** 修改 `vuln_capabilities.rs`、`vuln_adapters/nuclei.rs`、`vuln_adapters/landing.rs`。

1. 先增加失败测试：

```rust
#[test]
fn authoritative_foreground_timeout_is_budget_exhausted_but_plain_exit_124_is_runner_failure() {}

#[test]
fn third_consecutive_max_budget_deadline_becomes_producer_blocked() {}

#[test]
fn fourth_max_budget_deadline_after_legacy_exhaustion_becomes_producer_blocked() {}

#[test]
fn runner_config_mixed_and_duplicate_failures_never_become_budget_blocked() {}

#[test]
fn budget_blocked_maps_to_blocked_while_error_maps_to_partial() {}
```

2. RED命令：

```bash
just space-guard
cd backend && cargo nextest run -p golish-pentest-app -E 'test(authoritative_foreground_timeout_) | test(third_consecutive_max_budget_) | test(fourth_max_budget_) | test(runner_config_mixed_) | test(budget_blocked_maps_)'
```

3. 最小实现：
   - `NucleiFailureAttribution`增加可选 `failure_witness`，只有trusted timeout tuple设置 `foreground_wrapper_deadline_v1`；
   - history按distinct generation和现有exact SQL scope计算连续同类预算失败；当前必须有witness且`timeout_secs=600`；
   - 新增 `scanner_budget_blocked` metadata和 `NucleiCompletion::BudgetBlocked`，不要复用target transport；
   - landing把BudgetBlocked映射为blocked，在raw payload保留witness，网络尝试保持true；
   - landing/authority/CAS失败不把wrapper标为complete。
4. 重跑相同命令，预期GREEN；同时重跑既有target transport/operator cancel测试。

## 任务 3：Gate不变量与静态检查

**文件：** 不改Gate生产逻辑，只运行回归；若编译要求仅补exhaustive enum match。

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(vuln_triage_accepts_only_backend_owned_negative_terminals) | test(vuln_triage_error_or_partial_marker_never_closes_a_cell) | test(vuln_triage_denominator_blocks_partial_and_passes_when_full)'
just space-guard
cd backend && cargo clippy -p golish-agent-runtime -p golish-pentest-app --lib --tests -- -D warnings
cd backend && cargo fmt -p golish-agent-runtime -p golish-pentest-app -- --check
```

预期：Gate继续只接受可信backend terminal；两crate Clippy零warning、格式干净。

## 任务 4：文档与证据收尾

1. 更新四张模块卡与INDEX：scanner/runtime一般错误仍partial，只有可信max-budget deadline可以producer-blocked；记录旧attempt3唯一恢复。
2. 更新feature的用户行为、design/plan/verification/evidence，整条feature仍保持`in_progress`直到真实同operation继续后Gate PASS。
3. 最终检查：

```bash
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) == 1' feature_list.json
git diff --check -- backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/landing.rs docs/design/2026-07-20-vuln-scan-budget-terminalization.md docs/superpowers/plans/2026-07-20-vuln-scan-budget-terminalization.md docs/modules/backend/golish-agent-runtime.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-pentest-app.md docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/INDEX.md feature_list.json agent-progress.md
```

不运行未获授权的init/precommit/全workspace测试，不触发真实外部扫描，不直接修改当前live DB。

## 自检

- 规格覆盖：当前179/180 run可产生一次新可信执行；成功落found/empty，最终max-budget timeout落blocked；普通scanner/runtime故障仍fail closed。
- 占位内容扫描：所有任务都有精确文件、测试、实现条件与验证命令，无TODO/TBD。
- 类型一致性：`BudgetRecovery`只存在runtime shard层，`BudgetBlocked`只存在producer completion层；二者通过真实wrapper执行与evidence连接，不共享模型可写参数。
