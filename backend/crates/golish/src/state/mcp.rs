use std::sync::Arc;

use tokio::sync::RwLock;

/// MCP (Model Context Protocol) manager managed state.
pub struct McpManaged {
    pub manager: Arc<RwLock<Option<Arc<golish_mcp::McpManager>>>>,
}

impl McpManaged {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(RwLock::new(None)),
        }
    }
}
