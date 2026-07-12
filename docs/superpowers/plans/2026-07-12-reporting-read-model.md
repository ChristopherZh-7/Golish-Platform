# Scoped Cited Reporting Read Model 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 用 frozen scope、canonical facts、Candidate/Attempt lineage、post-exploit outcome、cleanup residual 和 evidence citation 构建 versioned ReportReadModel，让 LLM 只负责叙事排版而不能创造事实。

**架构：** 新建 reporting domain/app；builder 在 PostgreSQL `REPEATABLE READ READ ONLY` 快照中读取 frozen scope 与 typed truth，冻结逐 source version/hash 的 canonical manifest。事实落为 `report_claims`，引用落为 `report_claim_citations`；renderer 只能改 claim 对应叙事。revision 将 validation 与 publication 拆成两条正交状态轴：Reporting stage 只把 validation 置为 validated，Gate 始终认这份 attestation；只有用户显式 finalize 才改变 publication，并经过 content-addressed staging/hash/promote/DB commit。finalize 同事务向 P3 `knowledge_outbox_events` 追加 `ReportFinalized.v1`，delivery 由 catalog 推导；历史 final 不 cascade 删除且不反向喂 KG。

**技术栈：** Rust 2021、sqlx/PostgreSQL、Tauri/ts-rs、React/TypeScript、现有 artifact storage、cargo-nextest/Vitest。

**依赖：** P3 Memory Fabric、P2 Candidate V2、P6 Post-exploit、P7 Cleanup；schema/IPC/外部 LLM 渲染前取得用户确认。

---

## 1. 文件结构

### 新建

- `backend/crates/golish-reporting-domain/{Cargo.toml,src/lib.rs}`
- `backend/crates/golish-reporting-domain/src/{report.rs,section.rs,citation.rs,revision.rs,validation.rs}`
- `backend/crates/golish-reporting-app/{Cargo.toml,src/lib.rs}`
- `backend/crates/golish-reporting-app/src/{ports.rs,read_model.rs,renderer.rs,redaction.rs,finalizer.rs}`
- `backend/crates/golish-db/migrations/20260712000007_reporting_read_model.sql`
- `backend/crates/golish-db/src/repo/{reports.rs,report_revisions.rs,report_sections.rs,report_claims.rs,report_claim_citations.rs,report_artifacts.rs}`
- `backend/crates/golish-db/tests/reporting_read_model_migrations.rs`
- `resources/harness/stages/reporting/methodology.md`
- `backend/crates/golish-agent-app/src/ai/commands/reporting.rs`
- `backend/crates/golish-agent-app/src/ai/{db_bridge/reporting.rs,reporting_artifact_store.rs}`
- `backend/crates/golish-agent-app/tests/reporting_end_to_end.rs`
- `backend/crates/golish/src/commands_facade/reporting.rs`
- `frontend/lib/api/reporting.ts`
- `frontend/components/Engagement/{ReportReadModelView.tsx,ReportReadModelView.test.tsx}`
- `docs/modules/backend/{golish-reporting-domain.md,golish-reporting-app.md}`

### 修改

- `backend/Cargo.toml`
- `backend/crates/golish-db/src/repo/mod.rs`
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/{execute.rs,execute_harness_loop_tests.rs}`
- `backend/crates/golish-agent-kit/src/harness/gate/{context_builder.rs,rule_engine.rs}`
- `resources/harness/stages/reporting/spec.json`
- `backend/crates/golish-agent-app/src/ai/{commands/mod.rs,db_bridge/mod.rs}`
- `backend/crates/golish/src/commands_facade/mod.rs`
- `backend/crates/golish/src/commands_registry.rs`
- `frontend/components/Engagement/index.ts`
- `docs/modules/backend/{golish-db.md,golish-db/repo.md,golish-agent-kit/task_orchestrator.md,golish-agent-kit/harness.md,golish-agent-app/ai.md,golish/commands_facade.md}`
- `docs/modules/frontend/{components.md,lib.md}`
- `docs/modules/INDEX.md`

---

## Task 0：开工门禁与先建模块卡

**文件：** `docs/modules/backend/{golish-reporting-domain.md,golish-reporting-app.md}`、上列既有模块卡、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`。

### 步骤 1：验证上下文与权限

1. 读取 `agent-progress.md`、`feature_list.json`、`docs/modules/INDEX.md` 和本计划涉及的全部模块卡。
2. 若另一个 feature 已 `in_progress`，停止并请求用户决定；不自行切换。
3. schema/migration、IPC/ts-rs 类型链和 live LLM renderer 分别取得用户明确批准；普通测试使用 fake renderer/artifact store。
4. 运行 `./init.sh`，预期 exit 0；失败先处理基础 blocker。
5. 运行 `git status --short`；计划文件若已有用户 hunk，在干净 worktree 实施或请求 reconcile，禁止覆盖或目录级 staging。

### 步骤 2：代码前创建模块卡

按现有模板创建 reporting-domain/app 两张模块卡，先写清 claim/citation authority、REPEATABLE READ source manifest、validated-vs-final、artifact 两阶段提交、历史保留与测试入口；同步 INDEX 为 `planned`。在 DB/harness/app/commands/frontend 卡中登记 seam，后续 Task 随实现校正。

### 步骤 3：提交模块卡 checkpoint

```bash
git diff --check
git add -- docs/modules/backend/golish-reporting-domain.md docs/modules/backend/golish-reporting-app.md docs/modules/backend/golish-db.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish/commands_facade.md docs/modules/frontend/components.md docs/modules/frontend/lib.md docs/modules/INDEX.md agent-progress.md feature_list.json
just precommit
git diff --cached --name-only
git commit -m "docs(reporting): register cited report module boundaries"
```

预期：`just precommit` exit 0；暂存区只含上列精确文件。用户未授权 commit 时只记录 checkpoint。

---

## Task 1：定义 ReportReadModel、citation 与 validator

**文件：** reporting-domain 的 `report.rs`、`section.rs`、`citation.rs`、`revision.rs`、`validation.rs`、`lib.rs`。

### 步骤 1：写具体 RED

```rust
#[test]
fn candidate_without_verified_finding_cannot_enter_findings_section() {
    let mut model = valid_report_model();
    model.findings.push(report_finding_from_candidate("candidate-7", None));
    let error = validate_report(&model, &validation_truth()).expect_err("candidate is not finding");

    assert!(error.iter().any(|item| item.code == "FINDING_LINEAGE_REQUIRED"));
}

#[test]
fn every_claim_requires_a_resolvable_same_org_citation() {
    let mut model = valid_report_model();
    let claim_id = model.organization_sections[0].claims[0].claim_id;
    model.organization_sections[0].claims[0].citation_ids.clear();
    let error = validate_report(&model, &validation_truth()).expect_err("uncited claim");

    assert!(error.iter().any(|item| item.code == "CLAIM_CITATION_REQUIRED" && item.claim_id == Some(claim_id)));
}

#[test]
fn sibling_org_citation_and_secret_value_are_rejected() {
    let mut model = valid_report_model();
    model.organization_sections[0].claims[0].value = serde_json::json!({"password":"hunter2"});
    model.citations[0].organization_id_at_time = uuid("00000000-0000-0000-0000-000000000011");
    let error = validate_report(&model, &validation_truth()).expect_err("scope and secret violations");

    assert!(error.iter().any(|item| item.code == "CITATION_ORG_MISMATCH"));
    assert!(error.iter().any(|item| item.code == "SECRET_VALUE_FORBIDDEN"));
}

#[test]
fn blocked_cleanup_requires_residual_claim() {
    let mut model = valid_report_model();
    model.cleanup_residuals.clear();
    let truth = validation_truth().with_cleanup(cleanup_truth("blocked"));
    let error = validate_report(&model, &truth).expect_err("missing residual");

    assert!(error.iter().any(|item| item.code == "CLEANUP_RESIDUAL_REQUIRED"));
}
```

```bash
cd backend && cargo nextest run -p golish-reporting-domain report_validation --no-tests=fail --status-level fail
```

预期：crate/types 不存在，RED；不能以 0 tests 通过。

### 步骤 2：定义模型

```rust
pub struct ReportReadModel {
    pub report_id: Uuid,
    pub revision_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_hash: String,
    pub source_manifest: ReportSourceManifest,
    pub source_hash: String,
    pub executive_summary: ReportSectionModel,
    pub organization_sections: Vec<OrganizationReportSection>,
    pub findings: Vec<ReportFinding>,
    pub attack_paths: Vec<ReportAttackPath>,
    pub cleanup_residuals: Vec<ReportResidual>,
    pub methodology: Vec<StageOutcomeSummary>,
    pub limitations: Vec<ReportLimitation>,
}

pub struct ReportCitation {
    pub citation_id: Uuid,
    pub claim_id: Uuid,
    pub source_type: CitationSourceType,
    pub source_table: String,
    pub source_id: String,
    pub source_version: i64,
    pub source_hash: String,
    pub evidence_audit_id: Option<i64>,
    pub organization_id_at_time: Uuid,
    pub display_label: String,
}

pub struct ReportClaim {
    pub claim_id: Uuid,
    pub section_id: Uuid,
    pub organization_id_at_time: Option<Uuid>,
    pub claim_kind: ReportClaimKind,
    pub subject_ref: String,
    pub predicate: String,
    pub value: serde_json::Value,
    pub citation_ids: Vec<Uuid>,
    pub ordinal: i32,
}

pub struct ReportSourceManifest {
    pub transaction_snapshot: String,
    pub sources: Vec<ReportSourceVersion>,
}

pub struct ReportSourceVersion {
    pub table: String,
    pub row_id: String,
    pub version: i64,
    pub content_hash: String,
}
```

`ReportClaim` 只保存 typed value 与 citation ids，不复制 evidence body。manifest 按 `(table,row_id,version,content_hash)` 排序后 canonical JSON SHA-256；每个 claim/citation 也冻结对应 source version/hash。以后 source 改变只能创建新 revision，不能原地改旧 claim。

### 步骤 3：validator

```rust
pub fn validate_report(model: &ReportReadModel, truth: &ReportValidationTruth) -> Result<ReportValidationResult, ReportValidationError>;
```

检查：current verified Finding、candidate lineage、citation resolve、claim/citation 同 revision 同 org、frozen scope、redaction、cleanup residual、source manifest/hash、revision current。validator 不接受模型新造的 claim id，也不把 Citation label 当 source truth。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-reporting-domain report_validation --no-tests=fail --status-level fail
```

```bash
git diff --check
git add -- backend/Cargo.toml backend/crates/golish-reporting-domain/Cargo.toml backend/crates/golish-reporting-domain/src/lib.rs backend/crates/golish-reporting-domain/src/report.rs backend/crates/golish-reporting-domain/src/section.rs backend/crates/golish-reporting-domain/src/citation.rs backend/crates/golish-reporting-domain/src/revision.rs backend/crates/golish-reporting-domain/src/validation.rs
just precommit
git diff --cached --name-only
git commit -m "feat(reporting): define scoped cited report read model"
```

---

## Task 2：新增 versioned report schema/repo

**文件：** migration、migration test、`reports.rs`、`report_revisions.rs`、`report_sections.rs`、`report_claims.rs`、`report_claim_citations.rs`、`report_artifacts.rs`、`repo/mod.rs`。

### 步骤 1：写具体 RED

```rust
#[tokio::test]
async fn current_revision_composite_fk_rejects_revision_from_another_report() {
    let db = ReportingMigrationFixture::fresh().await;
    let report_a = db.insert_report(operation_id(1)).await;
    let report_b = db.insert_report(operation_id(2)).await;
    let revision_b = db.insert_revision(report_b, 1, source_manifest("sha256:b")).await;
    let error = db.set_current_revision(report_a, revision_b).await.expect_err("cross-report pointer");

    assert_eq!(error.constraint(), Some("reports_current_revision_belongs_to_report"));
}

#[tokio::test]
async fn final_history_cannot_be_deleted_by_report_or_revision_cascade() {
    let db = ReportingMigrationFixture::fresh().await;
    let report = db.insert_report(operation_id(1)).await;
    let revision = db.insert_final_revision(report, source_manifest("sha256:final")).await;
    db.insert_claim_with_citation(revision, source_version("findings", "f-1", 7, "sha256:f1")).await;
    let error = db.delete_report(report).await.expect_err("final history is retained");

    assert!(error.is_foreign_key_violation() || error.code() == Some("FINAL_HISTORY_IMMUTABLE"));
    assert_eq!(db.claim_count(revision).await, 1);
    assert_eq!(db.citation_count(revision).await, 1);
}

#[tokio::test]
async fn citation_must_belong_to_the_same_revision_and_freeze_source_version_hash() {
    let db = ReportingMigrationFixture::fresh().await;
    let (revision_a, claim_a) = db.insert_revision_with_claim(1).await;
    let revision_b = db.insert_revision_for_same_report(2).await;
    let error = db.insert_citation(revision_b, claim_a, source_version("audit_log", "77", 3, "sha256:audit-77-v3"))
        .await
        .expect_err("cross-revision citation");

    assert_eq!(error.constraint(), Some("report_claim_citations_same_revision"));
    let source = db.insert_citation(revision_a, claim_a, source_version("audit_log", "77", 3, "sha256:audit-77-v3")).await;
    assert_eq!(source.source_version, 3);
    assert_eq!(source.source_hash, "sha256:audit-77-v3");
}
```

```bash
cd backend && cargo nextest run -p golish-db reporting_read_model_migrations --no-tests=fail --status-level fail
```

预期：migration/tables 不存在，RED。测试必须在真实 ephemeral PostgreSQL 上跑 fresh migration，不允许 env 缺失时 early return。

### 步骤 2：创建表

```sql
CREATE TABLE reports (
    report_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL UNIQUE REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    scope_snapshot_id UUID NOT NULL REFERENCES operation_org_scope_snapshots(id) ON DELETE RESTRICT,
    scope_snapshot_hash TEXT NOT NULL,
    current_revision_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE report_revisions (
    revision_id UUID PRIMARY KEY,
    report_id UUID NOT NULL REFERENCES reports(report_id) ON DELETE RESTRICT,
    revision_number INTEGER NOT NULL,
    source_manifest JSONB NOT NULL,
    source_hash TEXT NOT NULL,
    validation_status TEXT NOT NULL CHECK (validation_status IN ('building','draft','validated','invalid')),
    publication_status TEXT NOT NULL CHECK (publication_status IN ('unpublished','final','superseded')),
    supersedes_revision_id UUID,
    validation_result JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    validated_at TIMESTAMPTZ,
    finalized_at TIMESTAMPTZ,
    finalized_by TEXT,
    UNIQUE(report_id, revision_id),
    UNIQUE(report_id, revision_number),
    CONSTRAINT report_revision_supersedes_same_report
      FOREIGN KEY (report_id, supersedes_revision_id)
      REFERENCES report_revisions(report_id, revision_id) ON DELETE RESTRICT
);

CREATE TABLE report_sections (
    section_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    organization_id_at_time UUID,
    organization_name_at_snapshot TEXT,
    section_kind TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    rendered_content TEXT,
    content_hash TEXT NOT NULL,
    UNIQUE(revision_id, section_id),
    UNIQUE(revision_id, organization_id_at_time, section_kind, ordinal)
);

CREATE TABLE report_claims (
    claim_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL,
    section_id UUID NOT NULL,
    organization_id_at_time UUID,
    claim_kind TEXT NOT NULL,
    subject_ref TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object_value JSONB NOT NULL,
    claim_hash TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    UNIQUE(revision_id, claim_id),
    UNIQUE(revision_id, section_id, ordinal),
    CONSTRAINT report_claims_same_revision_section
      FOREIGN KEY (revision_id, section_id)
      REFERENCES report_sections(revision_id, section_id) ON DELETE RESTRICT
);

CREATE TABLE report_claim_citations (
    citation_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL,
    claim_id UUID NOT NULL,
    citation_ordinal INTEGER NOT NULL,
    source_type TEXT NOT NULL,
    source_table TEXT NOT NULL,
    source_id TEXT NOT NULL,
    source_version BIGINT NOT NULL,
    source_hash TEXT NOT NULL,
    evidence_audit_id BIGINT REFERENCES audit_log(id),
    organization_id_at_time UUID,
    display_label TEXT NOT NULL,
    UNIQUE(revision_id, claim_id, citation_ordinal),
    CONSTRAINT report_claim_citations_same_revision
      FOREIGN KEY (revision_id, claim_id)
      REFERENCES report_claims(revision_id, claim_id) ON DELETE RESTRICT
);

CREATE TABLE report_artifacts (
    artifact_id UUID PRIMARY KEY,
    revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    format TEXT NOT NULL,
    content_key TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    redaction_version INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(revision_id, format),
    UNIQUE(content_key, sha256)
);

ALTER TABLE reports ADD CONSTRAINT reports_current_revision_belongs_to_report
  FOREIGN KEY (report_id, current_revision_id)
  REFERENCES report_revisions(report_id, revision_id)
  ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
```

不要给 report→revision→claim/citation/artifact 历史链使用 `ON DELETE CASCADE`。删除只走 repo `purge_unvalidated_draft`，显式逐表删且仅允许 `validation_status=building|draft AND publication_status=unpublished`；validated 或已发布/被 supersede 的 revision 全部拒绝。`organization_id_at_time`/`organization_name_at_snapshot` 是 frozen report provenance，写入时验证属于 report snapshot，但不建 live organizations FK；查询/授权 DTO 仍可叫 current organization scope，不能把历史列误当 live row。

### 步骤 3：repo transaction

```rust
pub async fn begin_revision(tx: &mut Transaction<'_, Postgres>, command: BeginReportRevision) -> Result<RevisionRow>;
pub async fn store_read_model(tx: &mut Transaction<'_, Postgres>, command: StoreReportReadModel) -> Result<()>;
pub async fn validate_revision(tx: &mut Transaction<'_, Postgres>, command: ValidateRevision) -> Result<()>;
pub async fn finalize_revision_with_artifacts_and_outbox(tx: &mut Transaction<'_, Postgres>, command: FinalizeRevision) -> Result<()>;
pub async fn purge_unvalidated_draft(tx: &mut Transaction<'_, Postgres>, report_id: Uuid, revision_id: Uuid) -> Result<()>;
```

`store_read_model` 同事务写 sections→claims→claim citations，并验证每个 claim 至少一个 resolvable citation、citation source version/hash 位于 manifest。`validate_revision` 用 expected source hash/version CAS 将 validation `draft→validated` 并更新 `reports.current_revision_id`，publication 保持 `unpublished`。`finalize_revision_with_artifacts_and_outbox` 只接受 current `validation_status=validated` revision、显式 actor、已由 artifact store 验证的 content refs；同事务把本 revision publication `unpublished→final`、旧 final publication→superseded、写 artifact rows，并调用 P3 `append_event_with_catalog_deliveries` 追加 immutable `ReportFinalized.v1` 到 `knowledge_outbox_events`。producer 只给 report/revision/artifact typed refs，不传 routes；catalog 固定派生 `report-artifact-indexer@1` delivery，禁止 Assertion/Document/Embedding/KG route。validation_status 永远保持 validated，claim/content/source manifest 永不原地修改。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-db -E 'test(report_) | test(reporting_read_model_migrations)' --no-tests=fail --status-level fail
```

```bash
git diff --check
git add -- backend/crates/golish-db/migrations/20260712000007_reporting_read_model.sql backend/crates/golish-db/tests/reporting_read_model_migrations.rs backend/crates/golish-db/src/repo/reports.rs backend/crates/golish-db/src/repo/report_revisions.rs backend/crates/golish-db/src/repo/report_sections.rs backend/crates/golish-db/src/repo/report_claims.rs backend/crates/golish-db/src/repo/report_claim_citations.rs backend/crates/golish-db/src/repo/report_artifacts.rs backend/crates/golish-db/src/repo/mod.rs
just precommit
git diff --cached --name-only
git commit -m "feat(db): add versioned cited report read model"
```

---

## Task 3：实现 deterministic read model builder

**文件：** `reporting-app/{read_model.rs,ports.rs,lib.rs}`、`agent-app/db_bridge/{reporting.rs,mod.rs}`、`backend/Cargo.toml`。

### 步骤 1：写具体 RED

```rust
#[tokio::test]
async fn repeatable_read_build_freezes_one_manifest_and_one_section_per_scope_unit() {
    let fixture = ReportBuilderFixture::new()
        .scope_units([scope_unit(10, "root"), scope_unit(11, "subsidiary")])
        .finding(versioned_finding("finding-1", 4, "sha256:finding-v4"));
    let built = fixture.builder().build(operation_id(1)).await.expect("build");

    assert_eq!(fixture.transaction_isolation(), "repeatable read");
    assert!(fixture.transaction_was_read_only());
    assert_eq!(built.model.organization_sections.len(), 2);
    assert_eq!(built.manifest.sources, vec![source("findings", "finding-1", 4, "sha256:finding-v4")]);
    assert_eq!(built.source_hash, sha256_of_canonical_manifest(&built.manifest));
}

#[tokio::test]
async fn builder_keeps_non_verified_candidates_out_of_findings() {
    let fixture = ReportBuilderFixture::new()
        .candidate(candidate_truth("candidate-rejected", "rejected"))
        .candidate(candidate_truth("candidate-refuted", "refuted"))
        .candidate(candidate_truth("candidate-verified", "verified"))
        .finding(finding_for_candidate("finding-7", "candidate-verified"));
    let model = fixture.builder().build(operation_id(1)).await.expect("build").model;

    assert_eq!(model.findings.iter().map(|f| f.finding_id.as_str()).collect::<Vec<_>>(), vec!["finding-7"]);
    assert!(model.methodology.iter().any(|item| item.subject_id == "candidate-rejected"));
    assert!(model.methodology.iter().any(|item| item.subject_id == "candidate-refuted"));
}

#[tokio::test]
async fn validation_rejects_source_version_changed_after_snapshot() {
    let fixture = ReportBuilderFixture::new().finding(versioned_finding("finding-1", 4, "sha256:v4"));
    let built = fixture.builder().build(operation_id(1)).await.expect("build");
    fixture.update_finding("finding-1", 5, "sha256:v5");
    let error = fixture.validator().validate_current(built).await.expect_err("stale manifest");

    assert_eq!(error, ReportBuildError::SourceManifestChanged { table: "findings".into(), row_id: "finding-1".into(), expected_version: 4, actual_version: 5 });
    assert_eq!(fixture.stored_revision_count(), 0);
}
```

```bash
cd backend && cargo nextest run -p golish-reporting-app report_read_model --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app reporting_bridge --no-tests=fail --status-level fail
```

预期：builder/adapter 尚不存在，RED。

### 步骤 2：定义 truth ports

```rust
#[async_trait]
pub trait ReportTruthReader {
    async fn read_snapshot(&self, tx: &mut Transaction<'_, Postgres>, operation_id: Uuid) -> Result<ReportTruthSnapshot>;
    async fn current_source_versions(&self, tx: &mut Transaction<'_, Postgres>, manifest: &ReportSourceManifest) -> Result<Vec<ReportSourceVersion>>;
}
```

`ReportTruthSnapshot` 一次包含 frozen scope、validated StageRunUnit outcomes、PASS handoff/Episode refs、current verified Findings、Candidate/Attempt lineage、post-exploit outcomes、cleanup attestations/residuals、evidence metadata；每行必须带 deterministic `(table,row_id,version,content_hash)`。adapter 精确落在 `backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs`，只实现 port 和 row→domain 转换，不在 agent-kit 拼散 SQL。

### 步骤 3：REPEATABLE READ + source manifest

1. `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`，在同一 snapshot 内读取全部 typed truth；禁止每类 source 各开 autocommit read。
2. 在读事务内把每个实际消费 row 的 table/id/version/content_hash 收入 canonical manifest；排序、计算 source hash，然后 commit。source hash 不是 `MAX(updated_at)` 或总行数。
3. 事务外只做纯 deterministic model 构造，不调用 RAG/LLM。
4. 短写事务中重新锁 report，并按 manifest 批量比较 current version/hash；完全一致才写 revision/sections/claims/citations并 validate。任一 source 新增、删除、版本或 hash 变化返回 `SourceManifestChanged`，整次不落半成品。

不存在版本列的旧 canonical table 必须先由 owning domain 提供 deterministic content hash/version adapter；不能退回 `updated_at` 猜测。所有 section claim 来自 typed truth，不调用开放式 RAG。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-reporting-app report_read_model --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app reporting_bridge --no-tests=fail --status-level fail
```

```bash
git diff --check
git add -- backend/Cargo.toml backend/crates/golish-reporting-app/Cargo.toml backend/crates/golish-reporting-app/src/lib.rs backend/crates/golish-reporting-app/src/ports.rs backend/crates/golish-reporting-app/src/read_model.rs backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs
just precommit
git diff --cached --name-only
git commit -m "feat(reporting): build reports from canonical operation truth"
```

---

## Task 4：实现 redaction、renderer 与 artifact finalizer

**文件：** `reporting-app/{renderer.rs,redaction.rs,finalizer.rs,ports.rs,lib.rs}`、`agent-app/reporting_artifact_store.rs`。

### 步骤 1：写具体 RED

```rust
#[test]
fn renderer_cannot_add_or_drop_claims() {
    let input = narrative_input([claim("claim-1", "Finding exists", ["citation-1"])]);
    let output = narrative_output([rendered_claim("claim-1", "Finding exists"), rendered_claim("claim-evil", "Admin access")]);
    let error = validate_narrative_output(&input, &output).expect_err("invented claim");

    assert_eq!(error, NarrativeError::ClaimSetMismatch { expected: ids(["claim-1"]), actual: ids(["claim-1", "claim-evil"]) });
}

#[test]
fn redaction_removes_secret_value_and_preserves_vault_reference_label() {
    let redacted = redact_report_value(serde_json::json!({
        "password":"hunter2",
        "credential":"vault_ref:00000000-0000-0000-0000-000000000021"
    })).expect("redacted");

    assert!(!redacted.to_string().contains("hunter2"));
    assert!(redacted.to_string().contains("vault_ref:00000000-0000-0000-0000-000000000021"));
}

#[tokio::test]
async fn content_addressed_stage_promote_and_replay_are_idempotent() {
    let store = TempReportArtifactStore::new();
    let staged = store.stage(revision_id(1), ReportFormat::Markdown, b"same bytes").await.expect("stage");
    let first = store.promote(&staged).await.expect("promote");
    let replay = store.promote(&staged).await.expect("replay");

    assert_eq!(first.sha256, sha256_hex(b"same bytes"));
    assert_eq!(first.content_key, replay.content_key);
    assert_eq!(store.final_blob_count(), 1);
    assert!(store.verify(&first).await.expect("verify"));
}

#[tokio::test]
async fn finalizer_rejects_unvalidated_revision_without_publishing_artifact_rows() {
    let fixture = FinalizerFixture::new()
        .validation_status(ValidationStatus::Draft)
        .publication_status(PublicationStatus::Unpublished);
    let error = fixture.finalizer().finalize(explicit_user_finalize("analyst-a")).await.expect_err("draft cannot finalize");

    assert_eq!(error, FinalizeError::RevisionNotValidated);
    assert_eq!(fixture.db().artifact_rows(), 0);
    assert_eq!(fixture.db().knowledge_outbox_event_rows(), 0);
}

#[tokio::test]
async fn gc_removes_stale_staging_and_unreferenced_final_but_keeps_referenced_final() {
    let store = TempReportArtifactStore::new()
        .with_stale_staging("stage-old")
        .with_final_blob("sha256-orphan")
        .with_final_blob("sha256-live");
    let result = store.gc(server_now(), referenced_hashes(["sha256-live"])).await.expect("gc");

    assert_eq!(result.removed_staging, vec!["stage-old"]);
    assert_eq!(result.removed_unreferenced, vec!["sha256-orphan"]);
    assert!(store.contains_final("sha256-live"));
}
```

```bash
cd backend && cargo nextest run -p golish-reporting-app -E 'test(renderer) | test(redaction) | test(finalizer) | test(content_addressed)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app reporting_artifact_store --no-tests=fail --status-level fail
```

预期：renderer/artifact port 不存在，RED。

### 步骤 2：renderer contract

```rust
#[async_trait]
pub trait NarrativeRenderer {
    async fn render(&self, input: NarrativeRenderInput) -> Result<NarrativeRenderOutput>;
}
```

输入只含 section structured facts/citation labels/redacted excerpts；输出必须按 claim id 回填 narrative，不允许新增 claim/finding id。validator 比较 source claim set，新增/缺失均拒绝。

renderer adapter 必须用 `ToolPreset::None`/空 registry，provider request 不附任何 tool definitions。真实 LLM rendering 属外部请求，默认测试只用 deterministic fake；无 renderer 时允许 deterministic template 输出，但仍不能改 claim set。

### 步骤 3：artifact pipeline

定义：

```rust
#[async_trait]
pub trait ReportArtifactStore {
    async fn stage(&self, revision_id: Uuid, format: ReportFormat, bytes: &[u8]) -> Result<StagedArtifact>;
    async fn promote(&self, staged: &StagedArtifact) -> Result<ContentAddressedArtifact>;
    async fn verify(&self, artifact: &ContentAddressedArtifact) -> Result<bool>;
    async fn discard_staging(&self, staged: &StagedArtifact) -> Result<()>;
    async fn gc(&self, now: DateTime<Utc>, referenced_hashes: BTreeSet<String>) -> Result<ArtifactGcReport>;
}
```

本地 adapter 精确落在 `backend/crates/golish-agent-app/src/ai/reporting_artifact_store.rs`：staging 写 `{workspace}/.golish/reports/.staging/<revision>/<uuid>.<format>`；边写边 SHA-256、fsync/close 后重读验证；promote 用同 filesystem 原子 rename/put-if-absent 到 `{workspace}/.golish/reports/sha256/<first-two>/<sha256>.<format>`，content key 只由 hash+format 生成，不接受 caller storage path。

finalize 顺序固定：

1. 事务外生成 Markdown/JSON 必需 bytes（PDF/DOCX 是可选独立 adapter）。
2. 事务外 stage→hash→promote→read-back verify。
3. 短 DB transaction 重新验证 explicit actor ownership、current revision、`validation_status=validated`、`publication_status=unpublished`、expected source hash/revision version；只把 publication 改为 final、写 artifact refs，并用 P3 catalog API 追加 `ReportFinalized.v1`；validation 仍为 validated。
4. commit 后删除 staging。DB rollback 后 final content blob 无引用，由 GC 清理；不得在 DB transaction 内做文件/LLM/HTTP。
5. startup/daily GC 删除超过 TTL 的 staging，以及不在 `report_artifacts.sha256` 中且超过 grace period 的 final blobs；永不删除有 DB 引用的 final artifact。

报告正文不作为 Assertion/KG 输入；`ReportFinalized.v1` payload 只含 report/revision/artifact typed pointer，delivery 由 P3 catalog 生成且不得路由到 Assertion/KG/embedding。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-reporting-app -E 'test(renderer) | test(redaction) | test(finalizer) | test(content_addressed)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app reporting_artifact_store --no-tests=fail --status-level fail
```

```bash
git diff --check
git add -- backend/crates/golish-reporting-app/src/renderer.rs backend/crates/golish-reporting-app/src/redaction.rs backend/crates/golish-reporting-app/src/finalizer.rs backend/crates/golish-reporting-app/src/ports.rs backend/crates/golish-reporting-app/src/lib.rs backend/crates/golish-agent-app/src/ai/reporting_artifact_store.rs
just precommit
git diff --cached --name-only
git commit -m "feat(reporting): render and finalize immutable cited artifacts"
```

---

## Task 5：替换 Reporting stage seam 与 Gate

**文件：** `execute.rs`、`execute_harness_loop_tests.rs`、gate `context_builder.rs/rule_engine.rs`、reporting `spec.json/methodology.md`。

### 步骤 1：写具体 RED

```rust
#[tokio::test]
async fn reporting_stage_builds_and_validates_but_never_auto_finalizes() {
    let fixture = ReportingStageFixture::new().with_valid_truth();
    let outcome = fixture.run_reporting_stage().await.expect("stage");

    assert_eq!(outcome.validation_status, ValidationStatus::Validated);
    assert_eq!(outcome.publication_status, PublicationStatus::Unpublished);
    assert_eq!(outcome.gate_verdict, GateVerdict::Pass);
    assert_eq!(fixture.finalizer_calls(), 0);
    assert_eq!(fixture.artifact_rows(), 0);
}

#[tokio::test]
async fn reporting_gate_blocks_unresolved_claim_citation() {
    let fixture = ReportingStageFixture::new().with_claim(claim_without_resolvable_citation("claim-7"));
    let outcome = fixture.run_reporting_gate().await.expect("gate outcome");

    assert_eq!(outcome.verdict, GateVerdict::Block);
    assert_eq!(outcome.reason_code, "REPORT_CITATION_UNRESOLVED");
}

#[tokio::test]
async fn reporting_gate_passes_only_current_validated_revision_with_current_manifest() {
    let fixture = ReportingStageFixture::new()
        .with_revision(revision(1, ValidationStatus::Validated, PublicationStatus::Final, "sha256:old"))
        .with_revision(revision(2, ValidationStatus::Validated, PublicationStatus::Unpublished, "sha256:current"))
        .with_current_revision(2)
        .with_current_source_hash("sha256:current");
    let outcome = fixture.run_reporting_gate().await.expect("gate outcome");

    assert_eq!(outcome.verdict, GateVerdict::Pass);
    assert_eq!(outcome.revision_id, revision_id(2));
}
```

```bash
cd backend && cargo nextest run -p golish-agent-kit reporting_read_model_gate --no-tests=fail --status-level fail
```

预期：旧 stage 仍可基于 execution summary 或自动 final，RED。

### 步骤 2：显式 stage task

`synthesize_stage_subtask(StageKind::Reporting)` 固定生成：build REPEATABLE READ snapshot/revision → deterministic/no-tools narrative render → validate → publish current validated revision。到此结束，禁止调用 finalize、artifact store、扫描工具和开放式 knowledge retrieval。reporting methodology 明确“Gate PASS 不等于报告已最终发布”。

### 步骤 3：Gate op

新增 `report_revision_validated`，DB truth 加载失败 BLOCK；只认 `reports.current_revision_id` 指向、`validation_status=validated` 的 revision，且 claims/citations 完整、source manifest/hash current、redaction/residual checks passed。publication 可以是 `unpublished|final`，不改变 validation attestation；`validation_status=draft|building|invalid` 一律 BLOCK。`publication_status=superseded` 的旧 revision因不是 current 不能 PASS。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-kit reporting_read_model_gate --no-tests=fail --status-level fail
python3 -m json.tool resources/harness/stages/reporting/spec.json
```

```bash
git diff --check
git add -- backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs backend/crates/golish-agent-kit/src/harness/gate/context_builder.rs backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs resources/harness/stages/reporting/spec.json resources/harness/stages/reporting/methodology.md
just precommit
git diff --cached --name-only
git commit -m "feat(harness): gate reporting on validated canonical read model"
```

---

## Task 6：Reporting IPC/UI

**文件：** app command/mod/adapter/artifact store、facade/registry、frontend API/component/test/index。

### 步骤 1：写具体 RED

```rust
#[tokio::test]
async fn finalize_requires_explicit_user_request_and_current_validated_revision() {
    let fixture = ReportingCommandFixture::new()
        .owned_operation(operation_id(1), "analyst-a")
        .current_revision(revision(2, ValidationStatus::Validated, PublicationStatus::Unpublished, "sha256:source-v2"));
    let response = reporting_finalize_revision(
        fixture.state_for("analyst-a"),
        FinalizeReportRequest {
            operation_id: operation_id(1),
            revision_id: revision_id(2),
            expected_source_hash: "sha256:source-v2".into(),
            expected_revision_version: 3,
            confirm_final_publish: true,
        },
    ).await.expect("explicit finalize");

    assert_eq!(response.status, "final");
    assert_eq!(response.artifacts.len(), 2);
    assert!(response.artifacts.iter().all(|artifact| artifact.content_key.starts_with("sha256/")));
    assert_eq!(fixture.audit_actor(), Some("analyst-a"));
}

#[tokio::test]
async fn finalize_rejects_idor_stale_hash_and_missing_confirmation() {
    let fixture = ReportingCommandFixture::new()
        .owned_operation(operation_id(1), "analyst-a")
        .current_revision(revision(2, ValidationStatus::Validated, PublicationStatus::Unpublished, "sha256:current"));
    let foreign = fixture.finalize_as("analyst-b", true, "sha256:current").await.expect_err("idor");
    let stale = fixture.finalize_as("analyst-a", true, "sha256:stale").await.expect_err("stale");
    let unconfirmed = fixture.finalize_as("analyst-a", false, "sha256:current").await.expect_err("confirm");

    assert_eq!(foreign.code, "REPORT_FORBIDDEN");
    assert_eq!(stale.code, "REPORT_SOURCE_CHANGED");
    assert_eq!(unconfirmed.code, "REPORT_FINALIZE_CONFIRMATION_REQUIRED");
    assert_eq!(fixture.finalized_count(), 0);
}
```

```tsx
it("finalizes only after the user confirms the validated revision", async () => {
  const api = mockReportingApi({ current: validatedRevision("rev-2"), artifacts: [] });
  render(<ReportReadModelView operationId="op-1" api={api} />);

  await screen.findByText("Validated draft");
  await userEvent.click(screen.getByRole("button", { name: "Finalize report" }));
  await userEvent.click(screen.getByRole("button", { name: "Confirm final publish" }));

  expect(api.finalizeRevision).toHaveBeenCalledWith(expect.objectContaining({ revisionId: "rev-2", confirmFinalPublish: true }));
  expect(await screen.findByText("Final artifact" )).toBeInTheDocument();
});
```

```bash
cd backend && cargo nextest run -p golish-agent-app reporting_commands --no-tests=fail --status-level fail
pnpm exec vitest run frontend/components/Engagement/ReportReadModelView.test.tsx
```

预期：commands/UI 不存在或 stage 自动 final，RED。

### 步骤 2：commands

```text
reporting_get_read_model
reporting_list_revisions
reporting_get_artifacts
reporting_finalize_revision
```

finalize command 使用 expected source hash/revision version，冲突后重新 build；不能 final 一个 stale draft。

`reporting_finalize_revision` 是唯一 final 入口：actor/ownership 从 authenticated app state 取得，request 只能提供 expected values 和显式确认。stage、LLM、renderer、background projector 均不得调用它。按 Tauri 五步实现 function→facade `pub use`→registry→`frontend/lib/api/reporting.ts` wrapper→ts-rs 类型生成；禁止手改 `frontend/lib/generated/`。

UI 必须覆盖 loading/error/empty、org sections、claim citation/source version、candidate disposition、cleanup residual、revision superseded badge；只有 current `validation_status=validated AND publication_status=unpublished` revision显示 finalize 按钮，final/superseded/draft 均不可重复发布。

### 步骤 3：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-app reporting_commands --no-tests=fail --status-level fail
pnpm exec vitest run frontend/components/Engagement/ReportReadModelView.test.tsx
just check-fe
just test-fe
```

```bash
git diff --check
git add -- backend/crates/golish-agent-app/src/ai/commands/reporting.rs backend/crates/golish-agent-app/src/ai/commands/mod.rs backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs backend/crates/golish-agent-app/src/ai/reporting_artifact_store.rs backend/crates/golish/src/commands_facade/reporting.rs backend/crates/golish/src/commands_facade/mod.rs backend/crates/golish/src/commands_registry.rs frontend/lib/api/reporting.ts frontend/components/Engagement/ReportReadModelView.tsx frontend/components/Engagement/ReportReadModelView.test.tsx frontend/components/Engagement/index.ts
just precommit
git diff --cached --name-only
git commit -m "feat(ui): present cited report revisions and residual risks"
```

---

## Task 7：包级验证与文档

### 步骤 1：增加跨层不变量测试

在 `backend/crates/golish-agent-app/tests/reporting_end_to_end.rs` 写真实 DB + fake artifact store 测试：

```rust
#[tokio::test]
async fn explicit_finalize_preserves_validation_gate_and_catalog_routing() {
    let fixture = ReportingEndToEndFixture::migrated().await;
    let revision = fixture.build_and_validate(operation_id(1)).await.expect("validated revision");
    let before = fixture.run_reporting_gate(operation_id(1)).await.expect("gate before");
    fixture.finalize_as("analyst-a", revision, true).await.expect("explicit finalize");
    let after = fixture.run_reporting_gate(operation_id(1)).await.expect("gate after");
    let stored = fixture.revision(revision).await;
    let event = fixture.knowledge_event("ReportFinalized.v1").await;

    assert_eq!(before.verdict, GateVerdict::Pass);
    assert_eq!(after.verdict, GateVerdict::Pass);
    assert_eq!(stored.validation_status, ValidationStatus::Validated);
    assert_eq!(stored.publication_status, PublicationStatus::Final);
    assert_eq!(event.delivery_projectors, vec!["report-artifact-indexer@1"]);
    assert!(!event.delivery_projectors.iter().any(|name| name.contains("assertion") || name.contains("graph") || name.contains("embedding")));
}

#[tokio::test]
async fn failed_db_finalize_leaves_gc_eligible_blob_and_no_publication() {
    let fixture = ReportingEndToEndFixture::migrated().await.with_finalize_commit_failure();
    let revision = fixture.build_and_validate(operation_id(1)).await.expect("validated revision");
    let error = fixture.finalize_as("analyst-a", revision, true).await.expect_err("commit failure");

    assert_eq!(error.code(), "REPORT_FINALIZE_COMMIT_FAILED");
    assert_eq!(fixture.revision(revision).await.publication_status, PublicationStatus::Unpublished);
    assert_eq!(fixture.artifact_rows().await, 0);
    assert_eq!(fixture.knowledge_events("ReportFinalized.v1").await.len(), 0);
    assert_eq!(fixture.gc_eligible_final_blobs().await.len(), 1);
}
```

```bash
cd backend && cargo nextest run -p golish-agent-app --test reporting_end_to_end --no-tests=fail --status-level fail
```

预期：在所有 Task 完成前 RED；完成后两条用例都 passed。

### 步骤 2：包级门禁

```bash
cd backend && cargo nextest run -p golish-reporting-domain --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-reporting-app --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-db -E 'test(report_) | test(reporting_read_model_migrations)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit reporting_read_model_gate --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app -E 'test(reporting_commands) | test(reporting_bridge) | test(reporting_artifact_store)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app --test reporting_end_to_end --no-tests=fail --status-level fail
pnpm exec vitest run frontend/components/Engagement/ReportReadModelView.test.tsx
just check-fe
just test-fe
cd backend && cargo clippy -p golish-reporting-domain -p golish-reporting-app -p golish-db -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings
cd backend && cargo fmt --all -- --check
just precommit
```

预期：全部 exit 0，nextest filters 均至少匹配一个测试，Clippy 零 warning；stage validated 后 PASS、用户 finalize 后仍 PASS，只有 publication 变化。

### 步骤 3：同步模块卡与提交

Task 0 已先创建 reporting domain/app 卡；此处把实际接口、DB constraints、P3 catalog event、artifact GC、Gate/IPC/UI 测试入口同步回两张新卡及 DB/harness/app/commands/frontend 卡和 INDEX，并把 fresh 命令证据写入 progress/feature。

```bash
git diff --check
git add -- backend/crates/golish-agent-app/tests/reporting_end_to_end.rs docs/modules/backend/golish-reporting-domain.md docs/modules/backend/golish-reporting-app.md docs/modules/backend/golish-db.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish/commands_facade.md docs/modules/frontend/components.md docs/modules/frontend/lib.md docs/modules/INDEX.md agent-progress.md feature_list.json
just precommit
git diff --cached --name-only
git commit -m "docs(reporting): document scoped cited finalization contract"
```

---

## 最终不变量

- Candidate 不能出现在 Findings section，除非有 current verified Finding lineage。
- 每个事实 claim 必须有可解析 citation。
- Reporting 不开放 RAG，不创造新事实。
- blocked/waived cleanup residual 必须披露。
- secret 值不能进入 read model、prompt 或 artifact。
- validation attestation 与 publication 分轴；finalize 后仍是 validated，Gate 重跑不倒退。
- final revision 的 claims/manifest/artifacts immutable；更新必须新建 revision，并只把旧 publication 标为 superseded。
- `ReportFinalized.v1` 只写 P3 `knowledge_outbox_events`，catalog 只派生 `report-artifact-indexer@1` delivery。
