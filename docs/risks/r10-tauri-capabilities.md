# R10 — Tauri Capabilities Fine-Grained Permissions

> Status: **Risk identified, full implementation deferred to a dedicated security PR.**
> Last updated: 2026-05-02.

## Current state

`backend/crates/golish/capabilities/default.json` declares ~17 permissions:

- `core:default` + `core:event:*` (all events)
- `core:window:*` (full window control)
- `dialog:default`, `shell:allow-open`
- `notification:*`

**It does NOT restrict access to any of the 548
`#[tauri::command]` functions.** Every IPC command is implicitly
allowed for the `main` and `detached-*` windows.

## Why this is a P2 risk

Tauri 2's capability system was designed precisely so the webview
cannot invoke arbitrary commands. Without per-command allowlists:

- A compromised npm dependency in `frontend/` (supply-chain attack)
  can call any backend IPC, including `vault_get_value`,
  `pentest_launch_tool`, `shell_integration_install`, etc.
- A `<iframe>` or detached window — if it can be navigated to a
  malicious origin — has the same IPC surface as the main window.
- The CSP is currently `null` (R2), so a single XSS payload can
  bootstrap into IPC abuse.

The risk surface scales with the 548-command count: every new
domain handler adds a default-allowed entry point.

## Recommended fix path

1. **Audit phase (1d)**: classify all 548 commands into 4 buckets:
   - `safe` — read-only metadata (e.g. `get_settings`, `list_themes`)
   - `mutating` — write back to local state (e.g. `target_add`)
   - `sensitive` — touch credentials / shell / launch external (e.g.
     `vault_get_value`, `pentest_launch_tool`, `pty_create`)
   - `internal` — only for cross-window IPC, not user-triggered
2. **Window-scoping (0.5d)**: restrict `detached-*` capability to
   `safe` + `mutating` for the specific tab type, no `sensitive`.
3. **Plugin migration (1d)**: for each `sensitive` command, prefer
   the corresponding Tauri plugin (`tauri-plugin-shell`,
   `tauri-plugin-fs`, etc.) where allowlists are richer.
4. **Per-command allow gate (1d)**: replace `core:default` with an
   explicit list of allowed `command:*` entries. Add to
   `capabilities/default.json`:
   ```json
   "permissions": [
     ... existing ...,
     { "identifier": "command:get_settings", "allow": [...] },
     ...
   ]
   ```
5. **CI guard (0.5d)**: add a `scripts/check_capabilities.py` that
   compares `commands_facade/<domain>.rs` against the capabilities
   allowlist and fails CI if a command is registered but not allowed
   (or vice versa).

Total: 4 days for full coverage.

## Quick win available now

**Without doing the full audit**, a single safe change reduces blast
radius today: lock down `detached-*` windows to a minimal capability
since they only render `Terminal` / security tabs. Open
`backend/crates/golish/capabilities/` and add a `detached.json`:

```json
{
  "identifier": "detached",
  "description": "Capability for floating tab windows",
  "windows": ["detached-*"],
  "permissions": [
    "core:event:allow-listen",
    "core:event:allow-emit",
    "core:window:allow-close",
    "core:window:allow-set-title"
  ]
}
```

Then drop `"detached-*"` from `default.json`'s `windows` array.

Result: detached windows lose access to `dialog:*`,
`shell:allow-open`, `notification:*`, etc. — none of which they need.

## References

- Tauri 2 capabilities docs: <https://v2.tauri.app/reference/acl/capability/>
- R2 (CSP null) — from the legacy architecture review; source review file may no longer be present
