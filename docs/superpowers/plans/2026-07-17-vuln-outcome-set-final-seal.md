# Vuln Outcome Set Final Seal 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 用一个可重算、可重放、可供 Reporting/Candidate 完整展开的 `TechniqueOutcomeSet` canonical authority 封存任意合法规模的 Vuln Triage 结果，消除 360 行被 256-key 上限截断的问题。

**架构：** Runtime 在 Gate PASS 后比较完整 terminal tuple set 并生成一个 run/set key；golish-db 在 final-seal 事务中锁定并重算完整 outcome set，StageHandoff 只保存集合引用。Reporting 和 Candidate 分别重算同一集合，前者展开全部 report sources，后者只在资产级聚合完成后执行 100-work-item policy。

**技术栈：** Rust 2021、Serde tagged enums、SQLx/Postgres、SHA-256 canonical JSON、cargo-nextest。

## 文件结构

- `backend/crates/golish-agent-kit/src/harness/handoff_catalog.rs`：新增共享 `TechniqueOutcomeSet` canonical key。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：Vuln 完整 tuple attestation、set key 构造、final-seal failure 分类。
- `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`：kit/DB canonical key 双向转换。
- `backend/crates/golish-db/src/repo/canonical_fact_refs.rs`：set member 规范化、集合摘要、事务内 resolver 与 replay authority。
- `backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`：完整 set handoff attestation 与聚合后 policy。
- `backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs`：legacy row ref 与 set ref 双读、完整展开。
- `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`：真实 Postgres final-seal/replay set 测试。
- `backend/crates/golish-agent-app/tests/reporting_authority.rs`：Reporting set authority 测试。
- `docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-db.md`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/INDEX.md`：同步模块事实源。
- `agent-progress.md`、`feature_list.json`：状态与新鲜验证证据。

## Task 1：定义集合 identity，并先写 RED 单测

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/harness/handoff_catalog.rs`
- 修改：`backend/crates/golish-db/src/repo/canonical_fact_refs.rs`

**步骤 1：** 在 kit 和 DB 的 `CanonicalFactKey` 中添加完全一致的 tagged variant：

```rust
TechniqueOutcomeSet {
    organization_id: Uuid,
    run_id: String,
    stage: String,
    terminal_cell_count: u32,
    outcome_set_sha256: String,
}
```

**步骤 2：** 先在 `canonical_fact_refs.rs` 测试模块定义 360 个 member，断言期望 API 能稳定生成相同的 count/identity/content/evidence attestation，并让测试因 API 尚未实现而失败。

```rust
let first = technique_outcome_set_attestation("vuln_triage", org, run, &members).unwrap();
let mut reversed = members.clone();
reversed.reverse();
let second = technique_outcome_set_attestation("vuln_triage", org, run, &reversed).unwrap();
assert_eq!(first, second);
assert_eq!(first.terminal_cell_count, 360);
```

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db technique_outcome_set --status-level fail
```

预期：RED，失败原因是缺少 set attestation 实现或新断言尚不满足，而不是测试编译配置错误。

**步骤 3：** 实现 `TechniqueOutcomeSetMember`、`TechniqueOutcomeSetAttestation` 和稳定排序/hash；拒绝空集合、重复 `(asset, technique)`、非终态、foreign org/run、空/非正 evidence。

**步骤 4：** 重新运行 focused test，预期全部 GREEN。

**提交：** `feat: define canonical technique outcome sets`

## Task 2：让 final seal 使用一个完整 set key

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`

**步骤 1：** 添加 RED 测试：360 个 `TechniqueOutcomeFact` 与 360 个 coverage cells 生成一个 `TechniqueOutcomeSet`；少一行、状态漂移和多一个 Finding 分别验证 fail-closed/独立计数。

```rust
assert!(matches!(
    &seal.canonical_fact_keys[0],
    CanonicalFactKey::TechniqueOutcomeSet { terminal_cell_count: 360, .. }
));
assert_eq!(seal.canonical_fact_keys.len(), 1 + deliverable.findings.len());
```

**步骤 2：** 运行 runtime focused test，确认旧逐行/截断实现 RED。

**步骤 3：** 将 `deterministic_canonical_fact_keys` 改为接收 `StageKind`。Vuln 分支先比较完整 normalized tuple sets，再生成 set key；其他阶段保持现有逐行行为。删除用全部 canonical ref total 比 terminal cells 的错误断言。

**步骤 4：** 在 app bridge 中增加 kit/DB set key 双向转换。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(technique_outcome_set) | test(vuln_final_seal)' --status-level fail
cd backend && cargo nextest run -p golish-agent-app runtime_memory --status-level fail
```

预期：focused tests GREEN，360-cell seal 只有一个 set ref，Finding 不影响 outcome count。

**提交：** `fix: seal complete vuln outcome sets`

## Task 3：在 DB final-seal 事务内解析和重放集合

**文件：**

- 修改：`backend/crates/golish-db/src/repo/canonical_fact_refs.rs`
- 修改：`backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

**步骤 1：** 添加 embedded-Postgres RED：插入 360 行 exact operation outcomes，使用 set key finalize，断言当前 resolver 不认识新 key；另加删除一行、状态变更、证据漂移的拒绝用例。

**步骤 2：** `resolve_one` 的 set 分支用单次有序 `FOR SHARE` 查询加载完整 rows，调用 Task 1 的纯 attestation，精确比较 key count/hash，再返回一个 `CanonicalFactRef`：

```rust
CanonicalFactRef {
    key: key.clone(),
    organization_id,
    observed_at: attestation.observed_at,
    content_sha256: attestation.content_sha256,
    evidence_ids: attestation.evidence_ids,
}
```

**步骤 3：** 新增只供 final seal 使用的 `resolve_for_final_seal`，显式接收
`Unit.started_at` freshness floor 与本次 DB transaction 的 seal ceiling；通用
`resolve_for_handoff` 和 `resolve_for_fact_delta` 明确拒绝 set key。response-loss
replay 使用已持久化的 `handoff.gate_passed_at` 作为同一 ceiling。保留 key 数、
payload 字节与 evidence 数上限，不扩大常量。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(technique_outcome_set) | test(final_seal)' --status-level fail
```

预期：360-row finalize/replay GREEN；缺行、状态漂移、evidence/content drift 均返回 typed rejection。

**提交：** `feat: resolve vuln outcome sets atomically`

## Task 4：Reporting 完整展开 set authority

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs`
- 修改：`backend/crates/golish-agent-app/tests/reporting_authority.rs`

**步骤 1：** 添加 RED：一个 set ref 封存 360 rows 时返回 360 个 `TechniqueOutcome` source + 1 个 StageHandoff source；少一行或变更一行返回 fail-closed 错误。

**步骤 2：** 解析 handoff 时同时收集 legacy individual refs 与 set refs。对 set ref，将 repeatable-read transaction 已读取的 exact operation rows转换为共享 member，重算 set identity/content/evidence 并与 sealed ref exact compare。

**步骤 3：** set exact 后把全部 underlying rows逐条输出为 report sources；禁止 sample、latest fallback 或未封存附加行。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-app --test reporting_authority --status-level fail
```

预期：set/legacy 双读 GREEN；missing/drift/unsealed rows fail closed。

**提交：** `feat: expand sealed outcome sets in reporting`

## Task 5：Candidate 只限制聚合后的工作项

**文件：**

- 修改：`backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`

**步骤 1：** 把现有 over-policy 测试改为两个 RED 行为：120 raw terminal cells聚合成 12 surface observations应 PASS；101 个最终 surface/positive observations应 FAIL。

**步骤 2：** `FormulaicHandoffAuthority` 读取 handoff payload；`attest_formulaic_outcomes` 要求 exact `TechniqueOutcomeSet` key/ref匹配完整 rows和 watermark，但删除 `terminal_cells <= MAX_ATTACK_MANIFEST_ITEMS`。

**步骤 3：** 保留 `materialize_initial_candidate_observations` 结束处和插入事务中的 `observations.len() <= MAX_ATTACK_MANIFEST_ITEMS`，确保 policy 只约束真正 Candidate 工作项。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db attack_candidate_work_items --status-level fail
```

预期：大 raw matrix/小 aggregate PASS；大 aggregate FAIL；negative cells只进入 surface context。

**提交：** `fix: apply candidate fuel after aggregation`

## Task 6：区分 Gate BLOCK 与 final-seal failure

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤 1：** 添加 RED：Gate PASS 后 build/resolve/finalize 返回错误时，scheduler gap code必须为 `COMPANY_CONTROLLER_FINAL_SEAL_FAILED`，runtime control停止当前请求，且没有 coverage repair directive。

**步骤 2：** 用 typed wrapper标记 final-seal assembly/commit 区间错误；scheduler只把 deterministic `OrgVerdict::Block` 送入 repair，typed finalization error进入 continuation-only gap。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime company_controller_final --status-level fail
```

预期：Gate BLOCK repair测试保持 GREEN；post-Gate finalization测试返回独立 typed code。

**提交：** `fix: separate gate repair from seal recovery`

## Task 7：模块卡、状态和完整验证

**文件：**

- 修改：上述五张模块卡与 `docs/modules/INDEX.md`
- 修改：`agent-progress.md`
- 修改：`feature_list.json`

**步骤 1：** 记录 `TechniqueOutcomeSet` authority、legacy兼容、Reporting展开、Candidate聚合后fuel和post-Gate分类。

**步骤 2：** 运行 JSON/diff/fmt、相关 crate 全套、Clippy 和 precommit。每次 Cargo 命令前先运行 `just space-guard`；遵守用户指令，不运行 `init.sh`。

**验证：**

```bash
jq empty feature_list.json
git diff --check
just space-guard
cd backend && cargo fmt --all -- --check
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-runtime -p golish-db -p golish-agent-app --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-runtime -p golish-db -p golish-agent-app --all-targets -- -D warnings
just precommit
```

预期：全部 exit 0，nextest 0 failed，Clippy 0 warning，precommit打印成功；`init.sh`未执行。

**提交：** `fix: make vuln final seals cardinality safe`
