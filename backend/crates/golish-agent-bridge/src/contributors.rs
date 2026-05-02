//! Default prompt contributor composition.
//!
//! Moved here from `golish-prompts::contributors` in A1: this helper
//! mixes contributors from three different crates (`golish-prompts`
//! provides the provider/skill/tavily contributors, `golish-sub-agents`
//! now owns the sub-agent contributor). The bridge layer is the natural
//! assembly point that already depends on both, so placing the combiner
//! here keeps `golish-prompts` free of its former back-edge into
//! `golish-sub-agents`.

use std::sync::Arc;

use tokio::sync::RwLock;

use golish_core::PromptContributor;
use golish_prompts::contributors::{
    ProviderBuiltinToolsContributor, SkillsPromptContributor, TavilyToolsContributor,
};
use golish_sub_agents::{SubAgentPromptContributor, SubAgentRegistry};

/// Create the default set of prompt contributors.
pub fn create_default_contributors(
    sub_agent_registry: Arc<RwLock<SubAgentRegistry>>,
) -> Vec<Arc<dyn PromptContributor>> {
    vec![
        Arc::new(SubAgentPromptContributor::new(sub_agent_registry)),
        Arc::new(ProviderBuiltinToolsContributor),
        Arc::new(TavilyToolsContributor),
        Arc::new(SkillsPromptContributor::new()),
    ]
}
