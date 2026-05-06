/**
 * AI agent IPC wrappers — re-exports the entire `frontend/lib/ai/`
 * subdirectory so the facade stays in sync with upstream automatically
 * (mirrors the glob pattern used for `mcp`, `indexer`, etc).
 *
 * The implementation files still live under `frontend/lib/ai/`:
 * - `persistence.ts` — chat session save/load/agent definitions/HITL config
 * - `session.ts`     — agent lifecycle, prompt streaming, sub-agents
 * - `approval.ts`    — tool approval policy, agent mode, execution mode
 * - `providers.ts`   — provider initialization (Anthropic/OpenAI/Vertex/Z.AI)
 * - `models.ts`, `tool-source.ts`, `streaming-buffer.ts`,
 *   `generation-suppress.ts`, `types.ts` — domain helpers
 *
 * Future cleanup (Phase 2D): move the implementation files into
 * `frontend/lib/api/ai/` and turn `frontend/lib/ai/<x>.ts` into compat
 * re-exports, mirroring the mcp/indexer/sidecar pattern. Deferred
 * because the AI domain is the largest (~70+ functions, 84+ invoke
 * call sites) and warrants a dedicated migration PR.
 */

export * from "../ai";
