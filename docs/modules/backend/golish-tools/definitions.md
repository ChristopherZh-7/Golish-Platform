# golish-tools / definitions

> **一句话职责**：把**所有** agent 工具的 JSON Schema 汇总成 LLM function declarations——`build_function_declarations()` 是 LLM「能看到哪些工具」的唯一出口。

- **类型**：目录模块（属于 crate [`golish-tools`](../golish-tools.md)）
- **路径**：`backend/crates/golish-tools/src/definitions/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 新增/改一个 agent 工具，需要让 LLM「看到」它时
- LLM 调工具报参数错、或某工具 LLM 根本不调（schema 没注册）时
- 想知道 agent 一共暴露了哪些工具时

## 职责

聚合全部工具声明。`build_function_declarations()` 把各分组的 schema 拼成一个 `Vec<FunctionDeclaration>`（当前 **43** 个），交给 LLM 做 function-calling。注意：这里声明的工具**不止 golish-tools 自己实现的**——还包括 memory / knowledge_base / graph / security / sploitus 等由别处实现、但在此统一声明的工具。

## 公开接口

| 符号 | 说明 |
|---|---|
| `build_function_declarations() -> Vec<FunctionDeclaration>` | 汇总全部工具 schema（43 个） |
| `FunctionDeclaration` | `{ name, description, parameters(JSON Schema) }` |

## 关键文件（按工具分组）

| 文件 | 声明的工具组 |
|---|---|
| `file_ops.rs` | `file_declarations()` + `directory_declarations()`（read/write/create/edit/delete/list_*/grep_file） |
| `core_tools.rs` | `plan_declarations()` / `shell_declarations()`（run_pty_cmd）/ `ast_declarations()` |
| `memory_tools.rs` | memory / code_store / guide_store |
| `knowledge_base.rs` | search/write/read knowledge、CVE/PoC |
| `security_tools.rs` | security analysis（log_operation/discover_apis/fingerprint_target…） |
| `graph_tools.rs` | graph_add_entity/relation/search/neighbors/attack_paths |
| `sploitus_tools.rs` | search_exploits |

## 依赖

- `serde` / `serde_json`（schema 序列化）

## 注意事项 / 坑

- **新增工具要两处同步**：① 在 [`registry.rs`](../golish-tools.md) 注册实例（让它能执行）；② 在本模块对应分组加 schema（让 LLM 能看到）。漏任一处 = 工具存在但 LLM 调不到，或 LLM 调了但执行报 UnknownTool。
- 单测 `test_build_function_declarations_returns_all_tools` 硬断言数量 `== 43`，加/减工具要同步改这个断言，否则测试红。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-tools definitions
```
