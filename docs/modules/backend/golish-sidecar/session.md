# golish-sidecar / session

> **一句话职责**：sidecar 会话存储——每个会话一个目录：`state.md`（YAML frontmatter + markdown 正文）+ `patches/{staged,applied}/`，含 `SessionStatus`（active/completed/abandoned）。

- **类型**：目录模块（属于 crate [`golish-sidecar`](../golish-sidecar.md)）
- **路径**：`backend/crates/golish-sidecar/src/session/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sidecar 会话目录结构、`state.md`（frontmatter + 正文）读写时
- 改会话状态机（active/completed/abandoned）或 patches staged/applied 布局时

## 职责

定义 sidecar 会话的磁盘存储：每会话一目录，`state.md` 用 YAML frontmatter 存元数据 + markdown 正文存上下文，`patches/staged/` 存待应用补丁、`patches/applied/` 存已应用（`git am` 后移入）。`SessionStatus` 跟踪生命周期。

## 公开接口

| 符号 | 说明 |
|---|---|
| `SessionStatus`（`Active`/`Completed`/`Abandoned`，`Display`/`FromStr`） | 会话状态 |
| （会话读写类型 + state.md frontmatter 解析） | 目录/文件管理 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 会话存储类型 + `SessionStatus` + state.md 读写 |
| `tests.rs` | 单测 |

## 依赖

- `tokio::fs`、`serde`（frontmatter）、`chrono`、`anyhow`

## 注意事项 / 坑

- `state.md` 是 YAML frontmatter + markdown 正文混合格式；解析要兼容两段，别只当纯 markdown。
- 补丁两阶段（staged→applied）：`git am` 成功后才移到 applied，别提前移动。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sidecar session
```
