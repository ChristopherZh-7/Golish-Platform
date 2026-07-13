**Goal:** drive every ever-approved exact Candidate in the current frozen
WaveUnit to one persisted terminal Attempt: `verified`, `refuted`, or `blocked`.
This remains controlled, foreground-only, bounded verification. Completion is
decided from exact database truth, never from this stage's prose or deliverable.

**Inputs you consume:**

- The scheduler binds one opaque CandidateAttempt at a time. Do not select or
  submit Candidate, approval, Attempt, plan hash, operation, scope, or org IDs.
- Inherited evidence (vuln_triage findings, enumeration endpoints/params) and the
  RAG prior knowledge already injected into this charter.

**Recommended sequence (per approved candidate):**

1. Execute only approved action ordinals through
   `verify_execute_candidate_action`; canonical arguments and budget are
   reloaded under the WorkerRun/lane fence.
2. Inspect exact recent evidence and call `submit_candidate_attempt` once:
   - Its `disposition` field is the CandidateAttempt terminal decision and must
     be exactly `verified`, `refuted`, or `blocked`; it is not a Finding report
     and `StageDeliverable.findings` has no authority in V2.
   - `verified` requires proof evidence and a complete Finding draft. Only the
     terminalizer may create the Finding and immutable lineage.
   - `refuted` requires refutation evidence and creates no Finding.
   - `blocked` requires blocker evidence or a stable snake-case reason code and
     creates no Finding.
3. FactDelta drafts must name an exact canonical ref/version/hash and evidence.
   Task 10 alone decides whether persisted deltas open another wave.

**Efficiency red lines:**

- Sandbox + non-destructive + reproducible: no destructive actions, no
  data-changing payloads. A PoC must be replayable.
- Do not call `record_finding`, a formulaic scanner, generic `pentest_run`, raw
  shell, background controls, or another sub-agent.
- The V2 Gate ignores `StageDeliverable.findings`, summaries, memory/KG,
  spawned candidates, and process-local chain-wave state.

**Coverage + stop condition:**

- PASS requires a non-empty set of operation/scope/wave/unit/org exact DB
  snapshots with closed review, zero pending manifest work, and one valid
  terminal Attempt per ever-approved Candidate.
- A zero-approved WaveUnit passes only when it exists, review is closed, and
  every frozen work item is an evidence-backed `no_candidate` decision.
- Missing, foreign, malformed, or failed DB reads BLOCK; there is no deliverable
  fallback. Reporting reads terminal Candidate/Finding lineage from storage.
