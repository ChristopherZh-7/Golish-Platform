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

提供嵌入式 Postgres 的启动与连接池，以及结构化数据访问。owns `graph_knowledge_base` 等 migration（golish-graphiti 的表也在此）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `GolishDb::start(DbConfig)` / `.pool()` / `.stop()` | DB 句柄（持有嵌入式 PG + 池） |
| `create_lazy_pool` / `PgPool`(re-export sqlx) | 连接池 |
| `repo::*`（sessions / tool_calls / memories / audit …） | 各表 CRUD（scoped） |
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

- **不变量 I2**：所有 CRUD 验资源所有权（IDOR），含批量；repo 是 scoped CRUD。
- **不变量 I9**：事务内禁止外部 HTTP/MQ/长耗操作（连接池雪崩）。
- **不变量 I10**：改 schema 必须向后兼容（先扩字段→上新代码→清旧字段）。
- migration `20260710000001_technique_outcomes_org_scoped_unique.sql` 是明确的 downtime-only 例外：它把 `technique_outcomes` 唯一键改为 `(organization_id, run_id, asset, technique)`，并把 target-bound `directory_entries` 唯一键改为 `(target_id, url, tool)`。部署时必须先停止旧 writer，再将 migration 与新 binary 一起上线；迁移完成后旧 binary 与新约束不兼容，禁止继续运行或回切旧 binary。该迁移只重建约束/索引并保留业务 rows，不授权在线混跑新旧写入方。
- `pool.rs::repair_migrations` 只能修复 `_sqlx_migrations` 中 `success=true`、版本与 description 均匹配的历史 checksum，并用旧 checksum 做 compare-and-swap；missing migration 必须留给下一次 SQLx migrator 真正执行 SQL，dirty/failed 与 phantom history 必须保持失败，绝不能伪造 success、跳过 DDL 或盲删追踪行。
- `repo/organizations.rs::subtree` 返回 root organization + descendants 的完整行，并按 root first / child sort order 排序；runtime `stage_run` 用它作为 continuation fan-out scope truth，防止模型续跑少传子公司导致资产分母缺口。
- `repo/tool_calls.rs::scoping_actions_for_session` 是 red-team scoping anti-shortcut gate 的 DB 审计口径：必须能识别真实 `unit_review`，并把 `manage_organizations(create_batch)` 的 `created`/`existing` id 当作已记录组织；但 REUSE 模式不应强迫创建。
- 相关设计：`docs/database-and-tools.md`、`docs/superpowers/plans/2026-05-30-p1-1-golish-db-scoped-crud-helper.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db
```
