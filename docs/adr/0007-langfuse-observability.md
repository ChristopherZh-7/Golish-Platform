# ADR-0007: Langfuse via OpenTelemetry for AI Observability

## Status

Accepted

## Context

Golish's AI subsystem makes multi-step LLM calls (planning → tool selection →
execution → synthesis) across multiple providers. We need observability for:

- **Cost tracking** — token usage per session, per model, per provider.
- **Latency profiling** — identify slow steps in agentic loops (e.g., which
  tool call or LLM round-trip is the bottleneck).
- **Quality evaluation** — associate human feedback and eval scores with
  specific traces.
- **Debugging** — inspect full prompt/completion pairs, tool call arguments
  and results, and error chains.

Requirements:

- **Rust-native** — no Python/JS sidecar for telemetry.
- **OpenTelemetry compatible** — leverage the OTel ecosystem for exporters,
  sampling, and future integrations (Datadog, Jaeger).
- **LLM-aware** — traces should capture LLM-specific semantics (model name,
  token counts, prompt/completion content), not just generic HTTP spans.

## Decision

Use **`opentelemetry-langfuse = "0.6"`** as the trace exporter, integrated
via the standard OpenTelemetry SDK stack:

```
tracing (Rust) → tracing-opentelemetry → opentelemetry_sdk → opentelemetry-langfuse
```

Dependency chain in `Cargo.toml`:

- `opentelemetry = "0.31"` (core API)
- `opentelemetry_sdk = "0.31"` (batch span processor)
- `opentelemetry-otlp = "0.31"` (fallback OTLP export)
- `tracing-opentelemetry = "0.32"` (bridge from `tracing` crate)
- `opentelemetry-langfuse = "0.6"` (Langfuse-specific exporter)

Key integration points:

- Each agentic loop iteration creates a trace with spans for: prompt
  assembly, LLM call, tool execution, and synthesis.
- Token counts and model names are attached as span attributes.
- `golish-synthesis` emits provider-specific metadata (Vertex project ID,
  OpenAI org, etc.) for cost attribution.

## Consequences

### Positive

- Full OpenTelemetry compatibility — can switch to any OTel-compatible
  backend (Jaeger, Honeycomb, Datadog) by changing the exporter.
- Langfuse provides a purpose-built UI for LLM traces: prompt playground,
  cost dashboards, eval scoring.
- Zero-overhead when disabled — `tracing` spans are compiled out when the
  Langfuse feature flag is off.
- Reuses the existing `tracing` ecosystem; no new logging framework.

### Negative

- `opentelemetry-langfuse` is community-maintained; updates may lag behind
  Langfuse API changes.
- OTel SDK adds ~2 MB to the binary and a background export thread.
- Langfuse is a hosted service (or self-hosted); requires network access
  for trace export. Local-only mode falls back to `tracing-subscriber` logs.
- Version alignment across `opentelemetry*` crates is fragile; all must
  be pinned to the same 0.31 release train.

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **Raw tracing-subscriber JSON logs** | No LLM-specific semantics, no cost tracking, no eval UI. |
| **LangSmith (LangChain)** | Python-only SDK; no Rust client. |
| **Helicone proxy** | Requires routing all LLM traffic through a proxy; adds latency and a SPOF. |
| **Custom Postgres logging** | Would need to build dashboards, aggregation, and retention from scratch. |
| **Datadog APM** | Expensive for a desktop tool's telemetry volume; overkill for dev/eval use. |
