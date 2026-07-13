#!/usr/bin/env bash
# Keep AI-driven Rust builds from exhausting the macOS data volume without
# forcing a cold `cargo clean`. Old Cargo artifacts are removed only while the
# compiler/test runner is idle.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
target_dir="$repo_root/backend/target"
min_free_gb="${GOLISH_MIN_FREE_GB:-80}"
target_cap="${GOLISH_TARGET_CAP:-30GB}"
lock_dir="${TMPDIR:-/tmp}/golish-cargo-space-guard.lock"

[ -d "$target_dir" ] || exit 0
command -v cargo-sweep >/dev/null 2>&1 || {
  echo "cargo-space-guard: cargo-sweep is missing; install it with: cargo install cargo-sweep --locked" >&2
  exit 0
}

# Never mutate target/ while Cargo, rustc, nextest, or another sweep is active.
for process in cargo rustc cargo-nextest nextest cargo-sweep; do
  if pgrep -x "$process" >/dev/null 2>&1; then
    exit 0
  fi
done

if ! mkdir "$lock_dir" 2>/dev/null; then
  exit 0
fi
trap 'rmdir "$lock_dir" 2>/dev/null || true' EXIT

available_kb="$(df -Pk /System/Volumes/Data | awk 'NR == 2 { print $4 }')"
minimum_kb="$((min_free_gb * 1024 * 1024))"
if [ "$available_kb" -ge "$minimum_kb" ]; then
  exit 0
fi

before_gb="$((available_kb / 1024 / 1024))"
echo "cargo-space-guard: ${before_gb}GB free is below ${min_free_gb}GB; pruning oldest Cargo artifacts to ${target_cap}."
(cd "$repo_root/backend" && cargo sweep --installed && cargo sweep --maxsize "$target_cap")
after_kb="$(df -Pk /System/Volumes/Data | awk 'NR == 2 { print $4 }')"

# The workspace disables dev incremental compilation because these snapshots
# were the dominant source of unbounded growth. Remove any leftovers from old
# builds only after confirming all Cargo processes are idle.
incremental_dir="$target_dir/debug/incremental"
if [ "$after_kb" -lt "$minimum_kb" ] && [ -d "$incremental_dir" ]; then
  echo "cargo-space-guard: free space is still low; removing stale dev incremental snapshots."
  rm -rf "$incremental_dir"
  mkdir -p "$incremental_dir"
  after_kb="$(df -Pk /System/Volumes/Data | awk 'NR == 2 { print $4 }')"
fi

after_gb="$((after_kb / 1024 / 1024))"
echo "cargo-space-guard: finished with ${after_gb}GB free; recent build artifacts were retained."
