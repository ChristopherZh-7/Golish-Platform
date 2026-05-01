# ADR-0003: graph-flow for Multi-Agent Orchestration

## Status

Accepted

## Context

Golish supports **multi-agent workflows** where a coordinator agent delegates
sub-tasks (reconnaissance, vulnerability analysis, exploit verification,
report generation) to specialized sub-agents. Requirements:

- **DAG execution** — sub-tasks have dependencies; e.g., "scan" must finish
  before "analyze findings."
- **Conditional branching** — skip exploit verification if no critical vulns
  found.
- **Parallel fan-out** — run independent sub-agents concurrently to reduce
  wall-clock time.
- **Deterministic replay** — for evals and debugging, re-run the same graph
  with recorded inputs.

We evaluated three approaches:

1. **Imperative async code** — `tokio::spawn` + channels; flexible but hard to
   visualize, serialize, or replay.
2. **Petri-net / state-machine libs** — overly formal for our use case.
3. **`graph-flow ^0.5`** — lightweight DAG runner with typed nodes, edges, and
   built-in topological execution.

## Decision

Use **`graph-flow = "0.5"`** as the execution engine for multi-agent
orchestration in `golish-pipeline` and `golish-sub-agents`.

Each pipeline step is a `graph_flow::Node` that wraps either:

- A **tool invocation** (Nuclei, Feroxbuster, …)
- An **LLM call** (via rig-core)
- A **sub-pipeline** (recursive composition)

The `golish-pipeline` crate owns template definitions (JSON), parsing, and
the orchestrator that maps templates → `graph_flow::Graph` → execution.

## Consequences

### Positive

- Graphs are serializable (JSON); enables pipeline templates
  (`templates/recon_basic.json`) that users can share and version.
- Topological sort guarantees correct execution order without manual
  dependency tracking.
- Fan-out nodes run concurrently via `tokio`; the framework handles join
  synchronization.
- Small dependency footprint (~1.5K LOC).

### Negative

- `graph-flow` is a niche crate; if abandoned, we may need to fork or
  rewrite the ~500 LOC integration layer.
- No built-in persistence for partial graph state; crash recovery requires
  our own checkpointing (currently not implemented).
- Conditional edges require runtime closures, which complicates serialization
  of "live" graphs.

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **Imperative tokio::spawn** | Works but graph structure is implicit; no serialization, no visual debugging. |
| **temporal-rs / durable execution** | Too heavy for a desktop app; requires a server. |
| **Custom DAG engine** | Would replicate what graph-flow already provides; maintenance burden. |
| **Python LangGraph sidecar** | Cross-language IPC, Python dependency, slower iteration. |
