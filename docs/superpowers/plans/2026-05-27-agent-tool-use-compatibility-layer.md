# Agent Tool-Use Compatibility Layer 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 给 Golish 增加一层可测试的 agent tool-use compatibility layer，让不同模型的原生/文本工具调用都先归一化、再过安全 gate、再执行，并能被 UI 与日志清楚观察。
**架构：** Provider output 先进入 ToolIntent normalizer，产生带来源和置信度的 `ToolIntent`；再进入 deterministic gate，区分 allow / require approval / require ask_human / reject；最后才进入现有 tool dispatch。MiMo 等不稳定模型只允许受控 textual recovery，不能用自然语言绕过用户确认。
**技术栈：** Rust 2021, `golish-agent-runtime`, `golish-agent-kit`, `golish-models`, `golish-events`, React 19, TypeScript 6, Vitest, cargo nextest.

## 文件结构

- `backend/crates/golish-models/src/tool_use_profile.rs`：新增模型/提供商工具调用能力画像类型。
- `backend/crates/golish-models/src/capabilities.rs`：把 `ToolUseProfile` 接入 `ModelCapabilities`。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_intent.rs`：新增 `ToolIntent`、来源、normalizer 输出类型。
- `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/textual_tool_calls.rs`：从临时 parser 升级成 normalizer 输入之一。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_gate.rs`：新增 deterministic gate。
- `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs`：在 dispatch 前调用 gate，并把 rejected/approval required 写回 chat history。
- `backend/crates/golish-events/src/events.rs`：新增或扩展 tool-intent/gate observation event payload。
- `backend/crates/golish-events/src/event_coordinator/coordinator.rs`：输出结构化可观测日志。
- `frontend/components/AIChatPanel/ToolCallSummary.tsx`：展示 "Model wanted / Golish allowed / executed / waiting"。
- `frontend/components/AIChatPanel/useChatAiEvents.ts` 和 `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`：消费新增 observation events。

## Task 1: 建立 ToolUseProfile 类型

**文件：**

- `backend/crates/golish-models/src/tool_use_profile.rs`
- `backend/crates/golish-models/src/lib.rs`
- `backend/crates/golish-models/src/capabilities.rs`

**步骤：**

1. 新增 `tool_use_profile.rs`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallMode {
    NativeStrict,
    NativeBestEffort,
    TextualXmlFallback,
    TextualJsonFallback,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallReliability {
    Reliable,
    NeedsAdapter,
    ChatOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolUseProfile {
    pub mode: ToolCallMode,
    pub reliability: ToolCallReliability,
    pub supports_required_tool_choice: bool,
    pub supports_parallel_tool_calls: bool,
    pub max_tool_calls_per_turn: usize,
    pub requires_tool_result_balance: bool,
}

impl ToolUseProfile {
    pub const fn native_reliable() -> Self {
        Self {
            mode: ToolCallMode::NativeStrict,
            reliability: ToolCallReliability::Reliable,
            supports_required_tool_choice: true,
            supports_parallel_tool_calls: true,
            max_tool_calls_per_turn: 8,
            requires_tool_result_balance: true,
        }
    }

    pub const fn needs_textual_xml_adapter() -> Self {
        Self {
            mode: ToolCallMode::TextualXmlFallback,
            reliability: ToolCallReliability::NeedsAdapter,
            supports_required_tool_choice: false,
            supports_parallel_tool_calls: false,
            max_tool_calls_per_turn: 1,
            requires_tool_result_balance: true,
        }
    }

    pub const fn disabled() -> Self {
        Self {
            mode: ToolCallMode::Disabled,
            reliability: ToolCallReliability::ChatOnly,
            supports_required_tool_choice: false,
            supports_parallel_tool_calls: false,
            max_tool_calls_per_turn: 0,
            requires_tool_result_balance: false,
        }
    }
}
```

2. Export from `lib.rs`:

```rust
mod tool_use_profile;
pub use tool_use_profile::*;
```

3. Add field to `ModelCapabilities`:

```rust
pub tool_use_profile: ToolUseProfile,
```

4. Set conservative defaults:

```rust
tool_use_profile: ToolUseProfile::disabled(),
```

5. Set OpenAI/Anthropic defaults to `native_reliable()`, and Xiaomi defaults to `needs_textual_xml_adapter()` until MiMo E2E proves otherwise.

**验证：**

```bash
cd backend && cargo fmt --package golish-models --check
cd backend && CARGO_TARGET_DIR=/tmp/golish-models-target cargo test -p golish-models tool_use_profile --lib
```

预期：fmt exit 0；新增/现有能力测试通过。

**提交：** `feat(models): add tool use profile metadata`

## Task 2: 把 textual parser 升级为 ToolIntent normalizer

**文件：**

- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_intent.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/mod.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/textual_tool_calls.rs`

**步骤：**

1. 新增 `tool_intent.rs`：

```rust
use rig::message::ToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolIntentSource {
    NativeToolCall,
    TextualXml,
    TextualJson,
    Recovered,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolIntent {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
    pub source: ToolIntentSource,
    pub confidence: f32,
    pub raw_span: Option<String>,
}

impl ToolIntent {
    pub fn from_native(call: ToolCall) -> Self {
        Self {
            id: call.id,
            name: call.function.name,
            args: call.function.arguments,
            source: ToolIntentSource::NativeToolCall,
            confidence: 1.0,
            raw_span: None,
        }
    }

    pub fn into_tool_call(self) -> ToolCall {
        ToolCall {
            id: self.id,
            function: rig::message::ToolFunction {
                name: self.name,
                arguments: self.args,
            },
        }
    }
}
```

2. Export module:

```rust
pub(crate) mod tool_intent;
```

3. Change textual parser to return `Vec<ToolIntent>` instead of direct `ToolCall` for recovered calls.

4. Keep existing tests but assert source/confidence:

```rust
assert_eq!(intent.source, ToolIntentSource::TextualXml);
assert!(intent.confidence < 1.0);
```

**验证：**

```bash
cd backend && cargo fmt --package golish-agent-runtime --check
cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime textual_tool_calls --lib
```

预期：现有 textual parser tests 继续通过，并新增 source/confidence 断言。

**提交：** `feat(runtime): normalize recovered tool calls as intents`

## Task 3: 加 ToolGate deterministic policy

**文件：**

- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_gate.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/mod.rs`

**步骤：**

1. 新增 `tool_gate.rs`：

```rust
use crate::agentic_loop::tool_intent::{ToolIntent, ToolIntentSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolGateDecision {
    Allow,
    RequireApproval { reason: String },
    RequireHumanAnswer { question: String },
    Reject { reason: String },
}

pub fn decide_tool_intent(intent: &ToolIntent, target_registered: bool) -> ToolGateDecision {
    if intent.name == "ask_human" {
        let question = intent
            .args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("Please confirm before continuing.")
            .to_string();
        return ToolGateDecision::RequireHumanAnswer { question };
    }

    if intent.name == "manage_targets" {
        let action = intent.args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if action == "add" && intent.source != ToolIntentSource::NativeToolCall {
            return ToolGateDecision::RequireApproval {
                reason: "Recovered textual target-add intent requires explicit user approval".to_string(),
            };
        }
    }

    if intent.name == "run_pipeline" && !target_registered {
        return ToolGateDecision::Reject {
            reason: "Cannot run pipeline before target is registered and in scope".to_string(),
        };
    }

    ToolGateDecision::Allow
}
```

2. Add tests:

```rust
#[test]
fn ask_human_is_hard_barrier() {
    let intent = ToolIntent {
        id: "t1".into(),
        name: "ask_human".into(),
        args: serde_json::json!({"question": "Add example.com?"}),
        source: ToolIntentSource::TextualXml,
        confidence: 0.7,
        raw_span: None,
    };

    assert!(matches!(
        decide_tool_intent(&intent, false),
        ToolGateDecision::RequireHumanAnswer { .. }
    ));
}
```

**验证：**

```bash
cd backend && cargo fmt --package golish-agent-runtime --check
cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime tool_gate --lib
```

预期：`ask_human`、recovered target add、unregistered pipeline 三类 gate tests 全绿。

**提交：** `feat(runtime): add deterministic tool gate`

## Task 4: 在 dispatch 前接入 gate

**文件：**

- `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`

**步骤：**

1. Keep `stream_processor` producing `ToolIntent` internally, then convert only allowed intents to `ToolCall`.

2. In `tool_dispatch.rs`, gate before existing permission filtering:

```rust
let mut permitted = Vec::new();
let mut rejected = Vec::new();

for intent in tool_intents {
    match decide_tool_intent(&intent, target_registered) {
        ToolGateDecision::Allow => permitted.push(intent.into_tool_call()),
        decision => rejected.push((intent, decision)),
    }
}
```

3. For `RequireHumanAnswer`, emit the existing `ask_human` mechanism instead of rendering prose.

4. For `Reject`, push a tool-result style observation back into chat history so the next LLM iteration sees a deterministic reason.

**验证：**

```bash
cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime textual_tool_calls tool_gate --lib
cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime test_hitl_approval_request_emitted --lib
```

预期：textual `ask_human` causes pending ask-human/approval path; recovered `manage_targets add` is not executed directly.

**提交：** `feat(runtime): gate normalized tool intents before dispatch`

## Task 5: 增加观察事件与日志

**文件：**

- `backend/crates/golish-events/src/events.rs`
- `backend/crates/golish-events/src/event_coordinator/coordinator.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs`

**步骤：**

1. Add event payload:

```rust
pub struct ToolIntentObservation {
    pub tool_name: String,
    pub source: String,
    pub decision: String,
    pub reason: Option<String>,
    pub raw_preview: Option<String>,
}
```

2. Emit one observation per recovered/rejected/gated intent.

3. Log with stable prefix:

```rust
tracing::info!(
    target: "agent-observe",
    session_id = %session_id,
    tool_name = %obs.tool_name,
    source = %obs.source,
    decision = %obs.decision,
    reason = ?obs.reason,
    "tool intent gate decision"
);
```

4. Keep raw preview truncated to 500 chars and never log full secret-like values.

**验证：**

```bash
cd backend && cargo fmt --package golish-events --package golish-agent-runtime --check
cd backend && CARGO_TARGET_DIR=/tmp/golish-events-target cargo test -p golish-events transcript --lib
```

预期：event serde tests pass；transcript persistence still includes known event shapes.

**提交：** `feat(events): observe tool intent gate decisions`

## Task 6: 前端 Details 展示 intent/gate 状态

**文件：**

- `frontend/components/AIChatPanel/ToolCallSummary.tsx`
- `frontend/components/AIChatPanel/useChatAiEvents.ts`
- `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`
- `frontend/components/AIChatPanel/MessageBlock.tsx`

**步骤：**

1. Extend timeline state with observation entries:

```ts
type ToolIntentObservation = {
  toolName: string;
  source: "native_tool_call" | "textual_xml" | "textual_json" | "recovered";
  decision: "allow" | "require_approval" | "require_human_answer" | "reject";
  reason?: string;
  rawPreview?: string;
};
```

2. Render four labels in details:

```tsx
<div className="grid gap-2 text-xs">
  <div>Model wanted: {toolName}</div>
  <div>Source: {sourceLabel}</div>
  <div>Golish decision: {decisionLabel}</div>
  {reason && <div>Reason: {reason}</div>}
</div>
```

3. Keep raw `<tool_call>` markup stripped from user-facing prose, but expose recovered preview in Details only.

**验证：**

```bash
pnpm exec tsc --noEmit
pnpm exec biome check frontend/components/AIChatPanel/ToolCallSummary.tsx frontend/components/AIChatPanel/MessageBlock.tsx frontend/components/AIChatPanel/useChatAiEvents.ts frontend/components/AIChatPanel/hooks/useAiChatEvents.ts
pnpm vitest run frontend/components/AIChatPanel
```

预期：typecheck exit 0；biome no fixes；AIChatPanel targeted tests pass or only documented baseline failures remain.

**提交：** `feat(chat): show tool intent gate decisions`

## Task 7: MiMo E2E replay test

**文件：**

- `backend/crates/golish-agent-runtime/src/agentic_loop/tests/mimo_textual_tool_call_tests.rs`
- `agent-progress.md`
- `feature_list.json`

**步骤：**

1. Add fixture text copied from the transcript shape:

```rust
const MIMO_TEXTUAL_ASK_AND_ADD: &str = r#"
example.com 不在当前目标列表中。
<tool_call>
<function=ask_human>
<parameter=question>是否添加 example.com 到目标列表?</parameter>
</function>
<function=manage_targets>
<parameter=action>add</parameter>
<parameter=targets>[{"value":"example.com"}]</parameter>
</function>
</tool_call>
"#;
```

2. Assert the first gated outcome is `RequireHumanAnswer`.

3. Assert `manage_targets add` is not dispatched in the same iteration.

4. Update progress and feature evidence with command output snippets.

**验证：**

```bash
cd backend && CARGO_TARGET_DIR=/tmp/golish-agent-runtime-target cargo test -p golish-agent-runtime mimo_textual_tool_call --lib
python3 -m json.tool feature_list.json >/dev/null
git diff --check
```

预期：MiMo replay test passes；feature JSON valid；diff has no whitespace errors.

**提交：** `test(runtime): replay mimo textual tool call gating`

## Task 8: Final verification

**文件：**

- `agent-progress.md`
- `feature_list.json`

**步骤：**

1. Run targeted checks from Tasks 1-7.
2. Run `just check-fe`.
3. Run `just test-fe`.
4. Run `cd backend && cargo nextest run -p golish-agent-runtime -p golish-events -p golish-models --status-level fail`.
5. Run `just precommit` only when baseline failures are fixed or explicitly recorded as pre-existing with fresh evidence.
6. Update `feature_list.json` status:

```json
"status": "passing"
```

Only set passing if the verification evidence is fresh and complete.

**验证：**

```bash
python3 -m json.tool feature_list.json >/dev/null
git diff --check
```

预期：JSON valid；no whitespace errors.

**提交：** `docs(progress): record tool compatibility verification`

## Self-Check

- Specification coverage: provider metadata, normalizer, safety gate, approval barrier, observability, UI details, MiMo replay, and final evidence are each mapped to a task.
- Placeholder scan: no "待定", no "TODO", no vague "add proper validation" steps.
- Type consistency: `ToolUseProfile`, `ToolIntent`, `ToolIntentSource`, and `ToolGateDecision` names are consistent across tasks.
