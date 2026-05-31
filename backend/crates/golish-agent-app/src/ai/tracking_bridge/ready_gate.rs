//! `CoreDbReadyGate`: newtype wrapping `golish_core::DbReadyGate` to implement
//! the agent-kit `DbReadinessGate` trait. Moved verbatim from
//! `tracking_bridge.rs`; re-exported by `mod.rs`.

use async_trait::async_trait;

#[derive(Clone)]
pub struct CoreDbReadyGate(pub golish_core::DbReadyGate);

#[async_trait]
impl golish_agent_kit::db_traits::DbReadinessGate for CoreDbReadyGate {
    fn is_ready(&self) -> bool {
        self.0.is_ready()
    }
    fn is_failed(&self) -> bool {
        self.0.is_failed()
    }
    async fn wait(&mut self) -> bool {
        self.0.wait().await
    }
    fn clone_box(&self) -> Box<dyn golish_agent_kit::db_traits::DbReadinessGate> {
        Box::new(self.clone())
    }
}
