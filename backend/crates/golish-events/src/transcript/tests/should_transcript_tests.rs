use super::*;
use golish_core::events::AiEvent;

/// Helper that constructs a sample instance for each AiEvent variant.
/// Uses a match statement so the compiler will force an update when new variants are added.
fn all_variants() -> Vec<(AiEvent, bool)> {
    // Each entry: (event, expected should_transcript result)
    // The match ensures exhaustiveness — adding a new AiEvent variant will cause a compile error here.
    let variants: Vec<(AiEvent, bool)> = vec![
        (
            AiEvent::Started {
                turn_id: "t".into(),
            },
            true,
        ),
        (
            AiEvent::UserMessage {
                content: "hi".into(),
            },
            true,
        ),
        (AiEvent::SystemHooksInjected { hooks: vec![] }, true),
        (
            AiEvent::TextDelta {
                delta: "x".into(),
                accumulated: "x".into(),
            },
            false,
        ),
        (
            AiEvent::ToolRequest {
                tool_name: "t".into(),
                args: serde_json::json!({}),
                request_id: "r".into(),
                source: Default::default(),
            },
            true,
        ),
        (
            AiEvent::ToolIntentObservation {
                request_id: "r".into(),
                tool_name: "t".into(),
                source: "textual_xml".into(),
                decision: "require_human_answer".into(),
                reason: Some("needs user".into()),
                raw_preview: None,
            },
            true,
        ),
        (
            AiEvent::ToolApprovalRequest {
                request_id: "r".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
                stats: None,
                risk_level: golish_core::hitl::RiskLevel::Low,
                can_learn: false,
                suggestion: None,
                source: Default::default(),
            },
            true,
        ),
        (
            AiEvent::ToolAutoApproved {
                request_id: "r".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
                reason: "ok".into(),
                source: Default::default(),
            },
            true,
        ),
        (
            AiEvent::ToolDenied {
                request_id: "r".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
                reason: "no".into(),
                source: Default::default(),
            },
            true,
        ),
        (
            AiEvent::ToolResult {
                tool_name: "t".into(),
                result: serde_json::json!(null),
                success: true,
                request_id: "r".into(),
                source: Default::default(),
            },
            true,
        ),
        (
            AiEvent::ToolOutputChunk {
                request_id: "r".into(),
                tool_name: "t".into(),
                chunk: "out".into(),
                stream: "stdout".into(),
                source: Default::default(),
            },
            false,
        ),
        (
            AiEvent::Reasoning {
                content: "think".into(),
            },
            false,
        ),
        (
            AiEvent::Completed {
                response: "done".into(),
                reasoning: None,
                input_tokens: None,
                output_tokens: None,
                duration_ms: None,
            },
            true,
        ),
        (
            AiEvent::Error {
                message: "err".into(),
                error_type: "e".into(),
            },
            true,
        ),
        (
            AiEvent::SubAgentStarted {
                agent_id: "a".into(),
                agent_name: "n".into(),
                task: "t".into(),
                depth: 0,
                parent_request_id: "p".into(),
            },
            true,
        ),
        (
            AiEvent::SubAgentToolRequest {
                agent_id: "a".into(),
                tool_name: "t".into(),
                args: serde_json::json!({}),
                request_id: "r".into(),
                parent_request_id: "p".into(),
            },
            false,
        ),
        (
            AiEvent::SubAgentToolResult {
                agent_id: "a".into(),
                tool_name: "t".into(),
                success: true,
                result: serde_json::json!(null),
                request_id: "r".into(),
                parent_request_id: "p".into(),
            },
            false,
        ),
        (
            AiEvent::SubAgentCompleted {
                agent_id: "a".into(),
                response: "ok".into(),
                duration_ms: 0,
                parent_request_id: "p".into(),
            },
            true,
        ),
        (
            AiEvent::SubAgentReasoning {
                agent_id: "a".into(),
                delta: "think".into(),
                accumulated: "think".into(),
                parent_request_id: "p".into(),
            },
            false,
        ),
        (
            AiEvent::SubAgentError {
                agent_id: "a".into(),
                error: "err".into(),
                parent_request_id: "p".into(),
            },
            true,
        ),
        (
            AiEvent::ContextWarning {
                utilization: 0.8,
                total_tokens: 800,
                max_tokens: 1000,
            },
            true,
        ),
        (
            AiEvent::ToolResponseTruncated {
                tool_name: "t".into(),
                original_tokens: 100,
                truncated_tokens: 50,
            },
            true,
        ),
        (
            AiEvent::Warning {
                message: "warn".into(),
            },
            true,
        ),
        (
            AiEvent::CompactionStarted {
                tokens_before: 100,
                messages_before: 5,
            },
            true,
        ),
        (
            AiEvent::CompactionCompleted {
                tokens_before: 100,
                messages_before: 5,
                messages_after: 2,
                summary_length: 50,
                summary: None,
                summarizer_input: None,
            },
            true,
        ),
        (
            AiEvent::CompactionFailed {
                tokens_before: 100,
                messages_before: 5,
                error: "err".into(),
                summarizer_input: None,
            },
            true,
        ),
        (
            AiEvent::LoopWarning {
                tool_name: "t".into(),
                current_count: 5,
                max_count: 10,
                message: "w".into(),
            },
            true,
        ),
        (
            AiEvent::LoopBlocked {
                tool_name: "t".into(),
                repeat_count: 10,
                max_count: 10,
                message: "b".into(),
            },
            true,
        ),
        (
            AiEvent::MaxIterationsReached {
                iterations: 50,
                max_iterations: 50,
                message: "m".into(),
            },
            true,
        ),
        (
            AiEvent::WorkflowStarted {
                workflow_id: "w".into(),
                workflow_name: "n".into(),
                session_id: "s".into(),
            },
            true,
        ),
        (
            AiEvent::WorkflowStepStarted {
                workflow_id: "w".into(),
                step_name: "s".into(),
                step_index: 0,
                total_steps: 1,
            },
            true,
        ),
        (
            AiEvent::WorkflowStepCompleted {
                workflow_id: "w".into(),
                step_name: "s".into(),
                output: None,
                duration_ms: 0,
            },
            true,
        ),
        (
            AiEvent::WorkflowCompleted {
                workflow_id: "w".into(),
                final_output: "ok".into(),
                total_duration_ms: 0,
            },
            true,
        ),
        (
            AiEvent::WorkflowError {
                workflow_id: "w".into(),
                step_name: None,
                error: "err".into(),
            },
            true,
        ),
        (
            AiEvent::PlanUpdated {
                version: 1,
                summary: golish_core::plan::PlanSummary {
                    total: 0,
                    completed: 0,
                    in_progress: 0,
                    pending: 0,
                },
                steps: vec![],
                explanation: None,
            },
            true,
        ),
        (
            AiEvent::ServerToolStarted {
                request_id: "r".into(),
                tool_name: "web_search".into(),
                input: serde_json::json!({}),
            },
            true,
        ),
        (
            AiEvent::WebSearchResult {
                request_id: "r".into(),
                results: serde_json::json!([]),
            },
            true,
        ),
        (
            AiEvent::WebFetchResult {
                request_id: "r".into(),
                url: "http://example.com".into(),
                content_preview: "preview".into(),
            },
            true,
        ),
        (
            AiEvent::PromptGenerationStarted {
                agent_id: "a".into(),
                parent_request_id: "p".into(),
                architect_system_prompt: "sys".into(),
                architect_user_message: "usr".into(),
            },
            true,
        ),
        (
            AiEvent::PromptGenerationCompleted {
                agent_id: "a".into(),
                parent_request_id: "p".into(),
                generated_prompt: Some("prompt".into()),
                success: true,
                duration_ms: 100,
            },
            true,
        ),
        (
            AiEvent::SubAgentTextDelta {
                agent_id: "a".into(),
                delta: "d".into(),
                accumulated: "d".into(),
                parent_request_id: "p".into(),
            },
            false,
        ),
        (
            AiEvent::SubtaskWaitingForInput {
                task_id: "t".into(),
                subtask_id: "s".into(),
                title: "title".into(),
                prompt: "question".into(),
            },
            true,
        ),
        (
            AiEvent::SubtaskUserInput {
                task_id: "t".into(),
                subtask_id: "s".into(),
                input: "answer".into(),
            },
            true,
        ),
        (
            AiEvent::TaskResumed {
                task_id: "t".into(),
                subtask_index: 2,
                total_subtasks: 5,
            },
            true,
        ),
        (
            AiEvent::EnricherResult {
                task_id: "t".into(),
                subtask_id: "s".into(),
                context_added: "extra context".into(),
            },
            true,
        ),
    ];

    // Compile-time exhaustiveness check: if a new variant is added to AiEvent,
    // this match will fail to compile, reminding you to add it above.
    fn _assert_exhaustive(e: &AiEvent) {
        match e {
            AiEvent::Started { .. }
            | AiEvent::UserMessage { .. }
            | AiEvent::SystemHooksInjected { .. }
            | AiEvent::TextDelta { .. }
            | AiEvent::ToolRequest { .. }
            | AiEvent::ToolIntentObservation { .. }
            | AiEvent::ToolApprovalRequest { .. }
            | AiEvent::ToolAutoApproved { .. }
            | AiEvent::ToolDenied { .. }
            | AiEvent::ToolResult { .. }
            | AiEvent::ToolOutputChunk { .. }
            | AiEvent::Reasoning { .. }
            | AiEvent::Completed { .. }
            | AiEvent::Error { .. }
            | AiEvent::SubAgentStarted { .. }
            | AiEvent::SubAgentToolRequest { .. }
            | AiEvent::SubAgentToolResult { .. }
            | AiEvent::SubAgentTextDelta { .. }
            | AiEvent::SubAgentReasoning { .. }
            | AiEvent::SubAgentCompleted { .. }
            | AiEvent::SubAgentError { .. }
            | AiEvent::ContextWarning { .. }
            | AiEvent::ToolResponseTruncated { .. }
            | AiEvent::Warning { .. }
            | AiEvent::CompactionStarted { .. }
            | AiEvent::CompactionCompleted { .. }
            | AiEvent::CompactionFailed { .. }
            | AiEvent::LoopWarning { .. }
            | AiEvent::LoopBlocked { .. }
            | AiEvent::MaxIterationsReached { .. }
            | AiEvent::WorkflowStarted { .. }
            | AiEvent::WorkflowStepStarted { .. }
            | AiEvent::WorkflowStepCompleted { .. }
            | AiEvent::WorkflowCompleted { .. }
            | AiEvent::WorkflowError { .. }
            | AiEvent::PlanUpdated { .. }
            | AiEvent::ServerToolStarted { .. }
            | AiEvent::WebSearchResult { .. }
            | AiEvent::WebFetchResult { .. }
            | AiEvent::PromptGenerationStarted { .. }
            | AiEvent::PromptGenerationCompleted { .. }
            | AiEvent::AskHumanRequest { .. }
            | AiEvent::AskHumanResponse { .. }
            | AiEvent::SubtaskWaitingForInput { .. }
            | AiEvent::SubtaskUserInput { .. }
            | AiEvent::TaskResumed { .. }
            | AiEvent::EnricherResult { .. }
            | AiEvent::TaskProgress { .. }
            | AiEvent::SubtaskCreated { .. }
            | AiEvent::SubtaskCompleted { .. } => {}
        }
    }

    variants
}

/// Tests that should_transcript returns the correct value for every AiEvent variant.
/// If a new variant is added to AiEvent, the exhaustive match in all_variants() will
/// fail to compile, forcing the developer to decide whether it should be transcribed.
#[test]
fn test_should_transcript_exhaustive() {
    for (event, expected) in all_variants() {
        let result = should_transcript(&event);
        assert_eq!(
            result,
            expected,
            "should_transcript({}) = {}, expected {}",
            event.event_type(),
            result,
            expected
        );
    }
}

/// Verify the specific filtered events return false.
#[test]
fn test_filtered_events() {
    let filtered = [
        AiEvent::TextDelta {
            delta: "x".into(),
            accumulated: "x".into(),
        },
        AiEvent::Reasoning {
            content: "think".into(),
        },
        AiEvent::ToolOutputChunk {
            request_id: "r".into(),
            tool_name: "t".into(),
            chunk: "out".into(),
            stream: "stdout".into(),
            source: Default::default(),
        },
        AiEvent::SubAgentToolRequest {
            agent_id: "a".into(),
            tool_name: "t".into(),
            args: serde_json::json!({}),
            request_id: "r".into(),
            parent_request_id: "p".into(),
        },
        AiEvent::SubAgentToolResult {
            agent_id: "a".into(),
            tool_name: "t".into(),
            success: true,
            result: serde_json::json!(null),
            request_id: "r".into(),
            parent_request_id: "p".into(),
        },
        AiEvent::SubAgentReasoning {
            agent_id: "a".into(),
            delta: "think".into(),
            accumulated: "think".into(),
            parent_request_id: "p".into(),
        },
    ];

    for event in &filtered {
        assert!(
            !should_transcript(event),
            "{} should be filtered from transcript",
            event.event_type()
        );
    }
}
