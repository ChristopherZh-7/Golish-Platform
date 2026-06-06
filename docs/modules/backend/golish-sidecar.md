# golish-sidecar

> **一句话职责**：Sidecar 上下文捕获——后台被动捕获会话上下文，用 markdown 存储（`~/.golish/sessions/{id}/state.md` + patches/ + artifacts/）。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-sidecar/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改会话上下文捕获、状态 md、staged 补丁（git format-patch 风格）、commit 流程时
- 与 artifacts 文档提案联动时

## 职责

在 agent 交互期间被动捕获会话上下文，落到每会话目录：`state.md`（YAML frontmatter 元数据 + markdown 正文上下文）、`patches/`（staged/applied 补丁）、`artifacts/`（pending/applied 文档提案）。

## 公开接口 / 关键类型

| 模块 | 说明 |
|---|---|
| `capture` | 上下文捕获 |
| `commits` | 提交/补丁 |
| `processor` | 处理流水 |
| `session` / `state` | 会话与状态 md |
| `config` / `events` | 配置 / 事件 |

## 依赖

- **内部**：`golish-core`、`golish-settings`、`golish-synthesis`、`golish-artifacts`

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `capture/` | 上下文捕获 | [→](golish-sidecar/capture.md) |
| `processor/` | 处理流水 | [→](golish-sidecar/processor.md) |
| `commits/` | 补丁/提交 | [→](golish-sidecar/commits.md) |
| `session/` | 会话目录管理 | [→](golish-sidecar/session.md) |
| `state/` | state.md 读写 | [→](golish-sidecar/state.md) |
| `events/` | 事件 | [→](golish-sidecar/events.md) |

## 注意事项 / 坑

- 存储落在 `~/.golish/sessions/{session_id}/`（与 AGENTS.md §8 运行产物位置相关）。
- 补丁是标准 `git am` 可应用的 .patch + meta sidecar；staged 待 review，applied 已提交。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sidecar
```
