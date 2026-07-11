# golish / mcp

> **一句话职责**：MCP（Model Context Protocol）Tauri 命令——管理 MCP server 连接：列服务/状态、手动 connect/disconnect、列工具、配置增删、项目配置信任。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/mcp/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 MCP server 连接管理 Tauri 命令、配置信任流程时

## 职责

`golish-mcp`（客户端集成）的 Tauri 命令面：服务列举/状态、connect/disconnect、列工具、配置读取、项目配置信任与 canonical builtin setup。列举来源遵循 executable merge 的真实优先级（trusted project > user > builtin）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `mcp_list_servers` / `mcp_connect` / `mcp_disconnect` / `mcp_list_tools` | 服务/工具管理 |
| `mcp_get_config` / `mcp_setup_builtin` | 读取 executable 配置 / 仅在 canonical builtin 目录执行 setup |
| `mcp_has_project_config` / `mcp_is_project_trusted` / `mcp_trust_project_config` | 项目配置信任 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 命令 re-export |
| `commands.rs` | MCP Tauri 命令 |

## 依赖

- `golish-mcp`（client/loader/oauth）、`tauri`、`state::McpManaged`

## 注意事项 / 坑

- 项目 MCP 配置信任是安全边界（防恶意配置）；命令面消费 loader 已经过滤的 executable config，不能自行读取未信任 project config 并交给 manager。
- `mcp_list_servers` 的 source 必须按 trusted project > user > builtin 判定，不能因同名 override 把项目 server 误标成 builtin。
- `mcp_setup_builtin` 只能调用 `golish_mcp::builtin_setup_directory`，不能从 merged config 的 args 反推目录，否则同名 override 可重定向 npm 执行位置。

## 测试入口

```bash
cd backend && cargo nextest run -p golish mcp
```
