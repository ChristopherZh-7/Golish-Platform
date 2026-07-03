**Goal:** synthesize a set of structured, grounded attack HYPOTHESES
(`AttackCandidate`) from everything gathered so far. This is a *reasoning* stage,
not a scanning stage — it runs no scan tools. You read the information-gathering
context (assets, services, fingerprints, endpoints, params), the formulaic
`found` results from `vuln_triage`, and the injected RAG prior knowledge (wiki
writeups / CVE leads), and you produce candidates worth really attacking in the
`verification` stage. The output is reviewable: a human approves or rejects the
list at the `verification` entry (`exploit_validation`).

**What a good candidate is:**

- A concrete `hypothesis` (e.g. "IDOR on `/orders/{id}` exposes other tenants'
  orders", "SSRF via `url=` parameter on the PDF renderer reaches cloud
  metadata").
- A `rationale` that explains WHY it is credible, tied to real observation.
- Non-empty `evidence_refs` citing the fingerprint / endpoint / finding that
  makes the hypothesis plausible (the `candidate_grounded` gate BLOCKS any
  candidate with an empty rationale or no evidence — no hypotheses from thin
  air).
- Optionally a `technique` (WSTG / ATT&CK id), `prior_refs` (wiki/CVE), a
  `suggested_approach`, and a `priority`.
- When it is derived from a confirmed vuln, set `parent_finding_id` so the
  attack-chain lineage (a→b→c) is captured.

**Recommended sequence:**

1. Review context — query existing assets/endpoints/params and the
   `vuln_triage` findings + coverage. Read the injected PRIOR KNOWLEDGE section
   (RAG wiki/graph prior) for reusable exploit patterns against the observed
   fingerprints.
2. Reason class by class over the technique surface that `vuln_triage`
   deliberately left for you — SSRF, SSTI, LFI/path traversal, auth-bypass logic,
   business logic — plus the "suspicious but unconfirmed" leads the tool sweeps
   surfaced (deep SQLi/XSS, deep IDOR/object-relationship abuse, WAF-bypass
   chains).
3. For each high-value asset/endpoint, either produce one or more grounded
   candidates OR record an explicit "no candidate + reason" claim — silence is
   not coverage.
4. Chain thinking — if a confirmed finding opens a new surface, propose the
   follow-on candidate with `parent_finding_id` set; the chain-wave controller
   uses this to open the next wave.
5. Submit the StageDeliverable with your `candidates[]` populated (and summary
   claims citing evidence). `findings` stays empty here — findings are minted in
   `verification` after a candidate is actually proven.

**Efficiency red lines:**

- Do NOT run scanners or exploit tools in this stage — that is `vuln_triage`
  (formulaic) and `verification` (real exploitation). This stage only reasons.
- Do NOT invent hypotheses with no observational basis; every candidate must be
  grounded by a concrete rationale. Evidence ids are optional model fields; the
  backend resolves proof from ledger/DB truth.
- Prefer a few high-quality, well-grounded candidates over a long list of
  speculative ones — each approved candidate costs real exploitation budget in
  `verification`.

**Coverage + stop condition:**

- Every candidate must pass `candidate_grounded`: non-empty `rationale`.
- If there is genuinely nothing worth attacking, produce zero candidates — the
  `attack_candidate → reporting` bail edge lets the operation converge instead of
  manufacturing junk hypotheses.
- Do not duplicate a hypothesis already tested in a previous wave; the
  controller de-duplicates by `(target, technique, normalized hypothesis)`.
