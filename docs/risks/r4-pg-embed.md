# R4 — `pg-embed` Embedded Postgres Cost Analysis

> Status: **Risk identified, decision pending — keep, replace, or both.**
> Last updated: 2026-05-02.

## Current state

- `backend/Cargo.toml:140` declares `pg-embed = { version = "1", features = ["rt_tokio"] }`.
- `golish-db` crate (in workspace, see `architecture.md`) wraps `sqlx`
  + `pg-embed` for all persistent state.
- Embedded Postgres binary ships inside the Tauri app bundle.
- ADR-0004 (`docs/adr/0004-embedded-postgres-vs-sqlite.md`) records the
  choice: rich SQL + JSON + future pgvector support.

## Observed cost

| Dimension | Number |
|---|---|
| First-launch latency | ~10s (download + extract + initdb) on a clean machine |
| App bundle size | ~100MB extra (pg binary + libs per platform) |
| Memory footprint | ~80MB Postgres backend process always running |
| Cold restart | ~2-3s (initdb skipped, but server start) |
| CI runner load | Embedded Postgres needs to come up before backend tests |

## When this matters

- **First user impression**: 10s "starting database…" before the
  AI Chat is usable. New macOS users have left bad reviews on
  similar Tauri+Postgres apps for this reason.
- **Bundle size**: 100MB extra makes `homebrew tap` updates slow
  and pushes the project past Apple's notarisation defaults.
- **Memory**: 80MB is meaningful on 8GB / 16GB laptops where the
  app is competing with the AI providers' SDK and the Tauri webview.

## Alternatives evaluated

### Option A — Stay on pg-embed
- Pros: All current SQL works as-is. pgvector is a single
  `CREATE EXTENSION` away.
- Cons: cost above stays.

### Option B — Migrate to SQLite + sqlx (with `sqlite-vec` extension)
- Pros: <1s startup. <5MB bundle increment. ~10MB memory.
- Cons:
  - Schema migration: the existing `migrations/` directory uses
    Postgres-specific syntax (`SERIAL`, `JSONB`, partial indexes).
  - `sqlite-vec` is younger than `pgvector`; some operators differ.
  - Loses concurrent multi-writer perf (rare for desktop app, OK).

### Option C — Hybrid: Postgres for vector + sqlite for hot data
- Pros: Best of both, but…
- Cons: Now you have two databases. Twice the state machines.
  Strongly recommend against.

## Recommendation

Two-week spike on Option B:
1. Day 1-2: prototype the schema rewrite (Postgres → SQLite syntax).
2. Day 3-4: wire `sqlite-vec` for the embeddings code (memory search,
   wiki suggestions).
3. Day 5: A/B benchmark startup time + memory + bundle size in CI.
4. Decide based on measured numbers, not hunches.

If Option B beats by ≥3× on startup AND memory grows by <20MB,
migrate. Otherwise stay on pg-embed and document the trade-off
publicly so users know what they're getting.

## References

- `docs/adr/0004-embedded-postgres-vs-sqlite.md` — original decision
- `backend/crates/golish-db/` — schema + repo layer
- `pg-embed` crate: <https://crates.io/crates/pg-embed>
- `sqlite-vec`: <https://github.com/asg017/sqlite-vec>
