# Vuln no-purge rollover 终态采用实现计划

> 目标：在不改 schema、不碰 Test1 live DB、不放宽 Gate 的前提下，让 finalizer rollover 保留已完成事实，并让当前 `175/180` 历史现场在克隆库中通过 CLI 收敛到真实 Gate PASS。

## 任务 1：冻结 RED 回归

**文件：**

- `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`
- `backend/crates/golish-db/src/repo/technique_outcomes.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 扩展 legacy finalizer fixture：source execution 有 exact submission 和 terminal Vuln outcomes，rollover 后断言 freshness floor 不变。
2. 构造已错误 replacement 的 `partial` cell，并保存同 operation/source epoch 的 terminal audit evidence；adoption 前 RED，adoption 后断言精确恢复。
3. 构造 later attempt-start `partial` 覆盖 terminal sibling 的 RED；期望整组 superseded 且终态不变。
4. runtime helper 断言 adoption 后 `180/180` 且 `build_vuln_worklist_shards` 返回空。

**定向验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(final_submitter)|test(no_purge_rollover)|test(attempt_marker)' --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(vuln_worklist)' --status-level fail
```

## 任务 2：保留 future rollover freshness epoch

**文件：**

- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`

**步骤：**

1. 给内部 checkpoint replacement 增加窄的 same-stage no-purge recovery mode。
2. 在 replacement 事务内重验 exact active execution、finalizer/submission/barrier witness。
3. 该模式创建新 execution/unit，但不推进 `operation_state.stage_started_at`；state blob 记录 source/replacement/provenance。
4. 普通 developer reset 与 destructive fact purge 继续使用新 epoch。

## 任务 3：实现 legacy compatibility terminal adoption

**文件：**

- `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 增加 exact Controller-fenced adoption DTO/trait/bridge。
2. DB 事务锁定 current replacement 与 source execution，验证唯一 old submission、same org/scope/manifest 和无 active child tool。
3. 对 current `partial/error` cell，从 source epoch 的 immutable evidence ledger 选择 exact latest terminal producer evidence；任一缺失则全回滚。
4. 重放 canonical outcomes并恢复 source freshness floor；保留新的 partial evidence作为诊断。
5. runtime 在 Vuln coverage snapshot 前调用 adoption；adopted 或 no-op 后重新读取 DB，绝不直接声明 PASS。
6. current replacement Unit 的 freshness floor 同步恢复到 exact source Unit epoch，保证 worklist、Gate 与 final-seal resolver 使用同一个窗口。

## 任务 4：阻止 terminal canonical truth 回退

**文件：**

- `backend/crates/golish-db/src/repo/technique_outcomes.rs`

**步骤：**

1. attempt-start 使用 conditional UPSERT：只允许 missing/partial/error。
2. sibling 中任一 terminal 冲突则整个事务 rollback并返回 `Superseded`。
3. terminal completion 的 generation CAS 保持不变；developer reset purge 行为保持不变。

## 任务 5：定向代码验证

1. 所有 Cargo 命令前运行 `just space-guard`。
2. 跑任务 1 的 focused nextest。
3. 跑受影响四 crate 的 `cargo check` 与 scoped Clippy `-D warnings`。
4. 跑 rustfmt check、JSON 和 diff check；不运行 init/precommit/全 workspace 测试。

## 任务 5.1：关闭克隆库暴露的 finalizer 兼容缺口

1. provider retry checkpoint 统一保存为 `{ _runtime, chain }`，其中 `chain` 原样保留 array/object；runtime 只解开该 server wrapper。
2. Vuln structural N/A materialization 使用 strict evidence asset key，保留 `scheme://host:port`，禁止 sibling origin 折叠。
3. 增加 `http:80`、`https:443`、`https:8443` sibling-origin focused 回归。

## 任务 6：克隆 Test1 并做 CLI 闭环

1. 使用独立数据库名复制当前 embedded `golish`；不修改 Test1 live database。
2. 记录克隆前后 source/test database identity 与关键行计数。
3. 新 binary 指向测试 DB，使用 Test1 workspace 的同 operation 运行 CLI continuation。
4. 断言：
   - 5 个 cell 恢复为 source terminal evidence；
   - 180/180 terminal，72 N/A 已 materialize；
   - finalizer-only 窗口没有新增 Nuclei evidence/tool invocation；
   - replacement execution 生成新 submission 与未失效 Vuln handoff；
   - 当前 stage slice 的 unit/execution PASS，CLI 返回 `stage_passed`；若通过 `--expect-stage` 限定单阶段验收，不要求 operation cursor 进入下一阶段；
   - old submission `7bf57b10-...` 字节级不变。
5. 测试库保留到本轮验收结束；若要删除，另行向用户确认。

## 任务 7：记录与模块卡

- 更新 `golish-db/repo`、`golish-agent-kit/db_traits`、`golish-agent-app/ai`、`golish-agent-runtime/agentic_loop` 模块卡与 `docs/modules/INDEX.md`。
- 把 focused test run id、CLI 输出、DB PASS 查询写入 `agent-progress.md` 和 `feature_list.json.evidence`；只有真实克隆库 Gate PASS 后才考虑把功能设为 `passing`。

## 完成状态（2026-07-20）

任务 1–7 已完成。独立克隆库 `golish_gatefix_20260720_d` 的 CLI continuation 返回 `gate=PASS` / `stage_passed=vuln_triage`，180 个 canonical cells 完整，finalizer-only 窗口没有重跑 scanner，source production DB 未变化。测试库按用户验收需要保留；删除仍需单独确认。
