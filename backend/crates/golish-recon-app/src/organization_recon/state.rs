use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::types::OrganizationReconRunSnapshot;

#[derive(Debug, Clone, Default)]
pub struct OrganizationReconState {
    runs: Arc<RwLock<HashMap<String, OrganizationReconRunSnapshot>>>,
}

impl OrganizationReconState {
    pub(crate) async fn insert(&self, run: OrganizationReconRunSnapshot) {
        self.runs.write().await.insert(run.run_id.clone(), run);
    }

    pub async fn get(&self, run_id: &str) -> Option<OrganizationReconRunSnapshot> {
        self.runs.read().await.get(run_id).cloned()
    }

    pub(crate) async fn update(
        &self,
        run_id: &str,
        update: impl FnOnce(&mut OrganizationReconRunSnapshot),
    ) -> Option<OrganizationReconRunSnapshot> {
        let mut runs = self.runs.write().await;
        let run = runs.get_mut(run_id)?;
        update(run);
        Some(run.clone())
    }
}
