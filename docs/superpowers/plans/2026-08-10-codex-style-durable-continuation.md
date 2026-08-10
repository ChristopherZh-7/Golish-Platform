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
