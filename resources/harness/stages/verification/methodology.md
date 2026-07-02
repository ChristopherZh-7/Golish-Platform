**Goal:** really attack the APPROVED candidates from `attack_candidate` and drive
each to a terminal disposition. This is the exploitation stage — controlled,
sandboxed, non-destructive PoC confirmation. The gate
(`candidate_disposition_complete`) requires every `approved` candidate to reach
one of `verified` / `refuted` / `blocked`; a `verified` one must carry
`poc`/`exploit_verified` evidence (finding_verification also enforces strong
evidence on high+ findings).

**Inputs you consume:**

- The candidate list from `attack_candidate` — work only the ones the human
  `approved` at the `exploit_validation` gate (proposed/rejected are not yours to
  run).
- Inherited evidence (vuln_triage findings, enumeration endpoints/params) and the
  RAG prior knowledge already injected into this charter.

**Recommended sequence (per approved candidate):**

1. Reproduce the hypothesis with the smallest safe PoC. Prefer read-only /
   non-destructive proofs (e.g. read another tenant's record for IDOR, reflect a
   benign marker for XSS, resolve an internal host for SSRF) — never destructive
   payloads.
2. Set the disposition honestly:
   - `verified` — the PoC proves impact. Attach `poc` / `exploit_verified`
     evidence and mint a `HarnessFinding` with real `evidence_refs`. Set
     `parent_finding_id` on any follow-on candidate so the a→b→c attack chain is
     captured.
   - `refuted` — you proved it is a false positive. Cite the evidence that
     disproves it (I8: refuted is a checked terminal state, NOT a skip).
   - `blocked` — a WAF / auth / authorization boundary stopped you. Record a note
     explaining the blocker (also a terminal state).
3. Chain forward. If a verified finding opens a NEW surface (a→b), propose the
   follow-on as a fresh candidate with `parent_finding_id`; the chain-wave
   controller decides whether to open another wave (bounded by dedupe + fuel +
   depth) or converge to reporting.

**Efficiency red lines:**

- Sandbox + non-destructive + reproducible: no destructive actions, no
  data-changing payloads. A PoC must be replayable.
- Do NOT re-run the formulaic scan here — that was `vuln_triage`. This stage is
  targeted exploitation of the approved hypotheses.
- Every `verified` needs strong evidence (poc/exploit_verified); do not mark a
  candidate verified on a hunch — the gate blocks unevidenced verified.

**Coverage + stop condition:**

- Every `approved` candidate must reach `verified` / `refuted` / `blocked` before
  submit — the `candidate_disposition_complete` gate blocks any still-`approved`
  candidate and any `verified` one missing evidence.
- `verified` candidates become findings (high+ findings carry poc/exploit_verified
  per `finding_verification`); reporting rebuilds the kill chain from the
  candidate lineage + findings.
