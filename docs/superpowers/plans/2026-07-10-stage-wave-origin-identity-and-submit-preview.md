# Stage wave origin identity and submit-preview parity 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 修复 canonical Web Origin 展开后 durable wave 身份丢失，并让 submit preview 与 worklist/final org gate 使用同一 running wave。
**架构：** durable wave 贯通对齐的 `target_ids + asset_values`，coverage 只按 target id 判成员并让 current-wave owner 优先 origin 去重。submit/worklist/final gate 共用受信 running wave；存在但损坏的 wave 三路 fail-closed，只有真正 NoWave 才回退 operation cutoff。
**技术栈：** Rust 2021、async-trait、sqlx/PostgreSQL、cargo nextest。

## 文件结构

- 修改 `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：保存 wave 原 target identity，并补多 origin 回归。
- 修改 `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`：扩窄查询 seam，解析受信 running wave，透传 snapshot。
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`：把窄 seam 接到既有 durable wave repo。
- 修改 `docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-kit/harness.md`、`docs/modules/backend/golish-db/repo.md`：同步合同说明。

## Task 1：先写 Web Origin wave identity 红灯测试

**文件：** `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`

**步骤 1：** 添加 domain/IP/URL-path fixture，展开后断言同一 target 的全部 origin 都不是 deferred；另加 foreign target 仍 deferred。

```rust
let expanded = expand_enumeration_web_origin_rows(vec![target]);
assert!(expanded.iter().all(|row| !is_deferred_wave_asset(
    row,
    None,
    Some(&BTreeSet::from([original_value.to_string()])),
)));
```

**步骤 2：** 运行红灯测试，预期旧实现把 canonical origin 与原 target value 比较，断言失败。

```bash
cd backend && cargo nextest run -p golish-agent-app wave_membership --status-level fail
```

**步骤 3：** 给 `StageAssetWaveView` 增加对齐的 `target_ids`，snapshot 同时接 ids/values，显式 wave filter 改比 `TargetCoverageRow.id`；origin dedupe 在共享 origin 时优先 current-wave owner。

**步骤 4：** 重跑同一测试，预期全部通过。

## Task 2：先写 submit-preview current-wave 红灯测试

**文件：** `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`

**步骤 1：** 扩 mock 记录 `stage_asset_coverage` 收到的 cutoff/wave ids/values；受信 operation/org/stage 返回 running wave，断言 preview 传入同一组值。

```rust
assert_eq!(seen_wave_ids, Some(vec![target_id]));
assert_eq!(seen_wave_values, Some(vec!["app.example.com".to_string()]));
assert_eq!(seen_started_at, Some(wave_started_at));
```

**步骤 2：** 运行红灯测试；旧 trait 无 wave 参数，测试应无法满足断言。

```bash
cd backend && cargo nextest run -p golish-agent-app submit_preview_uses_current_wave --status-level fail
```

**步骤 3：** 为 `EvidenceLedgerQuery` 增加 running-wave seam，并给 `stage_asset_coverage` 增加 ids/values 参数。`gate_context` 只从绑定的 operation id、org id、stage 读取 wave；有效 running wave 以 wave `started_at` 为 cutoff并透传 membership，真正无 wave 才回退 operation state，空/损坏 wave 返回 needs_fix。

**步骤 4：** `GolishDbRepoProvider` 实现 seam，委托 `stage_asset_wave_current_running_impl`，只返回 scoped repo 已验证的 wave。

**步骤 5：** 重跑 submit preview 测试，预期通过。

## Task 3：同步文档并做 focused 验证

**文件：** 三张模块卡与本设计/计划。

**步骤 1：** 模块卡写明 pre-expansion target identity 和三条 gate path 的 running-wave parity。

**步骤 2：** 运行格式化、focused tests、check/clippy 和 diff guard。

```bash
cd backend && cargo fmt -p golish-agent-app -p golish-agent-kit -- --check
cd backend && cargo nextest run -p golish-agent-app -p golish-agent-kit --status-level fail
cd backend && cargo check -p golish-agent-app -p golish-agent-kit
cd backend && cargo clippy -p golish-agent-app -p golish-agent-kit --all-targets -- -D warnings
git diff --check -- backend/crates/golish-agent-app docs/design/2026-07-10-stage-wave-origin-identity-and-submit-preview.md docs/superpowers/plans/2026-07-10-stage-wave-origin-identity-and-submit-preview.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-db/repo.md
```

**预期：** 全部 exit 0；无 warning；回归同时证明未在 wave 的 target 仍 fail-closed 为 `next_wave_pending`。

**提交：** 本子任务不提交，由主会话统一收口。
