# golish-mcp

> **一句话职责**：MCP（Model Context Protocol）**客户端**集成——加载 MCP 配置 + 信任处理、经 rmcp 管理 client/transport、把外部 MCP 工具转成 Golish 工具定义。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-mcp/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 接入/管理外部 MCP server、MCP 配置与信任、OAuth、SSE transport、MCP 工具转换时
- agent 调用 MCP 工具相关时

## 职责

作为 MCP **客户端**消费外部 MCP server 提供的工具：加载配置、处理项目级配置信任、管理连接与 transport、把 MCP 工具结果转成 Golish 工具结果格式。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `McpManager` / `ServerStatus` | 多 server 管理 |
| `McpClientConnection` / `McpClientHandler` | 客户端连接 |
| `McpConfigFile` / `McpServerConfig` / `McpTransportType` | 配置 |
| `load_mcp_config` / `trust_project_config` / `builtin_server_names` | 加载与信任 |
| `McpTool` / `convert_mcp_result_to_tool_result` / `parse_mcp_tool_name` | 工具转换 |

## 依赖

- **内部**：`golish-platform`；**外部**：`rmcp`、`rig-core`

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `loader/` | MCP 配置加载与信任 | [→](golish-mcp/loader.md) |
| `oauth/` | MCP OAuth 流程 | [→](golish-mcp/oauth.md) |

## 关键文件

`client.rs`、`config.rs`、`manager.rs`、`sse_transport.rs`、`tools.rs`。

## 注意事项 / 坑

- 这是 Golish 当 **MCP 客户端**（消费外部 server）；和 `golish-pentest-mcp`（Golish 自己当 **MCP server** 暴露 pentest 工具）方向相反，别混。
- 项目级 MCP 配置有**信任门禁**（`trust_project_config`），别绕过直接加载不受信配置。
- 相关：`docs/mcp.md`、`docs/mcp-implementation-plan.md`、`docs/superpowers/plans/2026-05-07-builtin-mcp-auto-init.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-mcp
```
