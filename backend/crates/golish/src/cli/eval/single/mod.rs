//! Scenario-style evaluation runners.
//!
//! Hosts the two top-level entry points that operate on the hand-curated
//! scenario catalogue (rather than benchmark suites):
//!
//! - [`run_evals`]               — full default eval suite (or filtered).
//! - [`run_openai_model_tests`]  — OpenAI connectivity sweep across the
//!   known model list.
//!
//! The parallel/sequential implementations live in [`runners`] and the
//! transcript pretty-printer in [`transcript`].

mod runners;
mod transcript;

use anyhow::Result;
use golish_evals::scenarios::{
    default_scenarios_for_provider, get_openai_model_scenario, get_scenario, list_openai_models,
    openai_model_scenarios,
};
use golish_evals::EvalProvider;
use tracing_subscriber::EnvFilter;

use super::{color, EvalOutputOptions};

/// Run evaluation scenarios.
pub async fn run_evals(
    scenario_filter: Option<&str>,
    json_output: bool,
    verbose: bool,
    parallel: bool,
    provider: EvalProvider,
    output_options: Option<EvalOutputOptions>,
) -> Result<()> {
    let log_level = if verbose { "debug" } else { "warn" };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(format!("golish={}", log_level).parse().unwrap())
                .add_directive(format!("golish_evals={}", log_level).parse().unwrap())
                .add_directive(format!("golish_agent_kit={}", log_level).parse().unwrap())
                .add_directive(
                    format!("golish_agent_runtime={}", log_level)
                        .parse()
                        .unwrap(),
                )
                .add_directive(
                    format!("golish_agent_bridge={}", log_level)
                        .parse()
                        .unwrap(),
                )
                .add_directive(format!("golish_prompts={}", log_level).parse().unwrap()),
        )
        .try_init();

    let scenarios = if let Some(name) = scenario_filter {
        match get_scenario(name) {
            Some(s) => vec![s],
            None => {
                eprintln!("Unknown scenario: {}", name);
                eprintln!("Use --list-scenarios to see available scenarios");
                anyhow::bail!("Unknown scenario: {}", name);
            }
        }
    } else {
        default_scenarios_for_provider(provider)
    };

    let use_new_output = output_options.is_some();
    let opts = output_options.unwrap_or(EvalOutputOptions {
        json: json_output,
        pretty: false,
        output_file: None,
        transcript: false,
    });

    let suppress_intermediate = use_new_output || opts.transcript;

    if !opts.json && !suppress_intermediate {
        println!("Using LLM provider: {}", provider);
    }

    let summary = if parallel && scenarios.len() > 1 {
        runners::run_parallel(
            scenarios,
            opts.json,
            verbose,
            provider,
            suppress_intermediate,
        )
        .await?
    } else {
        runners::run_sequential(
            scenarios,
            opts.json,
            verbose,
            provider,
            suppress_intermediate,
        )
        .await?
    };

    if let Some(ref output_path) = opts.output_file {
        let file = std::fs::File::create(output_path)?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &summary.to_json())?;
        eprintln!("Results saved to: {}", output_path.display());
    }

    if opts.transcript {
        transcript::print_transcript(&summary);
    }

    if opts.pretty {
        summary.print_ci_summary(&mut std::io::stdout(), &provider.to_string())?;
    } else if opts.json {
        println!("{}", serde_json::to_string(&summary.to_json())?);
    } else if !use_new_output {
        summary.print_summary(&mut std::io::stdout())?;
    }

    // Z.AI uses 80% pass threshold, others require 100%
    let pass_threshold = match provider {
        EvalProvider::Zai => 0.80,
        _ => 1.0,
    };
    let passed = summary.pass_rate() >= pass_threshold;

    println!();
    if !passed {
        println!("{}", color::red_line());
        println!(
            "{}",
            color::red(&format!(
                "  FAIL: {} of {} scenarios failed ({:.0}% pass rate, {:.0}% required)",
                summary.failed_count(),
                summary.reports.len(),
                summary.pass_rate() * 100.0,
                pass_threshold * 100.0
            ))
        );
        println!("{}", color::red_line());
        anyhow::bail!(
            "{} of {} scenarios failed ({:.0}% pass rate, {:.0}% required)",
            summary.failed_count(),
            summary.reports.len(),
            summary.pass_rate() * 100.0,
            pass_threshold * 100.0
        );
    } else {
        println!("{}", color::green_line());
        if summary.failed_count() > 0 {
            println!(
                "{}",
                color::green(&format!(
                    "  PASS: {}/{} scenarios passed ({:.0}% >= {:.0}% threshold)",
                    summary.passed_count(),
                    summary.reports.len(),
                    summary.pass_rate() * 100.0,
                    pass_threshold * 100.0
                ))
            );
        } else {
            println!(
                "{}",
                color::green(&format!(
                    "  PASS: All {} scenarios passed",
                    summary.reports.len()
                ))
            );
        }
        println!("{}", color::green_line());
    }

    Ok(())
}

/// Run OpenAI model connectivity tests.
///
/// Tests each OpenAI model (or a specific one) with a simple hello world
/// prompt to verify configuration and connectivity.
pub async fn run_openai_model_tests(
    model_filter: Option<&str>,
    json_output: bool,
    verbose: bool,
    parallel: bool,
) -> Result<()> {
    let log_level = if verbose { "debug" } else { "warn" };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(format!("golish={}", log_level).parse().unwrap())
                .add_directive(format!("golish_evals={}", log_level).parse().unwrap())
                .add_directive(format!("golish_agent_kit={}", log_level).parse().unwrap())
                .add_directive(
                    format!("golish_agent_runtime={}", log_level)
                        .parse()
                        .unwrap(),
                )
                .add_directive(
                    format!("golish_agent_bridge={}", log_level)
                        .parse()
                        .unwrap(),
                )
                .add_directive(format!("golish_prompts={}", log_level).parse().unwrap()),
        )
        .try_init();

    let scenarios = if let Some(model_id) = model_filter {
        match get_openai_model_scenario(model_id) {
            Some(s) => vec![s],
            None => {
                eprintln!("Unknown OpenAI model: {}", model_id);
                eprintln!("Available models:");
                for (id, name) in list_openai_models() {
                    eprintln!("  {} - {}", id, name);
                }
                anyhow::bail!("Unknown OpenAI model: {}", model_id);
            }
        }
    } else {
        openai_model_scenarios()
    };

    if !json_output {
        println!(
            "Testing OpenAI model connectivity ({} models)",
            scenarios.len()
        );
        println!("Provider: openai\n");
    }

    let provider = EvalProvider::OpenAi;

    let summary = if parallel && scenarios.len() > 1 {
        runners::run_parallel(scenarios, json_output, verbose, provider, false).await?
    } else {
        runners::run_sequential(scenarios, json_output, verbose, provider, false).await?
    };

    if json_output {
        println!("{}", serde_json::to_string(&summary.to_json())?);
    } else {
        summary.print_summary(&mut std::io::stdout())?;
    }

    println!();
    if summary.failed_count() > 0 {
        println!("{}", color::red_line());
        println!(
            "{}",
            color::red(&format!(
                "  FAIL: {} of {} models failed connectivity test",
                summary.failed_count(),
                summary.reports.len()
            ))
        );
        println!("{}", color::red_line());
        anyhow::bail!(
            "{} of {} models failed connectivity test",
            summary.failed_count(),
            summary.reports.len()
        );
    } else {
        println!("{}", color::green_line());
        println!(
            "{}",
            color::green(&format!(
                "  PASS: All {} models passed connectivity test",
                summary.reports.len()
            ))
        );
        println!("{}", color::green_line());
    }

    Ok(())
}
