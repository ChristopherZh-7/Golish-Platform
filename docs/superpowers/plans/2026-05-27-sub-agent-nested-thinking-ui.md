# Sub-Agent Nested Delegation + Thinking UI Plan

Date: 2026-05-27

## Goal

Make sub-agent execution read as a hierarchy instead of a flat stream:

- Main ChatPanel shows only main-agent delegation.
- A sub-agent detail view shows that sub-agent's own timeline.
- Nested `sub_agent_*` calls appear inside the parent sub-agent detail as compact delegation cards.
- Sub-agent reasoning/thinking is persisted and shown like main-agent Thinking, instead of flashing or disappearing.

## Current State

- Backend sub-agent stream processing collects `thinking_text`, but only records it on tracing spans and thinking history. It does not emit a frontend event.
- Frontend `ActiveSubAgent` stores text, tool calls, response, and prompt generation, but has no thinking fields.
- Sub-agent detail renders every `entry.kind === "tool_call"` as a normal tool block, so nested delegation is visually indistinguishable from ordinary tools.
- `SubAgentTreeView` already reconstructs nested relationships with `child.parentRequestId === parent.toolCalls[id]`; reuse that relationship rather than inventing a new tree model.

## Implementation

1. Add backend wire event
   - Add `AiEvent::SubAgentReasoning { agent_id, delta, accumulated, parent_request_id }`.
   - Add CLI JSON conversion as `sub_agent_reasoning`.
   - Emit it from sub-agent stream processing for standard/fallback reasoning chunks.

2. Store sub-agent thinking
   - Add `thinking`, `thinkingStartedAt`, `thinkingEndedAt` to `ActiveSubAgent`.
   - Add `updateSubAgentThinking(sessionId, parentRequestId, accumulated)`.
   - Sync changed sub-agent state back to timeline blocks.

3. Render thinking in detail
   - Reuse `AIChatPanel/ThinkingBlock`.
   - Active while sub-agent is running and there is no visible text/tool entry yet.
   - Once text/tool/nested delegation appears, it collapses to a settled "Thought for ..." row.

4. Render nested delegation in parent detail
   - When a sub-agent timeline entry points to a tool whose name starts with `sub_agent_`, find `activeSubAgents[sessionId].find(a => a.parentRequestId === tool.id)`.
   - If found, render a compact nested sub-agent card instead of `AgentToolCallBlock`.
   - Clicking the nested card switches detail focus to that child by updating `toolDetailRequestIds` to the child `parentRequestId`.
   - If not found yet, fall back to the normal tool block so streaming races still show something.

## Non-Goals

- Do not redesign `SubAgentTreeView`; it already supports nested hierarchy.
- Do not move sub-agent final response rendering back into detail.
- Do not change sub-agent execution semantics or depth limits.
- Do not run `init.sh` for this patch; user explicitly asked not to.

## Verification

- `cargo check -p golish-core -p golish-cli-output -p golish-sub-agents`
- Targeted event serialization tests for `golish-core` and CLI JSON if practical.
- `pnpm exec biome check` on touched frontend files.
- `pnpm exec tsc --noEmit` if frontend type surface changes require it.
- `git diff --check` on touched files.
