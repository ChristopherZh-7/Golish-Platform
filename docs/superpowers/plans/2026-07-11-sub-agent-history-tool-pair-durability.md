# Sub-agent history tool-pair durability implementation plan

1. Reproduce the live chain tail and prove the exact missing provider call id.
2. Add red tests for dangling history, barrier result ids, and SSE item errors.
3. Make barrier/mixed-batch dispatch return one result for every assistant call.
4. Append the result turn before barrier or stage-stall control-flow exits.
5. Validate tool-call/result pairing on both durable write and exact/latest read.
6. Fail the worker on provider stream errors without persisting the partial turn.
7. Track unsuccessful sub-agent results as failed dispatches.
8. Run focused tests, both full crate suites, clippy, fmt, and diff checks.
9. Re-run Test1 Enumeration and inspect transcript, run log, database truth, and
   unchanged target counts before marking the feature passing.
