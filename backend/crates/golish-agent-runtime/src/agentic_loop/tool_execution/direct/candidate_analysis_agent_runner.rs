//! Submit-only typed Candidate runner.
//!
//! The adapter deliberately accepts no general tool registry.  Its backend
//! gets one data-only JSON input and must return the value submitted through
//! the sole `submit_result` tool.

use async_trait::async_trait;
use golish_agent_kit::task_orchestrator::hypothesis_analysis::*;
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateSubmitOnlyResult {
    pub provider_attempt_id: Uuid,
    pub submit_result: Value,
}

#[async_trait]
pub trait CandidateSubmitOnlyExecutor: Send + Sync {
    async fn execute_submit_only(
        &self,
        binding: &CandidateAnalysisAgentBinding,
        input_schema: &'static str,
        input: Value,
        output_schema: &'static str,
    ) -> anyhow::Result<CandidateSubmitOnlyResult>;
}

pub struct DirectCandidateAnalysisAgentRunner<E> {
    executor: E,
}

impl<E> DirectCandidateAnalysisAgentRunner<E> {
    pub const fn new(executor: E) -> Self {
        Self { executor }
    }
}

impl<E> DirectCandidateAnalysisAgentRunner<E>
where
    E: CandidateSubmitOnlyExecutor,
{
    async fn execute<I, O>(
        &self,
        binding: CandidateAnalysisAgentBinding,
        expected_role: CandidateAnalysisAgentRole,
        input_schema: &'static str,
        input: I,
        output_schema: &'static str,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<O>>
    where
        I: serde::Serialize,
        O: serde::de::DeserializeOwned,
    {
        binding.validate_tool_free()?;
        anyhow::ensure!(
            binding.role == expected_role,
            "candidate typed runner role mismatch"
        );
        let result = self
            .executor
            .execute_submit_only(
                &binding,
                input_schema,
                serde_json::to_value(input)?,
                output_schema,
            )
            .await?;
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: result.provider_attempt_id,
            output: serde_json::from_value(result.submit_result)?,
        })
    }
}

#[async_trait]
impl<E> HypothesisAnalysisAgentRunner for DirectCandidateAnalysisAgentRunner<E>
where
    E: CandidateSubmitOnlyExecutor,
{
    async fn run_controller_dispatch(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerDispatchInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDispatchPlan>> {
        self.execute(
            binding,
            CandidateAnalysisAgentRole::Controller,
            "candidate_controller_dispatch_input.v1",
            input,
            "candidate_controller_dispatch_plan.v1",
        )
        .await
    }

    async fn run_analyst(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateAnalystInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>> {
        self.execute(
            binding,
            CandidateAnalysisAgentRole::Analyst,
            "candidate_analyst_input.v1",
            input,
            "hypothesis_proposal_artifact.v1",
        )
        .await
    }

    async fn run_critic(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateCriticInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>> {
        self.execute(
            binding,
            CandidateAnalysisAgentRole::Critic,
            "candidate_critic_input.v1",
            input,
            "hypothesis_critic_artifact.v1",
        )
        .await
    }

    async fn run_controller_final(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerFinalInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>> {
        self.execute(
            binding,
            CandidateAnalysisAgentRole::Controller,
            "candidate_controller_final_input.v1",
            input,
            "candidate_controller_decision_artifact.v1",
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    struct FakeSubmitOnlyExecutor {
        calls: Arc<AtomicUsize>,
        output: Value,
    }

    #[async_trait]
    impl CandidateSubmitOnlyExecutor for FakeSubmitOnlyExecutor {
        async fn execute_submit_only(
            &self,
            _binding: &CandidateAnalysisAgentBinding,
            _input_schema: &'static str,
            _input: Value,
            _output_schema: &'static str,
        ) -> anyhow::Result<CandidateSubmitOnlyResult> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CandidateSubmitOnlyResult {
                provider_attempt_id: Uuid::new_v4(),
                submit_result: self.output.clone(),
            })
        }
    }

    fn binding(allowed_tools: Vec<String>) -> CandidateAnalysisAgentBinding {
        CandidateAnalysisAgentBinding {
            analysis_attempt_id: Uuid::new_v4(),
            analysis_attempt_ordinal: 0,
            work_item_id: Uuid::new_v4(),
            worker_run_id: Uuid::new_v4(),
            role: CandidateAnalysisAgentRole::Controller,
            lane_ordinal: 0,
            read_only: true,
            allowed_tools,
        }
    }

    fn dispatch_input() -> CandidateControllerDispatchInput {
        CandidateControllerDispatchInput {
            snapshot_id: Uuid::new_v4(),
            snapshot_authority_hash: format!("sha256:{}", "a".repeat(64)),
            input_count: 1,
            input_chunk_census_set_hash: format!("sha256:{}", "b".repeat(64)),
            relationship_cross_index_hash: format!("sha256:{}", "c".repeat(64)),
        }
    }

    #[tokio::test]
    async fn candidate_submit_only_runner_rejects_general_tool_access_before_provider_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = DirectCandidateAnalysisAgentRunner::new(FakeSubmitOnlyExecutor {
            calls: calls.clone(),
            output: serde_json::json!({}),
        });
        let result = runner
            .run_controller_dispatch(
                binding(vec!["submit_result".into(), "browser".into()]),
                dispatch_input(),
            )
            .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn candidate_submit_only_runner_rejects_unknown_output_authority_fields() {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = DirectCandidateAnalysisAgentRunner::new(FakeSubmitOnlyExecutor {
            calls: calls.clone(),
            output: serde_json::json!({
                "requested_live_lanes": 1,
                "requested_inputs_per_microbatch": 1,
                "objective_clusters": [],
                "caller_semantic_root": "forbidden"
            }),
        });
        let result = runner
            .run_controller_dispatch(binding(vec!["submit_result".into()]), dispatch_input())
            .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
