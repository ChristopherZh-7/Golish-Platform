# ADR-0001: Tauri 2 vs Electron for Desktop Shell

## Status

Accepted

## Context

Golish Platform is a desktop-native penetration-testing workbench that orchestrates
security tools (Nuclei, Feroxbuster, ZAP …), manages long-running scans, and
surfaces AI-driven vulnerability analysis. The app needs:

- **Low memory overhead** — users run memory-intensive tools alongside the UI.
- **Native process control** — spawn, signal, and stream stdout of child processes
  (PTY, MCP stdio transports, scanner binaries).
- **Rust-native backend** — the entire AI/pentest/indexer stack is already Rust;
  IPC should be zero-copy where possible.
- **Cross-platform** — macOS primary, Linux secondary, Windows stretch goal.
- **Minimal bundle size** — security tooling users often work on constrained VMs.

## Decision

Use **Tauri 2** (`tauri = "2"`, `@tauri-apps/api ^2.10`) as the desktop shell.

Key reasons:

1. **Shared language** — backend crates link directly into the Tauri process;
   no FFI layer, no sidecar protocol overhead for hot-path calls.
2. **Memory** — WebView2/WKWebView reuses the OS web engine; idle RSS is
   ~40 MB vs Electron's ~150 MB.
3. **Process model** — `tauri-plugin-shell` and `portable-pty` give first-class
   PTY support; the Rust event loop can `tokio::spawn` scanners without IPC
   round-trips.
4. **Tauri 2 stability** — v2 ships stable IPC, multi-window, and plugin APIs
   that were missing in v1.

## Consequences

### Positive

- 3–4× smaller bundle (~15 MB vs ~60 MB Electron).
- Single-language backend; `cargo check --workspace` covers UI commands + domain
  logic in one pass.
- Tauri 2 plugin ecosystem (dialog, notification, shell) covers our needs.

### Negative

- Frontend must target the OS WebView engine (Safari on macOS), not Chromium;
  some CSS/JS APIs behave differently.
- Smaller community than Electron; fewer off-the-shelf plugins.
- Debugging requires both browser DevTools and `RUST_LOG` tracing — steeper
  learning curve for frontend-only contributors.

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **Electron** | 150 MB+ RSS, ships Chromium, Rust ↔ Node IPC adds latency and complexity. |
| **Neutralinojs** | Immature plugin system; no Rust-native backend integration. |
| **Wails (Go)** | Would force rewriting the Rust backend in Go or maintaining a sidecar. |
| **Pure CLI + TUI** | Loses the rich diff/graph/artifact UX that distinguishes Golish from CLI tools. |
