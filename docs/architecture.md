# Architecture

> Updated 2026-05-02 after the A1–A4 architecture cleanup. This document
> reflects the **real** dependency graph (verified by `Cargo.toml` scan),
> not aspirational text. If you change any `Cargo.toml` dependency,
> update this file and the corresponding layer assertions.

## Tech stack at a glance

| Layer | Choice |
|---|---|
| Desktop shell | Tauri 2 |
| Backend | Rust 2021 (44 crates, ~177K LOC) |
| Frontend | React 19 + TypeScript 6 + Vite 8 |
| State mgmt | Zustand + Immer (14 slices) |
| UI kit | Radix primitives + Tailwind 4 |
| Editor | CodeMirror 6 · `@uiw/react-codemirror` |
| Terminal | xterm.js |
| LLM orchestration | `rig-core` 0.36 + 4 in-tree provider forks |
| Persistence | Postgres (embedded via `pg-embed`) + sqlx |
| Observability | OpenTelemetry + Langfuse (GenAI semantic conventions) |
| Tests | Vitest · Playwright e2e · Rust unit + integration |

## Repo layout

```text
golish-platform/
├── frontend/                  # React 19 app (Vite + Tauri 2 host)
│   ├── App.tsx                # root entry
│   ├── components/            # 64 domain component groups
│   ├── hooks/                 # 33 Tauri-event subscriptions
│   ├── lib/                   # 73 typed invoke() wrappers + utilities
│   ├── store/                 # Zustand store (14 slices + selectors/effects)
│   ├── pages/                 # page-level views
│   └── services/              # client-side service layer
├── backend/
│   ├── Cargo.toml             # workspace root
│   └── crates/                # 44 Rust crates (see layer table below)
├── e2e/                       # 20 Playwright spec files
├── docs/                      # this documentation
└── .github/workflows/         # CI (incl. arch-check.yml DAG guard)
```

## Backend: 6-layer DAG

Every crate sits at exactly one layer. A crate **must only depend on
lower layers** — the `.github/workflows/arch-check.yml` job enforces
this on every PR.

```text
┌───────────────────────────────────────────────────────────────────┐
│ L6 Application                                                    │
│   golish  (Tauri app · 548 IPC commands · 6 managed sub-states)   │
└─────────────────────┬─────────────────────────────────────────────┘
                      │
┌─────────────────────▼─────────────────────────────────────────────┐
│ L5 Evaluation harnesses                                           │
│   golish-evals · golish-benchmarks · golish-swebench              │
└─────────────────────┬─────────────────────────────────────────────┘
                      │
┌─────────────────────▼─────────────────────────────────────────────┐
│ L4 Agent stack (three-tier, renamed in A2)                        │
│   L4c  golish-agent-bridge   ← AgentBridge + bridge_executor      │
│   L4b  golish-agent-runtime  ← streaming loop + eval + mocks      │
│   L4a  golish-agent-kit      ← tool executors / HITL / planner    │
└─────────────────────┬─────────────────────────────────────────────┘
                      │
┌─────────────────────▼─────────────────────────────────────────────┐
│ L3 Domain services                                                │
│   golish-sub-agents · golish-tools · golish-prompts               │
│   golish-pipeline · golish-sidecar                                │
└─────────────────────┬─────────────────────────────────────────────┘
                      │
┌─────────────────────▼─────────────────────────────────────────────┐
│ L2 Simple infrastructure (only depends on L1)                     │
│   golish-events · golish-session · golish-indexer · golish-models │
│   golish-llm-providers · golish-db · golish-pty · golish-web      │
│   golish-pentest · golish-vuln-intel · golish-scan-runner         │
│   golish-shell-exec · golish-skills · golish-synthesis            │
│   golish-artifacts · golish-cli-output · golish-pentest-mcp       │
│   rig-openai-responses · rig-zai-sdk                              │
└─────────────────────┬─────────────────────────────────────────────┘
                      │
┌─────────────────────▼─────────────────────────────────────────────┐
│ L1 Foundation (no internal golish-* deps)                         │
│   golish-core · golish-settings · golish-context · golish-models  │
│   golish-mcp · golish-projects · golish-graphiti                  │
│   golish-json-repair · golish-udiff                               │
│   golish-pentest-domain · golish-vuln-intel-domain                │
│   rig-anthropic-vertex · rig-gemini-vertex                        │
└───────────────────────────────────────────────────────────────────┘
```

### Crate catalog

#### L1 — Foundation (zero internal deps)

| Crate | Purpose |
|---|---|
| `golish-core` | Shared types, events, `PromptContributor`, `ToolName`, `GolishRuntime` trait, session types |
| `golish-settings` | `GolishSettings` + TOML loader/migration + template |
| `golish-context` | Token budget + context window tracking |
| `golish-models` | Model metadata tables (IDs, capabilities) |
| `golish-mcp` | MCP protocol client + `McpManager` + transport (stdio/http/sse) |
| `golish-projects` | Project directory discovery + metadata |
| `golish-graphiti` | Knowledge graph trait |
| `golish-json-repair` | Fixes malformed JSON emitted by LLMs |
| `golish-udiff` | Unified-diff parsing + application |
| `golish-pentest-domain` | Pentest data model (pure types) |
| `golish-vuln-intel-domain` | Vulnerability intel data model |
| `rig-anthropic-vertex` | Rig provider fork (Claude on Vertex) |
| `rig-gemini-vertex` | Rig provider fork (Gemini on Vertex) |

#### L2 — Simple infrastructure (only L1 deps)

| Crate | Depends on | Purpose |
|---|---|---|
| `golish-events` | core | Agent event coordinator + transcript writer |
| `golish-session` | core | Session archive + manager |
| `golish-indexer` | settings | File tree indexing via `vtcode-indexer` |
| `golish-llm-providers` | models, settings | Provider config + model capability checks |
| `golish-db` | core | Postgres pool + migrations + gatekeeper |
| `golish-pty` | core, settings | PTY manager (`portable-pty` wrapper) |
| `golish-web` | core | HTTP fetch + readability |
| `golish-tools` | core, settings, shell-exec, web | Tool registry + ast-grep + file/dir ops |
| `golish-pentest` | core, db, pentest-domain | Pentest engine |
| `golish-vuln-intel` | core, db, vuln-intel-domain | Vuln intel client + sploitus |
| `golish-scan-runner` | core, db | External scanner adapters (nuclei, whatweb, nmap, zap) |
| `golish-shell-exec` | core | Background shell execution |
| `golish-skills` | core | Skill discovery + activation |
| `golish-synthesis` | settings | LLM-driven synthesis prompts |
| `golish-artifacts` | settings | Artifact synthesis (patches → PR) |
| `golish-cli-output` | core | CLI colored output utilities |
| `golish-pentest-mcp` | core | Pentest-specific MCP tools |
| `rig-openai-responses` | json-repair | Rig provider fork (OpenAI reasoning models) |
| `rig-zai-sdk` | json-repair | Rig provider fork (Z.AI GLM) |

#### L3 — Domain services

| Crate | Depends on | Purpose |
|---|---|---|
| `golish-prompts` | core, llm-providers | System prompt + summarizer + contributor registry |
| `golish-sub-agents` | core, json-repair, llm-providers, shell-exec, skills, tools, udiff | Sub-agent registry + executor + defaults + prompt-contributor |
| `golish-pipeline` | core, db, pentest | Pentest pipeline orchestrator |
| `golish-sidecar` | artifacts, core, settings, synthesis | Session sidecar (artifact capture) |

> **Invariant**: `golish-prompts` must **not** depend on `golish-sub-agents`
> (fixed in A1 — see `CHANGELOG.md`). CI enforces this.

#### L4 — Agent stack (three-tier)

| Crate | Depends on | Role |
|---|---|---|
| `golish-agent-kit` (L4a) | core, context, events, indexer, json-repair, llm-providers, prompts, settings, sub-agents, tools | **Building blocks**: tool executors, HITL, loop detection, planner, tool policy, sidecar trait, system hooks, db trait + tracking, llm-client wiring |
| `golish-agent-runtime` (L4b) | L4a + all L4a deps | **Streaming loop**: `run_agentic_loop_unified`, eval harness, test mocks |
| `golish-agent-bridge` (L4c) | L4a + L4b + session + indexer + llm-providers, etc. | **Bridge to Tauri**: `AgentBridge` struct, `bridge_executor`, per-turn context preparation, contributor composition |

> **Renamed in A2**: formerly `golish-agent-loop` + `golish-agentic-loop`
> (one character apart — confusing). New names make responsibility
> distinguishable in 5 seconds.

> **Umbrella removed in A3**: formerly all of the above were re-exported
> via a `golish-ai` facade crate. That umbrella was a backward-compat
> leftover; consumers now depend on the specific crate.

#### L5 — Evaluation harnesses

| Crate | Purpose |
|---|---|
| `golish-evals` | Reusable agent executor for benchmarks |
| `golish-benchmarks` | HumanEval + custom benchmark scenarios |
| `golish-swebench` | SWE-bench Lite integration (Docker-based) |

#### L6 — Application

| Crate | Purpose |
|---|---|
| `golish` | Tauri app entry + 548 `#[tauri::command]` IPC handlers + embedded Postgres bootstrap + CLI mode |

## Frontend architecture

### Store (Zustand + Immer)

Single `useStore` hook composed from **14 slices**, each owning one
domain:

| Slice | Responsibility |
|---|---|
| `appShell` | Window geometry, focus, menu state |
| `appearance` | Theme / font / tokens |
| `ai` | Agent streaming buffers, tool calls, selected model |
| `conversation` | Timeline blocks, messages |
| `session` (4 sub-slices) | core · streaming · tabs · terminal · draft types · helpers |
| `context` | Context window display + trim |
| `panel` | Panel-level layout |
| `pane` | Pane-level layout |
| `git` | Git status + diff |
| `dialog` | Modals |
| `notification` | Toasts + banners |
| `hitl` | Human-in-the-loop approvals |
| `workflow` | Multi-step workflows |

Each slice ships with dedicated `selectors.ts` + `selectors.test.ts` +
`selectors.performance.test.ts` + `effects/` folder.

### Tauri IPC layer

Frontend calls the 548 backend commands via typed wrappers in
`frontend/lib/` (one file per domain, e.g. `ai.ts`, `git.ts`). There
is no runtime "router" — each command is a flat namespace. A future
namespace design is tracked in `docs/adr/` (see B4 in the refactor
plan).

### Managed Tauri state (A4)

The Tauri runtime manages **six independent sub-states** in addition
to the legacy `AppState`:

| Sub-state | Used by commands |
|---|---|
| `AppState` | Cross-domain commands (AI init, MCP refresh, sidecar apply-patch) |
| `DbState` | `sensitive_scan` · `zap/*` (+10 commands) |
| `TelemetryState` | `is_langfuse_active` · `get_telemetry_stats` |
| `McpManaged` | `mcp_list_servers` · `mcp_list_tools` |
| `PtyState` | All 7 `commands/proc/pty.rs` commands |
| `SidecarManaged` | 26 of 30 `sidecar/commands/*` |

Sub-states share the same underlying `Arc`s as `AppState`, so data
stays consistent. New domain-specific commands should prefer the
narrow sub-state over `AppState`.

## Evolution principles

1. **No back-edges** — A crate at layer N must only depend on crates
   at layers < N. CI enforces this (`arch-check.yml`). If the
   temptation arises to go back-edge, split out a new L_k crate
   instead.
2. **File size budget** — no Rust business file > 500 lines, no
   TS/TSX file > 800 lines (tests / fixtures excluded). CI
   enforces this.
3. **Every new crate writes its layer contract in `lib.rs`** — module
   docs must state "depends on / consumed by / layer".
4. **Prefer narrow Tauri State over AppState** — new commands take
   `State<'_, DbState>` / `State<'_, PtyState>` / etc. Use
   `State<'_, AppState>` only when a command genuinely crosses
   domains.

## Related docs

- [Refactor history + future work](../.cursor/rules/refactor-execution.mdc) — executable roadmap (A1–A4 done · C1 pending)
- [Independent architecture evaluation](../.cursor/rules/architecture-evaluation.mdc) — third-party assessment
- [Planning system](planning-system.md)
- [System hooks](system-hooks.md)
- [Tool use](tool-use.md)
- [MCP integration](mcp.md)
- [Prompt contributions](prompt-contributions.md)
- [Langfuse tracing](langfuse-tracing.md)
