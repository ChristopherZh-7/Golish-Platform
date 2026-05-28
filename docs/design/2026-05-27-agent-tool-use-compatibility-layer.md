# Agent Tool-Use Compatibility Layer

> Status: Design Draft (2026-05-27)
> Scope: Golish AI chat / task agent tool-call reliability, approval barriers, provider compatibility, and observability
> Related: `docs/design/2026-05-27-add-xiaomi-mimo-provider.md`, `docs/design/2026-05-26-harness-observability-plane.md`, `backend/crates/golish-agent-runtime/`

## 1. Problem

MiMo live testing exposed a class of failures that the current runtime does not make explicit enough:

1. The model sometimes emits tool calls as text markup, for example `<tool_call><function=ask_human>...`, while the backend sees `tool_calls=0`.
2. The model can ask the user for confirmation in prose, then later self-answer in prose, without a real `ask_human` request or approval event.
3. The UI shows a completed tool card and natural-language continuation, but the backend logs mostly show event types and token counts, so it is hard to tell what the model wanted, what Golish parsed, what Golish rejected, and what actually executed.
4. The same task may work with a smaller Mistral model but fail with MiMo, because different providers differ in native tool-call fidelity, strict schema adherence, tool-choice support, and streaming event shape.

This is not only a provider bug and not only a prompt bug. It is a missing runtime boundary. Golish needs a compatibility layer between provider output and tool execution.

## 2. Goals

- Normalize native and recovered tool calls into one internal `ToolIntent` shape.
- Make provider/model tool-use reliability explicit in model/provider metadata.
- Put deterministic policy gates between model intent and execution.
- Treat `ask_human` and side-effecting pentest actions as hard barriers that cannot be satisfied by model text.
- Add traceable observability for every step: raw model output, parsed intents, rejected intents, approval requests, approval decisions, tool results, and continuation.
- Keep existing reliable native tool-call providers on the fast path.

## 3. Non-Goals

- Do not replace `rig-core` or provider forks in this phase.
- Do not redesign every tool schema.
- Do not implement a full pentest stage harness here. This layer feeds the existing harness and evidence work, but it is narrower: model output to tool execution.
- Do not allow textual fallback to bypass safety. Recovered textual calls are less trusted than native calls.

## 4. Current Shape

The effective flow today is:

```text
Provider stream
  -> stream_processor
  -> Vec<ToolCall>
  -> assistant message push
  -> tool_dispatch
  -> HITL approval for some tools
  -> tool executor
  -> tool_result message
  -> next loop iteration
```

That works when the provider emits structured tool calls correctly. When it emits textual XML/JSON-like markup, the runtime historically treated it as normal assistant text. A reflector can nudge the model to retry, but that is advisory and can burn several iterations without surfacing the real reason to the user.

## 5. Proposed Architecture

```text
LLM Provider
  -> Provider Adapter
  -> Tool Intent Normalizer
  -> Policy / Safety Gate
  -> Approval / ask_human Barrier
  -> Tool Executor
  -> Observation / Trace
  -> LLM Continuation
```

### 5.1 Provider Adapter

Provider adapters remain responsible for transport-specific stream conversion. They should also expose tool-use capability metadata:

```rust
pub enum ToolCallMode {
    NativeStrict,
    NativeBestEffort,
    TextualXmlFallback,
    TextualJsonFallback,
    Disabled,
}

pub enum ToolCallReliability {
    Reliable,
    NeedsAdapter,
    ChatOnly,
}

pub struct ToolUseProfile {
    pub mode: ToolCallMode,
    pub reliability: ToolCallReliability,
    pub supports_required_tool_choice: bool,
    pub supports_parallel_tool_calls: bool,
    pub max_tool_calls_per_turn: usize,
    pub requires_tool_result_balance: bool,
}
```

Initial classification:

| Provider/model family | Initial mode | Why |
|---|---|---|
| OpenAI / Anthropic native paths | `NativeStrict` or `NativeBestEffort` | Native tool call protocol is expected |
| MiMo OpenAI-compatible path | `NativeBestEffort` + textual recovery enabled | Live logs show textual `<tool_call>` attempts |
| MiMo Anthropic-compatible path | `NativeBestEffort` + textual recovery enabled | Needs live E2E per protocol |
| Local/chat-only models | `Disabled` or `TextualJsonFallback` by explicit opt-in | Avoid pretending unreliable models are agent-capable |

### 5.2 Tool Intent Normalizer

All model requests become `ToolIntent` before execution:

```rust
pub enum ToolIntentSource {
    NativeToolCall,
    TextualXml,
    TextualJson,
    Recovered,
}

pub struct ToolIntent {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
    pub source: ToolIntentSource,
    pub confidence: f32,
    pub raw_span: Option<String>,
}
```

Rules:

- Native tool calls get highest confidence.
- Textual XML/JSON calls are recovered only for providers whose `ToolUseProfile` permits recovery.
- If one model output contains `ask_human` plus later side-effecting calls, only `ask_human` is allowed to proceed in that iteration.
- Malformed recovered calls become observable rejected intents, not silent prose.

### 5.3 Policy / Safety Gate

Before dispatch, a deterministic gate evaluates each intent:

```rust
pub enum ToolGateDecision {
    Allow,
    RequireApproval { reason: String },
    RequireHumanAnswer { question: String },
    Reject { reason: String },
}
```

Baseline policies:

- `ask_human` is a hard barrier. Only a real user response event can satisfy it.
- `manage_targets add` requires a preceding target lookup, target value validation, and either an explicit user approval event or a safe auto-add policy configured outside the model.
- `run_pipeline` requires a registered in-scope target.
- Natural language such as "好的，我先添加" is not authorization.
- Recovered textual side-effecting calls require stricter approval than native calls.
- Prefer one side-effecting tool per iteration for `NeedsAdapter` models.

### 5.4 Observability

The runtime should record a compact trace for each turn:

```text
model_output.raw_preview
tool_intents.native[]
tool_intents.recovered[]
tool_intents.rejected[]
gate.decisions[]
approval.requests[]
approval.decisions[]
tool_results[]
continuation.prompt_summary
```

The console log should answer:

- What did the model emit?
- Did Golish parse a tool intent?
- Did Golish reject or require approval?
- What request ID is waiting?
- Did a tool actually execute?
- Where is the transcript/trace file?

The UI "Details" view should separate:

- "Model wanted"
- "Golish allowed"
- "Golish executed"
- "Waiting for user"

This distinction matters for security reviews and for debugging provider compatibility.

## 6. Relationship To Cursor / Windsurf Style Agents

Large agentic tools generally avoid trusting one raw model string. They layer:

- provider-specific tool adapters,
- strict schemas where the provider supports them,
- tool choice controls such as required/none/auto,
- deterministic state machines around pending approvals,
- retry/reflection nudges for recoverable model mistakes,
- trace views showing tool calls, arguments, approvals, and outputs.

Golish should follow that pattern, with a stronger security posture because pentest actions have scope and authorization semantics.

## 7. MiMo-Specific Position

MiMo should stay supported, but it should be marked as a model family that may need adapter help until live E2E proves native structured tool calls are stable.

Immediate behavior should be:

- recover textual XML-style calls into `ToolIntent` when safe,
- prefer `ask_human` over later side-effecting calls in the same textual block,
- never execute a self-approved side-effecting action,
- log a warning whenever textual recovery happened,
- show recovered calls in Details as recovered, not native.

## 8. Rollout

Phase 0 is already partially done in this session:

- UI strips raw `<tool_call>` markup from visible assistant prose.
- Runtime detects textual tool-call markup.
- Runtime can convert MiMo XML-style textual calls into structured `ToolCall`.
- Event coordinator logs completed response previews and suspicious textual tool-call markup.

The remaining work should move from ad hoc recovery to a named compatibility layer with tests and UI trace affordances.

## 9. Risks

- Over-recovering textual markup could execute something the model merely discussed. Mitigation: provider opt-in, confidence, gate decisions, approval barriers.
- Too much logging could leak secrets. Mitigation: truncate previews and redact known secret-shaped fields.
- Provider capability metadata can drift. Mitigation: live probe tests and feature evidence when adding/updating a provider.
- A stricter gate may make some workflows feel slower. Mitigation: per-tool allowlist policy for read-only tools, but keep side effects gated.

## 10. Success Criteria

- A MiMo response containing textual `<function=ask_human>` produces a real pending `ask_human` UI request.
- A MiMo response containing `ask_human` plus `manage_targets add` does not execute add until the user actually responds.
- Logs and transcripts show raw preview, recovered intent, gate decision, approval request, and final execution result.
- Native OpenAI/Anthropic tool calls continue to pass existing tests unchanged.
- `feature_list.json` evidence includes targeted backend tests, frontend Details tests, and manual MiMo E2E notes.
