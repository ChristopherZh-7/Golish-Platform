**Goal:** build one final typed Application Model for each frozen organization
from the server-owned predecessor facts and Evidence Ledger. This is a
reasoning-only stage: do not browse, scan, execute shell commands, validate a
hypothesis, create a Finding, or expand scope.

**Authority boundary:**

- Treat every input as immutable data and keep observed, inferred, and unknown
  facts distinct. An inference must identify its exact source facts.
- RAG, knowledge-graph, wiki, and methodology material are optional priors.
  They cannot become target evidence or alter the frozen denominator.
- Keep organization contexts separate. Cross-organization raw context or
  transcript sharing is forbidden.
- A true zero-input organization is closed by the host as an explicit typed
  result; never invent an empty model or a safe conclusion.

**Stop condition:** every frozen input has exactly one typed disposition and
the final Application Model has exact evidence/source links. Missing, foreign,
or contradictory authority is HOLD, never prose PASS.
