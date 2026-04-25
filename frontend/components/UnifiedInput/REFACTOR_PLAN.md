# UnifiedInput Refactoring Plan (v2 — Terminal-Only)

> **NOTE.** The previous version of this PLAN described a 1487-line dual-mode (terminal + agent) component with image attachments and Tauri drag-drop. That code shape no longer exists in the repository — agent / image / drag-drop concerns have been extracted to the right-side AI Chat Panel, and the component is now terminal-only with two new subsystems (tool search via `/t`, tool-launch mode with parameter chips). This v2 PLAN matches the **actual current file** and supersedes the previous PLAN entirely.

## 1. Current Structure Analysis

### 1.1 Metrics (measured on current file)

| Metric | Value | Notes |
|---|---|---|
| `UnifiedInput.tsx` | **1322 lines** | (legacy PLAN said 1487) |
| `useState` calls | 18 | (legacy PLAN said 21) |
| `useRef` calls | 8 | textarea / dropZone / inputContainer / prevImeSource / state / lastTextareaHeight / toolContext / toolHistory |
| `useEffect` calls | 4 | caret settings, focus on session/mode, blur-on-running, textarea height |
| `useCallback` calls | 10 | adjustTextareaHeight, handleSubmit, handleSlashSelect, handleFileSelect, handlePathSelect, handlePathSelectFinal, handleHistorySelect, clearToolMode, handleToolSelect, handleKeyDown |
| `useMemo` calls | 2 (+ 1 in `GhostTextHint`) | filteredSlashCommands, ghostText |

### 1.2 Props signature (DO NOT change)

```tsx
interface UnifiedInputProps {
  sessionId: string;
}
```

The only caller is `frontend/components/PaneContainer/PaneLeaf.tsx` which passes only `sessionId`.

`workingDirectory` is read **inside** the component via `useStore((s) => s.sessions[sessionId]?.workingDirectory)` — it is not a prop. There is no `onOpenGitPanel` anywhere in the workspace.

### 1.3 Store subscriptions (DO NOT change selector shape)

Only field destructured from `useUnifiedInputState(sessionId)`:
- `isSessionDead`

Other store reads:
- `useStore((s) => s.sessions[sessionId]?.workingDirectory)`
- `useStore((s) => s.setLastSentCommand)`
- `usePendingCommand(sessionId)` → `isProcessRunning`
- `useStore.getState()` for ad-hoc imperative calls (`clearBlocks`, `requestTerminalClear`, `handlePromptStart`, …)

### 1.4 Main responsibilities (current 10)

1. **Text input state + auto-resize textarea** (rAF batching).
2. **Custom block caret overlay** (`BlockCaret`) + caret settings + IME source auto-swap on focus/blur.
3. **Command history navigation** (Arrow Up/Down on first/last line) with **tool-history badge restoration** (recalled commands that were originally tool launches re-enter tool mode).
4. **Ctrl+R history search popup**.
5. **Slash command popup** (`/` at start), with exact-match-plus-space close behavior.
6. **File @-mention popup** — *gated to `inputMode !== "terminal"`, so currently inert; it stays so we don't regress when this gets re-enabled.*
7. **Path completion popup** (Tab in terminal mode), with ghost-text hint of the top match.
8. **Tool search mode** (`/t <query>`): badge shown next to textarea, popup with installed-tool matches.
9. **Tool launch mode** (after a tool is picked): tool badge, parameter chips that toggle flags into the input, Backspace-on-empty-to-exit, history-recall restores the tool context.
10. **Submission**: `ptyWrite` for normal commands; `clear` does both `clearBlocks(sessionId)` + `requestTerminalClear(sessionId)` + `ptyWrite("clear\n")`; tool mode prefixes the typed args with the resolved `baseCmd` (and optional `cdPrefix`); `/t` prefixes are swallowed (never sent to PTY).

### 1.5 Things explicitly **NOT** in current code (and will stay out)

- Agent mode / `inputMode` toggle (mode is hardcoded `"terminal"`).
- `imageAttachments`, `visionCapabilities`, paste-image, Tauri drag-drop file ingestion.
- `isAgentBusy`, `sendPromptSession`, `sendPromptWithAttachments`.
- `InlineTaskPlan`, `InputStatusBadges`, rendering of `InputStatusRow`.
- `Cmd+I` / `Cmd+Shift+T` mode toggles.

### 1.6 Already-orphaned files in this directory (DO NOT delete, DO NOT re-import)

- `ImageAttachment.tsx`
- `InputStatusRow.tsx`
- `ModelSelector.tsx`
- `StatusBadges.tsx`

These are kept as-is per the agreed scope.

---

## 2. Target File Structure

```
frontend/components/UnifiedInput/
├── UnifiedInput.tsx                   # Orchestrator              (~200 lines)
├── InputContextRow.tsx                # ContextBar + tool params  (~120)
├── InputContainer.tsx                 # textarea + 5 popups + Send (~280)
├── ToolModeBadges.tsx                 # /t badge + activeTool chip (~80)
├── GhostTextHint.tsx                  # Pure presentational       (~30)
├── hooks/
│   ├── useInputSubmission.ts          # handleSubmit + tool ctx   (~100)
│   ├── useKeyboardNavigation.ts       # handleKeyDown state machine (~280)
│   ├── usePopupTriggers.ts            # onChange popup triggers   (~80)
│   ├── useToolMode.ts                 # activeTool + toolParams   (~100)
│   ├── useTextareaAutoResize.ts       # rAF height batcher        (~30)
│   └── useImeSourceSwap.ts            # focus/blur IME swap       (~30)
├── utils/
│   └── inputHelpers.ts                # extractWordAtCursor, isCursorOnFirstLine, isCursorOnLastLine, clearTerminal (~50)
├── index.ts                           # barrel: re-export UnifiedInput
└── REFACTOR_PLAN.md                   # this file
```

> Untouched (kept as-is): `BlockCaret.tsx`, `ContextBar.tsx`, `ModelSelector.tsx`, `StatusBadges.tsx`, `ImageAttachment.tsx`, `InputStatusRow.tsx` and all `*.test.tsx`.

---

## 3. Per-File Specs

### 3.1 `UnifiedInput.tsx` (orchestrator) — target ~200 lines

**Owns:**
- `input` state (single source of truth, threaded into hooks/components).
- `textareaRef`, `inputContainerRef`, `dropZoneRef` (ref creation only).
- `stateRef` mutation in render (the perf optimization the user explicitly wants preserved).
- Composition / layout JSX.

**Reads from store:**
- `useUnifiedInputState(sessionId)` → `{ isSessionDead }` (UNCHANGED).
- `useStore((s) => s.sessions[sessionId]?.workingDirectory)` (UNCHANGED).
- `usePendingCommand(sessionId)` (UNCHANGED).

**Calls into hooks:**
```ts
const submission   = useInputSubmission({ sessionId });
const toolMode     = useToolMode({ input });
const popupState   = useUnifiedPopupState();           // collects 18 popup-related useState (kept here, not in a hook, see §6)
const popupTriggers= usePopupTriggers({ input, inputMode: "terminal", commands, activeTool: toolMode.activeTool });
const keyDown      = useKeyboardNavigation({ ...stateRef, ...handlers });
useTextareaAutoResize(textareaRef, input);
useImeSourceSwap(textareaRef, prevImeSourceRef);
```

**Layout:**
```tsx
return (
  <div className="border-t border-[var(--border-subtle)]">
    <InputContextRow
      sessionId={sessionId}
      activeTool={toolMode.activeTool}
      toolParams={toolMode.toolParams}
      input={input}
      setInput={setInput}
      textareaRef={textareaRef}
    />
    <InputContainer
      sessionId={sessionId}
      input={input}
      setInput={setInput}
      isSessionDead={isSessionDead}
      isProcessRunning={isProcessRunning}
      textareaRef={textareaRef}
      inputContainerRef={inputContainerRef}
      dropZoneRef={dropZoneRef}
      submission={submission}
      toolMode={toolMode}
      popupState={popupState}
      popupTriggers={popupTriggers}
      onKeyDown={keyDown.handleKeyDown}
    />
  </div>
);
```

### 3.2 `InputContextRow.tsx` — target ~120 lines

Renders the existing `<ContextBar sessionId={sessionId} />` followed (conditionally) by the **tool params chip panel**.

```tsx
interface InputContextRowProps {
  sessionId: string;
  activeTool: ToolConfig | null;
  toolParams: ToolParam[];
  input: string;
  setInput: (next: string) => void;
  textareaRef: React.RefObject<HTMLTextAreaElement>;
}
```

The existing chip click logic (the ~50-line `onClick` that toggles `p.flag` into `input`) is moved here verbatim.

### 3.3 `InputContainer.tsx` — target ~280 lines

Owns the inner `<textarea>` + the 5 popups + the Send button + `BlockCaret` overlay + `GhostTextHint`. Receives all popup states & handlers via props (no internal popup useState).

```tsx
interface InputContainerProps {
  sessionId: string;
  input: string;
  setInput: (next: string) => void;
  isSessionDead: boolean;
  isProcessRunning: boolean;
  textareaRef: React.RefObject<HTMLTextAreaElement>;
  inputContainerRef: React.RefObject<HTMLDivElement>;
  dropZoneRef: React.RefObject<HTMLDivElement>;
  submission: ReturnType<typeof useInputSubmission>;
  toolMode: ReturnType<typeof useToolMode>;
  popupState: PopupBundle;
  popupTriggers: ReturnType<typeof usePopupTriggers>;
  onKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void;
}
```

It also keeps the `isFocused` + `caretSettings` local state (they're scoped purely to the textarea overlay).

### 3.4 `ToolModeBadges.tsx` — target ~80 lines

Two small sibling chips that may render before the textarea:

1. `/t` tool-search badge (orange, with `×` to clear input).
2. Active-tool badge (accent, with runtime initial avatar + `×` to exit).

```tsx
interface ToolModeBadgesProps {
  isToolSearchMode: boolean;
  activeTool: ToolConfig | null;
  onClearSearch: () => void;
  onClearActiveTool: () => void;
}
```

This is composed inside `InputContainer.tsx`.

### 3.5 `GhostTextHint.tsx` — target ~30 lines

The existing inline `memo` component, moved verbatim into its own file. Only Imports `memo`, `useMemo` from `react`.

### 3.6 `hooks/useInputSubmission.ts` — target ~100 lines

```ts
interface UseInputSubmissionOptions {
  sessionId: string;
  inputRef: React.MutableRefObject<{ input: string; activeTool: ToolConfig | null }>;
  resetHistory: () => void;
  addToHistory: (entry: string) => void;
  toolContextRef: React.MutableRefObject<{ cdPrefix: string; baseCmd: string }>;
  toolHistoryRef: React.MutableRefObject<Map<string, ToolHistoryEntry>>;
  clearToolMode: () => void;
  setInput: (next: string) => void;
}

interface UseInputSubmissionReturn {
  handleSubmit: () => void;
}
```

Encapsulates: `/t` swallow, `clear` special-case, tool-mode prefix construction, `setLastSentCommand`, `ptyWrite`. Reads `input` and `activeTool` from `stateRef` to keep callback stable (no deps on input).

### 3.7 `hooks/useKeyboardNavigation.ts` — target ~280 lines

The big `handleKeyDown` state machine. Takes the entire `stateRef` plus the suite of setters and select-handlers, returns a stable `handleKeyDown`.

Branches preserved in this exact order (matches current code):
1. Escape during `isProcessRunning` → `handlePromptStart`.
2. `/t` search-mode escape/backspace.
3. Active-tool escape / backspace-on-empty.
4. Tool popup nav (↑↓/Enter/Tab).
5. History search nav (Esc/Ctrl+G/Enter/Ctrl+R/↑↓/Backspace/printable).
6. Open history search (Ctrl+R).
7. Path popup nav (Esc/↑↓ + Ctrl+N/J/P/K, Tab, Enter).
8. Slash popup nav (Esc/↑↓/Tab/Enter).
9. File popup nav (Esc/↑↓/Tab/Enter).
10. Slash-with-args Enter handoff.
11. Plain Enter → submit.
12. Arrow Up history (with tool-history badge restoration).
13. Arrow Down history (same).
14. Terminal-only: Tab path completion, Ctrl+C, Ctrl+D, Ctrl+L.

### 3.8 `hooks/usePopupTriggers.ts` — target ~80 lines

Pure-derived: given `input` and `inputMode` and the slash `commands` list, returns:

```ts
interface UsePopupTriggersReturn {
  isToolSearchMode: boolean;       // /^\/t\s/i.test(input)
  toolSearchQuery: string;          // input.replace(/^\/t\s+/i, '')
  toolSearchEnabled: boolean;       // isToolSearchMode && query.length > 0 && !activeTool
  slashCommandName: string;
  filteredSlashCommands: SlashCommand[];
  fileQuery: string;                // last "@..." token
}
```

The actual popup-open/close `setShowXxx` calls stay in `InputContainer.onChange` — this hook only computes derived flags.

### 3.9 `hooks/useToolMode.ts` — target ~100 lines

Owns `activeTool`, `toolParams`, `toolContextRef`, `toolHistoryRef`. Exposes `handleToolSelect(tool)`, `clearToolMode()`, plus a `restoreFromHistory(cmd)` helper that the keyboard nav can call when `navigateUp/Down` returns a command.

### 3.10 `hooks/useTextareaAutoResize.ts` — target ~30 lines

Wraps the existing rAF height-batching logic. Returns nothing; just runs an effect on `[input]`.

### 3.11 `hooks/useImeSourceSwap.ts` — target ~30 lines

Returns `{ onFocus, onBlur }` to be wired onto the textarea. Internally uses a ref to remember the prior IME source.

### 3.12 `utils/inputHelpers.ts` — target ~50 lines

Pure functions, no React:
- `extractWordAtCursor(input, cursorPos): { word, startIndex }`
- `isCursorOnFirstLine(text, cursorPos): boolean`
- `isCursorOnLastLine(text, cursorPos): boolean`
- `clearTerminal(sessionId): void` (currently stays as a closure over `useStore.getState()`; moving it here keeps a stable import surface)

---

## 4. Migration Strategy (Phase 1 → 5)

Each Phase ends with the **same 3 commands**:
```bash
pnpm tsc --noEmit
pnpm biome check frontend/components/UnifiedInput
pnpm vitest run frontend/components/UnifiedInput
```

**Pass criteria** for each Phase:
1. `tsc --noEmit` exits 0.
2. `biome check` exits 0 on the directory.
3. `vitest` results vs. baseline:
   - Baseline (this PLAN's Step 0): `Tests 49 failed | 18 passed (67)` across 6 files.
   - **Each Phase must have `failed ≤ 49` and `passed ≥ 18`.** Newly red previously-green tests block the Phase.

### Phase 1 — Pure functions

1. Create `utils/inputHelpers.ts` with `extractWordAtCursor`, `isCursorOnFirstLine`, `isCursorOnLastLine`, `clearTerminal`.
2. Replace local definitions in `UnifiedInput.tsx` with imports.
3. Run the 3 checks.

### Phase 2 — Hooks (in dependency order)

1. **2a** — `useTextareaAutoResize` and `useImeSourceSwap` (no deps on each other).
2. **2b** — `usePopupTriggers` (pure derivations, no useState).
3. **2c** — `useToolMode` (owns `activeTool`, `toolParams`, two refs).
4. **2d** — `useInputSubmission` (depends on `useToolMode` for context refs + clearer).
5. **2e** — `useKeyboardNavigation` (depends on all of the above + popup state).

After each substep, all 3 checks must pass.

### Phase 3 — UI components

1. **3a** — `GhostTextHint.tsx` (verbatim move).
2. **3b** — `ToolModeBadges.tsx` (extract the two badges).
3. **3c** — `InputContextRow.tsx` (extract `ContextBar` wrapper + tool-params chip panel).
4. **3d** — `InputContainer.tsx` (extract textarea + popups + Send + BlockCaret).

### Phase 4 — Final orchestrator + barrel

1. Trim `UnifiedInput.tsx` to ≤ 250 lines, only doing composition + state ownership the children genuinely need from the parent.
2. Confirm `index.ts` still exports only `UnifiedInput`.

### Phase 5 — Final report

Deliverable to user:
- Original / new line counts.
- For every new file: relative path + final line count.
- Per-Phase test results (failed/passed) vs. baseline.
- Any deviation from this PLAN with rationale.

---

## 5. Hard Constraints (recap)

1. Exported component name `UnifiedInput` and props `{ sessionId: string }` MUST NOT change.
2. The selector shape of `useUnifiedInputState(sessionId)` MUST NOT change.
3. NO business-logic changes — extraction / move / rewire only.
4. `stateRef` per-property mutation pattern MUST be preserved (currently in `UnifiedInput.tsx` lines 245–289).
5. DO NOT touch: `BlockCaret.tsx`, `ContextBar.tsx`, `ModelSelector.tsx`, `StatusBadges.tsx`, `ImageAttachment.tsx`, `InputStatusRow.tsx`, and all `*.test.tsx`.
6. DO NOT re-introduce agent / image / drag-drop concerns.
7. TypeScript strict mode, NO `any`.
8. Testing baseline: do not add NEW failing tests; do not "fix" the 23 currently-red tests in the 3 named files.

---

## 6. State Ownership Decision Log

A few non-trivial calls explained:

- **18 useState stays in `UnifiedInput.tsx`?** No. Most popup-related useState (`showSlashPopup`, `slashSelectedIndex`, `showFilePopup`, `fileSelectedIndex`, `showPathPopup`, `pathSelectedIndex`, `pathQuery`, `showHistorySearch`, `historySearchQuery`, `historySelectedIndex`, `originalInput`, `showToolPopup`, `toolSelectedIndex`) move into `InputContainer.tsx` so the parent doesn't re-render on every popup nav. `input`, `setInput`, the 4 refs and `stateRef` stay in the parent because both `InputContextRow` (params chips read `input`, write `setInput`) and `InputContainer` need them.
- **`isFocused` + `caretSettings`** — confined to `InputContainer.tsx` (only the textarea + BlockCaret consume them).
- **`activeTool` + `toolParams`** — owned by `useToolMode` hook, consumed by both `InputContextRow` (chip panel) and `InputContainer` (active-tool badge + placeholder copy). Hook is instantiated once in the orchestrator and the return value is threaded down.
- **`stateRef` mutations** must remain in the orchestrator's render body to preserve the synchronous-availability semantics described in the original code's comment block.

---

## 7. Performance Properties to Preserve

1. `stateRef` per-property updates (no new object allocation per render).
2. `requestAnimationFrame` batching of textarea resize.
3. `lastTextareaHeightRef` cache to skip no-op writes.
4. `GhostTextHint` `memo` + `useMemo`'d style.
5. Single `useUnifiedInputState` selector subscription.
6. `usePathCompletion` `enabled` flag gating (only fetches when popup is open).

No new optimization is introduced in this PLAN — the goal is decomposition, not perf changes.

---

## 8. Success Metrics

1. `UnifiedInput.tsx` ≤ 250 lines (down from 1322).
2. No file in this directory > 300 lines after the refactor (excluding the already-existing untouched files).
3. `pnpm tsc --noEmit` clean.
4. `pnpm biome check frontend/components/UnifiedInput` clean.
5. `pnpm vitest run frontend/components/UnifiedInput`: `failed ≤ 49 && passed ≥ 18` (baseline preserved).
6. Visual + functional smoke (manual): typing, /t search, tool-launch chip toggling, history Up/Down with tool restoration, path Tab completion, slash Enter, BlockCaret rendering, IME swap on focus.
