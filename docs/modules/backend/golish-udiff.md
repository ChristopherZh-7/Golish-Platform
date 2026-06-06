# golish-udiff

> **一句话职责**：统一 diff 编辑模块——解析 LLM 输出的 unified diff，并以灵活匹配策略应用到文件（多 hunk 外科手术式编辑）。

- **类型**：crate（Layer 2 基础设施，纯 Rust 叶子）
- **路径**：`backend/crates/golish-udiff/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 diff 解析、hunk 应用、匹配策略时
- sub-agent 的 diff 编辑应用失败/匹配不上时

## 职责

把 LLM 输出里的 unified diff 解析成 hunk，并用灵活匹配应用到文件内容；匹配不上时回报带 suggestion 的错误供 LLM 修正。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `UdiffParser` / `ParsedDiff` / `ParsedHunk` | 解析 diff 块与 hunk |
| `UdiffApplier::apply_hunks` / `ApplyResult` | 应用 hunk（Success / NoMatch{suggestion} 等） |
| `PatchError` / `PatchErrorType` | 错误类型 |

## 依赖

- **内部**：无（纯 Rust 实现）

## 被谁依赖 / 改动影响面

`golish-sub-agents`（子 agent 的代码编辑）。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `applier/` | hunk 匹配与应用 | [→](golish-udiff/applier.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `parser.rs` | 解析 unified diff |
| `error.rs` | `PatchError` 类型 |

## 注意事项 / 坑

- `ApplyResult::NoMatch` 带 `suggestion`，用于把失败原因回报给 LLM 重试——别吞掉这个 suggestion。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-udiff
```
