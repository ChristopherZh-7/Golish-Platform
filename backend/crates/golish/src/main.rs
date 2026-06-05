// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Golish - AI-powered terminal emulator
//!
//! This is the unified entry point for both GUI and CLI modes:
//! - `golish` or `golish [path]` - Launches the Tauri GUI application
//! - `golish --headless [options]` - Runs in headless CLI mode
//! - `golish -e "prompt"` - Executes a single prompt (implies --headless)
//!
//! # Examples
//!
//! ```bash
//! # Launch GUI (default)
//! golish
//!
//! # Launch GUI in a specific directory
//! golish ~/Code/my-project
//!
//! # Headless mode: interactive REPL
//! golish --headless
//!
//! # Headless mode: execute a single prompt
//! golish -e "What files are in this directory?"
//!
//! # Headless mode: with auto-approval for testing
//! golish -e "Read Cargo.toml" --auto-approve
//! ```

use clap::Parser;

use golish_lib::cli::Args;

fn main() {
    // The agent runs a deep, non-recursive async future tree (orchestrator →
    // subtask → agentic loop → memory gatekeeper → LLM provider). In debug builds
    // those frames overflow tokio's default 2 MiB worker stack — confirmed via
    // lldb: EXC_BAD_ACCESS on a `tokio-rt-worker` at `one_shot_completion`'s
    // prologue. tokio worker/blocking threads inherit std's default stack size,
    // which honors `RUST_MIN_STACK`, and nothing here sets `thread_stack_size`, so
    // raising this floor before any runtime/thread starts fixes both GUI and CLI.
    if std::env::var_os("RUST_MIN_STACK").is_none() {
        std::env::set_var("RUST_MIN_STACK", "33554432"); // 32 MiB
    }

    // Install the default rustls CryptoProvider before any TLS usage.
    // Required since rustls 0.23 no longer auto-selects a backend.
    rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
        .expect("Failed to install default CryptoProvider");

    // Parse CLI arguments to determine mode
    let args = Args::parse();

    // Observability (design 2026-06-05): `golish --replay <session>` prints the
    // merged decision timeline for a run and exits. It only reads transcripts
    // from disk, so it must short-circuit before any GUI/CLI app bootstrap.
    if let Some(session) = args.replay.as_deref() {
        let base = golish_events::op_trace::default_transcript_base();
        let _ = golish_events::op_trace::write_trace_artifacts(&base, session);
        print!(
            "{}",
            golish_events::op_trace::render_timeline(&base, session)
        );
        return;
    }

    // Determine if we should run in headless mode:
    // - Explicit --headless flag
    // - Or -e (execute) or -f (file) flags imply headless
    let is_headless = args.headless || args.execute.is_some() || args.file.is_some();

    if is_headless {
        // Run in headless CLI mode
        run_cli(args);
    } else {
        // Run in GUI mode
        // Pass workspace path to GUI if provided and not the default "."
        if args.workspace.to_string_lossy() != "." {
            std::env::set_var("QBIT_WORKSPACE", &args.workspace);
        }
        golish_lib::run_gui();
    }
}

/// Run in headless CLI mode
fn run_cli(args: Args) {
    // Build a new tokio runtime for CLI mode
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    runtime.block_on(async move {
        if let Err(e) = run_cli_async(args).await {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    });
}

async fn run_cli_async(args: Args) -> anyhow::Result<()> {
    use golish_lib::cli::{execute_batch, execute_once, initialize, run_repl};

    // Initialize the full Golish stack
    let mut ctx = initialize(&args).await?;

    // Execute based on mode
    let result = if let Some(ref prompt) = args.execute {
        // Single prompt execution mode
        execute_once(&mut ctx, prompt).await
    } else if let Some(ref file) = args.file {
        // Batch file execution mode
        execute_batch(&mut ctx, file).await
    } else {
        // No prompt provided - enter interactive REPL mode
        run_repl(&mut ctx).await
    };

    // Graceful shutdown
    ctx.shutdown().await?;

    result
}
