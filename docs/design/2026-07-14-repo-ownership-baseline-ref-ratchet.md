# Repo ownership baseline-ref no-new-violation ratchet

**Date:** 2026-07-14
**Scope:** Runtime Memory / Candidate Pipeline V2 closure verification only
**Baseline:** `13b29628f2954b56b918329bfe3217132fe6eb56`
**Supplement to:** `2026-07-12-candidate-verification-pipeline-v2-corrected.md`

## Problem

The historical full-tree command remains useful as a debt inventory, but it is
not currently an actionable green gate. On the V2 closure worktree,
`python3 scripts/check_repo_ownership.py` reports hundreds of pre-existing
ownership occurrences plus historical raw-SQL files. Treating that output as a
new-feature failure obscures whether this slice introduced a new boundary
violation; treating it as green would be false.

This supplemental design replaces only the ownership verification command for
this closure slice. It does not delete, waive, or relabel any historical
violation, and it does not claim that the full checker is clean.

## Decision

Add:

```bash
python3 scripts/check_repo_ownership.py --baseline-ref 13b29628
```

The command evaluates the current worktree and the exact git tree at the
checkpoint with the rule constants and scanners loaded from the current script.
It never executes the script from the checkpoint. This keeps rule semantics
identical on both sides of the comparison.

Each violation has an exact identity:

```text
(category, existing checker message)
```

The categories are `ownership`, `raw-sql`, and `finding-write`. The two trees
produce deduplicated sets `current` and `baseline`:

```text
added   = current - baseline
removed = baseline - current
```

Only a non-empty `added` set fails the gate. A removed violation is ratchet
progress and cannot fail it. An unchanged historical violation remains visible
in the set counts but is not restated as a new failure.

## Snapshot semantics

- `WorktreeSnapshot` reads the filesystem, including untracked production
  files. A new untracked Rust file can therefore fail the gate before staging.
- `GitRefSnapshot` resolves the supplied ref to a commit, lists paths with
  `git ls-tree`, and reads blobs with `git show`; it never checks out or modifies
  the worktree.
- Both snapshots scan the same `SOURCE_ROOTS`, repo declarations, raw SQL, and
  production `INSERT INTO findings` authority.
- A missing required repo module, invalid ref, unknown CLI option, or git read
  failure is a setup error with exit code 2, not a clean result.
- Git is invoked as an argument vector without a shell. A baseline value that
  starts with `-` is rejected.

## CLI compatibility

Existing modes retain their meanings:

| command | contract |
|---|---|
| no option | full current-tree inventory; still fails while historical violations exist |
| `--finding-writes-only` | independent guarded Finding-writer authority |
| `--emit-allowlist` | print current candidate allowlist entries |
| `--baseline-ref <ref>` | fail only for exact violations newly present versus the ref |

The baseline success message explicitly says `historical violations not
asserted clean`. Exit codes remain 0 for the requested gate passing, 1 for gate
violations, and 2 for setup/usage errors.

## Baseline choice

`13b29628` is the user-approved Runtime Memory / Candidate V2 checkpoint. It
precedes the current closure delta and is immutable, so it answers one narrow
question: did the post-checkpoint implementation add a repository boundary,
raw-SQL, or Finding-write violation under today's rules?

The baseline is not an evergreen allowlist. A later feature must name its own
reviewed checkpoint or first restore the full checker to a genuinely clean
state. Changing the ref merely to hide a failure is outside this contract.

## Acceptance and current truth

The Python regression creates a temporary git repository containing one known
historical cross-domain violation, adds a new raw-SQL file only in the worktree,
and proves:

1. the unchanged historical violation appears in both sets and is suppressed;
2. the new raw-SQL violation appears exactly in `added`;
3. deleting the historical violation moves it to `removed` without changing
   the new-failure set.

The first real run against `13b29628` correctly returned exit 1 and identified
two genuine post-checkpoint violations: direct agent access to pentest-owned
`operation_state`, and raw SQL in `stage_run/runtime_v2.rs`. They must be fixed
through the existing service/repository boundaries. Adding them to an allowlist
would defeat this design.
