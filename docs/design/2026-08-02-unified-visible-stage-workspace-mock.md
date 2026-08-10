# 统一可见 Stage Workspace 实体 Mock

## 问题

当前 Company-scoped `stage_run` 详情以阶段特化的 Controller 汇总卡为入口。用户需要先看到一组状态卡，再逐层进入 Controller 和 Worker；不同阶段又存在不同的特化视图。这个结构无法直接回答三个产品问题：

1. 当前阶段有哪些真实 LLM Agent、确定性任务和依赖关系？
2. 选中的 Agent 正在说什么、调用什么工具、产生什么 artifact？
3. 阶段覆盖、Gate、证据与 Agent 对话如何在一个上下文中对应？

用户确认的目标不是只重做 Enumeration，而是让所有阶段的详情共享同一套 Workspace：左侧是 Company/Agent 拓扑，右侧是当前 Agent 的可见对话，阶段进度和 artifact/evidence 保持在同一页面。Controller 只是拓扑中的一个节点，不再作为整个详情页的产品模型。

## 本轮决策

### 1. 先交付真实 React mock，不修改生产运行链

本轮只在现有 `ComponentTestbed` 中挂载一个可交互的实体 mock。它使用仓库真实 React、Tailwind 和 UI token，可以在 `just dev` 或 `just dev-fe` 中直接评审；不接 Tauri IPC、数据库、扫描器、真实 provider 或 Stage Team scheduler。

入口保持现有路径：

```text
Cmd/Ctrl+K → Component Testbed → Unified Stage Workspace
```

本轮不替换 `ToolCallDetailView`、`StageTeamRunView` 或 `SubAgentDetailView` 的生产分支。UI 获批后再单独实现 adapter 和生产路由，避免在视觉结构未定稿前碰共享 dirty runtime。

### 2. 所有阶段共享一个纯展示模型

Mock 使用纯前端 `StageWorkspaceSnapshot`，不让通用组件出现 `isEnumeration`、`isVulnStage` 等不断扩张的条件分支。

```text
StageWorkspaceSnapshot
├─ stage：名称、状态、Gate、全局进度
├─ metrics：当前阶段最重要的 4 个覆盖维度
├─ companies：公司和 origin/group
├─ agents：Controller、LLM specialist、deterministic task
├─ conversations：公开 narration、工具调用、artifact
└─ artifacts：asset / endpoint / application_fact / vulnerability / verification
```

阶段只替换 fixture 数据和 artifact renderer，页面框架保持一致。首个 mock 覆盖：

- Recon
- Enumeration
- Application Understanding
- Vulnerability
- Verification

### 3. 统一 Workspace 结构

```text
StageWorkspaceMock
├─ Stage header + stage switcher
├─ Coverage metric strip
└─ Workspace body
   ├─ Agent rail
   │  ├─ Company Controller
   │  ├─ deterministic collectors
   │  └─ visible LLM specialists
   └─ Selected agent workspace
      ├─ Agent header
      ├─ Conversation timeline
      └─ Artifact / evidence inspector
```

用户点击阶段、Agent 或 artifact 时只改变 mock 的本地 React selection state，不写全局 store。

### 4. LLM 与确定性任务必须诚实区分

- 每个真实 LLM worker 显示为可选 Agent，并拥有公开对话、工具调用和终态。
- 爬虫、AST parser、scanner runner 等不使用 LLM 的任务可以出现在拓扑中，但必须标记“确定性任务 · 无 LLM”。
- 不展示 provider 私有 chain-of-thought。Mock 展示的是公开 narration、任务分配、工具生命周期、artifact 和 evidence。
- 禁止用假 Agent 对话包装普通工具结果。

### 5. Artifact 是跨阶段统一的证据入口

不同阶段共享一套 artifact 外壳，但内容按判别类型渲染：

- Recon：asset / relationship / service observation
- Enumeration：endpoint、raw expression、resolved URL、source file/line、参数和 evidence IDs
- Application Understanding：application fact、route/contract、auth/business relationship
- Vulnerability：candidate、severity、source observation、evidence completeness
- Verification：prepared action、oracle verdict、proof/refutation evidence

Enumeration mock 必须明确展示用户本轮关心的字段：接口在哪里发现、method、参数位置、base-path resolution chain 和当前可信状态。

### 6. 生产接入留到 UI 获批后

后续生产实现采用纯 adapter：

```text
StageTeamReadModel
+ ActiveSubAgent event projection
+ stage coverage read model
+ artifact/evidence read model
        ↓
stageTeamToWorkspaceSnapshot(...)
        ↓
StageWorkspaceView
```

现有 `SubAgentDetailView` 强耦合 session/store/navigation；后续应提取纯 `AgentConversationTimeline` 展示层，而不是让 mock 或生产 Workspace 伪造全局 store 状态。

## 响应式与状态

- 宽桌面：Agent rail + conversation/evidence 双栏。
- 窄窗口：Agent rail 置顶，conversation 和 evidence 顺序堆叠。
- 每个阶段 fixture 至少覆盖 running、completed、waiting、blocked 中的多种状态。
- 无 Agent、无 artifact、加载失败属于未来 production adapter 的三态，本轮 mock 保留可视占位，不声明真实数据 authority。

## 本轮边界

- 不改数据库 schema/migration。
- 不改 generated IPC 类型。
- 不调用真实 provider、网站或扫描器。
- 不修改 `Test1` 或任何 operation 数据。
- 不替换生产 `stage_run` 详情。
- 不运行未授权的全量前端测试、`just check`、`just precommit` 或全仓门禁。

## Mock 验收

1. `ComponentTestbed` 首屏可见统一 Stage Workspace。
2. 五个阶段使用同一 UI 框架并可切换。
3. 点击 Agent 会切换到对应公开对话。
4. LLM Agent 与确定性任务有明确标签。
5. 点击 endpoint/artifact 会更新证据区。
6. 页面不出现旧的 Company Controller 汇总卡布局。
7. focused Vitest、Biome 和 frontend typecheck 通过并记录证据。
