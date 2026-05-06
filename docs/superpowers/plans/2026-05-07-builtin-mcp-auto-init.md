# Built-in MCP Server Auto-Initialization Plan

## Problem

Built-in MCP servers (e.g. `js-reverse`) ship as source code under `tools/`.
On a fresh machine they fail to start because:

1. **Node.js may not be installed** — the backend calls `node build/src/index.js` but `node` is absent.
2. **Dependencies not installed** — `node_modules/` does not exist.
3. **Build artifacts missing** — `build/` directory does not exist; source code needs `npm install && npm run build`.
4. **No UI guidance** — MCP Settings just shows "Disconnected" with no explanation or fix action.

The Environment settings page and the MCP Servers page are unaware of each other.

## Goals

1. On a release build, built-in MCP tools start without user intervention.
2. On a dev build, auto-initialize when Node.js is detected.
3. When Node.js is unavailable, guide the user clearly: install Node.js first, then auto-setup.
4. Environment → MCP linkage: when user installs Node.js via Environment settings, re-check and auto-initialize MCP servers.

## File Map

| File | Responsibility |
|------|----------------|
| `backend/crates/golish-mcp/src/loader/mod.rs` | Built-in config resolution + **new: auto-init check** |
| `backend/crates/golish-mcp/src/loader/auto_init.rs` | **New**: detect missing deps, run npm install/build |
| `backend/crates/golish-mcp/src/manager.rs` | Server lifecycle (connect / disconnect / reconnect) |
| `backend/crates/golish-core/src/commands/mcp.rs` | Tauri commands exposed to frontend |
| `frontend/components/Settings/McpSettings.tsx` | MCP Settings UI — **new: setup guidance for built-in** |
| `frontend/components/Settings/PentestEnvSettings/index.tsx` | Environment page — **new: trigger MCP re-init after runtime install** |
| `frontend/lib/api/mcp.ts` | Frontend MCP API bindings |
| `frontend/lib/i18n/en.json` | English translations |
| `frontend/lib/i18n/zh-CN.json` | Chinese translations |
| `tauri.conf.json` or build script | **Release builds: pre-build tools/** |

## Design

### Layer 1: Build-time (Release Builds Only)

Add a `beforeBuildCommand` or Tauri build hook that runs:
```bash
cd tools/js-reverse-mcp && npm ci && npm run build
```

This ensures the release binary ships with `build/src/index.js` and `node_modules/` ready.

**Change**: `src-tauri/tauri.conf.json` or a build script in `scripts/`.

### Layer 2: Runtime Auto-Init (Dev Mode)

When the backend resolves a built-in server and finds the entry point but `node_modules/` or `build/` is missing:

1. Check if `node` is on PATH.
2. If yes → spawn `npm install && npm run build` in the tool directory.
3. Wait for completion, then attempt to start the MCP server.
4. If no → mark server status as `"needs_setup"` (new status) with `setup_reason: "node_not_found"`.

**New file**: `backend/crates/golish-mcp/src/loader/auto_init.rs`

```rust
pub struct AutoInitResult {
    pub success: bool,
    pub message: String,
}

pub async fn auto_init_builtin(tool_dir: &Path, entry_point: &str) -> AutoInitResult {
    // 1. Check if build artifact exists
    // 2. Check if node_modules exists
    // 3. If either missing, run npm install && npm run build
    // 4. Return result
}
```

### Layer 3: Frontend Guidance

#### MCP Settings page (`McpSettings.tsx`)

When a built-in server has status `"needs_setup"`:
- Show amber banner: "This server requires Node.js. Install it in Environment settings, then click Setup."
- "Setup" button triggers `mcp_auto_init_builtin` Tauri command
- After setup completes, auto-reconnect

#### Environment → MCP linkage

In `PentestEnvSettings` after a successful `installRuntime("nvm")`:
- Call `listServers()` to check if any built-in MCP servers are in `needs_setup` state
- If yes, auto-trigger `mcp_auto_init_builtin` for each
- Show toast: "Node.js installed. Initializing built-in MCP servers..."

### API Contract

```
# New Tauri command
POST pentest_auto_init_builtin_mcp
Req: { server_name: string }
Res 200: { success: boolean, message: string }
Res 400: { code: 40001, message: "Node.js not found" }

# Extended server info
GET mcp_list_servers → McpServerInfo[]
  New fields:
    setup_status: "ready" | "needs_build" | "needs_node" | null
    setup_message: string | null
```

## Tasks

### Task 1: Build-time pre-build (Release)
- [ ] Add build script to run `npm ci && npm run build` in `tools/js-reverse-mcp/`
- [ ] Hook into Tauri's `beforeBuildCommand` or `beforeBundleCommand`
- [ ] Verify: `cargo tauri build` produces working MCP server
- **Files**: Build script, `src-tauri/tauri.conf.json`

### Task 2: Backend auto-init logic
- [ ] Create `auto_init.rs` with `auto_init_builtin()` function
- [ ] Detect missing `node_modules/` or `build/` directory
- [ ] Check Node.js availability via `which node`
- [ ] Spawn npm process and collect output
- [ ] Add `setup_status` field to `McpServerInfo`
- [ ] New Tauri command `pentest_auto_init_builtin_mcp`
- [ ] Integration: call auto-init during server startup in `manager.rs`
- **Files**: `auto_init.rs`, `mod.rs`, `manager.rs`, `commands/mcp.rs`
- **Tests**: Unit test auto_init with mock file system

### Task 3: Frontend MCP Settings guidance
- [ ] Detect `setup_status` in server list
- [ ] Show "needs setup" banner with actionable button
- [ ] "Setup" button calls `pentest_auto_init_builtin_mcp`
- [ ] Show progress indicator during setup
- [ ] Auto-reconnect after successful setup
- [ ] i18n: `mcp.needsSetup`, `mcp.setupInProgress`, `mcp.setupComplete`, `mcp.needsNode`
- **Files**: `McpSettings.tsx`, `mcp.ts`, `en.json`, `zh-CN.json`

### Task 4: Environment → MCP linkage
- [ ] After `installRuntime("nvm")` succeeds, check for `needs_setup` MCP servers
- [ ] Auto-trigger initialization
- [ ] Show toast notification
- **Files**: `PentestEnvSettings/index.tsx` or `usePentestEnvForm.ts`

### Task 5: Cleanup & Documentation
- [ ] Remove `SetupHealthBanner.tsx` if no longer needed (currently unused)
- [ ] Clean up navigate event infrastructure if unused
- [ ] Update README/docs with built-in MCP setup flow

## Risk & Open Questions

- **npm ci vs npm install**: CI environments should use `npm ci`; dev mode can use `npm install`
- **Build time**: `npm install` for `js-reverse-mcp` may take 30+ seconds; need progress UI
- **Node.js version**: `js-reverse-mcp` requires `^20.19.0 || ^22.12.0 || >=23`; system Node may be too old
- **Multiple built-in servers**: Design should be generic, not hardcoded to `js-reverse`
- **Offline mode**: `npm install` requires network; consider vendoring `node_modules` in release
