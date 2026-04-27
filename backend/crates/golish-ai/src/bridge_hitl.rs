//! HITL (Human-in-the-Loop) extension for AgentBridge.
//!
//! This module contains methods for managing tool approval patterns and decisions.

use anyhow::Result;

use super::agent_bridge::AgentBridge;
use golish_core::hitl::{ApprovalDecision, ApprovalPattern, ToolApprovalConfig};

impl AgentBridge {
    // ========================================================================
    // HITL (Human-in-the-Loop) Methods
    // ========================================================================

    pub async fn get_approval_patterns(&self) -> Vec<ApprovalPattern> {
        self.access.approval_recorder.get_all_patterns().await
    }

    pub async fn get_tool_approval_pattern(&self, tool_name: &str) -> Option<ApprovalPattern> {
        self.access.approval_recorder.get_pattern(tool_name).await
    }

    pub async fn get_hitl_config(&self) -> ToolApprovalConfig {
        self.access.approval_recorder.get_config().await
    }

    pub async fn set_hitl_config(&self, config: ToolApprovalConfig) -> Result<()> {
        self.access.approval_recorder.set_config(config).await
    }

    pub async fn add_tool_always_allow(&self, tool_name: &str) -> Result<()> {
        self.access.approval_recorder.add_always_allow(tool_name).await
    }

    pub async fn remove_tool_always_allow(&self, tool_name: &str) -> Result<()> {
        self.access.approval_recorder.remove_always_allow(tool_name).await
    }

    pub async fn reset_approval_patterns(&self) -> Result<()> {
        self.access.approval_recorder.reset_patterns().await
    }

    pub async fn respond_to_approval(&self, decision: ApprovalDecision) -> Result<()> {
        if let Some(ref coordinator) = self.events.coordinator {
            coordinator.resolve_approval(decision.clone());
        } else {
            let sender = {
                let mut pending = self.access.pending_approvals.write().await;
                pending.remove(&decision.request_id)
            };

            if let Some(sender) = sender {
                let _ = sender.send(decision.clone());
            } else {
                tracing::warn!(
                    "No pending approval found for request_id: {}",
                    decision.request_id
                );
            }
        }

        if let Err(e) = self
            .access
            .approval_recorder
            .record_approval(
                decision
                    .request_id
                    .split('_')
                    .next_back()
                    .unwrap_or("unknown"),
                decision.approved,
                decision.reason.clone(),
                decision.always_allow,
            )
            .await
        {
            tracing::warn!("Failed to record approval pattern: {}", e);
        }

        Ok(())
    }
}
