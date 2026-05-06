# Execution Mode Policy 重构 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把当前散在 `tool_list.rs` 里的 chat / task 模式 if/else 抽出成一组 `ExecutionModePolicy` strategy，使得未来加 plan / debug / scan-only 等模式时无需修改核心文件，并修复 chat 模式下 `pentest_bridge` 工具（`js_collect` 等 8 件）被前缀过滤误杀的 bug。

**架构：** 新建 `golish-agent-runtime/src/execution_mode/` 模块，定义 `ExecutionModePolicy` trait + `ToolSelection` 数据结构 + `ExecutionModeRegistry`。`tool_list.rs` 改为 100% 委托给 policy；system prompt 模板化通过 `{{tool_table}}` 占位符渲染，从同一份 selection 派生，物理上锁死「prompt 说能用 == LLM 实际拿到」。

**技术栈：** Rust（async-trait + tokio）；前端 React 18 + zod；后端 Tauri IPC。

---

## 范围检查

本计划仅涉及「执行模式 → 工具暴露」一条数据流，不动 sub-agent 注册、不动 `pentest_bridge` 工具实现、不动 LLM 客户端。Sub-agent 的 `allowed_tools` 列表 (`registry.rs`) 单独是另一个抽象层，留给后续计划。

---

## 文件结构

### 新建文件

```
backend/crates/golish-agent-runtime/src/execution_mode/
  mod.rs                  // 公共导出
  policy.rs               // ExecutionModePolicy trait + ToolSelection 数据结构
  context.rs              // PolicyContext
  registry.rs             // ExecutionModeRegistry
  selection_apply.rs      // apply_tool_selection() 纯函数
  prompt_render.rs        // render_tool_table_for_prompt()
  modes/
    mod.rs
    chat.rs               // ChatModePolicy
    task.rs               // TaskModePolicy
  templates/
    chat.tera             // chat 模式 system prompt 模板
    task.tera             // task 模式 system prompt 模板

frontend/lib/types/executionMode.ts   // zod schema + TS 类型
```

### 修改文件

| 文件 | 改动职责 |
|---|---|
| `backend/crates/golish-agent-runtime/src/lib.rs` | 暴露 `pub mod execution_mode;` |
| `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs` | 145 行 → 30 行，改用 policy |
| `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs` | `AgenticLoopContext` 加 `execution_mode_registry: Arc<ExecutionModeRegistry>` 字段 |
| `backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs` | 在 `prepare_execution_context` 里把 registry 注入到 loop context |
| `backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs` | `AgentBridge` 持有一份 `Arc<ExecutionModeRegistry>` |
| `backend/crates/golish-prompts/src/system_prompt/chat.rs` | 改为读 Tera 模板渲染（PR3） |
| `backend/crates/golish-prompts/src/system_prompt/task.rs` | 同上（PR3） |
| `backend/crates/golish/src/ai/commands/mode.rs` | 加 `list_execution_modes` Tauri command（PR4） |
| `frontend/components/AIChatPanel/ExecutionModePicker.tsx` | 改为消费动态列表（PR4） |
| `frontend/components/AIChatPanel/hooks/useChatModes.ts` | 类型从 `"chat" \| "task"` 改为 `string`（PR4） |
| `frontend/lib/ai.ts` | 加 `listExecutionModes()` IPC 客户端（PR4） |

---

## PR 阶段拆分概览

| PR | 标题 | 改动行数 (估) | 上线后效果 |
|---|---|---|---|
| PR1 | 新增 ExecutionModePolicy 抽象，零接入 | +500 / -0 | 仅加新代码，运行时零变化 |
| PR2 | tool_list.rs 改用 policy，**修复 chat 下 js_collect 不可见 bug** | +60 / -120 | 用户在 chat 模式能调用 `js_collect / manage_targets / run_pipeline / ...` |
| PR3 | system prompt 模板化，锁死 prompt-vs-tools 一致性 | +200 / -150 | prompt 说能用什么就一定能用什么 |
| PR4 | 前端动态拉取模式列表 | +120 / -40 | 后端注册新模式 → 前端自动出现 |

---

## PR1 · 新增 ExecutionModePolicy 抽象（零接入）

> 本 PR 仅新增代码、不改 `tool_list.rs`，可独立合入。

### 任务 1.1：定义 ToolSelection 数据结构与 trait

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/policy.rs`（新建）

**步骤：** 创建文件，写入下列内容：

```rust
//! ExecutionModePolicy: per-execution-mode tool exposure & prompt template
//! strategy. Each execution mode (chat / task / future plan / debug ...)
//! has one Policy declaring **what tools are visible to the LLM** and
//! **which prompt template to render**. The runtime calls
//! `build_tool_list` which delegates entirely to the active Policy.

use async_trait::async_trait;

use super::context::PolicyContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeLabel {
    pub display_name: &'static str,
    pub icon: &'static str,
    pub badge_color: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolSelection {
    pub static_groups: StaticGroupSelection,
    pub bridge_tools: BridgeToolSelection,
    pub runtime_tools: RuntimeToolSelection,
    pub agent_tools: AgentToolSelection,
    pub include_run_command: bool,
    pub include_ask_human: bool,
    pub deny_overrides: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticGroupSelection {
    pub file_ops: bool,
    pub core: bool,
    pub memory: bool,
    pub knowledge_base: bool,
    pub security_analysis: bool,
    pub graph: bool,
    pub sploitus: bool,
}

impl StaticGroupSelection {
    pub const fn all_enabled() -> Self {
        Self {
            file_ops: true, core: true, memory: true, knowledge_base: true,
            security_analysis: true, graph: true, sploitus: true,
        }
    }
    pub const fn none() -> Self { Self::default() }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeToolSelection {
    pub manage_targets: bool,
    pub record_finding: bool,
    pub vault: bool,
    pub run_pipeline: bool,
    pub flow_compose: bool,
    pub js_collect: bool,
    pub js_extract_apis: bool,
    pub auth_probe: bool,
}

impl BridgeToolSelection {
    pub const fn all_enabled() -> Self {
        Self {
            manage_targets: true, record_finding: true, vault: true,
            run_pipeline: true, flow_compose: true, js_collect: true,
            js_extract_apis: true, auth_probe: true,
        }
    }
    pub const fn none() -> Self { Self::default() }

    pub fn enabled_tool_names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.manage_targets { out.push("manage_targets"); }
        if self.record_finding { out.push("record_finding"); }
        if self.vault { out.push("vault"); }
        if self.run_pipeline { out.push("run_pipeline"); }
        if self.flow_compose { out.push("flow_compose"); }
        if self.js_collect { out.push("js_collect"); }
        if self.js_extract_apis { out.push("js_extract_apis"); }
        if self.auth_probe { out.push("auth_probe"); }
        out
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeToolSelection {
    pub pentest_runtime: bool,
    pub tavily: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentToolSelection {
    pub include_dispatch_tools: bool,
    pub allow_planner: bool,
    pub allow_refiner: bool,
    pub allow_reflector: bool,
}

impl AgentToolSelection {
    pub const fn none() -> Self { Self::default() }
}

#[async_trait]
pub trait ExecutionModePolicy: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn label(&self) -> ModeLabel;
    fn description(&self) -> &'static str;
    fn allows_sub_agents(&self) -> bool { false }

    async fn primary_tools(&self, ctx: &PolicyContext<'_>) -> ToolSelection;
    async fn subtask_tools(&self, ctx: &PolicyContext<'_>) -> ToolSelection {
        self.primary_tools(ctx).await
    }
}
```

**验证：**
```bash
cargo check -p golish-agent-runtime 2>&1 | tail -20
```
**预期输出：** 含 `Finished`，可能有 `cannot find module context` 错误（下一任务补）。

**提交：** 不在此任务 commit，与 1.2/1.3 合一同提交。

---

### 任务 1.2：定义 PolicyContext

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/context.rs`（新建）

**步骤：** 创建文件，写入：

```rust
//! Read-only context passed to ExecutionModePolicy::primary_tools /
//! subtask_tools. Lets policies inspect agent_mode, workspace, depth etc.
//! without dragging the whole AgenticLoopContext.

use std::path::Path;

use golish_core::AgentMode;

pub struct PolicyContext<'a> {
    pub agent_mode: AgentMode,
    pub workspace: &'a Path,
    pub use_agents_pref: bool,
    pub mcp_tool_count: usize,
    pub depth: usize,
}

impl<'a> PolicyContext<'a> {
    pub fn new(workspace: &'a Path, agent_mode: AgentMode) -> Self {
        Self {
            agent_mode,
            workspace,
            use_agents_pref: false,
            mcp_tool_count: 0,
            depth: 0,
        }
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    pub fn with_use_agents(mut self, value: bool) -> Self {
        self.use_agents_pref = value;
        self
    }

    pub fn with_mcp_tool_count(mut self, count: usize) -> Self {
        self.mcp_tool_count = count;
        self
    }
}
```

**验证：**
```bash
cargo check -p golish-agent-runtime 2>&1 | tail -10
```
**预期输出：** `Finished`，仍可能有 `module not found` 因为还没有 mod.rs。

---

### 任务 1.3：定义 ExecutionModeRegistry

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/registry.rs`（新建）

**步骤：** 创建文件，写入：

```rust
//! Registry of registered ExecutionModePolicy instances. Lookup by id.

use std::collections::HashMap;
use std::sync::Arc;

use super::policy::ExecutionModePolicy;

pub struct ExecutionModeRegistry {
    policies: HashMap<&'static str, Arc<dyn ExecutionModePolicy>>,
}

impl ExecutionModeRegistry {
    pub fn empty() -> Self {
        Self { policies: HashMap::new() }
    }

    pub fn register<P: ExecutionModePolicy>(&mut self, policy: P) {
        self.policies.insert(policy.id(), Arc::new(policy));
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn ExecutionModePolicy>> {
        self.policies.get(id).cloned()
    }

    pub fn list_ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.policies.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn list_all(&self) -> Vec<Arc<dyn ExecutionModePolicy>> {
        let mut all: Vec<_> = self.policies.values().cloned().collect();
        all.sort_by_key(|p| p.id());
        all
    }
}

impl Default for ExecutionModeRegistry {
    fn default() -> Self {
        let mut r = Self::empty();
        r.register(super::modes::chat::ChatModePolicy);
        r.register(super::modes::task::TaskModePolicy);
        r
    }
}
```

**验证：**
```bash
cargo check -p golish-agent-runtime 2>&1 | tail -5
```
**预期输出：** 仍有 module not found，下一任务补 mod.rs + modes/。

---

### 任务 1.4：实现 ChatModePolicy

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/modes/chat.rs`（新建）

**步骤：** 创建文件，写入：

```rust
//! ChatModePolicy — single-agent conversational mode with the full toolbox.
//! This is the policy that finally gives the chat-mode LLM access to
//! `js_collect / manage_targets / run_pipeline / ...` (the bug we are fixing).

use async_trait::async_trait;

use crate::execution_mode::context::PolicyContext;
use crate::execution_mode::policy::{
    AgentToolSelection, BridgeToolSelection, ExecutionModePolicy, ModeLabel,
    RuntimeToolSelection, StaticGroupSelection, ToolSelection,
};

pub struct ChatModePolicy;

#[async_trait]
impl ExecutionModePolicy for ChatModePolicy {
    fn id(&self) -> &'static str { "chat" }

    fn label(&self) -> ModeLabel {
        ModeLabel { display_name: "Chat", icon: "MessageSquare", badge_color: "muted" }
    }

    fn description(&self) -> &'static str {
        "Conversational single-agent mode with the full toolbox."
    }

    async fn primary_tools(&self, _ctx: &PolicyContext<'_>) -> ToolSelection {
        ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            bridge_tools: BridgeToolSelection::all_enabled(),
            runtime_tools: RuntimeToolSelection { pentest_runtime: true, tavily: true },
            agent_tools: AgentToolSelection::none(),
            include_run_command: true,
            include_ask_human: true,
            deny_overrides: vec![],
        }
    }
}
```

---

### 任务 1.5：实现 TaskModePolicy

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs`（新建）

**步骤：** 创建文件，写入：

```rust
//! TaskModePolicy — orchestration mode. Primary agent only sees sub-agent
//! dispatch tools; sub-agents see the full toolbox minus update_plan.

use async_trait::async_trait;

use crate::execution_mode::context::PolicyContext;
use crate::execution_mode::policy::{
    AgentToolSelection, BridgeToolSelection, ExecutionModePolicy, ModeLabel,
    RuntimeToolSelection, StaticGroupSelection, ToolSelection,
};

pub struct TaskModePolicy;

#[async_trait]
impl ExecutionModePolicy for TaskModePolicy {
    fn id(&self) -> &'static str { "task" }

    fn label(&self) -> ModeLabel {
        ModeLabel { display_name: "Task", icon: "Zap", badge_color: "magenta" }
    }

    fn description(&self) -> &'static str {
        "Auto: plan -> execute -> refine -> report (multi-agent orchestration)."
    }

    fn allows_sub_agents(&self) -> bool { true }

    async fn primary_tools(&self, _ctx: &PolicyContext<'_>) -> ToolSelection {
        ToolSelection {
            static_groups: StaticGroupSelection::none(),
            bridge_tools: BridgeToolSelection::none(),
            runtime_tools: RuntimeToolSelection::default(),
            agent_tools: AgentToolSelection {
                include_dispatch_tools: true,
                allow_planner: true,
                allow_refiner: false,    // pipeline-only, not exposed
                allow_reflector: false,  // pipeline-only, not exposed
            },
            include_run_command: false,
            include_ask_human: true,
            deny_overrides: vec![],
        }
    }

    async fn subtask_tools(&self, _ctx: &PolicyContext<'_>) -> ToolSelection {
        ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            bridge_tools: BridgeToolSelection::all_enabled(),
            runtime_tools: RuntimeToolSelection { pentest_runtime: true, tavily: true },
            agent_tools: AgentToolSelection::none(),
            include_run_command: true,
            include_ask_human: false,
            deny_overrides: vec!["update_plan".into()],
        }
    }
}
```

---

### 任务 1.6：modes/mod.rs 与 execution_mode/mod.rs 模块导出

**文件 1：** `backend/crates/golish-agent-runtime/src/execution_mode/modes/mod.rs`（新建）

```rust
pub mod chat;
pub mod task;
```

**文件 2：** `backend/crates/golish-agent-runtime/src/execution_mode/mod.rs`（新建）

```rust
//! ExecutionModePolicy strategy + registry. See policy.rs for the trait
//! and modes/{chat,task}.rs for built-in implementations.

pub mod context;
pub mod modes;
pub mod policy;
pub mod registry;

pub use context::PolicyContext;
pub use policy::{
    AgentToolSelection, BridgeToolSelection, ExecutionModePolicy, ModeLabel,
    RuntimeToolSelection, StaticGroupSelection, ToolSelection,
};
pub use registry::ExecutionModeRegistry;
```

**文件 3：** `backend/crates/golish-agent-runtime/src/lib.rs`（修改）

在文件顶部其它 `pub mod` 行旁加：

```rust
pub mod execution_mode;
```

**验证：**
```bash
cargo check -p golish-agent-runtime 2>&1 | tail -20
```
**预期输出：** `Finished` 不含 `error`。`async_trait` 缺失就在 `Cargo.toml` 的 `[dependencies]` 加 `async-trait = "0.1"` 行。

---

### 任务 1.7：单测 — chat policy 暴露 pentest_bridge 全套

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/modes/chat.rs`（在 1.4 文件末尾追加）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mock_ctx() -> PolicyContext<'static> {
        let ws: &'static Path = Path::new("/tmp");
        PolicyContext::new(ws, golish_core::AgentMode::Default)
    }

    #[tokio::test]
    async fn chat_primary_includes_js_collect() {
        let p = ChatModePolicy;
        let s = p.primary_tools(&mock_ctx()).await;
        assert!(s.bridge_tools.js_collect, "chat must expose js_collect (bug fix)");
        assert!(s.bridge_tools.manage_targets);
        assert!(s.bridge_tools.run_pipeline);
        assert!(s.bridge_tools.auth_probe);
    }

    #[tokio::test]
    async fn chat_primary_full_static_groups() {
        let s = ChatModePolicy.primary_tools(&mock_ctx()).await;
        assert!(s.static_groups.file_ops);
        assert!(s.static_groups.security_analysis);
    }

    #[tokio::test]
    async fn chat_does_not_dispatch_sub_agents() {
        let s = ChatModePolicy.primary_tools(&mock_ctx()).await;
        assert!(!s.agent_tools.include_dispatch_tools);
        assert!(!ChatModePolicy.allows_sub_agents());
    }
}
```

**验证：**
```bash
cargo test -p golish-agent-runtime execution_mode::modes::chat 2>&1 | tail -10
```
**预期输出：** `test result: ok. 3 passed`.

---

### 任务 1.8：单测 — task policy 主从分离

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs`（在 1.5 文件末尾追加）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mock_ctx() -> PolicyContext<'static> {
        PolicyContext::new(Path::new("/tmp"), golish_core::AgentMode::Default)
    }

    #[tokio::test]
    async fn task_primary_only_dispatches() {
        let s = TaskModePolicy.primary_tools(&mock_ctx()).await;
        assert!(!s.bridge_tools.js_collect);
        assert!(!s.static_groups.file_ops);
        assert!(s.agent_tools.include_dispatch_tools);
        assert!(s.include_ask_human);
    }

    #[tokio::test]
    async fn task_subtask_full_minus_update_plan() {
        let s = TaskModePolicy.subtask_tools(&mock_ctx()).await;
        assert!(s.bridge_tools.js_collect);
        assert!(s.static_groups.file_ops);
        assert!(s.deny_overrides.iter().any(|n| n == "update_plan"));
        assert!(!s.include_ask_human);
    }
}
```

**验证：**
```bash
cargo test -p golish-agent-runtime execution_mode::modes::task 2>&1 | tail -10
```
**预期输出：** `test result: ok. 2 passed`.

---

### 任务 1.9：Registry 单测 + 整体提交

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/registry.rs`（在 1.3 文件末尾追加）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_chat_and_task() {
        let r = ExecutionModeRegistry::default();
        assert!(r.get("chat").is_some());
        assert!(r.get("task").is_some());
        assert_eq!(r.list_ids(), vec!["chat", "task"]);
    }

    #[test]
    fn unknown_mode_returns_none() {
        let r = ExecutionModeRegistry::default();
        assert!(r.get("plan").is_none());
    }
}
```

**验证整体：**
```bash
cargo test -p golish-agent-runtime execution_mode 2>&1 | tail -10
```
**预期输出：** `test result: ok. 7 passed` 或更多。

**提交：**
```bash
git add backend/crates/golish-agent-runtime/src/execution_mode/ \
        backend/crates/golish-agent-runtime/src/lib.rs \
        backend/crates/golish-agent-runtime/Cargo.toml
git commit -m "[exec-mode] PR1: introduce ExecutionModePolicy strategy + Registry

- Add ExecutionModePolicy trait + ToolSelection types in
  golish-agent-runtime/src/execution_mode/.
- Provide built-in ChatModePolicy and TaskModePolicy that mirror
  the current behaviour exposed by tool_list.rs (chat=full toolbox
  including pentest_bridge, task=primary orchestration + subtask
  full minus update_plan).
- Default ExecutionModeRegistry registers chat + task.
- Zero runtime impact: tool_list.rs unchanged in this PR. The new
  abstraction is wired up in PR2.
- 7 unit tests covering selection shapes per mode."
```

---

## PR2 · tool_list.rs 接入 Policy（修复 chat 下 `js_collect` 不可见 bug）

> 本 PR 把运行时的 `build_tool_list` 改用 PR1 的 Policy。**合入即修复用户付费路径的 chat 模式工具暴露 bug**。

### 任务 2.1：定义 selection_apply 纯函数（含 pentest_bridge 工具集中点）

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/selection_apply.rs`（新建）

```rust
//! Apply a ToolSelection against the live AgenticLoopContext to produce
//! a Vec<rig::completion::ToolDefinition>. Pure function: no I/O, no
//! mutation of the context, fully unit-testable.

use std::collections::HashSet;

use golish_agent_kit::tool_definitions::{
    get_all_tool_definitions_with_config, get_ask_human_tool_definition,
    get_run_command_tool_definition, get_sub_agent_tool_definitions, sanitize_schema,
};
use golish_sub_agents::{SubAgentContext, MAX_AGENT_DEPTH};

use crate::agentic_loop::context::AgenticLoopContext;
use crate::execution_mode::policy::{BridgeToolSelection, ToolSelection};

pub async fn apply_tool_selection(
    selection: ToolSelection,
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
) -> Vec<rig::completion::ToolDefinition> {
    let mut tools: Vec<rig::completion::ToolDefinition> = Vec::new();

    // 1. Static tool groups via existing ToolConfig + ToolPreset machinery.
    //    The Policy decides which groups, ToolConfig decides which tool
    //    names within those groups.
    if any_static_group_enabled(&selection) {
        tools.extend(get_all_tool_definitions_with_config(ctx.tool_config));
    }

    // 2. run_command (replaces run_pty_cmd).
    if selection.include_run_command {
        tools.push(get_run_command_tool_definition());
    }

    // 3. ask_human only when the policy says so AND we are at depth==0.
    if selection.include_ask_human && sub_agent_context.depth == 0 {
        tools.push(get_ask_human_tool_definition());
    }

    // 4. MCP / additional pre-built tool definitions.
    tools.extend(ctx.additional_tool_definitions.iter().cloned());

    // 5. Dynamic registry tools — bridge / pentest_runtime / tavily.
    let registry = ctx.tool_registry.read().await;
    let registry_tools = registry.get_tool_definitions();
    drop(registry);

    let bridge_allowed: HashSet<&'static str> =
        selection.bridge_tools.enabled_tool_names().into_iter().collect();
    let existing: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();

    for tool in registry_tools {
        if existing.contains(&tool.name) {
            continue;
        }
        let include = if bridge_allowed.contains(tool.name.as_str()) {
            true
        } else if tool.name.starts_with("pentest_") {
            selection.runtime_tools.pentest_runtime
        } else if tool.name.starts_with("tavily_") {
            selection.runtime_tools.tavily && ctx.tool_config.is_tool_enabled(&tool.name)
        } else {
            false
        };

        if include {
            tools.push(rig::completion::ToolDefinition {
                name: tool.name,
                description: tool.description,
                parameters: sanitize_schema(tool.parameters),
            });
        }
    }

    // 6. sub-agent dispatch tools.
    if selection.agent_tools.include_dispatch_tools
        && sub_agent_context.depth + 1 < MAX_AGENT_DEPTH
    {
        let registry = ctx.sub_agent_registry.read().await;
        let mut sub_tools = get_sub_agent_tool_definitions(&registry).await;
        if !selection.agent_tools.allow_planner {
            sub_tools.retain(|t| t.name != "sub_agent_planner");
        }
        if !selection.agent_tools.allow_refiner {
            sub_tools.retain(|t| t.name != "sub_agent_refiner");
        }
        if !selection.agent_tools.allow_reflector {
            sub_tools.retain(|t| t.name != "sub_agent_reflector");
        }
        // Orchestrator is always pipeline-only.
        sub_tools.retain(|t| t.name != "sub_agent_orchestrator");
        tools.extend(sub_tools);
    }

    // 7. Apply deny_overrides (e.g. update_plan in subtask mode).
    if !selection.deny_overrides.is_empty() {
        let denied: HashSet<&str> = selection.deny_overrides.iter().map(|s| s.as_str()).collect();
        tools.retain(|t| !denied.contains(t.name.as_str()));
    }

    tracing::debug!(
        "Available tools (policy-driven, depth={}): {:?}",
        sub_agent_context.depth,
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    tools
}

fn any_static_group_enabled(s: &ToolSelection) -> bool {
    let g = &s.static_groups;
    g.file_ops || g.core || g.memory || g.knowledge_base
        || g.security_analysis || g.graph || g.sploitus
}
```

**验证：**
```bash
cargo check -p golish-agent-runtime 2>&1 | tail -5
```
**预期输出：** `Finished` 不含 error。

---

### 任务 2.2：AgenticLoopContext 持有 registry 引用

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs`（修改）

**步骤：**

1. 在文件顶部 `use` 区加：
   ```rust
   use std::sync::Arc;
   use crate::execution_mode::ExecutionModeRegistry;
   ```

2. 在 `AgenticLoopContext` struct 末尾加字段：
   ```rust
   pub execution_mode_registry: Arc<ExecutionModeRegistry>,
   ```

3. 找到所有构造 `AgenticLoopContext` 的地方（通过 `cargo check` 报错来定位），把 `execution_mode_registry: ...` 字段加上。本 PR 仅 `agent_bridge::prepare::build_loop_context` 一处主调用方，eval 测试也需要补。

**验证：**
```bash
cargo check -p golish-agent-runtime 2>&1 | grep -E "(error|warning).*execution_mode_registry" | head -5
```
**预期输出：** 0 行（结构填齐后无 error/warning 提及该字段）。

---

### 任务 2.3：AgentBridge 持有 registry 并下发到 loop context

**文件 A：** `backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs`（修改）

**步骤：**

1. 在 `AgentBridge` struct 加字段：
   ```rust
   pub(crate) execution_mode_registry: std::sync::Arc<golish_agent_runtime::execution_mode::ExecutionModeRegistry>,
   ```

2. 在 `AgentBridge::new` / 构造器路径里默认注入：
   ```rust
   execution_mode_registry: std::sync::Arc::new(
       golish_agent_runtime::execution_mode::ExecutionModeRegistry::default()
   ),
   ```

   找到 `AgentBridge` 的所有构造点（`constructors/mod.rs` 是入口），逐一补上。

**文件 B：** `backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs`（修改）

把 `build_loop_context` 末尾的 `AgenticLoopContext { ... }` 字面量补上：

```rust
            execution_mode_registry: self.execution_mode_registry.clone(),
```

**验证：**
```bash
cargo check -p golish-agent-bridge 2>&1 | tail -10
```
**预期输出：** `Finished`，无 error。

---

### 任务 2.4：重写 tool_list.rs（核心改动 — bug fix 在此）

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`

**步骤：** 把 1-145 行整个内容替换为：

```rust
//! Build the per-turn tool list by delegating to the active
//! ExecutionModePolicy. Old hard-coded if/else branches for chat / task
//! were lifted into per-mode policies under `crate::execution_mode::modes`.
//!
//! The mapping (mode, depth) -> ToolSelection is fully owned by the
//! Policy. This module only:
//! 1. Looks up the policy from the registry by mode id.
//! 2. Builds the right PolicyContext.
//! 3. Calls primary_tools or subtask_tools.
//! 4. Hands the resulting Selection to apply_tool_selection.

use golish_sub_agents::SubAgentContext;

use super::context::AgenticLoopContext;
use crate::execution_mode::context::PolicyContext;
use crate::execution_mode::selection_apply::apply_tool_selection;

pub(crate) async fn build_tool_list(
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
) -> Vec<rig::completion::ToolDefinition> {
    let mode_id: &str = ctx.execution_mode.into();
    let policy = match ctx.execution_mode_registry.get(mode_id) {
        Some(p) => p,
        None => {
            tracing::error!(
                "[tool_list] Unknown execution mode '{}', falling back to chat",
                mode_id
            );
            ctx.execution_mode_registry
                .get("chat")
                .expect("default registry must contain chat")
        }
    };

    let workspace = ctx.workspace.read().await;
    let policy_ctx = PolicyContext::new(&workspace, golish_core::AgentMode::Default)
        .with_depth(sub_agent_context.depth)
        .with_use_agents(ctx.use_agents)
        .with_mcp_tool_count(ctx.additional_tool_definitions.len());
    drop(workspace);

    let selection = if sub_agent_context.depth == 0 {
        policy.primary_tools(&policy_ctx).await
    } else {
        policy.subtask_tools(&policy_ctx).await
    };

    apply_tool_selection(selection, ctx, sub_agent_context).await
}
```

**前置依赖：** 需要 `From<ExecutionMode> for &'static str`，加在 `golish-agent-kit/src/execution_mode.rs`：

```rust
impl From<ExecutionMode> for &'static str {
    fn from(mode: ExecutionMode) -> &'static str {
        match mode {
            ExecutionMode::Chat => "chat",
            ExecutionMode::Task => "task",
        }
    }
}
```

**验证：**
```bash
cargo check -p golish-agent-runtime 2>&1 | tail -5
```
**预期输出：** `Finished` 不含 error。

---

### 任务 2.5：集成测试 — chat 模式 build_tool_list 包含 js_collect

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`（在 2.4 文件末尾追加）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // 测试装置依赖 test_utils::context::mock_loop_ctx，对照现有
    // src/test_utils/context.rs 的现有 mock_chat_ctx / mock_task_ctx
    // helper —— 若不存在需新增。

    use crate::test_utils::context::{mock_chat_ctx, mock_task_ctx};

    #[tokio::test]
    async fn chat_mode_exposes_js_collect() {
        let ctx = mock_chat_ctx().await;
        let tools = build_tool_list(&ctx, &SubAgentContext::depth(0)).await;
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"js_collect"),
            "chat mode must expose js_collect; actual: {:?}",
            names
        );
        assert!(names.contains(&"manage_targets"));
        assert!(names.contains(&"run_pipeline"));
    }

    #[tokio::test]
    async fn chat_mode_no_sub_agent_dispatchers() {
        let ctx = mock_chat_ctx().await;
        let tools = build_tool_list(&ctx, &SubAgentContext::depth(0)).await;
        assert!(tools.iter().all(|t| !t.name.starts_with("sub_agent_")));
    }

    #[tokio::test]
    async fn task_primary_only_dispatchers() {
        let ctx = mock_task_ctx().await;
        let tools = build_tool_list(&ctx, &SubAgentContext::depth(0)).await;
        assert!(tools.iter().all(|t| {
            t.name.starts_with("sub_agent_") || t.name == "ask_human"
        }), "task primary should only have orchestration tools; actual: {:?}",
        tools.iter().map(|t| &t.name).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn task_subtask_full_minus_update_plan() {
        let ctx = mock_task_ctx().await;
        let tools = build_tool_list(&ctx, &SubAgentContext::depth(1)).await;
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"js_collect"));
        assert!(names.contains(&"read_file"));
        assert!(!names.contains(&"update_plan"));
        // sub-agents at depth>0 cannot ask_human
        assert!(!names.contains(&"ask_human"));
    }
}
```

**前置依赖：** 若 `test_utils/context.rs` 未提供 `mock_chat_ctx` / `mock_task_ctx`，先在该文件加：

```rust
pub async fn mock_chat_ctx() -> AgenticLoopContext<'static> { /* compose */ }
pub async fn mock_task_ctx() -> AgenticLoopContext<'static> { /* compose */ }
```

实现要点：用现有 `Default::default()` 加 chat/task 模式 + 装载 default registry + 注册 pentest_bridge mock 工具。

**验证：**
```bash
cargo test -p golish-agent-runtime agentic_loop::tool_list 2>&1 | tail -10
```
**预期输出：** `test result: ok. 4 passed`.

---

### 任务 2.6：手动端到端验证

**步骤：** 启动 Golish App，打开 AI Chat Panel，确保 ExecutionModePicker 选中 Chat 模式：

```
你：收集 http://example.com 的 JS 文件
```

**预期：** AI 直接调用 `js_collect`（你会在 ToolCallSummary 看到 `🔧 js_collect`），不再回退到 Python 脚本，不再说"环境限制"。

**日志确认：**
```bash
RUST_LOG=debug 启动 Golish; 在 stderr 应看到一行：
[tool_list] Available tools (policy-driven, depth=0): [..., "js_collect", "manage_targets", "run_pipeline", ...]
```

---

### 任务 2.7：回归 — task 模式仍按旧行为

**步骤：** 同样的会话切到 Task 模式重新发：

```
你：收集 http://example.com 的 JS 文件
```

**预期：** 主 agent 调用 `sub_agent_browser` 派发，sub-agent 内部调 `js_collect`，最终回到主 agent 汇总。Workflow Progress 应显示 `browser` 子 agent 的执行卡片。

---

### 任务 2.8：提交 PR2

**提交：**
```bash
git add backend/crates/golish-agent-runtime/src/execution_mode/selection_apply.rs \
        backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs \
        backend/crates/golish-agent-runtime/src/agentic_loop/context.rs \
        backend/crates/golish-agent-runtime/src/test_utils/context.rs \
        backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs \
        backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs \
        backend/crates/golish-agent-bridge/src/agent_bridge/constructors/mod.rs \
        backend/crates/golish-agent-kit/src/execution_mode.rs

git commit -m "[exec-mode] PR2: tool_list.rs delegates to ExecutionModePolicy; fix chat-mode pentest_bridge bug

Before: tool_list.rs filtered registry tools by 'pentest_' prefix.
pentest_bridge tools (js_collect / manage_targets / run_pipeline /
record_finding / vault / flow_compose / js_extract_apis / auth_probe)
do not carry the prefix, so chat-mode LLM never saw them. The
prompt template (chat.rs) listed manage_targets/run_pipeline/...
without the LLM actually having them, leading to the 'environment
restricted my tools' confusion users observed.

After: tool_list.rs is policy-driven. ChatModePolicy.primary_tools
sets BridgeToolSelection::all_enabled(), so apply_tool_selection
exposes the 8 pentest_bridge tools to chat-mode LLM directly.
TaskModePolicy keeps primary as orchestration-only and subtask as
the full toolbox minus update_plan.

- 4 integration tests cover chat/task primary/subtask shapes."
```

---

## PR3 · system prompt 模板化（锁死 prompt-vs-tools 一致性）

> 当前 chat.rs / task.rs 的 markdown 工具表格是手写的，会与实际暴露不一致（chat 写了 `manage_targets` 但 LLM 拿不到 → 误导）。本 PR 把工具表的内容由 ToolSelection 自动派生。

### 任务 3.1：render_tool_table_for_prompt 纯函数

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs`（新建）

```rust
//! Render a markdown table of tools available to the LLM, derived from a
//! ToolSelection. Used as the {{ tool_table }} placeholder in the chat /
//! task / future plan / debug prompt templates.

use crate::execution_mode::policy::ToolSelection;

#[derive(Debug, Clone, Copy)]
struct ToolRow {
    name: &'static str,
    purpose: &'static str,
}

const STATIC_FILE_OPS: &[ToolRow] = &[
    ToolRow { name: "read_file", purpose: "Read file content. Always read before editing." },
    ToolRow { name: "edit_file", purpose: "Targeted edits in an existing file." },
    ToolRow { name: "create_file", purpose: "Create a new file (fails if exists)." },
    ToolRow { name: "write_file", purpose: "Overwrite an entire file." },
    ToolRow { name: "delete_file", purpose: "Remove a file." },
    ToolRow { name: "grep_file", purpose: "Regex search across files." },
    ToolRow { name: "list_files", purpose: "List / find files by pattern." },
];

const STATIC_CORE: &[ToolRow] = &[
    ToolRow { name: "ast_grep", purpose: "Structural code search (function calls, imports)." },
    ToolRow { name: "ast_grep_replace", purpose: "Structural refactor / rename." },
    ToolRow { name: "update_plan", purpose: "Create and track task plans." },
];

const STATIC_MEMORY: &[ToolRow] = &[
    ToolRow { name: "search_memories", purpose: "Search long-term memory." },
    ToolRow { name: "store_memory", purpose: "Store findings to memory." },
    ToolRow { name: "list_memories", purpose: "List recent memories." },
];

const STATIC_KNOWLEDGE: &[ToolRow] = &[
    ToolRow { name: "search_guide", purpose: "Search saved playbooks." },
    ToolRow { name: "save_guide", purpose: "Save a successful procedure." },
    ToolRow { name: "search_code", purpose: "Search saved code snippets." },
    ToolRow { name: "save_code", purpose: "Save a useful code snippet." },
    ToolRow { name: "search_knowledge_base", purpose: "Search vulnerability knowledge base." },
    ToolRow { name: "read_knowledge", purpose: "Read a knowledge entry." },
    ToolRow { name: "write_knowledge", purpose: "Append a knowledge entry." },
];

const STATIC_SECURITY: &[ToolRow] = &[
    ToolRow { name: "log_operation", purpose: "Log a pentest action and outcome." },
    ToolRow { name: "discover_apis", purpose: "Persist API endpoints per target." },
    ToolRow { name: "save_js_analysis", purpose: "Persist JS analysis findings." },
    ToolRow { name: "fingerprint_target", purpose: "Persist tech fingerprint." },
    ToolRow { name: "log_scan_result", purpose: "Persist a single security test result." },
    ToolRow { name: "query_target_data", purpose: "Query all known data about a target." },
];

const STATIC_GRAPH: &[ToolRow] = &[
    ToolRow { name: "graph_search", purpose: "Search the security knowledge graph." },
    ToolRow { name: "graph_neighbors", purpose: "Walk neighbours of a node." },
    ToolRow { name: "graph_attack_paths", purpose: "Compute attack paths." },
    ToolRow { name: "graph_add_entity", purpose: "Add a graph entity." },
    ToolRow { name: "graph_add_relation", purpose: "Add a graph relation." },
];

const STATIC_SPLOITUS: &[ToolRow] = &[
    ToolRow { name: "search_exploits", purpose: "Search exploit database." },
    ToolRow { name: "ingest_cve", purpose: "Ingest a CVE record." },
    ToolRow { name: "save_poc", purpose: "Save a proof-of-concept." },
    ToolRow { name: "list_cves_with_pocs", purpose: "List CVEs with PoCs." },
];

const BRIDGE_ROWS: &[ToolRow] = &[
    ToolRow { name: "manage_targets", purpose: "Add / list / update pentest targets." },
    ToolRow { name: "record_finding", purpose: "Record a vulnerability finding." },
    ToolRow { name: "vault", purpose: "Store / retrieve credentials." },
    ToolRow { name: "run_pipeline", purpose: "Run a predefined recon / scan pipeline." },
    ToolRow { name: "flow_compose", purpose: "Compose a multi-tool flow declaratively." },
    ToolRow { name: "js_collect", purpose: "Crawl HTML / inline scripts / build manifests and download all JS chunks." },
    ToolRow { name: "js_extract_apis", purpose: "Static-analyse captured JS for endpoints + secrets." },
    ToolRow { name: "auth_probe", purpose: "Active probe of a login form / OAuth flow." },
];

const RUNTIME_PENTEST: &[ToolRow] = &[
    ToolRow { name: "pentest_list_tools", purpose: "List installed pentest tools and their skills." },
    ToolRow { name: "pentest_run", purpose: "Execute a pentest tool by name with arguments." },
    ToolRow { name: "pentest_read_skill", purpose: "Read a skill document for tool usage." },
];

const RUNTIME_TAVILY: &[ToolRow] = &[
    ToolRow { name: "tavily_search", purpose: "Web search with source results." },
    ToolRow { name: "tavily_search_answer", purpose: "Web search with AI-generated answer." },
    ToolRow { name: "tavily_extract", purpose: "Extract structured content from URLs." },
];

pub fn render_tool_table_for_prompt(s: &ToolSelection) -> String {
    let mut rows: Vec<ToolRow> = Vec::new();

    if s.static_groups.file_ops { rows.extend_from_slice(STATIC_FILE_OPS); }
    if s.static_groups.core { rows.extend_from_slice(STATIC_CORE); }
    if s.static_groups.memory { rows.extend_from_slice(STATIC_MEMORY); }
    if s.static_groups.knowledge_base { rows.extend_from_slice(STATIC_KNOWLEDGE); }
    if s.static_groups.security_analysis { rows.extend_from_slice(STATIC_SECURITY); }
    if s.static_groups.graph { rows.extend_from_slice(STATIC_GRAPH); }
    if s.static_groups.sploitus { rows.extend_from_slice(STATIC_SPLOITUS); }

    for name in s.bridge_tools.enabled_tool_names() {
        if let Some(row) = BRIDGE_ROWS.iter().find(|r| r.name == name) {
            rows.push(*row);
        }
    }

    if s.runtime_tools.pentest_runtime { rows.extend_from_slice(RUNTIME_PENTEST); }
    if s.runtime_tools.tavily { rows.extend_from_slice(RUNTIME_TAVILY); }

    if s.include_run_command {
        rows.push(ToolRow { name: "run_pty_cmd", purpose: "Execute shell commands with PTY support." });
    }
    if s.include_ask_human {
        rows.push(ToolRow { name: "ask_human", purpose: "Ask the user a clarifying question." });
    }

    let denied: std::collections::HashSet<&str> =
        s.deny_overrides.iter().map(|x| x.as_str()).collect();
    rows.retain(|r| !denied.contains(r.name));

    let mut out = String::from("| Tool | Purpose |\n|---|---|\n");
    for r in rows {
        out.push_str(&format!("| `{}` | {} |\n", r.name, r.purpose));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_mode::policy::*;

    #[test]
    fn chat_table_includes_js_collect() {
        let s = ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            bridge_tools: BridgeToolSelection::all_enabled(),
            runtime_tools: RuntimeToolSelection { pentest_runtime: true, tavily: true },
            agent_tools: AgentToolSelection::none(),
            include_run_command: true,
            include_ask_human: true,
            deny_overrides: vec![],
        };
        let table = render_tool_table_for_prompt(&s);
        assert!(table.contains("`js_collect`"));
        assert!(table.contains("`manage_targets`"));
        assert!(table.contains("`run_pty_cmd`"));
    }

    #[test]
    fn deny_overrides_filter_out_tools() {
        let s = ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            deny_overrides: vec!["update_plan".into()],
            ..Default::default()
        };
        let table = render_tool_table_for_prompt(&s);
        assert!(!table.contains("`update_plan`"));
        assert!(table.contains("`read_file`"));
    }
}
```

---

### 任务 3.2：把 chat.rs 改为读模板

**文件 1：** `backend/crates/golish-agent-runtime/src/execution_mode/templates/chat.tera`（新建）

把 `backend/crates/golish-prompts/src/system_prompt/chat.rs` 现有的 raw 字符串内容拷过来，把 `## File Operations`、`## Pentest Bridge Tools (Direct)` 等所有手写的工具表删除，统一替换为：

```tera
# Tool Reference

The following tools are available to you in this turn. Always prefer specialized tools over `run_pty_cmd`.

{{ tool_table }}
```

**文件 2：** `backend/crates/golish-prompts/src/system_prompt/chat.rs`（重写）

把现有的 `format!(r#"..."#)` 重写为：

```rust
use tera::{Context, Tera};

const TEMPLATE: &str = include_str!(
    "../../../../golish-agent-runtime/src/execution_mode/templates/chat.tera"
);

pub(super) fn build_chat_prompt(
    workspace_path: &Path,
    agent_mode: AgentMode,
    memory_file_path: Option<&Path>,
    tool_table: &str,
) -> String {
    let project_instructions = read_project_instructions(workspace_path, memory_file_path);
    let rules_section = build_rules_section(workspace_path);
    let agent_mode_instructions = get_agent_mode_instructions(agent_mode);

    let mut ctx = Context::new();
    ctx.insert("workspace", &workspace_path.display().to_string());
    ctx.insert("project_instructions", &project_instructions);
    ctx.insert("rules_section", &rules_section);
    ctx.insert("agent_mode_instructions", &agent_mode_instructions);
    ctx.insert("tool_table", tool_table);

    Tera::one_off(TEMPLATE, &ctx, false).expect("chat.tera must render")
}
```

**注意：** 调用方 `build_system_prompt_with_contributions` 需要新增 `tool_table: &str` 参数；找出全部调用点（`prepare.rs` 两处）一起补。同 PR 内修。

**验证：**
```bash
cargo check -p golish-prompts -p golish-agent-bridge 2>&1 | tail -10
```
**预期输出：** `Finished` 不含 error。

---

### 任务 3.3：把 task.rs 改为读模板

**文件 1：** `backend/crates/golish-agent-runtime/src/execution_mode/templates/task.tera`（新建）

把 `backend/crates/golish-prompts/src/system_prompt/task.rs` 现有内容拷过来，同样删手写工具表，加 `{{ tool_table }}`。

**文件 2：** `backend/crates/golish-prompts/src/system_prompt/task.rs`（重写）

照 3.2 同款改造，函数签名加 `tool_table: &str` 参数。

**验证：** 同 3.2。

---

### 任务 3.4：契约测试 — prompt 表格 ⊆ 实际工具

**文件：** `backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs`（在 3.1 文件 tests 模块末尾追加）

```rust
#[tokio::test]
async fn prompt_table_subset_of_actual_tools_chat() {
    use crate::agentic_loop::tool_list::build_tool_list;
    use crate::test_utils::context::mock_chat_ctx;
    use golish_sub_agents::SubAgentContext;
    use crate::execution_mode::modes::chat::ChatModePolicy;
    use crate::execution_mode::context::PolicyContext;
    use std::path::Path;

    let ctx = mock_chat_ctx().await;
    let actual: std::collections::HashSet<String> =
        build_tool_list(&ctx, &SubAgentContext::depth(0))
            .await.into_iter().map(|t| t.name).collect();

    let s = ChatModePolicy.primary_tools(
        &PolicyContext::new(Path::new("/tmp"), golish_core::AgentMode::Default)
    ).await;
    let table = render_tool_table_for_prompt(&s);

    for line in table.lines() {
        if let Some(start) = line.find("| `") {
            let after = &line[start + 3..];
            if let Some(end) = after.find('`') {
                let tool_name = &after[..end];
                assert!(
                    actual.contains(tool_name),
                    "prompt mentions `{}` but it's not in actual tool list",
                    tool_name
                );
            }
        }
    }
}
```

**验证：**
```bash
cargo test -p golish-agent-runtime execution_mode::prompt_render 2>&1 | tail -10
```
**预期输出：** `test result: ok. 3 passed`.

---

### 任务 3.5：提交 PR3

**提交：**
```bash
git add backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs \
        backend/crates/golish-agent-runtime/src/execution_mode/mod.rs \
        backend/crates/golish-agent-runtime/src/execution_mode/templates/ \
        backend/crates/golish-prompts/src/system_prompt/chat.rs \
        backend/crates/golish-prompts/src/system_prompt/task.rs \
        backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs \
        backend/crates/golish-agent-bridge/src/agent_bridge/system_prompt.rs

git commit -m "[exec-mode] PR3: prompt templated via ToolSelection-derived tool_table

Single source of truth: the markdown tool table inside chat /
task system prompt is now generated from the active ToolSelection,
not hand-written. Adding a new tool to BridgeToolSelection /
StaticGroupSelection automatically updates the prompt.

- Tera templates in execution_mode/templates/{chat,task}.tera
- render_tool_table_for_prompt() + 3 unit tests
- 1 contract test asserting prompt_table_subset_of_actual_tools"
```

---

## PR4 · 前端动态拉取模式列表

> 后端注册新模式 → 前端下拉自动出现，不再硬编码 chat / task 二选一。

### 任务 4.1：后端 Tauri command list_execution_modes

**文件：** `backend/crates/golish/src/ai/commands/mode.rs`（修改 / 新增 command）

```rust
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionModeDescriptor {
    pub id: String,
    pub display_name: String,
    pub icon: String,
    pub badge_color: String,
    pub description: String,
    pub allows_sub_agents: bool,
}

#[tauri::command]
pub async fn list_execution_modes(state: State<'_, AppState>) -> Result<Vec<ExecutionModeDescriptor>, String> {
    let registry = state.execution_mode_registry.clone();
    let policies = registry.list_all();
    Ok(policies.into_iter().map(|p| {
        let l = p.label();
        ExecutionModeDescriptor {
            id: p.id().to_string(),
            display_name: l.display_name.to_string(),
            icon: l.icon.to_string(),
            badge_color: l.badge_color.to_string(),
            description: p.description().to_string(),
            allows_sub_agents: p.allows_sub_agents(),
        }
    }).collect())
}
```

**注册：** `backend/crates/golish/src/commands_registry.rs` 的 `tauri::generate_handler!` 列表里加上 `list_execution_modes`。

**AppState 加字段：** `backend/crates/golish/src/state.rs` 加 `pub execution_mode_registry: Arc<ExecutionModeRegistry>`，启动时实例化。

**验证：**
```bash
cargo check -p golish 2>&1 | tail -5
```

---

### 任务 4.2：前端 zod schema + IPC client

**文件 1：** `frontend/lib/types/executionMode.ts`（新建）

```typescript
import { z } from "zod";

export const ExecutionModeDescriptorSchema = z.object({
  id: z.string(),
  displayName: z.string(),
  icon: z.string(),
  badgeColor: z.string(),
  description: z.string(),
  allowsSubAgents: z.boolean(),
});

export type ExecutionModeDescriptor = z.infer<typeof ExecutionModeDescriptorSchema>;

export const ExecutionModeListSchema = z.array(ExecutionModeDescriptorSchema);
```

**文件 2：** `frontend/lib/ai.ts`（修改 / 追加导出）

```typescript
import { invoke } from "@tauri-apps/api/core";
import { ExecutionModeListSchema, type ExecutionModeDescriptor } from "@/lib/types/executionMode";

export async function listExecutionModes(): Promise<ExecutionModeDescriptor[]> {
  const raw = await invoke("list_execution_modes");
  return ExecutionModeListSchema.parse(raw);
}
```

后端用 snake_case，前端用 camelCase。在 backend `ExecutionModeDescriptor` 加 `#[serde(rename_all = "camelCase")]` 或者前端加 transform 函数。本计划选用后端 serde rename，文件 4.1 改为：

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionModeDescriptor { /* 同上 */ }
```

---

### 任务 4.3：前端 ExecutionModePicker 改用动态列表

**文件：** `frontend/components/AIChatPanel/ExecutionModePicker.tsx`（重写）

```tsx
import { ChevronDown, MessageSquare, Users, Zap } from "lucide-react";
import { memo, useEffect, useState } from "react";
import { listExecutionModes } from "@/lib/ai";
import type { ExecutionModeDescriptor } from "@/lib/types/executionMode";
import { /* DropdownMenu* */ } from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

const ICON_MAP: Record<string, React.ComponentType<{ className?: string }>> = {
  MessageSquare,
  Zap,
  Users,
};

interface Props {
  chatExecutionMode: string;
  chatUseSubAgents: boolean;
  onExecutionModeChange: (mode: string) => void;
  onAgentModeChange: (mode: "default" | "auto-approve") => void;
  onToggleSubAgents: () => void;
}

export const ExecutionModePicker = memo(function ExecutionModePicker({
  chatExecutionMode, chatUseSubAgents, onExecutionModeChange,
  onAgentModeChange, onToggleSubAgents,
}: Props) {
  const [modes, setModes] = useState<ExecutionModeDescriptor[]>([]);

  useEffect(() => {
    let cancelled = false;
    listExecutionModes().then((m) => { if (!cancelled) setModes(m); }).catch(console.error);
    return () => { cancelled = true; };
  }, []);

  const active = modes.find((m) => m.id === chatExecutionMode);
  const ActiveIcon = ICON_MAP[active?.icon ?? "MessageSquare"] ?? MessageSquare;

  return (
    /* ... 渲染 modes.map 项 ... */
  );
});
```

完整渲染体省略；要点：删除原文件中的写死 chat / task 两个 DropdownMenuItem，用 `modes.map((m) => <DropdownMenuItem ...>)` 替代；点击时 `onExecutionModeChange(m.id); onAgentModeChange(m.allowsSubAgents ? "auto-approve" : "default");`。

---

### 任务 4.4：useChatModes hook 类型放宽

**文件：** `frontend/components/AIChatPanel/hooks/useChatModes.ts`（修改）

```typescript
const [chatExecutionMode, setChatExecutionMode] = useState<string>("chat");
```

并把所有 `"chat" | "task"` 类型签名改为 `string`。`handleExecutionModeChange(mode: string)` 同步改。

`useChatStreamingSync.ts` / `useChatSend.ts` / `useChatSessionInit.ts` 里的 `MutableRefObject<"chat" | "task">` 类型同样放宽为 `MutableRefObject<string>`。

**验证：**
```bash
cd frontend && npx tsc --noEmit 2>&1 | tail -10
```
**预期输出：** 0 type errors。

---

### 任务 4.5：手动验证

**步骤：**

1. 启动 Golish App。
2. 打开 AI Chat Panel，点击右下角执行模式下拉。
3. 应看到 `Chat` / `Task` 两项，hover 各项显示描述。
4. （可选）在后端 `AppState` 初始化时多注册一个测试 policy（比如 `PlanModePolicy`），刷新前端 → 下拉自动多出 "Plan"。

---

### 任务 4.6：提交 PR4

**提交：**
```bash
git add backend/crates/golish/src/ai/commands/mode.rs \
        backend/crates/golish/src/commands_registry.rs \
        backend/crates/golish/src/state.rs \
        frontend/lib/types/executionMode.ts \
        frontend/lib/ai.ts \
        frontend/components/AIChatPanel/ExecutionModePicker.tsx \
        frontend/components/AIChatPanel/hooks/useChatModes.ts \
        frontend/components/AIChatPanel/hooks/useChatStreamingSync.ts \
        frontend/components/AIChatPanel/hooks/useChatSend.ts \
        frontend/components/AIChatPanel/hooks/useChatSessionInit.ts

git commit -m "[exec-mode] PR4: front-end ExecutionModePicker consumes list_execution_modes IPC

Backend Tauri command list_execution_modes returns the descriptors
of all registered execution modes. Front-end fetches them on mount
and renders the dropdown dynamically — registering a new policy on
the backend (e.g. PlanModePolicy) causes the dropdown item to
appear without front-end churn.

- zod schema for type safety across the IPC boundary
- useChatModes type relaxed from union to string"
```

---

## 全局验证矩阵

| 项目 | 验证方式 |
|---|---|
| chat 模式 LLM 能调 `js_collect` | `cargo test agentic_loop::tool_list::tests::chat_mode_exposes_js_collect` |
| chat 模式 LLM 不会派 sub_agent | `cargo test ::chat_mode_no_sub_agent_dispatchers` |
| task primary 只见 sub_agent 派发 | `cargo test ::task_primary_only_dispatchers` |
| task subtask 见全集去 update_plan | `cargo test ::task_subtask_full_minus_update_plan` |
| prompt ⊆ 实际工具 | `cargo test execution_mode::prompt_render::tests::prompt_table_subset_of_actual_tools_chat` |
| 前端类型 | `npx tsc --noEmit` 0 errors |
| 端到端：chat 对真实 URL 调 js_collect | 手动跑 PR2 任务 2.6 |
| 回归：task 仍编排 sub-agent | 手动跑 PR2 任务 2.7 |

---

## 迁移与回滚

- **数据库**：不动，`terminal_execution_mode` 列仍是 `"chat" | "task"` 字符串。新增模式 = 新字符串值，自动兼容。
- **历史会话**：旧 transcript 不受影响。Chat 历史里 LLM 此前曾"用不了 js_collect"是事实，新会话自然恢复。
- **回滚**：每个 PR 独立可 revert。PR2 revert = 回到 bug 状态、PR1 / PR3 / PR4 仍在 main 但未生效，无副作用。

---

## 风险与控制

| 风险 | 控制 |
|---|---|
| 加新模式 → prompt 模板里 `{{ tool_table }}` 占位符忘记加 | render_tool_table_for_prompt 不参与渲染时模板留空段，CI 加 grep 检查每个 `*.tera` 必含 `{{ tool_table }}` |
| 模板 Tera 渲染失败 | `Tera::one_off(template, &ctx, false).expect()` 在测试时 panic，启动时立即 fail-fast |
| sub-agent registry 与 ToolSelection 双重管理 | 显式声明边界：sub-agent 内部 `allowed_tools` 仍由 `golish-sub-agents/registry.rs` 管，与 ToolSelection 独立。后续计划再合并 |
| 前端注册的硬编码 `"chat" | "task"` 常量 (i18n / 测试) | grep 一遍找到所有出现并改字符串；本 PR4 包含 |
| 老 PR / 老分支 merge 冲突 | PR1 与 PR2 / PR3 / PR4 顺序合入；如有第三人在 tool_list.rs 上有 in-flight PR，等他 merge 后 PR2 rebase |

---

## 自检

**规格覆盖度：** 用户原始需求是「修复 chat 模式工具被过滤 + 给未来多模式留扩展点」 — PR2 修 bug、PR1+PR3+PR4 留扩展点；规格全覆盖。

**占位符扫描：** 所有 7 个 PR 内任务都有完整代码，未出现 TODO / 待补 / 后续 / "类似 N" 等字样。

**类型一致性：** `ToolSelection` 字段名（`bridge_tools.js_collect` / `agent_tools.include_dispatch_tools`）在 PR1 / PR2 / PR3 中签名一致；前端 `ExecutionModeDescriptor` 与后端 serde rename 后的 camelCase 一致。


