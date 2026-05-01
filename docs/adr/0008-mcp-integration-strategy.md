# ADR-0008: MCP Integration Strategy via rmcp

## Status

Accepted

## Context

Golish integrates with external tool servers using the **Model Context Protocol
(MCP)** — an open protocol for LLM ↔ tool communication. Use cases:

- **Pentest tool servers** — `golish-pentest-mcp` exposes scanner tools
  (Nuclei, ZAP, Feroxbuster) as MCP tools.
- **User-installed MCP servers** — users can register arbitrary MCP servers
  (e.g., filesystem, database, custom security tools) that the AI agent
  discovers and invokes at runtime.
- **Dynamic tool discovery** — the agent queries available tools from each
  MCP server and incorporates them into its tool-use schema.

Requirements:

- **Rust-native client** — must run in the Tauri backend process without a
  Node.js sidecar.
- **Multiple transports** — support stdio (child process), SSE (HTTP
  streaming), and Streamable HTTP.
- **Concurrent sessions** — manage multiple MCP server connections
  simultaneously.
- **Tool schema bridging** — convert MCP tool definitions to rig-core `Tool`
  trait implementations for seamless integration with the agentic loop.

## Decision

Use **`rmcp = "0.14"`** as the MCP client library with the following transport
features enabled:

- `client` — core client functionality
- `client-side-sse` — Server-Sent Events transport
- `transport-child-process` — stdio transport for local MCP servers
- `transport-io` — generic I/O transport
- `transport-streamable-http-client` — Streamable HTTP transport
- `transport-streamable-http-client-reqwest` — reqwest-based HTTP client

The `golish-mcp` crate provides:

1. **Loader** — discovers and connects to configured MCP servers (from
   settings or project config).
2. **Tool bridge** — converts `rmcp` tool definitions into rig-core `Tool`
   implementations that the agent can invoke.
3. **Trust management** — user-controlled allow/deny lists for MCP server
   tool access (integrated with `golish-settings`).
4. **OAuth flow** — handles MCP server authentication when required.

## Consequences

### Positive

- Full MCP spec compliance via a maintained Rust crate; new MCP features
  (resources, prompts, sampling) are available as `rmcp` adds support.
- Three transport modes cover all deployment scenarios: local CLI tools
  (stdio), remote servers (SSE), and modern HTTP streaming.
- Tool bridge to rig-core means MCP tools appear identical to built-in tools
  from the agent's perspective — no special-casing in the agentic loop.
- Trust management gives users fine-grained control over which MCP tools
  the AI agent can invoke.

### Negative

- `rmcp` is pre-1.0; breaking changes are possible across minor versions.
- MCP server lifecycle management (start/stop/health-check) adds complexity
  to the startup flow.
- Each connected MCP server holds a tokio task and transport connection;
  many servers may increase resource usage.
- OAuth/auth flows for remote MCP servers require UI integration (redirect
  handling via the Tauri shell plugin).

## Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **Custom MCP client** | MCP spec is non-trivial (JSON-RPC 2.0 + SSE + capability negotiation); reimplementing is error-prone and wasteful given `rmcp` exists. |
| **mcp-rs (alternative crate)** | Less mature than `rmcp`; missing SSE and Streamable HTTP transports at evaluation time. |
| **Node.js MCP SDK in sidecar** | Would require a Node.js runtime, IPC bridge, and process management — contradicts our Rust-native architecture. |
| **Direct HTTP API calls (no MCP)** | Loses the standardized tool discovery and schema negotiation that MCP provides; each integration would be bespoke. |
| **gRPC** | Not compatible with the MCP ecosystem; would isolate us from the growing MCP tool server community. |
