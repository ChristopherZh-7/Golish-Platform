# Target Topology Redesign Implementation Plan

- **Date**: 2026-05-28
- **Feature**: `target-surface-workbench` / topology redesign slice
- **Design**: `docs/design/2026-05-28-target-topology-redesign.md`
- **Mock**: `.codex-screenshots/target-topology-redesign-mock.svg`

## Goal

Replace the old target-only Topology view with an Attack Surface Map that shows organization ownership, target surface, services, and evidence.

## Constraints

- Do not run `init`; user explicitly asked not to.
- Do not add network-installed dependencies in this slice.
- Keep `TargetGraphView` exported so `TargetPanel` wiring remains stable.
- Avoid naked `invoke()` from UI components.
- Do not modify backend or DB schema.
- Keep the graph implementation reusable so `elkjs` can replace the first local layout later.

## Task 1: Topology Model

Files:

- `frontend/components/TargetPanel/topology/types.ts`
- `frontend/components/TargetPanel/topology/buildTopologyModel.ts`

Steps:

1. Define node and edge types.
2. Build organization nodes from `Organization[]`.
3. Attach targets by `organization_id`.
4. Put orphan targets under a synthetic unassigned org.
5. Create service nodes from target `ports`.
6. Create evidence summary nodes from target metadata and service presence.
7. Add simple filtered graph mode support.

Verification:

- TypeScript compiles.
- Existing TargetPanel tests still pass.

## Task 2: Controls + Canvas + Inspector

Files:

- `TopologyControls.tsx`
- `TopologyCanvas.tsx`
- `TopologyInspector.tsx`

Steps:

1. Build the left controls from the mock.
2. Render a layered SVG canvas with semantic node blocks.
3. Support selecting nodes and focusing them from controls.
4. Build inspector states for org / target / service / evidence.
5. Include empty, loading, and error states.

Verification:

- TypeScript compiles.
- Biome passes for TargetPanel.

## Task 3: Replace Old TargetGraphView

Files:

- `TargetGraphView.tsx`
- delete old unused files if no references remain:
  - `GraphElements.tsx`
  - `hooks/useGraphLayout.ts`

Steps:

1. Fetch organizations through the existing API wrapper.
2. Build the topology model with organizations and targets.
3. Render controls, canvas, and inspector.
4. Remove old Cytoscape-specific imports and naked `invoke()`.
5. Delete the old graph helper files once references are gone.

Verification:

- `rg` shows no references to old helpers.
- `pnpm exec tsc --noEmit`
- `pnpm exec biome check frontend/components/TargetPanel frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json frontend/lib/security-analysis.ts`
- `pnpm exec vitest run frontend/components/TargetPanel`

## Task 4: Progress

Update:

- `agent-progress.md`
- `feature_list.json`

Do not mark the feature passing without full gates and visual QA.
