#!/usr/bin/env bash
# Architecture file-size guard.
#
# Fails the build if any non-test file exceeds its budget:
# - Rust:  500 lines (exclude tests/, *_tests.rs, tests.rs, mocks.rs)
# - TS/TSX: 800 lines (exclude __fixtures__/, mocks/, *.test.ts, *.test.tsx, mocks.ts)
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
# of 2026-05-02). CI will *fail* if any of these grows — use this list
# as a shrinking worklist. Encoded as a function so paths with slashes
# don't clash with Bash associative-array indexing.
ts_baseline() {
    case "$1" in
        # Removed (now within budget):
        #   - frontend/components/ToolManager/ToolManager.tsx (was 1044, now < 800 via QW5 split)
        #   - frontend/components/ProjectOverview/ProjectOverview.tsx (was 838, now < 800 via QW5 split)
        "frontend/components/VulnIntelPanel/WikiTab.tsx") echo 891 ;;
        "frontend/components/VulnIntelPanel/VulnIntelPanel.tsx") echo 819 ;;
        *) echo "" ;;
    esac
}

# Rust files exempted from the RUST budget. Same monotonic-decreasing
# contract as ts_baseline: CI fails if any of these grows.
#
#   - golish-core/src/events/event.rs is a single `#[derive(ts_rs::TS)]`
#     `AiEvent` wire-contract enum. A Rust enum's variants cannot be split
#     across files, and nesting variants into sub-enums would change the
#     serde `{ "type": ... }` JSON wire format consumed by the frontend
#     (an I5 break). It is intentionally one large enum; exempt by design.
rust_baseline() {
    case "$1" in
        "backend/crates/golish-core/src/events/event.rs") echo 504 ;;
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
    -not -name "tests.rs" \
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
    -not -path "*/mocks/*" \
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
