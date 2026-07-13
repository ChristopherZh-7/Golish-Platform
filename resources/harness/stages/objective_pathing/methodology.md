# Objective Pathing methodology (C6 / P6b)

## Contract

Objective Pathing synthesizes a bounded ordered graph from canonical internal
observations. Every edge binds two different frozen identity hashes, a typed
technique, a contiguous ordinal, and evidence. Duplicate, self-referential,
cross-scope, or evidence-free edges fail closed.

## P6b capability boundary

This stage still plans only. `post_exploit_build_objective_path` derives the path
hash and edge IDs server-side under the exact runtime fence. It exposes no AD
tool, credential action, tunnel, exploit, or mutation capability. An AttackPath
is a hypothesis, not proof that any external action ran.

## Canonical result

`attack_paths` + `attack_path_edges` + edge evidence are written in one
transaction with exact replay semantics. Model prose and diagrams are optional
renderings and never replace these rows.
