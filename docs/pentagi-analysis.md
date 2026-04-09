# PentAGI 完整架构与流程分析

> 本文档是对 PentAGI (Penetration Testing Artificial General Intelligence) 项目的完整技术分析。
> 目的是作为 golish-platform 项目借鉴的参考资料。

---

## 1. 项目概览

PentAGI 是一个 **AI 驱动的自动化渗透测试平台**，采用多智能体（Multi-Agent）架构，在 Docker 沙箱中自主执行安全测试。

| 维度 | 技术选型 |
|------|---------|
| 后端 | Go (REST + GraphQL via gqlgen) |
| 前端 | React + TypeScript + Apollo Client |
| 数据库 | PostgreSQL + pgvector (向量搜索) |
| 知识图谱 | Neo4j (via Graphiti, 可选) |
| 容器化 | Docker (SDK 直接调用，非 K8s) |
| 可观测性 | OpenTelemetry → VictoriaMetrics + Loki + Jaeger → Grafana |
| LLM 分析 | Langfuse |
| 实时通信 | GraphQL Subscriptions (WebSocket) |

---

## 2. 核心概念层级

```
Flow (渗透测试会话)
 └── Task (用户定义的目标, 一个 Flow 可有多个 Task)
      └── Subtask (系统自动分解的步骤, 最多 15 个)
           └── Action (Agent 执行的具体操作)
                └── Artifact / Memory (产出物 / 记忆)
```

### 2.1 Worker 体系

| Worker | 职责 |
|--------|------|
| **FlowWorker** | 管理 Flow 完整生命周期，协调 Task |
| **TaskWorker** | 执行 Task，管理 Subtask 生成与精炼 |
| **SubtaskWorker** | 通过 AI Agent 执行具体 Subtask |
| **AssistantWorker** | 管理 Flow 内的交互式助手模式 |

### 2.2 Provider 体系

| Provider | 职责 |
|----------|------|
| **ProviderController** | 工厂模式创建/管理不同 LLM Provider |
| **FlowProvider** | Flow 执行的核心接口，Agent 协调与编排 |
| **AssistantProvider** | 助手模式的专用 Provider |

---

## 3. AI Agent 完整体系 (13 种)

### 3.1 核心编排 Agent

| Agent | 类型 | 职责 | 最大迭代 |
|-------|------|------|----------|
| **Primary Agent** | 通用 | 主编排者，协调所有其他 Agent | 100 |
| **Assistant Agent** | 通用 | 交互式助手，`UseAgents` 控制委托行为 | 100 |

### 3.2 任务规划 Agent

| Agent | 类型 | 职责 | 最大迭代 |
|-------|------|------|----------|
| **Generator Agent** | 有限 | 将 Task 分解为 Subtask 列表(最多 15) | 20 |
| **Refiner Agent** | 有限 | 每个 Subtask 完成后审查/更新计划 | 20 |
| **Reporter Agent** | 有限 | 生成最终综合报告 | 20 |

### 3.3 专家 Agent

| Agent | 类型 | 职责 | 可用工具 | 最大迭代 |
|-------|------|------|----------|----------|
| **Pentester** | 通用 | 渗透测试和漏洞评估 | terminal, file, browser, guide, sploitus, graphiti + 委托 Agent | 100 |
| **Coder** | 通用 | 编写和维护代码 | browser, code search/store, graphiti + 委托 Agent | 100 |
| **Installer** | 通用 | 环境设置和工具安装 | terminal, file, browser, guide + 委托 Agent | 100 |
| **Memorist** | 有限 | 长期记忆存储和检索 | terminal, file, memory, graphiti | 20 |
| **Searcher** | 有限 | 互联网搜索和信息收集 | browser, 所有搜索引擎, memory | 20 |
| **Enricher** | 有限 | 从多个来源增强信息 | terminal, file, memory, graphiti, browser | 20 |
| **Adviser** | 有限 | 提供专家指导和建议 | (SimpleChain, 无工具) | 20 |

### 3.4 元 Agent

| Agent | 类型 | 职责 |
|-------|------|------|
| **Reflector** | 有限 | 纠正返回非结构化文本的 Agent，强制其使用工具调用 |
| **Summarizer** | 有限 | 上下文摘要，防止上下文窗口溢出 |
| **Tool Call Fixer** | 有限 | 修复无效的 JSON 工具调用参数 |
| **Mentor** (via Adviser) | 有限 | 执行监控，检测停滞并提供建议 |
| **Planner** (via Adviser) | 有限 | 为专家 Agent 生成结构化执行计划 |

---

## 4. 工具体系 (44+ 工具, 7 种类型)

### 4.1 工具类型分类

| 类型 | 名称 | 描述 |
|------|------|------|
| **Environment** | `terminal`, `file` | Docker 容器内命令执行和文件操作 |
| **SearchNetwork** | `browser`, `google`, `duckduckgo`, `tavily`, `traversaal`, `perplexity`, `searxng`, `sploitus` | 外部信息源搜索 |
| **SearchVectorDb** | `search_in_memory`, `search_guide`, `search_answer`, `search_code`, `graphiti_search` | 向量数据库语义搜索 |
| **Agent** | `search`, `maintenance`, `coder`, `pentester`, `advice`, `memorist` | 委托给专家 Agent |
| **StoreAgentResult** | `maintenance_result`, `code_result`, `hack_result`, `memorist_result`, `search_result`, `enricher_result`, `report_result`, `subtask_list`, `subtask_patch` | 存储 Agent 结果 |
| **StoreVectorDb** | `store_guide`, `store_answer`, `store_code` | 存储到向量数据库 |
| **Barrier** | `done`, `ask` | 控制流终止（完成/请求用户输入） |

### 4.2 各 Agent 可用工具映射

```
Primary Agent:
  done, ask(可选), advice, coder, maintenance, memorist, pentester, search

Pentester Agent:
  hack_result(barrier), advice, coder, maintenance, memorist, search,
  terminal, file, browser, store_guide, search_guide, graphiti_search, sploitus

Coder Agent:
  code_result(barrier), advice, maintenance, memorist, search,
  browser, search_code, store_code, graphiti_search

Installer Agent:
  maintenance_result(barrier), advice, memorist, search,
  terminal, file, browser, store_guide, search_guide

Searcher Agent:
  search_result(barrier), memorist,
  browser, google, duckduckgo, tavily, traversaal, perplexity, searxng, sploitus,
  search_answer, store_answer

Memorist Agent:
  memorist_result(barrier), terminal, file, search_in_memory, graphiti_search

Generator Agent:
  subtask_list(barrier), memorist, search, terminal, file, browser

Refiner Agent:
  subtask_patch(barrier), memorist, search, terminal, file, browser

Enricher Agent:
  enricher_result(barrier), terminal, file, search_in_memory, graphiti_search, browser

Assistant Agent (UseAgents=true):
  terminal, file, browser, advice, coder, maintenance, memorist, pentester, search

Assistant Agent (UseAgents=false):
  terminal, file, browser, google, duckduckgo, tavily, traversaal, perplexity, searxng, sploitus,
  search_in_memory, search_guide, search_answer, search_code
```

### 4.3 Barrier 工具详解

Barrier 工具是一种特殊工具，调用后会**终止当前 Agent Chain 的执行循环**：

- `done` — 完成当前 Subtask
- `ask` — 暂停执行，等待用户输入（通过 `ASK_USER` 环境变量控制）
- 各种 `*_result` 工具也是 Barrier，用于子 Agent 返回结果给父 Agent

---

## 5. 完整执行流程

### 5.1 Flow 创建流程

```
用户提交请求
  → FlowController 创建 FlowWorker
    → FlowProvider 初始化:
      1. Image Chooser Agent (选择 Docker 镜像)
      2. Language Chooser Agent (检测用户语言)
      3. Flow Descriptor Agent (生成 Flow 标题)
    → 创建 FlowToolsExecutor
    → Docker: 启动主容器 (kali-linux 或其他)
    → 启动 worker goroutine (后台事件循环)
    → PutInput(用户输入) → 开始处理
```

### 5.2 Task 执行流程

```
FlowWorker.processInput(用户输入):
  1. 检查是否有等待中的 Task → 如果有, PutInput 并继续
  2. 否则创建新 Task:
     a. GetTaskTitle (LLM 生成标题)
     b. 创建 TaskWorker
     c. GenerateSubtasks (Generator Agent):
        - 分析任务需求
        - 生成 Subtask 列表 (via subtask_list barrier tool)
        - 存储到数据库
  3. Task.Run():
     loop {
       a. 弹出下一个未完成的 Subtask
       b. SubtaskWorker.Run():
          - PrepareAgentChain (创建/恢复消息链)
          - PerformAgentChain (执行 Primary Agent)
       c. 根据结果:
          - Done → Subtask 完成
          - Ask → 暂停等待用户输入
          - Error → 标记失败
       d. RefineSubtasks (Refiner Agent):
          - 审查已完成的 Subtask
          - 更新/删除/添加计划中的 Subtask
     }
  4. Task 完成后:
     - Reporter Agent 生成最终报告
     - 更新 Flow 状态为 Waiting (等待新输入)
```

### 5.3 Agent Chain 执行循环 (核心)

这是整个系统最核心的循环，在 `performer.go` 的 `performAgentChain` 中实现：

```
performAgentChain(chain, executor):
  for iteration in [0, maxCallsLimit):

    // 1. 接近限制时优雅终止
    if iteration >= maxCallsLimit - 3:
      result = "即将达到迭代限制, 请使用 barrier 工具完成"
    else:
      // 2. 调用 LLM (带重试)
      result = callWithRetries(chain, executor)
        // 最多重试 3 次, 每次间隔 5 秒
        // 如果 3 次都失败 → 调用 Reflector (CallerReflector)

    // 3. 没有工具调用?
    if len(result.funcCalls) == 0:
      if agent 是 Assistant:
        return processAssistantResult(result)  // 直接返回文本
      else:
        // 调用 Reflector (最多 3 次)
        result = performReflector(chain, result)

    // 4. 存储到 Graphiti (如果启用)
    storeAgentResponseToGraphiti(result)

    // 5. 将 AI 消息添加到 chain
    chain.append(AI message with reasoning + tool calls)
    updateMsgChain(chain)

    // 6. 执行每个工具调用
    for each toolCall in result.funcCalls:
      // 重复检测
      if detector.detect(toolCall) && count >= threshold:
        return "tool repeating, aborting"

      // 执行工具 (带重试和参数修复)
      response = executor.Execute(toolCall)
        // 失败 → ToolCallFixer Agent 修复参数 → 重试 (最多 3 次)

      // 执行监控 (Mentor)
      if monitor.shouldInvokeMentor(toolCall):
        mentorResponse = performMentor(...)
        response = formatEnhanced(response, mentorResponse)

      // 工具结果添加到 chain
      chain.append(Tool response)
      updateMsgChain(chain)

      // Barrier 工具 → 终止循环
      if executor.IsBarrierFunction(funcName):
        wantToStop = true

    if wantToStop: return

    // 7. Chain 摘要 (防止上下文溢出)
    if summarizer != nil:
      chain = summarizer.SummarizeChain(chain)
```

### 5.4 工具执行流程

```
customExecutor.Execute(streamID, id, name, thinking, args):
  1. 查找 handler
  2. 创建 Observation (Langfuse):
     - Environment/SearchNetwork/StoreResult → Tool Observation
     - Agent → Agent Observation
     - Barrier → Span Observation
     - SearchVectorDb → NoOp (内部创建)
  3. 提取 message 字段 → PutMsg (消息日志)
  4. CreateToolcall (数据库记录)
  5. 执行 handler:
     a. 获取结果
     b. UTF-8 消毒
     c. 如果结果 > 16KB 且是 terminal/browser:
        → Summarizer Agent 生成摘要
     d. 如果结果 > 32KB 且无 summarizer:
        → 截断 (前 16KB + ... + 后 16KB)
     e. 更新 toolcall 记录
  6. 自动存储到向量数据库 (如果工具在允许列表中, 18 种)
  7. 更新消息结果
```

---

## 6. 消息链（Message Chain）管理

### 6.1 Chain 类型 (14 种)

| 类型 | Agent |
|------|-------|
| `PrimaryAgent` | 主编排 Agent |
| `Generator` | Subtask 生成 |
| `Refiner` | Subtask 精炼 |
| `Reporter` | 最终报告生成 |
| `Coder` | 代码开发 |
| `Pentester` | 安全测试 |
| `Installer` | 基础设施维护 |
| `Memorist` | 记忆操作 |
| `Searcher` | 信息检索 |
| `Adviser` | 专家咨询 |
| `Reflector` | 响应纠正 |
| `Enricher` | 上下文增强 |
| `Assistant` | 交互式助手 |
| `Summarizer` | 上下文摘要 |
| `ToolCallFixer` | 工具参数修复 |

### 6.2 Chain 生命周期

1. **创建**: `restoreChain()` — 如果已有 chain 则恢复，否则创建新的
2. **更新**: 每次 LLM 调用和工具执行后更新
3. **摘要**: 超过 token 限制时 Summarizer 自动压缩
4. **持久化**: JSON 序列化后存储在 PostgreSQL
5. **恢复**: 系统重启后从数据库恢复

### 6.3 Chain AST (抽象语法树)

PentAGI 使用自定义的 Chain AST 来结构化分析消息链：

```go
type ChainAST struct {
    Sections []Section  // 以 Human 消息为分隔的段落
}

type Section struct {
    Header     SectionHeader      // System + Human 消息
    Iterations []AgentIteration   // AI 消息 + Tool 调用/响应
}
```

用途：
- 恢复 chain 时确保一致性
- 为 Reflector 提取最后的 Human 消息
- Summarization 时结构化处理

---

## 7. 向量数据库 (RAG) 体系

### 7.1 存储类型

| doc_type | 描述 | 子类型 |
|----------|------|--------|
| `memory` | 工具执行结果和 Agent 观察 | `tool_name` 标签 |
| `guide` | 安装和配置程序 | install/configure/use/troubleshoot 等 |
| `answer` | Q&A 对 | guide/vulnerability/code/tool/other |
| `code` | 代码样本 | `code_lang` (python/bash 等) |

### 7.2 搜索参数

- **相似度阈值**: 0.2
- **结果限制**: 每次搜索最多 3 个文档
- **自动存储工具**: 18 种工具自动将结果存入向量数据库

### 7.3 文本分块

```go
textsplitter.NewRecursiveCharacter(
    ChunkSize: 2000,
    ChunkOverlap: 100,
    CodeBlocks: true,
    HeadingHierarchy: true,
)
```

---

## 8. 错误处理与恢复机制 (4 层)

### 第 1 层: 工具调用重试
- LLM 调用失败 → 最多重试 3 次，间隔 5 秒

### 第 2 层: Tool Call Fixer
- 工具参数 JSON 无效 → ToolCallFixer Agent 使用 schema 修复
- 最多重试 3 次

### 第 3 层: Reflector 纠正
- Agent 返回文本而非工具调用 → Reflector 提供纠正指导
- 最多 3 次 Reflector 迭代
- 有递归保护 (CallerReflector 只调用一次)

### 第 4 层: Chain 一致性
- 系统中断后 → AST 分析未响应的工具调用
- 添加默认响应内容，保持消息链完整

### 重复检测
- 连续相同工具调用超过 3 次 → 告知 Agent 换一个工具
- 超过 3+4 次 → 直接中止 chain

### 执行监控 (Mentor)
- 同一工具连续 5 次 → 触发 Mentor 审查
- 总共 10 次工具调用 → 触发 Mentor 审查
- 通过 `EXECUTION_MONITOR_ENABLED` 环境变量控制

---

## 9. 多 LLM Provider 支持

### 9.1 支持的 Provider (10+)

| Provider | 类型 |
|----------|------|
| OpenAI | 商用 API |
| Anthropic | 商用 API |
| Google Gemini | 商用 API |
| AWS Bedrock | 云服务 |
| DeepSeek | 商用 API |
| GLM (智谱 AI) | 商用 API |
| Kimi (月之暗面) | 商用 API |
| Qwen (阿里云) | 商用 API |
| Ollama | 本地部署 |
| Custom | 自定义 HTTP 端点 |
| 聚合器 | OpenRouter, DeepInfra |

### 9.2 Provider 配置

每个 Provider 为每种 Agent 类型定义独立的模型配置：

```yaml
# config.yml
primary_agent:
  model: "claude-3-5-sonnet"
  temperature: 0.3
  max_tokens: 8192

simple:
  model: "claude-3-5-haiku"
  temperature: 0.0
  max_tokens: 4096

# ... 13 种 Agent 类型的独立配置
```

---

## 10. Docker 容器管理

### 10.1 容器生命周期

```
Flow 创建 → Image Chooser 选择镜像
  → 启动主容器 (Primary Container)
    - 入口: tail -f /dev/null
    - 能力: NET_RAW + NET_ADMIN(可选)
    - 工作目录: /work
    - 端口: 28000 + flowID*2 (2 个端口)
  → 工具执行在容器内进行
  → Flow 结束 → 容器清理
```

### 10.2 安全隔离

- 每个 Flow 独立容器
- 沙箱化命令执行
- Web 抓取使用独立的 Scraper 容器
- 自动敏感数据脱敏 (Anonymizer)

---

## 11. 实时通信系统

### 11.1 GraphQL Subscriptions

| 事件类型 | 描述 |
|----------|------|
| FlowCreated/Updated | Flow 状态变化 |
| TaskCreated/Updated | Task 状态变化 |
| AgentLogAdded | Agent 间委托日志 |
| MessageLogAdded/Updated | 消息日志(含流式) |
| TerminalLogAdded | 终端命令输出 |
| SearchLogAdded | 搜索日志 |
| VectorStoreLogAdded | 向量存储操作日志 |
| ScreenshotAdded | 浏览器截图 |
| AssistantLogAdded/Updated | 助手交互日志 |

### 11.2 流式响应

```
StreamMessageChunk:
  Type: Thinking | Content | Update | Flush | Result
  MsgType: Answer | Terminal | Search | ...
  StreamID: 唯一标识符 (原子递增)
  Content: 文本内容
  Thinking: 推理内容
```

---

## 12. 日志系统 (7 层)

| 控制器 | Worker | 数据库表 | 描述 |
|--------|--------|----------|------|
| MsgLogController | FlowMsgLogWorker | msglogs | 用户交互消息 |
| AgentLogController | FlowAgentLogWorker | agentlogs | Agent 委托 (Initiator→Executor) |
| SearchLogController | FlowSearchLogWorker | searchlogs | 搜索引擎 + 查询 + 结果 |
| TermLogController | FlowTermLogWorker | termlogs | 终端 Stdin/Stdout |
| VectorStoreLogController | FlowVectorStoreLogWorker | vecstorelogs | 向量存储检索/存储 |
| ScreenshotController | FlowScreenshotWorker | screenshots | 浏览器截图 |
| AssistantLogController | FlowAssistantLogWorker | assistantlogs | 助手交互会话 |

---

## 13. 数据模型 (简化)

### 核心实体关系

```
Flow (1) ──< Task (N)
Task (1) ──< Subtask (N)
Subtask (1) ──< Action (N)
Action (1) ──< Artifact / Memory (N)

Flow (1) ──< Container (N)
Flow (1) ──< MsgChain (N)
Flow (1) ──< Assistant (N)
Flow (1) ──< MsgLog / AgentLog / SearchLog / TermLog / VecStoreLog / Screenshot (N)
```

### Flow 状态机

```
Created → Running → Waiting → Running → Finished
                 ↘ Failed
```

### Task 状态机

```
Created → Running → Waiting → Running → Finished
                 ↘ Failed
```

### Subtask 状态机

```
Created → Running → Waiting → Running → Finished
                 ↘ Failed
```

---

## 14. Chain Summarization 机制

### 14.1 触发条件
- 消息链 token 数接近模型上下文窗口限制

### 14.2 摘要类型
1. **Tool Call Summary** — AI 消息只包含 `SummarizationToolName` 工具调用
2. **Prefixed Summary** — AI 消息以 `SummarizedContentPrefix` 开头

### 14.3 Agent 处理规则
- 将摘要视为**历史记录**
- 提取有用信息指导当前策略
- **永远不要模仿**摘要格式
- 继续使用结构化工具调用

---

## 15. 前端架构

### 15.1 技术栈
- React + TypeScript + Vite
- Apollo Client (GraphQL, 状态管理)
- GraphQL Subscriptions (实时更新)
- Radix UI + shadcn/ui (组件库)
- Monaco Editor (终端显示)

### 15.2 页面结构

```
src/
├── pages/
│   ├── flows/        # Flow 管理 (列表/详情/新建/报告)
│   ├── dashboard/    # 仪表盘 (概览/分析)
│   ├── settings/     # 设置 (Provider/Prompt/API Token/MCP)
│   └── templates/    # Flow 模板
├── features/flows/
│   ├── agents/       # Agent 显示
│   ├── dashboard/    # Flow 仪表盘
│   ├── messages/     # 消息显示
│   ├── screenshots/  # 截图显示
│   ├── tasks/        # Task/Subtask 显示
│   ├── terminal/     # 终端显示
│   ├── tools/        # 工具调用显示
│   └── vector-stores/ # 向量存储日志
├── providers/        # React Context Provider
│   ├── flow-provider.tsx
│   ├── flows-provider.tsx
│   └── user-provider.tsx
└── graphql/          # 自动生成的 GraphQL 类型
```

### 15.3 实时数据流

```
后端 Agent 执行
  → GraphQL Subscription 推送
    → Apollo Client 缓存更新
      → React 组件自动重渲染
```

---

## 16. 认证体系

| 方式 | 描述 |
|------|------|
| Session Cookie | 浏览器登录 (secure, httpOnly) |
| OAuth2 | Google / GitHub |
| Bearer Token | API Token (编程访问) |

---

## 17. 搜索引擎优先级

Searcher Agent 按以下优先级检索信息：

1. **Priority 1-2: 内存工具** — 始终先检查内部知识
   - `search_answer` — 已有 Q&A 知识
   - `memorist` — Task/Subtask 执行历史
2. **Priority 3-4: 侦察工具** — 快速源发现
   - `google` / `duckduckgo` — 快速链接收集
   - `browser` — 定向内容提取
3. **Priority 5: 深度分析** — 复杂研究综合
   - `traversaal` — 结构化 Q&A
   - `tavily` — 研究级探索
   - `perplexity` — AI 增强的综合分析

**动作经济**: 每次查询最多 3-5 次搜索动作。

---

## 18. 与 Golish 的对比和借鉴价值

### 18.1 可直接借鉴的设计

| PentAGI 设计 | Golish 对应 | 借鉴价值 |
|-------------|------------|----------|
| Flow → Task → Subtask 层级 | 目前无类似层级 | 可为自动化渗透测试加入任务分解 |
| Generator + Refiner 模式 | 无 | 动态规划和迭代精炼 Subtask |
| 多 Agent 委托体系 | golish-sub-agents | 更细粒度的专家 Agent 分工 |
| Reflector Agent | 无 | 自动纠正 Agent 错误输出 |
| 向量数据库长期记忆 | 无 (计划中的 golish-db) | 渗透测试知识累积 |
| 重复检测 + 执行监控 | golish-ai 的 loop detection | PentAGI 的 Mentor 模式更完善 |
| Docker 沙箱执行 | golish-shell-exec | 可增加容器化隔离 |
| Chain Summarization | golish-ai 的 context compaction | 类似机制，可借鉴 AST 方法 |
| Barrier 工具模式 | golish-ai 的 HITL | 更结构化的控制流 |
| 7 层日志系统 | golish-session | 更细粒度的日志分类 |
| 搜索优先级策略 | golish-web | 可增加优先级和去重 |
| Graphiti 知识图谱 | 无 | 实体关系追踪 |
| 自动敏感数据脱敏 | 无 | Anonymizer 模式 |
| Tool Call Fixer | 无 | 自动修复无效 JSON |

### 18.2 架构差异

| 维度 | PentAGI | Golish |
|------|---------|--------|
| 架构 | Web 服务 (Docker Compose) | 桌面应用 (Tauri) |
| 后端语言 | Go | Rust |
| 状态管理 | PostgreSQL + GraphQL Subscriptions | 文件系统 + Zustand + Tauri Events |
| 容器化 | Docker SDK 直接调用 | 直接在宿主机执行 |
| Agent 通信 | 数据库持久化 + Chain | 内存中的消息传递 |
| 前端 | Apollo Client (GraphQL) | Zustand + Tauri invoke |

---

## 19. 关键配置参数

| 参数 | 默认值 | 描述 |
|------|--------|------|
| `MAX_GENERAL_AGENT_TOOL_CALLS` | 100 | 通用 Agent 最大工具调用 |
| `MAX_LIMITED_AGENT_TOOL_CALLS` | 20 | 有限 Agent 最大工具调用 |
| `EXECUTION_MONITOR_ENABLED` | false | 执行监控开关 |
| `EXECUTION_MONITOR_SAME_TOOL_LIMIT` | 5 | 同一工具连续调用阈值 |
| `EXECUTION_MONITOR_TOTAL_TOOL_LIMIT` | 10 | 总工具调用阈值 |
| `AGENT_PLANNING_STEP_ENABLED` | false | 任务规划开关 |
| `ASK_USER` | false | 用户交互工具开关 |
| `DOCKER_NET_ADMIN` | false | Docker NET_ADMIN 能力 |
| Terminal 默认超时 | 5 分钟 | 终端命令超时 |
| Terminal 硬限制 | 20 分钟 | 终端命令最大超时 |
| 向量搜索阈值 | 0.2 | 相似度最低阈值 |
| 向量搜索限制 | 3 | 每次搜索最多结果数 |
| Subtask 最大数量 | 15 | 每个 Task 最多 Subtask |
| Reflector 最大迭代 | 3 | 每个 Chain 最多反射次数 |
| LLM 调用重试 | 3 | 每次 LLM 调用最大重试 |
| 重试间隔 | 5 秒 | LLM 调用重试间隔 |
| 工具结果摘要阈值 | 16KB | 超过此值自动摘要 |

---

## 20. 部署架构

```
Docker Compose:
  ├── pentagi          (Go 后端 + React 前端)
  ├── pgvector         (PostgreSQL + pgvector)
  ├── scraper          (Web 抓取器)
  ├── searxng          (元搜索引擎, 可选)
  ├── neo4j            (知识图谱, 可选)
  ├── graphiti         (Graphiti API, 可选)
  ├── langfuse-*       (LLM 分析, 可选)
  │   ├── langfuse-server
  │   ├── langfuse-worker
  │   ├── langfuse-clickhouse
  │   ├── langfuse-redis
  │   └── langfuse-minio
  └── observability    (监控, 可选)
      ├── otel-collector
      ├── victoriametrics
      ├── loki
      ├── jaeger
      └── grafana
```
