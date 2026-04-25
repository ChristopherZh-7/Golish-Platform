use serde::Serialize;

// =============================================================================
// UI Events - Emitted to frontend for real-time updates
// =============================================================================

/// Events emitted to the frontend for real-time sidecar updates.
///
/// These events notify the UI about changes to sessions, patches, and artifacts
/// so it can update without polling.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum SidecarEvent {
    // Session lifecycle events
    /// A new session has started
    SessionStarted { session_id: String },
    /// A session has ended
    SessionEnded { session_id: String },

    // L2: Patch events
    /// A new patch has been created (staged)
    PatchCreated {
        session_id: String,
        patch_id: u32,
        subject: String,
    },
    /// A patch has been applied (committed via git am)
    PatchApplied {
        session_id: String,
        patch_id: u32,
        commit_sha: String,
    },
    /// A patch has been discarded
    PatchDiscarded { session_id: String, patch_id: u32 },
    /// A patch's commit message has been updated
    PatchMessageUpdated {
        session_id: String,
        patch_id: u32,
        new_subject: String,
    },

    // L3: Artifact events
    /// A new artifact has been created (pending)
    ArtifactCreated {
        session_id: String,
        filename: String,
        target: String,
    },
    /// An artifact has been applied to its target file
    ArtifactApplied {
        session_id: String,
        filename: String,
        target: String,
    },
    /// An artifact has been discarded
    ArtifactDiscarded {
        session_id: String,
        filename: String,
    },

    // State events
    /// The session state.md has been updated
    StateUpdated {
        session_id: String,
        /// The synthesis backend used (e.g., "VertexAnthropic", "OpenAi", "Template")
        backend: String,
    },

    /// A session title has been generated
    TitleGenerated { session_id: String, title: String },
}
