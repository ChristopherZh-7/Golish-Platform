# golish / mcp

> **一句话职责**：MCP（Model Context Protocol）Tauri 命令——管理 MCP server 连接：列服务/状态、手动 connect/disconnect、列工具、配置增删、项目配置信任。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/mcp/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 MCP server 连接管理 Tauri 命令、配置信任流程时

## 职责

`golish-mcp`（客户端集成）的 Tauri 命令面：服务列举/状态、connect/disconnect、列工具、配置 get/add/remove、项目配置信任（`mcp_trust_project_config`）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `mcp_list_servers` / `mcp_connect` / `mcp_disconnect` / `mcp_list_tools` | 服务/工具管理 |
| `mcp_get_config` / `mcp_setup_builtin` | 配置 |
| `mcp_has_project_config` / `mcp_is_project_trusted` / `mcp_trust_project_config` | 项目配置信任 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 命令 re-export |
| `commands.rs` | MCP Tauri 命令 |

## 依赖

- `golish-mcp`（client/loader/oauth）、`tauri`、`state::McpManaged`

## 注意事项 / 坑

- 项目 MCP 配置信任是安全边界（防恶意配置）；信任流程对应 `golish-mcp::loader` 的 `TrustedMcpConfigs`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish mcp
```
