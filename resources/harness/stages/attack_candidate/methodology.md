**Goal:** decide every item in the server-frozen Candidate V2 manifest. This is
a *reasoning* stage, not a scanning stage — it runs no scan tools. The manifest
has exactly one sealed entry kind: the initial Wave consumes
`vuln_triage_handoff`; a follow-on Wave consumes `fact_delta_consolidation`.
You read the information-gathering context (assets, services, fingerprints,
endpoints, params), the sealed entry, and injected RAG prior knowledge (wiki
writeups / CVE leads), and produce candidates worth really attacking in the
`verification` stage. Submit only `candidate_decisions[]`; operation, scope,
organization, wave, execution, submission, Candidate id and execution plan are
server-owned and must never appear in the model wire. A zero-input organization
unit is terminal: do not start an analyst worker and do not invent a placeholder
manifest or decision.

**What a good decision is:**

- A concrete `hypothesis` (e.g. "IDOR on `/orders/{id}` exposes other tenants'
  orders", "SSRF via `url=` parameter on the PDF renderer reaches cloud
  metadata").
- A `rationale` that explains WHY it is credible, tied to real observation.
- Non-empty `evidence_refs` from that exact frozen work item.
- `decision="candidate"` requires `hypothesis`; optional `technique` must equal
  the frozen technique.
- `decision="no_candidate"` requires a stable `no_candidate_reason_code` and
  evidence-backed rationale. It is the explicit I8 terminal state, not silence.
- Treat evidence outcome semantics literally. `blocked` means the check did not
  complete; it is neither a negative result nor proof of WAF/rate limiting,
  target resistance, or any other cause. Name a blocker cause only when that
  exact cause is present in trusted evidence supplied to this turn. Otherwise
  say only that the check was blocked and keep the resulting uncertainty in the
  rationale.
- `not_applicable` applies only to the exact technique and producer represented
  by that evidence row. It must not be generalized into "the target is safe".

**Recommended sequence:**

1. Review context — query existing assets/endpoints/params and the current
   server-frozen Wave manifest selected by its sealed entry kind. Read the
   injected PRIOR KNOWLEDGE section (RAG wiki/graph prior) for reusable exploit
   patterns against the observed fingerprints.
2. Reason class by class over the technique surface that `vuln_triage`
   deliberately left for you — SSRF, SSTI, LFI/path traversal, auth-bypass logic,
   business logic — plus the "suspicious but unconfirmed" leads the tool sweeps
   surfaced (deep SQLi/XSS, deep IDOR/object-relationship abuse, WAF-bypass
   chains).
3. For every `work_item_key`, submit exactly one `candidate` or `no_candidate`
   decision. Unknown, duplicate, or omitted keys BLOCK the Gate.
4. Submit the StageDeliverable with `candidate_decisions[]`. `findings` and
   legacy `candidates[]` stay empty. After final Gate PASS the server classifies
   immutable verifier plan/hash/risk and accepts the complete batch atomically.
   The accepted plans then enter durable Candidate review. Approval or rejection
   is bound to the exact immutable plan, and review/resume is recovered from
   database truth rather than process-local state. A generic stage approval does
   not authorize a Candidate V2 plan.

**Efficiency red lines:**

- Do NOT run scanners or exploit tools in this stage — that is `vuln_triage`
  (formulaic) and `verification` (real exploitation). This stage only reasons.
- Do NOT invent hypotheses with no observational basis; every decision must be
  grounded by a concrete rationale and non-empty evidence ids from its exact
  work item. Foreign or unlinked ids fail closed.
- Do NOT invent why a producer returned `blocked`. In particular, never rewrite
  a template/configuration/tooling blocker as "blocked by WAF" unless trusted
  evidence explicitly says WAF.
- Prefer a few high-quality, well-grounded candidates over a long list of
  speculative ones — each approved candidate costs real exploitation budget in
  `verification`.

**Coverage + stop condition:**

- Every frozen work item must terminate exactly once with rationale + evidence.
- If there is genuinely nothing worth attacking, submit an evidence-backed
  `no_candidate` decision for every work item. An empty decision array never
  passes while work remains; the reporting bail edge is taken only after the
  complete manifest is terminal.
- The analyst never accepts a FactDelta, decides a consolidation outcome, or
  opens the next Wave. After all organization units are terminal, durable global
  consolidation alone returns `opened_next_wave`, `closed_no_delta`, or
  `exhausted`; an exhausted fuel budget closes the pipeline with explicit
  residual risk instead of silently dropping proposed deltas.
