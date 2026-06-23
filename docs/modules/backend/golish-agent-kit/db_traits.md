# golish-agent-kit / db_traits

> **一句话职责**：DB 操作的 trait 抽象 + 本地模型类型——把 agent 层与 `golish-db`/`sqlx` 完全解耦：`DbRepoProvider`（CRUD）/ `DbTrackingBackend`（记录+记忆）/ `DbReadinessGate`（PG 就绪门）/ `TextEmbedder`（语义记忆嵌入），由 application 层注入实现。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/db_traits/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agent 层与 DB 的边界 trait（repo/tracking/readiness/embedder）或本地 DTO 时
- 排查 agent 层为何不直接依赖 golish-db/sqlx 时

## 职责

定义 agent 层访问 DB 所需的 trait + 本地模型，使本 crate **不依赖 golish-db/sqlx**（依赖倒置）。application 层（golish-agent-app 的 db_bridge）提供具体实现。

## 公开接口

| 符号 | 说明 |
|---|---|
| `DbRepoProvider` | 仓库操作（tasks/subtasks/plans/wiki… CRUD） |
| `DbTrackingBackend` | fire-and-forget 记录 + memory 存/搜 |
| `DbReadinessGate` | PG 启动就绪门 |
| `TextEmbedder` | 语义记忆文本嵌入 |
| `types` / `memory` / `repo` / `tracking`（本地 DTO） | trait + 本地模型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `DbReadinessGate` + re-export |
| `repo.rs` / `tracking.rs` / `memory.rs` / `types.rs` | 各 trait + 本地 DTO |

## 依赖

- `async-trait`、`uuid`；**无 golish-db/sqlx**（这是本模块的全部意义）

## 注意事项 / 坑

- **依赖倒置核心**：本 crate 定义 trait，golish-agent-app 注入实现——别在此引 golish-db/sqlx（会破坏 agent 栈的可测/解耦）。
- 本地 DTO 与 golish-db 的 row 类型是两套，由 bridge 转换。
- harness gate 相关读写也走 `DbRepoProvider` seam：`technique_outcome_facts` 投影 coverage truth，`source_query_facts` 投影 `source_query_log` 的 source/provider terminal rows；后者只证明 source 尝试，不证明 found。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit db_traits
```
