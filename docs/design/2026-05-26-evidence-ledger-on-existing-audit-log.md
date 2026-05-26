# Evidence Ledger on Existing audit_log

- **Author**: MCP-1
- **Date**: 2026-05-26
- **Status**: Implemented (Phase 1 · 见 commits 1792885 / e5eb552 / 03f24fa / af60bc3)
- **Source of truth**: `docs/design/2026-05-26-operation-harness-profile-dag-lab.md` §21
- **Audience**: 后续起草 Doc 2 / Doc 3 / implementation plan 的 agent + 后续 Phase 1 实施者

> 本文是 Doc 1·三份拆分中的第一份·Phase 0 design only。
>
> 仅为设计提案 / 接口 / schema 草案·**不动** `migrations/*.sql` 不动 `backend/crates/`。
>
> Phase 1 实施需获得用户 §AGENTS.md §2.7 明示授权。

---

## 1. 目标

把现有 `audit_log` 表升级为 Evidence Ledger·支持 IFC（Information Flow Control）语义的 scope 分类和因果回溯。

不新建 EvidenceRecord 表（与 §13.9 / F1 不变量一致：不造与现有重复的新表）。

---

## 2. 设计概览

```text
audit_log (现有 · pure append) 增加 audit_role 字段表达语义角色
  ↓ audit_role='evidence' 的行 = evidence 本体
evidence_classifications (新表 · append-only · bitemporal) 加 IFC 分类层
  ↓ 当前 classification = WHERE valid_to IS NULL
ScopeService (新 trait) 决定每条 evidence 的 ScopeLabel
  ↓ validate_relabel 守 invariant
re-label event 走 audit_role='approval' 的用户审批 + bitemporal 写新行
```

3 个新概念：

| 概念 | 是什么 | 在哪 |
|---|---|---|
| `audit_role` | audit_log 行的语义角色（action / evidence / classification / approval） | 给 audit_log 加 1 个 TEXT 字段 |
| `evidence_classifications` | IFC 分类层 + 因果回溯 | 新表（bitemporal） |
| `ScopeService` | 独立分类器·一手决 ScopeLabel | 新 trait |

---

## 3. Schema 草案

### 3.1 audit_log 加 audit_role 字段

```sql
ALTER TABLE audit_log
    ADD COLUMN IF NOT EXISTS audit_role TEXT NOT NULL DEFAULT 'action';
-- audit_role ∈ {'action', 'evidence', 'classification', 'approval'}

CREATE INDEX IF NOT EXISTS audit_log_audit_role_idx
    ON audit_log(audit_role);
```

**为什么不新建 `evidence_records` 表**：

- 现有 `audit_log` 已是 fact + status 三态 + detail JSONB 的合规审计行
- evidence 本质上是「一次 tool call 的输出 fact」，与 audit row 是同一个抽象
- 给 audit_log 加角色字段比拆出新表更顺手·与现有 PentestAudit 同构

**audit_role 四个取值的语义**：

| audit_role | 含义 | detail JSONB 内容 |
|---|---|---|
| `action` | 现有行为·普通审计行 | 工具调用 / 用户操作 / status 变更等 |
| `evidence` | evidence 本体·完整工具原始输出 | `{raw_output: "...", parsed: {...}, subject: "...", derived_from: [audit_id]}` |
| `classification` | re-label 历史链 | 给 evidence_classifications 表的 cross-reference |
| `approval` | 用户审批记录 | `{kind: "...", scope_changes_json: {...}, expires_at: "..."}` |

**application-level 约束**：

- `evidence_classifications.evidence_audit_id` 应通过 application 检查只能指向 `audit_role='evidence'` 行
- abandoned 行（status='abandoned'）不能被 evidence_classifications 引用（§5.3 fire-and-forget reclaim 规则）

### 3.2 evidence_classifications (新表 · bitemporal)

```sql
CREATE TABLE evidence_classifications (
    id BIGSERIAL PRIMARY KEY,
    evidence_audit_id BIGINT NOT NULL REFERENCES audit_log(id),
    classification TEXT NOT NULL,           -- 'in_scope' / 'out_of_scope' / 'derived_from_out_of_scope'
    scope_version BIGINT NOT NULL,          -- 对应 organizations.scope_rules_version 当时快照
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    valid_to TIMESTAMPTZ,                   -- NULL = 当前 active
    reason TEXT NOT NULL,
    relabel_decision TEXT,                  -- validate_relabel 返回的决策代号（如 'ScopeNarrowing' / 'ApprovedExpansion'）
    classified_by_session TEXT NOT NULL,    -- 哪个 Tauri session 触发的分类
    producing_stage_run_id UUID,            -- stage-scoped 隔离（O4 终态）
    schema_v INT NOT NULL DEFAULT 1
);

-- 唯一性：同一 evidence 的「当前」分类只能有一条
CREATE UNIQUE INDEX evidence_classifications_current_idx
    ON evidence_classifications(evidence_audit_id)
    WHERE valid_to IS NULL;

-- 查询性能：常按 producing_stage_run_id 过滤
CREATE INDEX evidence_classifications_stage_idx
    ON evidence_classifications(producing_stage_run_id)
    WHERE valid_to IS NULL;
```

**字段语义**：

- `valid_from / valid_to`：bitemporal 范围。当前活跃行 valid_to IS NULL。re-label 时关闭老行（设 valid_to=NOW()）+ 插新行（valid_from=NOW()）。
- `scope_version`：当时的 `organizations.scope_rules_version` 快照。Resume 时 ScopeService 锁定此值不读最新 rules（O7 race 解决方案）。
- `producing_stage_run_id`：哪个 stage 产生的这个 evidence。Gate 默认 `WHERE producing_stage_run_id = $current` 隔离跨 stage 误读。
- `classified_by_session`：哪个 session 触发分类。multi_session_relay 下 cross-session derived_from 检查需要它。

### 3.3 organizations.scope_rules_version

```sql
ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS scope_rules_version BIGINT NOT NULL DEFAULT 1;
```

任何 `organizations.scope_rules` 修改 → application 把 version +1。ScopeService 启动时锁定 cursor.last_scope_version 不读最新。

### 3.4 operation_state cursor 表（resume 写入侧）

```sql
CREATE TABLE operation_state (
    operation_id UUID PRIMARY KEY,
    profile TEXT NOT NULL,
    current_stage TEXT NOT NULL,
    stage_started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_evidence_audit_id BIGINT,
    last_classification_id BIGINT REFERENCES evidence_classifications(id),
    last_scope_version BIGINT,
    state_blob JSONB NOT NULL DEFAULT '{}',  -- harness 私有 resume 状态
    superseded_by UUID REFERENCES operation_state(operation_id)  -- cross-profile transition
);
```

**重要 § 21.5.4**：这是 cursor 表（resume 用），**不是** operations 表。没有 valid_until / authz_level / scope（那些走 targets / organizations）。这是用户 2026-05-17 删除 engagements 后唯一可接受的新表形状。

---

## 4. 模型决议（Rust 草案）

### 4.1 EvidenceScopeLabel 三变体

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScopeLabel {
    InScope,
    OutOfScope,
    DerivedFromOutOfScope,
}
```

**删除 Unverified**（§14.1 让步）：避免「谁来 verify」陷阱。scope 边界变动走 re-label event 而非 Unverified fallback。

### 4.2 ScopeService trait

```rust
pub trait ScopeService: Send + Sync {
    /// 启动时锁定 scope_version snapshot
    fn snapshot_version(&self) -> ScopeVersion;

    /// 基于当前 snapshot 给一个 subject (URL/IP/Domain) 分类
    fn classify_subject(&self, subject: &str) -> EvidenceScopeLabel;

    /// 给一组 derived_from evidence 推断继承的 label
    fn propagate_from_parents(&self, parents: &[EvidenceId]) -> EvidenceScopeLabel;
}
```

**实现位置**：`backend/crates/golish-pentest/src/scope_service.rs`（Phase 1 实施时）

**默认实现**：从 `organizations.scope_rules` 读 in_scope_set / out_of_scope_set，在 snapshot 后只读 in-memory 缓存。

### 4.3 EvidenceLedger trait

```rust
pub struct EvidenceLedger<'a> {
    pool: &'a PgPool,
    scope_service: Arc<dyn ScopeService>,
}

impl<'a> EvidenceLedger<'a> {
    /// 写一条 evidence + 一条 classification
    pub async fn append(&self, raw: ToolOutput) -> Result<EvidenceAuditId> {
        // 1. write audit_log row with audit_role='evidence'
        // 2. compute initial label via scope_service.classify_subject(raw.subject)
        // 3. compute inherited label via scope_service.propagate_from_parents(raw.derived_from)
        // 4. label = lattice_join(initial, inherited)
        // 5. write evidence_classifications row (valid_from=NOW(), valid_to=NULL)
        // ...
    }

    /// 读 evidence 当前 label
    pub async fn current_label(&self, eid: EvidenceAuditId) -> Result<EvidenceScopeLabel>;

    /// re-label 走 validate_relabel + bitemporal 写新行
    pub async fn relabel(
        &self,
        eid: EvidenceAuditId,
        new: EvidenceScopeLabel,
        ctx: RelabelContext,
    ) -> Result<()>;

    /// backward_slice: 给一个 evidence 回溯所有因果祖先 (PCAS 启发)
    pub async fn backward_slice(&self, seed: EvidenceAuditId) -> Result<Vec<EvidenceAuditId>>;
}
```

### 4.4 validate_relabel invariant guards (MCP-4 提)

```rust
pub fn validate_relabel(
    old: EvidenceScopeLabel,
    new: EvidenceScopeLabel,
    ctx: &RelabelContext,
) -> Result<RelabelDecision, RelabelError> {
    use EvidenceScopeLabel::*;
    match (old, new) {
        // 收紧 scope 总是允许
        (InScope, OutOfScope) => Ok(RelabelDecision::ScopeNarrowing),

        // 扩 scope 需用户 approval
        (OutOfScope, InScope) if ctx.has_user_approval() => Ok(RelabelDecision::ApprovedExpansion),
        (OutOfScope, InScope) => Err(RelabelError::ScopeExpansionNeedsApproval),

        // Derived 状态只能从父传播获得，不接受手动设置
        (_, DerivedFromOutOfScope) if !ctx.is_propagation_event() => {
            Err(RelabelError::IllegalDerivedSet)
        }

        // DerivedFromOutOfScope → InScope 需父 re-label 先
        (DerivedFromOutOfScope, InScope) => Err(RelabelError::NeedsParentRelabelFirst),

        _ => Ok(RelabelDecision::Other),
    }
}

pub enum RelabelDecision {
    ScopeNarrowing,
    ApprovedExpansion,
    Other,
}

pub enum RelabelError {
    ScopeExpansionNeedsApproval,
    IllegalDerivedSet,
    NeedsParentRelabelFirst,
}
```

decisions / errors 都被存到 `evidence_classifications.relabel_decision` TEXT 字段，方便事后追溯。

### 4.5 RelabelContext

```rust
pub struct RelabelContext {
    pub operation_id: OperationId,
    pub approval_kind: String,                 // 'scope_expansion' / 'authz_level_grant' / ...
    pub scope_change_request: serde_json::Value,
    pub is_propagation_event: bool,
}

impl RelabelContext {
    pub fn has_user_approval(&self) -> bool {
        // 查 audit_log WHERE audit_role='approval' AND status='completed'
        // AND detail->>'kind' = self.approval_kind
        // AND detail->'scope_changes_json' @> self.scope_change_request
        // AND (detail->>'expires_at' IS NULL OR NOW() < expires_at)
        // 返回 LIMIT 1 是否存在
    }
}
```

### 4.6 SkipReason 强制枚举

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    RateLimited { tool: ToolName, after_attempts: u32 },
    ScopeRestriction { restricted_target: TargetRef },
    EnvUnavailable { tool: ToolName, error_chain: Vec<String> },
    UserRequested { user_msg_id: MsgId },
    Other { explanation: String, evidence_ref: EvidenceAuditId },
}
```

前 4 变体由 tool wrapper 自动填，agent 不可操控。
Other 是唯一 agent 可填的，**必须**带 evidence_ref 指向 audit_log 中实际的工具调用错误行。

---

## 5. 业务规则

### 5.1 IFC propagation

evidence 创建时 ScopeService 决定 label（不是 tool wrapper 也不是 LLM）：

```text
label = lattice_join(
    scope_service.classify_subject(raw.subject),
    scope_service.propagate_from_parents(raw.derived_from)
)
```

**lattice ordering**（保守上推）：

```
OutOfScope > DerivedFromOutOfScope > InScope
```

任一 parent 是 OutOfScope → child 是 DerivedFromOutOfScope。
任一 parent 是 DerivedFromOutOfScope → child 是 DerivedFromOutOfScope。
都是 InScope → child 是 InScope。

### 5.2 finding 级才拆引用

evidence 是原子单元（一次 tool call 的完整输出），不拆字段级 OOS。**finding 级**才拆：

```text
deliverable.findings[i].evidence_refs : Vec<EvidenceAuditId>
  ↓
gate 验证每个 evidence_ref 当前 label = InScope
  ↓
若发现 DerivedFromOutOfScope evidence_ref → gate BLOCK
```

### 5.3 fire-and-forget started 行 startup reclaim

```rust
/// app 启动时调一次，把 status='started' 但超过 1h 未到终态的 audit_log 行标 'abandoned'
pub async fn reclaim_abandoned_audits(pool: &PgPool, threshold: Duration) -> Result<usize> {
    let cutoff = Utc::now() - threshold;
    let result = sqlx::query!(
        "UPDATE audit_log SET status = 'abandoned' \
         WHERE status = 'started' AND started_at < $1",
        cutoff
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() as usize)
}
```

**默认 threshold**: 1 hour。
**调用位置**：`backend/crates/golish/src/lib.rs::startup_hooks()` 启动钩子（Phase 1 实施时）。
**不补的后果**：abandoned 行被 evidence_classifications 引用 → re-label 链出现「一个 evidence 有一个 abandoned 动作为父」脏状态。

### 5.4 multi_session_relay 跨 session derive

```text
session A 创 evidence X (InScope, scope_version=10, classified_by_session=A)
session B 上线, scope_version 漂到 12
session B re-derive evidence Y, derived_from=[X]
  ↓
ScopeService 检查 Y.evidence_refs[*].classified_by_session == current_session ?
  ↓
不同 → 强制重跑 ScopeService.classify_subject(X)
继承上来的 label 不能 cross-session 复用，必须基于当前 scope_version 重新分类
```

**实现位置**：`EvidenceLedger::append()` 内部走这条检查。

### 5.5 cross-profile transition

```text
operation_state(operation_id=A, profile='assessment')
  ↓ 用户从 assessment 升 pentest
operation_state(operation_id=B, profile='pentest', superseded_by=...) 
  ↓
B.last_evidence_audit_id 不复用 A 的，必须重 ScopeService 跑一次 classify
A.superseded_by 设为 B（指向新 operation_state）
```

**新建 operation_state 行不是静默升级**。`max_authorization` 变 → 原 tool_allow_set 不足 → 所有 in-flight subtask 必须重 gate。

---

## 6. 资源驱动配置（不入 DB）

### 6.1 evidence_kind_aging（O7 final）

```json
// resources/harness/evidence_kinds.json
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

**为什么走 JSON 不走 DB**：
- aging 阈值是配置量·不需 audit / version / multi-tenant
- 改阈值 = 改 PR·走 git 版本化
- 与 harness profiles / stages 同层（`resources/harness/`）
- 避免 Phase 1 多一个 DB 表 + 对应 repo

stage_spec 可 override `max_evidence_age_for_finding`。
Gate 调用顺序：`stage_spec.override` → `evidence_kinds.json default` → `7 days fallback`。

---

## 7. Repository pattern（与现有 sqlx 模式同步）

Golish 现有 sqlx repo 无 trait 抽象（每个 repo 是自由函数 + `&PgPool`）。新 repo 文件按相同模式：

```rust
// backend/crates/golish-db/src/repo/evidence_classifications.rs
use sqlx::PgPool;
use anyhow::Result;

pub async fn insert(
    pool: &PgPool,
    evidence_audit_id: i64,
    classification: &str,
    scope_version: i64,
    reason: &str,
    classified_by_session: &str,
    producing_stage_run_id: Option<Uuid>,
) -> Result<i64> { /* ... */ }

pub async fn current_for(
    pool: &PgPool,
    evidence_audit_id: i64,
) -> Result<Option<ClassificationRow>> { /* ... */ }

pub async fn close_current_open_new(
    pool: &PgPool,
    evidence_audit_id: i64,
    new: ClassificationRow,
) -> Result<i64> {
    // 事务内：UPDATE old WHERE valid_to IS NULL SET valid_to=NOW() + INSERT new
}

pub async fn list_supersedes_chain(
    pool: &PgPool,
    evidence_audit_id: i64,
) -> Result<Vec<ClassificationRow>> { /* ... */ }
```

**事务边界**：close + new 必须在一个事务里（防 race + 防 partial state）。

`partial unique index WHERE valid_to IS NULL` 保证并发时只有一条赢，输了的需重读 latest_classification + 决定是否再次 INSERT（参考 §21.7.6）。

---

## 8. Approval audit_log 子集

`audit_role='approval'` 行的 detail JSONB schema：

```json
{
  "kind": "scope_expansion",
  "scope_changes_json": {
    "in_scope_add": ["admin.example.com"],
    "in_scope_remove": [],
    "out_of_scope_add": []
  },
  "approved_by_user": "user@host",
  "expires_at": "2026-12-31T00:00:00Z",
  "requesting_session_id": "...",
  "reason_provided": "...",
  "evidence_refs": [123, 456]
}
```

audit_log status 三态映射到 approval 生命周期：

| audit_log status | approval 含义 |
|---|---|
| `started` | 用户点了 "Request scope expansion" 按钮，等待审批 |
| `completed` | 用户点了 Approve，授权生效 |
| `failed` | 用户点了 Deny，授权拒绝 |

不需要 `scope_change_requests` 表——started 行就是待处理队列。

---

## 9. 与 §21 的对应关系

| Doc 1 章节 | §21 章节 |
|---|---|
| §3.1 audit_role 字段 | §21.5.1 |
| §3.2 evidence_classifications | §21.5.2 |
| §3.3 scope_rules_version | §21.5.5 |
| §3.4 operation_state | §21.5.4 |
| §4.1 EvidenceScopeLabel | §21.6.4 |
| §4.4 validate_relabel | §21.6.7 |
| §4.6 SkipReason | §21.6.5 |
| §5.3 startup reclaim | §21.8.4 |
| §5.4 cross-session re-classify | §21.6.6 (agent_continuity) + §15.5/16.4 |
| §6.1 evidence_kinds.json | §21.5.7 |
| §8 approval audit_log | §21.5.6 + §21.7.2 |

---

## 10. 实施前置依赖

Phase 1 实施 Doc 1 之前必须满足：

- `just precommit` 切绿（5 clippy + 2 baseline test failure 修完）
- `asset-intel-hydrate-disambiguation` 切 passing
- 用户明示 §AGENTS.md §2.7 授权 schema migration

---

## 11. 风险

| 风险 | 缓解 |
|---|---|
| ScopeService 实现错误·把 InScope evidence 误判 OOS·导致 gate 大量 false BLOCK | feature flag `harness.scope_service_enabled` 默认关·先与现有路径并行运行 |
| bitemporal `partial unique index` 在并发 INSERT 触发死锁 | 写新行前先尝试 `INSERT ... ON CONFLICT DO NOTHING`·失败则重读 latest |
| audit_log `audit_role` 字段索引膨胀 | 仅在 audit_role IS NOT NULL 时建索引（Postgres 部分索引） |
| `cross-session derived_from` 重跑 classifier 性能损失 | 仅在 producer_session != current_session 时触发·常态零成本 |

---

## 12. 不做（与 §21.3 不变量一致）

- 不新建 `evidence_records` 表（用 audit_log + audit_role）
- 不新建 `user_approvals` 表（用 audit_role='approval'）
- 不引入 saga 框架（PentestAudit 天然 saga-friendly）
- 不重构 task_orchestrator（scope creep）
- 不新增 4 个 crate（在现有 golish-db / golish-pentest / golish-agent-kit 加 module）

---

## 13. 后续

- Doc 2 (`mcp-resource-evidence-summary.md`) 会引用本 Doc 的 `EvidenceAuditId` / `EvidenceLedger`
- Doc 3 (`stage-harness-mvp-external-attack-surface.md`) 会引用本 Doc 的 `ScopeLabel` / `evidence_classifications.producing_stage_run_id`
- Plan (`task-mode-refactor-to-harness.md`) 会按本 Doc 的 schema + trait 拆 Phase 1 实施步骤

---

## 14. 状态

**Discussion Draft** · 待 user 明示 §2.7 授权进入 Phase 1 实施。
