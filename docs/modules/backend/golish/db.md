# golish / db

> **一句话职责**：DB 适配器占位——`Pg*Store`（domain trait 的 Postgres 实现）re-export 点；当前**为空**（`PgPentestStore` 已随 pentest 命令面搬到 `golish-pentest-app`，M3/M4-proper）。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/db/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 某个**留在 golish** 的 domain crate 采用 trait + adapter 模式、需要 re-export 其 `Pg*Store` 时

## 职责

为留在 golish crate 的 domain 提供 `Pg*Store` adapter re-export 的归属点。目前空：`PgPentestStore` 已随 crate-per-service 拆分搬到 `golish-pentest-app`。

## 公开接口

| 符号 | 说明 |
|---|---|
| （当前为空） | 占位 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 占位（仅 doc 注释） |

## 依赖

- 无（占位）

## 注意事项 / 坑

- 当前是**空占位**：别误以为缺实现——adapter 已随服务拆分外移；新增仅当有「留在 golish」的 domain 采用 adapter 模式时。

## 测试入口

```bash
cd backend && cargo nextest run -p golish db
```
