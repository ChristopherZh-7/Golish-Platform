//! CLI argument parsing using clap.
//!
//! Defines the command-line interface for golish in headless mode.

use clap::Parser;
use std::path::PathBuf;

/// Golish - AI-powered terminal emulator
///
/// By default, runs as a GUI application. Use --headless for CLI mode.
#[derive(Parser, Debug, Clone)]
#[command(name = "golish")]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Run in headless CLI mode (no GUI)
    #[arg(long)]
    pub headless: bool,

    /// Working directory (default: current directory)
    #[arg(default_value = ".")]
    pub workspace: PathBuf,

    /// Execute a single prompt and exit (implies --headless)
    #[arg(short = 'e', long, conflicts_with = "file")]
    pub execute: Option<String>,

    /// Execute prompts from a file (one per line) and exit
    #[arg(short = 'f', long, conflicts_with = "execute")]
    pub file: Option<PathBuf>,

    /// Override AI provider from settings
    ///
    /// Options: vertex_ai, vertex_gemini, openrouter, anthropic, openai,
    /// ollama, gemini, groq, xai, zai_sdk, nvidia, deepseek, xiaomi
    #[arg(short = 'p', long)]
    pub provider: Option<String>,

    /// Override model from settings
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// API key (overrides settings and env vars)
    #[arg(long, env = "QBIT_API_KEY")]
    pub api_key: Option<String>,

    /// Auto-approve tool calls and resolve only supported typed harness prompts
    /// from explicit CLI policy (DANGEROUS: for testing only)
    #[arg(long)]
    pub auto_approve: bool,

    /// Compatibility switch for a profile-defined generic phase confirmation.
    /// Built-in flows require routine confirmation only in Scoping and do not
    /// use this after Scoping. It never approves target, Candidate, or tool authz;
    /// `--auto-approve` is still required to deliver a compatible phase decision.
    #[arg(long, requires = "auto_approve")]
    pub approve_phase_boundaries: bool,

    /// Output events as JSON lines (for scripting/parsing)
    #[arg(long)]
    pub json: bool,

    /// Only output final response (suppress streaming)
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Show verbose output (debug information)
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Replay a run's merged decision timeline (main agent + sub-agents) and
    /// exit. Pass the chat-session id (the directory holding `transcript.json`).
    /// The base is resolved like the app writes it: `VT_TRANSCRIPT_DIR`, else the
    /// `[WORKSPACE]` arg / current dir's `.golish/transcripts`, else
    /// `~/.golish/transcripts` — so run it from (or pass) the workspace the run
    /// used. Reads existing transcripts only — no app startup, no DB. See
    /// docs/design/2026-06-05-unified-ai-harness-observability.
    #[arg(long, value_name = "SESSION")]
    pub replay: Option<String>,

    // ---- 方案 2 · headless single/range stage runner ----
    /// Run a single harness stage (or a `--from`..=`--to` slice) headlessly:
    /// boot embedded PG + real pentest tools + real LLM, run the slice, print a
    /// structured report (gate PASS/BLOCK + tools + evidence), then exit. No GUI.
    /// See docs/design/2026-06-06-headless-single-stage-runner.md.
    #[arg(long)]
    pub stage_run: bool,

    /// Create a new test operation from one exact GUI/CLI source operation,
    /// adopt its validated post-Scoping prefix, and execute only the requested
    /// stage slice. The source operation remains immutable.
    #[arg(
        long,
        value_name = "SESSION_OR_OPERATION",
        conflicts_with_all = [
            "stage_run",
            "stage_run_resume",
            "resume_to",
            "ephemeral_db",
            "keep_ephemeral_db",
            "profile",
            "org",
            "target",
            "include_subsidiaries",
            "subsidiary_threshold",
            "allow_orphan_running",
            "repair_missing_graph_flow",
            "repair_reaped_task"
        ]
    )]
    pub stage_run_fork: Option<String>,

    /// Resume one exact interrupted headless stage-run. Accepts its
    /// `stage-run-*` chat key, DB session UUID, or task/operation UUID. This is
    /// not a fresh run: it reuses the original session, operation, freshness
    /// epoch, and persisted specialist chain.
    #[arg(
        long,
        value_name = "SESSION_OR_OPERATION",
        conflicts_with_all = [
            "stage_run",
            "ephemeral_db",
            "keep_ephemeral_db",
            "profile",
            "from",
            "to",
            "only",
            "org",
            "target",
            "include_subsidiaries"
        ]
    )]
    pub stage_run_resume: Option<String>,

    /// Optional inclusive terminal stage for an exact resume. When omitted,
    /// resume remains limited to the persisted current stage. This never
    /// changes the frozen profile or scope.
    #[arg(long, value_name = "STAGE", requires = "stage_run_resume")]
    pub resume_to: Option<String>,

    /// Backward-compatible exact-identity assertion for a `running` task.
    /// Durable resume no longer downgrades it to `waiting`; the shared open-Turn
    /// claim fences the continuation directly.
    #[arg(long, requires = "stage_run_resume")]
    pub allow_orphan_running: bool,

    /// Explicitly permit synthesizing the missing `graph_flow` wrapper around a
    /// valid flat first-stage checkpoint. Requires exact expected identities.
    #[arg(long, requires = "stage_run_resume")]
    pub repair_missing_graph_flow: bool,

    /// Explicitly permit restoring a task that the legacy startup reaper marked
    /// failed with its exact abandoned-task marker. Requires exact expected
    /// identities and never applies to an ordinary failed task.
    #[arg(long, requires = "stage_run_resume")]
    pub repair_reaped_task: bool,

    /// Internal E2E recovery hook: in an explicitly selected `golish_gatefix_*`
    /// database or an explicitly retained ephemeral pgdata directory, replace
    /// an exhausted current V2 stage runtime without purging operation facts.
    /// Complete expected identities are mandatory. This is not a
    /// production/operator recovery switch.
    #[arg(
        long,
        requires_all = [
            "stage_run_resume",
            "expect_session",
            "expect_task",
            "expect_operation",
            "expect_org",
            "expect_stage"
        ],
        hide = true
    )]
    pub stage_run_test_restart_exhausted_stage: bool,

    /// Expected DB `sessions.id` for an exact resume.
    #[arg(long, value_name = "UUID", requires = "stage_run_resume")]
    pub expect_session: Option<uuid::Uuid>,

    /// Expected `tasks.id` for an exact resume.
    #[arg(long, value_name = "UUID", requires = "stage_run_resume")]
    pub expect_task: Option<uuid::Uuid>,

    /// Expected `operation_state.operation_id` for an exact resume.
    #[arg(long, value_name = "UUID", requires = "stage_run_resume")]
    pub expect_operation: Option<uuid::Uuid>,

    /// Expected engagement root organization for an exact resume.
    #[arg(long, value_name = "UUID", requires = "stage_run_resume")]
    pub expect_org: Option<uuid::Uuid>,

    /// Expected current harness stage for an exact resume.
    #[arg(long, value_name = "STAGE", requires = "stage_run_resume")]
    pub expect_stage: Option<String>,

    /// Re-open one retained `--keep-ephemeral-db` pgdata directory for an
    /// exact resume. A fresh random port is used and the ordinary application
    /// database is never touched.
    #[arg(
        long,
        value_name = "PGDATA",
        requires = "stage_run_resume",
        conflicts_with = "ephemeral_db"
    )]
    pub stage_run_resume_pgdata: Option<PathBuf>,

    /// Apply one explicit, hash/CAS-bound local-operator Campaign authority
    /// packet before an exact Investigation resume. This is accepted only for
    /// a retained ephemeral PostgreSQL directory and never derives authority
    /// from `--auto-approve`.
    #[arg(
        long,
        value_name = "JSON",
        requires_all = [
            "stage_run_resume",
            "stage_run_resume_pgdata",
            "expect_session",
            "expect_task",
            "expect_operation",
            "expect_org",
            "expect_stage"
        ],
        conflicts_with = "stage_run_test_database"
    )]
    pub stage_run_campaign_authority: Option<PathBuf>,

    /// Apply one explicit operation/org/exact-candidate scope decision while
    /// crossing Target Intel into active reconnaissance on an exact retained
    /// resume. `--auto-approve` alone never grants this authority.
    #[arg(
        long,
        value_name = "JSON",
        requires_all = [
            "stage_run_resume",
            "stage_run_resume_pgdata",
            "expect_session",
            "expect_task",
            "expect_operation",
            "expect_org",
            "expect_stage"
        ],
        conflicts_with = "stage_run_test_database"
    )]
    pub stage_run_active_recon_scope_authority: Option<PathBuf>,

    /// `--stage-run` test mode: use an isolated temporary embedded Postgres data
    /// directory and a random local port. The normal app DB is not touched.
    #[arg(long)]
    pub ephemeral_db: bool,

    /// Internal E2E hook: select a unified Investigation deployment default in
    /// a brand-new ephemeral database before the first operation is created.
    /// Rank 5 keeps the legacy read projection; rank 6 is new-only. This can
    /// never target the user's persistent database.
    #[arg(
        long,
        value_name = "JOINT_RANK",
        value_parser = clap::value_parser!(i16).range(5..=6),
        requires_all = ["stage_run", "ephemeral_db"],
        hide = true
    )]
    pub stage_run_test_joint_rank: Option<i16>,

    /// Internal E2E hook: preserve a previously resolved organization UUID
    /// while seeding a brand-new ephemeral database. The organization name is
    /// still required and the persistent user database can never be targeted.
    #[arg(
        long,
        value_name = "UUID",
        requires_all = ["stage_run", "ephemeral_db", "org"],
        hide = true
    )]
    pub stage_run_test_organization_id: Option<uuid::Uuid>,

    /// Internal controlled-fixture hook: replace the ordinary tools-config
    /// directory for one fresh ephemeral stage-run. The paired intel-provider
    /// directory is mandatory so a fixture cannot accidentally retain a real
    /// passive provider from either source.
    #[arg(
        long,
        value_name = "DIR",
        requires_all = [
            "stage_run",
            "ephemeral_db",
            "stage_run_test_intel_providers_dir",
            "stage_run_test_intel_provider_endpoint"
        ],
        hide = true
    )]
    pub stage_run_test_toolsconfig_dir: Option<PathBuf>,

    /// Internal controlled-fixture hook paired with
    /// `--stage-run-test-toolsconfig-dir`. Both overrides are accepted only for
    /// a fresh ephemeral database and never affect the normal application.
    #[arg(
        long,
        value_name = "DIR",
        requires_all = [
            "stage_run",
            "ephemeral_db",
            "stage_run_test_toolsconfig_dir",
            "stage_run_test_intel_provider_endpoint"
        ],
        hide = true
    )]
    pub stage_run_test_intel_providers_dir: Option<PathBuf>,

    /// Exact local HTTP endpoint admitted only for the paired controlled
    /// provider directories in a fresh ephemeral stage-run.
    #[arg(
        long,
        value_name = "URL",
        requires_all = [
            "stage_run",
            "ephemeral_db",
            "stage_run_test_toolsconfig_dir",
            "stage_run_test_intel_providers_dir"
        ],
        hide = true
    )]
    pub stage_run_test_intel_provider_endpoint: Option<url::Url>,

    /// Keep the temporary Postgres data directory after `--ephemeral-db` exits.
    /// Useful when a failed smoke run needs manual database inspection.
    #[arg(long, requires = "ephemeral_db")]
    pub keep_ephemeral_db: bool,

    /// Internal E2E hook: run a stage command against an already-created,
    /// isolated database in the normal local PostgreSQL cluster. Names are
    /// restricted to `golish_gatefix_*` so this cannot silently select the
    /// production `golish` database.
    #[arg(
        long,
        value_name = "NAME",
        conflicts_with = "ephemeral_db",
        hide = true
    )]
    pub stage_run_test_database: Option<String>,

    /// Local-admin-only Plan D deployment-default promotion. Without
    /// `--plan-d-maintenance-apply` this performs a rollback-only dry run and
    /// prints the exact evidence manifest hash that apply must echo.
    #[arg(
        long,
        value_name = "JOINT_RANK",
        value_parser = clap::value_parser!(i16).range(1..=6),
        conflicts_with_all = [
            "stage_run",
            "stage_run_resume",
            "stage_run_fork",
            "execute",
            "file",
            "replay"
        ]
    )]
    pub plan_d_maintenance_target_rank: Option<i16>,

    /// Apply a previously dry-run Plan D promotion. Both deployment safety
    /// holds must already be engaged; this flag never auto-releases them.
    #[arg(
        long,
        requires = "plan_d_maintenance_target_rank",
        requires = "plan_d_maintenance_plan_hash"
    )]
    pub plan_d_maintenance_apply: bool,

    /// Exact `sha256:` evidence manifest emitted by the dry run.
    #[arg(
        long,
        value_name = "SHA256",
        requires = "plan_d_maintenance_target_rank"
    )]
    pub plan_d_maintenance_plan_hash: Option<String>,

    /// Retained operator reason written only by apply.
    #[arg(long, value_name = "TEXT", requires = "plan_d_maintenance_target_rank")]
    pub plan_d_maintenance_reason: Option<String>,

    /// Print a database truth summary before `--stage-run` shuts down embedded
    /// Postgres. This is intended for real smoke tests that must prove rows
    /// landed, not just that the agent wrote a natural-language answer.
    #[arg(long)]
    pub db_smoke_summary: bool,

    /// Harness profile id for `--stage-run` (e.g. assessment / pentest /
    /// red_team / bug_bounty / cloud_assessment / smoke). Defaults to the
    /// `GOLISH_HARNESS_PROFILE` env default when omitted.
    #[arg(long, value_name = "PROFILE")]
    pub profile: Option<String>,

    /// `--stage-run` slice start stage (defaults to the DAG entry, `scoping`).
    #[arg(long, value_name = "STAGE")]
    pub from: Option<String>,

    /// `--stage-run` slice end stage (inclusive). The run stops after this stage
    /// passes its gate. Required unless `--only` is given.
    #[arg(long, value_name = "STAGE")]
    pub to: Option<String>,

    /// `--stage-run` shorthand for `--from X --to X` (run exactly one stage).
    /// Non-`scoping` single stages need upstream seeding (`--org` / `--target`).
    #[arg(long, value_name = "STAGE", conflicts_with_all = ["from", "to"])]
    pub only: Option<String>,

    /// `--stage-run` minimal upstream seed: create/select an organization by name
    /// (needed by stages that operate on an org, e.g. `target_intel`).
    #[arg(long, value_name = "NAME")]
    pub org: Option<String>,

    /// `--stage-run` minimal upstream seed: add an in-scope target (host/domain).
    /// Repeatable: `--target a.com --target b.com`.
    #[arg(long, value_name = "HOST")]
    pub target: Vec<String>,

    /// `--stage-run` Phase 2 (2026-06-12-redteam-phase2): the engagement scope
    /// includes subsidiaries. Scoping must then build the org tree — run
    /// subsidiary discovery (ENScan), filter by the ownership threshold, and
    /// land qualifying child organizations in the DB — before its gate passes.
    /// Without this flag scoping behaves exactly as before (no subsidiary gate).
    #[arg(long)]
    pub include_subsidiaries: bool,

    /// Minimum investment/ownership percent for a subsidiary to be in scope
    /// (only meaningful with `--include-subsidiaries`). Recorded as the run's
    /// scope policy for prompts/diagnostics; the actual filter runs in the
    /// asset-intel promote layer driven by the provider's toolsconfig
    /// `promote_when` (enscan-go: scale >= 51).
    #[arg(long, value_name = "PCT", default_value_t = 51)]
    pub subsidiary_threshold: u8,
}

impl Args {
    /// Resolve the workspace path to an absolute, validated directory.
    ///
    /// Delegates to [`crate::app::workspace::resolve_validated_workspace`] so
    /// the GUI and the CLI share **one** resolution policy:
    ///
    /// 1. `QBIT_WORKSPACE` environment variable (with `~/` expansion).
    /// 2. The CLI's positional `[WORKSPACE]` argument (defaults to `.`).
    /// 3. (Validation) the path must exist and be a directory.
    ///
    /// Returns an error if the path does not exist or is not a directory.
    pub fn resolve_workspace(&self) -> anyhow::Result<PathBuf> {
        crate::app::workspace::resolve_validated_workspace(Some(&self.workspace))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_default_values() {
        let args = Args::parse_from(["golish"]);
        assert_eq!(args.workspace, PathBuf::from("."));
        assert!(!args.auto_approve);
        assert!(!args.approve_phase_boundaries);
        assert!(!args.json);
        assert!(!args.quiet);
        assert!(!args.verbose);
    }

    #[test]
    fn test_args_execute_flag() {
        let args = Args::parse_from(["golish", "-e", "Hello world"]);
        assert_eq!(args.execute, Some("Hello world".to_string()));
    }

    #[test]
    fn test_args_stage_run_accepts_explicit_phase_boundary_approval() {
        let args = Args::try_parse_from([
            "golish",
            "--stage-run",
            "--to",
            "attack_candidate",
            "--auto-approve",
            "--approve-phase-boundaries",
        ])
        .expect("explicit phase approval is valid for an automated stage slice");
        assert!(args.approve_phase_boundaries);

        assert!(Args::try_parse_from([
            "golish",
            "--stage-run",
            "--to",
            "attack_candidate",
            "--approve-phase-boundaries",
        ])
        .is_err());
    }

    #[test]
    fn test_args_provider_and_model() {
        let args = Args::parse_from([
            "golish",
            "-p",
            "openrouter",
            "-m",
            "anthropic/claude-sonnet-4",
        ]);
        assert_eq!(args.provider, Some("openrouter".to_string()));
        assert_eq!(args.model, Some("anthropic/claude-sonnet-4".to_string()));
    }

    #[test]
    fn test_args_output_modes() {
        let args = Args::parse_from(["golish", "--json", "--quiet"]);
        assert!(args.json);
        assert!(args.quiet);
    }

    #[test]
    fn test_args_auto_approve() {
        let args = Args::parse_from(["golish", "--auto-approve"]);
        assert!(args.auto_approve);
    }

    #[test]
    fn test_args_stage_run_only() {
        // 方案 2: objective passed via -e (positional is `workspace`).
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--profile",
            "red_team",
            "--only",
            "target_intel",
            "-e",
            "scan acme",
        ]);
        assert!(args.stage_run);
        assert_eq!(args.profile.as_deref(), Some("red_team"));
        assert_eq!(args.only.as_deref(), Some("target_intel"));
        assert!(args.from.is_none());
        assert!(args.to.is_none());
        assert_eq!(args.execute.as_deref(), Some("scan acme"));
    }

    #[test]
    fn test_args_stage_run_to_with_repeated_targets() {
        let args = Args::parse_from([
            "golish",
            "--stage-run",
            "--profile",
            "assessment",
            "--to",
            "target_intel",
            "--org",
            "ACME",
            "--target",
            "a.com",
            "--target",
            "b.com",
        ]);
        assert!(args.stage_run);
        assert_eq!(args.to.as_deref(), Some("target_intel"));
        assert_eq!(args.org.as_deref(), Some("ACME"));
        assert_eq!(args.target, vec!["a.com".to_string(), "b.com".to_string()]);
    }

    #[test]
    fn test_args_stage_run_resume_with_orphan_identity_assertions() {
        let args = Args::parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--allow-orphan-running",
            "--repair-missing-graph-flow",
            "--repair-reaped-task",
            "--expect-session",
            "a15c0b0f-23ff-42f9-b950-7dcaf25de860",
            "--expect-task",
            "462b6c9f-2a0d-48af-8ff0-8b5c08416196",
            "--expect-operation",
            "462b6c9f-2a0d-48af-8ff0-8b5c08416196",
            "--expect-org",
            "0a431390-7726-48e5-b0a8-e692a9070e33",
            "--expect-stage",
            "enumeration",
            "-e",
            "继续",
            "/tmp/original-workspace",
        ]);

        assert_eq!(
            args.stage_run_resume.as_deref(),
            Some("stage-run-476558c3-c22a-4009-a82e-17e086a005de")
        );
        assert!(args.allow_orphan_running);
        assert!(args.repair_missing_graph_flow);
        assert!(args.repair_reaped_task);
        assert_eq!(
            args.expect_session,
            Some(
                uuid::Uuid::parse_str("a15c0b0f-23ff-42f9-b950-7dcaf25de860")
                    .expect("session uuid")
            )
        );
        assert_eq!(
            args.expect_task,
            Some(uuid::Uuid::parse_str("462b6c9f-2a0d-48af-8ff0-8b5c08416196").expect("task uuid"))
        );
        assert_eq!(
            args.expect_operation,
            Some(
                uuid::Uuid::parse_str("462b6c9f-2a0d-48af-8ff0-8b5c08416196")
                    .expect("operation uuid")
            )
        );
        assert_eq!(args.expect_stage.as_deref(), Some("enumeration"));
        assert_eq!(
            args.expect_org,
            Some(uuid::Uuid::parse_str("0a431390-7726-48e5-b0a8-e692a9070e33").expect("org uuid"))
        );
        assert_eq!(args.execute.as_deref(), Some("继续"));
        assert_eq!(args.workspace, PathBuf::from("/tmp/original-workspace"));
    }

    #[test]
    fn test_args_stage_run_resume_accepts_explicit_terminal_stage() {
        let args = Args::try_parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--resume-to",
            "attack_candidate",
        ])
        .expect("resume should accept an explicit continuation boundary");

        assert_eq!(
            args.stage_run_resume.as_deref(),
            Some("stage-run-476558c3-c22a-4009-a82e-17e086a005de")
        );
        assert_eq!(args.resume_to.as_deref(), Some("attack_candidate"));
        assert!(Args::try_parse_from(["golish", "--resume-to", "attack_candidate"]).is_err());
    }

    #[test]
    fn test_args_stage_run_resume_accepts_retained_pgdata() {
        let args = Args::try_parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--stage-run-resume-pgdata",
            "/tmp/golish-stage-run-db-retained/pgdata",
        ])
        .expect("resume should accept one retained ephemeral pgdata directory");

        assert_eq!(
            args.stage_run_resume_pgdata,
            Some(PathBuf::from("/tmp/golish-stage-run-db-retained/pgdata"))
        );
        assert!(Args::try_parse_from([
            "golish",
            "--stage-run-resume-pgdata",
            "/tmp/golish-stage-run-db-retained/pgdata"
        ])
        .is_err());
    }

    #[test]
    fn test_args_campaign_authority_requires_exact_retained_resume_identity() {
        let args = Args::try_parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--stage-run-resume-pgdata",
            "/tmp/golish-stage-run-db-retained/pgdata",
            "--stage-run-campaign-authority",
            "/tmp/campaign-authority.json",
            "--expect-session",
            "a15c0b0f-23ff-42f9-b950-7dcaf25de860",
            "--expect-task",
            "462b6c9f-2a0d-48af-8ff0-8b5c08416196",
            "--expect-operation",
            "462b6c9f-2a0d-48af-8ff0-8b5c08416196",
            "--expect-org",
            "0a431390-7726-48e5-b0a8-e692a9070e33",
            "--expect-stage",
            "investigation",
        ])
        .expect("exact retained resume accepts an explicit Campaign authority packet");
        assert_eq!(
            args.stage_run_campaign_authority,
            Some(PathBuf::from("/tmp/campaign-authority.json"))
        );
        assert!(!args.auto_approve);

        assert!(Args::try_parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--stage-run-campaign-authority",
            "/tmp/campaign-authority.json",
        ])
        .is_err());
    }

    #[test]
    fn test_args_retained_pgdata_restart_requires_complete_identity() {
        let complete = Args::try_parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--stage-run-resume-pgdata",
            "/tmp/golish-stage-run-db-retained/pgdata",
            "--stage-run-test-restart-exhausted-stage",
            "--expect-session",
            "a15c0b0f-23ff-42f9-b950-7dcaf25de860",
            "--expect-task",
            "462b6c9f-2a0d-48af-8ff0-8b5c08416196",
            "--expect-operation",
            "462b6c9f-2a0d-48af-8ff0-8b5c08416196",
            "--expect-org",
            "0a431390-7726-48e5-b0a8-e692a9070e33",
            "--expect-stage",
            "application_understanding",
        ])
        .expect("retained exhausted-stage restart should accept complete identities");
        assert!(complete.stage_run_test_restart_exhausted_stage);

        assert!(Args::try_parse_from([
            "golish",
            "--stage-run-resume",
            "stage-run-476558c3-c22a-4009-a82e-17e086a005de",
            "--stage-run-resume-pgdata",
            "/tmp/golish-stage-run-db-retained/pgdata",
            "--stage-run-test-restart-exhausted-stage",
        ])
        .is_err());
    }

    #[test]
    fn test_args_stage_run_resume_rejects_fresh_run_and_ephemeral_selectors() {
        for conflicting in [
            vec!["--stage-run"],
            vec!["--only", "enumeration"],
            vec!["--ephemeral-db"],
        ] {
            let mut argv = vec!["golish", "--stage-run-resume", "stage-run-abc"];
            argv.extend(conflicting);
            assert!(
                Args::try_parse_from(argv).is_err(),
                "resume must reject fresh/ephemeral selector"
            );
        }
    }

    #[test]
    fn test_args_stage_run_fork_accepts_only_or_complete_range() {
        let only = Args::try_parse_from([
            "golish",
            "--stage-run-fork",
            "pentest-chat-1784364775375-1",
            "--only",
            "vuln_triage",
        ])
        .expect("fork should accept one selected post-Scoping stage");
        assert_eq!(only.only.as_deref(), Some("vuln_triage"));

        let range = Args::try_parse_from([
            "golish",
            "--stage-run-fork",
            "425c7693-99fb-4598-8361-62275c9413b1",
            "--from",
            "enumeration",
            "--to",
            "attack_candidate",
        ])
        .expect("fork should accept one explicit contiguous stage range");
        assert_eq!(range.from.as_deref(), Some("enumeration"));
        assert_eq!(range.to.as_deref(), Some("attack_candidate"));
    }

    #[test]
    fn test_args_stage_run_fork_rejects_fresh_resume_and_scope_overrides() {
        for conflicting in [
            vec!["--stage-run"],
            vec!["--stage-run-resume", "stage-run-abc"],
            vec!["--ephemeral-db"],
            vec!["--profile", "pentest"],
            vec!["--org", "ACME"],
            vec!["--target", "a.example"],
            vec!["--include-subsidiaries"],
            vec!["--subsidiary-threshold", "60"],
        ] {
            let mut argv = vec![
                "golish",
                "--stage-run-fork",
                "425c7693-99fb-4598-8361-62275c9413b1",
                "--only",
                "enumeration",
            ];
            argv.extend(conflicting);
            assert!(
                Args::try_parse_from(argv).is_err(),
                "fork must reject fresh/resume/scope overrides"
            );
        }
    }

    #[test]
    fn test_args_orphan_assertions_require_stage_run_resume() {
        assert!(
            Args::try_parse_from(["golish", "--allow-orphan-running"]).is_err(),
            "orphan override must be scoped to the explicit resume command"
        );
        assert!(
            Args::try_parse_from(["golish", "--repair-reaped-task"]).is_err(),
            "reaped-task repair must be scoped to the explicit resume command"
        );
    }

    #[test]
    fn test_args_only_conflicts_with_to() {
        // --only is mutually exclusive with --from/--to.
        let res = Args::try_parse_from([
            "golish",
            "--stage-run",
            "--only",
            "scoping",
            "--to",
            "reporting",
        ]);
        assert!(res.is_err(), "--only with --to must be rejected by clap");
    }
}
