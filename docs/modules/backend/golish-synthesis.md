# golish-synthesis

> **一句话职责**：基于 LLM 的生成——commit 消息、状态更新、会话标题。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-synthesis/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 自动生成 commit message、会话标题、状态摘要时
- 调整这些生成用的 prompt/模板时

## 职责

封装若干 LLM 生成任务：提交消息、会话状态更新、会话标题，含各自 prompt 与模板。

## 公开接口 / 关键类型

| 模块 | 说明 |
|---|---|
| `commit` | commit 消息生成 |
| `title` | 会话标题生成 |
| `state` | 状态更新生成 |
| `prompts` / `template` / `config` | prompt / 模板 / 配置 |

## 依赖

- **内部**：`golish-settings`

## 被谁依赖 / 改动影响面

`golish`、`golish-sidecar`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `state/` | 状态更新生成 | [→](golish-synthesis/state.md) |

## 关键文件

`commit.rs`、`title.rs`、`prompts.rs`、`template.rs`、`config.rs`。

## 注意事项 / 坑

- 纯生成逻辑，依赖很轻（只 settings）；改 prompt 注意与 sidecar 的状态/提交流程对齐。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-synthesis
```
