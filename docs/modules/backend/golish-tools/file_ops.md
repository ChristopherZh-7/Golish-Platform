# golish-tools / file_ops

> **一句话职责**：agent 的 5 个文件操作工具（read / write / create / edit / delete），每个动词一个文件，全部经 `path_policy` 沙箱校验。

- **类型**：目录模块（属于 crate [`golish-tools`](../golish-tools.md)）
- **路径**：`backend/crates/golish-tools/src/file_ops/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `read_file` / `write_file` / `create_file` / `edit_file` / `delete_file` 任一行为时
- agent 报"文件没找到 / 越权 / 二进制文件读不了"时
- 要调整 `edit_file` 的匹配语义或 diff 输出时

## 职责

实现 5 个 `Tool`，逐个对应文件动词。每个工具：① 用 `golish_core::utils::get_required_str` 解析参数；② 用 `path_policy::resolve_path_checked` 把路径锁进 workspace；③ 按「成功/失败契约」返回 JSON（失败带 `error` 字段）。

## 公开接口

| 工具 struct | 工具名 | 关键参数 / 行为 |
|---|---|---|
| `ReadFileTool` | `read_file` | `path`，可选 `line_start` / `line_end`；成功返回 `content` |
| `WriteFileTool` | `write_file` | `path` + `content`；覆盖写 |
| `CreateFileTool` | `create_file` | `path` + `content`；**文件已存在则报错** |
| `EditFileTool` | `edit_file` | `path` + `old_text` + `new_text`；`old_text` **必须恰好匹配一次**，返回 unified diff |
| `DeleteFileTool` | `delete_file` | `path`；删除文件 |

均 `pub use` 出去，由上层 `registry.rs` 注册。

## 关键文件（单文件，不再细分卡）

| 文件 | 作用 |
|---|---|
| `read.rs` / `write.rs` / `create.rs` / `edit.rs` / `delete.rs` | 5 个工具各自实现 |
| `helpers.rs` | `is_binary_file()`：查前 8000 字节有无 `\0` 判定二进制（防把二进制当文本读写） |
| `tests.rs` | 模块单测 |

## 依赖

- `crate::path_policy::resolve_path_checked`（**强制**：所有路径都先过它，防逃逸出 workspace）
- `golish_core::{Tool, utils::*}`、`similar`（edit 生成 diff）

## 注意事项 / 坑

- `edit_file` 的 `old_text` **匹配 0 次或 >1 次都直接失败**（返回带建议的 `error`），不会瞎改。改这里要同步更新错误提示文案。
- 所有路径必须走 `resolve_path_checked`，新增/修改工具时别图省事直接 `fs::*` 拼路径，否则有路径逃逸风险（破坏 I2/I3 安全不变量）。
- 这 5 个工具只是「实现」，对 LLM 的 schema 在 [`definitions/`](definitions.md)（`file_declarations()`）——改参数要两边同步。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-tools file_ops
```
