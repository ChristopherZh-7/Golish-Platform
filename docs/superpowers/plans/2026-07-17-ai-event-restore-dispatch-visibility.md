# AI 事件恢复与派工可见性收敛实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划，并使用 test-driven-development 完成每个 RED→GREEN 循环。

**目标：** 在会话恢复窗口可靠重放 AI events，并从 DB-authoritative Stage Team read model 补出历史父 tool 已丢失时的 durable Worker 卡片。

**架构：** `useAiEvents` 为具体但暂不可解析的 AI session 建立有界 FIFO，并在 Zustand 的 conversation/terminal 映射就绪后走原 sequence/handler 路径重放。`SubAgentDetailView` 只用 exact operation/execution/controller WorkerRun 查询现有 Stage Team read model，恢复当前 Controller 的 Request/WorkItem/WorkerRun 投影。

**技术栈：** React 19、TypeScript 6、Zustand、Vitest、Testing Library、Tauri typed API。

---

## 文件结构

- 修改 `frontend/hooks/useAiEvents.ts`：有界 pending-session FIFO、session 解析、store-change drain。
- 修改 `frontend/hooks/useAiEvents.test.ts`：恢复窗口事件不丢失的集成回归。
- 修改 `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：exact Stage Team pointer、read-model 恢复投影与三态 UI。
- 修改 `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：缺失父 dispatch tool 的 DB 恢复回归。
- 修改 `frontend/lib/i18n/en.json`、`frontend/lib/i18n/zh-CN.json`：恢复分组、loading/error 文案。
- 修改 `docs/modules/frontend/hooks.md`、`docs/modules/frontend/services.md`、`docs/modules/frontend/components.md`、`docs/modules/INDEX.md`：同步事件与 UI 合同。
- 修改 `feature_list.json`、`agent-progress.md`：记录状态与证据。

### Task 1：建立事件恢复窗口 RED

**文件：**

- 修改：`frontend/hooks/useAiEvents.test.ts`

**步骤：**

1. mount `useAiEvents`，向尚不存在 conversation/terminal 映射的 `pentest-chat-restore` 依次发送
   `sub_agent_started`、`sub_agent_tool_request`、`sub_agent_tool_result`。
2. 断言事件没有错误路由到当前 home terminal，且 pending count 为 3。
3. 添加 `aiSessionId=pentest-chat-restore` conversation，再把 `term-restored` 加入该 conversation。
4. 断言 `activeSubAgents[term-restored]` 出现一个 child，tool status 为 completed/result 保留，pending count 为 0。

**验证：**

```bash
pnpm exec vitest run frontend/hooks/useAiEvents.test.ts
```

预期：新增测试在旧实现下因事件被直接丢弃而失败。

### Task 2：实现有界缓冲和重放

**文件：**

- 修改：`frontend/hooks/useAiEvents.ts`
- 修改：`frontend/hooks/useAiEvents.test.ts`

**步骤：**

1. 增加 module-scoped `Map<string, AiEvent[]>`，每桶 512、最多 32 桶；导出测试 reset/count helper。
2. 抽出 `resolveAiEventSessionId(rawSessionId, state)`：直接 terminal → conversation terminal → active
   conversation exact match，均不存在则返回 null。
3. 将原 sequence 检查和 `dispatchEvent` 放进只接收 resolved terminal id 的函数。
4. 未解析的具体 session 进入 FIFO；store subscription 在映射变化后 drain，成功投递后删除桶。
5. cleanup 取消 Tauri listener 与 store subscription，但不因 React remount 清空 pending FIFO。

**验证：**

```bash
pnpm exec vitest run frontend/hooks/useAiEvents.test.ts
```

预期：原测试和新恢复窗口测试全部通过。

### Task 3：建立 DB 恢复派工卡 RED

**文件：**

- 修改：`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`

**步骤：**

1. 构造无 `stage_team_dispatch_workers` tool 的 Controller，session 中只有两个
   `${dispatch_id}::worker:<worker_run_id>` live child。
2. `SessionStageRun` row 提供 exact operation/execution/controller request id。
3. 注入 read API：返回同一 Controller parentWorkerRunId 的 3 个 accepted requests、3 个 child WorkItems，
   其中两个 running、一个 queued。
4. 等待恢复读取结束，断言 3 个 assignment、2 个可点击 child、1 个 queued 卡与恢复分组文案都存在。

**验证：**

```bash
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

预期：旧实现没有 read-model 恢复分组，新增断言失败。

### Task 4：实现 exact DB read-model 恢复投影

**文件：**

- 修改：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
- 修改：`frontend/lib/i18n/en.json`
- 修改：`frontend/lib/i18n/zh-CN.json`

**步骤：**

1. 从 `::lead:<worker_run_id>` 和匹配 `SessionStageRun.rows[]` 得到 exact
   `operationId/stageExecutionId/controllerWorkerRunId`；任一缺失即不读取。
2. 注入/默认调用 `getStageTeamReadModel`，只选择
   `request.parentWorkerRunId === controllerWorkerRunId` 的 rows。
3. 用 `acceptedWorkItemId` 找 WorkItem，再用 WorkerRun id 接回 live ActiveSubAgent；没有 live Agent 时按 durable
   work item/worker status 显示 queued/running/completed/error。
4. 原 dispatch tool 存在时禁用恢复分组；loading/error/empty 分开呈现，error 提供 retry。
5. 恢复分组不展示模型猜测 objective，只展示 read model request kind/role 或 live child task。
6. 用同一 Controller durable Requests 的最早 `createdAt` 作为 synthetic dispatch timestamp；在父运行流中插到
   第一个更晚的 timestamped Thought/tool 之前，禁止继续作为 timeline footer 渲染。

**验证：**

```bash
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

预期：新增和原有 dispatch/retry/error tests 全部通过。

补充回归：在缺父 tool fixture 中加入派工完成后的 Controller Thought/正文，断言恢复分组 DOM 顺序早于后续
正文。旧 footer 实现应 RED；按 durable Request 时间插入后应 GREEN。

### Task 5：同步文档并做聚焦门禁

**文件：**

- 修改：`docs/modules/frontend/hooks.md`
- 修改：`docs/modules/frontend/services.md`
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`feature_list.json`
- 修改：`agent-progress.md`

**步骤：**

1. 记录具体 AI session 不再直接 drop、sequence 在重放时推进，以及 Controller detail 的 DB-authoritative
   recovery contract。
2. 运行 focused tests、TypeScript、相关文件 Biome、JSON 与 diff check。
3. 按用户明确要求不运行 `init.sh`、`just precommit` 或全量测试；没有完整门禁时 feature 保持
   `in_progress`。
4. 共享 dirty tree 不 stage、commit、push，也不清理其它功能改动。

**验证：**

```bash
pnpm exec vitest run frontend/hooks/useAiEvents.test.ts frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
pnpm typecheck
pnpm exec biome check frontend/hooks/useAiEvents.ts frontend/hooks/useAiEvents.test.ts frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json
jq empty feature_list.json
git diff --check
```

预期：全部 exit 0；feature 因用户要求跳过完整 precommit 而保持 `in_progress`。
