use std::sync::Arc;

use tokio::sync::RwLock;

/// MCP (Model Context Protocol) manager managed state.
///
/// Reserved for Tauri `manage<McpManaged>()` migration (refactor-roadmap P2-2);
/// not yet wired into commands.
#[allow(dead_code)]
pub struct McpManaged {
    pub manager: Arc<RwLock<Option<Arc<golish_mcp::McpManager>>>>,
}

#[allow(dead_code)]
impl McpManaged {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(RwLock::new(None)),
        }
    }
}
