# frontend / store

> **一句话职责**：全局状态——单个 Zustand store（`useStore`，immer + devtools），由 12 个 slice 组合（appearance/context/conversation/dialog/notification/panel/session/ai/workflow/pane/hitl/app-shell）+ selectors + types。

- **类型**：前端子系统
- **路径**：`frontend/store/`（~61 ts）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改全局状态字段、新 slice、selector、store-hooks 时
- 排查会话/面板/AI 流式/HITL/工作流的前端状态来源时

## 职责

组合式 Zustand store：`store/index.ts` 用 `create + devtools + immer` 把 12 个 slice 合成 `GolishState`；`slices/` 每个 slice 一个 `create*Slice` 工厂 + 状态接口；`selectors/` 派生选择器 + `store-hooks`；`types/` 共享状态类型。

## 关键子目录 / 文件

| 区域 | 说明 |
|---|---|
| `index.ts` | `useStore` 组合（immer + devtools + enableMapSet） |
| `slices/`（12） | appearance / context / conversation / dialog / notification / panel / session / ai / workflow / pane / hitl / app-shell（root 字段） |
| `slices/session*`（core/streaming/tabs/terminal/draft-types/helpers） | 会话 slice 拆分 |
| `slices/workflow/` | 工作流/计划状态（含 markStagePassed/passedStages） |
| `selectors/`（app/session/agent-tree/anchors/pane-leaf/store-hooks） | 派生选择器 |
| `types/` | 共享状态类型 |

## 依赖

- `zustand`（+ devtools/immer middleware）、`immer`；消费 `lib`（类型/api）、被 `components`/`hooks` 订阅

## 注意事项 / 坑

- **单 store + slice 组合**：加状态先归到对应 slice（或新 slice 并在 `index.ts` + `slices/index.ts` 注册），别在组件里散落全局可变状态。
- immer 写法（draft 可变）+ `enableMapSet()`（slice 用 Map/Set 时必需）。
- 三态 UI（loading/error/empty，AGENTS.md §2.3）的状态多源自此（如 session streaming/hitl）。

## 测试入口

```bash
just check-fe
just test-fe   # 含 appearance/context/conversation/notification/session 等 slice 单测
```
