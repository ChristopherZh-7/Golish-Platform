//! Error / skip-report builders.

use golish_evals::metrics::MetricResult;
use golish_evals::outcome::EvalReport;
use golish_evals::scenarios::Scenario;

use super::SWEBenchScenario;

impl SWEBenchScenario {
    /// Create an error report when something goes wrong.
    pub(super) fn create_error_report(
        &self,
        agent_output: &golish_evals::runner::AgentOutput,
        duration_ms: u64,
        error_message: &str,
    ) -> EvalReport {
        let mut report = EvalReport::new(self.name(), agent_output.clone(), duration_ms);

        report.add_metric(
            "swebench-tests",
            MetricResult::Fail {
                reason: error_message.to_string(),
            },
        );

        report
    }

    /// Create a skip report when an instance can't be evaluated.
    pub(super) fn create_skip_report(
        &self,
        agent_output: &golish_evals::runner::AgentOutput,
        duration_ms: u64,
        reason: &str,
    ) -> EvalReport {
        let mut report = EvalReport::new(self.name(), agent_output.clone(), duration_ms);

        report.add_metric(
            "swebench-tests",
            MetricResult::Skip {
                reason: reason.to_string(),
            },
        );

        report
    }
}
