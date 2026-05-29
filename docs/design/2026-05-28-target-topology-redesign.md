# Target Topology Redesign

- **Date**: 2026-05-28
- **Status**: Draft / user-approved mock direction
- **Mock**: `.codex-screenshots/target-topology-redesign-mock.svg`
- **Scope**: `frontend/components/TargetPanel/TargetGraphView.tsx` and new topology components

## Decision

The old Topology view should be replaced rather than visually patched. It currently renders `Target[]` as a domain-group graph, which makes it a second target list instead of a useful attack-surface map.

The new view should model:

```text
Organization -> Target -> Service -> Evidence
```

This matches the post-ZAP Target Manager direction: organization ownership on the left, target surface in the middle, and evidence as the completion contract.

## Current Problems

1. The old graph ignores `organizations`.
   - It groups root targets by `getRootDomain(target.value)`.
   - It does not represent subsidiaries, ownership, or unassigned targets.

2. The old layout is fixed-coordinate.
   - `GAP_X`, `GAP_Y`, and `CHILD_GAP_X` work only for small examples.
   - The graph becomes hard to read once organizations, ports, evidence, and findings enter the same view.

3. The old sidebar duplicates the tree.
   - Topology should not keep another target list next to the canvas.
   - It should expose graph mode, filters, focus controls, and selection state.

4. The old inspector is too thin.
   - It shows ports/findings/notes, while the new target workflow needs surface, JS/API, sensitive signals, and evidence trail.

## Proposed Shape

The approved mock uses a three-column layout:

- Left controls:
  - graph mode: `Ownership`, `Surface`, `Evidence`
  - visible node types: orgs, targets, services, findings/evidence
  - focus actions: auto-layout, fit selected
- Center canvas:
  - layered left-to-right graph
  - nodes are compact information blocks, not decorative icons
  - edges carry semantic labels through styling and mode
- Right inspector:
  - selected node summary
  - target surface counts
  - evidence trail
  - actions that open the target workbench or start recon

## Node Model

First implementation slice:

- `organization`: root orgs and subsidiaries
- `target`: confirmed targets attached to `organization_id`; orphan targets under an `unassigned` synthetic org
- `service`: derived from `target.ports`
- `evidence`: derived from target metadata for now; later from `target_timeline` / surface summary

Later slices can add:

- `api_endpoint`
- `js_file`
- `sensitive_signal`
- `finding`
- `recon_run`

## Implementation Strategy

Keep `TargetGraphView` as the exported component so `TargetPanel` does not need a broad rewrite. Internally replace the old implementation with:

- `topology/types.ts`
- `topology/buildTopologyModel.ts`
- `topology/TopologyControls.tsx`
- `topology/TopologyCanvas.tsx`
- `topology/TopologyInspector.tsx`

The first slice should use a deterministic layered layout implemented in frontend code. `elkjs` can be added later once the visual and interaction model are approved; the model boundary should make that a layout-engine swap, not a component rewrite.

## Non-Goals

- Do not bring back ZAP or SecurityView.
- Do not add active scan/exploit actions.
- Do not change DB schema.
- Do not keep old and new topology as separate user-facing modes.
- Do not let graph completion rely on LLM prose; evidence nodes must map to persisted data.
