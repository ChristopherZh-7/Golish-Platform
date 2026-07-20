# ChatPanel Stage Reset Consistency Implementation Plan

> Follow systematic debugging, TDD, and verification-before-completion. Run `just space-guard` before every Cargo build/test. Do not run init/precommit/full-workspace gates without explicit user authorization.

**Goal:** Make ChatPanel stage reset atomic and visibly consistent for the four in-place-safe Company stages, while failing closed for immutable stage families.

**Architecture:** Keep stage semantics in `golish-agent-app`, pass a data-only purge plan into the `golish-db` compound reset transaction, derive scope from the sealed operation snapshot, and drive frontend roadmap rewind from the committed reset receipt.

**Tech stack:** Rust/sqlx/Tauri, React/TypeScript/Zustand/Vitest.

---

### Task 1: Lock the safe stage policy with RED tests

**Files:**
- Modify: `backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs`
- Modify: `frontend/components/AIChatPanel/StageResetMenu.test.ts`

1. Add pure backend tests proving Company stages are supported and Scoping/Candidate/Verification/Reporting are rejected for full in-place reset.
2. Add a reached-stage/current-family preflight test seam so an unvisited DAG branch, a historical forward jump, an unknown stage name, and an immutable downstream history are rejected.
3. Replace the frontend linear-frontier expectations with tests proving only `current || passed` Company stages are selectable, unknown/null current disables reset, and an unvisited Enumeration stage remains locked after a direct EAS→Reporting route.
4. Run the two focused test targets and record the expected RED failures.

### Task 2: Lock transaction, scope, and state-namespace behavior with RED tests

**Files:**
- Modify: `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- Modify: `backend/crates/golish-db/src/repo/stage_purge.rs`

1. Add `StageCheckpointPurgePlan` fixture coverage to the existing migrated V2 runtime reset test.
2. Prove a purge failure after relational mutation leaves the original cursor/execution/facts unchanged.
3. Prove a V2 operation with `engagement_org_id=NULL` uses sealed snapshot units and returns non-zero purge scope/counts.
4. Prove affected Target Intel/EAS/Enumeration state namespaces are removed while unrelated sibling state survives.
5. Prove an overlapping active operation sharing any frozen organization blocks before mutation, while terminal siblings do not.
6. Prove current operation/session run aliases are deleted without deleting historical run-owned rows, and ownership-ambiguous Finding/Vuln history is retained.
7. Prove exact Worker lease mismatch fails closed, and an exact `received|running` tool blocks reset until its explicit finish/stop path clears the pointer.
8. Run the exact focused nextest filters and record RED.

### Task 3: Implement the compound backend reset

**Files:**
- Modify: `backend/crates/golish-db/src/repo/stage_purge.rs`
- Modify: `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs`

1. Introduce a data-only purge domain enum/plan and one transaction-owned purge executor in `golish-db`.
2. Extend `SupersedeStageCheckpointRow` with an optional purge plan and extend returned stats with scope/count receipt data.
3. Load and lock the exact sealed frozen scope before mutation; compare a present legacy root but accept a null legacy root, and reject overlap with another active sealed-scope operation.
4. Run fact/ledger/status purge inside the same transaction as runtime supersede, graph/state cleanup, and cursor update.
5. Scope run-owned tables to current operation + Task session aliases; retain `findings`, `vuln_scan_history`, and other rows without trustworthy current-operation ownership.
6. Reject a reset with an exact active external tool before mutation; do not confuse terminalizing a DB row with cancelling the underlying writer. Preserve exact lease/epoch identity validation for stale pointers.
7. Clear affected stage-owned state namespaces in the V2 state blob.
8. Add full-reset preflight in the command, remove the second purge transaction, and build the purge plan from embedded specs.
9. Run focused GREEN tests.

### Task 4: Make the frontend receipt-driven and fail closed

**Files:**
- Modify: `backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs`
- Generate: `frontend/lib/generated/HarnessDevStageCheckpointResetResult.ts`
- Modify: `frontend/lib/api/harness-dev.ts`
- Create: `frontend/lib/stage-reset.ts`
- Modify: `frontend/store/slices/workflow/plan.ts`
- Modify: `frontend/store/slices/workflow/types.ts`
- Modify: `frontend/store/workflow.test.ts`
- Modify: `frontend/components/AIChatPanel/stagePlanPersistence.ts`
- Modify: `frontend/components/AIChatPanel/StageResetMenu.tsx`
- Modify: `frontend/components/AIChatPanel/StageResetMenu.test.ts`
- Modify: `frontend/components/AIChatPanel/AIChatPanel.tsx`
- Modify: `frontend/components/AIChatPanel/ConversationTabs.tsx`
- Create: `frontend/components/AIChatPanel/ConversationTabs.test.tsx`
- Modify: `frontend/components/AIChatPanel/hooks/useChatSend.ts`
- Modify: `frontend/components/AIChatPanel/ExecutionModePicker.tsx`
- Modify: `frontend/components/AIChatPanel/ChatModelSelector.tsx`

1. Export the existing reset result through ts-rs and return it from the API wrapper.
2. Add one pure reset-contract helper for receipt validation/sanitization, current-stage inference, and deterministic selected-stage v0 seed creation.
3. Add a Zustand `rewindStagePlans(sessionId, affectedStages, selectedStage)` action and matching persistence helper; RED/GREEN tests must prove affected plans are removed, selected v0 `in_progress` exists immediately, no missing session alias is created, and durable-only descendants are included from the snapshot's own stage order.
4. Replace index-based menu selection with the supported/current/passed policy and render concise disabled reasons.
5. Treat backend return as the commit boundary, reconcile one existing canonical roadmap owner plus conversation persistence, then validate the full receipt before auto-resume; malformed post-commit receipts must not be reported as an uncommitted reset.
6. Give reset, ordinary send, textarea, mode/model selectors, attachments, and conversation select/new/close/history one mutual-exclusion gate; only the reset-owned visible auto-resume may bypass it, and a failed send leaves `继续跑` available for retry.
7. Add focused helper/component/send/store tests, then run focused Vitest, Biome, and typecheck.

### Task 5: Documentation and focused verification

**Files:**
- Modify: `docs/modules/backend/golish-agent-app/ai.md`
- Modify: `docs/modules/backend/golish-agent-app.md`
- Modify: `docs/modules/backend/golish-db.md`
- Modify: `docs/modules/backend/golish-db/repo.md`
- Modify: `docs/modules/frontend/components.md`
- Modify: `docs/modules/frontend/lib.md`
- Modify: `docs/modules/frontend/store.md`
- Modify: `docs/modules/INDEX.md`
- Modify: superseded/partially-superseded stage-reset design notes
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

1. Update module cards with supported stage matrix, frozen-scope authority, transaction boundary, and receipt-driven UI behavior.
2. Run scoped Rust nextest/clippy/rustfmt and focused frontend Vitest/Biome/typecheck.
3. Run JSON validation and scoped `git diff --check`.
4. Record commands, exit codes, key output, remaining unsupported-stage routing, and uncommitted file list.
5. Mark the feature passing only if every focused verification item has fresh evidence; otherwise leave in progress/blocked.
