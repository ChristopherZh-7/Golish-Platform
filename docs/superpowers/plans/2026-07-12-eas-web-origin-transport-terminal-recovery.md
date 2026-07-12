# EAS Web-Origin Transport Terminal Recovery 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划，并严格执行 test-driven-development。

**目标：** 让可精确归因的 WhatWeb target-side opening failure 成为 guarded exact-origin `blocked` 终态，消除 EAS WEB gate 的无限重试，同时保持未知错误 fail-closed。

**架构：** EAS wrapper 先用纯函数把 stdout records 与 ANSI-stripped `ERROR Opening:` stderr 映射到已授权 exact origins。仅当非零批次中的每个 origin 都能被正常记录或精确错误覆盖时，正常成员继续结构化落库，失败成员发布 target-bound blocked evidence/outcome；gate 和 coverage read model只接受相同 origin/technique/outcome/evidence id 的可信生产者事实。

**技术栈：** Rust 2021、sqlx/Postgres evidence ledger、cargo-nextest、embedded stage gate。

## 文件

- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`：WhatWeb per-origin 分类、guarded blocked publication、wrapper completion diagnostics、单元测试。
- 修改 `backend/crates/golish-agent-kit/src/harness/gate/eas_web_origin_check.rs`：exact blocked completion。
- 修改 `backend/crates/golish-agent-kit/src/harness/org_gate.rs`：EAS strict outcome projection。
- 修改 `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`：仅 producer-owned EAS WEB blocked evidence 可关闭父 cell。
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`：EAS WEB blocked 的 target-bound exact identity。
- 修改 `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`：blocked origin loader/aggregate/read-model parity。
- 修改 EAS methodology、对应模块卡、`docs/modules/INDEX.md`、`feature_list.json` 与 `agent-progress.md`：记录合同和验证证据。

## Task 1：RED — 固定 WhatWeb exact-origin 分类合同

**测试文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

1. 添加纯测试 `whatweb_opening_failures_are_exact_authorized_blocked_outcomes`，使用 live 样本：

```text
ERROR Opening: http://222.186.129.58:80 - end of file reached
ERROR Opening: https://43.248.77.17:443 - Connection reset by peer - SSL_connect
```

断言两个 exact origins 各自为 `blocked`；错 scheme/host/port 不匹配。
2. 添加 `whatweb_mixed_batch_preserves_success_and_blocks_only_attributed_origin`：一个 stdout success sibling + 一个精确 opening failure 必须产生独立计划。
3. 添加 `whatweb_unattributed_runtime_or_truncated_stderr_remains_nonterminal`：未知 stderr、未授权 URL、截断标记均拒绝 recovery。
4. 运行并确认因分类 API 尚不存在而 RED：

```bash
cd backend && cargo nextest run -p golish-pentest-app -E 'test(whatweb_opening_failures_are_exact_authorized_blocked_outcomes) | test(whatweb_mixed_batch_preserves_success_and_blocks_only_attributed_origin) | test(whatweb_unattributed_runtime_or_truncated_stderr_remains_nonterminal)' --status-level fail
```

**提交：** 本轮不自动 commit；用户尚未要求提交。

## Task 2：GREEN — 最小 per-origin wrapper terminalization

**实现文件：** `backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

1. 新增纯分类结构，至少包含 canonical origin、`found|empty|blocked` verdict、scoped raw output，以及 unknown stderr/missing member 错误。
2. 只接受 ANSI-stripped `ERROR Opening: <absolute-http-url> - <reason>`，且 normalized reason 必须精确等于 `end of file reached`、`connection reset by peer` 或 live 样本 `connection reset by peer - SSL_connect`；前后夹带这些字样或追加 runtime prose 仍拒绝。将 URL canonicalize 后与 `ActiveTargetAuthorizationKind::ExactWebOrigin` 精确匹配。
3. 为可完全归因的 mixed/non-zero batch构造仅供 structured landing 的成功 stdout view；逐 authorization 调 `append_guarded` 与 `upsert_batch_guarded`，blocked 行使用：

```rust
outcome: "blocked",
source: Some("eas_fingerprint_web_stack".to_string()),
technique: GOLISH_EAS_WEB_FINGERPRINT,
asset: canonical_exact_origin,
```

4. 只有 batch 至少含一个精确 blocked observation、全部成员都已归因且全部 guarded writes 完整时，才将 wrapper 归一为成功，并保留 `wrapped_exit_code`、bounded `terminal_blocked_origins`；exit 1 但全是 stdout success、未知错误或空解释沿用原整批失败。
5. 重跑 Task 1 命令，确认 GREEN；再跑：

```bash
cd backend && cargo nextest run -p golish-pentest-app eas_capabilities --status-level fail
```

**提交：** 本轮不自动 commit；用户尚未要求提交。

## Task 3：RED→GREEN — exact gate 与 strict evidence parity

**测试/实现文件：**

- `backend/crates/golish-agent-kit/src/harness/gate/eas_web_origin_check.rs`
- `backend/crates/golish-agent-kit/src/harness/org_gate.rs`
- `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`
- `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`

1. 先添加以下失败测试：
   - `guarded_blocked_completion_requires_matching_exact_origin_and_evidence_identity`；
   - `exact_origin_barrier_passes_after_guarded_blocked_terminal`；
   - `eas_web_blocked_requires_matching_guarded_outcome_not_business_found`；
   - `eas_web_blocked_evidence_requires_trusted_exact_whatweb_identity`。
2. 最小实现：仅 EAS WEB 的 `blocked` 可进入 strict EAS terminal set；要求 evidence id > 0、exact origin、outcome/id 匹配，且 outcome source/producer 为 `eas_fingerprint_web_stack` / WhatWeb。
3. `coverage_complete` 仅让 `derive_from_evidence` 的 EAS WEB blocked fact关闭 cell；保留 model-authored exact-origin exception 禁令。
4. 运行：

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(guarded_blocked_completion_requires_matching_exact_origin_and_evidence_identity) | test(exact_origin_barrier_passes_after_guarded_blocked_terminal) | test(eas_web_blocked_requires_matching_guarded_outcome_not_business_found)' --status-level fail
cd backend && cargo nextest run -p golish-agent-app eas_web_blocked_evidence_requires_trusted_exact_whatweb_identity --status-level fail
```

**提交：** 本轮不自动 commit；用户尚未要求提交。

## Task 4：RED→GREEN — preflight/read-model terminal parity

**测试/实现文件：** `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`

1. 先添加 `eas_web_blocked_origin_is_terminal_and_remains_visible`：required origins 为一个 found、一个 producer-blocked；断言 `missing_origins=[]`、`blocked_origins` 精确、无 WhatWeb suggestion、evidence refs 都保留。
2. Loader 接受严格验证后的 WEB blocked outcome。Aggregate 在无 missing origin 时：有 found 可保持父 cell found并在 details 显示 blocked origins；全部 blocked 时父 cell为 blocked。任何 missing 仍 pending/partial。
3. 运行：

```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(eas_web_cell_stays_partial_until_every_exact_origin_is_terminal) | test(eas_web_blocked_origin_is_terminal_and_remains_visible)' --status-level fail
```

**提交：** 本轮不自动 commit；用户尚未要求提交。

## Task 5：合同、模块卡与验证

1. 更新 `resources/harness/stages/external_attack_surface/methodology.md`：target-side exact opening failure=`blocked`，正常无指纹=`checked_empty`，internal/unattributed failure仍未完成。
2. 更新三张模块卡及 `docs/modules/INDEX.md` 状态描述；把新设计/计划、RED→GREEN 命令与结果写入 `agent-progress.md`，并为当前唯一 `in_progress` feature追加验证项/evidence，不标 `passing`。
3. 运行完整聚焦验证：

```bash
cd backend && cargo nextest run -p golish-pentest-app eas_capabilities --status-level fail
cd backend && cargo nextest run -p golish-agent-kit -E 'test(eas_web_origin) | test(eas_web_blocked)' --status-level fail
cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail
cd backend && cargo clippy -p golish-pentest-app -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
jq empty feature_list.json
git diff --check
```

4. 不运行 `./init.sh`，遵守当前用户约束；不发起真实外部扫描。Fresh compiled live continuation仍是最终 acceptance，完成前 feature 保持 `in_progress`。

**提交：** 本轮不自动 commit；用户尚未要求提交。
