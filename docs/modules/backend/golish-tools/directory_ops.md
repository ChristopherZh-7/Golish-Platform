# golish-tools / directory_ops

> **一句话职责**：agent 的 3 个目录/检索工具——`list_files`（glob）、`list_directory`（浅列）、`grep_file`（正则搜内容），全部 gitignore 友好且经 `path_policy` 沙箱校验。

- **类型**：目录模块（属于 crate [`golish-tools`](../golish-tools.md)）
- **路径**：`backend/crates/golish-tools/src/directory_ops/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改文件检索/列目录工具行为时
- agent 搜不到文件、漏 `.gitignore` 内文件、或 grep 结果不对时

## 职责

实现 3 个 `Tool`，覆盖「列文件 / 列目录 / 搜内容」。检索类（list_files / grep_file）用 `ignore::WalkBuilder`，**默认尊重 `.gitignore`**；都经 `path_policy` 限制在 workspace 内。

## 公开接口

| 工具 struct | 工具名 | 关键参数 / 行为 |
|---|---|---|
| `ListFilesTool` | `list_files` | 可选 `path` / `pattern`(glob 如 `**/*.ts`) / `recursive`(默认 true)；尊重 `.gitignore` |
| `ListDirectoryTool` | `list_directory` | `path`(必填)；浅列单层，带文件/目录类型标记 |
| `GrepFileTool` | `grep_file` | `pattern`(必填正则) + 可选 `path` / `include`(glob 过滤如 `*.rs`)；返回匹配行 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `list_files.rs` / `list_directory.rs` / `grep_file.rs` | 3 个工具各自实现 |

## 依赖

- `crate::path_policy::{is_within_workspace, join_workspace}`（沙箱）
- `golish_core::{Tool, utils::*}`、`ignore`（gitignore-aware 遍历）、`regex`（grep）、`glob`

## 注意事项 / 坑

- 检索默认**跳过 `.gitignore` 命中的文件**；要搜被忽略文件得另调整 `WalkBuilder`，别误以为是 bug。
- `grep_file` 用 Rust `regex` 语法（非 PCRE），无效正则返回 `error` 而非 panic。
- schema 在 [`definitions/`](definitions.md) 的 `directory_declarations()`，改参数两边同步。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-tools directory_ops
```
