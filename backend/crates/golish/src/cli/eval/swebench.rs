//! SWE-bench Lite runner with incremental result-saving and resume support.
//!
//! Three things make SWE-bench different from a generic benchmark, and
//! they're all the reason this lives in its own module:
//!
//! 1. Each instance is expensive (real Docker test runs); we save results
//!    to disk after every completed instance so a crash or Ctrl-C
//!    doesn't lose hours of work — see [`save_instance_result`] and the
//!    sequential-with-saving runner below.
//! 2. On startup we look at the on-disk results directory and skip any
//!    instance that's already complete, enabling resume — see
//!    [`get_completed_instances`].
//! 3. There's a `--test-only` mode that skips the agent entirely and
//!    just re-runs Docker tests against an existing workspace, useful
//!    for debugging individual instances.
//!
//! The parallel path is delegated to [`super::humaneval::run_parallel_benchmark`]
//! because parallel mode doesn't yet support incremental saving (the
//! progress-bar UI assumes the runner owns its own writers).

use std::path::PathBuf;

use anyhow::Result;
use golish_evals::outcome::{EvalReport, EvalSummary};
use golish_evals::runner::EvalRunner;
use golish_evals::EvalProvider;
use tracing_subscriber::EnvFilter;

use super::{color, humaneval::run_parallel_benchmark, EvalOutputOptions};

/// Helper to save an individual eval report to the results directory.
fn save_instance_result(results_dir: &std::path::Path, report: &EvalReport) -> Result<()> {
    let filename = format!("{}.json", report.scenario.replace(['/', '\\'], "_"));
    let path = results_dir.join(&filename);

    let detailed_json = report.to_detailed_json();
    let file = std::fs::File::create(&path)?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &detailed_json)?;

    Ok(())
}

/// Check which instances already have results in the directory.
fn get_completed_instances(results_dir: &std::path::Path) -> std::collections::HashSet<String> {
    let mut completed = std::collections::HashSet::new();

    if let Ok(entries) = std::fs::read_dir(results_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Some(stem) = path.file_stem() {
                    // The filename is the scenario name (with slashes replaced by underscores)
                    // For SWE-bench, the scenario name is the instance ID
                    let name = stem.to_string_lossy().to_string();
                    completed.insert(name);
                }
            }
        }
    }

    completed
}

/// Run SWE-bench scenarios sequentially with incremental result saving.
async fn run_swebench_sequential_with_saving(
    scenarios: Vec<Box<dyn golish_evals::scenarios::Scenario>>,
    verbose: bool,
    provider: EvalProvider,
    model: Option<&str>,
    results_dir: &std::path::Path,
    resume: bool,
) -> Result<EvalSummary> {
    let runner = EvalRunner::new_verbose_with_provider(verbose, provider)?
        .with_model(model.map(|s| s.to_string()));
    let mut summary = EvalSummary::default();

    // Get list of already completed instances if resuming
    let completed = if resume {
        get_completed_instances(results_dir)
    } else {
        std::collections::HashSet::new()
    };

    let total = scenarios.len();
    let mut skipped = 0;

    for (idx, scenario) in scenarios.into_iter().enumerate() {
        let name = scenario.name().to_string();

        // Skip if already completed (when resuming)
        if completed.contains(&name) {
            skipped += 1;
            eprintln!(
                "[{}/{}] Skipping {} (already completed)",
                idx + 1,
                total,
                name
            );
            continue;
        }

        eprintln!("\n[{}/{}] Running {}...", idx + 1, total, name);

        match scenario.run(&runner).await {
            Ok(report) => {
                // Save result immediately
                if let Err(e) = save_instance_result(results_dir, &report) {
                    eprintln!("  Warning: Failed to save result for {}: {}", name, e);
                } else {
                    eprintln!("  Saved result to {}.json", name);
                }

                // Show result status
                let status = if report.passed {
                    color::green("SOLVED")
                } else {
                    color::red("FAILED")
                };
                eprintln!("  Result: {} ({}ms)", status, report.duration_ms);

                summary.add(report);
            }
            Err(e) => {
                eprintln!("  Error: {:#}", e);
                // Save error result
                let error_json = serde_json::json!({
                    "scenario": name,
                    "error": format!("{:#}", e),
                    "passed": false,
                });
                let filename = format!("{}.error.json", name);
                let path = results_dir.join(&filename);
                if let Ok(file) = std::fs::File::create(&path) {
                    let _ = serde_json::to_writer_pretty(file, &error_json);
                }
            }
        }
    }

    if skipped > 0 {
        eprintln!("\nSkipped {} already-completed instances", skipped);
    }

    Ok(summary)
}

/// Run SWE-bench Lite benchmark.
///
/// # Arguments
/// * `filter` - Optional instance filter (e.g., "django__django-11133" or "0-10")
/// * `json_output` - Whether to output JSON
/// * `verbose` - Whether to show verbose output
/// * `parallel` - Whether to run scenarios in parallel
/// * `concurrency` - Maximum number of concurrent scenarios when parallel
/// * `provider` - LLM provider to use
/// * `model` - Optional model override
/// * `output_options` - Optional output configuration
/// * `workspace_dir` - Optional persistent workspace directory (for debugging)
/// * `test_only` - Skip agent, only run Docker tests (requires workspace_dir)
/// * `results_dir` - Optional directory to save per-instance detailed JSON results
#[allow(clippy::too_many_arguments)]
pub async fn run_swebench(
    filter: Option<&str>,
    json_output: bool,
    verbose: bool,
    parallel: bool,
    concurrency: usize,
    provider: EvalProvider,
    model: Option<&str>,
    output_options: Option<EvalOutputOptions>,
    workspace_dir: Option<PathBuf>,
    test_only: bool,
    results_dir: Option<PathBuf>,
) -> Result<()> {
    // Initialize tracing
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("golish=error".parse().unwrap())
                .add_directive("golish_evals=error".parse().unwrap())
                .add_directive("golish_ai=error".parse().unwrap())
                .add_directive("golish_swebench=error".parse().unwrap()),
        )
        .try_init();

    // Check Docker availability
    if !golish_swebench::check_docker().await? {
        anyhow::bail!(
            "Docker is not available. Please ensure Docker is installed and running.\n\
             SWE-bench requires Docker for test execution."
        );
    }

    // Handle test-only mode (skip agent, run Docker tests on existing workspace)
    if test_only {
        let workspace = workspace_dir.ok_or_else(|| {
            anyhow::anyhow!(
                "--test-only requires --workspace-dir to specify the workspace location"
            )
        })?;

        let instance_id = filter.ok_or_else(|| {
            anyhow::anyhow!("--test-only requires --instance to specify which instance to test")
        })?;

        println!("Running tests only (skipping agent)");
        println!("  Instance: {}", instance_id);
        println!("  Workspace: {}\n", workspace.display());

        let result = golish_swebench::run_tests_only(instance_id, &workspace).await?;

        // Print final result
        if result.is_solved() {
            println!("{}", color::green_line());
            println!("{}", color::green("  SWE-BENCH: Instance SOLVED"));
            println!("{}", color::green_line());
        } else {
            println!("{}", color::red_line());
            println!("{}", color::red("  SWE-BENCH: Instance FAILED"));
            println!("{}", color::red_line());
        }

        return Ok(());
    }

    let scenarios = golish_swebench::get_benchmark_scenarios(filter).await?;

    if scenarios.is_empty() {
        anyhow::bail!(
            "No instances found for filter '{}'",
            filter.unwrap_or("none")
        );
    }

    // Create results directory (use provided or create timestamped default)
    let results_dir = results_dir.unwrap_or_else(|| {
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".golish")
            .join("swebench-results")
            .join(timestamp.to_string())
    });
    std::fs::create_dir_all(&results_dir)?;

    // Check for existing results (for resume capability)
    let completed = get_completed_instances(&results_dir);
    let resume = !completed.is_empty();

    if !json_output {
        let (name, desc, _) = golish_swebench::benchmark_info();
        println!("Running {} benchmark ({} instances)", name, scenarios.len());
        println!("{}", desc);
        println!("Provider: {}", provider);
        println!("Results: {}", results_dir.display());
        if resume {
            println!("Resuming: {} instances already completed", completed.len());
        }
        println!();
    }

    // Determine if we should suppress normal output
    let use_new_output = output_options.is_some();
    let opts = output_options.unwrap_or(EvalOutputOptions {
        json: json_output,
        pretty: false,
        output_file: None,
        transcript: false,
    });

    // Use the new incremental saving function for SWE-bench (sequential only for now)
    // This saves results after each instance completes, so progress isn't lost on interruption
    let summary = if parallel && scenarios.len() > 1 && !resume {
        // Parallel mode doesn't support resume yet - fall back to old behavior
        // TODO: Add parallel support with incremental saving
        eprintln!("Warning: Parallel mode doesn't save results incrementally. Consider using sequential mode with --no-parallel for long runs.");
        let suppress_intermediate = use_new_output || opts.transcript;
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
        // Sequential mode with incremental saving
        run_swebench_sequential_with_saving(
            scenarios,
            verbose,
            provider,
            model,
            &results_dir,
            resume,
        )
        .await?
    };

    // Summary file is always written at the end
    let summary_path = results_dir.join("summary.json");
    let summary_file = std::fs::File::create(&summary_path)?;
    serde_json::to_writer_pretty(summary_file, &summary.to_json())?;
    eprintln!("Summary saved to: {}", summary_path.display());

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
                "  SWE-BENCH: {}/{} solved ({:.1}%)",
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
                "  SWE-BENCH: All {} instances solved (100%)",
                summary.reports.len()
            ))
        );
        println!("{}", color::green_line());
    }

    Ok(())
}
