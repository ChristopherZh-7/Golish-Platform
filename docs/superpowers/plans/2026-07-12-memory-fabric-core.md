# Evidence-backed Memory Fabric Core 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 建立带 scope、source version、evidence、有效期和失效语义的 Episode/Assertion/Document/Outbox，使长期知识从 canonical facts 可靠投影，而不是从 tool stdout 或模型 prose 自动保存。

**架构：** 新建纯领域 crate `golish-memory-domain` 和应用 crate `golish-memory-app`；`OperationScope` 只描述单次运行，`AssertionVisibility` 单独描述跨 operation 的组织长期知识或 `global_sanitized` 知识。canonical writer 在同一短事务写 immutable outbox event 与每个 projector 的 delivery；projector 只 claim 自己的 delivery，并在事务外工作。Assertion 是长期知识权威来源，Document/Embedding/KG 都是可失效、可重建投影。现有通用 `memories` 表保持兼容，但不再作为 harness stage 的客户事实源。

**技术栈：** Rust 2021、sqlx/PostgreSQL/pgvector、serde、async-trait、cargo-nextest。

**依赖：** P1 Runtime Foundation 已提供 `project_scopes` registry、frozen snapshot 的 `project_scope_id` 与 StageRunUnit/Handoff，以及 P2 的 CandidateAttempt schema/terminalizer contract；实施 migration 前取得用户确认。

---

## 1. 文件结构

### 新建 crate

- `backend/crates/golish-memory-domain/{Cargo.toml,src/lib.rs}`
- `backend/crates/golish-memory-domain/src/{scope.rs,episode.rs,assertion.rs,classification.rs,event_catalog.rs}`
- `backend/crates/golish-memory-app/{Cargo.toml,src/lib.rs}`
- `backend/crates/golish-memory-app/src/{ports.rs,promotion.rs,invalidation.rs,outbox.rs,supervisor.rs}`

### 新建 DB

- `backend/crates/golish-db/migrations/20260712000003_memory_fabric_core.sql`
- `backend/crates/golish-db/src/repo/{stage_episodes.rs,knowledge_assertions.rs,knowledge_documents.rs,knowledge_embeddings.rs,knowledge_outbox.rs}`

### 修改

- `backend/Cargo.toml`
- `backend/crates/golish-db/src/repo/mod.rs`
- `backend/crates/golish-agent-app/Cargo.toml`
- `backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,knowledge_memory.rs}`
- `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`：只注册 DB-global supervisor；不得按 AI session 启多个 projector worker。
- `backend/crates/golish/src/cli/bootstrap/{mod.rs,agent_init.rs}`：CLI 与桌面端使用同一 supervisor 生命周期。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{mod.rs,sub_agent_call.rs}`：关闭客户事实的 prose/stdout auto-promotion。
- `backend/crates/golish-agent-kit/src/db_tracking/memory/**`：标注 legacy/general memory，不作为 harness truth。
- 对应模块卡与 `docs/modules/INDEX.md`。

---

## Task 0：先建立新 crate 模块卡

**文件：**

- 新建 `docs/modules/backend/golish-memory-domain.md`
- 新建 `docs/modules/backend/golish-memory-app.md`
- 修改 `docs/modules/INDEX.md`

### 步骤 1：按现有模块卡模板写职责边界

模块卡必须先写清：domain 不依赖 sqlx/Graphiti/provider；app 只通过 ports 操作 canonical repo、outbox delivery 和时钟；任何 projector 都不是 Gate authority。此步骤发生在创建 crate 之前。

### 步骤 2：验证、门禁与提交

```bash
rg -n "OperationScope|AssertionVisibility|outbox delivery|Gate authority" \
  docs/modules/backend/golish-memory-domain.md \
  docs/modules/backend/golish-memory-app.md
just precommit
git add docs/modules/backend/golish-memory-domain.md docs/modules/backend/golish-memory-app.md docs/modules/INDEX.md
git commit -m "docs(memory): define memory fabric module boundaries"
```

预期：`rg` 四类边界均有命中；`just precommit` exit 0 后才允许 commit。

---

## Task 1：建立 scope、Episode、Assertion 纯领域模型

**文件：**

- 新建 `backend/crates/golish-memory-domain/Cargo.toml`
- 新建 `backend/crates/golish-memory-domain/src/lib.rs`
- 新建 `backend/crates/golish-memory-domain/src/scope.rs`
- 新建 `backend/crates/golish-memory-domain/src/classification.rs`
- 新建 `backend/crates/golish-memory-domain/src/assertion.rs`
- 新建 `backend/crates/golish-memory-domain/src/episode.rs`
- 新建 `backend/crates/golish-memory-domain/src/event_catalog.rs`
- 修改 `backend/Cargo.toml`

### 步骤 1：写 RED

```rust
#[test]
fn global_sanitized_rejects_customer_and_vault_material() {
    let draft = assertion_draft(
        AssertionVisibility::GlobalSanitized,
        AssertionKind::TechniqueExperience,
        KnowledgeClassification::CustomerConfidential,
        AssertionObject::VaultRef(VaultRef(Uuid::from_u128(7))),
        Some(TargetRef::new(Uuid::from_u128(9))),
    );

    let error = draft.validate().expect_err("global knowledge must be sanitized");
    assert_eq!(error, AssertionValidationError::GlobalContainsCustomerMaterial);
}
```

同一测试模块再用 table cases 覆盖：`CheckedEmpty` 缺 `fresh_until`、Hypothesis→Verified、blocked Episode→永久 negative assertion、空 evidence、明文 secret object；每个 case 都断言精确 error enum，不允许只断言 `is_err()`。

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-memory-domain --status-level fail
```

预期：crate 不存在。

### 步骤 2：定义类型

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ProjectScopeId(pub Uuid);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct VaultRef(pub Uuid);

pub struct OperationScope {
    pub project_scope_id: ProjectScopeId,
    pub source_operation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub scope_snapshot_hash: String,
}

pub enum AssertionVisibility {
    OrganizationLongTerm {
        project_scope_id: ProjectScopeId,
        organization_id_at_time: Uuid,
    },
    GlobalSanitized,
}

pub struct AssertionProvenance {
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_hash: String,
    pub source: SourceRef,
}

pub struct SourceRef {
    pub source_kind: CanonicalSourceKind,
    pub row_id: Uuid,
    pub source_stream_key: String,
    pub version: i64,
}

pub enum AssertionStatus { Active, Superseded, Refuted, Expired }
pub enum AssertionKind { Observation, CheckedEmpty, VerifiedOutcome, RefutedOutcome, TechniqueExperience, CleanupAttestation, ResidualRisk }
pub enum KnowledgeClassification { Public, Internal, CustomerConfidential, Restricted }
pub enum AssertionObject { Json(serde_json::Value), VaultRef(VaultRef) }
```

`ProjectScopeId` 由 P1 的持久化 `project_scopes` registry/frozen snapshot 提供；P1 resolver 负责 canonicalize workspace path 后查 UUID。path 仅用于 registry 显示与显式 rebind，任何授权、查询、去重都只使用 `ProjectScopeId`。`KnowledgeAssertionDraft::validate()` 强制 provenance/source stream/version/evidence/validity/classification；`GlobalSanitized` 只允许 Public/Internal `TechniqueExperience`，禁止 target/customer canonical ref、`VaultRef` 和可反查客户的正文。`VaultRef` 是 object kind，不是 classification。

### 步骤 3：Episode 与 event

```rust
pub struct StageEpisode {
    pub episode_id: Uuid,
    pub scope: OperationScope,
    pub stage_kind: StageKind,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub candidate_attempt_id: Option<Uuid>,
    pub wave: Option<i32>,
    pub verdict: EpisodeVerdict,
    pub reason_codes: Vec<String>,
    pub fact_refs: Vec<SourceRef>,
    pub evidence_refs: Vec<i64>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

pub struct KnowledgeEventEnvelopeV1 {
    pub event_id: Uuid,
    pub event_name: KnowledgeEventNameV1,
    pub schema_version: u16,
    pub aggregate_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub occurred_at: DateTime<Utc>,
    pub payload: KnowledgeEventPayloadV1,
}

pub enum KnowledgeEventNameV1 {
    StageEpisodeClosed,
    SourceFindingVerified,
    SourceCandidateRefuted,
    SourceTechniqueOutcomeTerminal,
    SourceFootholdVerified,
    SourceObjectiveOutcomeTerminal,
    SourceCleanupVerified,
    SourceResidualRecorded,
    SourceScopeInvalidated,
    AssertionUpserted,
    AssertionInvalidated,
    DocumentReady,
    DocumentInvalidated,
    ReportFinalized,
}

pub enum KnowledgeEventPayloadV1 {
    StageEpisodeClosed { episode_id: Uuid },
    FindingVerified { finding_id: Uuid, evidence_refs: Vec<i64> },
    CandidateRefuted { candidate_id: Uuid, attempt_id: Uuid, evidence_refs: Vec<i64> },
    TechniqueOutcomeTerminal { outcome_id: i64, evidence_refs: Vec<i64> },
    FootholdVerified { foothold_id: Uuid, evidence_refs: Vec<i64> },
    ObjectiveOutcomeTerminal { objective_attempt_id: Uuid, disposition: ObjectiveOutcomeDispositionV1, evidence_refs: Vec<i64> },
    CleanupVerified { obligation_id: Uuid, evidence_refs: Vec<i64> },
    ResidualRecorded { residual_id: Uuid, evidence_refs: Vec<i64> },
    ScopeInvalidated { project_scope_id: ProjectScopeId, organization_id_at_time: Uuid, reason_code: String },
    AssertionChanged { assertion_id: Uuid },
    DocumentChanged { document_id: Uuid },
    ReportFinalized { report_id: Uuid, revision_id: Uuid, artifact_ids: Vec<Uuid>, source_hash: String },
}

pub enum ObjectiveOutcomeDispositionV1 { Succeeded, Refuted, Blocked, Aborted }
```

`event_catalog.rs` 定义每个 event name 的唯一字符串、payload Rust type、允许的 source kind 和 mandatory projector routes。V1 固定 routing：canonical source events（包含 P6 后续 producer 的 `SourceFootholdVerified`/`SourceObjectiveOutcomeTerminal` typed refs）→`assertion-promoter@1`；`StageEpisodeClosed`→`document-projector@1`；Assertion events→`document-projector@1`、`graph-projector@1`、`embedding-projector@1`，其中 embedding delivery 依赖 document delivery；invalidation events 同样为 document/graph/embedding 各建 delivery。`ReportFinalized` payload 只允许 report/revision/artifact typed refs，且只路由到 `report-artifact-indexer@1`，明确禁止 Assertion/Document/Embedding/KG route。启停状态由 persisted projector registry 决定，producer 不能传 routes 或省略消费者。未知 event/version fail closed；不得猜 payload。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-memory-domain --status-level fail
```

```bash
just precommit
git add backend/Cargo.toml \
  backend/crates/golish-memory-domain/Cargo.toml \
  backend/crates/golish-memory-domain/src/lib.rs \
  backend/crates/golish-memory-domain/src/scope.rs \
  backend/crates/golish-memory-domain/src/classification.rs \
  backend/crates/golish-memory-domain/src/assertion.rs \
  backend/crates/golish-memory-domain/src/episode.rs \
  backend/crates/golish-memory-domain/src/event_catalog.rs
git commit -m "feat(memory): define scoped episode and assertion domain"
```

---

## Task 2：新增历史安全的 Memory Fabric schema/repo

**文件：**

- 新建 `backend/crates/golish-db/migrations/20260712000003_memory_fabric_core.sql`
- 新建 `backend/crates/golish-db/src/repo/stage_episodes.rs`
- 新建 `backend/crates/golish-db/src/repo/knowledge_assertions.rs`
- 新建 `backend/crates/golish-db/src/repo/knowledge_documents.rs`
- 新建 `backend/crates/golish-db/src/repo/knowledge_embeddings.rs`
- 新建 `backend/crates/golish-db/src/repo/knowledge_outbox.rs`
- 修改 `backend/crates/golish-db/src/repo/mod.rs`

### 步骤 1：写会因“一个 consumer 吞掉另一个 consumer”而 RED 的测试

```rust
#[sqlx::test(migrations = "migrations")]
async fn assertion_event_gets_all_catalog_deliveries_and_ack_is_isolated(pool: PgPool) -> anyhow::Result<()> {
    seed_projector_registry(&pool, &[
        enabled("document-projector", 1),
        enabled("graph-projector", 1),
        enabled("embedding-projector", 1),
    ]).await?;
    let event = source_assertion_event(Uuid::from_u128(11), "finding:11", 3);
    let mut tx = pool.begin().await?;
    append_event_with_catalog_deliveries(&mut tx, &event).await?;
    tx.commit().await?;

    let rows = list_deliveries(&pool, event.event_id).await?;
    assert_eq!(rows.iter().map(|r| r.projector_name.as_str()).collect::<BTreeSet<_>>(),
        BTreeSet::from(["document-projector", "embedding-projector", "graph-projector"]));
    assert_eq!(status(&rows, "embedding-projector"), DeliveryStatus::BlockedDependency);

    let graph = claim_delivery_batch(&pool, "graph-projector", 1, "g-1", 1).await?;
    assert_eq!(graph.len(), 1);
    ack_delivery(&pool, graph[0].event_id, "graph-projector", 1, "g-1").await?;

    let document = claim_delivery_batch(&pool, "document-projector", 1, "d-1", 1).await?;
    assert_eq!(document.len(), 1, "graph ack must not consume document delivery");
    Ok(())
}
```

同一 repo 测试矩阵必须具体覆盖：producer 无 route 参数；paused projector 有 pending delivery 但不可 claim；disabled projector 有 suppressed delivery/reason；document ack 原子唤醒 embedding；source transaction rollback 时 event/delivery 都不存在；event row不可更新/删除；delivery lease expiry 可重领；ack 前 crash 可幂等重放；同一 source stream 的旧 version 进入 stale terminal；不同 source stream 的 version 不互相比较；组织硬删除后 Episode/Assertion 历史仍存在；global sanitized 违反约束写入失败；document/embedding invalidation 只关闭时间窗不删除；`dimensions != 1536` 写入失败。

### 步骤 2：引用 P1 稳定 project scope 并创建历史表

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE stage_episodes (
    episode_id UUID PRIMARY KEY,
    project_scope_id UUID NOT NULL REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    source_operation_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    source_scope_snapshot_hash TEXT NOT NULL,
    stage_kind TEXT NOT NULL,
    stage_run_unit_id_at_time UUID,
    worker_run_id_at_time UUID,
    candidate_attempt_id_at_time UUID,
    wave INTEGER,
    verdict TEXT NOT NULL CHECK (verdict IN ('passed','blocked','exhausted','failed','superseded')),
    reason_codes JSONB NOT NULL DEFAULT '[]',
    fact_refs JSONB NOT NULL DEFAULT '[]',
    evidence_refs BIGINT[] NOT NULL DEFAULT '{}',
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ NOT NULL CHECK (ended_at >= started_at),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE knowledge_assertions (
    assertion_id UUID PRIMARY KEY,
    visibility TEXT NOT NULL CHECK (visibility IN ('organization_long_term','global_sanitized')),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    target_id_at_time UUID,
    source_operation_id UUID NOT NULL,
    source_scope_snapshot_hash TEXT NOT NULL,
    subject_ref JSONB NOT NULL,
    predicate TEXT NOT NULL,
    object_kind TEXT NOT NULL CHECK (object_kind IN ('json','vault_ref')),
    object_value JSONB,
    vault_ref UUID,
    assertion_kind TEXT NOT NULL CHECK (assertion_kind IN (
      'observation','checked_empty','verified_outcome','refuted_outcome',
      'technique_experience','cleanup_attestation','residual_risk'
    )),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','refuted','expired')),
    source_kind TEXT NOT NULL,
    source_row_id UUID NOT NULL,
    source_stream_key TEXT NOT NULL,
    source_version BIGINT NOT NULL CHECK (source_version >= 0),
    assertion_identity_hash TEXT NOT NULL,
    evidence_refs BIGINT[] NOT NULL CHECK (cardinality(evidence_refs) > 0),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    fresh_until TIMESTAMPTZ,
    classification TEXT NOT NULL CHECK (classification IN ('public','internal','customer_confidential','restricted')),
    superseded_by UUID REFERENCES knowledge_assertions(assertion_id) ON DELETE RESTRICT,
    content_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((object_kind = 'json' AND object_value IS NOT NULL AND vault_ref IS NULL)
        OR (object_kind = 'vault_ref' AND object_value IS NULL AND vault_ref IS NOT NULL)),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    CHECK (assertion_kind <> 'checked_empty' OR fresh_until IS NOT NULL),
    CHECK (
      (visibility = 'organization_long_term'
        AND project_scope_id IS NOT NULL AND organization_id_at_time IS NOT NULL)
      OR
      (visibility = 'global_sanitized'
        AND project_scope_id IS NULL AND organization_id_at_time IS NULL
        AND target_id_at_time IS NULL AND vault_ref IS NULL
        AND assertion_kind = 'technique_experience'
        AND classification IN ('public','internal'))
    ),
    UNIQUE(source_stream_key, source_version, predicate)
);
```

历史表故意不引用 live `organizations`/`targets`/`operation_state`；`*_at_time` 是不可变 provenance。组织删除只发布 invalidation，不抹除历史。`project_scopes` 由 P1 创建并由 frozen snapshot 持有；P3 只引用其稳定 UUID，不再按 path 解析或创建另一份 registry。

### 步骤 3：创建可失效 Document/Embedding

```sql
CREATE TABLE knowledge_documents (
    document_id UUID PRIMARY KEY,
    assertion_id UUID REFERENCES knowledge_assertions(assertion_id) ON DELETE RESTRICT,
    episode_id UUID REFERENCES stage_episodes(episode_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL,
    source_version BIGINT NOT NULL,
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version > 0),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','invalidated')),
    document_type TEXT NOT NULL,
    redacted_content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    redaction_version INTEGER NOT NULL CHECK (redaction_version > 0),
    classification TEXT NOT NULL CHECK (classification IN ('public','internal','customer_confidential','restricted')),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((assertion_id IS NOT NULL)::int + (episode_id IS NOT NULL)::int = 1),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    UNIQUE(source_stream_key, source_version, projection_schema_version, content_hash)
);

CREATE TABLE knowledge_embeddings (
    embedding_id UUID PRIMARY KEY,
    document_id UUID NOT NULL REFERENCES knowledge_documents(document_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL,
    source_version BIGINT NOT NULL,
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version > 0),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','invalidated')),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimensions INTEGER NOT NULL CHECK (dimensions = 1536),
    content_hash TEXT NOT NULL,
    embedding vector(1536) NOT NULL,
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    UNIQUE(document_id, provider, model, dimensions, projection_schema_version, content_hash)
);
```

V1 明确只支持 1536 维；引入其它维度必须新增 additive migration/物理表，不能让 `dimensions` 与 vector typmod 不一致。invalidator 更新 `status/valid_to`；不得删除 Document/Embedding 历史。

### 步骤 4：创建 immutable event 与 per-projector delivery

```sql
CREATE TABLE knowledge_outbox_events (
    event_id UUID PRIMARY KEY,
    event_name TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    aggregate_id UUID NOT NULL,
    source_stream_key TEXT NOT NULL,
    source_version BIGINT NOT NULL CHECK (source_version >= 0),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    source_operation_id UUID NOT NULL,
    payload JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    dedupe_key TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(event_name, schema_version, source_stream_key, source_version)
);

CREATE TABLE knowledge_projection_deliveries (
    event_id UUID NOT NULL REFERENCES knowledge_outbox_events(event_id) ON DELETE RESTRICT,
    projector_name TEXT NOT NULL,
    projector_schema_version INTEGER NOT NULL CHECK (projector_schema_version > 0),
    status TEXT NOT NULL DEFAULT 'pending'
      CHECK (status IN ('blocked_dependency','pending','leased','processed','retryable_failed','stale','dead_letter','suppressed')),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    depends_on_projector TEXT,
    depends_on_schema_version INTEGER,
    suppressed_reason TEXT,
    last_error TEXT,
    processed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY(event_id, projector_name, projector_schema_version)
);

CREATE TABLE knowledge_projector_registry (
    projector_name TEXT NOT NULL,
    projector_schema_version INTEGER NOT NULL CHECK (projector_schema_version > 0),
    lifecycle TEXT NOT NULL CHECK (lifecycle IN ('enabled','paused','disabled')),
    disabled_reason TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY(projector_name, projector_schema_version),
    CHECK (lifecycle <> 'disabled' OR disabled_reason IS NOT NULL)
);

INSERT INTO knowledge_projector_registry(projector_name, projector_schema_version, lifecycle)
VALUES
  ('assertion-promoter', 1, 'enabled'),
  ('document-projector', 1, 'paused'),
  ('graph-projector', 1, 'paused'),
  ('embedding-projector', 1, 'paused'),
  ('report-artifact-indexer', 1, 'paused');

CREATE FUNCTION reject_knowledge_outbox_event_mutation() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'knowledge_outbox_events are immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER knowledge_outbox_events_immutable
BEFORE UPDATE OR DELETE ON knowledge_outbox_events
FOR EACH ROW EXECUTE FUNCTION reject_knowledge_outbox_event_mutation();

CREATE INDEX knowledge_deliveries_claim
ON knowledge_projection_deliveries(projector_name, projector_schema_version, status, available_at);
CREATE INDEX knowledge_assertions_org_active
ON knowledge_assertions(project_scope_id, organization_id_at_time, status, fresh_until)
WHERE visibility = 'organization_long_term';
CREATE INDEX stage_episodes_operation_stage
ON stage_episodes(source_operation_id, organization_id_at_time, stage_kind, ended_at DESC);
```

Event row 是 append-only。数据库权限/trigger 测试拒绝 UPDATE/DELETE；所有 lease、retry、ack 和 DLQ 只修改 delivery。`append_event_with_catalog_deliveries` 必须为 catalog 的每个 mandatory route 建行：enabled→pending；paused→pending 但 claim SQL 不可领取；disabled→suppressed 并记录原因；embedding→blocked_dependency，直到同 event 的 document delivery processed 后原子唤醒。最大 8 次后只把该 delivery 标为 `dead_letter`，不能阻断同 event 的其它 projector。

### 步骤 5：实现 transaction/repo API

```rust
pub async fn close_episode_with_event(tx: &mut Transaction<'_, Postgres>, episode: &StageEpisode) -> Result<Uuid>;
pub async fn upsert_assertion_with_event(tx: &mut Transaction<'_, Postgres>, assertion: &KnowledgeAssertion) -> Result<Uuid>;
pub async fn invalidate_assertion_with_event(tx: &mut Transaction<'_, Postgres>, command: InvalidateAssertion) -> Result<()>;
pub async fn append_event_with_catalog_deliveries(tx: &mut Transaction<'_, Postgres>, event: &KnowledgeEventEnvelopeV1) -> Result<()>;
pub async fn claim_delivery_batch(pool: &PgPool, projector: &str, schema: i32, worker: &str, limit: i64) -> Result<Vec<ClaimedDelivery>>;
pub async fn ack_delivery(pool: &PgPool, event_id: Uuid, projector: &str, schema: i32, worker: &str) -> Result<()>;
pub async fn retry_or_dead_letter_delivery(pool: &PgPool, failure: DeliveryFailure) -> Result<()>;
```

`append_event_with_catalog_deliveries` 从 `event_catalog.rs` 与 `knowledge_projector_registry` 服务器端推导 routes，并在 source transaction 内插入 event 和全部 mandatory delivery；producer API 不接受 routes。claim 使用 `FOR UPDATE SKIP LOCKED`，并 join registry 只领取 enabled projector。projector 网络/embedding 工作发生在 claim transaction 结束后。source version 只与同 `source_stream_key` 比较。

### 步骤 6：GREEN、全门禁与精确提交

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-db \
  -E 'test(stage_episode) | test(knowledge_assertion) | test(knowledge_document) | test(knowledge_embedding) | test(knowledge_outbox)' \
  --status-level fail
just precommit
git add backend/crates/golish-db/migrations/20260712000003_memory_fabric_core.sql \
  backend/crates/golish-db/src/repo/mod.rs \
  backend/crates/golish-db/src/repo/stage_episodes.rs \
  backend/crates/golish-db/src/repo/knowledge_assertions.rs \
  backend/crates/golish-db/src/repo/knowledge_documents.rs \
  backend/crates/golish-db/src/repo/knowledge_embeddings.rs \
  backend/crates/golish-db/src/repo/knowledge_outbox.rs
git commit -m "feat(db): add historical memory fabric and projector deliveries"
```

---

## Task 3：实现 promotion/invalidation 应用服务

**文件：**

- 新建 `backend/crates/golish-memory-app/Cargo.toml`
- 新建 `backend/crates/golish-memory-app/src/lib.rs`
- 新建 `backend/crates/golish-memory-app/src/ports.rs`
- 新建 `backend/crates/golish-memory-app/src/promotion.rs`
- 新建 `backend/crates/golish-memory-app/src/invalidation.rs`
- 新建 `backend/crates/golish-memory-app/src/outbox.rs`
- 修改 `backend/Cargo.toml`

### 步骤 1：写 RED promotion matrix

```rust
#[tokio::test]
async fn proposed_candidate_is_rejected_before_repository_write() {
    let uow = RecordingKnowledgeUnitOfWork::default();
    let service = PromotionService::new(&uow, fixed_clock("2026-07-12T12:00:00Z"));
    let event = source_candidate_event(CandidateDisposition::Proposed, vec![evidence_ref(41)]);

    let decision = service.promote(event).await.expect("policy decision");

    assert_eq!(decision, PromotionDecision::Reject(UntrustedHypothesis));
    assert!(uow.commands().is_empty(), "rejected hypothesis must not reach DB");
}
```

用同一 `RecordingKnowledgeUnitOfWork` table cases 覆盖：verified Candidate→VerifiedOutcome；refuted Candidate→RefutedOutcome；checked-empty 必须带 fresh-until；Gate BLOCK 只生成 Episode；cleanup/scope/credential revoke 关闭 assertion/document/embedding；RawToolOutput/AgentProse/未知 event version 拒绝；global sanitized 仅接受已脱敏 TechniqueExperience。

### 步骤 2：定义 ports

```rust
#[async_trait]
pub trait KnowledgeUnitOfWork {
    async fn close_episode_and_emit(&self, command: CloseEpisode) -> Result<Uuid>;
    async fn promote_assertion_and_emit(&self, command: PromoteAssertion) -> Result<Uuid>;
    async fn invalidate_projection_chain_and_emit(&self, command: InvalidateProjectionChain) -> Result<()>;
}
```

Port 方法必须对应单个 DB transaction；禁止暴露“先写 Assertion、后写 event”的两个独立调用。`InvalidateProjectionChain` 在同一事务关闭 Assertion/Document/Embedding 时间窗并 append typed invalidation event/deliveries。

### 步骤 3：实现 deterministic promotion policy

policy 只接收 `event_catalog.rs` 已注册且 schema version 精确匹配的 typed canonical events：FindingVerified、CandidateRefuted、TechniqueOutcomeTerminal、StageGatePassed、CleanupVerified、ResidualRecorded、ScopeInvalidated。任何 RawToolOutput/AgentProse/未知 payload 返回稳定 error code。source version 只在同 `source_stream_key` 内比较；old version 产生 `Stale` delivery outcome，不能覆盖新状态。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-memory-app --status-level fail
```

```bash
just precommit
git add backend/Cargo.toml \
  backend/crates/golish-memory-app/Cargo.toml \
  backend/crates/golish-memory-app/src/lib.rs \
  backend/crates/golish-memory-app/src/ports.rs \
  backend/crates/golish-memory-app/src/promotion.rs \
  backend/crates/golish-memory-app/src/invalidation.rs \
  backend/crates/golish-memory-app/src/outbox.rs
git commit -m "feat(memory): promote only typed evidence backed knowledge"
```

---

## Task 4：接入 Stage Episode/Handoff source events

**文件：**

- 新建 `backend/crates/golish-agent-app/src/ai/db_bridge/knowledge_memory.rs`
- 新建 `backend/crates/golish-memory-app/src/supervisor.rs`
- 修改 `backend/crates/golish-memory-app/src/lib.rs`
- 修改 `backend/crates/golish-agent-app/Cargo.toml`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/candidate_attempt_submit_tool.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改 `backend/crates/golish-agent-kit/src/db_tracking/memory/mod.rs`
- 修改 `backend/crates/golish-agent-kit/src/db_tracking/memory/store.rs`
- 修改 `backend/crates/golish/src/cli/bootstrap/mod.rs`
- 修改 `backend/crates/golish/src/cli/bootstrap/agent_init.rs`

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn gate_pass_failure_rolls_back_unit_handoff_episode_event_and_deliveries() {
    let fixture = MemoryBridgeFixture::new().await;
    fixture.fail_at(FailPoint::AfterEpisodeInsert);
    let command = gate_pass_command(unit_id(7), org_id(8), vec![evidence_ref(91)]);

    let error = fixture.bridge.terminalize_gate_pass(command).await.expect_err("injected failure");

    assert_eq!(error.code(), "MEMORY_EVENT_ATOMIC_WRITE_FAILED");
    assert_eq!(fixture.count_stage_handoffs().await, 0);
    assert_eq!(fixture.count_stage_episodes().await, 0);
    assert_eq!(fixture.count_outbox_events().await, 0);
    assert_eq!(fixture.count_projection_deliveries().await, 0);
}
```

同一 fixture 覆盖：Gate PASS 原子写 unit/handoff/episode/event/deliveries；Gate BLOCK 写 Episode 但不发布 PASS handoff；CandidateAttempt terminal transaction 保留 attempt/evidence/source stream；重复 terminalizer 只产生一个 event；tool stdout/sub-agent prose 含 IP/CVE/URL 时既不写 Assertion，也不写 legacy customer memory/KG。

### 步骤 2：接 canonical transaction

- StageRunUnit PASS terminal transaction 写 unit + Handoff + Episode + immutable event + catalog routes 对应 deliveries。
- StageRunUnit BLOCK terminal transaction写 unit + Episode + deliveries，不发布 PASS Handoff。
- CandidateAttempt terminal transaction 写 Attempt/Finding lineage + Episode + typed source event + deliveries。
- StageHandoff 保持 P1 authoritative pass artifact，并作为 Episode fact ref，不复制 payload。

所有调用走 `knowledge_memory.rs` 的 `KnowledgeUnitOfWork` adapter；不得在 terminalizer 中先 commit canonical row 再 fire-and-forget 写 memory。

### 步骤 3：关闭客户事实 auto-memory

在 runtime direct/sub-agent 路径移除或 feature-gate 从 stdout/prose 自动写客户长期 memory/KG；通用用户显式 memory tool 保留，但不能被 harness Gate/RAG 默认读取。

### 步骤 4：注册 DB-global supervisor

`golish-memory-app/src/supervisor.rs` 定义 `KnowledgeProjectorSupervisor`：按 `(projector_name,schema_version)` 注册唯一 worker、持有 cancellation token、续租/退避并在 shutdown 等待当前 batch 完成。桌面端在 `bridge_config.rs` 的 DB ready 生命周期启动一次；不得跟随 session 初始化重复启动。CLI 在 `cli/bootstrap/agent_init.rs` 启动同一 supervisor，并在 `cli/bootstrap/mod.rs::shutdown` 停止。P3 只注册已实现的 `assertion-promoter@1`；P4/P5 通过 registry 增加 graph/document/embedding，不复制 loop。

### 步骤 5：GREEN、全门禁与精确提交

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-agent-app memory_fabric --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-agent-runtime auto_memory --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-memory-app supervisor --status-level fail
```

```bash
just precommit
git add backend/crates/golish-memory-app/src/lib.rs \
  backend/crates/golish-memory-app/src/supervisor.rs \
  backend/crates/golish-agent-app/Cargo.toml \
  backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs \
  backend/crates/golish-agent-app/src/ai/db_bridge/knowledge_memory.rs \
  backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs \
  backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs \
  backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs \
  backend/crates/golish-agent-app/src/ai/candidate_attempt_submit_tool.rs \
  backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs \
  backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs \
  backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs \
  backend/crates/golish-agent-kit/src/db_tracking/memory/mod.rs \
  backend/crates/golish-agent-kit/src/db_tracking/memory/store.rs \
  backend/crates/golish/src/cli/bootstrap/mod.rs \
  backend/crates/golish/src/cli/bootstrap/agent_init.rs
git commit -m "feat(memory): emit episodes from canonical terminal transactions"
```

---

## Task 5：包级验证与文档

### 步骤 1：故障注入矩阵

逐项运行并把 exit code/关键断言写入 `agent-progress.md`：source transaction rollback；两个 projector 独立 claim/ack；delivery lease expiry；duplicate event；同 stream old source version；不同 stream 版本互不阻塞；poison delivery 独立 DLQ；组织删除后的历史保留与 projection invalidation；global sanitized customer/vault rejection；document/embedding temporal close；DB-global supervisor shutdown/restart 不重复处理。

### 步骤 2：门禁

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-memory-domain --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-memory-app --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-db -E 'test(knowledge_) | test(stage_episode)' --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-agent-app memory_fabric --status-level fail
cd backend && cargo clippy -p golish-memory-domain -p golish-memory-app -p golish-db -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings
cd backend && cargo fmt --all -- --check
just precommit
```

### 步骤 3：模块卡与提交

更新 Task 0 已先创建的两个新 crate 模块卡，以及 DB repo、agent-app AI、runtime loop、legacy memory、CLI 卡和 INDEX。随后逐项执行 `clean-state-checklist.md`；确认只包含本包文件后再提交。

```bash
just precommit
git add docs/modules/backend/golish-memory-domain.md \
  docs/modules/backend/golish-memory-app.md \
  docs/modules/backend/golish-db/repo.md \
  docs/modules/backend/golish-agent-app/ai.md \
  docs/modules/backend/golish-agent-runtime/agentic_loop.md \
  docs/modules/backend/golish-agent-kit/db_tracking.md \
  docs/modules/backend/golish/cli.md \
  docs/modules/INDEX.md \
  agent-progress.md \
  feature_list.json
git commit -m "docs(memory): document memory fabric authority and outbox"
```

---

## 本计划完成后仍不做的事

- 不调用 Graphiti；由 P4 projector 实现。
- 不把 embedding/RAG 注入 prompt；由 P5 实现。
- 不让通用 `memories`/global fallback 成为 harness 默认知识。
- 不在 DB transaction 内调用 embedding、LLM 或 HTTP。
