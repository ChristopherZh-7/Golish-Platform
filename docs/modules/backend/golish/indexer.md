# golish / indexer

> **一句话职责**：代码索引薄包装——re-export `golish-indexer` + Tauri 命令（因依赖 AppState 留主 crate）+ `vtcode_bridge`。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/indexer/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改索引相关 Tauri 命令、vtcode 后端桥接时

## 职责

`golish-indexer`（基础设施）的薄 Tauri 包装：`commands`（索引命令，因依赖 AppState 留主 crate）+ `vtcode_bridge`（vtcode-indexer 后端桥）+ `pub use golish_indexer::*`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `commands` | 索引 Tauri 命令 |
| `vtcode_bridge` | vtcode 后端桥 |
| re-export `golish_indexer::*` | 基础设施类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export + 子模块声明 |
| `commands/` | Tauri 命令 |

## 依赖

- `golish-indexer`、`vtcode-indexer`、`tauri`

## 注意事项 / 坑

- 命令因 AppState 依赖留主 crate；本层**不** re-export commands（避免命令名 shadow `golish_indexer::*` 类型）——经 `crate::indexer::commands::*` 访问。

## 测试入口

```bash
cd backend && cargo nextest run -p golish indexer
```
