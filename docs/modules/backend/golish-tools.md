# golish-tools

> **一句话职责**：AI agent 的工具执行系统——一个 workspace 受限的 `ToolRegistry`，把读写文件、目录检索、shell 执行、AST 代码搜索、联网搜索等能力按统一契约暴露给 LLM。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-tools/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 要新增 / 修改一个 agent 可调用的工具（tool）时
- agent 调用工具的返回格式（成功 / 失败判定）出问题时
- 文件操作越权、路径逃逸出 workspace 的安全问题时
- 需要知道"agent 到底有哪些工具可用 / 工具 schema 怎么生成给 LLM"时

## 职责

提供 `ToolRegistry`：按名字注册并执行工具，所有文件操作被沙箱限制在 workspace（或临时目录）内。它是 `vtcode_core::tools::ToolRegistry` 的**drop-in 替代**，接口必须保持兼容（见下「接口契约」）。同时通过 `build_function_declarations()` 把全部工具的 JSON Schema 导出给 LLM 做 function-calling。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `ToolRegistry` | 工具注册表，`HashMap<String, Arc<dyn Tool>>` + workspace 路径 |
| `ToolRegistry::new(workspace)` / `with_config(workspace, cfg)` | 创建注册表（异步） |
| `ToolRegistry::execute_tool(name, args) -> Result<Value>` | 按名执行工具 |
| `ToolRegistry::available_tools() -> Vec<String>` | 列出已注册工具名 |
| `ToolRegistryConfig` | 携带 `GolishSettings`，决定是否启用联网搜索工具 |
| `build_function_declarations() -> Vec<FunctionDeclaration>` | 导出全部工具 schema（当前 41 个）给 LLM |
| `FunctionDeclaration` | `{ name, description, parameters }` 的 LLM 工具声明格式 |
| `ToolError` | 工具层错误（如 `UnknownTool`） |
| `Tool`（re-export 自 golish-core） | 所有工具实现的 trait |

### 接口契约（改动前必读，破坏即端到端炸）

```rust,ignore
ToolRegistry::new(workspace).await          // 创建
registry.execute_tool(name, args).await     // -> Result<Value>
registry.available_tools()                  // -> Vec<String>
```

**成功/失败契约**（由 `golish-agent-runtime` 的 agentic_loop 依赖）：
- 成功 = 返回 JSON **没有** `error` 字段；shell 命令额外带 `"exit_code": 0`
- 失败 = 返回 JSON **有** `error` 字段；shell 命令带非零 `exit_code`

## 依赖

- **内部 crate**：`golish-core`（基础类型/`Tool` trait）、`golish-settings`（配置）、`golish-shell-exec`（`run_pty_cmd`）、`golish-web`（Tavily / Brave 搜索）
- **关键外部**：`rig-core`（`ToolDefinition`）、`ast-grep-core` / `ast-grep-language`（AST 搜索）、`ignore` / `glob` / `regex` / `walkdir`（目录检索）

## 被谁依赖

`golish`（主程序）、`golish-app-core`、`golish-agent-runtime`、`golish-sub-agents`、`golish-agent-kit`、`golish-agent-bridge`

## 子模块（各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `file_ops/` | 文件读写增删改 5 个工具 | [→](golish-tools/file_ops.md) |
| `directory_ops/` | `list_files` / `list_directory` / `grep_file` | [→](golish-tools/directory_ops.md) |
| `ast_grep/` | 基于 ast-grep 的结构化代码搜索与替换 | [→](golish-tools/ast_grep.md) |
| `definitions/` | 把工具 schema 汇总成 LLM function declarations | [→](golish-tools/definitions.md) |

## 关键文件（单文件模块，不单独成卡）

| 文件 | 作用 |
|---|---|
| `registry.rs` | `ToolRegistry` 本体：注册哪些工具、按名执行 |
| `path_policy.rs` | **安全核心**：`resolve_path_checked` 等，强制路径落在 workspace/temp 内，防逃逸 |
| `error.rs` | `ToolError` 定义 |
| `lib.rs` | crate 入口，文档化接口契约 |

## 注意事项 / 坑

- 联网搜索工具（Tavily / Brave）**条件注册**：只有配置了对应 API key（或显式开 `tools.web_search`）才会出现在 `available_tools()` 里，写测试别假设它们总在。
- 改 `execute_tool` 返回结构前，先确认不破坏「成功/失败契约」，否则 agentic loop 会误判工具成败。
- 新增工具要两处同步：① 在 `registry.rs` 注册实例；② 在 `definitions/` 加 schema，否则 LLM 看不到或调不通。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-tools
```

单测覆盖：注册表创建、各工具成功/失败返回格式、schema 数量与必填字段（见 `registry.rs` 与 `definitions/mod.rs` 的 `#[cfg(test)]`）。
