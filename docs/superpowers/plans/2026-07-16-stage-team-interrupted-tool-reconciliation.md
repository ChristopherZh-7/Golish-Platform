# Stage Team 中断工具确定性恢复实现计划

**目标：** 让停机时卡在 bounded Enumeration crawler 或 EAS coverage wrapper 的 Worker 沿原 WorkerRun/message chain
继续，先查 durable worklist 再补缺口；未知或高风险工具继续 fail closed。

**架构：** 在 DB claim transaction 内用 closed tool policy 封存旧 active tool 并重排同一 Worker/WorkItem；runtime 从
server-owned checkpoint marker 生成 worklist-first objective。无 migration、无 IPC 变更、无外部工具直接重放。

## Task 1：RED—锁定 crawler same-chain 恢复合同

- 修改 `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`
- 新增 active crawler 经 startup park 后可 claim 的集成测试，断言同 WorkItem/WorkerRun/chain、epoch +1、旧 tool failed、
  marker 存在。
- 保留并强化非白名单 active tool 的 manual recovery 测试。
- `just space-guard` 后用 `cargo nextest` 跑单测，确认旧实现 RED。

## Task 2：GREEN—实现服务端 closed policy 与原子重排

- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 为 `enum_crawl_same_origin_urls` 与四个 EAS wrapper 增加 closed `resume_after_reconcile` policy。
- 在既有 expired/recovery candidate reconciliation 中：终态化旧 tool、写 checkpoint marker、清旧 lease/active tool、同一
  Worker/WorkItem 回 queued；attempt budget 与 exact identity/CAS 继续生效。
- 非白名单和 budget exhausted 返回现有 `RecoveryRequired`。

## Task 3：注入 worklist-first 同链恢复指令

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 从 checkpoint marker 构造 host-owned child objective suffix；要求先查 status/next，只处理 pending/error/partial，禁止重做
  terminal cells或盲重放旧 args。
- 添加纯函数单测，证明 crawler/EAS marker 与普通/高风险 checkpoint 分流。

## Task 4：CLI 回归与文档同步

- 运行目标 `golish-db` integration 与 `golish-agent-runtime` unit tests。
- 运行相关 package clippy、workspace fmt check、`git diff --check`。
- 更新 `docs/modules/backend/golish-db/repo.md`、
  `docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/INDEX.md`、
  `agent-progress.md` 与 `feature_list.json`。
- 不运行 `init.sh`；未跑完整 `just precommit` 时 feature 保持 `in_progress`。
