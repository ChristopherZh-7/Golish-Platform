# Candidate Technique Method Contracts Implementation Plan

## Goal

Make fresh Candidate hypothesis generation method-card-bound and fail closed,
without loading CyberStrike at runtime, changing the database schema, or
weakening Plan C's typed oracle/Finding boundary.

## Tasks

1. Add a pure `golish-core` technique-method registry with deterministic card
   snapshots, digests, lookup, attack-class filtering and exact-set hashes.
2. Extend Candidate analyst/checklist/proposal DTOs with host-projected method
   cards, exact card-set authority and typed prerequisite assessments.
3. Inject the frozen catalog into every analyst microbatch and every coverage
   checklist member; update closed prompts to require card selection and gap
   preservation.
4. Validate proposal card identity, predicate binding, signals, prerequisite
   exact set, proof roles and readiness before persistence.
5. Carry the binding/digest through conflict summaries, host compiler recipe,
   revision ingredients and Verification contract rule/prerequisite hashes.
6. Update focused fixtures and add RED/GREEN tests for unknown card, digest
   drift, missing prerequisites and illegal `ready_for_strategy` escalation.
7. Update module cards, index, feature list and progress evidence; run only the
   focused checks authorized by the request, with `just space-guard` before
   every Cargo invocation.

## Focused verification

```bash
cd backend && just space-guard
cargo nextest run -p golish-core -E 'test(candidate_technique_)' --status-level fail
cargo nextest run -p golish-agent-app -E 'test(candidate_technique_) | test(candidate_analysis_runtime)' --status-level fail
cargo nextest run -p golish-sub-agents -E 'test(candidate_hypothesis_)' --status-level fail
cargo clippy -p golish-core -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-sub-agents --all-targets -- -D warnings
```

No entity run, external target request, migration, generated IPC regeneration,
commit, stage or push is part of this slice.
