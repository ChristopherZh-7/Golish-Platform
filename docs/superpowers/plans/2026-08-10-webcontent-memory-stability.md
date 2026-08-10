# WebContent memory stability implementation plan

1. Add RED focused tests for projection-only ChatPanel DOM, bounded Stage transcript rendering, and
   value-stable Target surface subscriptions.
2. Add a render-mode boundary to `AIChatPanel` and pass detail focus through `AppShell` without
   unmounting its event projection hook.
3. Add a presentation-only Stage transcript window that pins the current Plan and reports omitted
   history.
4. Canonicalize Target surface target-id identity before constructing reload callbacks/listeners.
5. Run only the affected Vitest files, TypeScript no-emit, scoped Biome, and diff checks.
6. Record the WebKit `ExceededMemoryLimit` diagnosis and fresh verification in the active feature,
   progress log, and frontend module cards.
