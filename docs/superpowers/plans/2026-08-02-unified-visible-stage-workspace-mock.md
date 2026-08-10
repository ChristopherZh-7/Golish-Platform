# Unified Visible Stage Workspace Mock Implementation Plan

> **For AI workers:** Follow `AGENTS.md`. Preserve the shared dirty worktree. This plan creates a dev-only React mock and must not modify production Stage Team routing, backend, schema/migrations, generated IPC, real operations, providers, or targets.

**Goal:** Deliver an interactive, repository-native frontend mock showing one unified Stage Workspace for Recon, Enumeration, Application Understanding, Vulnerability, and Verification.

**Architecture:** Build a pure fixture-driven `StageWorkspaceMock` under `frontend/components/Engagement/StageWorkspace/`, mount it at the top of the existing `ComponentTestbed`, and keep all selection state local. Reuse product tokens and UI primitives, but do not reuse the existing Controller summary cards the user rejected. The later production adapter is explicitly out of scope.

**Stack:** React 19, TypeScript 6, Tailwind 4, lucide-react, Vitest, Testing Library, Biome.

---

## Task 1: Register the mock feature and design boundary

**Files:**

- Create: `docs/design/2026-08-02-unified-visible-stage-workspace-mock.md`
- Create: `docs/superpowers/plans/2026-08-02-unified-visible-stage-workspace-mock.md`
- Modify: `docs/design/INDEX.md`
- Modify: `docs/superpowers/plans/INDEX.md`
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

**Steps:**

1. Register exactly one `in_progress` feature.
2. Record that this is an actual React dev mock, not a production runtime cutover.
3. Lock no-backend/no-DB/no-external-request boundaries.
4. Validate JSON, one-active-feature, document links, and scoped diff whitespace.

## Task 2: Write focused interaction tests first

**Files:**

- Create: `frontend/components/Engagement/StageWorkspace/StageWorkspaceMock.test.tsx`

**RED assertions:**

1. Default view renders Enumeration in the shared Workspace shell.
2. Switching to another stage reuses the same shell and changes metrics/agents.
3. Selecting a visible LLM Agent changes the conversation.
4. Selecting a deterministic task shows the explicit no-LLM label.
5. Selecting an endpoint/artifact updates the evidence inspector.
6. The rejected `COMPANY CONTROLLERS` summary-card heading is absent.

```bash
pnpm exec vitest run frontend/components/Engagement/StageWorkspace/StageWorkspaceMock.test.tsx
```

## Task 3: Implement the pure Workspace mock

**Files:**

- Create: `frontend/components/Engagement/StageWorkspace/types.ts`
- Create: `frontend/components/Engagement/StageWorkspace/mock-fixtures.ts`
- Create: `frontend/components/Engagement/StageWorkspace/StageWorkspaceMock.tsx`
- Create: `frontend/components/Engagement/StageWorkspace/index.ts`

**Steps:**

1. Define stage, metric, agent, conversation-entry, parameter, and artifact fixture types.
2. Add realistic fixtures for five stages.
3. Build one responsive shell with stage switcher, coverage strip, agent rail, conversation timeline, and artifact/evidence inspector.
4. Keep Controller as the first agent node, not the page model.
5. Label deterministic tasks honestly and never create fake LLM dialogue.
6. Make stage, agent, and artifact controls keyboard-accessible native buttons.

## Task 4: Mount the entity mock in Component Testbed

**Files:**

- Modify: `frontend/pages/ComponentTestbed.tsx`

**Steps:**

1. Render the unified Workspace before the generic component catalog.
2. Give the mock enough width to exercise the desktop two-column design.
3. Keep the existing component catalog intact below it.
4. Document the review path: `Cmd/Ctrl+K → Component Testbed`.

## Task 5: Update module cards and run focused verification

**Files:**

- Modify: `docs/modules/frontend/components.md`
- Modify: `docs/modules/frontend/pages.md`
- Modify: `docs/modules/INDEX.md`
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

**Verification:**

```bash
pnpm exec vitest run frontend/components/Engagement/StageWorkspace/StageWorkspaceMock.test.tsx
pnpm exec biome check frontend/components/Engagement/StageWorkspace frontend/pages/ComponentTestbed.tsx
pnpm typecheck
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json
git diff --check -- frontend/components/Engagement/StageWorkspace frontend/pages/ComponentTestbed.tsx docs/design/2026-08-02-unified-visible-stage-workspace-mock.md docs/superpowers/plans/2026-08-02-unified-visible-stage-workspace-mock.md docs/modules/frontend/components.md docs/modules/frontend/pages.md docs/modules/INDEX.md docs/design/INDEX.md docs/superpowers/plans/INDEX.md feature_list.json agent-progress.md
```

Only mark `passing` after fresh focused evidence is recorded. Do not run `just check-fe`, `just test-fe`, `just precommit`, `init.sh`, or any full-workspace suite without explicit user authorization.
