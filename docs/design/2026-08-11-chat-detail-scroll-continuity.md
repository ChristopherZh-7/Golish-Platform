# Chat 与 Detail 滚动连续性设计

## 问题

进入全屏 Detail 时，`AIChatPanel` 保持事件投影但通过 `renderUi=false` 释放重消息 DOM；返回 timeline 后滚动容器是新节点。现有 `useChatAutoScroll` 只在组件首次 mount 时绑定 listener，并只在 messages 变化时重绑 ResizeObserver，因此新节点从 `scrollTop=0` 开始且没有完整的自动跟随能力。

unified Investigation 的 full-pane 使用 `InvestigationWorkspaceView` 自己的 `overflow-y-auto` 主区域，但没有接入已有的 transcript stickiness hook。长 transcript 初次打开和持续增长时都停在上方；用户必须手动滚到底部。

## 决策

- 保留 Detail focus 下 `renderUi=false` 的内存保护，不让长 ChatPanel DOM 常驻。
- `useChatAutoScroll` 接收当前 UI 是否可见与 conversation key。旧节点隐藏前保存 scroll position/follow intent；同 conversation 的新节点出现时，原来贴底则滚到最新 bottom，原来主动上滑则恢复旧 `scrollTop`。listener 与 ResizeObserver 必须绑定新节点。
- `useTranscriptAutoScroll` 接收可选的 transcript key。切换到另一个 actor 时重新开启 follow 并定位到该 actor transcript bottom；同 actor 内用户向上滚动后暂停，回到底部后恢复。
- `InvestigationWorkspaceView` 只在 agent transcript selection 下把 main viewport 与 selected transcript wrapper接入该 hook。Hypothesis/Campaign 等非对话页面保留普通滚动，不被强制贴底。
- 自动滚动只属于展示状态，不改变 transcript、operation、evidence、Gate 或任何后端 authority。

## 非目标

- 不撤销 200 条 Stage transcript DOM 窗口或 ChatPanel projection-only 内存优化。
- 不在 store/DB 持久化像素位置，也不跨应用重启恢复 scroll。
- 不对用户主动向上阅读的同一对话持续强制滚底。

## 验证

- Chat hook 覆盖 visible → hidden → visible 的新 DOM：贴底时回到最新 bottom，主动上滑时恢复旧位置，并确认 observer 重新绑定。
- Transcript hook 覆盖 actor key 切换重启 follow，同时保留同 actor 主动上滑暂停。
- Investigation workspace DOM 测试覆盖选中长 transcript 初次定位到底部、增长跟随与向上阅读保护。
- 运行 focused Vitest、受影响文件 Biome、TypeScript no-emit、JSON 与 diff 检查。
