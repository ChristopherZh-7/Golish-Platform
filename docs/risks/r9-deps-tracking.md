# R9 — Bleeding-Edge Dependency Tracking

> Status: **Risk identified, mitigation = quarterly cadence + pinned ownership.**
> Last updated: 2026-05-02.

## Current state

The frontend stack (`package.json`) is on the absolute upstream
edge of the JS ecosystem:

| Dependency | Version | Released | Risk profile |
|---|---|---|---|
| TypeScript | 6.0.3 | 2026-04 | major release; full ecosystem still adapting |
| React | 19.1 | 2025 | new compiler / async rendering semantics |
| Vite | 8.0.10 | 2026 | major; plugin compatibility risk |
| Tailwind CSS | 4.1.17 | 2025 | new engine; some plugins lag |
| `@vitejs/plugin-react-swc` | 4.3.0 | 2026 | tracks Vite 8 |
| Vitest | 4.0.14 | 2026 | major; **see D1 — 90 test failures from React 19 compat** |
| `@testing-library/react` | 16.3.0 | matches React 19 | OK |

Backend (`backend/Cargo.toml`) is more conservative:
- Rust edition 2021, stable toolchain.
- `tauri = "2"` (mature).
- `sqlx 0.8`, `tokio 1`, `rig-core 0.36` — all current-stable.

## Why this is a P3 risk (lurking, not on fire)

- **Compatibility surprise**: a major dep release in any of TS/React/Vite
  can break the build between `pnpm install` runs. We just saw it in D1
  (vitest 4 + React 19 = 90 test failures).
- **Security advisories** in newly-released code are higher density —
  dependabot will start opening PRs the moment 0-days drop on TS 6.x.
- **Onboarding tax**: a contributor whose machine has Node 18 may not
  realise Vite 8 needs Node 20+.
- **Downstream library lag**: many React libraries still target React 18;
  silent prop-type warnings creep in.

## Mitigation cadence

### Per-quarter dep-audit (4h / quarter)
1. Run `pnpm outdated` — collect majors that have moved upstream.
2. For each major, decide: pin / upgrade / hold.
3. Record decisions in `docs/risks/r9-deps-tracking.md` (this file).
4. Run `cargo outdated --workspace` — same for backend.

### Per-PR guardrails
1. Pin Node engine in `package.json`:
   ```json
   "engines": { "node": ">=20.0.0", "pnpm": ">=9.0.0" }
   ```
2. Renovate / Dependabot config that **does NOT** auto-merge majors.
3. CI check that fails if `package-lock.json` and `pnpm-lock.yaml`
   are out of sync (or remove one — currently both exist, see
   "Stray lockfiles" below).

### Stray lockfiles
The repo currently ships **two** lockfiles:
- `pnpm-lock.yaml` (303KB) — the canonical one (`pnpm-workspace.yaml` is set up).
- `package-lock.json` (369KB) — leftover from a bygone npm flow.

This is bug-bait: an absent-minded `npm install` updates one and not
the other. **Action**: delete `package-lock.json`, add to `.gitignore`.

## Owner assignment

R9 needs a **named human** (rotating quarterly) responsible for the
dep audit. Without that, the cadence rots. The original architecture
report's R9 listed "needs long-term owner" — until that owner exists,
this risk persists.

Suggested protocol: at the start of each quarter, pick one engineer
to file a `chore(deps): YYYY-Q? audit` PR with the audit results.
The PR review forces the team to discuss any majors before they
auto-update.

## References

- `package.json` — frontend deps
- `backend/Cargo.toml:97-263` — backend deps
- Architecture eval R1-R10 — legacy architecture review; source review file may no longer be present
