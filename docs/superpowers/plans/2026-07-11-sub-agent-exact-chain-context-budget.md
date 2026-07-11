# Sub-agent exact-chain context budget implementation plan

1. Add RED exact-resume fixtures for bulky tool results, duplicate repair
   directives, durable rewrite, tool-pair validity, and retry idempotence.
2. Add a deterministic history compactor with per-tool structured projections,
   duplicate-directive collapse, atomic tool-turn retention, and a total replay
   ceiling.
3. Apply it on exact/latest restore before provider I/O, before every model
   stream in a long segment, and at final chain persistence.
4. Keep internal full coverage actions for execution guards while model-visible
   repair text uses a bounded page summary (implemented at the repair-mode seam).
5. Classify explicit provider input-context overflow as a typed chain failure,
   expose a stable runtime contract, and make stage-run treat it as
   non-retryable without generalizing ordinary HTTP 400 failures.
6. Run focused tests, the full `golish-sub-agents` suite, formatting, and scoped
   clippy; record any environment blocker rather than claiming completion.
