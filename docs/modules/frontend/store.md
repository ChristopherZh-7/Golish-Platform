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
| `slices/live-output.ts` | 高频工具 live output 尾部窗口工具，供 session / ai / workflow sub-agent 输出写点共用 |
| `slices/session*`（core/streaming/tabs/terminal/draft-types/helpers） | 会话 slice 拆分 |
| `slices/workflow/` | 工作流/计划状态（含 markStagePassed/passedStages） |
| `selectors/`（app/session/agent-tree/anchors/pane-leaf/store-hooks） | 派生选择器 |
| `types/` | 共享状态类型 |

`Session.stageRuns` 按 `stage_run` 工具 `requestId` 缓存逐 org 进度；`Session.stageRun` 保留为当前/兼容快照。中断后继续产生新的 `stage_run` 时，UI 必须按 requestId 读取对应快照，不能用 session 级单槽覆盖旧/新 run。

`Session.candidateReviewHint` 只保存 operation/wave/status/resume version 与单调 `refreshVersion`，供 attack_candidate detail 触发 DB reload；它不保存 approvals，也不能被 gate/resume 当作 durable truth。

`Session.reportingReadModelHint` 只保存当前 Reporting operation 与单调 `refreshVersion`，供 `AIChatPanel` 触发 authoritative report IPC reload；同 operation 递增 refresh，operation 改变时 replace 并从 version 1 开始，clear/session switch 必须移除旧视图。hint 不保存 report rows、Gate verdict 或 publish authority。

`conversation.updateMessageToolResult` 优先按 `requestId` 精确回填工具结果，工具名只作旧路径兜底；后台工具完成事件通过 `updateMessageToolResultByJobId` 按原 backgrounded result 里的 `job_id` 回填聊天气泡，避免同名 `pentest_run` 串结果或 backgrounded 长期显示成功态。
`workflow/sub-agent` 的 `entries` 按 sub-agent LLM response 边界维护：`sub_agent_text_delta.accumulated` 和 `sub_agent_reasoning.accumulated` 应回填上一条 `tool_call` 之后的当前 text/thinking entry，而不是因为中间插入了 thinking 就新建重复 `Agent Output`；只有新的 tool call 才是新的 response 边界。
`tool_output_chunk` 写入 store 时必须走 `slices/live-output.ts::appendLiveToolOutput`，让 `activeToolCalls` / timeline `ai_tool_execution` / sub-agent `toolCalls` 都只保留 bounded live tail；完整结果仍从最终 `tool_result` / transcript / run.log 追溯，避免 route_probe_paths、browser_collect_js_api、js_extract_apis 等高频工具把 React state 膨胀到几十万字符。

## 依赖

- `zustand`（+ devtools/immer middleware）、`immer`；消费 `lib`（类型/api）、被 `components`/`hooks` 订阅

## 注意事项 / 坑

- **单 store + slice 组合**：加状态先归到对应 slice（或新 slice 并在 `index.ts` + `slices/index.ts` 注册），别在组件里散落全局可变状态。
- immer 写法（draft 可变）+ `enableMapSet()`（slice 用 Map/Set 时必需）。
- 三态 UI（loading/error/empty，AGENTS.md §2.3）的状态多源自此（如 session streaming/hitl）。
- `actions.ts::clearConversation` 是 backend/local timeline 的原子边界：先让 backend clear 成功，再 `clearTimeline`。session busy 或其他真实错误必须保留 timeline 并向调用方返回错误；只有明确的 unavailable-command 错误才调用 legacy clear，不能对 busy 二次 invoke。

## 测试入口

```bash
just check-fe
just test-fe   # 含 appearance/context/conversation/notification/session 等 slice 单测
```
