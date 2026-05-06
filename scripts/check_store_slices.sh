#!/usr/bin/env bash
# Architecture guard for the Zustand store slice surface (R6).
#
# Fails the build if `frontend/store/slices/` grows beyond the
# documented upper bound. Rationale: docs/architecture.md describes a
# 14-slice composition; runaway slice growth signals a missing
# domain split (e.g. each new "feature" getting its own slice
# instead of folding into a richer existing one).
#
# Counts only top-level `*.ts` slice files — excludes test files,
# helper modules (session-helpers, session-streaming, etc.), the
# `index.ts` aggregator, and `*.test.ts`. The 16-file ceiling
# leaves room for one or two organic additions before the next
# review.

set -euo pipefail

SLICES_DIR="frontend/store/slices"
LIMIT=16

if [[ ! -d "$SLICES_DIR" ]]; then
    echo "ERROR: $SLICES_DIR does not exist" >&2
    exit 1
fi

# Slice files are top-level *.ts only (workflow/ has its own internal
# decomposition that exposes a single `workflow` slice via its
# index.ts). Helpers, type-only files, and tests are excluded.
# Also count `workflow/index.ts` as 1 (the bundle's public face).
slices=$(
    find "$SLICES_DIR" -maxdepth 1 -type f -name "*.ts" \
        ! -name "*.test.ts" \
        ! -name "index.ts" \
        ! -name "types.ts" \
        ! -name "session-helpers.ts" \
        ! -name "session-streaming.ts" \
        ! -name "session-core.ts" \
        ! -name "session-draft-types.ts" \
        ! -name "session-tabs.ts" \
        ! -name "session-terminal.ts" \
        | sort
)
if [[ -f "$SLICES_DIR/workflow/index.ts" ]]; then
    slices="$slices
$SLICES_DIR/workflow/index.ts"
fi

count=$(echo "$slices" | wc -l | tr -d ' ')

echo "Store slice budget check"
echo "------------------------"
echo "$slices" | sed 's|^|  |'
echo
echo "Counted: $count"
echo "Limit:   $LIMIT"

if [[ "$count" -gt "$LIMIT" ]]; then
    echo
    echo "ERROR: store has $count slices, exceeding the $LIMIT-slice budget." >&2
    echo "       See docs/architecture.md." >&2
    echo "       Either fold the new slice into an existing one, or" >&2
    echo "       raise the limit here in $0 with a justifying comment." >&2
    exit 1
fi

echo
echo "OK: store slice count within budget."
