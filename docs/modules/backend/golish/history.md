# golish / history

> **一句话职责**：命令/终端历史管理——`entry`（历史条目）+ `manager`（历史管理器）+ `storage`（文件存储，fs2 文件锁）+ `error`。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/history/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改命令/终端历史的条目结构、读写存储、文件锁、历史管理器时

## 职责

管理 shell/命令历史：`entry` 条目类型、`manager` 历史管理器（背景初始化，见 `app::bootstrap::init_history_manager_background`）、`storage` 文件持久化（`fs2` 文件锁防并发损坏）、`error` 错误。

## 公开接口

| 符号 | 说明 |
|---|---|
| `entry::*` | 历史条目类型 |
| `manager::*` | 历史管理器 |
| `storage::*` | 文件存储（fs2 锁） |
| `error::*` | 错误类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `entry.rs` / `manager.rs` | 条目 / 管理器 |
| `storage.rs` / `error.rs` | 文件存储 / 错误 |

## 依赖

- `fs2`（文件锁）、`serde`

## 注意事项 / 坑

- `fs2` 文件锁防多进程并发写损坏；改存储别去掉锁。
- 管理器后台初始化（不阻塞启动）；见 `app::bootstrap`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish history
```
