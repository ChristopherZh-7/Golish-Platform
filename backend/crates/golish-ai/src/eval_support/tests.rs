//! Eval support tests.

use std::path::PathBuf;

use super::*;

use super::*;

#[test]
fn test_eval_config_default() {
    let config = EvalConfig::default();
    assert_eq!(config.provider_name, "anthropic");
    assert!(!config.require_hitl);
}

#[test]
fn test_eval_config_openai() {
    let config = EvalConfig::openai("gpt-5.1", PathBuf::from("/tmp"));
    assert_eq!(config.provider_name, "openai");
    assert_eq!(config.model_name, "gpt-5.1");
}
