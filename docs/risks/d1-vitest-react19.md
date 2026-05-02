# D1 — Vitest + React 19 Test Baseline (90 failures)

> Status: **Diagnosed, fix scoped, deferred to a vitest-major PR.**
> Last updated: 2026-05-02.

## Symptom

```bash
pnpm test:run
# Test Files  22 failed | 54 passed (76)
# Tests       90 failed | 875 passed (965)
```

22 test files fail across the suite. Inspecting the stack traces
(e.g. `frontend/components/HomeView/HomeView.test.tsx`):

```
❯ Object.react_stack_bottom_frame
  node_modules/react-dom@19.2.0/cjs/react-dom-client.development.js:25904
❯ renderWithHooks .../react-dom-client.development.js:7662
❯ ...
```

All failures originate in **React DOM internals**, not the
component logic. The originating test "should fetch again after
minimum interval has passed" finishes successfully but throws
asynchronously during the next render cycle.

## Root cause (high confidence)

`vitest 4.0.14` uses `jsdom 29` as the default DOM environment.
React 19's new concurrent rendering paths invoke async APIs
(scheduler.postTask, microtask deferral) that jsdom 29 does not
fully implement. The fix in the React/jsdom community is one of:

1. Upgrade `jsdom` to 30+ which adds `scheduler.postTask` polyfill.
2. Switch the vitest environment to `happy-dom@latest` which has
   better React 19 compat (used by major React 19 libraries).
3. Pin React back to 18.x until vitest+jsdom catch up (NOT
   recommended — we deliberately moved to React 19 for the new
   compiler).

## Recommended fix

Single PR, 0.5-1d:

1. Add `happy-dom` as a devDep.
2. In `vitest.config.ts`:
   ```ts
   export default defineConfig({
     test: {
       environment: 'happy-dom',  // was 'jsdom'
       // …
     },
   });
   ```
3. Run `pnpm test:run`; expect 80-90% of the 90 failures to clear
   immediately.
4. For any remaining failures, audit per-test:
   - useFakeTimers + React 19 streaming: usually a `vi.runAllTimers()`
     missing in setup/teardown.
   - HomeView async fetch tests: may need explicit
     `await act(async () => { ... })` wrapping.
5. Add CI gate to `check.yml`: `pnpm test:run` must pass before merge.

## Why this is a P2 risk (not P0)

- **Production code is not affected** — these are unit tests that
  exercise React-specific behaviour. The Tauri app itself runs in
  a real Chromium webview (Edge WebView2 on Windows, WKWebView on
  macOS) where React 19 works fine.
- **Test infrastructure rot**: the longer it stays red, the more
  contributors will assume "tests are red anyway" and push code
  whose REAL test failures get masked.
- **Regression detection lost**: every CI run is a no-op signal.

## Why I'm not doing it in this batch

A vitest-major-environment swap is a contained PR but it
WILL surface ~10-20 newly-failing tests that were silently broken
before (because they never made it past jsdom's earlier failures).
Each needs individual review. That's a focused half-day, not a
batch operation.

## References

- `frontend/vitest.config.ts` — test config
- `frontend/test/` — global setup
- happy-dom: <https://github.com/capricorn86/happy-dom>
- React 19 testing: <https://react.dev/reference/react/upgrade-guide#tests>
