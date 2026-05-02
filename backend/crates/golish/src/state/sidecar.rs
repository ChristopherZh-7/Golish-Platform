use std::sync::Arc;

use crate::sidecar::{SidecarConfig, SidecarState};

/// Sidecar managed state (configuration + runtime).
///
/// Managed independently of `AppState` as of A4: sidecar commands take
/// `State<'_, SidecarManaged>` directly. Field names deliberately mirror
/// `AppState` so commands only need to rename the `State<>` type.
pub struct SidecarManaged {
    pub sidecar_config: SidecarConfig,
    pub sidecar_state: Arc<SidecarState>,
}

impl SidecarManaged {
    #[allow(dead_code)]
    pub fn new(config: SidecarConfig, state: Arc<SidecarState>) -> Self {
        Self {
            sidecar_config: config,
            sidecar_state: state,
        }
    }

    /// Build from AppState-owned `Arc`s so the two views share runtime state.
    pub fn from_shared(config: SidecarConfig, state: Arc<SidecarState>) -> Self {
        Self {
            sidecar_config: config,
            sidecar_state: state,
        }
    }
}
