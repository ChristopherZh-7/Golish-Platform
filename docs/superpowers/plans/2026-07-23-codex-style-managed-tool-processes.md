> Superseded by `docs/superpowers/plans/2026-07-23-codex-same-session-process-yield.md` for the global AI-tool process contract. This plan remains the evidence history for the first EAS-scoped pass.

# Codex 式受管工具进程实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 移除普通工具进程的 elapsed-time 自动 kill，让 AI/operator按存活与输出活动决定 wait/check/kill，并让 EAS Naabu full在 typed guarded continuation完成后安全发布v3 evidence。

**架构：** `golish-app-core` 的进程manager负责无deadline child lifecycle、活动指标、完整spool和业务无关reconciler；`golish-pentest-app`为EAS安装持有冻结授权/plan的typed reconciler；`golish-agent-app`只消费已reconcile completion、短路legacy generic hooks并让submit快速把live handles交还AI；前端呈现manual lifecycle。历史v1/v2 evidence只读兼容。

**技术栈：** Rust 2021、Tokio、async-trait、serde_json、sqlx/Postgres guarded repositories、React 19、TypeScript 6、Vitest、Biome。

## Task 1：锁定无自动 deadline 的 manager 合同

**文件：**

- 修改：`backend/crates/golish-app-core/src/background_jobs.rs`
- 修改：`backend/crates/golish-app-core/src/pty_interactive.rs`

1. 先新增失败测试：短hard-limit参数过去后子进程仍为running；显式kill与session stop仍会kill/reap；terminal reason能区分natural exit和explicit/session cancellation。
2. 运行：`cd backend && just space-guard && cargo nextest run -p golish-app-core -E 'test(background_job_has_no_elapsed_deadline) | test(explicit_kill_still_reaps_managed_job) | test(session_stop_still_reaps_managed_job)' --status-level fail`，确认RED来自现有watchdog。
3. 删除 `hard_limit` sleep/select、DNS hard-limit特判和 `COMMAND_TIMEOUT`通用合成；manager只select child wait与explicit cancellation。
4. 把soft timeout改名/文案为inline handoff wait；auto/background mode到点promote session并返回handle，不kill。foreground-policy只响应明确cancellation。
5. 复跑上述focused测试至GREEN。

## Task 2：增加活动指标、完整spool与受管读取

**文件：**

- 修改：`backend/crates/golish-app-core/src/background_jobs.rs`
- 修改：`backend/crates/golish-app-core/src/pty_interactive.rs`

1. 先新增失败测试：running job报告stdout/stderr累计字节和last-output-age；超过512KiB内存tail的输出在terminal spool中仍完整可读；spool截断/写失败标output incomplete。
2. 运行对应exact tests，确认RED。
3. 为每job创建server-owned stdout/stderr spool，pump同时更新有界tail、总字节与`last_output_at`；两个pipe EOF后才finalize。
4. 扩展 `JobSnapshot` / `RunningJob` / `check_job` / `wait_for_background_jobs` 返回activity fields、retention metadata和`automatic_kill=false`；bounded wait超时不改变job状态。
5. prune/remove时只清理该job已知spool；保持terminal handle在manager保留期内可读。
6. 复跑focused manager/pty tests至GREEN。

## Task 3：引入业务无关 typed reconciler

**文件：**

- 修改：`backend/crates/golish-app-core/src/background_jobs.rs`
- 修改：`backend/crates/golish-pentest-app/src/pentest_ai/run.rs`

1. 先新增fixture reconciler失败测试：natural exit+drain后reconciler执行一次且早于completion broadcast；inline完成返回typed result；background完成携typed result并设置skip-generic；reconciler失败不回退generic。
2. 运行exact tests确认RED。
3. 定义object-safe async `BackgroundJobReconciler`、terminal output descriptor和typed reconciliation result；manager在terminal broadcast前调用一次并把结果保存在job state/completion。
4. 在 `PentestRunTool` 增加crate-internal guarded-managed入口；普通模型JSON不能安装reconciler，Candidate verifier仍禁止detach；launch前target/local witnesses继续最后重验。
5. 确保inline完成job不被session completion listener重复消费；只有返回handle时才promote为session outstanding。
6. 复跑focused tests至GREEN。

## Task 4：用 TDD 接入 EAS port typed continuation 与 v3 producer

**文件：**

- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

1. 先新增失败测试：v3 quick/standard/full exact recipes包含 `-Pn -c 128 -timeout 500 -retries 1 -verify -warm-up-time 0 -nc -duc`，host budgets为128/32/4，不包含automatic process deadline。
2. 新增失败测试：full返回managed handle时立即零landing；natural exit后同一guarded continuation发布found/empty；kill/nonzero/spool truncation/target drift保持partial/error。
3. 新增>512KiB fixture，证明v3 stdout hash和parser读完整spool而非completion tail。
4. 运行exact producer tests，确认RED。
5. 把现 `execute_wrapped` 的landing/evidence/completion后半段抽成可由inline和detached共同调用的单一路径；EAS port plan创建typed reconciler，捕获session/tool/operation/org/auth/plan/workspace/pool。
6. 实现v3 recipe、manifest domain separator与coverage/attestation字段；跨调用manifest去重和process-wide full并发治理明确留作后续独立调度任务。
7. 所有EAS typed terminal结果设置`structured_storage_disabled/generic_evidence_disabled`，失败也不允许legacy parser接管。
8. 复跑focused producer tests至GREEN。

## Task 5：严格增加 v3 Gate read side并保留 v2

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`

1. 先新增失败测试：合法Naabu v3 accepted；篡改workers/connect timeout/retries/Pn/automatic kill/termination reason/manifest/stdout hash/receipt逐项rejected；现有strict v2 fixture保持accepted。
2. 运行v3 exact validator测试，确认RED。
3. 在tool/schema dispatch中保留Nmap v1、Naabu v2原validator，新增独立v3 validator和v3 manifest hash；不放宽旧分支。
4. 复跑exact tests至GREEN。

## Task 6：让 completion/submit 把控制权及时交还 AI

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- 修改：`backend/crates/golish-sub-agents/src/defaults/{builder,prompts,tests}`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改：`resources/harness/stages/external_attack_surface/{methodology.md,spec.json}`

1. 先新增失败测试：typed EAS completion跳过全部legacy generic evidence/structured/outcome hooks，typed note/evidence trace先落，最后才`mark_reconciled`；submit发现live job在短grace后返回handle/activity而不等待30分钟。
2. 运行exact tests确认RED。
3. bridge listener优先消费manager已完成的typed reconcile result；`skip_generic_persistence`为true时绝不调用旧 `maybe_store_background_*`。
4. 把默认submit reconcile wait改为短grace/snapshot，`needs_fix`返回elapsed、last output age/bytes、quiet-expected/slow-SLO提示；文案允许bounded wait/check/kill，不把等待窗口结束称为process deadline。
5. submit/tool文案说明silent producer不必然挂死；超过slow-SLO后结合存活和活动决定继续等、kill或缩批。现有stage计划继续承担避免重复manifest调度的责任。
6. Prober默认与registry工具面加入wait/check/kill；prompt与stage methodology把idle/inline wait定义为观察信号，只有AI/operator结合workload和activity判断确实挂死或无用途时才显式kill。
7. 复跑focused tests至GREEN。

## Task 7：更新后台任务 UI

**文件：**

- 修改：`frontend/store/types/background-job.ts`
- 修改：`frontend/store/slices/ai.ts`
- 修改：`frontend/store/slices/workflow/sub-agent.ts`
- 修改：`frontend/services/ai-events/tool-handlers.ts`
- 修改：`frontend/components/BackgroundJobPanel/BackgroundJobPanel.tsx`
- 修改：`frontend/components/BackgroundJobPanel/BackgroundJobPanel.test.tsx`
- 修改：`frontend/store/background-jobs.test.ts`
- 修改：`frontend/services/ai-events/registry.test.ts`
- 修改：`frontend/lib/i18n/en.json`
- 修改：`frontend/lib/i18n/zh-CN.json`

1. 先把component test改为期待“不会自动停止/Managed manually”、inline handoff wait和last output，不再期待hard deadline；运行该文件确认RED。
2. 后端result解析`automatic_kill=false`与activity metadata；store不再保存/计算hard deadline。
3. panel删除deadline倒计时，显示manual lifecycle，保留stop按钮、background elapsed与last output。
4. 运行focused Vitest、受影响文件Biome和typecheck至GREEN。

## Task 8：模块卡、定向门禁与证据收尾

**文件：**

- 修改：`docs/modules/backend/golish-app-core.md`
- 修改：`docs/modules/backend/golish-pentest-app.md`
- 修改：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改：`docs/modules/backend/golish-agent-app.md`
- 修改：`docs/modules/backend/golish-agent-app/ai.md`
- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/backend/golish-sub-agents/defaults.md`
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/frontend/store.md`
- 修改：`docs/modules/frontend/services.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`agent-progress.md`
- 修改：`feature_list.json`

1. 同步职责、公开接口、依赖、坑和测试入口；INDEX状态保持✅并新增本次日期说明。
2. 每次Cargo前运行 `cd backend && just space-guard`。
3. 运行focused nextest：`golish-app-core` manager/pty exact tests、`golish-pentest-app` EAS v3 exact tests、`golish-agent-app` v3 Gate/completion/submit exact tests。
4. 运行受影响crate scoped Clippy `--all-targets -- -D warnings` 和受影响Rust文件rustfmt check。
5. 运行focused frontend Vitest、受影响文件Biome、`pnpm exec tsc --noEmit --pretty false`。
6. 运行 `jq empty feature_list.json`、单一active feature检查、scoped `git diff --check`；不运行未获授权的init/precommit/全workspace suites，也不发真实外部扫描。
7. 把命令、退出码和关键结果写入`agent-progress.md`/feature evidence；只有全部focused证据覆盖行为与风险后才把feature改为`passing`。
