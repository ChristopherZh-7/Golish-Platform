# Target Surface Workbench Implementation Plan

- **Date**: 2026-05-28
- **Feature**: `target-surface-workbench`
- **Design**: `docs/design/2026-05-28-target-surface-workbench.md`
- **Mock**: `.codex-screenshots/target-surface-workbench-mock.svg`

## Goal

Turn the post-ZAP Target Manager into a target-centric recon workspace. Keep the organization tree as the spine, but promote target recon data into a formal Target Surface Workbench.

## Constraints

- Do not reintroduce ZAP or SecurityView.
- Do not modify DB schema in the first slice.
- Use existing frontend API wrappers; no naked `invoke()` from components.
- Async UI paths need loading / error / empty states.
- Existing organization workflows must keep working.
- Evidence/provenance must stay visible for recon output.

## Task 1: Clean Current Copy

Files:

- `frontend/components/TargetPanel/TargetGroupedView.tsx`
- `frontend/components/TargetPanel/NewEngagementDialog.tsx`
- `frontend/lib/i18n/en.json`
- `frontend/lib/i18n/zh-CN.json`

Steps:

1. Replace old "Network/List" empty-state copy with Org Tree / Graph language.
2. Replace "Discovery orchestration is not wired yet" with current behavior:
   - creates org
   - preserves discovery settings
   - guides user to run `Discover subsidiaries` / `Enrich profile`
3. Ensure English and Chinese strings describe the same workflow.

Verification:

- `pnpm exec biome check` on touched frontend files.
- `pnpm exec tsc --noEmit`.

## Task 2: Extract Target Surface Data Hook

Files:

- `frontend/components/TargetPanel/hooks/useTargetSurfaceData.ts`
- `frontend/components/TargetPanel/TargetDetail.tsx`
- tests if current test harness has a natural place

Steps:

1. Move the current parallel calls out of `TargetDetailView`:
   - `targetAssetsList`
   - `apiEndpointsList`
   - `fingerprintsList`
   - `jsAnalysisList`
   - `oplogListByTarget`
2. Return `{ data, loading, error, reload }`.
3. Preserve current inline detail behavior until the new workbench is wired.
4. Keep failures visible in the new workbench instead of swallowing all errors.

Verification:

- `pnpm exec tsc --noEmit`.
- Targeted Vitest if a nearby hook/component test exists.

## Task 3: Add Target Surface Workbench Component

Files:

- `frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`
- `frontend/components/TargetPanel/TargetSurfaceTabs.tsx` if useful

Steps:

1. Build the workbench shell:
   - target heading
   - scope badge
   - source/evidence metadata
   - staged action buttons
   - tabs: `Identity`, `Surface`, `Sitemap`, `JS / API`, `Sensitive`, `Evidence`
2. Implement `Identity` from existing `Target` fields.
3. Implement `Surface` from target ports + fingerprints + HTTP metadata.
4. Implement `JS / API` from `jsAnalysisList` and `apiEndpointsList`.
5. Implement `Evidence` from `oplogListByTarget` initially.
6. Add honest empty states for `Sitemap` and `Sensitive`.

Verification:

- Component renders with no target.
- Component renders with a target with no recon data.
- Component renders with mock ports / JS / endpoints.

## Task 4: Wire Selection Into TargetGroupedView

Files:

- `frontend/components/TargetPanel/TargetGroupedView.tsx`
- `frontend/components/TargetPanel/TargetDetail.tsx`

Steps:

1. Add `selectedTargetId` state.
2. Clicking a target selects it and opens the workbench.
3. Keep inline target editing separate from workbench selection.
4. When selected org changes, clear selected target if it is not under that org.
5. Keep current org workspace tabs intact.

Verification:

- Existing target CRUD still works.
- Target row selection does not trigger accidental edit/delete/toggle actions.
- Keyboard/hover states remain readable.

## Task 5: Visual QA

Steps:

1. Run `pnpm dev` or use existing Vite server.
2. Open Target Manager in the in-app browser.
3. Verify:
   - empty state copy is current
   - org workspace still works
   - selecting a target opens the workbench
   - text does not overlap at 1280x720
   - workbench handles empty recon data gracefully
4. Capture screenshots into `.codex-screenshots/`.

Verification:

- Browser screenshot for empty state.
- Browser screenshot for selected target workbench, mocked or real data.

## Task 6: Final Verification and Progress

Run:

```bash
pnpm exec tsc --noEmit
pnpm exec biome check frontend/components/TargetPanel frontend/lib/i18n
pnpm exec vitest run frontend/components/TargetPanel
```

If backend was touched, also run targeted Rust checks. Only run `just precommit` when the baseline is healthy or the user asks for the full gate.

Update:

- `agent-progress.md`
- `feature_list.json`

Do not mark the feature `passing` until verification evidence is recorded.

