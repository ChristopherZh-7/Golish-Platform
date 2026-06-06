# frontend / hooks

> **一句话职责**：React 自定义 hooks——封装行为/副作用：Tauri 事件订阅（`useTauriEvents`/`useAiEvents`/`useSidecarEvents`）、终端（`useCreateTerminalTab`/`useTerminalPortal`）、补全/搜索（`usePathCompletion`/`useToolSearch`/`useHistorySearch`）、主题/键盘/文件等。

- **类型**：前端子系统
- **路径**：`frontend/hooks/`（~32 ts/tsx）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 React hook（Tauri 事件订阅、终端 tab、路径/工具/历史搜索、主题、键盘处理、文件 watcher/index）时
- 排查前端从哪订阅后端事件、`tauri-event-types` 契约时

## 职责

把可复用的行为/副作用封装成 hooks，连接 `store` / `lib/api` / `services` 与组件。事件类 hook 是后端→前端事件的订阅入口（`useTauriEvents`/`useAiEvents`/`useSidecarEvents`），经 `services/ai-events` 分发到 store。

## 关键文件（节选）

| hook | 说明 |
|---|---|
| `useTauriEvents` / `useAiEvents` / `useSidecarEvents` | Tauri / AI / sidecar 事件订阅 |
| `tauri-event-types.ts` | 前端侧 Tauri 事件类型契约 |
| `useCreateTerminalTab` / `useTerminalPortal` | 终端 tab / portal |
| `usePathCompletion` / `useToolSearch` / `useHistorySearch` / `useSlashCommands` / `useFileCommands` | 补全/搜索/命令 |
| `useTheme` / `useKeyboardHandlerContext` / `usePaneControls` | 主题 / 键盘 / 面板 |
| `useFileIndex` / `useFileWatcher` / `useFileEditorSidebar` / `useAsyncQuery` / `useProviderSettings` | 文件 / 异步查询 / provider 设置 |

## 依赖

- `react`；消费 `store`、`lib/api`、`services`（事件分发）

## 注意事项 / 坑

- 事件 hook 经 `services/ai-events` 注册表分发——加新 AI 事件类型要 hook + service handler + `tauri-event-types` 三处同步。
- hook 多带 `.test.ts`（useAiEvents/useCreateTerminalTab/useFileIndex/usePathCompletion/useTauriEvents/useTheme/useThrottledResize）；改行为同步测试。

## 测试入口

```bash
just check-fe
just test-fe   # vitest（含 useAiEvents/useTauriEvents 等 hook 测试）
```
