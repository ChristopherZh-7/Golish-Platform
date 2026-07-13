# golish-db

> **一句话职责**：Golish 的 PostgreSQL 持久化层——嵌入式 PG（pg_embed 自动下载+生命周期）+ pgvector 语义检索 + session→task→subtask→tool_call 层级 + pentest 数据 + token 用量分析。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-db/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 任何 DB 读写、加 repo CRUD、改 schema/migration、向量记忆检索时
- DB 启动失败、连接池、事务问题时
- ⚠️ 改 schema/migration 是 **AGENTS.md §2.7 高风险操作，必须先问用户**

## 职责

- C4 Post-Exploit canonical spine：`foothold_candidates`、`footholds`、`internal_asset_observations`、`attack_paths/edges`、`post_exploit_actions/approvals`、`objective_attempts`；全部绑定 frozen operation/project/org-at-time 与 evidence。
- Foothold / ObjectiveOutcome terminal row 与 `PostExploitFactTerminal.v1` outbox deliveries 同事务提交；P6a 数据库 trigger 拒绝任何非 `none` side effect。
- 四个信息阶段（Target Intel / EAS / Enumeration / Vuln Triage）的 final seal 在同一 caller-owned compound transaction 内关闭 immutable `StageEpisode` 并写 `StageEpisodeClosed.v1` outbox/catalog deliveries；Memory 写失败会连同 Unit/Worker/Handoff/completion 一起回滚。
- Candidate V2 terminalizer 在同一 caller-owned transaction 内终结 Attempt/Candidate、创建 verified Finding/lineage、提出 FactDelta、释放 lane/Worker，并写 deterministic `CandidateAttemptTerminal.v1` + catalog deliveries；outbox 任一写失败会把全部 canonical terminal state 回滚，exact response-loss replay 不重复事件或 delivery。
- C7 `knowledge_context` 只暴露 scope-first read model：先锁定 operation/frozen snapshot/org/classification/validity，再排序或做向量相似度；没有 project-null/org-null/legacy global fallback。
- C5/C8 Cleanup ledger：side-effect action 与 cleanup obligation 通过 deferred exact back-reference 原子绑定；terminal obligation 必须在 deferred commit boundary 具备 exact attempt/absence/waiver/blocked decision 与独立 evidence 关系，Gate 与组织删除 precheck 同时重算该 relational truth，不能只信 status 字符串。`CleanupObligationTerminal.v1` 读取的 obligation/attempt/absence/waiver/blocked-decision evidence membership 与 parent terminal transition 通过行锁串行化：同一事务可先写 child 再 terminalize，terminal commit 后五类 child 均禁止新增，既有 membership 继续禁止 UPDATE/DELETE。CleanupAttempt 只允许一次 live→`verified_absent|verification_failed|execution_failed`；OLD 已 terminal 后整行 UPDATE（含 no-op）/DELETE 均拒绝，失败重试必须创建下一 ordinal 的新 Attempt。
- side-effect action+obligation compound transaction 在 exact evidence 后从 persisted rows 生成 `PostExploitActionPrepared.v1` + mandatory deliveries；payload 不含 raw plan/secret。

提供嵌入式 Postgres 的启动与连接池，以及结构化数据访问。owns `graph_knowledge_base` 等 migration（golish-graphiti 的表也在此）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `GolishDb::start(DbConfig)` / `.pool()` / `.stop()` | DB 句柄（持有嵌入式 PG + 池） |
| `create_lazy_pool` / `PgPool`(re-export sqlx) | 连接池 |
| `repo::*`（sessions / tool_calls / memories / audit …） | 各表 CRUD（scoped） |
| Candidate V2 repos | operation-frozen rollout、Wave/manifest/plan-bound approval、Attempt+WorkerRun+lane compound lease、FactDelta、verified Finding lineage |
| Runtime Memory final seal | 四信息阶段的 Unit/Worker/Handoff/completion + `StageEpisode` + `StageEpisodeClosed.v1` deliveries 原子提交 |
| `repo::knowledge_context` | DB-owned ContextPack authorization snapshot 与 canonical/runtime/handoff/assertion/document/vector scoped queries |
| `DbConfig` / `DbError` / `models::*` | 配置/错误/数据模型 |
| `gatekeeper` / `embeddings` | 准入门 / 向量 |

## 依赖

- **内部**：`golish-core`、`golish-platform`

## 被谁依赖 / 改动影响面

`golish`、各 `*-app`、`golish-app-core`、`golish-graphiti`、`golish-integrations`、`golish-scan-runner`、`golish-pentest`、`golish-vuln-intel`。**改 schema 影响面极大**。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `repo/` | 各表 scoped CRUD helper | [→](golish-db/repo.md) |
| `models/` | DB 数据模型 | [→](golish-db/models.md) |
| `embedded/` | 嵌入式 PG 启动/生命周期 | [→](golish-db/embedded.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `config.rs` / `pool.rs` / `error.rs` | 配置 / 连接池与 fail-closed migration repair / 错误 |
| `gatekeeper.rs` | 准入门 |
| `embeddings.rs` | 向量嵌入 |

## 注意事项 / 坑

- **C9 Reporting read model（2026-07-13）**：`20260712000011_reporting_read_model.sql` 新增 reports/revisions/sections/claims/citations/source-manifest/content-addressed blob refs。manifest/section/claim/citation 在 validation 进入 `validated|invalid` 后即对 INSERT/UPDATE/DELETE 全冻结，不等 publication；artifact child 可在 validated+unpublished 阶段 attach，但 final/superseded 后同样冻结。UPDATE 同时检查 OLD/NEW owner 防“搬到 draft”绕过；所有 child I/U/D 都对 parent 取 `FOR UPDATE`，与 draft→validated/finalize 的 parent row mutation 冲突，禁止验证后夹带 child。validated+unpublished revision 本体拒绝 attestation/source/revision metadata 的任何 UPDATE，只允许保持其余列 exact 的 publication transition；final→superseded 也只允许 `publication_status` 与 trigger-owned `row_version` 改变。`final` 不是裸 enum flip：deferred constraint 在 commit 时要求 exact current revision、active local principal、至少一个 content-addressed artifact ref，以及 row-version/operation/project/stream/payload exact 的 `ReportRevisionFinalized.v1` outbox；普通 SQL 只设 final 必须失败。被任一 revision 引用的 blob 拒绝所有 UPDATE；被 validated/invalid/final/superseded revision 作为 `stage_handoff` source 保留的 final seal也拒绝 invalidation/delete（`REPORT_SEALED_REF_RETAINED`）。终态 `candidate_attempts` / `cleanup_obligations` 是 deterministic outbox canonical source：只允许一次 nonterminal→terminal，之后 canonical at-time/event 字段、no-op UPDATE 与 DELETE 都以 `TERMINAL_CANONICAL_SOURCE_IMMUTABLE` 拒绝，保证 row-version/event replay identity 稳定。唯一例外是删除 live target 后由 FK 驱动的 `candidate_attempts.target_live_id non-NULL→NULL`；该字段不是 canonical snapshot 内容，其他整行必须 exact 不变且 `row_version` 不递增，目标仍存在时的直接清空仍被拒绝。finalize 必须完整重读 source set、exact compare、CAS，并与 artifact refs/outbox 同事务；RAG/KG 不参与 Gate。

- **不变量 I2**：所有 CRUD 验资源所有权（IDOR），含批量；repo 是 scoped CRUD。
- **不变量 I9**：事务内禁止外部 HTTP/MQ/长耗操作（连接池雪崩）。
- **不变量 I10**：改 schema 必须向后兼容（先扩字段→上新代码→清旧字段）。
- migration `20260710000001_technique_outcomes_org_scoped_unique.sql` 是明确的 downtime-only 例外：它把 `technique_outcomes` 唯一键改为 `(organization_id, run_id, asset, technique)`，并把 target-bound `directory_entries` 唯一键改为 `(target_id, url, tool)`。部署时必须先停止旧 writer，再将 migration 与新 binary 一起上线；迁移完成后旧 binary 与新约束不兼容，禁止继续运行或回切旧 binary。该迁移只重建约束/索引并保留业务 rows，不授权在线混跑新旧写入方。
- `20260712000001_runtime_memory_foundation.sql` 是已冻结的 additive runtime-memory expand migration：四态 rollout singleton 只能 `rank+1`，operation contract/project scope 不可漂移，scope snapshot 采用 `snapshot → units → sealed_at` 一次性冻结，tool/worker/submission/handoff 以 composite identity + row locks 防跨 operation 与 lease TOCTOU。其 SHA-384 由 integration test 固定；后续变化必须新建更高版本 migration，不能改写本文件。
- `20260712000003_trusted_operator_principal.sql` 提供 server-owned local operator UUID；Candidate approval 的 `decided_by` 只 FK 到该 principal，request/model 不能选择 actor。`20260712000004_attack_execution_v2.sql` 是 additive Candidate V2 ownership spine：attack rollout 只能 forward CAS，operation 创建时冻结 contract，final-pass handoff/完整 manifest、plan-bound approval、Attempt/WorkerRun/lane、relational evidence 和 Finding lineage 均由 DB composite FK/trigger 约束。
- `20260712000009_cleanup_obligation_ledger.sql` 在不开放任何 Tool 的前提下安装 P7a kernel：非 `none` action 必须在同事务带 active server principal 与 exact obligation back-reference；三态 live Attempt partial unique、independent absence evidence、trusted waiver residual/CAS 与 retained history 均由 DB 纵深约束。
- `20260712000010_cleanup_closeout.sql` 安装 exact terminal-truth deferred constraint、Cleanup evidence/decision immutability 与两阶段组织删除。Deletion job 只冻结 active server-owned `project_scopes.canonical_project_path`，锁后重读 subtree 防并发扩张，active subtree 禁止外部 reparent；artifact/hard-delete 使用独立 durable backoff deadline，防失败 hot-loop 与队列饥饿。
- Candidate review barrier 是 durable wake authority：review close 写 `resume_pending`，resume command CAS `dispatching`，成功后写 `resumed`；DB startup reaper 会把超时 dispatch 重置为 pending，同时保留所有 plan-bound approval/rejection 行。
- `pool.rs::repair_migrations` 只允许按 **version + description + exact old/new SHA-384** 明确登记的 migration-specific checksum repair，并用旧 checksum 做 compare-and-swap；当前 allowlist 为空。仅版本/description 匹配不足以修复，任何未登记 drift 必须 fail closed，尤其不得把扩写过的 runtime foundation 误标成已完整执行。missing migration 留给下一次 SQLx migrator 真正执行 SQL，dirty/failed 与 phantom history 同样保持失败。
- `repo/organizations.rs::subtree` 返回 root organization + descendants 的完整行，并按 root first / child sort order 排序；runtime `stage_run` 用它作为 continuation fan-out scope truth，防止模型续跑少传子公司导致资产分母缺口。
- `repo/tool_calls.rs::scoping_actions_for_session` 是 red-team scoping anti-shortcut gate 的 DB 审计口径：必须能识别真实 `unit_review`，并把 `manage_organizations(create_batch)` 的 `created`/`existing` id 当作已记录组织；但 REUSE 模式不应强迫创建。
- 相关设计：`docs/database-and-tools.md`、`docs/superpowers/plans/2026-05-30-p1-1-golish-db-scoped-crud-helper.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db
```
