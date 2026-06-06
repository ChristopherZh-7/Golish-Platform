# golish-sub-agents / executor_helpers

> **一句话职责**：executor 辅助工具——assistant content 构建、聊天历史序列化/反序列化、小 helper（epoch_secs / extract_file_path / is_write_tool）。

- **类型**：目录模块（属于 crate [`golish-sub-agents`](../golish-sub-agents.md)，`pub(crate)`）
- **路径**：`backend/crates/golish-sub-agents/src/executor_helpers/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sub-agent 的 assistant content 构建、聊天历史序列化、写工具判定时

## 职责

为 `executor` 提供内部 helper：`build_assistant_content`（拼 assistant 消息内容）、`serialize_chat_history`/`deserialize_chat_history`（消息链持久化用）、`is_write_tool`/`extract_file_path`/`epoch_secs` 等小工具。

## 公开接口

| 符号 | 说明 |
|---|---|
| `build_assistant_content` | 构建 assistant 消息内容 |
| `serialize_chat_history` / `deserialize_chat_history`（`pub(crate)`） | 聊天历史序列化 |
| `is_write_tool` / `extract_file_path` / `epoch_secs`（`pub(crate)`） | 小 helper |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export |
| `assistant_content.rs` | content 构建 + 历史序列化 |
| `chain.rs` / `helpers.rs` | 消息链 / 小 helper |

## 依赖

- crate 内 `executor`/`chain_persist` 消费；`rig`、`serde_json`

## 注意事项 / 坑

- 多为 `pub(crate)` 内部 helper：仅供 `executor` 用，别对外暴露。
- 历史序列化格式与 `chain_persist`（消息链持久化）配套；改格式两边同步。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents executor_helpers
```
