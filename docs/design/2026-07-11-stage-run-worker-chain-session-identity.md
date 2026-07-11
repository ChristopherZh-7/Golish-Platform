# Stage-run worker chain session identity

## 问题

Headless `stage_run` 同时存在两种 session identity：

- `stage-run-<uuid>` 是事件、transcript 和 tracing 使用的稳定文本键；
- `chat_sessions.id` 是 message-chain persistence 使用的真实数据库 UUID。

旧的 `SubAgentExecutorContext` 只携带前者。`chain_persist` 把该文本键直接按 UUID
解析，解析失败后静默跳过 chain create。Enumerator 因此可以完成一段真实工作，却不会在
响应尾部返回 `sub_agent_session_id`。Enumeration capacity continuation 随后无法精确续回
同一 worker，把一次有效分段错误关闭为 retry budget exhausted。

## 决策

1. `SubAgentExecutorContext.session_id` 继续只表示事件/trace session 文本键，不改变
   transcript 路由语义。
2. 增加独立的 `persistence_session_id: Option<Uuid>`，只表示 message-chain 所属的
   数据库 session。
3. 上层 runtime/bridge 从已经完成 `upsert_by_chat_key` 绑定的 `DbTracker::session_uuid()`
   注入该字段；nested sub-agent 原样继承。
4. `chain_persist` 优先使用 `persistence_session_id`，并保留把 legacy 裸 UUID
   `session_id` 解析成数据库 session 的兼容回退。
5. 没有任何可用 persistence identity 时继续 fail closed：不创建 chain，也不允许
   `stage_run` 用 fresh worker 冒充同链 continuation。
6. 精确 UUID resume 只能加载当前 persistence session、当前 agent 所属的 chain；miss、
   DB error、ownership mismatch 或损坏 JSON 均直接失败，绝不能落到 latest/fresh create。
7. 只有字面 `latest` 走 latest；非法 resume 字符串直接失败。显式 resume 找不到内容时
   也不隐式 fresh，调用方必须省略 `resume` 才表示新 worker。
8. worker 结束后只有 chain body 成功写回且更新行数恰为 1，才向调用方追加
   `sub_agent_session_id` marker；usage 统计失败只告警，因为 history 已 durable。
9. chain failure 以稳定 typed kind 跨到 runtime。`Finalize` 表示 worker 已可能产生副作用，
   `stage_run` 必须 non-retryable stop；`RestoreExact` 只有已有同一 exact id 时才可有界重试，
   不允许任何错误路径把它转成 fresh worker。

## 不变量

- trace/transcript key 与 DB persistence UUID 不得再共用同一字段或互相改写。
- Enumeration continuation 只允许使用返回的精确 `sub_agent_session_id`；禁止
  `resume:"latest"` 或 fresh worker 替代。
- 精确 chain UUID 不是资源授权；DB load 必须同时校验当前 session 与 agent ownership。
- 持久化失败不能输出可恢复 marker，也不能被普通 gate retry 再派发成重复扫描。
- 不重置 `StageRunReentryGuard`，不手工修改 operation/worklist/coverage 数据。
- session identity wiring 只修复持久化能力，不扩大 stage、org、target、exact-origin、
  工具或网络授权边界。

## 验证

- 非 UUID event session + 有效 persistence UUID 能创建 chain，并保留原 event session。
- legacy 裸 UUID event session 在未显式注入 persistence UUID 时仍能创建 chain。
- exact miss/error/wrong-owner/malformed history 均在工具执行前失败且 create count 为 0；
  final update 失败不产生 marker，Postgres 0-row update 视为错误。
- typed failure policy 覆盖 `RestoreExact` / `RestoreLatest` / `CreateFresh` / `Finalize`，证明
  final persistence failure 不会触发第二次 worker dispatch。
- runtime、bridge、reflector、nested delegate 和测试构造点均显式传递正确字段。
- `golish-sub-agents` / `golish-agent-runtime` / `golish-agent-bridge` focused + full tests、
  clippy、fmt 通过；Test1 fresh Enumeration 能在第一段结束后返回精确 chain id 并继续同链。
