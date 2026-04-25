use crate::metrics::Metric;

use super::extract_last_number;
use super::judge::LlmJudgeMetric;
use super::score::LlmScoreMetric;

#[test]
fn test_llm_judge_metric_creation() {
    let metric = LlmJudgeMetric::new("test", "criteria here", 0.8);
    assert_eq!(metric.name(), "test");
    assert_eq!(metric.criteria, "criteria here");
}

#[test]
fn test_llm_judge_with_criteria() {
    let metric = LlmJudgeMetric::with_criteria("test", "criteria");
    assert_eq!(metric.threshold, 0.7);
}

#[test]
fn test_llm_score_metric_scale_10() {
    let metric = LlmScoreMetric::scale_10("quality", "code quality", 7.0);
    assert_eq!(metric.min_score, 7.0);
    assert_eq!(metric.max_score, 10.0);
}

#[test]
fn test_extract_last_number_simple() {
    assert_eq!(extract_last_number("10"), Some(10.0));
    assert_eq!(extract_last_number("7.5"), Some(7.5));
}

#[test]
fn test_extract_last_number_with_reasoning() {
    assert_eq!(
        extract_last_number("The score is 8 out of 10."),
        Some(10.0) // Last number is 10
    );
    assert_eq!(
        extract_last_number("I would give this a score of 9"),
        Some(9.0)
    );
}

#[test]
fn test_extract_last_number_with_decimal() {
    assert_eq!(
        extract_last_number("Based on the criteria, my score is 8.5"),
        Some(8.5)
    );
}

#[test]
fn test_extract_last_number_none() {
    assert_eq!(extract_last_number("no numbers here"), None);
    assert_eq!(extract_last_number(""), None);
}
