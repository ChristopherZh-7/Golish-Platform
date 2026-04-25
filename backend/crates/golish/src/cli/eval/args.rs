//! `--list-*` printers used by the eval CLI flags.
//!
//! These three functions back the help-style flags that print what's
//! available rather than running anything:
//!
//! - [`list_scenarios`]              — `--list-scenarios`
//! - [`list_openai_model_scenarios`] — `--list-openai-models`
//! - [`list_benchmark_options`]      — `--list-benchmarks`
//!
//! Kept in their own module so the runners (`single`, `humaneval`,
//! `swebench`) don't have to drag the scenario/benchmark catalogue imports
//! along with them.

use golish_evals::scenarios::{all_scenarios, list_openai_models};

/// List all available scenarios.
pub fn list_scenarios() {
    println!("Available evaluation scenarios:\n");
    for scenario in all_scenarios() {
        println!("  {} - {}", scenario.name(), scenario.description());
    }
    println!();
}

/// List available OpenAI models for testing.
pub fn list_openai_model_scenarios() {
    println!("Available OpenAI models for connectivity testing:\n");
    for (model_id, model_name) in list_openai_models() {
        println!("  {} - {}", model_id, model_name);
    }
    println!();
    println!("Run with: --openai-models");
    println!("Run specific model: --openai-models --openai-model gpt-5.1");
    println!();
}

/// List available benchmarks (HumanEval-style + SWE-bench).
pub fn list_benchmark_options() {
    println!("Available benchmarks:\n");
    for (name, description, count) in golish_benchmarks::list_benchmarks() {
        println!("  {} - {} ({} problems)", name, description, count);
    }

    // Add SWE-bench info
    let (name, desc, count) = golish_swebench::benchmark_info();
    println!("  {} - {} ({} instances)", name, desc, count);

    println!();
    println!("Run with:");
    println!("  --benchmark humaneval              # HumanEval benchmark");
    println!("  --swebench                         # SWE-bench Lite benchmark");
    println!();
    println!("Filter examples:");
    println!("  --benchmark humaneval --problems 0-9");
    println!("  --swebench --instance django__django-11133");
    println!("  --swebench --problems 0-9");
    println!();
}
