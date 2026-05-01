# ADR-0005: vtcode-indexer for Code / File Indexing

## Status

Accepted

## Context

Golish's AI agent needs to understand the target codebase to provide
context-aware security analysis. Requirements:

- **AST-level indexing** — extract symbols (functions, classes, imports) from
  source files in multiple languages.
- **Incremental updates** — re-index only changed files on file-system events.
- **Embedding-ready chunks** — produce text chunks suitable for vector
  embedding and semantic search.
- **Rust-native** — must integrate as a library crate, not a subprocess.

The `golish-indexer` crate wraps the indexing engine and exposes it to the
AI agent loop for context retrieval.

## Decision

Use **`vtcode-indexer = "0.105"`** as the file indexing and chunking engine.

`vtcode-indexer` provides:

- Tree-sitter-based AST parsing for 20+ languages.
- Configurable chunk strategies (by symbol, by line window, by semantic
  boundary).
- `.gitignore`-aware file walking (complemented by our use of the `ignore`
  crate).
- Streaming async API compatible with `tokio`.

## Consequences

### Positive

- Reuses battle-tested Tree-sitter grammars; adding a new language is a
  grammar plugin, not a parser rewrite.
- Chunk boundaries align with symbol boundaries (functions, classes), which
  improves embedding quality for RAG.
- Incremental diffing support — only re-parse files whose mtime changed.
- Pure Rust; no Python/Node subprocess needed.

### Negative

- `vtcode-indexer` is a relatively niche crate; API stability is not
  guaranteed across minor versions.
- Binary size increases due to bundled Tree-sitter grammars (~5 MB).
- Configuration surface is large; we wrap it in `golish-indexer` to
  expose only the subset we need.

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **tree-sitter direct** | Too low-level; would need to build chunking, file walking, and incremental logic ourselves. |
| **ast-grep** | Focused on pattern matching / linting, not indexing/chunking. (We do use ast-grep separately in `golish-tools` for code search and replace.) |
| **ripgrep as library** | Text-only; no AST awareness, no chunking. |
| **Python unstructured** | Cross-language dependency; slower; not embeddable as a Rust lib. |
