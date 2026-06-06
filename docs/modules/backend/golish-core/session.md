# golish-core / session

> **一句话职责**：本地会话归档——AI 对话的持久化（创建/finalize/查找/列举），JSON 文件存 `~/.golish/sessions/`（或 `$VT_SESSION_DIR`），是 `vtcode_core::utils::session_archive` 的 drop-in 替代，接口严格向后兼容。

- **类型**：目录模块（属于 crate [`golish-core`](../golish-core.md)）
- **路径**：`backend/crates/golish-core/src/session/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改会话归档格式、metadata、消息结构（role/content/tool_call_id）时
- 改会话查找/列举（按 identifier 找、列最近 N 条）时
- 会话文件读不出、旧格式兼容、`VT_SESSION_DIR` 解析问题时

## 职责

提供 AI 对话的会话持久化能力，作为 vtcode-core session_archive 的 drop-in 替代，**接口契约必须保留**（`SessionArchive::new` / `finalize` / `find_session_by_identifier` / `list_recent_sessions` / `SessionMessage::with_tool_call_id` / `content.as_text()`）。存储为 JSON 文件，向后兼容已有会话文件。

## 公开接口

| 符号 | 说明 |
|---|---|
| `SessionArchive` / `SessionArchiveMetadata` | 归档主体 + metadata（`new` → `finalize`） |
| `SessionMessage` / `MessageRole` / `MessageContent` | 消息类型（`with_tool_call_id` / `user` / `assistant`） |
| `find_session_by_identifier` / `list_recent_sessions` | 查找单条 / 列最近（倒序） |
| `SessionListing` / `SessionSnapshot` | 列举结果 + 快照（预览 helper） |
| `get_sessions_dir(_for)` / `find_session_in_workspace` / `list_sessions_for_workspace` | 存储目录解析（内部/测试用） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `archive.rs` | `SessionArchive` + metadata（创建/finalize） |
| `listing.rs` | 查找/列举 + 预览 |
| `message.rs` | `SessionMessage` / `MessageRole` / `MessageContent` |
| `storage/` | 存储目录解析 + 跨 workspace 查找 |

## 依赖

- `tokio::fs`（异步文件）、`serde_json`、`chrono`、`dirs`（home 解析）

## 注意事项 / 坑

- **接口契约硬约束**：被 `golish-session`（上层）依赖，签名改了会断 drop-in 兼容（见 mod.rs 头 7 条契约）。
- 存储目录优先 `$VT_SESSION_DIR`，否则 `~/.golish/sessions/`；测试用 env 隔离 + `serial_test`（并发会串目录）。
- 必须读旧格式 JSON（向后兼容），改 schema 别破坏旧文件反序列化。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-core session
```
