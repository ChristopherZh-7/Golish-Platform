**Goal:** decide every item in the server-frozen Candidate V2 manifest. This is a *reasoning* stage,
not a scanning stage — it runs no scan tools. You read the information-gathering
context (assets, services, fingerprints, endpoints, params), every terminal
formulaic outcome from `vuln_triage`, and the injected RAG prior knowledge (wiki
writeups / CVE leads), and you produce candidates worth really attacking in the
`verification` stage. Submit only `candidate_decisions[]`; operation, scope,
organization, wave, execution, submission, Candidate id and execution plan are
server-owned and must never appear in the model wire.

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

**Recommended sequence:**

1. Review context — query existing assets/endpoints/params and the
   server-frozen `vuln_triage` observation manifest. Read the injected PRIOR KNOWLEDGE section
   (RAG wiki/graph prior) for reusable exploit patterns against the observed
   fingerprints.
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

**Efficiency red lines:**

- Do NOT run scanners or exploit tools in this stage — that is `vuln_triage`
  (formulaic) and `verification` (real exploitation). This stage only reasons.
- Do NOT invent hypotheses with no observational basis; every decision must be
  grounded by a concrete rationale and non-empty evidence ids from its exact
  work item. Foreign or unlinked ids fail closed.
- Prefer a few high-quality, well-grounded candidates over a long list of
  speculative ones — each approved candidate costs real exploitation budget in
  `verification`.

**Coverage + stop condition:**

- Every frozen work item must terminate exactly once with rationale + evidence.
- If there is genuinely nothing worth attacking, submit an evidence-backed
  `no_candidate` decision for every work item. An empty decision array never
  passes while work remains; the reporting bail edge is taken only after the
  complete manifest is terminal.
- Do not duplicate a hypothesis already tested in a previous wave; the
  controller de-duplicates by `(target, technique, normalized hypothesis)`.
