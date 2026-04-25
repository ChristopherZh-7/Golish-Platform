//! Generic benchmark runner used for HumanEval (and similar problem-set benchmarks).
//!
//! [`run_benchmark`] is the public CLI entry point; the
//! [`run_sequential_benchmark`] / [`run_parallel_benchmark`] helpers do the
//! actual work and are also reused by [`super::swebench`] for its parallel
//! fallback path (sequential SWE-bench has its own incremental-saving
//! runner in `swebench.rs`).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::future::join_all;
use golish_evals::indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use golish_evals::outcome::EvalSummary;
use golish_evals::runner::EvalRunner;
use golish_evals::EvalProvider;
use tokio::sync::Semaphore;
use tracing_subscriber::EnvFilter;

use super::{color, EvalOutputOptions};

/// Run a benchmark suite.
///
/// # Arguments
/// * `benchmark` - Name of the benchmark to run (e.g., "humaneval")
/// * `problems` - Optional problem filter (e.g., "0-10" or "0,5,10")
/// * `json_output` - Whether to output JSON
/// * `verbose` - Whether to show verbose output
/// * `parallel` - Whether to run scenarios in parallel
/// * `concurrency` - Maximum number of concurrent scenarios when parallel
/// * `provider` - LLM provider to use
/// * `model` - Optional model override
/// * `output_options` - Optional output configuration
#[allow(clippy::too_many_arguments)]
pub async fn run_benchmark(
    benchmark: &str,
    problems: Option<&str>,
    json_output: bool,
    verbose: bool,
    parallel: bool,
    concurrency: usize,
    provider: EvalProvider,
    model: Option<&str>,
    output_options: Option<EvalOutputOptions>,
) -> Result<()> {
    // Initialize tracing for evals - always use error level to suppress noise
    // We handle our own verbose output display
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("golish=error".parse().unwrap())
                .add_directive("golish_evals=error".parse().unwrap())
                .add_directive("golish_ai=error".parse().unwrap())
                .add_directive("golish_benchmarks=error".parse().unwrap()),
        )
        .try_init();

    let scenarios = golish_benchmarks::get_benchmark_scenarios(benchmark, problems)?;

    if scenarios.is_empty() {
        anyhow::bail!(
            "No problems found for benchmark '{}' with filter '{}'",
            benchmark,
            problems.unwrap_or("none")
        );
    }

    if !json_output {
        println!(
            "Running {} benchmark ({} problems)",
            benchmark,
            scenarios.len()
        );
        println!("Provider: {}\n", provider);
    }

    // Determine if we should suppress normal output
    let use_new_output = output_options.is_some();
    let opts = output_options.unwrap_or(EvalOutputOptions {
        json: json_output,
        pretty: false,
        output_file: None,
        transcript: false,
    });

    let suppress_intermediate = use_new_output || opts.transcript;

    let summary = if parallel && scenarios.len() > 1 {
        run_parallel_benchmark(
            scenarios,
            opts.json,
            verbose,
            provider,
            model,
            suppress_intermediate,
            concurrency,
        )
        .await?
    } else {
        run_sequential_benchmark(
            scenarios,
            opts.json,
            verbose,
            provider,
            model,
            suppress_intermediate,
        )
        .await?
    };

    // Handle output based on options
    if let Some(ref output_path) = opts.output_file {
        let file = std::fs::File::create(output_path)?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &summary.to_json())?;
        eprintln!("Results saved to: {}", output_path.display());
    }

    if opts.pretty {
        summary.print_ci_summary(&mut std::io::stdout(), &provider.to_string())?;
    } else if opts.json {
        println!("{}", serde_json::to_string(&summary.to_json())?);
    } else if !use_new_output {
        summary.print_summary(&mut std::io::stdout())?;
    }

    // Print final pass rate
    let pass_rate = summary.pass_rate();
    println!();
    if pass_rate < 1.0 {
        println!("{}", color::red_line());
        println!(
            "{}",
            color::red(&format!(
                "  {}: {}/{} passed ({:.1}%)",
                benchmark.to_uppercase(),
                summary.passed_count(),
                summary.reports.len(),
                pass_rate * 100.0
            ))
        );
        println!("{}", color::red_line());
    } else {
        println!("{}", color::green_line());
        println!(
            "{}",
            color::green(&format!(
                "  {}: All {} problems passed (100%)",
                benchmark.to_uppercase(),
                summary.reports.len()
            ))
        );
        println!("{}", color::green_line());
    }

    Ok(())
}

/// Run benchmark scenarios sequentially.
async fn run_sequential_benchmark(
    scenarios: Vec<Box<dyn golish_evals::scenarios::Scenario>>,
    json_output: bool,
    verbose: bool,
    provider: EvalProvider,
    model: Option<&str>,
    quiet: bool,
) -> Result<EvalSummary> {
    // Enable verbose to show tool calls and reasoning in real-time
    let runner = EvalRunner::new_verbose_with_provider(verbose, provider)?
        .with_model(model.map(|s| s.to_string()));
    let mut summary = EvalSummary::default();

    for scenario in scenarios {
        if !json_output && !quiet {
            println!("\n{}", color::cyan(&format!("=== {} ===", scenario.name())));
            if verbose {
                println!("\n{}:", color::yellow("Prompt"));
                println!("{}", scenario.prompt());
                println!();
            }
        }

        match scenario.run(&runner).await {
            Ok(report) => {
                if json_output && !quiet {
                    println!("{}", serde_json::to_string(&report.to_json())?);
                } else if !quiet {
                    // Show agent response in verbose mode
                    if verbose {
                        println!("{}:", color::yellow("Response"));
                        println!("{}", report.agent_output.response);
                        println!();

                        // Show tool calls
                        if !report.agent_output.tool_calls.is_empty() {
                            println!("{}:", color::yellow("Tool Calls"));
                            for tc in &report.agent_output.tool_calls {
                                let status = if tc.success { "✓" } else { "✗" };
                                println!("  {} {}", status, tc.name);
                            }
                            println!();
                        }
                    }

                    let status = if report.passed {
                        color::green("PASS")
                    } else {
                        color::red("FAIL")
                    };
                    println!("Result: {} ({}ms)", status, report.duration_ms);

                    // Show failure details
                    if !report.passed {
                        for metric in &report.metrics {
                            if !metric.result.passed() {
                                if let golish_evals::MetricResult::Fail { reason } = &metric.result {
                                    println!("  {} failed: {}", metric.name, reason);
                                }
                            }
                        }
                    }
                }
                summary.add(report);
            }
            Err(e) => {
                eprintln!("Error running {}: {:#}", scenario.name(), e);
            }
        }
    }

    Ok(summary)
}

/// Run benchmark scenarios in parallel with concurrency limiting.
///
/// Visible to the rest of the eval module so [`super::swebench`] can fall
/// back to it for its parallel path (the sequential SWE-bench runner
/// supports incremental on-disk saving and lives in `swebench.rs`).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_parallel_benchmark(
    scenarios: Vec<Box<dyn golish_evals::scenarios::Scenario>>,
    json_output: bool,
    verbose: bool,
    provider: EvalProvider,
    model: Option<&str>,
    quiet: bool,
    concurrency: usize,
) -> Result<EvalSummary> {
    let model_owned = model.map(|s| s.to_string());
    let semaphore = Arc::new(Semaphore::new(concurrency));

    // For JSON output or quiet mode, use simple execution
    if json_output || quiet {
        let futures: Vec<_> = scenarios
            .into_iter()
            .map(|scenario| {
                let name = scenario.name().to_string();
                let model_clone = model_owned.clone();
                let sem = semaphore.clone();
                async move {
                    // Acquire semaphore permit to limit concurrency
                    let _permit = sem.acquire().await.unwrap();
                    let runner = match EvalRunner::new_with_provider(provider) {
                        Ok(r) => r.with_model(model_clone),
                        Err(e) => return (name, Err(e)),
                    };
                    let result = scenario.run(&runner).await;
                    (name, result)
                }
            })
            .collect();

        let results = join_all(futures).await;
        let mut summary = EvalSummary::default();

        for (name, result) in results {
            match result {
                Ok(report) => {
                    if !quiet {
                        println!("{}", serde_json::to_string(&report.to_json())?);
                    }
                    summary.add(report);
                }
                Err(e) => {
                    eprintln!("Error running {}: {}", name, e);
                }
            }
        }

        return Ok(summary);
    }

    // Progress bar display
    let multi_progress = MultiProgress::new();
    let header = multi_progress.add(ProgressBar::new_spinner());
    header.set_style(ProgressStyle::default_spinner().template("{msg}").unwrap());
    header.set_message(format!(
        "Running {} problems in parallel (max {} concurrent)",
        scenarios.len(),
        concurrency
    ));
    header.tick();

    let spinner_style = ProgressStyle::default_spinner()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
        .template("  {spinner:.cyan} {wide_msg}")
        .unwrap();

    // Create progress bars showing "queued" initially
    let progress_bars: Vec<_> = scenarios
        .iter()
        .map(|scenario| {
            let pb = multi_progress.add(ProgressBar::new_spinner());
            pb.set_style(spinner_style.clone());
            pb.set_message(format!("{:<20} queued", scenario.name()));
            pb.enable_steady_tick(Duration::from_millis(100));
            pb
        })
        .collect();

    let futures: Vec<_> = scenarios
        .into_iter()
        .zip(progress_bars.into_iter())
        .map(|(scenario, pb)| {
            let name = scenario.name().to_string();
            let model_clone = model_owned.clone();
            let sem = semaphore.clone();
            async move {
                // Acquire semaphore permit to limit concurrency
                let _permit = sem.acquire().await.unwrap();
                pb.set_message(format!("{:<20} running...", name));

                let runner = match EvalRunner::new_verbose_with_provider(verbose, provider) {
                    Ok(r) => r.with_model(model_clone),
                    Err(e) => {
                        pb.set_style(
                            ProgressStyle::default_spinner()
                                .template("  {msg}")
                                .unwrap(),
                        );
                        pb.finish_with_message(format!(
                            "{} {:<20} error: {}",
                            color::x_mark(),
                            name,
                            e
                        ));
                        return (name, Err(e));
                    }
                };

                let result = scenario.run(&runner).await;

                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("  {msg}")
                        .unwrap(),
                );

                match &result {
                    Ok(report) => {
                        let status = if report.passed {
                            format!(
                                "{} {:<20} {}",
                                color::check_mark(),
                                name,
                                color::green("passed")
                            )
                        } else {
                            format!("{} {:<20} {}", color::x_mark(), name, color::red("failed"))
                        };
                        pb.finish_with_message(status);
                    }
                    Err(e) => {
                        pb.finish_with_message(format!(
                            "{} {:<20} {}: {}",
                            color::x_mark(),
                            name,
                            color::red("error"),
                            e
                        ));
                    }
                }

                (name, result)
            }
        })
        .collect();

    let results = join_all(futures).await;
    header.finish_and_clear();

    let mut summary = EvalSummary::default();
    for (_name, result) in results {
        if let Ok(report) = result {
            summary.add(report);
        }
    }

    println!();
    Ok(summary)
}
