# Stage Team downstream Company Controller convergence

> Status: approved for implementation by the user's 2026-07-16 request; no schema migration.

## Problem

`stage_run` currently presents one product concept but executes two orchestration contracts. `target_intel`
declares `team_scheduler`, so every company receives a durable Company Controller, chain-local `update_plan`,
bounded child WorkItems, and the DB-backed `StageTeamRunView`. `external_attack_surface`, `enumeration`, and
`vuln_triage` have no Team policy, so the runtime silently falls back to the legacy one-specialist-per-company
path and the frontend renders `Main Agent -> Prober/Enumerator/Vuln Scanner` cards. The tool name, detail
surface, sub-agent tree, plan ownership, and recovery semantics therefore disagree across adjacent stages.

## Decision

The three downstream company-scoped collection stages join the same Company Controller V1 scheduler:

| Stage | Frozen executor specialist | Allowed child role | Allowed request kinds | Risk lane |
|---|---|---|---|---|
| `external_attack_surface` | `prober` | `prober` | `surface_probe`, `coverage_recheck` | `active_recon` |
| `enumeration` | `enumerator` | `enumerator` | `content_enumeration`, `coverage_recheck` | `active_recon` |
| `vuln_triage` | `vuln_scanner` | `vuln_scanner` | `formulaic_scan`, `coverage_recheck` | `formulaic_scan` |

Every policy also allows `company_stage_controller`. The durable Unit's frozen specialist, not the model's
role string, selects the executable sub-agent definition. The Controller and its children therefore retain
only that stage's existing wrapper/tool allowlist. A Controller may work directly or dispatch bounded children;
only the Controller owns `update_plan` and final submission. Scope authority, evidence, coverage and Gate PASS
remain server-owned.

The frontend needs no stage-specific card. Exact `operation_id + stage_execution_id` progress pointers must
route all four Team stages through the existing DB-backed `StageTeamRunView`; the legacy card remains only for
old executions without an exact Team pointer.

## Explicit exclusions

- `attack_candidate` and `verification` do not join the general Team scheduler. They keep the Wave and
  CandidateAttempt recovery/fencing contracts.
- Post-exploit, Reporting and Cleanup retain their typed stage-specific schedulers.
- No migration, generated IPC edit, scope widening, real external-target scan, or Gate relaxation is part of
  this change.
- The current no-migration schema permits one Controller Gate repair for a stable WorkerRun. Runtime freezes
  `max_controller_gate_repairs=1` and fails closed before a second durable gap-source insert; more than one
  Controller Gate repair still requires the separately documented forward-migration boundary.

## Acceptance

1. Embedded specs for Target Intel, EAS, Enumeration and Vuln all seed exactly one `leader:primary` Controller.
2. Frozen stage specialists map Controller and allowed child roles to `recon`, `prober`, `enumerator`, or
   `vuln_scanner`; cross-stage role reuse fails closed.
3. All four stages expose Controller-local `update_plan`, while ordinary children do not.
4. Exact downstream Team progress renders Company Controller UI and never the legacy `Main Agent -> Specialist`
   card.
5. An isolated loopback CLI slice from EAS through Vuln, together with the already accepted Target Intel Team
   path, shows `company_controller_v1`, one Controller per company, terminal per-stage Gate truth, and no
   external target in transcript/DB truth.

## Implementation result (2026-07-16)

The three downstream specs now declare the bounded Team policy and runtime resolves both Controller and child
execution from the Unit's frozen specialist. Controller parking drops the live heartbeat before child drain.
Only Intel/EAS use compatibility terminal-coverage materialization after Gate PASS; Enumeration/Vuln retain
their producer-owned authoritative outcomes. Legacy `agent_logs` folds newly precise scanner roles to the
existing `pentester` enum only at telemetry persistence, while runtime/UI identity remains precise.

The localhost CLI exposed one additional recovery contract: anonymous-access plan validation filters the
eligible endpoint set more narrowly than the general endpoint query. A mismatch response now returns the
sorted server-owned `eligible_endpoint_ids`, count, retained partial result, and exact retry action, without
exposing new URL/path authority. This lets the Controller repair the request instead of guessing subsets.

Final isolated acceptance used workspace
`/private/tmp/golish-downstream-v2only-finalfix-20260716-bsYnCm`, session
`stage-run-2cebfd1b-87cf-4863-97b6-df263032aead`, operation
`9599a356-58be-40f7-b34f-19754a607976`, and a fixture bound only to `127.0.0.1:54610`.
EAS, Enumeration and Vuln each exited PASS under `company_controller_v1`; DB truth shows the frozen
`prober`, `enumerator`, and `vuln_scanner` specialists, three passed Controllers, durable submissions/handoffs,
and evidence-bound terminal techniques. No heartbeat-loss, invalid telemetry enum, duplicate gap-source, or
Company Controller failure signature appears in the final run.
