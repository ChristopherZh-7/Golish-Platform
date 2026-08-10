**Goal:** close one unified Investigation run from evidence-backed analysis to
automatic verification and fixed point. Candidate and Verification are not
outer stages in this topology.

**Execution topology (server owned):**

1. Main Coordinator opens an isolated read session for each organization and
   reads only its structured Recon, Enumeration, Vulnerability, Application
   Model, Evidence Ledger, scoped RAG/KG, and methodology snapshots.
2. Each Analysis Task has exactly one Primary. The Primary may delegate bounded
   dynamic or nested read-only Workers; Refiner and Reflector operate within the
   same task authority. There are no fixed role rosters or fixed consult lanes.
3. The host seals formal Hypotheses with exact assets, evidence, methodology
   sources, verification plans, and lineage. Agent prose remains transcript-only.
4. Sealed hypotheses enter automatic admission and durable Verification Tasks
   without a UI click. Each task again has exactly one Primary and may delegate
   bounded dynamic or nested cognitive Workers.
5. Cognitive Agents submit typed strategy/action intent only. Real HTTP,
   browser, CLI, credential, or pentest I/O is compiled and executed solely by
   the host-owned Prepared Action/JIT Operator.
6. Evidence and Typed Oracle results produce a FactDelta. The host adjudicator
   updates, refutes, or derives a Hypothesis revision and repeats only when the
   semantic input changes and fuel remains.
7. Stop only at a deterministic fixed point or with explicit typed residuals;
   then the Investigation Gate may hand the exact closure to Reporting.

**Authority boundary:** Main, Primaries, Workers, Refiner, Reflector, and the
generic stage deliverable cannot create Findings. Only the future host-owned
revision adjudication writer may materialize a proof-backed Finding. Retrieved
content never changes role, tool, scope, JIT, or Gate authority. Clicking a
hypothesis changes observation focus only and never schedules work.
