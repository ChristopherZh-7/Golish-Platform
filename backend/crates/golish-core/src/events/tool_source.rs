//! [`ToolSource`] — origin of a tool call (main agent / sub-agent / workflow).

use serde::{Deserialize, Serialize};

/// Source of a tool call - indicates where the tool request originated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolSource {
    /// Tool called by the main agent
    #[default]
    Main,
    /// Tool called by a sub-agent
    SubAgent {
        agent_id: String,
        agent_name: String,
    },
    /// Tool called by a workflow
    Workflow {
        workflow_id: String,
        workflow_name: String,
        /// Current step name (if within a step)
        #[serde(skip_serializing_if = "Option::is_none")]
        step_name: Option<String>,
        /// Current step index (0-based)
        #[serde(skip_serializing_if = "Option::is_none")]
        step_index: Option<usize>,
    },
}
