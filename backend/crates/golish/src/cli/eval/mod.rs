//! CLI evaluation runners.
//!
//! Originally a single 1500-line file, the eval CLI is now split by
//! sub-command surface so each entry point is self-contained:
//!
//! - [`single`]    — scenario-style evals (`run_evals`) and OpenAI model
//!   connectivity tests (`run_openai_model_tests`), plus their parallel /
//!   sequential runners and the transcript pretty-printer.
//! - [`humaneval`] — generic benchmark runner (`run_benchmark`) used for
//!   HumanEval and friends, plus its sequential/parallel implementations.
//!   The parallel runner is reused by `swebench`.
//! - [`swebench`]  — SWE-bench Lite runner with incremental result-saving
//!   and resume-from-disk support.
//! - [`args`]      — `--list-*` printers (scenarios / benchmarks / models).
//!
//! Shared helpers live in this module: the `color` ANSI helpers (CI-aware),
//! the [`EvalOutputOptions`] DTO threaded through every runner, and
//! [`metric_pass_threshold`] which decides whether per-metric results
//! count as passing for a given provider.
//!
//! Public CLI entry points are re-exported here so callers continue to
//! reach them at `crate::cli::eval::*`.

mod args;
mod humaneval;
mod single;
mod swebench;

use std::path::PathBuf;

use golish_evals::EvalProvider;

pub use args::{list_benchmark_options, list_openai_model_scenarios, list_scenarios};
pub use humaneval::run_benchmark;
pub use single::{run_evals, run_openai_model_tests};
pub use swebench::run_swebench;

/// Color helpers that respect CI environment.
///
/// In CI, ANSI escape codes are stripped for cleaner logs; locally we keep
/// them so the runner's output is scannable at a glance.  Kept private to
/// the eval submodules — nothing outside this tree should be drawing
/// summary boxes.
pub(super) mod color {
    use std::sync::OnceLock;

    static IS_CI: OnceLock<bool> = OnceLock::new();

    fn is_ci() -> bool {
        *IS_CI.get_or_init(|| std::env::var("CI").map(|v| v == "true").unwrap_or(false))
    }

    pub fn red(s: &str) -> String {
        if is_ci() {
            s.to_string()
        } else {
            format!("\x1b[31m{}\x1b[0m", s)
        }
    }

    pub fn green(s: &str) -> String {
        if is_ci() {
            s.to_string()
        } else {
            format!("\x1b[32m{}\x1b[0m", s)
        }
    }

    pub fn yellow(s: &str) -> String {
        if is_ci() {
            s.to_string()
        } else {
            format!("\x1b[33m{}\x1b[0m", s)
        }
    }

    pub fn cyan(s: &str) -> String {
        if is_ci() {
            s.to_string()
        } else {
            format!("\x1b[36m{}\x1b[0m", s)
        }
    }

    pub fn red_line() -> &'static str {
        if is_ci() {
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        } else {
            "\x1b[31m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m"
        }
    }

    pub fn green_line() -> &'static str {
        if is_ci() {
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        } else {
            "\x1b[32m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m"
        }
    }

    pub fn check_mark() -> &'static str {
        if is_ci() {
            "[PASS]"
        } else {
            "\x1b[32m✓\x1b[0m"
        }
    }

    pub fn x_mark() -> &'static str {
        if is_ci() {
            "[FAIL]"
        } else {
            "\x1b[31m✗\x1b[0m"
        }
    }
}

/// Options for eval output, shared between the single-scenario and
/// benchmark runners.
pub struct EvalOutputOptions {
    /// Output JSON to stdout.
    pub json: bool,
    /// Pretty print CI-friendly summary.
    pub pretty: bool,
    /// Save JSON results to a file.
    pub output_file: Option<PathBuf>,
    /// Print the full agent transcript before results.
    pub transcript: bool,
}

/// Get the metric pass threshold for a provider.
///
/// Z.AI uses 80% threshold (it's a smaller model and we accept some
/// flakiness), every other provider requires 100%.
pub(super) fn metric_pass_threshold(provider: EvalProvider) -> f64 {
    match provider {
        EvalProvider::Zai => 0.80,
        _ => 1.0,
    }
}
