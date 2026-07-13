# golish-graphiti

> **一句话职责**：同一 PostgreSQL 上并存的两套图 API——既有 `GraphClient` 保持 legacy 调试语义；独立 `TemporalGraphClient` 只查询 Assertion-backed、scope/lineage/validity 完整且可 generation rebuild 的 V2 projection。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-graphiti/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 操作 legacy 实体/关系图、攻击路径、知识图谱查询时
- 修改 scoped temporal graph、Assertion lineage、generation rebuild/cutover 时
- agent 的 `graph_*` 工具（add_entity/relation/search/neighbors/attack_paths）相关问题时

## 职责

提供两个互不替代的类型化 client，复用 golish-db 管理的**同一个**嵌入式 PG 实例：

- `GraphClient` 继续读写原 `graph_entities/graph_relations`，签名与旧命令语义不变；legacy 行不进入 scoped RAG。
- `TemporalGraphClient` 只读写新的 local V2 identity/lineage/generation 表。entity/relation identity 不承载单一 Assertion；每条 Assertion lineage 独立失效，仍有其它有效 lineage 时 identity 继续 active。
- rebuild 先写 building generation，验证后原子切 active；失败 generation 可丢弃，切换前旧 generation 持续服务。

V2 图只消费 immutable Memory outbox + `KnowledgeAssertion`；模型 prose、tool stdout、legacy graph 均不能反向成为事实。外部 Graphiti 不在 C3 默认路径，不发真实请求。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `GraphClient::new(pool)` | 图 client |
| `.upsert_entity(type, name, props, project)` / `.upsert_relation(from, to, type, props)` | 增改实体/关系 |
| `EntityType` / `RelationType` / `GraphEntity` / `GraphRelation` / `GraphQueryResult` | 图类型 |
| `GraphError` | 错误 |
| `TemporalGraphClient::new(pool)` | 独立 scoped temporal client，不改变 legacy client |
| `ScopedGraphQuery` / `TemporalGraphQueryResult` | exact project+org 或显式 global-sanitized 的 node+edge lineage 结果 |
| generation/rebuild API | 单 scope/schema building fence、content attestation、building→active 原子切换；失败 generation 不影响 active read |

## 依赖

- **内部**：`golish-db`（typed temporal repos + `PgPool`）、`golish-memory-domain`（Assertion/scope/source contract）

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`。agent 的图工具底座。

## 关键文件（无目录子模块）

| 文件 | 作用 |
|---|---|
| `client.rs` | `GraphClient` 实现 |
| `temporal_client.rs` | V2 scoped identity/lineage/query/rebuild client |
| `types.rs` | 实体/关系/查询类型 |
| `error.rs` | `GraphError` |

## 注意事项 / 坑

- 建表 migration 在 **golish-db**（`graph_knowledge_base`），不在本 crate；改图表结构去 golish-db 改 migration（高风险，§2.7）。
- 构造 client 必须传 `golish-db` 的 pool，别再自己 new 一个 sqlx 连接。
- 不得把 temporal error 吞成 scoped empty result；cross-project/org、global 非 technique、无 active generation均 fail closed 或返回明确空查询结果。
- node 与 edge 都只在至少一条 canonical Assertion lineage 当前有效且 fresh 时可见；edge 的两个 endpoint 也必须各自仍有 current lineage。
- incremental active write 必须由 DB trigger 同事务刷新 generation hash/count；rebuild 同一 scope/schema 同时最多一个 building owner，禁止旧慢 generation 后到反向切换。
- legacy `GraphClient` 的公开签名与现有 `kg_*` 命令必须保持兼容；C3 不重写旧表、不回填旧事实。
- 相关：`docs/graph-flow-integration.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-graphiti
```
