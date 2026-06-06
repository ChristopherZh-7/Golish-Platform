# golish-tools / ast_grep

> **一句话职责**：基于 ast-grep 的**结构化**代码搜索与替换——理解语法树而非纯文本，用元变量 `$VAR` / `$$$VAR` 匹配代码模式。

- **类型**：目录模块（属于 crate [`golish-tools`](../golish-tools.md)）
- **路径**：`backend/crates/golish-tools/src/ast_grep/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `ast_grep` / `ast_grep_replace` 工具，或调整支持的语言/模式语法时
- 需要按语法结构（函数定义、调用、if 等）搜/改代码而非正则时

## 职责

提供两个 `Tool` + 一组纯函数。结构化搜索：把模式编译成 AST 在源码树里找匹配（用 `WalkDir` 遍历）。支持语言：rust / typescript / javascript / python / go / java / c / cpp（按扩展名自动识别或显式指定）。无效模式返回友好 `error`（内部 `catch_unwind` 兜住 ast-grep panic）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `AstGrepTool`（`ast_grep`） | 搜索。`pattern`(必填) + 可选 `path` / `language`(enum 8 语言)；返回匹配位置与文本 |
| `AstGrepReplaceTool`（`ast_grep_replace`） | 结构化替换 |
| `search(workspace, pattern, path, language) -> SearchResult` | 搜索纯函数 |
| `replace` / `replace_source` | 替换纯函数 |
| `detect_language` / `parse_language` | 语言识别 |
| `SearchResult` / `SearchMatch` / `ReplaceResult` / `Replacement` | 结果类型 |

模式示例：`fn $NAME($$$ARGS) { $$$BODY }`（Rust 函数）、`console.log($MSG)`（JS 日志）。

## 关键文件

| 文件 | 作用 |
|---|---|
| `tool.rs` | 两个 `Tool` 实现 |
| `language.rs` | 语言识别/解析（`SupportLang`） |
| `result.rs` | 结果数据结构 |
| `replace_ops.rs` | 替换逻辑 |
| `mod.rs` | `search()` / `search_source()` 等遍历与搜索实现 |

## 依赖

- `golish_core::{Tool, utils::*}`、`ast-grep-core` / `ast-grep-language`、`walkdir`

## 注意事项 / 坑

- 模式**必须是完整语法结构**，否则 ast-grep 会报错；错误信息已引导用 `fn $NAME($$$ARGS)` 这类写法。
- 不读 `.gitignore`（用 `WalkDir` 全量遍历），与 directory_ops 的 gitignore-aware 不同。
- schema 在 [`definitions/`](definitions.md) 的 `ast_declarations()`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-tools ast_grep
```
