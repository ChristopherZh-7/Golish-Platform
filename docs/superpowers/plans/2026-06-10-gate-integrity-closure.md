# Plan: Gate 真实性闭合（H1/H2/H3）

> 设计: `docs/design/2026-06-10-gate-integrity-closure.md`（Approved）。
> 全程 TDD：每个行为变化先红后绿。验证命令均在 `backend/` 下执行。

## Task 1 · H3 target_intel 接 named_check:min_invocations

1. RED: `stage_spec.rs` 测试新增断言——target_intel 的 gate_rules 含 `NamedCheck { check: "min_invocations" }`，总数 6。
2. GREEN: `resources/harness/stages/target_intel.json` gate_rules 追加 `{ "op": "named_check", "check": "min_invocations" }`。
3. 验证: `cargo nextest run -p golish-agent-kit stage_spec`

## Task 2 · H1 capability_match 纯模块

1. 新建 `harness/capability_match.rs`：`resolve_ledger_tool` / `capability_matches_tool` / `is_known_capability` / `ledger_missing_invocations`；`harness/mod.rs` 导出。
2. 单测（先写）：wrapper 解析、三能力键映射、recon_enrich_assets 收编、未知键 fail-closed、守卫测试 `all_embedded_min_invocation_keys_are_recognized`。
3. 验证: `cargo nextest run -p golish-agent-kit capability_match`

## Task 3 · H1 数据通路

1. `db_traits/repo.rs`: `evidence_tool_rows_for_session` 默认 `Ok(None)`。
2. golish-db `repo/audit/mod.rs`: `evidence_tool_rows_for_session` SQL。
3. `golish-agent-app/db_bridge/evidence.rs` + `mod.rs`: impl 返回 `Some(rows)`。
4. 验证: `cargo check -p golish-db -p golish-agent-kit -p golish-agent-app`

## Task 4 · H1 orchestrator 回查

1. `HarnessGateOutcome` 加 `min_invocations: HashMap<String,u32>`（spec 透传；全部构造点补字段）。
2. RED: loop 测试——MemRepo 加 evidence_rows 开关；`enforce_min_invocations_ledger` 三例（None 跳过 / Some([]) BLOCK / 满足 PASS）。
3. GREEN: 实现 `enforce_min_invocations_ledger` + 两个 gate 调用点接线（existence→kinds→freshness→min_invocations→scoping）。
4. `min_invocations_check.rs` 顶部注释更新（Phase C 已落，self-report 为快速路径）。
5. 验证: `cargo nextest run -p golish-agent-kit -E 'test(execute_harness_loop)'` + 全 crate

## Task 5 · H2 fail-closed + strict

1. RED: hook 测试——`harness_profile_id=Some("nonexistent")` 时 outcome 必须是 Some(BLOCK) 而非 None。
2. GREEN: `internal_error_gate_outcome` + 两处 `return (content, None)` 改 BLOCK。
3. `feature_flags::harness_strict_enabled`（`GOLISH_HARNESS_STRICT` 默认关）+ 纯函数测试；5 处 enforce_* infra Err 分支接 strict 翻 BLOCK。
4. 验证: `cargo nextest run -p golish-agent-kit`

## Task 6 · 收口

1. `cargo fmt` + `cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-db --lib -- -D warnings`
2. `cargo nextest run -p golish-agent-kit -p golish-agent-app -p golish-db`
3. ReadLints 全改动文件；agent-progress.md + feature_list evidence 填写。
