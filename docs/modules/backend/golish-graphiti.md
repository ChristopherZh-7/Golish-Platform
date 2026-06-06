# golish-graphiti

> **一句话职责**：PostgreSQL 支撑的图知识库——存 pentest 过程中积累的安全发现（host/service/vuln/credential/technique/endpoint）及其有向边（runs_service / has_vulnerability / exploited_by / lateral_move …）。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-graphiti/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 操作实体/关系图、攻击路径、知识图谱查询时
- agent 的 `graph_*` 工具（add_entity/relation/search/neighbors/attack_paths）相关问题时

## 职责

提供类型化的实体/关系图 client。复用 golish-db 管理的**同一个**嵌入式 PG 实例（建表 migration 在 golish-db），构造时传入 `golish_db::PgPool`。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `GraphClient::new(pool)` | 图 client |
| `.upsert_entity(type, name, props, project)` / `.upsert_relation(from, to, type, props)` | 增改实体/关系 |
| `EntityType` / `RelationType` / `GraphEntity` / `GraphRelation` / `GraphQueryResult` | 图类型 |
| `GraphError` | 错误 |

## 依赖

- **内部**：`golish-db`（取 `PgPool`，共用嵌入式 PG）

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`。agent 的图工具底座。

## 关键文件（无目录子模块）

| 文件 | 作用 |
|---|---|
| `client.rs` | `GraphClient` 实现 |
| `types.rs` | 实体/关系/查询类型 |
| `error.rs` | `GraphError` |

## 注意事项 / 坑

- 建表 migration 在 **golish-db**（`graph_knowledge_base`），不在本 crate；改图表结构去 golish-db 改 migration（高风险，§2.7）。
- 构造 client 必须传 `golish-db` 的 pool，别再自己 new 一个 sqlx 连接。
- 相关：`docs/graph-flow-integration.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-graphiti
```
