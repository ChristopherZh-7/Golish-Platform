# Golish - Tauri Terminal App
# Run `just` to see all available commands

# Default recipe - show help
default:
    @just --list

# ============================================
# Development
# ============================================

# Start development server (frontend + backend)
# Usage: just dev [path]
# Example: just dev ~/Code/my-project
dev path="":
    {{ if path == "" { "pnpm tauri dev" } else { "pnpm tauri dev -- -- " + path } }}

# Start only the frontend dev server
dev-fe:
    pnpm dev

# Replay a run's merged decision timeline (main agent + sub-agents) for debugging.
# Pass the chat-session id (the dir name under ~/.golish/transcripts). Reads
# transcripts only — no app/DB startup. See
# docs/design/2026-06-05-unified-ai-harness-observability.md
# Usage: just replay <session-id>
replay session:
    cd backend && cargo run -q -p golish --bin golish -- --replay {{ session }}

# Headless single-stage runner (方案 2): boot the backend without the GUI, run one
# harness stage — or the scoping..=<to> slice — with a real LLM, print a structured
# report (gate PASS/BLOCK + tools + evidence), and exit. Auto-approves scoping HITL.
# Full transcript is written for `just replay` / GUI viewing. Needs an LLM key in
# ~/.golish/settings.toml. See docs/design/2026-06-06-headless-single-stage-runner.md
# Usage: just stage <profile> <to-stage> "<objective>"
# Example: just stage red_team target_intel "recon acme.com"
stage profile to objective:
    cd backend && cargo run -q -p golish --bin golish -- --stage-run --profile {{ profile }} --to {{ to }} --auto-approve -e "{{ objective }}"

# Real stage smoke test with an isolated temporary embedded DB and a local HTTP
# fixture target. Prints db_smoke_summary before the runner shuts Postgres down.
# Usage: just stage-smoke <profile> <to-stage> "<objective>"
# Example: just stage-smoke assessment target_intel "smoke target_intel"
stage-smoke profile to objective:
    python3 scripts/stage_smoke.py --fixture-web --profile {{ profile }} --to {{ to }} --objective "{{ objective }}"

# ============================================
# Testing
# ============================================

# Run all tests (frontend + backend)
test: test-fe test-rust

# Run frontend tests (quiet - only shows failures)
test-fe:
    #!/usr/bin/env bash
    if output=$(pnpm --silent test:run -- --reporter=dot --silent 2>&1); then
        :
    else
        echo "$output"
        exit 1
    fi

# Run frontend tests in watch mode
test-watch:
    pnpm test

# Run frontend tests with UI
test-ui:
    pnpm test:ui

# Run frontend tests with coverage
test-coverage:
    pnpm test:coverage

# Run e2e tests (Playwright)
test-e2e *args:
    pnpm exec playwright test {{args}}

# Run Rust tests (quiet - only shows failures)
test-rust:
    #!/usr/bin/env bash
    if output=$(cd backend && cargo nextest run --status-level fail 2>&1); then
        :
    else
        echo "$output"
        exit 1
    fi

# Run all Rust tests including the Tauri app crate (for CI/quality gate)
test-rust-all:
    #!/usr/bin/env bash
    if output=$(cd backend && cargo nextest run --workspace --status-level fail 2>&1); then
        :
    else
        echo "$output"
        exit 1
    fi

# Run Rust tests with output
test-rust-verbose:
    cd backend && cargo nextest run --status-level all

# Fast, deterministic harness/gate closed-loop tests (NO live LLM, NO UI run).
# Use this instead of running a full pentest in the UI to verify the stage gate
# + submit_stage_deliverable wiring is correct. Covers (golish-agent-kit +
# golish-agent-runtime): every harness gate check, the stage-transition
# closed-loop driver (in-memory operation_state), and the execution-mode tool
# exposure (submit_stage_deliverable surfacing in task mode, not chat). Runs in
# ~1s after compile.
test-harness:
    #!/usr/bin/env bash
    set -euo pipefail
    cd backend && cargo nextest run \
        -p golish-agent-kit -p golish-agent-runtime \
        -E '(package(golish-agent-kit) & test(harness)) | (package(golish-agent-runtime) & (test(execution_mode) | test(tool_list)))' \
        --status-level fail

# Same as `test-harness` but also includes the submit_stage_deliverable tool
# HANDLER tests in golish-agent-app (parse/accept/reject/needs_fix). Slower on
# first run because it compiles the heavier app crate; prefer `test-harness` for
# the tight inner loop.
test-harness-full: test-harness
    #!/usr/bin/env bash
    set -euo pipefail
    cd backend && cargo nextest run \
        -p golish-agent-app \
        -E 'package(golish-agent-app) & test(harness_submit_tool)' \
        --status-level fail

# ============================================
# Building
# ============================================

# Build for production
build:
    cd backend && cargo build -p golish --release
    pnpm tauri build

# Build frontend only
build-fe:
    pnpm build

# Build Rust backend only (debug)
build-rust:
    cd backend && cargo build

# Build Rust backend (release)
build-rust-release:
    cd backend && cargo build --release

# ============================================
# Code Quality
# ============================================

# Run all checks (format, lint, typecheck, tests)
check:
    @just step fmt
    @just step check-fe
    @just step test-fe
    @just step lint-rust
    @just step test-rust-all
    @just step check-types
    @printf '\033[1;32m━━━ OK ━━━\033[0m\n'

[private]
step recipe:
    #!/usr/bin/env bash
    set -euo pipefail
    printf '\033[1;36m━━━ %s ━━━\033[0m\n' "{{recipe}}"
    start=$SECONDS
    just {{recipe}}
    elapsed=$((SECONDS - start))
    printf '\033[1;32m✓ passed\033[0m \033[2m(%ds)\033[0m\n' "$elapsed"

# Check frontend (biome + typecheck)
check-fe:
    @pnpm --silent check > /dev/null
    @pnpm --silent typecheck

# Fast Rust check (type check + fmt check)
check-rust:
    @cd backend && cargo check -q
    @cd backend && cargo fmt --check

# Regenerate cross-IPC TypeScript bindings from Rust (ts-rs). Types annotated
# with #[ts(export)] are written to frontend/lib/generated/ as a side effect of
# running their auto-generated `export_bindings_*` tests. See docs/design/2026-05-29-architecture-optimization.md §4.2.
gen-types:
    @cd backend && cargo test --workspace export_bindings -q

# Fail if the committed ts-rs bindings drift from the Rust source of truth (I5).
check-types: gen-types
    @git diff --exit-code -- frontend/lib/generated/

# Lint Rust (clippy + fmt check — all workspace crates)
lint-rust:
    @cd backend && cargo clippy --workspace -q -- -D warnings
    @cd backend && cargo fmt --check

# Fix frontend issues (biome)
fix:
    pnpm check:fix

# Format all code
fmt: fmt-fe fmt-rust

# Format frontend
fmt-fe:
    @pnpm --silent format > /dev/null

# Format Rust
fmt-rust:
    @cd backend && cargo fmt

# Lint frontend
lint:
    pnpm lint

# Lint and fix frontend
lint-fix:
    pnpm lint:fix

# ============================================
# Cleaning
# ============================================

# Clean all build artifacts
clean: clean-fe clean-rust

# Clean frontend
clean-fe:
    rm -rf dist node_modules/.vite

# Clean Rust
clean-rust:
    cd backend && cargo clean

# Reclaim stale target/ space via cargo-sweep: drop old-toolchain + >N-day artifacts (default 14).
clean-stale days="14":
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-sweep >/dev/null 2>&1; then echo "cargo-sweep not installed — run: cargo install cargo-sweep"; exit 1; fi
    if [ ! -d backend/target ]; then echo "backend/target/ absent — nothing to sweep."; exit 0; fi
    cd backend && cargo sweep --installed && cargo sweep --time {{days}}

# Hard-cap target/ size via cargo-sweep: delete oldest artifacts until under SIZE (default 30GB).
clean-cap size="30GB":
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-sweep >/dev/null 2>&1; then echo "cargo-sweep not installed — run: cargo install cargo-sweep"; exit 1; fi
    if [ ! -d backend/target ]; then echo "backend/target/ absent — nothing to sweep."; exit 0; fi
    cd backend && cargo sweep --maxsize {{size}}

# Reclaim only stale Rust artifacts when the data volume is low. Safe to call
# before any AI-driven Cargo command; it is a no-op while Cargo/rustc is active.
space-guard:
    ./scripts/cargo_space_guard.sh

# Install a local macOS watchdog that checks the space guard every 10 minutes.
# The watchdog never sweeps while Cargo/rustc/nextest is active.
space-guard-install:
    #!/usr/bin/env bash
    set -euo pipefail
    repo="$(pwd)"
    plist="$HOME/Library/LaunchAgents/com.golish.cargo-space-guard.plist"
    mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.golish"
    /usr/bin/python3 - "$repo" "$plist" <<'PY'
    import plistlib
    import sys
    from pathlib import Path

    repo, destination = sys.argv[1:]
    payload = {
        "Label": "com.golish.cargo-space-guard",
        "ProgramArguments": [str(Path(repo) / "scripts/cargo_space_guard.sh")],
        "RunAtLoad": True,
        "StartInterval": 600,
        "StandardOutPath": str(Path.home() / ".golish/cargo-space-guard.log"),
        "StandardErrorPath": str(Path.home() / ".golish/cargo-space-guard.log"),
        "EnvironmentVariables": {
            "PATH": f"{Path.home()}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
            "GOLISH_MIN_FREE_GB": "80",
            "GOLISH_TARGET_CAP": "30GB",
        },
    }
    with open(destination, "wb") as handle:
        plistlib.dump(payload, handle)
    PY
    launchctl bootout "gui/$(id -u)/com.golish.cargo-space-guard" >/dev/null 2>&1 || true
    launchctl bootstrap "gui/$(id -u)" "$plist"
    echo "✓ Cargo space guard installed (80GB free-space floor, checked every 10 minutes)."

space-guard-uninstall:
    #!/usr/bin/env bash
    set -euo pipefail
    plist="$HOME/Library/LaunchAgents/com.golish.cargo-space-guard.plist"
    launchctl bootout "gui/$(id -u)/com.golish.cargo-space-guard" >/dev/null 2>&1 || true
    rm -f "$plist"
    echo "✓ Cargo space guard uninstalled."

# Enable target/ auto-cap: route git through .githooks/ (post-merge + post-checkout). One-time.
hooks-install:
    git config core.hooksPath .githooks
    @echo "✓ Auto-cap enabled. backend/target/ capped (default 40GB) on every pull / branch switch."
    @echo "  Tune ceiling: GOLISH_TARGET_CAP=30GB | skip once: GOLISH_SKIP_TARGET_CAP=1 | off: just hooks-uninstall"

# Disable target/ auto-cap: revert to the default .git/hooks.
hooks-uninstall:
    @git config --unset core.hooksPath || true
    @echo "✓ Reverted to default .git/hooks."

# Deep clean (includes node_modules)
clean-all: clean
    rm -rf node_modules

# ============================================
# Dependencies
# ============================================

# Install all dependencies
install:
    pnpm install --silent

# Update frontend dependencies
update-fe:
    pnpm update

# Update Rust dependencies
update-rust:
    cd backend && cargo update

# ============================================
# CLI & Evaluations
# ============================================

# Build CLI binary (unified with GUI - use --headless flag for CLI mode)
build-cli:
    cd backend && cargo build -p golish

# ============================================
# Utilities
# ============================================

# Kill any running dev processes (including server)
kill:
    -pkill -f "target/debug/golish" 2>/dev/null
    -pkill -f "golish-cli" 2>/dev/null
    -pkill -f "vite" 2>/dev/null
    -lsof -ti:1420 | xargs kill -9 2>/dev/null

# Restart dev (kill + dev)
restart: kill dev

# Show Rust dependency tree
deps:
    cd backend && cargo tree

# Open Rust docs
docs:
    cd backend && cargo doc --open

# Run a quick sanity check before committing
precommit: check test
    @echo "✓ All checks passed!"

# Run full CI suite (check + e2e + build)
ci: check test-e2e build
    @echo "✓ Full CI suite passed!"

# ============================================
# Release
# ============================================

# Show release status (pending PRs, latest release)
release-status:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Latest Release ==="
    gh release view --json tagName,publishedAt,name --jq '"\(.name) (\(.tagName)) - \(.publishedAt)"' 2>/dev/null || echo "No releases yet"

    echo ""
    echo "=== Pending Release PR ==="
    PR=$(gh pr list --label "autorelease: pending" --json number,title,url --jq '.[0]' 2>/dev/null)
    if [ -n "$PR" ] && [ "$PR" != "null" ]; then
        echo "$PR" | jq -r '"#\(.number): \(.title)\n\(.url)"'
    else
        echo "No pending release PR"
    fi

# Publish a new release (merges pending release-please PR)
publish:
    #!/usr/bin/env bash
    set -euo pipefail

    # Find the release PR
    PR_NUM=$(gh pr list --label "autorelease: pending" --json number --jq '.[0].number' 2>/dev/null)

    if [ -z "$PR_NUM" ] || [ "$PR_NUM" = "null" ]; then
        echo "No pending release PR found."
        echo ""
        echo "To create a release:"
        echo "  1. Make changes and push to main"
        echo "  2. Release-please will create a PR automatically"
        echo "  3. Run 'just publish' to merge it"
        exit 1
    fi

    echo "Found release PR #$PR_NUM"
    gh pr view "$PR_NUM"

    echo ""
    read -p "Merge this release PR? [y/N] " -n 1 -r
    echo ""

    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo "Merging PR #$PR_NUM..."
        gh pr merge "$PR_NUM" --squash --auto
        echo ""
        echo "✓ Release PR merged! CI will build and publish the release."
        echo "  Watch progress: gh run watch"
    else
        echo "Aborted."
    fi

# Create a manual release (bypasses release-please)
release-manual version:
    #!/usr/bin/env bash
    set -euo pipefail

    VERSION="{{version}}"

    # Validate version format
    if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "Error: Invalid version format. Use semver (e.g., 1.2.3)"
        exit 1
    fi

    echo "Creating release v$VERSION..."

    # Check for uncommitted changes
    if ! git diff --quiet; then
        echo "Error: You have uncommitted changes. Please commit or stash them first."
        exit 1
    fi

    # Create and push tag
    git tag -a "v$VERSION" -m "Release v$VERSION"
    git push origin "v$VERSION"

    echo "✓ Tag v$VERSION pushed. CI will build and publish the release."
    echo "  Watch progress: gh run watch"

# Run architecture guards locally (DAG + repo data-ownership). CI runs these
# in .github/workflows/arch-check.yml; this is the local mirror. Both guards
# run unconditionally so one failing guard doesn't hide the other's status.
arch:
    #!/usr/bin/env bash
    set -uo pipefail
    rc=0
    python3 scripts/check_dag.py || rc=1
    python3 scripts/check_repo_ownership.py || rc=1
    exit $rc
