# ADR-0002: rig-core vs LangChain-rs for LLM Orchestration

## Status

Accepted

## Context

Golish's AI subsystem needs to:

- Call multiple LLM providers (OpenAI, Anthropic, Google Vertex, Grok/xAI)
  through a **uniform completion API**.
- Support **tool-use / function-calling** with structured JSON schemas.
- Allow **streaming** token responses for real-time CLI and UI output.
- Integrate cleanly with the Rust async ecosystem (`tokio`, `reqwest`).

At decision time (early 2025), the Rust LLM orchestration landscape offered:

| Crate | Maturity | Multi-provider | Tool-use | Streaming |
|---|---|---|---|---|
| `rig-core` | Active, v0.3x | Yes (via companion crates) | First-class | Yes |
| `langchain-rs` | Early alpha | Partial | Partial | Limited |
| `llm-chain` | Stale | OpenAI only | No | No |
| Hand-rolled `reqwest` | N/A | Manual | Manual | Manual |

## Decision

Adopt **`rig-core ^0.36`** with companion provider crates:

- `rig-vertexai` — Google Vertex AI (Gemini)
- `rig-anthropic-vertex` — Anthropic via Vertex (in-tree fork)
- `rig-openai-responses` — OpenAI Responses API (in-tree fork)
- `rig-zai-sdk` — xAI / Grok (in-tree fork)
- `rig-gemini-vertex` — Gemini direct (in-tree fork)

## Consequences

### Positive

- Unified `CompletionModel` / `Agent` traits decouple domain logic from any
  single provider; swapping providers is a config change.
- Built-in `Tool` trait maps directly to JSON Schema function-calling; our
  `golish-tools` crate implements it without glue code.
- Active upstream maintenance with semver releases; breaking changes are
  manageable at the `^0.36` pin.

### Negative

- Pre-1.0 API — minor version bumps can break; we maintain 4 in-tree fork
  crates (`rig-*`) to patch provider-specific quirks faster than upstream.
- `rig-core` does **not** provide built-in agentic loops, memory, or RAG
  pipelines; those are implemented in `golish-ai` / `golish-sub-agents`.
- Companion crates for niche providers (xAI, Vertex Anthropic) are thin and
  maintained by us, not upstream.

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **langchain-rs** | Alpha quality, incomplete tool-use, limited provider coverage. |
| **llm-chain** | Unmaintained, OpenAI-only. |
| **Raw reqwest** | Too much boilerplate; every provider has different auth, streaming, and tool-call wire formats. |
| **Python sidecar (LangChain / LlamaIndex)** | Cross-language IPC overhead, deployment complexity, loses Rust type safety. |
