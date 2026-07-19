# 仅 Scoping 保留常规人工确认实现计划

**目标：** Scoping 之后的普通 stage 在确定性 Gate PASS 后自动推进，不再等待通用
phase-boundary confirmation；精确目标、Candidate、工具授权和 Gate BLOCK 保持不变。

**架构：** 在 `TaskOrchestrator::two_level_phase_gate` 的专用安全 barrier 之后增加明确的
post-Scoping 自动推进规则。保留已有 generic approval 实现用于 Scoping/兼容路径，不改
profile、DAG、数据库或前端协议。

## 任务 1：用测试锁定 post-Scoping 自动推进

**文件：**
`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs`

1. 把 Enumeration → Vuln 的 fallback channel 用例改为无需回复即可 `Allowed`。
2. 断言不会发 `waiting_approval`。
3. 给 orchestrator 接入 `EventCoordinator`，断言 post-Scoping crossing 不发
   `AskHumanRequest`，也不会进入 human rework。
4. 保留 Target Intel exact target scope 与 Candidate review barrier 的既有回归。

预期 RED：旧实现等待 user/coordinator reply，测试超时或观察到 confirmation 事件。

## 任务 2：实现 Scoping-only 常规确认资格

**文件：**
`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

1. 在 Candidate 和 active-recon target-scope 专用 barrier 之后判断 `from_stage`。
2. `from_stage != Scoping` 直接返回 `PhaseGateDecision::Allowed`。
3. 保留 Gate BLOCK、无 successor、Candidate、target-scope 和 Scoping generic compatibility
   的既有顺序。
4. 更新函数注释，明确 routine confirmation 与 typed authorization 的边界。

## 任务 3：同步事实源

**文件：**

- `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- `docs/modules/backend/golish/stage_run.md`
- `docs/modules/INDEX.md`
- `feature_list.json`
- `agent-progress.md`

记录 Scoping-only 常规确认、保留的三类安全 barrier、CLI 兼容参数，以及定向验证证据。

## 任务 4：定向验证

每个 Cargo 命令前先运行 `just space-guard`，然后执行：

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(two_level_phase_gate) | test(active_recon_scope_review) | test(pre_eas) | test(direct_eas) | test(review_barrier) | test(runner_graph_interrupts_when_runner_reports_blocked_gate)' --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(harness_authz_rejects_scan_tool_not_in_allowed_types) | test(harness_authz_rejects_intent_above_ceiling)' --status-level fail
cd backend && cargo nextest run -p golish -E 'test(test_args_stage_run_accepts_explicit_phase_boundary_approval)' --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish --tests -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml -p golish-agent-kit -p golish -- --check
jq empty feature_list.json
git diff --check
```

不运行 `./init.sh`、`just precommit` 或全 workspace 测试，除非用户另行明确授权。
