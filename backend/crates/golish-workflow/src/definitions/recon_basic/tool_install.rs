use async_trait::async_trait;
use graph_flow::{Context, NextAction, Task, TaskResult};

use super::state::{ReconState, STATE_KEY};

pub struct ToolInstallTask;

#[async_trait]
impl Task for ToolInstallTask {
    fn id(&self) -> &str {
        "tool_install"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let state: ReconState = context.get(STATE_KEY).await.unwrap_or_default();

        let config = golish_pentest::PentestConfig::default();
        let toolsconfig_dir = config.toolsconfig_dir();
        let tools_dir = config.tools_dir();

        let scan = golish_pentest::scan_toolsconfig_with_status(toolsconfig_dir, tools_dir);

        let missing: Vec<_> = scan
            .tools
            .iter()
            .filter(|t| !t.installed && t.install.is_some())
            .collect();

        if missing.is_empty() {
            tracing::info!("[recon] All configured tools already installed");
            context.set(STATE_KEY, state).await;
            return Ok(TaskResult::new(
                Some("All tools ready".into()),
                NextAction::Continue,
            ));
        }

        let missing_names: Vec<String> = missing.iter().map(|t| t.id.clone()).collect();
        tracing::warn!(
            "[recon] Missing tools: {}. User should install via Tool Manager.",
            missing_names.join(", ")
        );

        context.set(STATE_KEY, state).await;
        Ok(TaskResult::new(
            Some(format!(
                "Missing tools: {}. Install them in Tool Manager (left sidebar) then re-run.",
                missing_names.join(", ")
            )),
            NextAction::Continue,
        ))
    }
}
