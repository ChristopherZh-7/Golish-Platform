# Structured Temporal Knowledge Graph Projector 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让知识图谱只从 typed knowledge outbox 投影 scope/evidence/validity 完整的节点和边，并停止从 stdout、扫描文本或模型 prose 自动创造可信关系。

**架构：** `KnowledgeAssertion` 是 source；DB-global Graph Projector 只 claim `(graph-projector, schema=1)` delivery。图的 entity/relation identity 与 assertion lineage 分表：同一 canonical entity 可由多条 Assertion 独立支撑，失效一条 lineage 不会关闭其它有效来源；source version 只在同 `source_stream_key` 内判 stale。P4 默认写新的 local V2 graph tables；现有 `graph_entities/graph_relations` 保留为 legacy/non-authoritative。外部 Graphiti 是另一个需授权的 projector delivery，不能与 local graph 共用 ack。

**技术栈：** Rust 2021、golish-memory-app、golish-graphiti、PostgreSQL JSONB、async worker、cargo-nextest。

**依赖：** P3 Memory Fabric Core 完成；实施 graph schema migration/外部 Graphiti acceptance 前取得用户确认。

---

## 1. 文件结构

### 新建

- `backend/crates/golish-db/migrations/20260712000004_structured_temporal_graph.sql`
- `backend/crates/golish-memory-app/src/projectors/{mod.rs,graph.rs}`
- `backend/crates/golish-memory-app/src/graph_projection.rs`

### 修改

- `backend/crates/golish-memory-app/src/lib.rs`
- `backend/crates/golish-memory-app/src/supervisor.rs`
- `backend/crates/golish-graphiti/src/{types.rs,client.rs}`
- `backend/crates/golish-agent-kit/src/tool_executors/{graph_trait.rs,graph.rs}`
- `backend/crates/golish-agent-app/src/ai/{graph_bridge.rs,commands/graph.rs}`
- `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`
- `backend/crates/golish/src/cli/bootstrap/agent_init.rs`
- `backend/crates/golish-agent-kit/src/harness/rag_prior.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{mod.rs,sub_agent_call.rs}`
- `docs/modules/backend/{golish-memory-app.md,golish-graphiti.md}`
- `docs/modules/backend/golish-agent-app/ai.md`
- `docs/modules/backend/golish-agent-kit/tool_executors.md`
- `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- `docs/modules/INDEX.md`

---

## Task 0：在改代码前更新模块卡

**文件：**

- 修改 `docs/modules/backend/golish-memory-app.md`
- 修改 `docs/modules/backend/golish-graphiti.md`
- 修改 `docs/modules/backend/golish-agent-app/ai.md`
- 修改 `docs/modules/INDEX.md`

### 步骤 1：先写清 identity/lineage 和 worker ownership

模块卡必须明确：local V2 graph 是可重建 projection；legacy graph 不进入 scoped RAG；entity/relation identity 不承载单一 assertion；Graph worker 由 P3 的 DB-global supervisor 持有，不随 session 启停；外部 Graphiti 使用独立 delivery 和数据策略。

### 步骤 2：验证、全门禁和精确提交

```bash
rg -n "identity|lineage|DB-global|legacy|Graphiti" \
  docs/modules/backend/golish-memory-app.md \
  docs/modules/backend/golish-graphiti.md \
  docs/modules/backend/golish-agent-app/ai.md
just precommit
git add docs/modules/backend/golish-memory-app.md \
  docs/modules/backend/golish-graphiti.md \
  docs/modules/backend/golish-agent-app/ai.md \
  docs/modules/INDEX.md
git commit -m "docs(graph): define temporal graph projection boundaries"
```

---

## Task 1：定义 deterministic graph projection contract

**文件：**

- 新建 `backend/crates/golish-memory-app/src/graph_projection.rs`
- 新建 `backend/crates/golish-memory-app/src/projectors/mod.rs`
- 修改 `backend/crates/golish-memory-app/src/lib.rs`

### 步骤 1：写 RED

```rust
#[test]
fn two_assertions_share_identity_but_keep_independent_lineage() {
    let a1 = host_assertion(assertion_id(1), "target:7/host:10.0.0.5", "target:7", 1);
    let a2 = host_assertion(assertion_id(2), "target:7/host:10.0.0.5", "dns:9", 4);

    let p1 = project_assertion(&a1).expect("first projection");
    let p2 = project_assertion(&a2).expect("second projection");

    assert_eq!(p1.entities[0].identity, p2.entities[0].identity);
    assert_ne!(p1.entity_lineage[0].assertion_id, p2.entity_lineage[0].assertion_id);
    let close = project_invalidation(&a1, fixed_time("2026-07-12T12:00:00Z"));
    assert_eq!(close.close_assertion_id, a1.assertion_id);
    assert_ne!(close.close_assertion_id, a2.assertion_id);
}
```

同一模块 table cases 覆盖：organization/global scope identity；raw prose/stdout/未知 event version 返回 `UntrustedEventClass`；未知 predicate 返回 `UnsupportedPredicate`；属性白名单去除 credential/token/raw response；旧 source version 仅在同 stream 判 stale；不同 stream 的数字版本互不比较；invalidation 只关闭目标 assertion lineage。

### 步骤 2：定义 projection DTO

```rust
pub struct GraphEntityIdentityProjection {
    pub scope_key: GraphScopeKey,
    pub visibility: AssertionVisibility,
    pub project_scope_id: Option<ProjectScopeId>,
    pub organization_id_at_time: Option<Uuid>,
    pub canonical_ref: String,
    pub entity_type: GraphEntityTypeV1,
    pub display_name: String,
    pub properties: Value,
}

pub struct GraphAssertionLineageProjection {
    pub canonical_ref: String,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: String,
    pub projection_schema_version: i32,
}

pub struct GraphRelationIdentityProjection {
    pub scope_key: GraphScopeKey,
    pub visibility: AssertionVisibility,
    pub project_scope_id: Option<ProjectScopeId>,
    pub organization_id_at_time: Option<Uuid>,
    pub from_canonical_ref: String,
    pub to_canonical_ref: String,
    pub relation_type: GraphRelationTypeV1,
}

pub struct GraphRelationLineageProjection {
    pub relation: GraphRelationIdentityProjection,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}
```

实现：

```rust
pub fn project_assertion(assertion: &KnowledgeAssertion) -> Result<GraphProjection, ProjectionError>;
pub fn project_invalidation(assertion: &KnowledgeAssertion, at: DateTime<Utc>) -> GraphInvalidation;
```

V1 entity 闭集固定为 `organization,target,host,service,endpoint,vulnerability,finding,technique`；relation 闭集固定为 `contains,resolves_to,runs_service,exposes_endpoint,has_vulnerability,supported_by_finding,associated_technique`。`GlobalSanitized` 只能投影 `technique`。每种 entity 的 properties 使用显式 allowlist；禁止 vault ref、credential value、token、session、raw stdout、HTTP body、exploit payload。未知 predicate/event/version 写当前 graph delivery failure，不猜关系。

### 步骤 3：GREEN

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-memory-app graph_projection --status-level fail
```

### 步骤 4：提交

```bash
just precommit
git add backend/crates/golish-memory-app/src/lib.rs \
  backend/crates/golish-memory-app/src/graph_projection.rs \
  backend/crates/golish-memory-app/src/projectors/mod.rs
git commit -m "feat(graph): define typed assertion graph projection"
```

---

## Task 2：新增 identity/lineage 分离的 local V2 graph schema

**文件：**

- 新建 `backend/crates/golish-db/migrations/20260712000004_structured_temporal_graph.sql`
- 修改 `backend/crates/golish-graphiti/src/types.rs`
- 修改 `backend/crates/golish-graphiti/src/client.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/graph_bridge.rs`

### 步骤 1：写“失效一个 assertion 不关闭另一个来源”的 RED

```rust
#[sqlx::test(migrations = "../golish-db/migrations")]
async fn closing_one_lineage_keeps_shared_entity_active(pool: PgPool) -> anyhow::Result<()> {
    let graph = TemporalGraphAdapter::new(pool);
    let entity = graph.upsert_entity_identity(host_identity("target:7/host:10.0.0.5")).await?;
    graph.attach_entity_assertion(entity, lineage(assertion_id(1), "target:7", 1)).await?;
    graph.attach_entity_assertion(entity, lineage(assertion_id(2), "dns:9", 4)).await?;

    graph.close_assertion_lineage(assertion_id(1), fixed_time("2026-07-12T12:00:00Z")).await?;

    let active = graph.get_active_entity(entity).await?;
    assert!(active.is_some(), "second active assertion still supports the identity");
    assert_eq!(graph.active_lineage_count(entity).await?, 1);
    Ok(())
}
```

同一 integration module 覆盖：同 canonical ref 在 sibling org 生成不同 entity；global technique 与客户 entity 不碰撞；跨 scope relation 被 composite FK 拒绝；relation 由两条 assertion 支撑时关闭一条仍 active；同 stream version 5 后到 version 4 标 stale；不同 stream version 1 可与 version 99 并存；历史组织删除后 V2 graph lineage 仍可按 `organization_id_at_time` 审计；V2 查询永不读取 legacy rows。

### 步骤 2：只新增 V2 tables，不把不可信 legacy graph 回填为事实

```sql
CREATE TABLE knowledge_graph_entities (
    entity_id UUID PRIMARY KEY,
    scope_key TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('organization_long_term','global_sanitized')),
    project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    canonical_ref TEXT NOT NULL,
    entity_type TEXT NOT NULL CHECK (entity_type IN (
      'organization','target','host','service','endpoint','vulnerability','finding','technique'
    )),
    display_name TEXT NOT NULL,
    properties JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
      (visibility = 'organization_long_term'
        AND project_scope_id IS NOT NULL AND organization_id_at_time IS NOT NULL
        AND scope_key LIKE 'org:%')
      OR
      (visibility = 'global_sanitized'
        AND project_scope_id IS NULL AND organization_id_at_time IS NULL
        AND entity_type = 'technique' AND scope_key = 'global_sanitized')
    ),
    UNIQUE(entity_id, scope_key),
    UNIQUE(scope_key, canonical_ref)
);

CREATE TABLE knowledge_graph_entity_assertions (
    entity_id UUID NOT NULL REFERENCES knowledge_graph_entities(entity_id) ON DELETE RESTRICT,
    assertion_id UUID NOT NULL REFERENCES knowledge_assertions(assertion_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL,
    source_version BIGINT NOT NULL CHECK (source_version >= 0),
    evidence_refs BIGINT[] NOT NULL CHECK (cardinality(evidence_refs) > 0),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','refuted','expired','invalidated')),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    fresh_until TIMESTAMPTZ,
    classification TEXT NOT NULL CHECK (classification IN ('public','internal','customer_confidential','restricted')),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    PRIMARY KEY(entity_id, assertion_id, projection_schema_version),
    UNIQUE(entity_id, source_stream_key, source_version, projection_schema_version)
);

CREATE TABLE knowledge_graph_relations (
    relation_id UUID PRIMARY KEY,
    scope_key TEXT NOT NULL,
    from_entity_id UUID NOT NULL,
    to_entity_id UUID NOT NULL,
    relation_type TEXT NOT NULL CHECK (relation_type IN (
      'contains','resolves_to','runs_service','exposes_endpoint',
      'has_vulnerability','supported_by_finding','associated_technique'
    )),
    properties JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (from_entity_id <> to_entity_id),
    FOREIGN KEY(from_entity_id, scope_key)
      REFERENCES knowledge_graph_entities(entity_id, scope_key) ON DELETE RESTRICT,
    FOREIGN KEY(to_entity_id, scope_key)
      REFERENCES knowledge_graph_entities(entity_id, scope_key) ON DELETE RESTRICT,
    UNIQUE(scope_key, from_entity_id, to_entity_id, relation_type)
);

CREATE TABLE knowledge_graph_relation_assertions (
    relation_id UUID NOT NULL REFERENCES knowledge_graph_relations(relation_id) ON DELETE RESTRICT,
    assertion_id UUID NOT NULL REFERENCES knowledge_assertions(assertion_id) ON DELETE RESTRICT,
    source_stream_key TEXT NOT NULL,
    source_version BIGINT NOT NULL CHECK (source_version >= 0),
    evidence_refs BIGINT[] NOT NULL CHECK (cardinality(evidence_refs) > 0),
    status TEXT NOT NULL CHECK (status IN ('active','superseded','refuted','expired','invalidated')),
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    classification TEXT NOT NULL CHECK (classification IN ('public','internal','customer_confidential','restricted')),
    projection_schema_version INTEGER NOT NULL CHECK (projection_schema_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (valid_to IS NULL OR valid_to >= valid_from),
    PRIMARY KEY(relation_id, assertion_id, projection_schema_version),
    UNIQUE(relation_id, source_stream_key, source_version, projection_schema_version)
);
```

现有 `graph_entities/graph_relations` 不改 schema、不回填、不参与 scoped RAG；它们仅供 legacy/general debug。P6 若增加 foothold/internal-asset/path 类型，必须在自己的 additive migration 与 mapping tests 中显式扩展，P4 不预留未定义字符串。

`GraphScopeKey` 是封闭类型：organization visibility 固定编码为 `org:<project_scope_id>:<organization_id_at_time>`；global 固定为 `global_sanitized`。构造函数校验 key 与结构化字段一致。relation 同时携带 key，并通过两个 composite FK 保证 from/to 不可能跨 scope。

### 步骤 3：实现 source-stream-aware adapter

```rust
async fn upsert_entity_identity(&self, entity: GraphEntityIdentityProjection) -> Result<GraphEntityId>;
async fn attach_entity_assertion(&self, entity: GraphEntityId, lineage: GraphAssertionLineageProjection) -> Result<LineageWrite>;
async fn upsert_relation_identity(&self, relation: GraphRelationIdentityProjection) -> Result<GraphRelationId>;
async fn attach_relation_assertion(&self, relation: GraphRelationId, lineage: GraphRelationLineageProjection) -> Result<LineageWrite>;
async fn close_assertion_lineage(&self, assertion_id: Uuid, valid_to: DateTime<Utc>) -> Result<()>;
async fn max_source_version(&self, source_stream_key: &str, schema: i32) -> Result<Option<i64>>;
```

Identity upsert 从不比较 source version。projector 先按 `source_stream_key + schema` 查询 max version；更旧 delivery 标 `stale`，相同版本幂等重放，不同 stream 独立写 lineage。active entity/relation 查询必须使用 `EXISTS` active lineage，并在 SQL WHERE 中先筛 project scope、org-at-time、classification、validity。

### 步骤 4：GREEN、全门禁与精确提交

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-agent-app graph_bridge --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-graphiti --status-level fail
just precommit
git add backend/crates/golish-db/migrations/20260712000004_structured_temporal_graph.sql \
  backend/crates/golish-graphiti/src/types.rs \
  backend/crates/golish-graphiti/src/client.rs \
  backend/crates/golish-agent-app/src/ai/graph_bridge.rs
git commit -m "feat(graph): split graph identity from assertion lineage"
```

---

## Task 3：实现 outbox Graph Projector worker

**文件：**

- 新建 `backend/crates/golish-memory-app/src/projectors/graph.rs`
- 修改 `backend/crates/golish-memory-app/src/projectors/mod.rs`
- 修改 `backend/crates/golish-memory-app/src/lib.rs`
- 修改 `backend/crates/golish-memory-app/src/supervisor.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`
- 修改 `backend/crates/golish/src/cli/bootstrap/agent_init.rs`

### 步骤 1：写 RED failure tests

```rust
#[tokio::test]
async fn local_write_before_ack_crash_replays_without_duplicate_lineage() -> anyhow::Result<()> {
    let deliveries = FakeDeliveryPort::with_one("graph-projector", 1, assertion_event(17));
    let graph = RecordingTemporalGraph::default();
    deliveries.fail_next_ack();

    let first = run_graph_projector_once(&deliveries, &graph, "graph-1", 8).await;
    assert!(matches!(first, Err(ProjectorError::AckFailed { .. })));
    assert_eq!(graph.lineage_count(assertion_id(17)), 1);

    let second = run_graph_projector_once(&deliveries, &graph, "graph-2", 8).await?;
    assert_eq!(second.processed, 1);
    assert_eq!(graph.lineage_count(assertion_id(17)), 1, "replay is idempotent");
    Ok(())
}
```

同一 fake suite 覆盖：projector 只 claim 自己的 delivery；document delivery 保持 pending；同 stream 旧 version 标 stale；unknown predicate/version 重试 8 次后仅 graph delivery dead-letter；local DB call 发生在 claim transaction 结束后；shutdown 释放/等待 lease；Graphiti failure 不影响 local graph delivery。

### 步骤 2：实现 worker loop

```rust
pub async fn run_graph_projector_once(
    deliveries: &dyn ProjectionDeliveryPort,
    graph: &dyn TemporalGraphPort,
    worker_id: &str,
    batch_size: usize,
) -> Result<ProjectorBatchResult>;
```

执行顺序：claim `(graph-projector,1)` delivery 的短事务结束 → typed decode/catalog validation → 查询 Assertion → deterministic mapping → local graph 短事务写 identity+lineage → 单独 ack delivery。任何 projector 都不更新 immutable event。外部 Graphiti 若启用，catalog 额外创建 `(graphiti-projector,N)` delivery，由独立 worker、数据策略和 DLQ 处理；local success 不等待 Graphiti。

在 P3 的 `KnowledgeProjectorSupervisor` registry 注册 local worker。只有 migration、adapter 和 replay tests 全绿后，安装事务才把 `knowledge_projector_registry(graph-projector,1)` 从 `paused` 切为 `enabled`，已有 pending deliveries 随即由 supervisor 领取。桌面 `bridge_config.rs` 与 CLI `agent_init.rs` 只提供 `TemporalGraphPort` adapter；不得各自复制 loop，也不得按 AI session 启 worker。

### 步骤 3：实现 rebuild

```rust
pub async fn rebuild_scope(
    assertions: &dyn AssertionReader,
    graph: &dyn TemporalGraphPort,
    scope: RebuildScope,
    schema_version: i32,
) -> Result<RebuildReport>;
```

`RebuildScope` 只能是 exact `(project_scope_id, organization_id_at_time)` 或显式 `GlobalSanitized`。rebuild 从 Assertion 历史重建 identity/lineage，不从 legacy/V2 graph 倒推；schema 升级先写新 `projection_schema_version`，验证数量/lineage/hash 后切 read version，旧 lineage 保留到独立 contract cleanup。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-memory-app graph_projector --status-level fail
```

```bash
just precommit
git add backend/crates/golish-memory-app/src/lib.rs \
  backend/crates/golish-memory-app/src/projectors/mod.rs \
  backend/crates/golish-memory-app/src/projectors/graph.rs \
  backend/crates/golish-memory-app/src/supervisor.rs \
  backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs \
  backend/crates/golish/src/cli/bootstrap/agent_init.rs
git commit -m "feat(graph): project knowledge outbox into temporal graph"
```

---

## Task 4：关闭 prose/stdout 自动提升并收紧 graph API

**文件：**

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/rag_prior.rs`
- 修改 `backend/crates/golish-agent-kit/src/tool_executors/graph_trait.rs`
- 修改 `backend/crates/golish-agent-kit/src/tool_executors/graph.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/graph.rs`

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn stdout_that_looks_like_security_facts_creates_no_v2_graph_delivery() -> anyhow::Result<()> {
    let fixture = RuntimeGraphFixture::new().await;
    fixture.complete_tool("nmap", "10.0.0.5 CVE-2026-0001 https://victim.test/admin").await?;

    assert_eq!(fixture.v2_entity_count().await?, 0);
    assert_eq!(fixture.delivery_count("graph-projector", 1).await?, 0);
    assert!(fixture.evidence_count().await? > 0, "raw output remains evidence, not knowledge");
    Ok(())
}
```

同一 integration module 覆盖：sub-agent prose 不生成 graph event；`feed_findings_to_graph` 不再接受 prose；无 operation/org 的 Tauri query 返回稳定 scope error code；sibling org query 返回 ownership error；GlobalSanitized 必须显式 knowledge class；legacy row 永不出现在 V2 scoped query。

### 步骤 2：移除/禁用 auto feed

删除客户 harness 路径对 regex entity extraction、`feed_findings_to_graph` prose adapter 的调用。现有 legacy GraphClient 可供非客户 debug 内部代码使用，但不得注册无 scope 的 Tauri/agent customer search，也不得进入 V2 scoped RAG。canonical Finding 只能通过 versioned source event→Assertion→graph delivery 进入 V2。

### 步骤 3：收紧命令

`commands/graph.rs` 的客户查询输入只接受 operation/org/query；command 从 trusted frozen snapshot 解析 `project_scope_id` 和 scope hash并验证 ownership，再调用 V2 port。禁止 caller/model直接传 `project_scope_id`、`organization_id_at_time` 集合或 classification ceiling。返回 source assertion/evidence/validity/classification，但不返回 vault、secret、raw properties。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-agent-runtime graph_auto_promotion --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-agent-app graph_scope --status-level fail
```

```bash
just precommit
git add backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs \
  backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs \
  backend/crates/golish-agent-kit/src/harness/rag_prior.rs \
  backend/crates/golish-agent-kit/src/tool_executors/graph_trait.rs \
  backend/crates/golish-agent-kit/src/tool_executors/graph.rs \
  backend/crates/golish-agent-app/src/ai/commands/graph.rs
git commit -m "fix(graph): reject prose derived customer knowledge"
```

---

## Task 5：包级验证与文档

先记录故障矩阵：两个 assertion 同 entity；两个 assertion 同 relation；同 stream 乱序；不同 stream 版本；local write 后 ack crash；Graph delivery DLQ 不影响 Document delivery；supervisor restart；organization hard delete 后历史 lineage；legacy/V2 隔离；stdout/prose 无投影；sibling-org IDOR；rebuild schema-version cutover。

```bash
cd backend && cargo nextest run --no-tests=fail -p golish-memory-app -E 'test(graph_projection) | test(graph_projector)' --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-graphiti --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-agent-app -E 'test(graph_bridge) | test(graph_scope)' --status-level fail
cd backend && cargo nextest run --no-tests=fail -p golish-agent-runtime graph_auto_promotion --status-level fail
cd backend && cargo clippy -p golish-memory-app -p golish-graphiti -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings
cd backend && cargo fmt --all -- --check
just precommit
```

更新 Task 0 已预先修改的 memory/graph/app 模块卡，以及 tool/runtime 卡；逐项执行 `clean-state-checklist.md`。外部 Graphiti 不属于本包默认验收：只有用户明确批准 endpoint、project scope、provider、classification ceiling、expiry 后，catalog 才创建独立 `graphiti-projector` delivery；其失败、retry、DLQ 不得改变 local graph delivery。

```bash
just precommit
git add docs/modules/backend/golish-memory-app.md \
  docs/modules/backend/golish-graphiti.md \
  docs/modules/backend/golish-agent-app/ai.md \
  docs/modules/backend/golish-agent-kit/tool_executors.md \
  docs/modules/backend/golish-agent-runtime/agentic_loop.md \
  docs/modules/INDEX.md \
  agent-progress.md \
  feature_list.json
git commit -m "docs(graph): document structured temporal projection"
```

外部 Graphiti live acceptance 必须另获授权；本包默认使用 mock/local deterministic tests。
