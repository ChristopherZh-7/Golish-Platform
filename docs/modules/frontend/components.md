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
| pentest UI | `FindingsPanel` / `MethodologyPanel` / `DashboardPanel` / `AuditLogPanel` / `QuickNotes` |
| 布局/导航 | `PaneContainer` / `TabBar` / `ActivityBar` / `HomeView` / `DetachedView` / `CommandPalette` / `QuickOpenDialog` |
| 渲染/弹窗 | `Markdown` / `MarkdownEditor` / `DiffView` / `ImageModal` / `*Popup`（FileCommand/Path/Slash/History） |
| 其它 | `Settings` / `Sidecar` / `SessionBrowser` / `FileEditorSidebar` / `NotificationWidget` / `ErrorBoundary` |

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
