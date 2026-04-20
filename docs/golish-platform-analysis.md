# Golish Platform 完整架构与流程分析

> 本文档是对 Golish Platform 项目的完整技术分析。
> 目的是提供一份详尽的参考资料，以避免反复查看代码。

---

## 1. 项目概览

Golish 是一个 **AI 驱动的终端模拟器 + 安全测试平台**，使用 Tauri 2 构建（Rust 后端 + React 19 前端），集成了 LLM Agent 系统，支持多种 AI Provider。

| 维度 | 技术选型 |
|------|---------|
| 应用框架 | Tauri 2 (桌面应用) |
| 后端 | Rust (28 个 crate, 4 层架构) |
| 前端 | React 19 + TypeScript + Tailwind v4 |
| 状态管理 | Zustand + Immer (enableMapSet) |
| LLM 框架 | rig-core (Rust LLM 抽象) |
| 终端 | portable-pty + vte + xterm.js |
| 工作流 | graph-flow (图执行引擎) |
| 测试 | Vitest (前端), cargo nextest (Rust), Playwright (E2E) |
| 构建 | Vite (前端), Cargo (后端), Just (任务管理) |
| 可观测性 | OpenTelemetry + Langfuse (LLM 分析) |
| 数据库 | PostgreSQL + SQLx (可选, 正在集成) |

---

## 2. 架构层级 (4 层)

```
Layer 4: Application (golish)
  ├── Tauri Commands (AI, PTY, Shell, Themes, Files, Skills, Settings, Sidecar, Indexer)
  ├── CLI Entry (--headless, -e prompt)
  └── Runtime & History

Layer 3: Domain (golish-ai)
  ├── Agent Orchestration
  ├── Agentic Loop
  ├── Planning & HITL
  ├── Loop Detection & Tool Policy
  └── Indexer (Codebase Analysis)

Layer 2: Infrastructure (22 crates)
  ├── golish-context     (token budget, context pruning, compaction)
  ├── golish-session     (conversation persistence)
  ├── golish-tools       (tool system, registry, file ops, directory ops, AST search)
  ├── golish-sub-agents  (sub-agent definitions and execution)
  ├── golish-workflow     (graph-based multi-step tasks)
  ├── golish-pty         (PTY terminal sessions)
  ├── golish-shell-exec  (shell execution)
  ├── golish-sidecar     (context capture)
  ├── golish-web         (web search, content fetching)
  ├── golish-settings    (TOML config management)
  ├── golish-udiff       (unified diff system)
  ├── golish-mcp         (MCP client)
  ├── golish-synthesis   (session synthesis)
  ├── golish-artifacts   (artifact management)
  ├── golish-evals       (evaluation framework)
  ├── golish-db          (PostgreSQL persistence, NEW)
  └── Provider crates:
      ├── rig-anthropic-vertex
      ├── rig-gemini-vertex
      ├── rig-zai / rig-zai-anthropic
      └── rig-openai-responses

Layer 1: Foundation (golish-core)
  ├── Event types (AiEvent, AiEventEnvelope)
  ├── Runtime trait (GolishRuntime)
  ├── Session types
  ├── HITL interfaces
  ├── Planning types
  └── Zero internal dependencies
```

---

## 3. 核心执行流程

### 3.1 Agent Turn 生命周期

```
用户输入 (前端 UnifiedInput)
  → Tauri invoke: send_message()
  → AgentBridge.process_message()
    → build_system_prompt()
      ├── 通用系统提示 (agent 身份、工具说明、任务管理)
      ├── project instructions (CLAUDE.md / memory file)
      └── agent mode 指令 (default / auto-approve / planning)
    → 添加 user message 到 conversation_history
    → run_agentic_loop_unified() (核心循环)
    → 更新 conversation_history
    → 持久化到 session / transcript
    → 发送 AiEvent::Completed 到前端
```

### 3.2 Agentic Loop 核心循环

这是整个系统的核心，位于 `agentic_loop.rs` 的 `run_agentic_loop_unified()`:

```
run_agentic_loop_unified(model, system_prompt, initial_history, sub_agent_ctx, ctx, config):

  // 初始化
  reset loop_detector
  create LoopCaptureContext (sidecar)
  create HookRegistry
  
  // 构建工具列表
  tools = get_all_tool_definitions_with_config(tool_config)
  tools += run_command (run_pty_cmd wrapper)
  tools += ask_human (barrier tool)
  tools += additional_tool_definitions (eval 注入)
  tools += dynamic registry tools (Tavily, pentest, etc.)
  if depth < MAX_AGENT_DEPTH - 1:
    tools += sub_agent_tools (from SubAgentRegistry)

  for iteration in [1, MAX_TOOL_ITERATIONS=100]:

    // 1. Context Compaction 检查
    if should_compact(compaction_state, model_name):
      perform_compaction(session_id, chat_history)
        → read transcript
        → format_for_summarizer()
        → call Summarizer LLM
        → extract <summary> from response
        → replace history with [summary + last user message]
        → emit CompactionCompleted event
    
    // 2. Token 估算 (proactive, tokenx-rs)
    estimated_tokens = system_prompt_tokens + history_tokens
    compaction_state.update_tokens_estimated(estimated_tokens)
    
    // 3. 构建 CompletionRequest
    request = {
      preamble: system_prompt,
      chat_history: one_or_many(history),
      tools: tools.clone(),
      temperature: 0.3 (if supported),
      max_tokens: 10_000,
      additional_params: { web_search?, reasoning?, provider_prefs? }
    }
    
    // 4. Stream Request (with retry)
    for attempt in [1, STREAM_START_MAX_ATTEMPTS=3]:
      stream = model.stream(request)
      if error:
        classify → context_overflow / authentication / rate_limit / timeout / api_error
        if retriable: backoff + retry
        else: emit Error event, return TerminalErrorEmitted

    // 5. 处理 Streaming Response
    while chunk in stream:
      match chunk:
        Text → accumulate response, emit TextDelta
        [Thinking] prefix → emit Reasoning
        Reasoning → emit Reasoning, track thinking_content + signature
        ReasoningDelta → emit Reasoning (OpenAI Responses API)
        ToolCall → collect pending tool calls
        [WEB_SEARCH_RESULT:...] → emit WebSearchResult (server tool)
        [WEB_FETCH_RESULT:...] → emit WebFetchResult (server tool)

    // 6. 更新 Token Usage
    if FinalResponse has usage:
      update compaction_state with actual tokens
      track total_usage
    else:
      use heuristic estimation

    // 7. 构建 Assistant Message (加入 history)
    assistant_message = reasoning(if any) + text + tool_calls
    chat_history.push(assistant_message)

    // 8. 无工具调用 → 返回最终响应
    if no tool_calls:
      return (accumulated_response, accumulated_thinking, history, usage)

    // 9. 执行工具调用
    for tool_call in tool_calls_to_execute:
      
      // 9a. System Hooks (post-tool)
      inject_system_hooks_if_any(hook_registry, tool_call)
      
      // 9b. Loop Detection
      detection = loop_detector.check(tool_name, args)
      if detected: emit LoopDetected, inject warning

      // 9c. Tool Routing (see §4)
      result = route_tool_execution(tool_name, args, ...)

      // 9d. ask_human Barrier
      if tool_name == "ask_human":
        → emit AskHumanRequest
        → wait for user response (up to 600s)
        → emit AskHumanResponse
        → break iteration loop

      // 9e. Tool Policy + HITL Approval (see §5)
      // 9f. Execute tool
      // 9g. Emit ToolResult event
      // 9h. Add ToolResult to history

    // 10. 继续下一个 iteration
```

### 3.3 Stream Start 重试机制

```
Stream Start Retry:
  ├── 最大重试次数: 3 (initial + 2 retries)
  ├── 基础延迟: 300ms
  ├── 最大延迟: 3000ms
  ├── 退避策略: exponential + 20% jitter
  ├── 流超时: 180s (3 分钟)
  │
  ├── 不可重试错误 (立即失败):
  │   ├── context_length_exceeded
  │   └── authentication / 401 / 403
  │
  └── 可重试错误:
      ├── rate_limit / 429
      ├── timeout
      └── transient (connection, 500, 502, 503, 504)
```

---

## 4. 工具体系

### 4.1 工具类别

| 类别 | 工具 | 描述 |
|------|------|------|
| **搜索 & 发现** | `grep_file`, `list_files` | 文件搜索和目录列表 |
| **AST 代码操作** | `ast_grep`, `ast_grep_replace` | 基于 AST 的结构化搜索和替换 |
| **文件操作** | `read_file`, `create_file`, `edit_file`, `write_file`, `delete_file` | 文件 CRUD |
| **Shell** | `run_command` (→`run_pty_cmd`) | PTY 命令执行 |
| **代码执行** | `execute_code` | 执行代码片段 |
| **补丁** | `apply_patch` | 应用统一差异补丁 |
| **Web** | `web_fetch` | Readability 内容提取 |
| **Tavily 搜索** | `tavily_search`, `tavily_search_answer`, `tavily_extract`, `tavily_crawl`, `tavily_map` | Tavily API 集成 |
| **计划** | `update_plan` | 任务计划更新 |
| **Barrier** | `ask_human` | 暂停执行，请求用户输入 |
| **子 Agent** | `sub_agent_{id}` | 委托给专家子 Agent |

### 4.2 Tool Preset 系统

```
ToolPreset::Minimal (4 tools):
  read_file, edit_file, write_file, run_pty_cmd

ToolPreset::Standard (12 tools, 默认):
  grep_file, list_files, ast_grep, ast_grep_replace,
  read_file, create_file, edit_file, write_file, delete_file,
  run_pty_cmd, web_fetch, update_plan

ToolPreset::Full:
  所有已注册工具

ToolConfig::main_agent() (主 Agent 配置):
  Standard preset
  + execute_code, apply_patch
  + tavily_search, tavily_search_answer, tavily_extract, tavily_crawl, tavily_map
  - run_pty_cmd (替换为 run_command wrapper)
```

### 4.3 工具路由 (tool_execution.rs)

```
route_tool_execution(tool_name, args):
  ├── "web_fetch"         → execute_web_fetch_tool (readability extraction)
  ├── "tavily_*"          → Tavily API tools (from registry)
  ├── "update_plan"       → execute_plan_tool (PlanManager)
  ├── "sub_agent_*"       → execute_sub_agent_with_client (if depth allows)
  ├── "run_command"       → normalize args → route to run_pty_cmd
  ├── "ask_human"         → execute_ask_human_tool (barrier)
  ├── "pentest_*"         → Registry dynamic tools
  └── default             → ToolRegistry.execute(tool_name, args, workspace)
```

### 4.4 Schema 兼容性处理

所有工具参数 schema 经过 `sanitize_schema()` 处理以兼容多 Provider:

- 移除 `anyOf`, `allOf`, `oneOf` (Anthropic 不支持)
- 简化 `oneOf` 到第一个选项
- 添加 `additionalProperties: false` (OpenAI strict mode)
- 非必需属性添加 `null` 类型 (nullable)
- 所有属性加入 `required` 数组

---

## 5. HITL (Human-in-the-Loop) 审批系统

### 5.1 审批流程

```
Agent 发起工具调用
  → ToolPolicy 检查:
      ├── 只读模式? → 阻止写入工具
      ├── 已被 deny? → 直接拒绝
      └── 自定义约束? → 检查参数
  
  → ApprovalRecorder 检查:
      ├── 已有自动审批模式? → ToolAutoApproved
      └── 需要审批 → emit ToolApprovalRequest
          → 前端显示审批对话框
          → 用户选择:
              ├── Allow → 执行工具
              ├── Allow Always → 记录模式 + 执行
              └── Deny → 返回拒绝结果给 Agent

  → 审批超时: 300 秒 (5 分钟)
```

### 5.2 Agent Mode

| 模式 | 描述 | 审批行为 |
|------|------|----------|
| `default` | 正常模式 | 根据 policy 审批 |
| `auto-approve` | 自动审批 | 所有工具自动批准 |
| `planning` | 规划模式 | 只允许只读工具 |

### 5.3 Risk Level

| 等级 | 描述 |
|------|------|
| `Low` | 只读操作 (read_file, grep_file) |
| `Medium` | 可逆修改 (edit_file, create_file) |
| `High` | 不可逆或系统操作 (delete_file, run_command) |

### 5.4 ask_human Barrier Tool

```json
{
  "name": "ask_human",
  "parameters": {
    "question": "string (required)",
    "input_type": "credentials | choice | freetext | confirmation (required)",
    "options": ["array of strings (for choice type)"],
    "context": "string (additional context)"
  }
}
```

- 暂停 Agent 执行循环
- 通过 EventCoordinator 注册 approval channel
- 前端显示 AskHumanDialog
- 超时: 600 秒 (10 分钟)
- 用户可以回复或跳过

---

## 6. 子 Agent 系统

### 6.1 子 Agent 定义

```rust
SubAgentDefinition {
    id: String,              // 唯一标识 (e.g., "worker", "coder", "analyzer")
    name: String,            // 人类可读名称
    description: String,     // 描述 (供主 Agent 理解何时调用)
    system_prompt: String,   // 系统提示
    allowed_tools: Vec<String>, // 允许的工具 (空 = 全部)
    max_iterations: usize,   // 最大迭代次数
    model_override: Option<(String, String)>, // 可选模型覆盖
    timeout_secs: Option<u64>,      // 总超时 (默认 600s)
    idle_timeout_secs: Option<u64>, // 空闲超时 (默认 180s)
    prompt_template: Option<String>, // 可选的提示生成模板
}
```

### 6.2 默认子 Agent

| ID | 名称 | 描述 | 特殊能力 |
|----|------|------|----------|
| `worker` | Worker | 通用任务执行 | 动态提示生成 (`prompt_template`) |
| `coder` | Coder | 精确代码编辑 | 接收 `<implementation_plan>` 生成 unified diff |
| `analyzer` | Analyzer | 代码分析 | 只读工具, 结构化分析报告 |

### 6.3 Worker 动态提示生成

Worker Agent 的特殊之处在于 `prompt_template` 字段:

```
主 Agent 调用 sub_agent_worker(task="...")
  → 检测到 prompt_template 非空
  → 使用 prompt_template 作为 system prompt, task 作为 user message
  → 调用 LLM 生成优化后的 system prompt
  → 使用生成的 prompt 执行子 Agent
```

`WORKER_PROMPT_TEMPLATE` 是一个 "elite AI agent architect" 角色提示，
指导 LLM 为即将执行的任务生成最优系统提示。

### 6.4 子 Agent 执行

```
execute_sub_agent(agent_def, args, parent_context, model, ctx):
  1. 深度检查: depth < MAX_AGENT_DEPTH (5)
  2. 超时设置: timeout_secs (default 600s), idle_timeout_secs (default 180s)
  3. 创建 Langfuse span
  4. 确定系统提示:
     ├── 有 prompt_template → 调用 LLM 生成 → 使用生成结果
     └── 无 prompt_template → 直接使用 system_prompt
  5. 构建子 Agent 工具列表:
     ├── 过滤 allowed_tools
     └── 添加 run_command, ask_human
  6. 执行 mini agentic loop (与主循环类似):
     ├── Stream LLM response
     ├── 执行工具调用 (无 HITL, 直接执行)
     ├── 处理 unified diff (自动 apply)
     └── 跟踪 files_modified
  7. 返回 SubAgentResult { response, files_modified, duration_ms, success }
```

### 6.5 Agent 层级限制

```
MAX_AGENT_DEPTH = 5

Main Agent (depth=0)
  └── Sub-Agent (depth=1)
       └── Sub-Agent (depth=2)
            └── Sub-Agent (depth=3)
                 └── Sub-Agent (depth=4) ← 最大深度, 无法再调用子 Agent
```

---

## 7. 工作流系统 (Workflow)

### 7.1 架构

工作流使用 `graph-flow` 库进行图执行：

```rust
WorkflowDefinition trait:
  fn name() → &str
  fn description() → &str  
  fn build_graph(executor) → Arc<Graph>
  fn init_state(input) → serde_json::Value
  fn start_task() → &str
```

### 7.2 内置工作流

#### Git Commit Workflow

```
git_commit:
  analyze_changes → generate_message → review → commit
```

#### Recon Basic Workflow (侦察基础)

```
recon_basic:
  initialize → tool_check → tool_install → dns_lookup → http_probe → port_scan → tech_fingerprint → summarize
```

**Graph 结构:**
```
initialize ──→ tool_check ──→ tool_install ──→ dns_lookup
                                                    │
                                                    ▼
summarize ◄── tech_fingerprint ◄── port_scan ◄── http_probe
```

**ReconState 数据模型:**
```rust
ReconState {
    targets: Vec<String>,
    project_path: String,
    project_name: String,
    proxy_url: Option<String>,
    stage: ReconStage,
    available_tools: AvailableTools,
    target_data: HashMap<String, TargetReconData>,
    summary: Option<String>,
}

TargetReconData {
    dns: DnsResult,
    http: HttpResult,
    ports: Vec<PortInfo>,
    technologies: Vec<String>,
}
```

### 7.3 Workflow 执行

```
WorkflowRunner:
  ├── AgentWorkflowBuilder → 构建包含 SubAgentTask / RouterTask 的 Graph
  ├── WorkflowStorage → InMemory / PostgreSQL
  ├── FlowRunner (from graph-flow) → 按图执行 Task
  └── Context → 共享状态 (get/set 键值对)
```

---

## 8. Context 管理系统

### 8.1 Token Budget

```rust
TokenBudgetConfig {
    model_context_windows: HashMap<String, usize>,  // 模型上下文窗口映射
    warning_threshold: 0.80,   // 80% 触发警告
    critical_threshold: 0.90,  // 90% 触发压缩
    compaction_threshold: 0.80, // 80% 触发 compaction
    default_context_window: 200_000,
}

TokenAlertLevel:
  Normal → Warning(80%) → Critical(90%)
```

### 8.2 Context Compaction (上下文压缩)

```
Compaction 触发:
  ├── 每次迭代开始时检查 (should_compact)
  ├── 使用 proactive token 估算 (tokenx-rs, ~96% 精度)
  └── 阈值: 80% context window

Compaction 流程:
  1. 读取 transcript (JSONL)
  2. format_for_summarizer() → 结构化输入
  3. 调用 Summarizer LLM (独立 Agent, 无工具):
     ├── 系统提示: SUMMARIZER_SYSTEM_PROMPT
     ├── 分析 (<analysis> 标签) + 摘要 (<summary> 标签)
     └── 输出结构化摘要 (8 节)
  4. extract_summary_text() → 提取 <summary> 内容
  5. 替换 history: [系统摘要 + 最后一条用户消息]
  6. 保存工件:
     ├── ~/.golish/artifacts/compaction/summarizer-input-{ts}.md
     └── ~/.golish/artifacts/summaries/summary-{ts}.md

Summarizer 摘要结构:
  1. Primary Request and Intent
  2. Key Technical Concepts  
  3. Files and Code Sections
  4. Errors and Fixes
  5. Problem Solving
  6. All User Messages
  7. Pending Tasks
  8. Current Work
  9. Optional Next Step
```

### 8.3 Tool Output 截断

```
aggregate_tool_output(output, max_tokens):
  if output > max_tokens:
    保留前 60% + "[...truncated...]" + 后 40%
```

---

## 9. 循环检测 (Loop Detection)

```rust
LoopDetector:
  ├── 检测方法: 检查相同的 (tool_name, normalized_args) 是否重复出现
  ├── 阈值: 连续 3 次相同调用触发 Warning
  ├── 硬限制: 连续 5 次触发 Break
  └── 重置: 每个 turn 开始时重置

LoopDetectionResult:
  ├── NotDetected → 继续执行
  ├── Warning(count) → 注入系统消息提醒 Agent 换工具
  └── Break(count) → 终止当前 iteration
```

---

## 10. 事件系统 (Event System)

### 10.1 AiEvent 枚举 (40+ 变体)

| 分类 | 事件 | 描述 |
|------|------|------|
| **生命周期** | `Started`, `Completed`, `Error`, `Warning` | Agent turn 生命周期 |
| **流式输出** | `TextDelta`, `Reasoning` | LLM 流式文本和推理 |
| **工具** | `ToolRequest`, `ToolApprovalRequest`, `ToolAutoApproved`, `ToolDenied`, `ToolResult`, `ToolOutputChunk` | 工具执行全流程 |
| **HITL** | `AskHumanRequest`, `AskHumanResponse` | 用户交互 |
| **子 Agent** | `SubAgentStarted`, `SubAgentToolRequest`, `SubAgentToolResult`, `SubAgentCompleted`, `SubAgentError` | 子 Agent 生命周期 |
| **工作流** | `WorkflowStarted`, `WorkflowStepStarted`, `WorkflowStepCompleted`, `WorkflowCompleted` | 工作流生命周期 |
| **上下文** | `ContextUtilization`, `CompactionStarted`, `CompactionCompleted`, `CompactionFailed` | 上下文管理 |
| **循环** | `LoopDetected`, `LoopProtection` | 循环保护 |
| **计划** | `PlanUpdated` | 任务计划更新 |
| **系统** | `SystemHooksInjected`, `UserMessage` | 系统级事件 |
| **Web** | `WebSearchResult`, `WebFetchResult`, `ServerToolStarted` | Web/Server 工具 |

### 10.2 Event Coordinator (事件协调器)

```
EventCoordinator (单 Tokio task):
  ├── 拥有所有事件相关状态 (消除死锁可能)
  ├── event_sequence: u64 (单调递增序列号)
  ├── frontend_ready: bool
  ├── event_buffer: Vec<AiEventEnvelope>
  ├── pending_approvals: HashMap<String, Sender<ApprovalDecision>>
  │
  ├── CoordinatorCommand:
  │   ├── EmitEvent { event }
  │   ├── MarkFrontendReady (刷新缓冲区)
  │   ├── RegisterApproval { request_id, response_tx }
  │   ├── ResolveApproval { decision }
  │   ├── SetTranscriptWriter { writer }
  │   └── Shutdown
  │
  └── AiEventEnvelope { seq, ts, event }
      ├── seq: 单调递增序列号 (用于排序和间隙检测)
      └── ts: RFC 3339 时间戳

通信路径:
  AgentBridge → CoordinatorHandle (send) → EventCoordinator (single task)
    → runtime.emit("ai-event", envelope) → Frontend
```

### 10.3 Transcript (转录)

```
TranscriptWriter:
  ├── 路径: ~/.golish/transcripts/{session_id}/transcript.json
  ├── 格式: JSONL (每行一个 JSON 对象)
  ├── 追加写入 (append-only)
  │
  ├── 过滤规则 (should_transcript):
  │   ├── 排除 TextDelta (由 Completed 聚合)
  │   ├── 排除 Reasoning (由 Completed.reasoning 聚合)
  │   ├── 排除 ToolOutputChunk (由 ToolResult 聚合)
  │   └── 排除 SubAgentToolRequest/Result (内部事件)
  │
  └── 子 Agent 转录:
      └── 独立文件: {base_dir}/{session_id}/sub-agent-{id}-{ts}.json
```

---

## 11. LLM Provider 系统

### 11.1 支持的 Provider (12+)

| Provider | 类型 | 实现 |
|----------|------|------|
| Anthropic (Vertex) | Vertex AI | `rig-anthropic-vertex` |
| Anthropic (Direct) | API | `rig::providers::anthropic` |
| OpenAI | API | `rig::providers::openai` |
| OpenAI Responses | API | `rig-openai-responses` |
| Gemini (Vertex) | Vertex AI | `rig-gemini-vertex` |
| Gemini (Direct) | API | `rig::providers::gemini` |
| OpenRouter | 聚合器 | `rig::providers::openrouter` |
| Ollama | 本地 | `rig::providers::ollama` |
| Groq | API | `rig::providers::groq` |
| xAI | API | `rig::providers::xai` |
| Z.AI (智谱) | API | `rig-zai` |
| NVIDIA NIM | API | `rig::providers::openai` (兼容) |

### 11.2 LlmClient 枚举

```rust
enum LlmClient {
    VertexAnthropic(CompletionModel),
    RigOpenRouter(CompletionModel),
    RigOpenAi(CompletionModel),
    RigOpenAiResponses(CompletionModel),
    OpenAiReasoning(CompletionModel),
    RigAnthropic(CompletionModel),
    RigOllama(CompletionModel),
    RigGemini(CompletionModel),
    RigGroq(CompletionModel),
    RigXai(CompletionModel),
    RigZaiSdk(CompletionModel),
    RigNvidia(CompletionModel),
    VertexGemini(CompletionModel),
    Mock,
}
```

### 11.3 Model Capabilities

```rust
ModelCapabilities {
    supports_thinking_history: bool,  // 是否支持 extended thinking
    supports_temperature: bool,       // 是否支持 temperature 参数
}
```

### 11.4 Provider 特殊处理

| Provider | 特殊处理 |
|----------|----------|
| OpenAI | Web Search Preview tool, Reasoning effort, Responses API (rs_ IDs) |
| Anthropic | 签名 (signature) 用于 thinking history 恢复 |
| NVIDIA NIM | 系统提示移到 user message (API 兼容性) |
| OpenRouter | Provider preferences JSON |
| OpenAI Reasoning | o-series 模型不支持 temperature |

---

## 12. 系统提示 (System Prompt)

### 12.1 构建逻辑

```
build_system_prompt_with_contributions(workspace, mode, memory_file, registry, context):
  
  if provider == "openai":
    → build_codex_style_prompt() (Codex 风格, 更简洁)
  else:
    → 通用系统提示:
       1. Agent 身份和角色
       2. 安全声明 (仅协助授权的安全测试)
       3. 语气和风格指南
       4. 专业客观性
       5. 规划 (无时间估算)
       6. 任务管理 (update_plan 工具使用指南 + 示例)
       7. 提问指导
       8. 任务执行指南
       9. 项目指令 (CLAUDE.md / memory file)
       10. Agent mode 指令
```

### 12.2 Memory File (项目指令)

搜索优先级:
1. 用户配置的 `memory_file_path` (来自 codebase settings)
2. `{workspace}/CLAUDE.md`

---

## 13. Session 管理

### 13.1 文件结构

```
~/.golish/sessions/{session_id}/
  ├── metadata.json     # Session 元数据
  └── messages.json     # 消息历史

~/.golish/transcripts/{session_id}/
  ├── transcript.json   # 主 Agent 事件日志 (JSONL)
  └── sub-agent-*.json  # 子 Agent 内部事件日志

~/.golish/artifacts/
  ├── compaction/
  │   └── summarizer-input-{ts}.md  # Compaction 输入
  └── summaries/
      └── summary-{ts}.md          # Compaction 摘要
```

### 13.2 GolishSessionManager

```
GolishSessionManager:
  ├── create_session(workspace, mode)
  ├── save_messages(messages)
  ├── load_messages() → Vec<Message>
  ├── archive_session()
  └── list_sessions()
```

---

## 14. 前端架构

### 14.1 状态管理 (Zustand Store)

```
GolishState:
  ├── Sessions
  │   ├── sessions: Record<string, Session>
  │   ├── activeSessionId: string
  │   └── tabLayouts: Record<string, TabLayout>
  │
  ├── AI
  │   ├── aiConfig: AiConfig (provider, model, status)
  │   ├── streamingBlocks: Map<string, StreamingBlock[]>
  │   ├── processedToolRequests: Set<string>
  │   └── taskPlan: TaskPlan | null
  │
  ├── Agent Mode
  │   ├── agentMode: "default" | "auto-approve" | "planning"
  │   └── reasoningEffort: "low" | "medium" | "high"
  │
  ├── Slices:
  │   ├── ConversationSlice (per-session chat messages)
  │   ├── ContextSlice (token metrics, compaction info)
  │   ├── PanelSlice (sidebar, dashboard, targets)
  │   ├── AppearanceSlice (theme, font)
  │   ├── NotificationSlice
  │   └── GitSlice
  │
  └── Types:
      ├── SessionMode: "terminal" | "agent"
      ├── InputMode: "terminal" | "agent" | "auto"
      ├── RenderMode: "timeline" | "fullterm"
      ├── TabType: "terminal" | "settings" | "home" | "browser" | "security"
      └── AgentMode: "default" | "auto-approve" | "planning"
```

### 14.2 组件架构

```
App.tsx
  ├── TerminalPortalProvider
  │   ├── TabBar (标签栏 + 通知)
  │   ├── PaneContainer (分屏系统)
  │   │   └── PaneLeaf (单个面板)
  │   │       ├── UnifiedTimeline (主内容: 命令 + Agent 消息)
  │   │       │   ├── CommandBlock (命令历史块)
  │   │       │   ├── AgentChat (AI 聊天 UI)
  │   │       │   │   ├── ThinkingBlock
  │   │       │   │   ├── ToolCallDisplay
  │   │       │   │   ├── DiffView
  │   │       │   │   └── UdiffResultBlock
  │   │       │   └── WorkflowTree
  │   │       └── Terminal (xterm.js)
  │   ├── TerminalLayer (React portals, 状态持久化)
  │   ├── UnifiedInput (输入切换: terminal/agent)
  │   ├── AIChatPanel (AI 聊天面板)
  │   └── Settings (设置对话框)
  │       ├── AI Settings
  │       ├── Terminal Settings
  │       ├── Codebases
  │       └── Advanced
  │
  ├── HomeView (项目列表, 最近目录)
  ├── DashboardPanel
  ├── TargetPanel
  ├── PipelinePanel
  └── ProjectOverview
```

### 14.3 Terminal Portal 架构

```
Terminal 使用 React portals 跨面板持久化:

TerminalPortalProvider (根组件)
  ├── 维护 portal targets 注册表
  └── 为所有 Terminal 提供挂载目标

TerminalLayer (稳定位置渲染)
  ├── 为每个 session 创建 Terminal
  └── 使用 createPortal 将 Terminal 渲染到对应的 PaneLeaf

PaneLeaf
  ├── useTerminalPortalTarget(sessionId) → 注册 portal target DOM 元素
  └── 面板结构变化时, Terminal 实例保持不变 (仅 portal target 移动)
```

### 14.4 AI Event 处理

```
useAiEvents.ts:
  ├── 监听 Tauri "ai-event" 事件
  ├── 30+ 事件类型处理器
  ├── 更新 Zustand store
  └── 管理 streaming blocks

Event Handler Registry:
  ├── tool-handlers.ts  → ToolRequest, ToolResult, ToolApprovalRequest, etc.
  └── registry.ts       → 所有其他事件类型的处理器
```

### 14.5 Fullterm Mode (全终端模式)

```
自动检测:
  ├── CSI ESC[?1049h → 进入 alternate screen → 切换到 fullterm
  └── CSI ESC[?1049l → 退出 alternate screen → 切换回 timeline

回退列表 (不使用 alternate screen 的应用):
  ├── 内置: claude, cc, codex, cdx, aider, cursor, gemini
  └── 自定义: settings.toml [terminal] fullterm_commands = [...]
```

---

## 15. 配置系统

### 15.1 Settings 文件

```
~/.golish/settings.toml (自动生成)

[ai]
provider = "anthropic_vertex"
model = "claude-sonnet-4-20250514"
summarizer_model = "claude-3-5-haiku-latest"

[terminal]
fullterm_commands = []

[tools]
web_search = true

[context]
compaction_threshold = 0.80
warning_threshold = 0.80
critical_threshold = 0.90

[db]  # NEW
url = "postgresql://..."
```

### 15.2 环境变量

| 变量 | 描述 |
|------|------|
| `TAVILY_API_KEY` | Web search API key |
| `VT_SESSION_DIR` | Session 存储路径覆盖 |
| `QBIT_WORKSPACE` | 工作区路径覆盖 |

---

## 16. 可观测性 (Observability)

### 16.1 Langfuse 集成 (via OpenTelemetry)

```
Trace 层级:
  chat_message (root trace)
    └── agent (agent observation)
        ├── llm_completion (generation observation)
        │   ├── gen_ai.request.model
        │   ├── gen_ai.usage.prompt_tokens
        │   ├── gen_ai.usage.completion_tokens
        │   └── gen_ai.completion
        ├── tool_call (tool observation)
        │   ├── tool_name
        │   ├── tool_args
        │   └── tool_result
        └── sub_agent (agent observation)
            ├── llm_completion
            └── tool_call
```

### 16.2 Span 属性

每个 span 包含:
- `langfuse.observation.type` = "generation" | "agent" | "tool" | "event"
- `langfuse.session.id` = session ID
- `langfuse.observation.input` / `.output`
- `gen_ai.operation.name` = "chat_completion"
- `gen_ai.request.model` / `gen_ai.system`

---

## 17. 安全特性

### 17.1 Agent 安全声明

系统提示中包含安全限制:
- 仅协助**授权的**安全测试、防御安全、CTF、教育
- 拒绝破坏性技术、DoS、批量攻击、供应链攻击、恶意规避
- 双用途工具 (C2, credential testing, exploit dev) 需要明确授权上下文

### 17.2 HITL 控制

- 默认模式下, 所有修改操作需要用户审批
- `auto-approve` 模式需要用户主动启用
- `planning` 模式限制为只读操作

### 17.3 渗透测试工具集成 (NEW)

```
pentest_* 工具:
  ├── 通过 ToolRegistry 动态注册
  ├── 始终包含在 Agent 工具列表中 (不受 tool_config 过滤)
  └── 包括: 扫描器, 目标管理, 发现, 笔记, 审计, 漏洞情报等
```

---

## 18. 数据库集成 (golish-db, NEW)

### 18.1 架构

```
golish-db:
  ├── config.rs     → 数据库配置
  ├── pool.rs       → 连接池管理
  ├── embedded.rs   → 嵌入式模式
  ├── models.rs     → 数据模型
  ├── embeddings.rs → 向量嵌入
  └── repo/         → Repository 层
      ├── audit.rs       → 审计日志
      ├── findings.rs    → 发现
      ├── memories.rs    → 记忆 (长期)
      ├── message_chains.rs → 消息链
      ├── methodology.rs → 方法论
      ├── notes.rs       → 笔记
      ├── pipelines.rs   → 管道
      ├── search_logs.rs → 搜索日志
      ├── sessions.rs    → 会话
      ├── targets.rs     → 目标
      ├── tasks.rs       → 任务
      ├── terminal_logs.rs → 终端日志
      ├── tool_calls.rs  → 工具调用记录
      ├── topology.rs    → 拓扑
      ├── vault.rs       → 密钥保管
      └── vuln_intel.rs  → 漏洞情报
```

### 18.2 Migrations

```sql
20260408000001_initial.sql       → 基础表
20260408000002_recordings.sql    → 录制功能
20260409000001_enhance_memories.sql → 增强记忆
20260409000002_target_status.sql → 目标状态
20260409000003_operation_source.sql → 操作来源
```

### 18.3 DbTracker (后台数据库记录)

```rust
DbTracker:
  ├── record_tool_call(session_id, tool_name, args, result, duration)
  ├── record_token_usage(session_id, provider, model, input, output)
  ├── audit(action, source, detail)
  └── 所有操作异步后台执行 (不阻塞 Agent)
```

---

## 19. 与 PentAGI 的对比

| 维度 | PentAGI | Golish |
|------|---------|--------|
| **架构** | Web 服务 (Docker Compose) | 桌面应用 (Tauri 2) |
| **语言** | Go (后端) | Rust (后端) |
| **Agent 数量** | 13 种专门 Agent | 3 种子 Agent (worker, coder, analyzer) + 主 Agent |
| **任务分解** | Flow→Task→Subtask 层级 | 单一 Agent Loop + Plan 工具 |
| **规划** | Generator + Refiner Agent | update_plan 工具 |
| **工具数量** | 44+ | 20+ |
| **Docker** | 必需 (沙箱执行) | 可选 (直接宿主机执行) |
| **数据库** | PostgreSQL + pgvector (必需) | PostgreSQL (可选, 新增) |
| **向量搜索** | pgvector + LangChain | 无 (计划中) |
| **知识图谱** | Neo4j/Graphiti | 无 |
| **错误恢复** | 4 层 (重试+ToolCallFixer+Reflector+Chain一致性) | 2 层 (stream 重试+loop detection) |
| **上下文管理** | Chain Summarization | Context Compaction (Summarizer Agent) |
| **搜索引擎** | 8 种 (Google, DDG, Tavily, etc.) | 2 种 (Tavily, web_fetch) |
| **渗透测试** | 核心功能 (完整集成) | 正在集成 (pentest_* 工具, recon_basic 工作流) |
| **可观测性** | OpenTelemetry + Grafana Stack | OpenTelemetry + Langfuse |
| **实时通信** | GraphQL Subscriptions | Tauri Events (ai-event) |
| **前端** | React + Apollo Client | React + Zustand + Tauri invoke |

---

## 20. 关键常量和限制

| 参数 | 值 | 位置 |
|------|-----|------|
| `MAX_TOOL_ITERATIONS` | 100 | agentic_loop.rs |
| `APPROVAL_TIMEOUT_SECS` | 300 (5 min) | agentic_loop.rs |
| `ASK_HUMAN_TIMEOUT_SECS` | 600 (10 min) | tool_executors.rs |
| `MAX_COMPLETION_TOKENS` | 10,000 | agentic_loop.rs |
| `STREAM_START_MAX_ATTEMPTS` | 3 | agentic_loop.rs |
| `STREAM_START_RETRY_BASE_DELAY_MS` | 300 | agentic_loop.rs |
| `STREAM_START_RETRY_MAX_DELAY_MS` | 3,000 | agentic_loop.rs |
| Stream Timeout | 180s (3 min) | agentic_loop.rs |
| `MAX_AGENT_DEPTH` | 5 | golish-sub-agents |
| Sub-agent timeout | 600s (10 min) | executor.rs |
| Sub-agent idle timeout | 180s (3 min) | executor.rs |
| Worker max_iterations | 50 | defaults.rs |
| Compaction threshold | 80% | context_manager.rs |
| Warning threshold | 80% | context_manager.rs |
| Critical threshold | 90% | context_manager.rs |
| Default context window | 200,000 tokens | context_manager.rs |
| Temperature | 0.3 | agentic_loop.rs |
| Loop detection threshold | 3 (warning), 5 (break) | loop_detection |

---

## 21. 运行模式

### 21.1 GUI 模式 (默认)

```bash
golish              # 启动 GUI
golish ~/Code/foo   # 打开指定目录
```

### 21.2 CLI 模式

```bash
golish --headless           # 无头 CLI 模式
golish -e "prompt"          # 执行单条提示 (隐含 --headless)
golish -e "prompt" --auto-approve  # 自动审批所有工具
```

### 21.3 Dev 模式

```bash
just dev           # 完整应用 (当前目录)
just dev ~/Code/foo # 完整应用 (指定目录)
just dev-fe        # 仅前端 (Vite, port 1420)
```

---

## 22. 文件系统布局

```
~/.golish/
  ├── settings.toml         # 全局配置
  ├── sessions/             # 会话持久化
  ├── transcripts/          # Agent 事件转录
  ├── artifacts/            # 工件 (compaction, summaries)
  ├── projects/             # 项目配置 (TOML)
  ├── skills/               # 全局 Agent Skills
  ├── frontend.log          # 前端日志
  └── backend.log           # 后端日志

{project}/.golish/
  ├── skills/               # 项目级 Agent Skills (覆盖全局)
  └── prompts/              # 项目级 Prompts (覆盖 skills)
```
