# ADR-0009: Tauri Command Namespace Strategy

## Status

**Proposed** (2026-05-02). Not yet implemented — this ADR enumerates the
options and their trade-offs so a decision can be made deliberately in a
future cycle. Until then, all commands live in the flat registry in
`backend/crates/golish/src/commands_registry.rs`.

## Context

After A1–A4 landed, the Golish `golish` crate exposes **548
`#[tauri::command]` functions** via a single
`tauri::generate_handler![…]` macro invocation (split out into
`commands_registry.rs` and brought back into `lib.rs` with `include!`
because Tauri's `#[macro_export]`-ed internal symbols must be in scope
at the generate-handler call site).

Symptoms of the flat registry:

1. **No grouping** — frontend code must know 548 distinct command names
   without any hierarchy. IDE autocomplete is a wall of strings.
2. **Startup cost** — every command is resolved at Tauri init, even if
   it's never used (there is no lazy binding).
3. **Attack surface** — every IPC endpoint is implicitly a trust
   boundary. Flat enumeration makes it hard to reason about per-domain
   auth/rate limits/auditing.
4. **Name collisions** — adding a `list_tools` command for pentest
   tooling almost clashed with the MCP `list_tools` handler. Only an
   `mcp_` manual prefix prevented the collision.
5. **Refactor hazard** — renaming a command means touching both sides
   of the IPC contract in lockstep, with no compile-time guard.

Contributors already feel this: `commands_registry.rs` is organized
into sections with comment headers (`// ── git_pty (PTY / shell / git /
themes / IME) ───────────`), which is a documentation substitute for
missing language support.

## Decision

**Defer a full namespace rollout**. Instead, this ADR:

1. **Adopts a naming convention** for all new commands:
   `<domain>_<verb>_<object>` (e.g. `ai_send_prompt`, `pty_create`,
   `git_commit`). The registry header comments already follow this
   informally — formalize it in CONTRIBUTING.md.
2. **Catalogs three concrete migration paths** below so that the next
   architecture cycle can pick without re-doing the research.

## Options evaluated

### Option A — Manual bundles per domain

Each `commands/*` module exposes a `pub fn handlers() -> Vec<Handler>`
or a macro-expanded tuple. The `golish` lib.rs composes them:

```rust
tauri::generate_handler![
    ..ai::commands::handlers(),
    ..pty::commands::handlers(),
    ..git::commands::handlers(),
    // …
]
```

**Pros**
- Clear domain grouping in source code.
- Each domain's handler list is co-located with its commands.
- Per-domain testing becomes easier (spin up a Tauri mock with only one
  bundle).

**Cons**
- Tauri 2's `generate_handler!` is a proc-macro that expects
  identifiers at call-site; splat (`..`) syntax doesn't work. Needs a
  helper macro (`tauri::collect_commands!`) or manual flattening.
- Discoverability doesn't change at the IPC boundary — command names
  are still flat strings on the wire.

**Effort**: 1 week to refactor all 548 commands into ~15 domain bundles.
**Risk**: low. **Reversible**: yes.

### Option B — `tauri::collect_commands!` + auto-namespace via prefix

Relies on Tauri 2's `tauri::collect_commands!` macro (stable since 2.1).
Commands keep their flat identifiers but are registered in chunks and
the frontend `invoke()` wrapper rewrites `domain.verb` → `domain_verb`
before the call.

**Pros**
- Minimal backend churn: only the registry composition changes.
- Frontend gets namespace ergonomics (`api.ai.send(...)` instead of
  `invoke('ai_send_prompt', ...)`).
- Bundle split is natural.

**Cons**
- Tauri still sees flat names on the IPC wire — no native namespace
  benefit for security isolation.
- Frontend helper needs careful typing to keep TypeScript inference.

**Effort**: 3–5 days.
**Risk**: medium — requires new frontend layer at
`frontend/lib/invoke.ts` on top of the existing 73 wrapper files.
**Reversible**: yes, since the helper is a thin facade.

### Option C — Code generation from `#[tauri::command]` attribute scan

Add a `build.rs` in `golish` crate that scans the source tree for
`#[tauri::command]` annotations and emits:
- `commands_registry.rs` (currently hand-maintained)
- A typed TypeScript client in `frontend/lib/generated/ipc.ts`

**Pros**
- Single source of truth. Rename a command → frontend & registry
  update in the same build.
- Can attach metadata per-command (auth level, rate limit, audit
  category) via additional attributes like
  `#[tauri::command(audit = "sensitive")]`.
- Enables compile-time attack-surface inventories.

**Cons**
- Significant build-time complexity; `build.rs` scanning is brittle if
  files move.
- Requires the team to adopt a custom code-generator; onboarding cost.
- TypeScript generation is a commitment — every command's input/output
  must be `ts-rs`-compatible.

**Effort**: 2–3 weeks including TS generator.
**Risk**: high — touches build pipeline and introduces a new
maintenance surface.
**Reversible**: moderate (generator output can be frozen into
hand-edited files if the experiment fails).

## Consequences

- **If we do nothing**: the flat registry scales linearly — at 1,000
  commands the `commands_registry.rs` macro invocation will approach
  proc-macro expansion limits, and the 70+ frontend wrapper files will
  be further fragmented.
- **If we pick A**: incremental, low-risk. Recommended as a stepping
  stone to B or C.
- **If we pick B**: wins the frontend DX battle but doesn't fix
  security/audit story.
- **If we pick C**: solves the whole problem but is a one-time
  architecture spend.

## Recommendation for next cycle

Sequence:

1. **Now** — adopt the naming convention + formalize it in
   CONTRIBUTING.md (zero cost).
2. **Next cycle** — execute Option A (one week of boring refactor) to
   create domain bundles. This is the reversible prerequisite for B/C.
3. **After Option A** — reassess. If the main pain has become frontend
   DX, do B. If it's shifted to audit/security, do C. If neither, stop.

This ADR will be revisited when Option A ships or when command count
exceeds 700.

## References

- `backend/crates/golish/src/commands_registry.rs` — current flat
  registry (205 LOC, 548 commands).
- `backend/crates/golish/src/lib.rs:77` — `include!` workaround for
  Tauri's macro-hygiene limitation.
- [Tauri 2.1 release notes](https://tauri.app/blog/tauri-2-1/) —
  `collect_commands!` macro.
- [ADR-0001](./0001-tauri2-vs-electron.md) — original Tauri decision.
