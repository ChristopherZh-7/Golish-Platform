# Scoping session lifecycle and trusted-review implementation plan

1. Add regression coverage for a shared tracker session identity and for an empty
   trusted target snapshot being an organization-only no-op.
2. Make `DbTracker` session identity shared across clones, expose immutable bridge
   rebinding, and bind it to the durable TaskMode session immediately after upsert.
3. Add `customer_provided` to the trusted snapshot query and keep non-empty review
   comparison strict.
4. Make the lifecycle read retain every review attempt and reject repeated review;
   make org-bound customer target import identity-aware and legacy-claim guarded.
5. Align Scoping prompts/methodology with the conditional target review contract.
6. Run focused tests and a no-GUI regression; record exact commands and outcomes in
   `agent-progress.md` and `feature_list.json`. Focused tests/checks are complete;
   the fresh no-GUI operation remains pending explicit authorization because it
   invokes the configured external LLM.
7. Reproduce the second live-run blocker and add a red gate test for explicit
   parent-only scope completing without an empty `unit_review`.
8. Split the subsidiary contract into root-only versus included branches; bind
   choice/proposal/review lifecycles to the trusted root and reject skipped,
   failed, malformed, or out-of-order included-branch actions.
9. Add a frontend fake-timer regression and disable auto-confirm for structured
   and legacy subsidiary-scope choices. Re-run focused Rust and frontend suites;
   leave the feature `in_progress` until a restarted live run proves no retry.
