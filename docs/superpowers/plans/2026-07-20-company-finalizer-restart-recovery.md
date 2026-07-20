# Company Controller finalizer 重启恢复实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 Company Controller 在 final seal 失败或重启后可确定性继续，并对当前已被错误标成 `failed/exhausted` 的 Vuln run 做保留事实的追加式自动恢复。

**架构：** 新失败在 exact finalizer fence 上原地 park/requeue；startup reaper 对已有 durable submission 的 finalizer 使用独立 closeout retry 语义。历史不可变终态不复活，而由 claim-time witness 驱动现有 no-purge checkpoint replacement，随后明确 halt 当前请求并在下一 Turn 使用新 execution。

**技术栈：** Rust 2021、sqlx/Postgres、async-trait、cargo nextest。

## 文件职责

- `backend/crates/golish-db/src/repo/stage_teams.rs`：识别 exact durable final submitter，隔离 producer attempt fuel。
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：finalization park 事务、legacy terminalized witness 与 no-purge rollover。
- `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`：sqlx-free finalizer park DTO/trait 合同。
- `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`：DTO 映射到具体 DB 事务。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：final seal error landing、runtime-recovered 分类与用户动作。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_dispatch.rs`：终止当前 top-level request。
- `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`：持久化状态机集成回归。
- 对应 `docs/modules/`、`feature_list.json`、`agent-progress.md`：同步合同与证据。

## 任务 1：先写 DB 失败回归

**文件：** `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

**步骤：**

1. 基于现有 Company Controller fixture，持久化 exact deliverable submission 后把 worker 设为过期且 `attempt_epoch > max_attempts`。
2. 调 `tasks::startup_reap_abandoned`，断言 worker/item 变为 `queued/queued`，没有新增 `stage_team_attempts_exhausted` output。
3. 模拟 final seal failure parking，断言 plan closed/pointer 不变、lease 清除，随后 `claim_stage_aggregator` 领回同 worker/message chain。
4. 构造历史 `failed/exhausted` + immutable lease-expiry output；再次 claim，断言返回 runtime-replaced code、旧 execution 保留为 failed、新 execution started、operation-scoped fact 数量不变。
5. 去掉 exact submission 的反例必须仍返回 `stage_team_final_submitter_not_replaceable`。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(final_submitter)' --status-level fail
```

预期：新用例在实现前失败；实现后全部通过，普通 producer exhaustion 回归仍通过。

## 任务 2：实现 DB closeout retry 与历史 rollover

**文件：** `backend/crates/golish-db/src/repo/stage_teams.rs`、`backend/crates/golish-db/src/repo/runtime_memory_tx.rs`

**步骤：**

1. 在 `reap_expired_clean_stage_worker` 内用 plan/item/worker + exact submission 查询计算 `finalizer_retry_scheduled`；该值为真时跳过 `work_item_max_attempts` terminalization。
2. 新增 `park_stage_team_finalizer_after_failure`，只接受 closed Company Controller 的 exact final submitter fence 和 exact deliverable submission；事务内把 item/worker 回到 queued 并清租约。
3. 给 `SupersedeStageCheckpointRow` 增加仅仓库内部使用的 optional recovery witness；在 replacement 事务持锁后验证 finalizer、item、immutable output、submission、row versions 和 barrier hash。
4. `claim_stage_aggregator` 仅在原错误为 `stage_team_final_submitter_not_replaceable` 且 witness 完整时调用 `fact_purge=None` 的 same-stage replacement；成功后返回 `stage_team_final_submitter_runtime_replaced`。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(final_submitter)|test(startup_reaper_exhausts_clean_team_worker)' --status-level fail
```

预期：finalizer recovery 绿，普通 producer 仍写 terminal blocked output。

## 任务 3：接通 runtime port 与错误 landing

**文件：** `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 增加 `ParkStageTeamFinalizerAfterFailure`/view 及 repository 默认 fail-closed 方法。
2. DB bridge 映射 exact fence、plan/item/submission/checkpoint，并返回更新后的 item/worker view。
3. `execute_company_controller_final_turn` 捕获 deterministic finalization error 后，先在 mutation lock 下 park exact finalizer，再包装 `CompanyControllerFinalSealFailed`。
4. scheduler 识别 `RuntimeMemoryError::Conflict { code: "stage_team_final_submitter_runtime_replaced" }`，生成 `COMPANY_CONTROLLER_RUNTIME_RECOVERED`，summary 明确“事实已保留、另发一次继续”。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(company_controller)' --status-level fail
```

预期：finalization failure 和 runtime replacement 都不会落到 generic `COMPANY_CONTROLLER_FAILED`。

## 任务 4：终止同 Turn 的二次执行

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_dispatch.rs`

**步骤：**

1. 增加 finalization-failed/runtime-recovered halt variant。
2. `stage_run_halt_reason` 校验 scheduler、passed、retry exhaustion 和明确 reason 后返回对应 halt。
3. 增加 JSON ToolResult 单测，断言同批后续工具被 server-authored halt barrier 跳过。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(stage_run_halt)|test(tool_dispatch)' --status-level fail
```

预期：两类 closeout halt 均立即结束当前 request。

## 任务 5：格式、静态检查与记录证据

**文件：** 受影响 Rust 文件、`docs/modules/backend/{golish-db.md,golish-db/repo.md,golish-agent-runtime.md,golish-agent-runtime/agentic_loop.md,golish-agent-app/ai.md}`、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`

**步骤：**

1. 同步模块卡里的公开合同、恢复语义和测试入口。
2. 运行 rustfmt、受影响 crate 的 focused nextest 与 scoped clippy。
3. 把命令、退出码、run id/关键断言写入 progress；保持 feature 为 `in_progress`，直到真实 Test1 continuation 验收。

**验证：**

```bash
cd backend && cargo fmt --all -- --check
just space-guard
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings
python3 -m json.tool feature_list.json >/dev/null
```

预期：全部退出 0；不运行未获授权的全仓测试或 `just precommit`。
