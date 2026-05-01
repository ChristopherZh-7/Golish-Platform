# ADR-0004: Embedded PostgreSQL (pg-embed) vs SQLite

## Status

Accepted

## Context

Golish persists scan results, vulnerability records, session history, project
metadata, and a graph knowledge base (`golish-graphiti`). Requirements:

- **JSONB queries** — vulnerability payloads, tool outputs, and LLM responses
  are semi-structured; we need indexed JSON path queries.
- **Full-text search** — searching across scan findings, CVE descriptions.
- **Concurrent writes** — multiple async tasks (scanner output, AI analysis,
  user actions) write simultaneously.
- **Graph queries** — `golish-graphiti` stores entity-relation graphs for
  security reasoning; recursive CTEs and JSONB operators are essential.
- **Zero-install** — desktop app; users should not need to install PostgreSQL.

## Decision

Use **`pg-embed ^1`** to run an embedded PostgreSQL instance, accessed via
**`sqlx 0.8`** (async, compile-time checked queries).

`golish-db` manages the lifecycle:

1. On first launch, `pg-embed` downloads PG binaries to a platform-specific
   cache directory.
2. Starts a local PG process on a random port.
3. Runs `sqlx::migrate!()` to apply schema migrations.
4. Provides a `PgPool` shared across all crates via dependency injection.

## Consequences

### Positive

- Full PostgreSQL feature set: JSONB, GIN indexes, recursive CTEs, full-text
  search with `tsvector`, window functions.
- `sqlx` compile-time query checking catches SQL errors at build time.
- Single database engine for both relational data (`golish-db`) and graph
  data (`golish-graphiti`); no need for a separate graph DB.
- Users can connect external tools (pgAdmin, DBeaver) for debugging.

### Negative

- **First-launch latency** — downloading PG binaries (~50 MB) on first run.
- **Disk footprint** — PG data directory grows larger than an equivalent
  SQLite file; ~200 MB baseline.
- **Process management** — must handle PG process lifecycle (start, health
  check, graceful shutdown, crash recovery).
- **Platform coverage** — `pg-embed` binary availability depends on
  upstream; ARM Linux may lag.

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **SQLite (rusqlite)** | No JSONB operators, limited concurrent write throughput (WAL helps but still single-writer), no recursive CTEs comparable to PG. |
| **DuckDB** | Optimized for OLAP, not OLTP; poor concurrent write support. |
| **SurrealDB embedded** | Immature Rust driver, unclear production stability. |
| **Remote PostgreSQL** | Requires user to provision a server; breaks zero-install desktop UX. |
| **SQLite + separate graph DB (neo4j-embedded)** | Two engines to maintain; neo4j has no lightweight Rust embedding. |
