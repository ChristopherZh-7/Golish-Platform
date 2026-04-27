//! Tool policy extension for AgentBridge.
//!
//! This module contains methods for managing tool policies (allow/prompt/deny rules).

use anyhow::Result;

use super::agent_bridge::AgentBridge;
use crate::loop_detection::{LoopDetectorStats, LoopProtectionConfig};
use crate::tool_policy::{ToolPolicy, ToolPolicyConfig};

impl AgentBridge {
    // ========================================================================
    // Tool Policy Methods
    // ========================================================================

    pub async fn get_tool_policy_config(&self) -> ToolPolicyConfig {
        self.access.tool_policy_manager.get_config().await
    }

    pub async fn set_tool_policy_config(&self, config: ToolPolicyConfig) -> Result<()> {
        self.access.tool_policy_manager.set_config(config).await
    }

    pub async fn get_tool_policy(&self, tool_name: &str) -> ToolPolicy {
        self.access.tool_policy_manager.get_policy(tool_name).await
    }

    pub async fn set_tool_policy(&self, tool_name: &str, policy: ToolPolicy) -> Result<()> {
        self.access.tool_policy_manager.set_policy(tool_name, policy).await
    }

    pub async fn reset_tool_policies(&self) -> Result<()> {
        self.access.tool_policy_manager.reset_to_defaults().await
    }

    pub async fn enable_full_auto_mode(&self, allowed_tools: Vec<String>) {
        self.access.tool_policy_manager
            .enable_full_auto(allowed_tools)
            .await;
    }

    pub async fn disable_full_auto_mode(&self) {
        self.access.tool_policy_manager.disable_full_auto().await;
    }

    pub async fn is_full_auto_mode_enabled(&self) -> bool {
        self.access.tool_policy_manager.is_full_auto_enabled().await
    }

    // ========================================================================
    // Loop Protection Methods
    // ========================================================================

    pub async fn get_loop_protection_config(&self) -> LoopProtectionConfig {
        self.access.loop_detector.read().await.config().clone()
    }

    pub async fn set_loop_protection_config(&self, config: LoopProtectionConfig) {
        self.access.loop_detector.write().await.set_config(config);
    }

    pub async fn get_loop_detector_stats(&self) -> LoopDetectorStats {
        self.access.loop_detector.read().await.stats()
    }

    pub async fn is_loop_detection_enabled(&self) -> bool {
        self.access.loop_detector.read().await.is_enabled()
    }

    pub async fn disable_loop_detection_for_session(&self) {
        self.access.loop_detector.write().await.disable_for_session();
    }

    pub async fn enable_loop_detection(&self) {
        self.access.loop_detector.write().await.enable();
    }

    pub async fn reset_loop_detector(&self) {
        self.access.loop_detector.write().await.reset();
    }
}
