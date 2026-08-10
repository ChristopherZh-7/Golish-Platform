# WebContent memory stability for long Stage runs

## Problem

Long-running Company-stage sessions can keep a large conversation, Stage Agent transcript, and
Target surface mounted at the same time. In detail-focus mode the ChatPanel was hidden with the
HTML `hidden` attribute but its complete message/tool DOM remained mounted and continued to update.
The selected Agent transcript rendered every historical entry, and Target surface listeners used
array identity rather than the canonical target-id set. A real EAS run grew the macOS WebContent
process to about 4 GB; WebKit terminated it with `ExceededMemoryLimit`, leaving a white Tauri shell
while the backend and database continued normally.

## Decision

- Keep `AIChatPanel` mounted in a lightweight projection mode so its existing event-to-conversation
  authority continues to run, but return no ChatPanel DOM while detail focus owns the workspace.
  Returning to the timeline remounts the same UI from store truth.
- Render a bounded tail of the selected Agent transcript. Preserve the latest visible Plan entry
  even if it is older than the tail, and show an explicit omission notice. The complete transcript
  remains in the store and durable transcript/run log; the bound is presentation-only.
- Derive Target surface subscription identity from the canonical ordered target-id value set, not
  the caller's array object identity. Equivalent refreshes reuse listeners and loaded state.
- Do not weaken evidence, background-job, or Stage authority. This change performs no database or
  network mutation and does not change Naabu lifetime semantics.

## Verification

- focused AIChatPanel test proves projection-only mode mounts no message/report/input DOM;
- focused Stage workspace test proves large transcripts render a bounded window plus omission
  notice while retaining the current Plan;
- focused Target surface hook test proves an equivalent target-id rerender does not reload or
  resubscribe;
- focused Vitest, TypeScript no-emit, scoped Biome, and diff checks.
