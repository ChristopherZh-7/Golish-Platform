# Task Mode Refactor to Harness · Implementation Plan

> ℹ️ **2026-06-01**: 本计划「现状/优先级」部分已被 [../../design/2026-06-01-harness-rebuild.md](../../design/2026-06-01-harness-rebuild.md) + [2026-06-01-harness-rebuild.md](2026-06-01-harness-rebuild.md) re-anchor。Phase 1 已实现（见本文 §12），后续 re-anchor + 验证以 2026-06-01 plan 为准；本文 task-mode 重构步骤作历史保留。

- **Author**: MCP-1
- **Date**: 2026-05-26
- **Status**: Implemented (Phase 1 · 17 Task / 16 commits)
- **Source of truth**: `docs/design/2026-05-26-operation-harness-profile-dag-lab.md` §21
- **Depends on**: Doc 1 (evidence-ledger) + Doc 2 (mcp-resource) + Doc 3 (stage-harness-mvp)

> 本文是把 Doc 1/2/3 三份设计文档落地为 Phase 1 实施步骤的总计划。Phase 0 plan only·**不动代码**·**不出 migration**。
>
> Phase 1 实施需获得用户 §AGENTS.md §2.7 明示授权。

---

## 1. 目标

把 chat panel 现有的 task 模式（PentAGI 风格的 task_orchestrator）逐步重构成 Doc 3 描述的 harness。MVP 1 stage 跑通后再扩。

**前置条件**（必须先满足）：

- `just precommit` 切绿（5 clippy + 2 baseline test failure 修完）
- `feat/asm-intel-providers` 分支的 `asset-intel-hydrate-disambiguation` 切 passing
- 用户明示 §2.7 授权 schema migration

---

## 2. 总体策略

```text
Phase 1a · Schema 落地 (Doc 1)
  └─ migration → repo → trait → 测试 (TDD)

Phase 1b · MCP Resource 落地 (Doc 2)
  └─ Tauri command → commands_facade → frontend api wrapper

Phase 1c · Stage Harness MVP 落地 (Doc 3 · 仅 1 stage)
  └─ resources/harness/ JSON → harness module → Gate → 接入 task_orchestrator

Phase 1d · Feature flag gradual rollout
  └─ default off → demo stage 验证 → 单元测试 → e2e demo

(Phase 2-5 → 扩 enumeration / pentest / Lab · 不在本计划范围)
```

---

## 3. Phase 1a · Evidence Ledger Schema (Doc 1)

### Task 1a.1 · 创建 SQL migration

**File**: `migrations/20260601000001_evidence_ledger.sql`（新建·Phase 1）

```sql
-- Step 1: audit_log 加 audit_role 字段
ALTER TABLE audit_log
    ADD COLUMN IF NOT EXISTS audit_role TEXT NOT NULL DEFAULT 'action';
CREATE INDEX IF NOT EXISTS audit_log_audit_role_idx ON audit_log(audit_role);

-- Step 2: organizations 加 scope_rules_version
ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS scope_rules_version BIGINT NOT NULL DEFAULT 1;

-- Step 3: evidence_classifications (bitemporal)
CREATE TABLE evidence_classifications (
    id BIGSERIAL PRIMARY KEY,
    evidence_audit_id BIGINT NOT NULL REFERENCES audit_log(id),
    classification TEXT NOT NULL,
    scope_version BIGINT NOT NULL,
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    valid_to TIMESTAMPTZ,
    reason TEXT NOT NULL,
    relabel_decision TEXT,
    classified_by_session TEXT NOT NULL,
    producing_stage_run_id UUID,
    schema_v INT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX evidence_classifications_current_idx
    ON evidence_classifications(evidence_audit_id) WHERE valid_to IS NULL;
CREATE INDEX evidence_classifications_stage_idx
    ON evidence_classifications(producing_stage_run_id) WHERE valid_to IS NULL;

-- Step 4: operation_state cursor
CREATE TABLE operation_state (
    operation_id UUID PRIMARY KEY,
    profile TEXT NOT NULL,
    current_stage TEXT NOT NULL,
    stage_started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_evidence_audit_id BIGINT,
    last_classification_id BIGINT REFERENCES evidence_classifications(id),
    last_scope_version BIGINT,
    state_blob JSONB NOT NULL DEFAULT '{}',
    superseded_by UUID REFERENCES operation_state(operation_id)
);

-- Step 5: stage_runs (for Doc 3 sprint_contracts FK)
CREATE TABLE stage_runs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    stage_kind TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'started',
    active_sprint_contract_id UUID  -- FK 在 Task 1c.2 加
);

-- Step 6: sprint_contracts (Doc 3 §7)
CREATE TABLE sprint_contracts (
    id UUID PRIMARY KEY,
    stage_run_id UUID NOT NULL REFERENCES stage_runs(id),
    contract_text TEXT NOT NULL,
    locked_after TIMESTAMPTZ NOT NULL,
    superseded_by UUID REFERENCES sprint_contracts(id),
    status TEXT NOT NULL DEFAULT 'active',
    planner_llm_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Step 7: stage_runs.active_sprint_contract_id FK
ALTER TABLE stage_runs
    ADD CONSTRAINT stage_runs_active_sprint_contract_fk
    FOREIGN KEY (active_sprint_contract_id) REFERENCES sprint_contracts(id);
```

**Verification**:

```bash
cargo run -p golish-db --bin migrate -- --dry-run
# 应输出预计执行的 7 步 SQL，不动数据
```

**Commit**: `feat(db): add evidence ledger + sprint_contracts schema`

### Task 1a.2 · Rust DTO + repo

**Files**:

- `backend/crates/golish-pentest/src/evidence_ledger/types.rs`（新建）
- `backend/crates/golish-pentest/src/evidence_ledger/mod.rs`（新建）
- `backend/crates/golish-db/src/repo/evidence_classifications.rs`（新建）
- `backend/crates/golish-db/src/repo/operation_state.rs`（新建）
- `backend/crates/golish-db/src/repo/stage_runs.rs`（新建）
- `backend/crates/golish-db/src/repo/sprint_contracts.rs`（新建）

**Steps**:

1. 写 Doc 1 §4.1-§4.6 中的 enum / struct（EvidenceScopeLabel / RelabelDecision / SkipReason）
2. 写 repo 4 个文件，每个 4-6 个自由函数（insert / current_for / close_current_open_new / list_supersedes_chain）
3. 写 ScopeService trait（Doc 1 §4.2）
4. 写 EvidenceLedger struct + 6 个方法（Doc 1 §4.3）
5. 写 validate_relabel（Doc 1 §4.4）

**Verification**:

```bash
cd backend
cargo test -p golish-pentest --lib evidence_ledger
cargo test -p golish-db --lib evidence_classifications
```

**Commit**: `feat(evidence): add EvidenceLedger + ScopeService + repo functions`

### Task 1a.3 · startup reclaim

**File**: `backend/crates/golish/src/lib.rs`（修改 startup_hooks）

**Steps**:

1. 加 `reclaim_abandoned_audits(pool, Duration::hours(1))` 调用到 startup
2. 写单测验证: 1h 前的 started 行被标 abandoned；1h 内的不动

**Verification**:

```bash
cargo test -p golish --lib reclaim_abandoned
```

**Commit**: `feat(evidence): startup reclaim abandoned audit rows`

### Task 1a.4 · 资源驱动配置

**File**: `resources/harness/evidence_kinds.json`（新建）

```json
{
  "dns_a": { "default_max_age_secs": 86400 },
  "dns_aaaa": { "default_max_age_secs": 86400 },
  "ct_log": { "default_max_age_secs": 604800 },
  "cve_feed": { "default_max_age_secs": 86400 },
  "nmap": { "default_max_age_secs": 259200 },
  "http_probe": { "default_max_age_secs": 21600 },
  "shodan_query": { "default_max_age_secs": 3600 },
  "whois": { "default_max_age_secs": 2592000 }
}
```

**Steps**:

1. 创建 JSON
2. 在 `golish-pentest/src/evidence_kinds.rs` 加 loader（serde_json::from_str + LazyLock 缓存）
3. 加 Rust 测试：lookup `dns_a` 返 86400

**Commit**: `feat(evidence): add evidence_kinds.json static config`

### Phase 1a 总验证

```bash
just precommit  # 必须全绿
```

---

## 4. Phase 1b · MCP Resource Evidence Summary (Doc 2)

### Task 1b.1 · EvidenceSanitizer

**File**: `backend/crates/golish-pentest/src/evidence_sanitizer.rs`（新建）

**Steps**:

1. 写 4 步 sanitize pipeline (control char strip / HTML escape / length cap / structural fence)
2. 写 parse_structured 按 kind 走对应 parser（dns / http / whois / ...）
3. 单测覆盖：sanitize 后的输出 ≤ 4KB、HTML 被 escape、control char 被剥

**Commit**: `feat(evidence): add EvidenceSanitizer for prompt injection defense`

### Task 1b.2 · Tauri command read_evidence

**Files**:

- `backend/crates/golish/src/tools/evidence.rs`（新建·按 docs/development.md 5 步走）
- `backend/crates/golish/src/commands_facade/evidence.rs`（新建）
- `backend/crates/golish/src/commands_registry.rs`（修改: 加 evidence_read entry）
- `frontend/lib/api/evidence.ts`（新建）
- `frontend/lib/generated/`（ts-rs 自动生成）

**Steps**:

1. 写 `evidence_read(state, request)` 函数体
2. IDOR check + scope_version snapshot check
3. 读 audit_log + evidence_classifications + 调 EvidenceSanitizer
4. 注册到 facade + registry
5. 前端 wrapper（按 §I3 + §G1.4）
6. 单测 + e2e（mock evidence + 验证 sanitize 输出）

**Verification**:

```bash
cargo test -p golish --lib evidence_read
pnpm vitest run frontend/lib/api/evidence.test.ts
```

**Commit**: `feat(evidence): add read_evidence Tauri command with sanitize layer`

### Task 1b.3 · stream_retry classifier 加 evidence_read 频率拦截

**File**: `backend/crates/golish-agent-runtime/src/agentic_loop/stream_retry.rs`（修改）

**Steps**:

1. 加 `count_recent_evidence_reads(session_id, Duration)` helper
2. `classify_tool_call` 加分支：若 call.name == "evidence_read" 且 1min 内 > 50 次 → 返 ToolCallWarning::EvidenceReadFlooding
3. 单测覆盖：< 50 通过、> 50 警告

**Commit**: `feat(stream-retry): rate-limit evidence_read tool calls`

### Phase 1b 总验证

```bash
just precommit
just test-fe
```

---

## 5. Phase 1c · Stage Harness MVP (Doc 3)

### Task 1c.1 · 资源驱动配置

**Files**（全部新建·Phase 1）：

- `resources/harness/profiles/assessment.json`
- `resources/harness/profiles/assessment.sprint_skeleton.json`
- `resources/harness/stages/external_attack_surface.json`
- `resources/harness/graph/operation_graph.json` (base DAG)

详见 Doc 3 §2 / §4 / §7。

**Verification**:

```bash
python3 -m json.tool resources/harness/profiles/assessment.json
python3 -m json.tool resources/harness/stages/external_attack_surface.json
```

**Commit**: `feat(harness): add assessment profile + external_attack_surface stage spec`

### Task 1c.2 · harness module 骨架

**Files**:

- `backend/crates/golish-agent-kit/src/harness/mod.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/types.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/profile.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/nl_slice.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/intent_classifier.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/pre_action_authorizer.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/stage_harness.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/sprint_contract.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/gate/mod.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/gate/schema_check.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/gate/scope_check.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/gate/contract_check.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/gate/vacuous_check.rs`（新建）
- `backend/crates/golish-agent-kit/src/harness/gate/freshness_check.rs`（新建）

**Steps**: 每个文件按 Doc 3 对应章节写。

**Verification**:

```bash
cargo check -p golish-agent-kit
cargo test -p golish-agent-kit --lib harness
```

**Commit**: `feat(harness): add stage harness module skeleton (no orchestrator wiring yet)`

### Task 1c.3 · IntentClassifier 词库

**File**: `backend/crates/golish-agent-kit/src/harness/intent_classifier.rs`

按 Doc 3 §6.1 写词库 + classify 函数 + 单测。

**Commit**: `feat(harness): add deterministic IntentClassifier`

### Task 1c.4 · Sprint Contract 生成

**File**: `backend/crates/golish-agent-kit/src/harness/sprint_contract.rs`

按 Doc 3 §7 写：

1. SprintContract DTO
2. SprintContractGenerator trait
3. impl: load skeleton from JSON + planner LLM 填变量 + lock
4. 写 repo: insert / list / mark_superseded（对接 sprint_contracts 表）

**Commit**: `feat(harness): add Sprint Contract generator with cross-vendor LLM`

### Task 1c.5 · Gate validator

按 Doc 3 §8 把 6 个 check 逐一写。每个 check 一个文件 + 单测。

**Commit per check**: 6 个 commit·每个 `feat(harness): add <check_name>`

### Task 1c.6 · 接入 task_orchestrator

**File**: `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（修改）

**Steps**:

1. 在 imports 加 `use crate::harness::{StageHarness, validate_external_attack_surface_gate, ...}`
2. 在 `execute_single_subtask` 末端、return 前插入：

```rust
if let Some(stage_hint) = planned.harness_stage.clone() {
    let harness = StageHarness::for_stage(stage_hint)?;
    let deliverable = harness.parse_deliverable(&agent_result.content)?;
    let decision = harness.validate_gate(&deliverable, &evidence_ledger, &sprint_contract)?;
    if !decision.allowed {
        let recovery = decision.recovery_actions.clone();
        let content = format!(
            "{}\n\n## Harness Gate Decision\n```json\n{}\n```",
            agent_result.content,
            serde_json::to_string_pretty(&decision)?
        );
        return (content, agent_result.token_usage, Some(recovery));
    }
}
```

3. 改 PlannedSubtask 加 `harness_stage: Option<HarnessStageHint>` + `nl_slice: Option<NlSlice>`（`#[serde(default)]` 保后向兼容）

**Verification**:

```bash
cargo test -p golish-agent-kit task_orchestrator
cargo test -p golish-agent-kit harness
```

**Commit**: `feat(orchestrator): wire stage harness gate into execute_single_subtask`

### Task 1c.7 · Feature flag

**File**: `backend/crates/golish-agent-kit/src/lib.rs`

加 `pub fn harness_stage_mode_enabled() -> bool { /* read from settings.toml */ }`

execute_single_subtask 入口判断：feature flag off → 走旧路径；on → 走 stage harness。

默认 **off**·与旧路径并行。

**Commit**: `feat(harness): feature flag harness.stage_mode_enabled (default off)`

### Phase 1c 总验证

```bash
just precommit
just test-rust
```

---

## 6. Phase 1d · 端到端验证

### Task 1d.1 · Demo stage 单测

**File**: `backend/crates/golish-agent-kit/src/harness/tests.rs`

写测试模拟「assessment profile + external_attack_surface stage」完整跑通：

1. mock ScopeService + EvidenceLedger
2. mock 3 个 evidence (dns_a / http_probe / ct_log)
3. mock deliverable
4. 跑 validate_gate
5. 断言 allowed=true / blocked + recovery_actions

**Commit**: `test(harness): add e2e demo for external_attack_surface stage`

### Task 1d.2 · Playwright e2e（可选）

仅当 Phase 1d 用户希望验证 UI 路径时·写一个 Playwright 测试：

1. 启动 just dev
2. 新建 task 模式 task
3. 启用 harness.stage_mode_enabled
4. 输入 "评估 example.com 表面 attack surface"
5. 断言 stage 走 external_attack_surface + gate 出 deliverable

**Commit**: `test(e2e): playwright demo for harness stage mode`

### Task 1d.3 · 文档更新

**Files**:

- `agent-progress.md` 记本轮实施
- `feature_list.json` 加 `harness-mvp-external-attack-surface` entry
- `docs/architecture.md` 加章节 "Operation Harness"
- 三份 Doc 1/2/3 头部 status 改 `Implemented (Phase 1)`

**Commit**: `docs(harness): record Phase 1 implementation + update feature_list`

---

## 7. 总验证（实施完成判定）

```bash
just precommit  # 必须全绿
just test-rust  # 全部 Rust 测试通过
just test-fe    # 全部前端测试通过
just test-e2e   # Playwright 测试通过（如有）
```

**手动验证**（用户做）：

1. 启动 just dev
2. settings.toml 启用 harness.stage_mode_enabled
3. 新建 task：「评估 example.com 表面 attack surface」
4. 观察 UI：
   - Stage banner 显示 "external_attack_surface · L2 active_recon"
   - Inner loop subtask 显示
   - Tool call evidence_read 出现在 timeline
   - Sprint Contract 链接显示
   - Gate decision JSON 显示在 deliverable 之后
5. 故意构造 vacuous deliverable（agent 不调任何工具就交 deliverable）→ gate 应 BLOCK

---

## 8. 风险 & 回滚

| 风险 | 缓解 |
|---|---|
| schema migration 影响生产数据 | feature flag + migration 仅 ALTER ADD COLUMN（向后兼容）+ down-migration 写好 |
| harness 路径性能差 | feature flag off 默认·与旧路径并行测·切上需 ≥ 90% 任务通过率 |
| Sprint Contract LLM 调用增本 | cross-vendor 强约束 + v0 fallback 同厂商 + 上限 token budget |
| task_orchestrator 改造破坏 PentAGI | feature flag + 单测全覆盖 + 旧路径不动 |
| ts-rs 类型链断 | PlannedSubtask 加新字段而非删 + `#[serde(default)]` 保后向兼容 |

**回滚**：feature flag off + revert task_orchestrator/execute.rs 改动·schema 不动可保留。

---

## 9. 不在本计划范围（Phase 2+）

- enumeration stage（next stage）
- pentest profile + L3-L5 authz
- vuln_triage / verification stage
- Harness Lab (AHE-style)
- MCP resource server (Phase 1 仅 Tauri command)
- 二阶 LLM vacuous detector

---

## 10. 估时

| Phase | 任务数 | 估时 |
|---|---|---|
| 1a Evidence Ledger | 4 tasks | 1-1.5 工作日 |
| 1b MCP Resource | 3 tasks | 0.5-1 工作日 |
| 1c Stage Harness MVP | 7 tasks | 1.5-2 工作日 |
| 1d E2E 验证 | 3 tasks | 0.5 工作日 |
| **总计** | 17 tasks | **3.5-5 工作日** |

注：上述估时基于 Rust 单 agent 顺序实施。如 cursor agent 自动执行，所需时间应在 1-2 个工作日内完成。

---

## 11. 后续

- Phase 1 实施完毕后 → Doc 1/2/3 进入 Implemented 状态
- 用户在 chat panel 用 task 模式 + harness 跑一次完整 assessment 流程验证
- 验证通过 → 启动 Phase 2 (enumeration stage)
- 跑数据收集（trace + evidence + gate result）→ 启动 Phase 4 (Harness Lab)

---

## 12. 状态

**Implemented (Phase 1)** · 全部 17 个 Task 落地 · 16 commits（含 Commit 0 feature_list.json 切换） · 详见 feature_list.json `harness-mvp-external-attack-surface` entry 的 evidence 字段.

主要 commit 索引：
- Phase 1a (Evidence Ledger schema): 1792885 / e5eb552 / 03f24fa / af60bc3
- Phase 1b (MCP Resource): aa7e6bf / b215046 / ffee39a
- Phase 1c (Stage Harness MVP): 163f04e / 559416f / bb98f3e / 1bcdc52 / 52f70d4 / 1b0a23e / 10dd927
- Phase 1d (e2e demo + docs): b106d55 / (本 commit)
- Doc 4 (Observability Plane): Acknowledged (Phase 1 partial-satisfy)·完整 Observability 推 Phase 2+

Plan 偏差修正记录见 feature_list.json notes 字段。
