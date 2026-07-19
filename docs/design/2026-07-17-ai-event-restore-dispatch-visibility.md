# AI 事件恢复与派工可见性收敛

## 背景与实体证据

会话 `pentest-chat-1784267739419-1` 的 Enumeration Company Controller 已在后端成功执行
`stage_team_dispatch_workers`：tool request `65a7d956-0572-4513-9e42-250b1f3f6d2b` 返回
`accepted_count=3`。客户端当时处于恢复/首页窗口，活动 session 是 `home-1784269890161`；
`frontend.log` 随后把该会话的 `sub_agent_tool_request`、`sub_agent_tool_result`、
`sub_agent_started` 记录为 `AI event dropped for unknown session`。

用户发送“继续”后，后端恢复了原 WorkerRun，包括
`052b01cb-d7e8-4536-bc9b-6bbaa1e5ec8f` 和
`ef511faf-c93a-4650-a7ca-f50bb6749d59`。新的 `sub_agent_started` 能创建 child 卡片，但
`SubAgentDetailView` 只会把 `${dispatch_request_id}::worker:<worker_run_id>` child 挂到已经存在的
`stage_team_dispatch_workers` tool entry；父 tool 事件早已丢失，因此 UI 没有挂载点。

## 目标

1. 带具体 `session_id` 的 AI event 在 conversation/terminal 尚未恢复时先进入有界缓冲，不再直接丢弃。
2. conversation → terminal 映射建立后按原到达顺序重放事件；sequence 去重只在真正分发时推进。
3. 缓冲有明确容量上限，title-generation 与真正 `unknown` session 的既有隔离语义保持不变。
4. 对已经丢失历史父 tool 的运行，`SubAgentDetailView` 从现有 DB-authoritative Stage Team read model
   恢复该 Controller 发出的 Request、WorkItem、WorkerRun，并展示 queued/running/completed/error 状态。
5. 不从模型 prose 猜派工，不伪造不存在的第三方 child，不修改历史 transcript、数据库行或 IPC 类型。
6. DB 恢复分组必须回到 durable Request 的历史创建位置；主 Agent 后续 Thought/工具继续追加时，恢复卡自然留在
   旧时间线中并滚离视口，不能作为常驻 footer 反复挂在最底部。

## 设计

### 1. 恢复窗口事件缓冲

`useAiEvents` 继续以 backend `event.session_id` 为路由权威。若该 ID 既不是 terminal session，又暂时无法经
conversation 找到 terminal，则把完整 `AiEvent` 放进 module-scoped、按 AI session 分桶的 FIFO：

- 每个 session 最多 512 条，总 session 桶最多 32 个；超限只丢最旧事件并记录 warning；
- 事件入缓冲时不更新 `lastSeenSeq`；
- hook 订阅 Zustand store，任何 conversation/session/terminal 映射变化后尝试 drain；
- drain 先解析一个真实 terminal session，再按 FIFO 经过原 handler/sequence 路径分发；
- hook remount 不清空 module-scoped 缓冲，测试/显式 session cleanup 可调用 reset helper。

`title-gen-*` 仍立即忽略。后端给出空/`unknown` session 时仍只允许当前 active terminal fallback；没有 active
terminal 时无法建立安全归属，保持丢弃。

### 2. DB-authoritative 历史派工恢复

Company Controller 的 `parentRequestId` 含稳定 controller WorkerRun id：
`<stage_run>::team::<org>::lead:<worker_run_id>`。对应 `SessionStageRun.rows[]` 已含 exact
`operationId + stageExecutionId` refresh pointer。现有 `ai_get_stage_team_read_model` 返回：

- `requests[].parentWorkerRunId`：精确证明 Request 由该 Controller 发出；
- `acceptedWorkItemId`：Request → child WorkItem；
- `workItems[].workers[].workerRunId/status`：child durable identity 与状态。

当 Controller 本地没有 `stage_team_dispatch_workers` tool、但 session 中存在 detached worker-shaped child 时，
detail view 用上述 exact pointer读取 read model，只选择 `parentWorkerRunId` 等于当前 Controller WorkerRun 的
Request。已有 live `ActiveSubAgent` 按 WorkerRun id 接回可点击卡；没有 live event 的 durable child 仍以
DB 状态显示为 queued/running/completed/error。

该恢复分组明确标记“从持久化调度状态恢复”。read model loading/error/empty 都有独立投影；empty 不渲染假卡。
如果原 dispatch tool 已存在，原始 tool args/result 继续优先，DB 恢复分组不重复出现。

### 3. 恢复分组的时间线位置

恢复分组不是实时状态 footer。detail view 取当前 Controller 所有匹配 Request 的最早 `createdAt` 作为派工时间，
在父 Agent 时间线中寻找第一个不早于该时间的 timestamped Thought 或 tool call，并把恢复分组插在该 entry 之前。
因此历史派工后的复核、补扫和提交继续显示在卡片下方；后续 streaming 只扩展时间线，不会把已完成 Worker 卡重新
挪回底部。若当前还没有任何可证明更晚的 timestamped entry，分组暂时位于末尾；第一个后续 Thought/tool 到达后
会自动归位。纯文本 entry 没有可信时间戳，不能靠文字内容猜测位置。

## 安全与一致性

- 不把 unknown AI session 路由到当前聊天；只缓存带具体 session identity 的事件。
- sequence 在真实投递时检查，防止缓冲事件先占用序号而把后续重放判成重复。
- DB read model 经现有 IPC 重验 operation/execution scope；event/prose 只用于 UI identity，不成为调度权威。
- 历史缺失 objective 不从自然语言补写；卡片只展示 durable request kind/role 和已有 child task。
- 本修复不改 schema、migration、`frontend/lib/generated` 或外部服务。

## 验证

- Hook RED/GREEN：先发送 `sub_agent_started → tool_request → tool_result` 到尚未恢复的 AI session；建立
  conversation 和 terminal 映射后，断言同一 child/tool 按序收敛且缓冲归零。
- Component RED/GREEN：Controller 本地无 dispatch tool，DB read model 有 3 个 accepted requests；断言两张
  live child 卡和一张 DB queued 卡全部可见，read 请求使用 exact operation/execution，并且恢复分组位于后续
  主 Agent Thought/正文之前而不是时间线 footer。
- 仅运行两个 focused Vitest 文件、`pnpm typecheck`、相关文件 Biome、JSON/diff check。按用户明确要求不运行
  `init.sh`、`just precommit` 或完整前后端测试，因此 feature 保持 `in_progress`。
