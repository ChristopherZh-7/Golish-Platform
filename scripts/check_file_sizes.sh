#!/usr/bin/env bash
# Architecture file-size guard.
#
# Fails the build if any non-test file exceeds its budget:
# - Rust:  500 lines (exclude tests/, *_tests.rs, mocks.rs)
# - TS/TSX: 800 lines (exclude __fixtures__/, *.test.ts, *.test.tsx, mocks.ts)
#
# Rationale: big files block parallel reviews and slow incremental builds.
# Encode budgets in CI so drift is caught on the PR that introduces it.
#
# GRANDFATHER LIST: a few existing TSX files are over-budget today; they
# are explicitly exempted here so the CI gate turns green *right now* but
# the list is monotonic-decreasing — new files at the same paths cannot
# get larger. Remove entries as you shrink the files.

RUST_LIMIT=500
TS_LIMIT=800

# Files exempted from the TS budget (each with its current line count as
# of 2026-05-08). CI will *fail* if any of these grows — use this list
# as a shrinking worklist. Encoded as a function so paths with slashes
# don't clash with Bash associative-array indexing.
ts_baseline() {
    case "$1" in
        # Removed (now within budget):
        #   - frontend/components/ToolManager/ToolManager.tsx (was 1044, now < 800 via QW5 split)
        #   - frontend/components/ProjectOverview/ProjectOverview.tsx (was 838, now < 800 via QW5 split)
        "frontend/components/VulnIntelPanel/WikiTab.tsx") echo 891 ;;
        "frontend/components/SecurityView/ScanToolsPanel.tsx") echo 843 ;;
        "frontend/components/VulnIntelPanel/VulnIntelPanel.tsx") echo 819 ;;
        "frontend/components/ToolManager/hooks/useToolInstall.ts") echo 810 ;;
        *) echo "" ;;
    esac
}

# Files exempted from the Rust budget (each with its current line count
# as of 2026-05-08). Same rules as ts_baseline: monotonic-decreasing
# worklist — CI fails if any of these grows. Tests like `tests.rs` live
# here too; consider adding `-not -name tests.rs` to the find filter
# once the legacy test file is properly split.
rust_baseline() {
    case "$1" in
        "backend/crates/golish/src/tools/pentest_bridge/js_collect.rs") echo 1365 ;;
        "backend/crates/golish-pipeline/src/engine/steps/single.rs") echo 960 ;;
        "backend/crates/golish/src/ai/tracking_bridge.rs") echo 709 ;;
        "backend/crates/golish/src/ai/db_bridge.rs") echo 679 ;;
        "backend/crates/golish/src/tools/methodology.rs") echo 568 ;;
        "backend/crates/golish-pipeline/src/engine/tests.rs") echo 564 ;;
        "backend/crates/golish-pipeline/src/engine/orchestrator.rs") echo 558 ;;
        "backend/crates/golish-pentest/src/models.rs") echo 536 ;;
        "backend/crates/golish-js-analyzer/src/lib.rs") echo 531 ;;
        "backend/crates/golish-llm-providers/src/lib.rs") echo 530 ;;
        "backend/crates/golish/src/ai/commands/mod.rs") echo 525 ;;
        "backend/crates/golish-db/src/repo/audit.rs") echo 524 ;;
        "backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs") echo 506 ;;
        *) echo "" ;;
    esac
}

violations=0

# ---- Rust --------------------------------------------------------------
echo "[check_file_sizes] scanning Rust files > ${RUST_LIMIT} lines …"
while IFS=$'\t' read -r lines path; do
  baseline=$(rust_baseline "$path")
  if [ -n "$baseline" ]; then
    if [ "$lines" -gt "$baseline" ]; then
      echo "[check_file_sizes] ✗ $path grew: $lines > baseline $baseline" >&2
      violations=$((violations + 1))
    else
      echo "    (grandfather ≤ $baseline) $path = $lines"
    fi
    continue
  fi
  echo "[check_file_sizes] ✗ $path: $lines lines > ${RUST_LIMIT}" >&2
  violations=$((violations + 1))
done < <(
  find backend/crates -name "*.rs" \
      -not -path "*/tests/*" \
      -not -name "*_tests.rs" \
      -not -name "mocks.rs" \
      -print0 \
    | xargs -0 wc -l 2>/dev/null \
    | awk -v L="$RUST_LIMIT" '$1 > L && $2 != "total" { print $1 "\t" $2 }' \
    | sort -rn
)

# ---- TS / TSX ----------------------------------------------------------
echo "[check_file_sizes] scanning TS/TSX files > ${TS_LIMIT} lines …"
while IFS=$'\t' read -r lines path; do
  baseline=$(ts_baseline "$path")
  if [ -n "$baseline" ]; then
    if [ "$lines" -gt "$baseline" ]; then
      echo "[check_file_sizes] ✗ $path grew: $lines > baseline $baseline" >&2
      violations=$((violations + 1))
    else
      echo "    (grandfather ≤ $baseline) $path = $lines"
    fi
    continue
  fi
  echo "[check_file_sizes] ✗ $path: $lines lines > ${TS_LIMIT}" >&2
  violations=$((violations + 1))
done < <(
  find frontend \( -name "*.ts" -o -name "*.tsx" \) \
    -not -path "*/__fixtures__/*" \
    -not -name "*.test.ts" \
    -not -name "*.test.tsx" \
    -not -name "mocks.ts" \
    -print0 \
  | xargs -0 wc -l 2>/dev/null \
  | awk -v L="$TS_LIMIT" '$1 > L && $2 != "total" { print $1 "\t" $2 }'
)

# ---- summary -----------------------------------------------------------
if [ "$violations" -eq 0 ]; then
  echo "[check_file_sizes] ✓ all files within size budget (with grandfather list)"
  exit 0
fi

echo "[check_file_sizes] ${violations} violation(s) — split the offending files into modules."
exit 1
