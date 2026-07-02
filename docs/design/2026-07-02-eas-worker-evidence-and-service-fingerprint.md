# EAS worker evidence citation + service-fingerprint truth fixes

> 2026-07-02 · Root-cause fix for the `external_attack_surface` (EAS) prober loop
> observed in `pentest-chat-1783002737901-1` (workspace `~/golish-platform/Test1`,
> run 2026-07-02 22:35). Analysed with `scripts/run_tree.py --db` + direct embedded
> Postgres inspection.

## 0. TL;DR

The last EAS run burned all 3 reflector retries on the prober without passing.
Three defects stacked:

1. **A (the actual dealbreaker):** the EAS `every claim must cite evidence` gate rule
   is un-winnable for the worker, because the worker has **no tool that lists the
   run's real evidence ledger ids with enough context to map an id to a claim**.
2. **B (amplifier):** the submit-preview (`check_stage_asset_coverage`) and the
   authoritative gate disagree in time — scan output lands in the business tables
   via a fire-and-forget background hook, so the gate can grade before `nmap -sV`
   has landed while the preflight reads after it lands.
3. **C (data quality):** `tcpwrapped` counts as a SERVICE-FINGERPRINT, while a real
   `nmap -sV` service/version (e.g. MySQL 5.1.35) is **never** written to the
   `fingerprints` table (only into `targets.ports[].service`).

Plus two follow-ons:

4. **D:** subdomain `real_ip`s that only expose port 53 (shared DNS/CDN infra) are
   forced through LIVENESS + PORT + SERVICE-FINGERPRINT with no informative service.
5. **E:** the 3-retry budget (`MAX_REFLECTOR_RETRIES`) is too tight to absorb async
   landing + evidence-id reconciliation.

This document specifies fixes for all five. **A, C2, E are landed in this change**
(additive, do not tighten the gate). **B, C1, D tighten or re-time the authoritative
gate and are specified here + in the plan but staged for a change that includes a
compile + targeted-test cycle** (they can silently wedge the gate if wrong; the
harness gate is invariant I7/I8 territory).

## 1. Evidence (timeline of the failing run)

All times UTC (+8 = local). From `run_tree.py --db` + `run.log`:

| time (UTC) | event |
| --- | --- |
| 14:37:38 | EAS stage starts (freezes the coverage denominator cutoff) |
| 14:51:24 | submit → gate BLOCK on two reasons: (1) `every claim must cite evidence`; (2) `coverage_complete`: 13 IP×SERVICE-FINGERPRINT `never attempted` |
| 14:54:22 | `nmap -sV` `tcpwrapped` fingerprints finally land; 12/13 SERVICE cells flip to found |
| 14:54:53 / 14:55:26 / 14:56:15 | three more submits, all now blocked ONLY on `every claim must cite evidence` |
| 14:57:29 | still waiting on background httpx, retry 3/3 exhausted, run aborts |

The coverage gap (the 13 cells) was closed by 14:54:22 — it was **not** the real
dead-end. The 3 retries were burned on the un-winnable `every claim must cite
evidence` loop (A), aggravated by the landing timing (B). `tcpwrapped` closing the
SERVICE cells is a data-quality smell (C).

## 2. Root cause A — `every claim must cite evidence` is un-winnable for the worker

`resources/harness/stages/external_attack_surface/spec.json`:

```json
{ "op": "for_all", "over": "claims",
  "require": { "pred": "non_empty", "field": "evidence_ids" },
  "on_fail": { "reason": "every claim must cite evidence", ... } }
```

Every claim must carry non-empty `evidence_ids`. The worker's only sources of a real
ledger id today:

- The additive `_evidence_id` echoed on a single foreground tool result
  (`golish-agent-runtime/.../direct/mod.rs:772-780`). Background scans and the
  output-store hook append evidence rows the worker never sees an id for, and the id
  keeps drifting run-to-run (transcript shows 14017 → 14091 → 14115).
- `available_evidence_ids` returned by `submit_stage_deliverable` — but only **after**
  a `needs_fix`, i.e. by using submit as a probe (which the prompt forbids).

`DbRepoProvider::recent_evidence_ids` (`golish-agent-kit/.../db_traits/repo.rs:699`,
impl `golish-db/.../repo/audit/mod.rs:250`) already reads the session's recent real
evidence ids — but returns bare `Vec<i64>` with no tool / asset / technique / kind
context, so even if the worker saw them it could not decide *which* id backs *which*
claim.

### Fix A — `list_recent_evidence` read-only tool

Add a read-only tool `list_recent_evidence` that returns the run's recent real
evidence rows with the context needed to cite them:

```
[{ evidence_id, tool, subject, technique, asset, outcome, kind, age_seconds }, ...]
```

- New repo query `recent_evidence_detailed_for_session(pool, session_id, limit)` in
  `golish-db/.../repo/audit/mod.rs`, selecting from `audit_log WHERE audit_role='evidence'
  AND session_id=$1 ORDER BY id DESC LIMIT $2`, projecting `id, tool_name,
  details/evidence_asset, evidence_technique, evidence_asset, evidence_outcome,
  detail->>'kind', EXTRACT(EPOCH FROM NOW()-created_at)`.
- New trait method `DbRepoProvider::recent_evidence_detailed(session_id, limit) ->
  Vec<serde_json::Value>` (mirrors the `Vec<Value>`-returning shape of
  `in_scope_targets` / `attack_surface_seeds`; default `Ok(Vec::new())` so test
  doubles are unaffected), implemented on `GolishDbRepoProvider`.
- New `FunctionDeclaration` `list_recent_evidence` in
  `golish-tools/.../definitions/security_tools.rs` (params: optional `limit`
  default 25, max 200).
- Route it: add to `is_security_analysis_direct_tool`
  (`golish-agent-runtime/.../direct/mod.rs`) + the `is_sec_tool` match and a handler
  arm in `golish-agent-kit/.../tool_executors/security.rs`.
- Expose it: add to the prober + enumerator + pentester `.with_tools(...)`
  (`golish-sub-agents/.../builder/registry.rs`) and to `READ_ONLY_QUERY_TOOLS`
  (`golish-agent-runtime/.../tool_list.rs`) so the stage orchestrator gets it too.
- Prompt: tell the worker to call `list_recent_evidence` before building the
  deliverable and to cite ids from it (`golish-sub-agents/.../prompts/execution_planning.rs`).
- Frontend display label in `frontend/lib/tools.ts`.

This turns the un-winnable loop into a solvable one: the worker reads the real ids
+ their (tool, asset, technique) context, then cites them. It does **not** relax the
gate — claims still need real evidence — it just makes the required ids discoverable.

We keep the existing `evidence_refs`-can-be-empty design; A is about the per-claim
`evidence_ids` the EAS spec demands.

## 3. Root cause B — preflight vs authoritative gate disagree in TIME (not formula)

Both `check_stage_asset_coverage` (read model `stage_coverage.rs`) and the
authoritative gate (`org_gate.rs` → `validate_stage_gate_with_context`) read the same
DB truth (`coverage_truth_facts` + `technique_outcome_facts`) with the same
`stage_started_at` cutoff. The 351-done-vs-13-never-attempted split was a **timing**
artifact: `pentest_run` appends its evidence row synchronously, but the business-table
landing (`targets`/`fingerprints` via the structured-storage hook) is dispatched with
`tokio::spawn` (`golish-agent-runtime/.../direct/mod.rs:494-503`) — fire-and-forget.
The submit reconciliation barrier (`harness_submit_tool.rs:454`) only waits for
explicitly-backgrounded jobs (`bg_jobs.running_for_session`), not the spawned
output-store hook. So a foreground `nmap -sV` can return, the gate can grade, and the
`fingerprints`/`ports` write can land 3 minutes later.

### Fix B (specified, staged — needs compile+test)

Make the scan output-store landing observable to the submit barrier so the gate never
grades a stage whose scan writes are still in flight. Two candidate implementations:

- **B1 (preferred):** inside a harness stage, `await` the structured-storage hook for
  scan tools (`pentest_run`) instead of `tokio::spawn`-ing it, so `fingerprints`/
  `ports` are landed before the tool result (and thus the id) is returned. Localised
  to `direct/mod.rs`; trade-off is added foreground latency on scan tools.
- **B2:** register the spawned hook as a tracked background job so
  `reconcile_background_jobs` waits for it at submit.

Risk: changes tool-execution timing / the submit barrier; must be verified against
`golish-agent-runtime` tests + a live EAS smoke. Staged, not shipped blind.

## 4. Root cause C — tcpwrapped counts; real -sV service is lost

`build_service_fp_values_sql` (`golish-db/.../repo/coverage_truth.rs:297`) treats
SERVICE-FINGERPRINT as found when either a `fingerprints` row exists **or**
`ports_have_service_hint_sql` matches (`coverage_truth.rs:171`):

```
WHERE NULLIF(p->>'service','') IS NOT NULL OR NULLIF(p->>'version','') IS NOT NULL ...
```

`tcpwrapped` is a non-empty `service`, so a `tcpwrapped`-only port satisfies
SERVICE-FINGERPRINT. Meanwhile `store_fingerprints` (`golish-pentest/.../output_store/targets.rs:264`)
only writes `fingerprints` from `webserver` / `technologies` / `cdn` / `os` fields —
**never** from an nmap `-sV` port `service`/`version`. The nmap parser
(`golish-pentest/.../output_parser.rs:271`) captures the whole port-line tail as
`service` and it lands only in `targets.ports[].service`. So a real MySQL 5.1.35
`-sV` result is never a `fingerprints` row (lost for reporting + the enumeration
handoff).

### Fix C2 (landed here — additive, safe)

In `store_fingerprints`, when the field set carries an informative port `service`
(from `nmap -sV`), also write a `service`-category `fingerprints` row (name = service
product, version = version when parsed). "Informative" excludes `tcpwrapped`,
`unknown`, `open`, empty. This materialises the real service fingerprint so:

- SERVICE-FINGERPRINT becomes legitimately found via a real row (not via tcpwrapped),
- the finding survives into reporting + the enumeration stage handoff.

Purely additive (writes more rows); does not tighten any gate.

### Fix C1 (specified, staged — TIGHTENS the gate)

Exclude non-informative pseudo-services (`tcpwrapped`, `unknown`, `open`, empty) from
`ports_have_service_hint_sql` so a bare `tcpwrapped` port no longer satisfies
SERVICE-FINGERPRINT. This is consumed by **both** the read model and the authoritative
gate (single shared truth), so they stay consistent — **but** it makes the gate
STRICTER: a port-53-only tcpwrapped IP flips SERVICE from found → pending and will
BLOCK unless it has a terminal path (see D). **C1 must ship together with D** and with
a test cycle, because on its own it deadlocks those IPs.

## 5. Root cause D — port-53-only real_ips forced through SERVICE-FINGERPRINT

13 IPs in the failing run were subdomain `real_ip`s (shared DNS/CDN infra) exposing
only port 53. EAS requires LIVENESS + PORT + SERVICE-FINGERPRINT for concrete IPs.
The read model already derives SERVICE `not_applicable` when PORT is
`checked_empty`/`not_applicable` (`stage_coverage.rs::apply_eas_service_dependency`),
but a port-53-open IP has PORT = found, so SERVICE stays pending. The authoritative
gate (`rule_engine.rs`) has **no** such service-dependency derivation at all — it
requires each applicable technique cell to reach a terminal state independently.

### Fix D (specified, staged — TIGHTENS/RESHAPES the gate)

Give a port-found-but-no-informative-service IP a deterministic terminal SERVICE cell
in **both** the read model and the authoritative gate, so C1 does not deadlock them:

- Read model: extend `apply_eas_service_dependency` to also mark SERVICE
  `not_applicable` when PORT is found but the asset has no informative service surface
  (requires threading the asset's port services into the coverage row).
- Gate: mirror the same derivation in `rule_engine.rs` `coverage_complete` (or supply
  a DB-truth `service_fp_not_applicable` set the gate consumes), so preflight and gate
  agree.

Risk: touches the authoritative rule engine (`rule_engine.rs`) which is I7/I8-critical;
must be covered by new unit tests matching the existing `rule_engine`/`stage_coverage`
test suites before shipping.

## 6. Root cause E — retry budget too tight

`MAX_REFLECTOR_RETRIES = 3` (`golish-agent-kit/.../task_orchestrator/types.rs:13`) is
the per-subtask gate-repair budget. With async landing (B) + evidence-id
reconciliation (A) each consuming a turn, 3 is not enough for a healthy EAS run.

### Fix E (landed here — low risk)

Raise `MAX_REFLECTOR_RETRIES` 3 → 5. This is the documented single source of the
repair budget (design 2026-05-26 O3); several docs reference "N=3", so this is a knob
change, not a new constant. Strictly more headroom before `paused_needs_user`.
"Don't count a retry that made progress" is the more surgical variant and is left as a
follow-up (it needs a progress signal from the gate delta).

## 7. What lands in this change vs staged

| Fix | Lands now | Why |
| --- | --- | --- |
| A `list_recent_evidence` | yes | additive read-only tool; the explicit dealbreaker |
| C2 land `-sV` service fingerprint | yes | additive write; materialises real service data |
| E retry budget 3→5 | yes | one-constant knob; strictly more headroom |
| B async-landing barrier | staged | re-times tool execution / submit barrier |
| C1 tcpwrapped exclusion | staged | tightens the authoritative gate; coupled to D |
| D port-53 terminal derivation | staged | touches the I7/I8 rule engine |

Staged fixes are fully located above and in
`docs/superpowers/plans/2026-07-02-eas-worker-evidence-and-service-fingerprint.md`.
They must go through `just check` + targeted `golish-agent-kit` / `golish-db` /
`golish-agent-runtime` tests + a live EAS smoke before flipping to `passing`.

## 8. Invariants touched

- I7 (stage evidence, not prose): A strengthens the worker's ability to cite REAL
  evidence; it does not weaken the requirement.
- I8 ("checked-empty" ≠ "unchecked"): C1/D must preserve this — a not_applicable
  SERVICE cell for an infra IP is "the technique does not apply", never "checked and
  empty without a scan".
- C2 only adds provenance rows; no schema change.
