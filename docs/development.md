# Development

## Commands

```bash
just dev              # Full app with hot reload
just dev-fe           # Frontend only (Vite on port 1420)
just check            # All checks (biome + clippy + fmt)
just test             # All tests (frontend + Rust)
just test-e2e         # E2E tests (Playwright)
just build            # Production build
just eval             # Run evaluation scenarios
```

Run `just --list` for all available commands.

## Frontend-only development

```bash
just dev-fe
```

This starts Vite with a mock Tauri environment (useful for rapid UI iteration without LLM costs).

## Faster local Rust builds (sccache)

The 48-crate Rust workspace takes ~1m 20s for a clean
`cargo check -p golish` on an M-series Mac. CI already wires
[`sccache`](https://github.com/mozilla/sccache) via
[mozilla-actions/sccache-action](https://github.com/mozilla-actions/sccache-action)
and `Swatinem/rust-cache@v2` (see `.github/workflows/check.yml`).

To get the same speedup locally:

```bash
# macOS
brew install sccache

# Linux
cargo install sccache --locked

# Either platform: enable for all cargo invocations in this shell
export RUSTC_WRAPPER=sccache

# Optional: persist across shells
echo 'export RUSTC_WRAPPER=sccache' >> ~/.zshrc   # or ~/.bashrc
```

Verify it's working:

```bash
sccache --show-stats     # before
cargo check -p golish    # do a build
sccache --show-stats     # after — `Cache hits` should grow
```

Typical local incremental check after warm-up: 5–15s instead of
80s+. The cache lives at `~/Library/Caches/Mozilla.sccache`
(macOS) or `~/.cache/sccache` (Linux).

## Adding a new tool

Agent tools are split across the post-`golish-ai` crates:

1. Define or update schemas in `backend/crates/golish-agent-kit/src/tool_definitions/`.
2. Implement executors in `backend/crates/golish-agent-kit/src/tool_executors/` or the domain crate that owns the capability.
3. Wire dispatch through `backend/crates/golish-agent-kit/src/tool_execution/` and, when needed, the runtime loop in `backend/crates/golish-agent-runtime/src/agentic_loop/`.
4. Add prompt contribution changes in `backend/crates/golish-prompts/src/` if the agent must learn when to use the tool.
5. Add frontend event handling only when the tool emits new UI-visible events.

## Adding a new Tauri command (IPC)

Golish groups Tauri commands per domain via `commands_facade/<domain>.rs`
files. **Never add a command directly to `commands_registry.rs` without
touching the facade.**

### Naming convention

```
<domain>_<verb>_<object>
```

Examples:

| Good | Bad |
|------|-----|
| `ai_send_prompt` | `sendPrompt` (camelCase in Rust) |
| `pty_create` | `create_pty` (verb before domain) |
| `git_commit` | `commit` (no domain prefix) |
| `pentest_launch_tool` | `launchTool` (collision risk) |
| `mcp_list_servers` | `list_mcp_servers` (domain buried) |

Rules:

- `<domain>` matches one of the `commands_facade/<domain>.rs` files
  (`ai`, `git_pty`, `indexer`, `settings`, `sidecar`, `pentest`,
  `pipeline`, `vuln_intel`, `mcp`, `workspace`). If the command does
  not fit any existing domain, **propose a new facade file** in the PR
  description rather than adding a 12th `use` line to
  `commands_registry.rs`.
- `<verb>` is imperative English (`create`, `list`, `get`, `update`,
  `delete`, `launch`, `cancel`, `check`, `resolve`).
- `<object>` is singular noun or compound (`tool`, `session`,
  `api_key`, `github_readme`).

### Five-step checklist to ship a new command

1. **Write the `#[tauri::command]` function** in its module under
   `backend/crates/golish/src/<area>/commands/…` (e.g.
   `ai/commands/core/chat.rs`).
2. **Expose it via the facade**: add a `pub use` line in the matching
   `commands_facade/<domain>.rs` file. This keeps the domain's
   public command surface readable top-to-bottom in one file.
3. **Register it** in `commands_registry.rs` inside the existing
   `tauri::generate_handler![…]` block, under the correct domain
   section comment. Tauri's proc-macro needs flat identifiers at the
   call site, so this step is unavoidable until we adopt Option C
   (codegen).
4. **Write the typed frontend wrapper** in
   `frontend/lib/api/<domain>.ts` (e.g. `frontend/lib/api/ai.ts`).
   Never call `invoke()` directly from components — always go through
   `frontend/lib/api/`.
5. **If request/response types cross the IPC boundary**, derive them
   with `#[derive(ts_rs::TS)]` in the Rust domain crate so
   `frontend/lib/generated/` stays in sync.

### Anti-patterns

- **Skipping the facade** — e.g. adding a fresh
  `use crate::foo::commands::*;` glob back to `commands_registry.rs`.
  Will be caught in code review; facades are the canonical domain
  surface.
- **Naming without a domain prefix** — `list_tools` collided with MCP
  `mcp_list_tools` in the past. Always prefix.
- **Bare `invoke("some_cmd")` in components** — use
  `api.<domain>.<verb>` so rename-safety and IDE autocomplete work.

## Debugging a harness run (self-discoverable trace)

When a Task-mode pentest run gets stuck (e.g. a stage gate keeps blocking), you
do **not** need to grep `~/.golish/backend.log` by hand. The harness emits its
decisions (gate PASS/BLOCK, background evidence booked) as first-class
`AiEvent::HarnessTrace` events that land in the run transcript, and there is a
unified, operation-scoped view. Design:
`docs/design/2026-06-05-unified-ai-harness-observability.md`.

Three self-service steps (no need to point an AI at files):

1. **Read the manifest / call the tool.** The in-product agent can call the
   `harness_trace` tool (no args → the current run) to get the merged,
   decision-only timeline of the main agent **and** every sub-agent, each line
   tagged with an `agent_path` (`main`, `main>pentester`, `main>pentester>reporter`).
2. **`just replay <session-id>`** — prints the manifest summary + the merged
   timeline for a run. `<session-id>` is the directory name under
   `~/.golish/transcripts/`. This reads transcripts only (no app/DB startup) and
   also writes `manifest.json` + `timeline.jsonl` into that session directory.

   ```bash
   just replay pentest-chat-abc123
   ```

   You'll see, e.g., `submit needs_fix cited=[1,2,3]` immediately followed by
   `gate BLOCK` — the "cited placeholders while real ids existed" failure mode at
   a glance.
3. **Need per-check detail?** Raise the log level to show harness decisions while
   hiding token-level streaming, then grep by operation:

   ```bash
   RUST_LOG="golish=info,harness=debug" just dev
   # then:
   rg "harness::" ~/.golish/backend.log
   ```

   The `harness=debug` directive surfaces the `harness::hook` / `harness::evidence`
   / `harness::gate::*` targets; `golish=info` keeps the per-token `trace!` noise
   off. (Settings: `advanced.log_level` controls the default.)

Trace files per run live next to the transcript:

```text
~/.golish/transcripts/<session-id>/
├── transcript.json     # main-agent events (existing)
├── subagents/<id>/transcript.json   # per sub-agent (existing)
├── timeline.jsonl      # merged main+subagent+harness, ordered (just replay)
└── manifest.json       # one-glance summary: status, stages, agent_paths, last_decision
```

See also:
- [Browser-only frontend development](browser-dev.md)
- [Architecture](architecture.md)
