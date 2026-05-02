use std::sync::Arc;

use tokio::sync::RwLock;

/// MCP (Model Context Protocol) manager managed state.
///
/// Managed independently of `AppState` as of A4: MCP-related commands
/// take `State<'_, McpManaged>` directly instead of the monolithic
/// `AppState`.
pub struct McpManaged {
    pub manager: Arc<RwLock<Option<Arc<golish_mcp::McpManager>>>>,
}

impl McpManaged {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            manager: Arc::new(RwLock::new(None)),
        }
    }

    /// Build from an existing shared manager handle (used by `AppState::extract_mcp_managed`).
    pub fn from_shared(manager: Arc<RwLock<Option<Arc<golish_mcp::McpManager>>>>) -> Self {
        Self { manager }
    }
}
