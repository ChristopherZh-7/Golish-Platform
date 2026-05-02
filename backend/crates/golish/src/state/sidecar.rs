use std::sync::Arc;

use crate::sidecar::{SidecarConfig, SidecarState};

/// Sidecar managed state (configuration + runtime).
///
/// Reserved for Tauri `manage<SidecarManaged>()` migration (P2-2).
#[allow(dead_code)]
pub struct SidecarManaged {
    pub config: SidecarConfig,
    pub state: Arc<SidecarState>,
}

#[allow(dead_code)]
impl SidecarManaged {
    pub fn new(config: SidecarConfig, state: Arc<SidecarState>) -> Self {
        Self { config, state }
    }
}
