# frontend / components

> **一句话职责**：React UI 组件——按功能域分 ~39 个目录（AIChatPanel / AgentChat / HomeView / Settings / GridTerminal / FindingsPanel / MethodologyPanel / Sidecar / SubAgent* / TabBar / PaneContainer / CommandPalette / Markdown …），是整个桌面 UI 的视图层。

- **类型**：前端子系统
- **路径**：`frontend/components/`（~404 ts/tsx）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何 UI 组件（聊天面板、终端、设置、findings/methodology 面板、sub-agent 视图、面板布局、命令面板等）时
- 改组件的 loading/error/empty 三态、与 store/hooks/api 的接线时

## 职责

桌面应用的视图层，按功能域分目录。关键域：`AIChatPanel`/`AgentChat`（AI 对话）、`GridTerminal`/`LiveTerminalBlock`/`CommandBlock`（终端）、`Settings`（设置含 IntelProviders）、`FindingsPanel`/`MethodologyPanel`/`DashboardPanel`/`AuditLogPanel`（pentest UI）、`SubAgentCard`/`SubAgentTreeView`/`SubAgentDetailView`（sub-agent）、`HomeView`（首页/engagement）、`PaneContainer`/`TabBar`/`ActivityBar`（布局）、`Markdown`/`StreamingOutput`/`DiffView`（渲染）。

## 关键子目录（节选）

| 域 | 组件目录 |
|---|---|
| AI 对话 | `AIChatPanel` / `AgentChat` / `StreamingOutput` / `SubAgentCard` / `SubAgentTreeView` / `SubAgentDetailView` / `SystemHooksCard` |
| 终端 | `GridTerminal` / `LiveTerminalBlock` / `CommandBlock` / `Ansi` |
| pentest UI | `TargetPanel` / `FindingsPanel` / `MethodologyPanel` / `DashboardPanel` / `AuditLogPanel` / `QuickNotes` |
| 布局/导航 | `PaneContainer` / `TabBar` / `ActivityBar` / `HomeView` / `DetachedView` / `CommandPalette` / `QuickOpenDialog` |
| 渲染/弹窗 | `Markdown` / `MarkdownEditor` / `DiffView` / `ImageModal` / `*Popup`（FileCommand/Path/Slash/History） |
| 其它 | `Settings` / `Sidecar` / `SessionBrowser` / `FileEditorSidebar` / `NotificationWidget` / `ErrorBoundary` |

AIChatPanel 切换 conversation tab 时会激活关联 terminal 作为上下文，但必须通过 `terminalAutoFocus` suppression 保持 DOM 焦点在 chat 输入；`GridTerminal` / xterm / `UnifiedInput` 的自动 focus 都要尊重这个窗口，用户主动点击 terminal 时再清掉 suppression。

`StageRunOrgRows` 渲染 `stage_run` 的 AI worker 执行边界：详情页必须表达 `Main Agent` 只负责调度，`Recon/Prober/Enumerator Agent` 等 specialist worker 按 org 执行并可 drill-in 到子 agent 对话/工具调用；即使只有 1 个 org，也显示为 `1 worker`，不要折叠成主 agent 自己完成阶段。`stage_run_org_progress` 是最后一次 live 快照；父 `stage_run` tool 已 completed/error/interrupted/expired 时，running/queued worker 只能投影为 stopped 展示，不能继续画 spinner / Running 文案。

后台工具（`status:"backgrounded"`）必须在聊天工具卡、`ToolExecutionCard`、`ToolCallDetailView`、`SubAgentDetailView` 和 `UnifiedInput` 状态行里保持同一语义：backgrounded 是 live/non-terminal，不显示成功绿勾；detail 模式会隐藏底部输入行，所以 header 要挂 `BackgroundJobsBadge` 作为会话级后台任务入口。sub-agent 后台工具要保留 `backgrounded` 状态并按 `job_id` 接收 `tool_background_completed` 回填，避免完成前突然切成最终 Output 样式。
`BackgroundJobsBadge` 不能只依赖已到达的 job registry；detail 已经能从当前工具或 sub-agent 工具列表看出 backgrounded 数量时，要用 fallback count 显示 `N running`，避免详情页没有任何后台入口。

`ToolCallDetailView` 和 `SubAgentDetailView` 对 shell-like 工具（`run_pty_cmd` / `run_command` / `pentest_run`，以及带 `args.tool_name` + `background/timeout_secs` 的后台工具包装参数）必须在 running/backgrounded 时固定显示 Output 区；没有 stdout/stderr chunk 时显示 pending 状态，一旦 `tool_output_chunk` 到达就用同一区域追加，避免 detail 只显示 Input、让用户误以为工具没有运行。completed/error 且 stdout/stderr 为空时也要显示 `No output.`，不要把 Output 区整个隐藏。注意：sub-agent 里的 `pentest_run` 不是 `run_pty_cmd`，但仍应按 shell-like 输出渲染，同时保留它自己的工具名和 Input args。
detail/live thinking/output 的自动跟随滚动必须用 rAF 合并，并且只在用户贴近底部时跟随；running/backgrounded 的长 Output 只渲染尾部窗口，完整数据保留在 store/result，避免每个 chunk 重新 parse 全量 ANSI 文本。

detail 里的状态图标不能只信 transport/completed 状态；`whatweb` 这类工具可能 `exit_code=0` 但 stdout/stderr 表达依赖缺失或 fatal error，主工具 detail、sub-agent detail、聊天摘要和 tool execution card 都要复用 `toolResultIndicatesFailure` 后再画绿色勾。
`SubAgentDetailView` header 也不能只信原始 `subAgent.status`：如果 completed agent 仍有 running/backgrounded 工具，header 要显示运行态/后台态；如果 completed agent 的最后一个工具调用失败（典型是 `submit_stage_deliverable` needs_fix/error 后无成功提交），header 要显示错误，避免“业务卡住但顶部已完成”的误导。

TargetPanel 左侧树默认只做组织导航：子组织和公司计数保留，但 IP/URL/域名资产不在左树展开；右侧 Targets 面板按 IP 联合展示资产，点击 IP、域名或 URL 进入 target workbench。不要把大量 IP 重新铺成 org 的第一层 children，否则母子公司层级会被资产列表淹没。
Target detail 展开区必须显式展示 active landing 写回的 top-level recon fields（`real_ip` / `http_status` / `http_title` / `webserver` / `cdn_waf` / `os_info` / `content_type`），即使 `ports[]` 还没有对应 entry；per-port metadata 和 fingerprints 继续在 Services / Fingerprints 区展示。
`TargetSurfaceWorkbench` 的 IP/host Surface 相关域名列表必须可 drill-in：无论当前 subject 是 synthetic host 还是真实 IP target，都要传 `onSelectDomain`，点击域名行应切换到对应 domain target。
Target 左侧 org tree 的 chevron 只表示“有可展开内容”：有子公司，或在资产树模式下有资产组；没有下一层的 org leaf 不显示 chevron。展开/收起只能点 chevron，点击公司行主体只选中并展示右侧详情；双击公司行主体也可展开/收起。root-level 主公司折叠采用 accordion 口径：展开一个主公司时收起其它主公司。

## 依赖

- `react`、Tailwind 4；消费 `store`（状态）、`hooks`（行为）、`lib/api`（后端）、`lib/generated`（类型）

## 注意事项 / 坑

- **不变量（AGENTS.md §2.3）**：组件**禁裸 `invoke()`**，走 `lib/api/<domain>`；三态 UI（loading/error/empty）每条异步路径都要画。
- 跨 IPC 类型 import 自 `lib/generated/`（ts-rs），别手写。
- 组件多，改前先定位功能域目录；大组件（AIChatPanel/HomeView/GridTerminal）已内部拆分，遵循其既有拆分。

## 测试入口

```bash
just check-fe   # biome + typecheck
just test-fe    # vitest（含组件快照/交互测试）
```
