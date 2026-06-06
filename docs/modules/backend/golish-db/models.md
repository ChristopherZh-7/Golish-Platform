# golish-db / models

> **一句话职责**：DB 数据模型——表的 row 类型与 insert 类型，按域分（enums / pentest / session / wiki）。

- **类型**：目录模块（属于 crate [`golish-db`](../golish-db.md)）
- **路径**：`backend/crates/golish-db/src/models/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改某表的 row 类型或 insert 结构、DB 枚举时
- repo 方法返回类型对不上、`sqlx` row 映射报错时

## 职责

owns DB 行类型与插入类型，供 `repo/` 各表模块映射 `sqlx` 查询结果。按域拆分：`enums`（DB 枚举）/ `pentest`（pentest 域 row）/ `session`（会话/任务/工具调用 row）/ `wiki`（wiki 知识库 row）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `enums::*` | DB 枚举类型 |
| `pentest::*` | pentest 域 row + insert 类型 |
| `session::*` | 会话/任务/工具调用 row 类型 |
| `wiki::*` | wiki 知识库 row 类型 |

均经 `models/mod.rs` 的 `pub use *::*` 平铺导出。

## 关键文件

| 文件 | 作用 |
|---|---|
| `enums.rs` | DB 枚举 |
| `pentest.rs` | pentest 域 row/insert |
| `session.rs` | 会话/任务/工具调用 row |
| `wiki.rs` | wiki row |

## 依赖

- `sqlx`（`FromRow`）、`serde`、`chrono`

## 注意事项 / 坑

- row 类型与 DB schema（migration）绑定：改字段要同步 migration（I10 向后兼容）+ repo 查询。
- 跨 IPC 暴露给前端的类型应走 ts-rs（在 app 层 DTO），这里的 DB row 不一定等于 wire 类型，别混用。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db models
```
