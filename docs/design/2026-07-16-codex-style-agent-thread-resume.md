# Codex 式 Agent Thread / Turn 恢复设计

> 本文关于 outcome-unknown 工具一律人工处置的边界，已由
> `2026-07-16-stage-team-interrupted-tool-reconciliation.md` 部分取代；Thread/Turn、operation resume 与
> same-chain 身份合同仍以本文为准。

- **日期**：2026-07-16
- **状态**：Approved；用户已明确要求开始实现并授权本设计所需的向前 migration
- **范围**：Task/Operation 继续、Company Controller 与动态 child 的崩溃恢复、同一 chain 的 UI identity
- **非目标**：重放 outcome-unknown 外部工具、跨 session 接管、放宽 scope/Gate、改写历史 terminal operation

## 1. 用户合同

用户发送“继续”或在应用重启后恢复时，Golish 必须继续原来的逻辑 Agent，而不是重新创建一个空白
Agent。主 Agent、Company Controller 和每个 child 都遵守同一条规则：

```text
Agent Thread（稳定身份与历史）
  ├─ Turn 1（一次 provider/工具执行尝试）
  ├─ Turn 2（崩溃或等待后继续）
  └─ Turn N
```

恢复前后必须保持相同 operation、WorkItem、WorkerRun 与 `message_chain_id`。新的执行只增加 Turn，当前
实现以 `attempt_epoch` 表示 worker Turn；顶层 Operation 以新增的 `operation_turns` 留下同样的 durable
审计记录。只有用户明确选择 fresh restart，或 chain 已损坏且用户接受丢弃上下文时，才允许创建新 chain。

## 2. 当前断点

### 2.1 顶层预检和 exact claim 使用了不同状态集合

`latest_resumable_by_session` 接受 `running | waiting`，但 `exact_resumable_runtime_source` 和 claim 只接受
`waiting`。应用刚退出留下的 task 仍是 `running`，一小时 startup reaper 尚未把它降为 `waiting`，所以
预检选中了 operation，exact claim 随后却返回 “no complete idle runtime-memory source”。

### 2.2 普通 Team child 把 Turn 失败错误建模为 Agent replacement

无 active tool 的过期 child 当前被改成 `superseded`，WorkItem 经 `retry_pending` 回到 `queued`；下一次
claim 插入新的 WorkerRun 和新的 `message_chain_id`。这保留了工作项，却丢掉了 Agent Thread 身份和短期
记忆。Controller 特殊路径已经采用正确行为：重领同一 WorkerRun、递增 `attempt_epoch`、复用同一 chain。

### 2.3 executor 已经具备正确的下半段

bound sub-agent executor 会从 exact `message_chain_id` 读取完整消息、tool result 和 evidence id，再把恢复
指令作为下一条 user message 追加。因此本设计不重写 executor；修复点是上游的 durable identity 和 claim。

## 3. 持久化模型

### 3.1 顶层 Operation Thread

`tasks.id == operation_state.operation_id` 继续作为稳定 Operation Thread。新增：

```sql
CREATE TABLE operation_turns (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    ordinal BIGINT NOT NULL CHECK (ordinal > 0),
    trigger_input TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('running', 'waiting', 'completed', 'interrupted', 'failed')
    ),
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (operation_id, ordinal)
);

CREATE UNIQUE INDEX operation_turns_one_open
    ON operation_turns(operation_id)
    WHERE status IN ('running', 'waiting');
```

创建 operation 时写入 ordinal 1。exact resume 在同一 DB transaction 中：

1. 锁定 task 与当前 open turn；
2. 验证 complete runtime source、当前 session/project 与“没有 live Worker lease”；
3. 把旧 open turn 标成 `interrupted`；
4. 插入 ordinal + 1 的 `running` turn，保存本次 continuation input；
5. 把 task CAS 为 `running` 并返回 pinned runtime source。

同一 GUI/CLI kernel 的 `TopLevelRequestLease` 继续阻止当前进程内并发请求；DB transaction 阻止两个
resume caller 同时推进 ordinal。`running` task 不再依赖一小时 reaper 才能恢复。

### 3.2 Controller / child Agent Thread

现有结构足够表达 sub-agent Thread，无需再造通用 Agent 表：

| 语义 | 现有持久化对象 |
|---|---|
| Agent Thread identity | `stage_work_items.id + stage_worker_runs.id` |
| Conversation history | `stage_worker_runs.message_chain_id -> message_chains` |
| Turn number / lease fence | `stage_worker_runs.attempt_epoch` |
| Turn checkpoint | `checkpoint + checkpoint_version` |
| Tool attempt | `tool_calls.worker_run_id + attempt_epoch + lease_token` |

无 active tool 的 lease expiry 必须把同一 WorkerRun 变为 claimable，保留 checkpoint/chain，并在重新 claim
时增加 `attempt_epoch`。attempt budget按历史 WorkerRun 的实际 attempt epochs计算，不能因为复用 row 而失去
上限。旧的多个 WorkerRun仍可读，但新恢复不得再制造 replacement Worker。

### 3.3 outcome unknown

`active_tool_call_id` 存在且 lease 失效时仍进入 `recovery_required`。任何网络、扫描、写操作都不能凭
Thread 连续性自动重放。operator完成 typed recovery decision 后，才在同一 Thread 上追加下一 Turn。

## 4. 状态转换

```text
live lease                         → refuse duplicate resume
expired/no lease + no active tool → same Thread, next Turn
expired lease + active tool       → recovery_required, never replay
waiting_background + deps ready   → same Thread, next Turn
Gate BLOCK + repair fuel          → same Controller Thread, next Turn
explicit fresh restart            → new Operation/Thread
```

启动 reaper与请求时 recovery必须调用同一个 compound transition，避免一条路径复用 chain、另一条路径又
创建 fresh chain。

## 5. UI 合同

同一 `worker_run_id` 保证 Controller/child 卡片、transcript path 和 drill-in identity不变。恢复只更新状态与
`attempt_epoch`，可显示“继续执行 · Turn N”；不得新增一张同名 child 卡，也不得把旧 Thought 混入 sibling。
权威完成状态仍来自 Unit/Gate read model。

## 6. 验收

1. 应用退出后不到一小时发送“继续”，exact claim成功且 `operation_id`不变。
2. 新增一条 `operation_turns`，旧 turn为 `interrupted`，新 turn为 `running`，ordinal连续。
3. child无 active tool时崩溃：`work_item_id / worker_run_id / message_chain_id`不变，`attempt_epoch + 1`。
4. restored executor能读到旧消息与 tool results，初始 objective不重复插入。
5. child有 unknown active tool时仍进入 recovery，tool call不重放。
6. parked Controller等待 children后继续相同 chain；Gate BLOCK repair也不产生 fresh Aggregator/Controller。
7. live lease并发继续稳定拒绝；两个 resume caller最多一个得到下一 ordinal。
