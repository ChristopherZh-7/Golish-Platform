/**
 * MCP API — re-exports through the unified client.
 */
export {
  connect,
  disconnect,
  getConfig,
  hasProjectConfig,
  isProjectTrusted,
  listServers,
  listTools,
  type McpEvent,
  type McpServerConfig,
  type McpServerInfo,
  type McpServerStatus,
  type McpToolInfo,
  trustProjectConfig,
} from "../mcp";
