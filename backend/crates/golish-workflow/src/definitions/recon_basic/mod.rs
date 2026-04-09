mod dns_lookup;
mod http_probe;
mod port_scan;
pub mod state;
mod tech_fingerprint;
mod tool_check;

pub use state::{AvailableTools, PortInfo, ReconStage, ReconState, TargetReconData};
pub use tool_check::check_recon_tools;

use std::sync::Arc;

use async_trait::async_trait;
use graph_flow::{Context, GraphBuilder, NextAction, Task, TaskResult};

use crate::models::{WorkflowDefinition, WorkflowLlmExecutor};

use self::dns_lookup::DnsLookupTask;
use self::http_probe::HttpProbeTask;
use self::port_scan::PortScanTask;
use self::state::STATE_KEY;
use self::tech_fingerprint::TechFingerprintTask;

pub struct ReconBasicWorkflow;

impl WorkflowDefinition for ReconBasicWorkflow {
    fn name(&self) -> &str {
        "recon_basic"
    }

    fn description(&self) -> &str {
        "Basic reconnaissance pipeline: DNS, HTTP, ports, tech fingerprinting"
    }

    fn build_graph(&self, _executor: Arc<dyn WorkflowLlmExecutor>) -> Arc<graph_flow::Graph> {
        let initialize = Arc::new(InitializeTask);
        let dns = Arc::new(DnsLookupTask);
        let http = Arc::new(HttpProbeTask);
        let ports = Arc::new(PortScanTask);
        let tech = Arc::new(TechFingerprintTask);
        let summarize = Arc::new(SummarizeTask);

        let graph = GraphBuilder::new("recon_basic")
            .add_task(initialize.clone())
            .add_task(dns.clone())
            .add_task(http.clone())
            .add_task(ports.clone())
            .add_task(tech.clone())
            .add_task(summarize.clone())
            .add_edge(initialize.id(), dns.id())
            .add_edge(dns.id(), http.id())
            .add_edge(http.id(), ports.id())
            .add_edge(ports.id(), tech.id())
            .add_edge(tech.id(), summarize.id())
            .build();

        Arc::new(graph)
    }

    fn init_state(&self, input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        #[derive(serde::Deserialize, Default)]
        struct Input {
            #[serde(default)]
            targets: Vec<String>,
            #[serde(default)]
            project_path: String,
            #[serde(default)]
            project_name: String,
            #[serde(default)]
            proxy_url: Option<String>,
        }

        let parsed: Input = if input.is_null() || input == serde_json::json!({}) {
            Input::default()
        } else {
            serde_json::from_value(input).unwrap_or_default()
        };

        let state = ReconState {
            targets: parsed.targets,
            project_path: parsed.project_path,
            project_name: parsed.project_name,
            proxy_url: parsed.proxy_url,
            ..Default::default()
        };

        Ok(serde_json::to_value(state)?)
    }

    fn start_task(&self) -> &str {
        "initialize"
    }

    fn state_key(&self) -> &str {
        STATE_KEY
    }

    fn task_count(&self) -> usize {
        6
    }
}

struct InitializeTask;

#[async_trait]
impl Task for InitializeTask {
    fn id(&self) -> &str {
        "initialize"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let state: Option<ReconState> = context.get(STATE_KEY).await;

        match state {
            Some(s) if s.targets.is_empty() => Ok(TaskResult::new(
                Some("Error: No targets provided".to_string()),
                NextAction::End,
            )),
            Some(mut s) => {
                // Populate tool availability during initialization
                s.tools = tool_check::get_available_tools();
                s.stage = ReconStage::ToolCheck;

                tracing::info!(
                    "[recon] Starting basic recon for {} targets: {:?}",
                    s.targets.len(),
                    s.targets
                );
                context.set(STATE_KEY, s.clone()).await;
                Ok(TaskResult::new(
                    Some(format!("Initialized with {} targets", s.targets.len())),
                    NextAction::Continue,
                ))
            }
            None => Ok(TaskResult::new(
                Some("Error: Workflow state not initialized".to_string()),
                NextAction::End,
            )),
        }
    }
}

struct SummarizeTask;

#[async_trait]
impl Task for SummarizeTask {
    fn id(&self) -> &str {
        "summarize"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let mut state: ReconState = context
            .get(STATE_KEY)
            .await
            .unwrap_or_default();

        state.stage = ReconStage::Completed;

        let mut lines = Vec::new();
        lines.push("## Recon Summary\n".to_string());

        let tool_list: Vec<&str> = [
            ("dig", state.tools.dig),
            ("curl", state.tools.curl),
            ("nmap", state.tools.nmap),
            ("subfinder", state.tools.subfinder),
            ("httpx", state.tools.httpx),
            ("whatweb", state.tools.whatweb),
            ("nc", state.tools.nc),
        ]
        .iter()
        .filter(|(_, avail)| *avail)
        .map(|(name, _)| *name)
        .collect();

        let missing: Vec<&str> = [
            ("dig", state.tools.dig),
            ("curl", state.tools.curl),
            ("nmap", state.tools.nmap),
            ("subfinder", state.tools.subfinder),
            ("httpx", state.tools.httpx),
            ("whatweb", state.tools.whatweb),
            ("nc", state.tools.nc),
        ]
        .iter()
        .filter(|(_, avail)| !*avail)
        .map(|(name, _)| *name)
        .collect();

        lines.push(format!("**Tools used**: {}", tool_list.join(", ")));
        if !missing.is_empty() {
            lines.push(format!("**Missing tools**: {} (install for deeper recon)", missing.join(", ")));
        }
        lines.push(String::new());

        for result in &state.results {
            lines.push(format!("### {}\n", result.value));

            if !result.ips.is_empty() {
                lines.push(format!("- **IPs**: {}", result.ips.join(", ")));
            }

            if let Some(status) = result.http_status {
                lines.push(format!("- **HTTP**: {} (server: {})", status, result.http_server));
                if !result.http_redirect.is_empty() {
                    lines.push(format!("- **Redirect**: {}", result.http_redirect));
                }
            }

            if !result.ports.is_empty() {
                lines.push(format!("- **Open ports** ({}):", result.ports.len()));
                for p in &result.ports {
                    let detail = if p.version.is_empty() {
                        p.service.clone()
                    } else {
                        format!("{} ({})", p.service, p.version)
                    };
                    lines.push(format!("  - {}/{} — {} [{}]", p.port, p.protocol, detail, p.state));
                }
            }

            if !result.technologies.is_empty() {
                lines.push(format!("- **Technologies**: {}", result.technologies.join(", ")));
            }

            lines.push(String::new());
        }

        if !state.errors.is_empty() {
            lines.push("### Warnings\n".to_string());
            for e in &state.errors {
                lines.push(format!("- {e}"));
            }
        }

        let summary = lines.join("\n");
        state.summary = Some(summary.clone());
        context.set(STATE_KEY, state).await;

        Ok(TaskResult::new(Some(summary), NextAction::End))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockExecutor;

    #[async_trait]
    impl WorkflowLlmExecutor for MockExecutor {
        async fn complete(
            &self,
            _system_prompt: &str,
            _user_prompt: &str,
            _context: HashMap<String, serde_json::Value>,
        ) -> anyhow::Result<String> {
            Ok("Mock response".to_string())
        }
    }

    #[test]
    fn test_workflow_definition() {
        let wf = ReconBasicWorkflow;
        assert_eq!(wf.name(), "recon_basic");
        assert_eq!(wf.start_task(), "initialize");
        assert_eq!(wf.state_key(), STATE_KEY);
        assert_eq!(wf.task_count(), 8);
    }

    #[test]
    fn test_init_state() {
        let wf = ReconBasicWorkflow;
        let input = serde_json::json!({
            "targets": ["example.com", "10.0.0.1"],
            "project_path": "/tmp/test",
            "project_name": "test-project"
        });

        let state_val = wf.init_state(input).unwrap();
        let state: ReconState = serde_json::from_value(state_val).unwrap();

        assert_eq!(state.targets.len(), 2);
        assert_eq!(state.targets[0], "example.com");
        assert_eq!(state.project_name, "test-project");
    }

    #[test]
    fn test_init_state_empty() {
        let wf = ReconBasicWorkflow;
        let state_val = wf.init_state(serde_json::json!({})).unwrap();
        let state: ReconState = serde_json::from_value(state_val).unwrap();

        assert!(state.targets.is_empty());
    }

    #[test]
    fn test_build_graph() {
        let executor = Arc::new(MockExecutor);
        let wf = ReconBasicWorkflow;
        let _graph = wf.build_graph(executor);
    }
}
