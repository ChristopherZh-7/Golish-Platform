# Engine v2 · P0 Evidence 写入闭环 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
>
> **执行约束**：commit / push 按 AGENTS.md §2.7（未经用户授权不 commit、不 push）。每个 Task 后跑该 Task 的 `验证` 命令；Rust 编译慢，可按 Task 末尾的指定命令局部编译，不必每行都 `just check`。

**目标：** 把 Evidence Ledger 从「schema+读路径有、写入没接」补成「工具产出自动入账（带 OpenFang 哈希链）+ gate 回查真证据」，让 gate 从「信 AI 自报 `evidence_refs`」升到「可交叉验证」。

**架构：** 工具执行后置 hook → `EvidenceLedger::append()` 写 `audit_log(audit_role='evidence')` 行（+ sha256 哈希链）+ `evidence_classifications` 分类行 → gate 用 `freshness_check::run_with_freshness`（已存在）+ ledger 真计数回查。复用现有读路径（`evidence_read`）、域类型、`ScopeService` trait。

**技术栈：** Rust 2021（`golish-pentest` 域层 + `golish-db` repo + `golish-agent-kit` gate + `golish-agent-runtime` tool_dispatch）、sqlx、嵌入式 Postgres、sha2。Feature flag `GOLISH_HARNESS_STAGE_MODE`（默认 ON）。

---

## 0. 现状（2026-06-02 本会话亲核）

| 件 | 文件 | 状态 |
|---|---|---|
| schema（audit_log.audit_role + evidence_classifications + stage_runs）| `golish-db/migrations/20260601000001_evidence_ledger.sql` | ✅ 已落 |
| 读路径 `evidence_read` | `golish-pentest-app/src/evidence.rs` | ✅ 完整 + 单测 |
| 域类型 + `ScopeService` trait + `InMemoryScopeService` | `golish-pentest/src/evidence_ledger/{mod,types}.rs` | ✅ |
| audit_log 写入 `log_operation` / `log_operation_with_lineage` | `golish-db/src/repo/audit/mod.rs` | ✅（但不 set audit_role，默认 'action'）|
| gate `validate_stage_gate` + `freshness_check::run_with_freshness` | `golish-agent-kit/src/harness/gate/{mod,freshness_check}.rs` | ⚠️ mod 调 sanity `run()`，未喂 ledger |
| **`EvidenceLedger::append()`** | `evidence_ledger/mod.rs` | ❌ 仅注释（Phase 1b 推迟）|
| **tool_dispatch 后置入账 hook** | `golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs` | ❌ 未接 |

> 核心：**不是从零建**，是补「写入 append + gate 回查」两段，复用一堆现成件。

---

## 1. 文件结构（创建/修改一览）

- **修改** `backend/crates/golish-db/src/repo/audit/mod.rs`：加 `log_evidence`（set `audit_role='evidence'` + 返回 `AuditEntry`）。
- **创建** `backend/crates/golish-pentest/src/evidence_ledger/hash_chain.rs`：sha256 哈希链纯函数（borrow OpenFang）。
- **创建** `backend/crates/golish-pentest/src/evidence_ledger/append.rs`：`EvidenceLedger` struct + `append()`（编排 hash + log_evidence + classify）。
- **修改** `backend/crates/golish-pentest/src/evidence_ledger/mod.rs`：`pub mod hash_chain; pub mod append;` + re-export。
- **修改** `backend/crates/golish-db/src/repo/evidence_classifications.rs`：确认/加 `insert_current`（写 valid_to=NULL 当前分类）。
- **修改** `backend/crates/golish-agent-kit/src/harness/gate/mod.rs`：加 `validate_stage_gate_with_ledger`（喂 evidence_kinds/ages 给 `run_with_freshness`）。
- **修改** `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs`：工具执行**后**调 `EvidenceLedger::append`（flag 内）。
- **修改** `backend/crates/golish-pentest/Cargo.toml`：确认 `sha2` 依赖。

---

## 2. 总体设计决策（保证 task 间类型一致）

- **D-a · 哈希链存 JSON（MVP）**：evidence 行的 `hash`/`prev_hash` 存进 `audit_log.detail` JSON（`detail.hash` / `detail.prev_hash`），不改表结构（向后兼容；列化推 P2）。
- **D-b · append 返回 `EvidenceAuditId`**：`log_evidence` 返回 `AuditEntry`，`append()` 取 `entry.id` 包成 `EvidenceAuditId::new(entry.id)`（与读路径 `evidence_read` 用 `row.id` 一致）。
- **D-c · gate 双入口**：保留现有 `validate_stage_gate`（sanity）；新增 `validate_stage_gate_with_ledger`（喂 ledger 数据走 `run_with_freshness`）。运行时 hook 在 flag on + ledger 可用时调后者，否则回退前者。**不破坏现有单测。**
- **D-d · ScopeService MVP**：P0 先用 `InMemoryScopeService`（默认 InScope）保证闭环跑通；生产版（查 `organizations.scope_rules`）作为 P0 末的可选 Task 7，不阻塞主链。

---

## 3. Tasks

### Task 1 · DB writer：`log_evidence`

**文件：** 修改 `backend/crates/golish-db/src/repo/audit/mod.rs`

**步骤：** 在 `log_operation_with_lineage` 之后加（仿其 INSERT，多 bind `audit_role`）：

```rust
/// 写一条 evidence 行 (audit_role='evidence'). 返回带 id 的 AuditEntry,
/// 调用方 (EvidenceLedger::append) 取 .id 包成 EvidenceAuditId.
#[allow(clippy::too_many_arguments)]
pub async fn log_evidence(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    project_path: Option<&str>,
    source: &str,
    target_id: Option<Uuid>,
    session_id: Option<&str>,
    tool_name: Option<&str>,
    detail: &Value,
    run_id: Option<Uuid>,
) -> Result<AuditEntry> {
    let row = sqlx::query_as::<_, AuditEntry>(
        r#"INSERT INTO audit_log
               (action, category, details, project_path, source,
                target_id, session_id, tool_name, status, detail,
                run_id, audit_role)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'completed', $9, $10, 'evidence')
           RETURNING *"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(project_path)
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(detail)
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}
```

**确认点：** `AuditEntry`（`crate::models::AuditEntry`）的 `RETURNING *` 必须能反序列化新 `audit_role` 列——若 `AuditEntry` 结构体未含 `audit_role` 字段，给它加 `pub audit_role: String`（migration 已建列，默认 'action'，老行不破）。

**验证：** `cargo check -p golish-db` → exit 0。

---

### Task 2 · 哈希链纯函数（borrow OpenFang）

**文件：** 创建 `backend/crates/golish-pentest/src/evidence_ledger/hash_chain.rs`

**步骤：**

```rust
//! Evidence 防篡改哈希链 (borrow OpenFang Merkle-style audit chain).
//!
//! 每条 evidence 的 hash = sha256(prev_hash ‖ canonical_detail ‖ created_at_rfc3339).
//! prev_hash = 同 operation 上一条 evidence 的 hash (首条用 GENESIS).

use sha2::{Digest, Sha256};

pub const GENESIS: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// 计算一条 evidence 的链式哈希. `canonical_detail` 应是稳定序列化的 detail
/// (调用方用 serde_json::to_string 后传入; 字段顺序由 struct 定义固定).
pub fn chain_hash(prev_hash: &str, canonical_detail: &str, created_at_rfc3339: &str) -> String {
    let mut h = Sha256::new();
    h.update(prev_hash.as_bytes());
    h.update(b"\x1f"); // unit separator, 防拼接歧义
    h.update(canonical_detail.as_bytes());
    h.update(b"\x1f");
    h.update(created_at_rfc3339.as_bytes());
    hex::encode(h.finalize())
}

/// 校验一段链: 给定按时间升序的 (prev_hash, canonical_detail, created_at, hash),
/// 逐条重算并比对. 返回第一处断裂的索引 (None = 链完整).
pub fn verify_chain(rows: &[(String, String, String, String)]) -> Option<usize> {
    let mut expect_prev = GENESIS.to_string();
    for (i, (prev, detail, created, hash)) in rows.iter().enumerate() {
        if prev != &expect_prev {
            return Some(i);
        }
        if &chain_hash(prev, detail, created) != hash {
            return Some(i);
        }
        expect_prev = hash.clone();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_hash_is_deterministic() {
        let a = chain_hash(GENESIS, r#"{"kind":"dns_a"}"#, "2026-06-02T00:00:00Z");
        let b = chain_hash(GENESIS, r#"{"kind":"dns_a"}"#, "2026-06-02T00:00:00Z");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn chain_detects_tamper() {
        let h1 = chain_hash(GENESIS, r#"{"kind":"dns_a"}"#, "t1");
        let h2 = chain_hash(&h1, r#"{"kind":"http_probe"}"#, "t2");
        let mut rows = vec![
            (GENESIS.to_string(), r#"{"kind":"dns_a"}"#.to_string(), "t1".to_string(), h1.clone()),
            (h1.clone(), r#"{"kind":"http_probe"}"#.to_string(), "t2".to_string(), h2.clone()),
        ];
        assert_eq!(verify_chain(&rows), None);
        // 篡改第 2 条 detail → 链在 index 1 断裂
        rows[1].1 = r#"{"kind":"TAMPERED"}"#.to_string();
        assert_eq!(verify_chain(&rows), Some(1));
    }
}
```

**确认点：** `Cargo.toml` 加 `sha2` 与 `hex`（若未在 `golish-pentest` 依赖树）。`grep sha2` 确认版本，沿用 workspace 统一版本。

**验证：** `cargo nextest run -p golish-pentest -E 'test(evidence_ledger::hash_chain)'` → 2/2。

---

### Task 3 · `EvidenceLedger::append()`

**文件：** 创建 `backend/crates/golish-pentest/src/evidence_ledger/append.rs`

**步骤：**

```rust
//! EvidenceLedger::append() · 工具产出入账 (Phase 1b 补完).
//!
//! 编排: 取上一条 hash → 算本条 hash → log_evidence 写 audit_log(audit_role='evidence')
//!       → ScopeService 分类 → 写 evidence_classifications. 返回 EvidenceAuditId.

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::hash_chain::{chain_hash, GENESIS};
use super::types::EvidenceAuditId;
use super::ScopeService;

/// 一条工具产出的入账输入.
pub struct EvidenceInput<'a> {
    pub kind: &'a str,          // e.g. "dns_a" / "http_probe"
    pub subject: &'a str,       // e.g. "api.example.com"
    pub raw_output: &'a str,    // 工具原始输出 (后续 evidence_read 经 sanitizer)
    pub tool_name: &'a str,
    pub operation_id: Uuid,     // = task_id
    pub stage_run_id: Option<Uuid>,
    pub project_path: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

/// 取同 operation 最新一条 evidence 的 hash (detail->>'hash'), 没有则 GENESIS.
async fn prev_hash_for(pool: &PgPool, operation_id: Uuid) -> String {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r#"SELECT al.detail->>'hash'
           FROM audit_log al
           WHERE al.audit_role = 'evidence' AND al.run_id = $1
           ORDER BY al.id DESC
           LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    row.and_then(|(h,)| h).unwrap_or_else(|| GENESIS.to_string())
}

/// 写一条 evidence (含哈希链) + 当前分类行. 返回新 evidence 的 id.
pub async fn append(
    pool: &PgPool,
    scope: &dyn ScopeService,
    input: EvidenceInput<'_>,
) -> Result<EvidenceAuditId, crate::PentestError> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let prev = prev_hash_for(pool, input.operation_id).await;

    // detail 不含 hash 时先算 canonical, 再把 hash 塞回 detail 落库.
    let mut detail = json!({
        "kind": input.kind,
        "subject": input.subject,
        "raw_output": input.raw_output,
        "prev_hash": prev,
    });
    let canonical = serde_json::to_string(&detail).unwrap_or_default();
    let hash = chain_hash(&prev, &canonical, &created_at);
    detail["hash"] = json!(hash);

    let entry = golish_db::repo::audit::log_evidence(
        pool,
        &format!("{}_completed", input.tool_name),
        "evidence",
        &format!("[{}] {}", input.kind, input.subject),
        input.project_path,
        "harness",
        None,
        input.session_id,
        Some(input.tool_name),
        &detail,
        Some(input.operation_id),
    )
    .await
    .map_err(|e| crate::PentestError::Db(e.to_string()))?;

    let eid = EvidenceAuditId::new(entry.id);

    // 当前分类行 (valid_to=NULL). MVP: InMemory 默认 InScope.
    let label = scope.classify_subject(input.subject);
    golish_db::repo::evidence_classifications::insert_current(
        pool,
        entry.id,
        label,                       // 序列化为 'in_scope' 等
        scope.snapshot_version().as_i64(),
        input.stage_run_id,
        "tool_output_append",
    )
    .await
    .map_err(|e| crate::PentestError::Db(e.to_string()))?;

    Ok(eid)
}
```

**确认点：** ① `crate::PentestError` 变体名（看 `golish-pentest/src/lib.rs` 实际错误枚举，用其 `Db`/`Database` 对应变体）。② `golish-db::repo::evidence_classifications::insert_current` 签名——若不存在则在 Task 4 先建。③ `golish-pentest` 是否已依赖 `golish-db`（看 Cargo.toml；evidence 域层原设计 DB 落 golish-db，append 编排可放 golish-pentest-app 若分层要求 domain 不依赖 db——届时把 append.rs 移到 `golish-pentest-app/src/evidence_append.rs`，逻辑不变）。

**验证：** `cargo check -p golish-pentest`（或 `-p golish-pentest-app`）→ exit 0。

---

### Task 4 · classifications writer：`insert_current`

**文件：** 修改 `backend/crates/golish-db/src/repo/evidence_classifications.rs`

**步骤：** 确认是否已有写当前分类的函数；没有则加：

```rust
use golish_pentest::evidence_ledger::EvidenceScopeLabel;

/// 写一条「当前」分类行 (valid_to=NULL). partial unique index 保证同一
/// evidence 只有一条 current. 重复写同一 evidence 应先 close 旧行 (relabel
/// 路径; P0 append 只在新建 evidence 后写一次, 不冲突).
pub async fn insert_current(
    pool: &PgPool,
    evidence_audit_id: i64,
    label: EvidenceScopeLabel,
    scope_version: i64,
    producing_stage_run_id: Option<Uuid>,
    reason: &str,
) -> Result<i64> {
    let classification = match label {
        EvidenceScopeLabel::InScope => "in_scope",
        EvidenceScopeLabel::OutOfScope => "out_of_scope",
        EvidenceScopeLabel::DerivedFromOutOfScope => "derived_from_out_of_scope",
    };
    let (id,): (i64,) = sqlx::query_as(
        r#"INSERT INTO evidence_classifications
               (evidence_audit_id, classification, scope_version,
                valid_from, reason, classified_by_session, producing_stage_run_id)
           VALUES ($1, $2, $3, NOW(), $4, 'harness', $5)
           RETURNING id"#,
    )
    .bind(evidence_audit_id)
    .bind(classification)
    .bind(scope_version)
    .bind(reason)
    .bind(producing_stage_run_id)
    .fetch_one(pool)
    .await?;
    Ok(id)
}
```

**确认点：** 列名与 migration 对齐（`classified_by_session` NOT NULL → 传 'harness'；`schema_v` 有默认）。

**验证：** `cargo check -p golish-db` → exit 0。

---

### Task 5 · tool_dispatch 后置入账 hook

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs`

**步骤：**
1. 先读该文件，定位「工具执行**完成、拿到结果**」的点（已有 harness gate / authorizer 接缝在此文件，见 explainer §6）。
2. 在该点后插入（flag 内、失败不阻断主流程——证据入账失败只 warn）：

```rust
// P0: harness on 时把工具产出入账 evidence ledger (失败仅 warn, 不阻断).
if crate::harness::stage_mode_enabled() {
    if let (Some(op_id), Some(pool)) = (exec_ctx.operation_id, exec_ctx.db_pool.as_ref()) {
        let input = golish_pentest::evidence_ledger::append::EvidenceInput {
            kind: tool_kind_for(&tool_name),     // 工具名→evidence kind 映射 (见确认点)
            subject: &observed_subject,          // 从工具结果抽 (host/url); 缺则用 tool_name
            raw_output: &tool_result_text,
            tool_name: &tool_name,
            operation_id: op_id,
            stage_run_id: exec_ctx.stage_run_id,
            project_path: exec_ctx.project_path.as_deref(),
            session_id: exec_ctx.session_id.as_deref(),
        };
        let scope = golish_pentest::evidence_ledger::InMemoryScopeService::new(
            golish_pentest::evidence_ledger::ScopeVersion::new(1),
        );
        if let Err(e) =
            golish_pentest::evidence_ledger::append::append(pool, &scope, input).await
        {
            tracing::warn!(target: "harness::evidence", error = %e, "evidence append failed (continuing)");
        }
    }
}
```

**确认点（必须读文件后定）：** ① `exec_ctx` 是否已带 `operation_id` / `db_pool` / `stage_run_id` / `session_id`——若没有，从 `task_orchestrator` 侧信道（C3 已把 `HarnessAuthz` 穿到 dispatch，见 `harness-full-impl` 计划）补穿这几个字段。② `tool_result_text` / `observed_subject` 的真实变量名。③ `tool_kind_for` 映射：先做 `fn tool_kind_for(t:&str)->&str { t }`（直接用工具名，MVP），P3 再细化。④ 确认 `golish-agent-runtime` 依赖 `golish-pentest`。

**验证：** `cargo check -p golish-agent-runtime` → exit 0。

---

### Task 6 · gate 回查真证据：`validate_stage_gate_with_ledger`

**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/gate/mod.rs`

**步骤：** 加新入口（复用现有 `freshness_check::run_with_freshness`，它已实现真 max_age）：

```rust
use std::collections::HashMap;
use std::time::Duration as StdDuration;
use golish_pentest::evidence_ledger::EvidenceAuditId;

/// 带 ledger 的 gate: 结构 check 同 validate_stage_gate, 但 freshness 走
/// run_with_freshness (真 max_age). evidence_kinds/ages 由调用方查 ledger 填.
pub fn validate_stage_gate_with_ledger(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
    evidence_kinds: &HashMap<EvidenceAuditId, String>,
    evidence_ages: &HashMap<EvidenceAuditId, StdDuration>,
) -> GateResult {
    let mut outcomes = vec![
        schema_check::run(deliverable, spec),
        contract_check::run(deliverable, contract),
        vacuous_check::run(deliverable, spec),
        freshness_check::run_with_freshness(deliverable, spec, evidence_kinds, evidence_ages),
    ];
    let mut ran: HashSet<&'static str> = HashSet::new();
    for name in &spec.required_checks {
        let check_id = match name.as_str() {
            "scope_status_present" | "out_of_scope_targets_excluded" => "scope",
            "surface_workbench_coverage" => "surface_coverage",
            "min_tool_invocations_per_check" => "min_invocations",
            _ => continue,
        };
        if !ran.insert(check_id) { continue; }
        outcomes.push(match check_id {
            "scope" => scope_check::run(deliverable),
            "surface_coverage" => surface_coverage_check::run(deliverable),
            "min_invocations" => min_invocations_check::run(deliverable, spec),
            _ => unreachable!(),
        });
    }
    aggregate(outcomes)
}
```

> contract/min_invocations 的「真计数」升级（读 ledger 而非 `evidence_refs` 长度近似）作为 P0 末 stretch 或 P2 接入；P0 先把 freshness 走真版 + 数据从 ledger 来，已显著优于「全信自报」。

**确认点：** gate hook 调用方（`task_orchestrator/subtask_phases/execute.rs::apply_harness_gate_hook`）需查 ledger 填 `evidence_kinds`（detail->>'kind'）+ `evidence_ages`（NOW()-created_at），按 `deliverable.evidence_refs` 的 id 批量查 `audit_log`。

**验证：** `cargo nextest run -p golish-agent-kit -E 'test(harness::gate)'` → 现有全绿 + 新增「假 refs 被 BLOCK」测试通过。

---

### Task 7 · 集中编译 + 测试 + 验收

**步骤：**
1. `cargo check -p golish-db -p golish-pentest -p golish-agent-kit -p golish-agent-runtime` → 全 exit 0（改错）。
2. `cargo nextest run -p golish-pentest -E 'test(evidence_ledger)'` → append + hash_chain 全绿。
3. `cargo nextest run -p golish-agent-kit -E 'test(harness)'` → 现有 + 新 gate 测全绿。
4. `cargo clippy -p golish-pentest -p golish-db -p golish-agent-kit -- -D warnings` → exit 0。
5. `just precommit` → 全绿（结尾跑一次）。
6. **活体（用户·需运行时）**：`GOLISH_HARNESS_STAGE_MODE=true just dev` → task『评估 <target> 的 external attack surface』→ 跑后查
   `SELECT count(*) FROM audit_log WHERE audit_role='evidence' AND run_id=<task_id>` > 0；
   `evidence_read` 读回一条；用 `hash_chain::verify_chain` 重算该 operation 的 evidence 链返回 None（完整）。

**验收（全满足才算 P0 done）：** 上 1–5 全绿 + 6 活体证据记录进 `agent-progress.md` 的「已记录证据」段（命令 + 退出码 + 关键输出）。

---

## 4. 自检（writing-plans）

- **规格覆盖**：本计划覆盖设计 §4 全部组件（append/log_evidence/hash-chain/classify/gate-with-ledger/tool hook）；生产 ScopeService（设计 §4.3）标为 P0 末可选（D-d），不阻塞主链。
- **类型一致**：`EvidenceInput` / `EvidenceAuditId` / `EvidenceScopeLabel` / `AuditEntry` 跨 Task 命名一致；`append()` 返回 `EvidenceAuditId`（Task 3）= gate 查 ledger 用的 key（Task 6）。
- **确认点**：每个 Task 标了「确认点」= 实现前必须读真实文件确认的现有签名（非占位符，是防我没亲眼见的签名漂移）。
