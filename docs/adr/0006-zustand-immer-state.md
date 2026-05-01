# ADR-0006: Zustand + Immer for Frontend State Management

## Status

Accepted

## Context

Golish's React frontend manages complex, deeply nested state:

- **AI chat sessions** — message arrays with streaming tokens, tool calls,
  and artifacts.
- **Scan state** — multiple concurrent scans, each with progress, findings,
  and log streams.
- **Settings** — user preferences, model configs, MCP server registrations.
- **UI layout** — panel sizes, active tabs, terminal sessions.

Requirements:

- **Minimal boilerplate** — the team is small; Redux-style ceremony slows
  iteration.
- **Immutable updates** — React 19 concurrent features rely on referential
  equality for bailout optimizations.
- **Fine-grained subscriptions** — components should re-render only when
  their slice changes, not on every store update.
- **TypeScript-first** — full type inference without manual action types.

## Decision

Use **`zustand ^5`** for state management with **`immer ^11`** middleware
for immutable updates.

Store structure:

```
frontend/lib/stores/
├── ai-store.ts          # chat sessions, streaming state
├── pentest-store.ts     # scan lifecycle, findings
├── settings-store.ts    # user preferences
├── ui-store.ts          # layout, panels, theme
└── ...
```

Each store is a standalone Zustand store (not a single global store).
Immer's `produce()` is applied via Zustand's `immer` middleware, enabling
mutable-style syntax that produces immutable updates.

## Consequences

### Positive

- ~5 lines to define a store with full TypeScript inference; no action
  creators, reducers, or switch statements.
- `useStore(selector)` provides automatic fine-grained subscriptions;
  components only re-render when selected state changes.
- Immer middleware allows `state.scans[id].progress = 50` syntax while
  preserving immutability — reduces bugs in deeply nested updates.
- No provider wrapper needed; stores are importable functions, simplifying
  testing and SSR (not currently needed but future-proof).

### Negative

- Multiple independent stores can lead to cross-store coordination
  challenges (e.g., "scan completed" must update both pentest-store and
  ai-store); addressed via event listeners or explicit cross-store calls.
- Immer adds ~12 KB gzipped to the bundle.
- Devtools integration is less mature than Redux DevTools (though
  `zustand/middleware/devtools` exists).

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **Redux Toolkit** | Too much boilerplate for a small team; slice/action/thunk ceremony. |
| **Jotai** | Atom-based model is elegant but harder to reason about for complex derived state (scan progress → UI badge). |
| **React Context + useReducer** | No built-in selector optimization; causes unnecessary re-renders in large component trees. |
| **MobX** | Proxy-based reactivity conflicts with React 19 concurrent mode assumptions; decorator syntax adds complexity. |
| **Valtio** | Proxy-based like MobX; same concurrent mode concerns. Zustand is from the same author but uses a subscription model instead. |
