//! [`LlmScoreMetric`] — numeric LLM scoring on a 0–N scale.

use anyhow::Result;
use async_trait::async_trait;
use rig::completion::{CompletionModel as RigCompletionModel, Message};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

use crate::metrics::{EvalContext, Metric, MetricResult};

use super::{create_judge_client, extract_last_number, JUDGE_SYSTEM_PROMPT};

/// Metric that uses an LLM to score output on a numeric scale.
///
/// Returns a [`MetricResult::Score`] with the LLM's numeric evaluation,
/// or `Fail` if the score is below `min_score` or unparseable.
pub struct LlmScoreMetric {
    /// Name of this metric instance.
    pub(super) name: String,
    /// Criteria for scoring.
    pub(super) criteria: String,
    /// Minimum passing score.
    pub(super) min_score: f64,
    /// Maximum possible score.
    pub(super) max_score: f64,
}

impl LlmScoreMetric {
    /// Create a new LLM score metric.
    pub fn new(
        name: impl Into<String>,
        criteria: impl Into<String>,
        min_score: f64,
        max_score: f64,
    ) -> Self {
        Self {
            name: name.into(),
            criteria: criteria.into(),
            min_score,
            max_score,
        }
    }

    /// Create a metric that scores on a 0–10 scale.
    pub fn scale_10(
        name: impl Into<String>,
        criteria: impl Into<String>,
        min_passing: f64,
    ) -> Self {
        Self::new(name, criteria, min_passing, 10.0)
    }
}

#[async_trait]
impl Metric for LlmScoreMetric {
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
                    "Failed to create LLM client for score metric"
                );
                return Ok(MetricResult::Skip {
                    reason: format!("LLM client unavailable: {}", e),
                });
            }
        };

        // Build tool calls section if any (mirrors LlmJudgeMetric).
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

        let prompt = format!(
            r#"## Original Task
{prompt}

## Assistant Response
{response}{tool_calls_section}

## Scoring Criteria
{criteria}

## Instructions
Score the assistant's response on a scale of 0 to {max_score:.0}.

Consider:
- How well the criteria are met
- Code quality and correctness
- Completeness of the solution

Respond with EXACTLY one number between 0 and {max_score:.0} (can include decimals like 7.5).
Do not include any other text.

Your score:"#,
            prompt = ctx.prompt,
            response = ctx.agent_output.response,
            tool_calls_section = tool_calls_section,
            criteria = self.criteria,
            max_score = self.max_score,
        );

        let chat_history: Vec<Message> = vec![Message::User {
            content: OneOrMany::one(UserContent::Text(Text { text: prompt })),
        }];

        let request = rig::completion::CompletionRequest {
            preamble: Some(JUDGE_SYSTEM_PROMPT.to_string()),
            chat_history: OneOrMany::many(chat_history.clone())
                .unwrap_or_else(|_| OneOrMany::one(chat_history[0].clone())),
            documents: vec![],
            tools: vec![],
            temperature: Some(0.0), // Deterministic evaluation
            max_tokens: Some(256),  // Allow space for reasoning before score
            tool_choice: None,
            additional_params: None,
            model: None,
            output_schema: None,
        };

        let response = model.completion(request).await?;

        let response_text = response
            .choice
            .iter()
            .filter_map(|c| match c {
                rig::completion::AssistantContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Try exact match first, then extract the last number from a
        // response that includes reasoning before the score.
        let score_str = response_text.trim();
        let score = score_str
            .parse::<f64>()
            .ok()
            .or_else(|| extract_last_number(score_str));

        match score {
            Some(score) => {
                let clamped = score.clamp(0.0, self.max_score);
                if (score - clamped).abs() > 0.01 {
                    tracing::warn!(
                        metric = %self.name,
                        raw_score = score,
                        clamped = clamped,
                        "Score was out of range, clamped"
                    );
                }

                if clamped >= self.min_score {
                    Ok(MetricResult::Score {
                        value: clamped,
                        max: self.max_score,
                    })
                } else {
                    Ok(MetricResult::Fail {
                        reason: format!("Score {:.1} below minimum {:.1}", clamped, self.min_score),
                    })
                }
            }
            None => {
                tracing::warn!(
                    metric = %self.name,
                    response = %response_text,
                    "Failed to parse LLM score response"
                );
                Ok(MetricResult::Fail {
                    reason: format!(
                        "Invalid score response: {}",
                        score_str.chars().take(50).collect::<String>()
                    ),
                })
            }
        }
    }
}
