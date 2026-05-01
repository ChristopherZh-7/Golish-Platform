//! Sequential and parallel scenario runners.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::future::join_all;
use golish_evals::indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use golish_evals::outcome::{EvalReport, EvalSummary};
use golish_evals::runner::EvalRunner;
use golish_evals::scenarios::Scenario;
use golish_evals::EvalProvider;

use super::super::{color, metric_pass_threshold};

/// Run scenarios sequentially.
pub(super) async fn run_sequential(
    scenarios: Vec<Box<dyn Scenario>>,
    json_output: bool,
    verbose: bool,
    provider: EvalProvider,
    quiet: bool,
) -> Result<EvalSummary> {
    let runner = EvalRunner::new_verbose_with_provider(verbose, provider)?;
    let mut summary = EvalSummary::default();
    let threshold = metric_pass_threshold(provider);

    for scenario in scenarios {
        if !json_output && !quiet {
            println!("Running scenario: {}", scenario.name());
        }

        match scenario.run(&runner).await {
            Ok(mut report) => {
                report.apply_pass_threshold(threshold);

                if json_output && !quiet {
                    println!("{}", serde_json::to_string(&report.to_json())?);
                } else if !quiet {
                    report.print_summary(&mut std::io::stdout())?;
                }
                summary.add(report);
            }
            Err(e) => {
                eprintln!("Error running scenario {}: {}", scenario.name(), e);
            }
        }
    }

    Ok(summary)
}

/// Run scenarios in parallel with animated progress display.
pub(super) async fn run_parallel(
    scenarios: Vec<Box<dyn Scenario>>,
    json_output: bool,
    verbose: bool,
    provider: EvalProvider,
    quiet: bool,
) -> Result<EvalSummary> {
    let log_dir = if verbose {
        let dir = std::env::temp_dir().join("golish-eval-logs");
        std::fs::create_dir_all(&dir)?;
        Some(Arc::new(dir))
    } else {
        None
    };

    if json_output || quiet {
        return run_parallel_simple(scenarios, log_dir, provider, quiet).await;
    }

    let multi_progress = MultiProgress::new();

    let header = multi_progress.add(ProgressBar::new_spinner());
    header.set_style(ProgressStyle::default_spinner().template("{msg}").unwrap());
    let scenario_count = scenarios.len();
    if let Some(ref dir) = log_dir {
        header.set_message(format!(
            "Running {} scenarios in parallel (logs: {})",
            scenario_count,
            dir.display()
        ));
    } else {
        header.set_message(format!("Running {} scenarios in parallel", scenario_count));
    }
    header.tick();

    let spinner_style = ProgressStyle::default_spinner()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
        .template("  {spinner:.cyan} {wide_msg}")
        .unwrap();

    let progress_bars: Vec<_> = scenarios
        .iter()
        .map(|scenario| {
            let pb = multi_progress.add(ProgressBar::new_spinner());
            pb.set_style(spinner_style.clone());
            pb.set_message(format!("{:<20} running...", scenario.name()));
            pb.enable_steady_tick(Duration::from_millis(100));
            pb
        })
        .collect();

    let futures: Vec<_> = scenarios
        .into_iter()
        .zip(progress_bars.into_iter())
        .map(|(scenario, pb)| {
            let name = scenario.name().to_string();
            let log_dir_clone = log_dir.clone();
            let log_file = log_dir_clone
                .as_ref()
                .map(|dir| dir.join(format!("{}.log", name)));
            let log_path_for_result = log_file.clone();

            async move {
                let runner = if let Some(path) = log_file {
                    match EvalRunner::new_with_log_file_and_provider(path, provider) {
                        Ok(r) => r,
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
                            return (name, Err(e), None::<PathBuf>);
                        }
                    }
                } else {
                    match EvalRunner::new_with_provider(provider) {
                        Ok(r) => r,
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
                            return (name, Err(e), None);
                        }
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
                        let passed = report.metrics.iter().filter(|m| m.result.passed()).count();
                        let total = report.metrics.len();
                        let duration_secs = report.duration_ms as f64 / 1000.0;

                        let status = if report.passed {
                            format!(
                                "{} {:<20} {} ({}/{} metrics) [{:.1}s]",
                                color::check_mark(),
                                name,
                                color::green("passed"),
                                passed,
                                total,
                                duration_secs
                            )
                        } else {
                            format!(
                                "{} {:<20} {} ({}/{} metrics) [{:.1}s]",
                                color::x_mark(),
                                name,
                                color::red("failed"),
                                passed,
                                total,
                                duration_secs
                            )
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

                (name, result, log_path_for_result)
            }
        })
        .collect();

    let results = join_all(futures).await;

    header.finish_and_clear();

    let mut summary = EvalSummary::default();
    let mut reports: Vec<(String, EvalReport, Option<PathBuf>)> = Vec::new();
    let mut errors: Vec<(String, anyhow::Error)> = Vec::new();
    let threshold = metric_pass_threshold(provider);

    for (name, result, log_path) in results {
        match result {
            Ok(mut report) => {
                report.apply_pass_threshold(threshold);
                summary.add(report.clone());
                reports.push((name, report, log_path));
            }
            Err(e) => errors.push((name, e)),
        }
    }

    println!();

    if verbose && !reports.is_empty() {
        println!("Verbose logs:");
        reports.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, _, log_path) in &reports {
            if let Some(path) = log_path {
                if path.exists() {
                    println!("  {} → {}", name, path.display());
                }
            }
        }
        println!();
    }

    for (name, e) in errors {
        eprintln!("Error running scenario {}: {}", name, e);
    }

    Ok(summary)
}

/// Simple parallel execution without progress bars (for JSON output or quiet mode).
async fn run_parallel_simple(
    scenarios: Vec<Box<dyn Scenario>>,
    log_dir: Option<Arc<PathBuf>>,
    provider: EvalProvider,
    quiet: bool,
) -> Result<EvalSummary> {
    let futures: Vec<_> = scenarios
        .into_iter()
        .map(|scenario| {
            let name = scenario.name().to_string();
            let log_file = log_dir
                .as_ref()
                .map(|dir| dir.join(format!("{}.log", name)));

            async move {
                let runner = if let Some(path) = log_file {
                    match EvalRunner::new_with_log_file_and_provider(path, provider) {
                        Ok(r) => r,
                        Err(e) => return (name, Err(e)),
                    }
                } else {
                    match EvalRunner::new_with_provider(provider) {
                        Ok(r) => r,
                        Err(e) => return (name, Err(e)),
                    }
                };
                let result = scenario.run(&runner).await;
                (name, result)
            }
        })
        .collect();

    let results = join_all(futures).await;

    let mut summary = EvalSummary::default();
    let threshold = metric_pass_threshold(provider);

    for (name, result) in results {
        match result {
            Ok(mut report) => {
                report.apply_pass_threshold(threshold);
                if !quiet {
                    println!("{}", serde_json::to_string(&report.to_json())?);
                }
                summary.add(report);
            }
            Err(e) => {
                eprintln!("Error running scenario {}: {}", name, e);
            }
        }
    }

    Ok(summary)
}
