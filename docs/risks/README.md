# Risk-Analysis Docs

This directory captures architectural risks identified by the
2026-05-02 architecture review. Some source review files lived under
`.cursor/rules/` in earlier snapshots and may no longer be present; treat the
risk docs here and `docs/architecture.md` as the current references.

Each file follows the template:

1. **Status** — open / mitigated / deferred.
2. **Current state** — observable evidence (file paths, command output,
   metrics).
3. **Why this matters** — concrete business / engineering impact.
4. **Mitigation tracks** — ranked options with effort estimates.
5. **Recommendation** — what to do next, with a clear owner / date.
6. **References** — code, ADRs, upstream docs.

## Index

| Doc | Risk | Severity | Owner |
|---|---|---|---|
| [r4-pg-embed.md](r4-pg-embed.md) | Embedded Postgres bundle / startup cost | P2 | unassigned |
| [r5-rig-fork-maintenance.md](r5-rig-fork-maintenance.md) | 4 rig-core forks rebase tax | P2 | unassigned |
| [r9-deps-tracking.md](r9-deps-tracking.md) | Bleeding-edge frontend deps (TS6/Vite8/React19) | P3 | unassigned |
| [r10-tauri-capabilities.md](r10-tauri-capabilities.md) | 548 IPC commands all default-allowed | P2 | unassigned |
| [d1-vitest-react19.md](d1-vitest-react19.md) | Vitest+jsdom incompat with React 19 (90 fails) | P2 | unassigned |

Risks already mitigated in this same review cycle (see CHANGELOG):

| Risk | Mitigation commit |
|---|---|
| R1 — IPC namespace flat 548 commands | Phase 1 + Phase 2A-D facade rollout |
| R3 — Slow Rust cold compile | sccache CI + docs |
| R6 — Frontend store slice growth | scripts/check_store_slices.sh + arch-check.yml |
| R7 — Stale root artefacts (`nmap_scan.txt` etc) | One-shot cleanup + .gitignore |
| R8 — Frontend error reporting | Verified existing ErrorBoundary + setupGlobalErrorHandlers |
| N1 — Cargo.lock not committed | Commit + .gitignore comment |
| N2 — Type duplicates (PipelineSummary collision) | Renamed to PipelineRunSummary |
| N5 — workspace.rs catch-all | Split into vault/wiki/findings sub-facades |
| N4 — IPC traceId | Frontend 50% — `lib/api/client.ts` + logger thread trace id |

## Process

When you start work on a risk:

1. Move the doc's `Owner: unassigned` to your name.
2. Update `Status:` from `open` to `in-progress`.
3. PR title prefix: `risk(rN): …`
4. After landing, update the doc's `Status:` to `mitigated` and
   the table above to add the mitigation commit hash.

Risks not in this directory but tracked elsewhere:
- **R2 (Tauri CSP null)** — security; explicitly deferred per
  product owner directive 2026-05-02.
