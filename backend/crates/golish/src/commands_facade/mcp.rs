//! MCP (Model Context Protocol) server and tool commands.
//!
//! Expected commands exposed here (documentation only; actual set is
//! whatever `crate::mcp` re-exports at its root):
//! - `mcp_list_servers`, `mcp_list_tools`, `mcp_get_config`
//! - `mcp_is_project_trusted`, `mcp_trust_project_config`,
//!   `mcp_has_project_config`
//! - `mcp_connect`, `mcp_disconnect`

pub use crate::mcp::*;
