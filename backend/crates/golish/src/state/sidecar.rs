use std::sync::Arc;

use crate::sidecar::{SidecarConfig, SidecarState};

/// Sidecar managed state (configuration + runtime).
pub struct SidecarManaged {
    pub config: SidecarConfig,
    pub state: Arc<SidecarState>,
}

impl SidecarManaged {
    pub fn new(config: SidecarConfig, state: Arc<SidecarState>) -> Self {
        Self { config, state }
    }
}
