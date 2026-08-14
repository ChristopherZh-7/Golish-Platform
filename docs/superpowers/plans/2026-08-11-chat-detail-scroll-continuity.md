# Chat 与 Detail 滚动连续性实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 返回 ChatPanel 时恢复正确的贴底/阅读位置，并让 Investigation Detail 的选中 Agent transcript 默认跟随最新内容。

**架构：** 保留 ChatPanel projection-only DOM 释放边界，在两个既有滚动 hook 内显式建模 viewport 生命周期与 follow intent。Chat hook按 visibility/conversation key恢复重建节点；transcript hook按 actor key重启新对话的 follow，Investigation view只在 Agent selection 时接线。

**技术栈：** React 19 hooks、TypeScript、Vitest、Testing Library、ResizeObserver、requestAnimationFrame、Biome。

## 文件结构

- 修改 `frontend/components/AIChatPanel/useChatAutoScroll.ts`：保存/恢复 Chat viewport 并重绑新 DOM。
- 修改 `frontend/components/AIChatPanel/useChatAutoScroll.test.tsx`：覆盖隐藏返回的贴底与历史阅读。
- 修改 `frontend/components/AIChatPanel/AIChatPanel.tsx`：传入 `renderUi` 与 active conversation key。
- 修改 `frontend/components/Engagement/useTranscriptAutoScroll.ts`：用 actor key 重置新 transcript follow。
- 修改 `frontend/components/Engagement/useTranscriptAutoScroll.test.tsx`：覆盖 actor 切换与用户阅读保护。
- 修改 `frontend/components/Engagement/InvestigationWorkspaceView.tsx`：把 Agent transcript 接到 viewport/content refs。
- 修改 `frontend/components/Engagement/InvestigationWorkspaceView.test.tsx`：覆盖初次/增量贴底和手动上滑暂停。
- 更新 `docs/modules/frontend/components.md`、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`：同步契约与证据。

## Task 1：ChatPanel 返回位置 RED → GREEN

1. 在 `useChatAutoScroll.test.tsx` 让 harness 按 `active` 真正卸载/重建 scroller，写两个失败用例：

```tsx
rerender(<Harness active={false} messages={messages} />);
rerender(<Harness active messages={[...messages, newest]} />);
expect(newViewport.scrollTop).toBe(newViewport.scrollHeight);

hookState!.userScrolledUpRef.current = true;
oldViewport.scrollTop = 240;
rerender(<Harness active={false} messages={messages} />);
rerender(<Harness active messages={messages} />);
expect(newViewport.scrollTop).toBe(240);
```

2. 运行：

```bash
pnpm exec vitest run frontend/components/AIChatPanel/useChatAutoScroll.test.tsx
```

预期：新用例因新节点未恢复/未重绑而失败。

3. 把 hook 签名改为：

```ts
useChatAutoScroll(messages, { active: renderUi, scrollKey: activeConvId ?? "no-conversation" })
```

保存旧节点 `scrollTop` 与 `userScrolledUpRef`；新节点在下一 animation frame 中按 follow intent恢复；effects依赖 `active`/`scrollKey`并重绑 listener/ResizeObserver。

4. 重跑同一测试，预期全部通过。

## Task 2：Investigation transcript RED → GREEN

1. 在 `useTranscriptAutoScroll.test.tsx` 写 actor key 切换测试：同 actor手动上滑后不跟随，切换新 key后重新定位新 transcript bottom。

2. 在 `InvestigationWorkspaceView.test.tsx` 写生产接线测试：选中有长 transcript 的 Agent 时 main viewport带 exact content wrapper并贴底；增长继续跟随，向上 wheel后暂停。

3. 运行：

```bash
pnpm exec vitest run frontend/components/Engagement/useTranscriptAutoScroll.test.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx
```

预期：hook key与Investigation接线缺失导致新用例失败。

4. 给 `useTranscriptAutoScroll(followKey?: string | null)` 增加 layout-phase key reset；ResizeObserver随 key/content重新绑定。

5. `InvestigationWorkspaceView` 使用：

```tsx
const transcriptKey = selectedActor?.actorId ?? null;
const transcriptScroll = useTranscriptAutoScroll(transcriptKey);
```

只在 `selectedActor` 存在时把 main 设为 viewport，并把 selected actor section设为 content；非 Agent selection不挂 handlers。

6. 重跑两个测试文件，预期全部通过。

## Task 3：定向门禁与状态回写

1. 运行：

```bash
pnpm exec vitest run frontend/components/AIChatPanel/useChatAutoScroll.test.tsx frontend/components/AIChatPanel/AIChatPanel.reporting.test.tsx frontend/components/Engagement/useTranscriptAutoScroll.test.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx
pnpm exec biome check frontend/components/AIChatPanel/useChatAutoScroll.ts frontend/components/AIChatPanel/useChatAutoScroll.test.tsx frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/Engagement/useTranscriptAutoScroll.ts frontend/components/Engagement/useTranscriptAutoScroll.test.tsx frontend/components/Engagement/InvestigationWorkspaceView.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx
pnpm typecheck
jq empty feature_list.json
git diff --check
```

预期：focused tests、Biome、typecheck、JSON与diff均exit 0；不运行未获授权的init/precommit/全量suite。

2. 将模块卡写明projection-only remount恢复与actor-key stickiness，记录RED/GREEN命令、退出码、风险和未提交文件；不commit、不push。
