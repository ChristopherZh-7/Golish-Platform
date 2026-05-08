#!/usr/bin/env python3
"""Architecture DAG guard for the Golish Rust workspace.

Parses every `backend/crates/*/Cargo.toml`, checks that the declared
`golish-*` dependencies only point at crates in lower layers. The
layer table below is authoritative — it must match `docs/architecture.md`.

Rules:
- A crate at layer N may depend on crates at layer ≤ N (siblings OK).
- The **graph must be acyclic** across the entire workspace
  (circular deps would prevent compilation anyway, but we also
  surface this with a nicer message).
- **Upward edges** (dep_layer > crate_layer) are hard errors.
- Anything in the workspace outside the `LAYER_TABLE` is an error
  (forces authors to place new crates on the map explicitly).
- **L2 sub-cluster coverage** (added in QW6, 2026-05): every L2 crate
  must appear in `L2_CLUSTER` with one of `ALLOWED_L2_CLUSTERS` as
  value. The cluster grouping is documentary (mirrors
  `docs/architecture.md` `### Crate catalog · L2`) — it does **not**
  add edges to the DAG, but it forces docs and code to stay in sync.

Scope:
- Only inspects `golish-*` dependency declarations. External `rig-*`
  deps (the in-tree provider forks share the `rig-` prefix with the
  upstream `rig-core` crates.io package) are filtered via the
  workspace `crates/` directory listing so third-party crates like
  `rig-core`, `rig-vertexai`, etc. are ignored.

Exit code:
- 0: clean
- 1: one or more illegal edges, a cycle, or an L2 cluster mismatch
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

# ---------------------------------------------------------------------------
# Layer table — single source of truth.
# ---------------------------------------------------------------------------
# "layer" is a float so we can encode 4a / 4b / 4c as 4.1 / 4.2 / 4.3.

LAYER_TABLE: dict[str, float] = {
    # L1 Foundation (zero internal golish-* deps)
    "golish-core": 1.0,
    "golish-platform": 1.0,
    "golish-settings": 1.0,
    "golish-context": 1.0,
    "golish-mcp": 1.0,
    "golish-projects": 1.0,
    "golish-graphiti": 1.0,
    "golish-json-repair": 1.0,
    "golish-udiff": 1.0,
    "golish-pentest-domain": 1.0,
    "golish-vuln-intel-domain": 1.0,
    "golish-js-analyzer": 1.0,
    "rig-anthropic-vertex": 1.0,
    "rig-gemini-vertex": 1.0,
    # L2 Simple infrastructure (depends on L1 and optionally L2 siblings)
    "golish-models": 2.0,
    "golish-events": 2.0,
    "golish-session": 2.0,
    "golish-indexer": 2.0,
    "golish-llm-providers": 2.0,
    "golish-db": 2.0,
    "golish-pty": 2.0,
    "golish-web": 2.0,
    "golish-tools": 2.0,
    "golish-pentest": 2.0,
    "golish-vuln-intel": 2.0,
    "golish-scan-runner": 2.0,
    "golish-shell-exec": 2.0,
    "golish-auth-probe": 2.0,
    "golish-skills": 2.0,
    "golish-synthesis": 2.0,
    "golish-artifacts": 2.0,
    "golish-cli-output": 2.0,
    "golish-pentest-mcp": 2.0,
    "rig-openai-responses": 2.0,
    "rig-zai-sdk": 2.0,
    # L3 Domain services
    "golish-prompts": 3.0,
    "golish-sub-agents": 3.0,
    "golish-pipeline": 3.0,
    "golish-sidecar": 3.0,
    # L4 Agent stack (three-tier)
    "golish-agent-kit": 4.1,
    "golish-agent-runtime": 4.2,
    "golish-agent-bridge": 4.3,
    # L5 Evaluation harnesses
    "golish-evals": 5.0,
    "golish-benchmarks": 5.0,
    "golish-swebench": 5.0,
    # L6 Application
    "golish": 6.0,
}

# ---------------------------------------------------------------------------
# L2 sub-cluster table — purely documentary.
# ---------------------------------------------------------------------------
# Mirrors the `### Crate catalog · L2` section of `docs/architecture.md`.
# The DAG guard only enforces the layer constraint (a crate at L_n may
# depend only on crates at L_{≤n}); sub-clusters are for human discoverability.
# Every L2 crate must appear in this table — adding a new L2 crate without
# registering it here will fail the docs guard below.
L2_CLUSTER: dict[str, str] = {
    # persistence — data / session / index / model registry
    "golish-db": "persistence",
    "golish-models": "persistence",
    "golish-session": "persistence",
    "golish-indexer": "persistence",
    # os — process / PTY / event scheduling
    "golish-pty": "os",
    "golish-shell-exec": "os",
    "golish-events": "os",
    # llm — LLM provider clients
    "golish-llm-providers": "llm",
    "rig-openai-responses": "llm",
    "rig-zai-sdk": "llm",
    # pentest — penetration testing domain
    "golish-pentest": "pentest",
    "golish-vuln-intel": "pentest",
    "golish-scan-runner": "pentest",
    "golish-pentest-mcp": "pentest",
    "golish-auth-probe": "pentest",
    # assets — skills / synthesis / output / tools
    "golish-skills": "assets",
    "golish-synthesis": "assets",
    "golish-artifacts": "assets",
    "golish-cli-output": "assets",
    "golish-web": "assets",
    "golish-tools": "assets",
}

ALLOWED_L2_CLUSTERS: frozenset[str] = frozenset(
    {"persistence", "os", "llm", "pentest", "assets"}
)

# Regex matches e.g. `golish-foo = { workspace = true }` or `rig-openai-responses = { path = "..." }`
# at the start of a line. We filter to in-workspace crates after parsing.
DEP_RE = re.compile(r"^([a-z][a-z0-9-]+)\s*=", re.MULTILINE)


@dataclass
class Violation:
    reason: str


def parse_deps(toml_text: str, workspace_crates: set[str]) -> set[str]:
    """Extract workspace crate names declared as deps (drops external crates)."""
    candidates = {m.group(1) for m in DEP_RE.finditer(toml_text)}
    return candidates & workspace_crates


def load_crate_deps(crates_dir: Path) -> dict[str, set[str]]:
    """Load {crate_name: {dep_crate_names}} from every Cargo.toml."""
    workspace_crates = {p.name for p in crates_dir.iterdir() if p.is_dir()}
    graph: dict[str, set[str]] = {}
    for cargo in crates_dir.glob("*/Cargo.toml"):
        crate_name = cargo.parent.name
        graph[crate_name] = parse_deps(cargo.read_text(), workspace_crates)
    return graph


def detect_cycles(graph: dict[str, set[str]]) -> list[list[str]]:
    """Return any simple cycles as lists of crate names (empty if DAG)."""
    WHITE, GRAY, BLACK = 0, 1, 2
    color = {n: WHITE for n in graph}
    cycles: list[list[str]] = []
    stack: list[str] = []

    def dfs(n: str) -> None:
        if color[n] == GRAY:
            idx = stack.index(n)
            cycles.append(stack[idx:] + [n])
            return
        if color[n] == BLACK:
            return
        color[n] = GRAY
        stack.append(n)
        for m in graph.get(n, set()):
            dfs(m)
        stack.pop()
        color[n] = BLACK

    for node in graph:
        if color[node] == WHITE:
            dfs(node)
    return cycles


def check(graph: dict[str, set[str]]) -> list[Violation]:
    violations: list[Violation] = []

    # 1. Layer table membership
    for crate in graph:
        if crate not in LAYER_TABLE:
            violations.append(
                Violation(
                    f"unknown crate `{crate}` — add it to LAYER_TABLE in scripts/check_dag.py"
                )
            )

    # 2. Upward edges
    for crate, deps in graph.items():
        if crate not in LAYER_TABLE:
            continue
        crate_layer = LAYER_TABLE[crate]
        for dep in deps:
            if dep not in LAYER_TABLE:
                violations.append(
                    Violation(
                        f"`{crate}` depends on unknown crate `{dep}` — add it to LAYER_TABLE"
                    )
                )
                continue
            dep_layer = LAYER_TABLE[dep]
            if dep_layer > crate_layer:
                violations.append(
                    Violation(
                        f"illegal upward edge: `{crate}` (L{crate_layer}) "
                        f"depends on `{dep}` (L{dep_layer}) which sits at a higher layer"
                    )
                )

    # 3. Cycle check (siblings at the same layer are OK as long as acyclic)
    for cycle in detect_cycles(graph):
        pretty = " → ".join(cycle)
        violations.append(Violation(f"dependency cycle detected: {pretty}"))

    # 4. L2 sub-cluster coverage (documentary; mirrors docs/architecture.md)
    l2_crates = {c for c, layer in LAYER_TABLE.items() if layer == 2.0}
    clustered = set(L2_CLUSTER.keys())
    for crate in sorted(l2_crates - clustered):
        violations.append(
            Violation(
                f"L2 crate `{crate}` is missing from L2_CLUSTER — assign it "
                f"to one of {sorted(ALLOWED_L2_CLUSTERS)} (see "
                f"docs/architecture.md `### Crate catalog · L2`)"
            )
        )
    for crate in sorted(clustered - l2_crates):
        violations.append(
            Violation(
                f"`{crate}` is in L2_CLUSTER but its LAYER_TABLE entry is "
                f"not L2.0 — remove it from L2_CLUSTER or fix its layer"
            )
        )

    # 5. L2 cluster name validity
    for crate, cluster in sorted(L2_CLUSTER.items()):
        if cluster not in ALLOWED_L2_CLUSTERS:
            violations.append(
                Violation(
                    f"`{crate}` has unknown cluster `{cluster}` — must be "
                    f"one of {sorted(ALLOWED_L2_CLUSTERS)}"
                )
            )

    return violations


def main() -> int:
    crates_dir = Path(__file__).resolve().parent.parent / "backend" / "crates"
    if not crates_dir.is_dir():
        print(f"[check_dag] ERROR: crates dir not found: {crates_dir}", file=sys.stderr)
        return 2

    graph = load_crate_deps(crates_dir)
    violations = check(graph)

    if not violations:
        print(f"[check_dag] ✓ DAG clean across {len(graph)} crates")
        return 0

    print(f"[check_dag] ✗ {len(violations)} violation(s):", file=sys.stderr)
    for v in violations:
        print(f"  - {v.reason}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
