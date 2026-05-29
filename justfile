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
    {{ if path == "" { "pnpm tauri dev" } else { "pnpm tauri dev -- " + path } }}

# Start only the frontend dev server
dev-fe:
    pnpm dev

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
    @printf '\033[1;32m━━━ OK ━━━\033[0m\n'

[private]
step recipe:
    #!/usr/bin/env bash
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
