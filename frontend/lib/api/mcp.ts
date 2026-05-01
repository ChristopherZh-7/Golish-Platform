/**
 * MCP API — re-exports through the unified client.
 */
export {
  type McpServerStatus,
  type McpServerInfo,
  type McpToolInfo,
  type McpServerConfig,
  type McpEvent,
  listServers,
  connect,
  disconnect,
  listTools,
  getConfig,
  hasProjectConfig,
  isProjectTrusted,
  trustProjectConfig,
} from "../mcp";
