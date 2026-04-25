//! [`LlmJudgeMetric`] — pass/fail evaluation via an LLM judge.
//!
//! Runs an agentic loop where the judge can optionally use read-only tools
//! (`read_file`, `list_files`) to inspect the workspace before committing
//! to a verdict.

use anyhow::Result;
use async_trait::async_trait;
use rig::completion::{CompletionModel as RigCompletionModel, Message, ToolDefinition};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

use crate::metrics::{EvalContext, Metric, MetricResult};

use super::tools::{build_tool_definitions, execute_list_files, execute_read_file, PathArg};
use super::{create_judge_client, JUDGE_SYSTEM_PROMPT};

/// Metric that uses an LLM to judge whether output meets criteria.
///
/// Returns `Pass` if the LLM determines the criteria are met, `Fail`
/// otherwise.
pub struct LlmJudgeMetric {
    /// Name of this metric instance.
    pub(super) name: String,
    /// Criteria for the LLM to evaluate against.
    pub(super) criteria: String,
    /// Threshold for passing (0.0–1.0). Default is 0.7.
    #[allow(dead_code)]
    pub(super) threshold: f64,
    /// Whether to give the judge read-only tools to explore the workspace.
    pub(super) use_tools: bool,
}

impl LlmJudgeMetric {
    /// Create a new LLM judge metric.
    pub fn new(name: impl Into<String>, criteria: impl Into<String>, threshold: f64) -> Self {
        Self {
            name: name.into(),
            criteria: criteria.into(),
            threshold,
            use_tools: false,
        }
    }

    /// Create with default threshold of 0.7.
    pub fn with_criteria(name: impl Into<String>, criteria: impl Into<String>) -> Self {
        Self::new(name, criteria, 0.7)
    }

    /// Enable read-only tools (`read_file`, `list_files`) for the judge to
    /// explore the workspace.
    pub fn with_tools(mut self) -> Self {
        self.use_tools = true;
        self
    }
}

#[async_trait]
impl Metric for LlmJudgeMetric {
    fn name(&self) -> &str {
        &self.name
    }

    async fn evaluate(&self, ctx: &EvalContext) -> Result<MetricResult> {
        let model = match create_judge_client().await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    metric = %self.name,
                    error = %e,
                    "Failed to create LLM client for judge metric"
                );
                return Ok(MetricResult::Skip {
                    reason: format!("LLM client unavailable: {}", e),
                });
            }
        };

        // Build tool calls section if any.
        let tool_calls_section = if ctx.agent_output.tool_calls.is_empty() {
            String::new()
        } else {
            let calls: Vec<String> = ctx
                .agent_output
                .tool_calls
                .iter()
                .map(|tc| {
                    format!(
                        "- {}({}): {}",
                        tc.name,
                        serde_json::to_string(&tc.input).unwrap_or_default(),
                        if tc.success { "success" } else { "failed" }
                    )
                })
                .collect();
            format!("\n\n## Tool Calls Made\n{}", calls.join("\n"))
        };

        let tools_note = if self.use_tools {
            "\nYou have access to read_file and list_files tools to explore the workspace and verify the actual code/changes. Use them as needed before making your verdict.\n"
        } else {
            ""
        };

        let initial_prompt = format!(
            r#"## Original Task
{prompt}

## Assistant Response
{response}{tool_calls_section}

## Evaluation Criteria
{criteria}

## Instructions
Evaluate whether the assistant's response meets the criteria above.
{tools_note}
When you are ready to give your verdict, your response MUST start with exactly one of these two words:
- PASS - if the criteria are fully met
- FAIL - if the criteria are not met

If FAIL, add a brief reason after a colon, like: FAIL: reason here"#,
            prompt = ctx.prompt,
            response = ctx.agent_output.response,
            tool_calls_section = tool_calls_section,
            criteria = self.criteria,
            tools_note = tools_note,
        );

        let tools: Vec<ToolDefinition> = if self.use_tools {
            build_tool_definitions()
        } else {
            vec![]
        };

        // Agentic loop.
        let mut chat_history: Vec<Message> = vec![Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: initial_prompt,
            })),
        }];

        const MAX_ITERATIONS: usize = 10;
        for iteration in 0..MAX_ITERATIONS {
            let request = rig::completion::CompletionRequest {
                preamble: Some(JUDGE_SYSTEM_PROMPT.to_string()),
                chat_history: OneOrMany::many(chat_history.clone())
                    .unwrap_or_else(|_| OneOrMany::one(chat_history[0].clone())),
                documents: vec![],
                tools: tools.clone(),
                temperature: Some(0.0),
                max_tokens: Some(1024),
                tool_choice: None,
                additional_params: None,
                model: None,
                output_schema: None,
            };

            let response = model.completion(request).await?;

            let tool_calls: Vec<_> = response
                .choice
                .iter()
                .filter_map(|c| match c {
                    rig::completion::AssistantContent::ToolCall(tc) => Some(tc.clone()),
                    _ => None,
                })
                .collect();

            let response_text: String = response
                .choice
                .iter()
                .filter_map(|c| match c {
                    rig::completion::AssistantContent::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            // If no tool calls, check for verdict.
            if tool_calls.is_empty() {
                return Self::parse_verdict(&response_text, &self.name);
            }

            tracing::debug!(
                metric = %self.name,
                iteration = iteration,
                tool_count = tool_calls.len(),
                "Judge using tools"
            );

            // Add assistant message with tool calls.
            chat_history.push(Message::Assistant {
                id: None,
                content: response.choice.clone(),
            });

            // Execute tools and add results.
            for tool_call in tool_calls {
                let args_str = tool_call.function.arguments.to_string();
                let result = match tool_call.function.name.as_str() {
                    "read_file" => match serde_json::from_str::<PathArg>(&args_str) {
                        Ok(arg) => execute_read_file(&ctx.workspace, &arg.path),
                        Err(e) => format!("Error parsing arguments: {}", e),
                    },
                    "list_files" => match serde_json::from_str::<PathArg>(&args_str) {
                        Ok(arg) => execute_list_files(&ctx.workspace, &arg.path),
                        Err(e) => format!("Error parsing arguments: {}", e),
                    },
                    _ => format!("Unknown tool: {}", tool_call.function.name),
                };

                chat_history.push(Message::User {
                    content: OneOrMany::one(UserContent::ToolResult(rig::message::ToolResult {
                        id: tool_call.id.clone(),
                        call_id: Some(tool_call.id),
                        content: OneOrMany::one(rig::message::ToolResultContent::Text(Text {
                            text: result,
                        })),
                    })),
                });
            }
        }

        Ok(MetricResult::Fail {
            reason: "Judge exceeded maximum tool iterations without verdict".to_string(),
        })
    }
}

impl LlmJudgeMetric {
    /// Parse the verdict from the response text. Looks for `PASS` / `FAIL`
    /// at the start of the response first; falls back to a substring scan
    /// when the LLM doesn't follow the verdict-prefix convention.
    fn parse_verdict(response_text: &str, metric_name: &str) -> Result<MetricResult> {
        let response_trimmed = response_text.trim();
        let response_upper = response_trimmed.to_uppercase();

        tracing::debug!(
            metric = %metric_name,
            response = %response_text,
            "LLM judge full response"
        );

        if response_upper.starts_with("PASS") {
            tracing::info!(metric = %metric_name, "Judge verdict: PASS");
            return Ok(MetricResult::Pass);
        }
        if response_upper.starts_with("FAIL") {
            let reason = response_trimmed
                .strip_prefix("FAIL:")
                .or_else(|| response_trimmed.strip_prefix("FAIL"))
                .or_else(|| response_trimmed.strip_prefix("Fail:"))
                .or_else(|| response_trimmed.strip_prefix("Fail"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "Criteria not met".to_string());
            tracing::info!(metric = %metric_name, reason = %reason, "Judge verdict: FAIL");
            return Ok(MetricResult::Fail { reason });
        }

        // Fallback: look for PASS/FAIL anywhere in the response.
        if response_upper.contains("PASS") && !response_upper.contains("FAIL") {
            tracing::info!(
                metric = %metric_name,
                "Judge verdict: PASS (found in response body)"
            );
            return Ok(MetricResult::Pass);
        }
        if response_upper.contains("FAIL") {
            let reason = if let Some(pos) = response_trimmed.to_uppercase().find("FAIL") {
                let after_fail = &response_trimmed[pos + 4..];
                after_fail
                    .strip_prefix(':')
                    .or(Some(after_fail))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Criteria not met".to_string())
            } else {
                "Criteria not met".to_string()
            };
            tracing::info!(
                metric = %metric_name,
                reason = %reason,
                "Judge verdict: FAIL (found in response body)"
            );
            return Ok(MetricResult::Fail { reason });
        }

        tracing::warn!(
            metric = %metric_name,
            response = %response_text,
            "Unexpected LLM judge response format - no PASS/FAIL found"
        );
        let preview_len = response_text.len().min(500);
        Ok(MetricResult::Fail {
            reason: format!(
                "Unexpected judge response (no PASS/FAIL): {}{}",
                response_text.chars().take(preview_len).collect::<String>(),
                if response_text.len() > preview_len {
                    "..."
                } else {
                    ""
                }
            ),
        })
    }
}
