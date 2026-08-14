# Codex-style durable continuation plan

1. Tighten Stage Team recovery classification and prove that a Controller
   claim reconciles one safe child before reading its aggregate barrier.
2. Add an epoch-guarded EAS port-producer admission namespace to
   `operation_state`; preserve it across generic checkpoint writes and reset it
   only for a new EAS epoch.
3. Add a trusted managed-process policy deadline without changing generic
   bounded-yield semantics. Wire it only to the fixed full-port wrapper.
4. On repeated/exhausted full-port admission, persist guarded blocked evidence
   and monotonic LIVENESS/PORT outcomes without network launch.
5. Run focused DB, app-core and pentest-app tests, scoped Clippy/rustfmt and
   diff checks.
6. Rebuild once, resume the retained `moresec` operation from the exact CLI
   source, and inspect `run_tree.py --db --full`, transcript and run log. Do not
   ask the user to send another continuation during this acceptance.
7. For unified Investigation, add exact persisted-WorkItem, dispatch and
   Refiner-patch reads; allow only the witnessed parked Primary to resume as a
   read-only planning binding, then reuse the sealed Main read-session instead
   of recomputing context or methodology RAG.
8. Prove restart behavior with focused runtime and embedded-Postgres tests,
   then resume the retained Moresec operation through all Analysis children,
   Verification Campaign closure and concise Reporting. Record exact DB sets
   and zero Prepared Action/authorization/execution counts.

## 2026-08-11 completion evidence

- Runtime nextest run `1d5abe78-ad46-47fc-a6e5-0c7b257d6fc5`: 5/5 passed,
  covering outer request identity, parked Primary, durable child identity and
  planning-only continuation.
- Embedded-Postgres nextest run `a9908545-899c-48f7-aece-8bf16ba19667`: 1/1
  passed for restart-time parked Primary/child reload.
- Scoped Clippy for `golish-db`, `golish-agent-kit`, `golish-agent-app` and
  `golish-agent-runtime`: exit 0 with `-D warnings`.
- Entity operation `4d5f17a5-88f5-423e-9dcb-3e9cad6e1003`: eight StageRuns
  completed; task `finished`; four Campaigns terminal; four inconclusive
  FactDelta bundles; zero Prepared Actions, authorizations and executions;
  one validated unpublished report with 3 sections, 55 claims, 77 citations
  and 214 frozen source members.
