# Session generation ownership and isolated-history recovery

> 状态：Accepted（2026-07-10）。扩展
> `2026-07-10-per-bridge-task-single-flight.md`：owner 从单个 bridge 提升到稳定的
> logical session slot，并补齐 isolated Task history 的 abort/panic 恢复。

## 1. 问题

单个 `AgentBridge` 的 atomic gate 能阻止同一实例并发，却不能阻止
`init_ai_session` 把 map 换成一个带全新 gate 的 bridge。旧请求仍持有 `Arc` 时，旧/新
bridge 会用同一 session id 并发写 events、transcript 和 DB。`shutdown_ai_session` 也可能
被一个“已 clone、尚未 begin”的旧 sender 绕过：sender 在 remove 后 acquire 旧 gate，
再把 cancel 清回 false。

另一个独立缺口在 Task history isolation：旧实现把 durable conversation history
`mem::take` 到跨 await 的局部变量。future abort 或 panic 会丢弃局部 backup，使下一个
请求看到空历史或 subtask 临时历史。

bridge replacement 还有两个同源入口：per-bridge background listener 若直接 retire-old
再 subscribe-new 会漏 completion，重叠订阅又会重复 evidence；full session restore 若操作
application-global sidecar，则恢复结果不会进入该 bridge 实际使用的 per-session backend。

## 2. Stable session slot

`AiState` 为每个 logical `session_id` 保留跨 bridge generation 的 slot：

- `SessionRequestSlot`：共享 `in_flight`、`current_generation`、`accepting`；
- per-session lifecycle mutex：串行 init 与 shutdown，但不同 session 仍可并行；
- bridge map：只保存当前 generation 的 `Arc<AgentBridge>`。

每个 bridge 绑定 `(slot Arc, generation)`。request acquire 同时要求：

1. stable slot 的 single-flight CAS 成功；
2. slot 正在 accepting；
3. bridge generation 等于 current generation。

lease 的 `belongs_to` 同时比较 slot identity 和 generation。foreign bridge lease、旧
generation lease 都不能初始化 executor 或清理另一 bridge。

shutdown 后 tombstone 只有在 bridge map 为空、`AiSessionSlot` 无 lifecycle clone、且内部
`SessionRequestSlot` 无 late bridge/request lease 引用时才可 GC。只检查 wrapper Arc 会让
old active owner 与 same-id 新 slot 并行，禁止。busy shutdown 当下不能 prune 时，后续
任一 init 会做同样强引用条件的 opportunistic sweep，避免不同 closed tab tombstone 累积。

### Init

`init_ai_session` 在 provider/bridge 构建前用 `try_lock_owned + try_begin_transition`
fail-fast 预约 session。预约覆盖构建、configure 和 publish：

- old request running → 立即 busy，old bridge 不替换；
- concurrent init → 立即 busy，不排队重复昂贵构建；
- candidate 在最后一个 setup await 后预订阅 background broadcasts，但不启动处理；
- publish transition 内 activate/bind、继承 old `pending_background` Arc、replace、retire old，
  再同步启动 new listener；transition Drop 前新 request 不能进入；
- output/completion broadcast clone 共享 exactly-once claim；old retirement 只 drain `rx.len()`
  固定快照，避免 gap、重复 evidence 和持续流量下无界追尾；
- 构建失败 Drop reservation；首次 init 的无 bridge tombstone随后安全 prune。

### Shutdown

shutdown 等待该 session lifecycle，然后按固定顺序：

1. invalidate generation / `accepting=false`；
2. 从 map remove 当前 bridge；
3. 对 returned Arc 设置 cancel，并清理 background jobs。

因此 late old clone 永久 stale。若 shutdown 在 init 构建期间到达，它在 init publish 后继续
invalidate/remove，最终状态仍是 shut down，不会被候选 bridge 复活。

request acquire 在 gate CAS 前采样 cancellation epoch；取得 owner 后只有 epoch 未变化才清
旧 cancel。CAS 与 reset 之间到达的 Stop 会推进 epoch并保持可见，busy contender仍不能清
active owner 的 cancellation。

## 3. Isolated-history recovery

`AgentBridge` 增加同步 recovery slot。进入 isolated scope 时：

1. 先恢复任何旧 backup；
2. 取得 history write lock 并 `mem::take`；
3. 不经过任何 await，立即把 durable history 写入同步 recovery slot；
4. 执行 isolated future；
5. normal success/error/Stop 先取得 history write lock，再从 recovery slot take 并覆盖。

abort/panic 不会清 recovery slot。last lease Drop 的 Release 发生在栈展开之后，因此 backup
已经可见；下一次 `begin_top_level_request` 在 reset 后、任何 execution 前完成恢复。恢复
必须先 await history lock、后 take backup，避免恢复 future 自身被 cancel 时丢 backup。

## 4. 其它 history mutation ingress

- `retry_compaction` 必须持 owner 覆盖 CompactionStarted、summarizer await、history/context
  更新；busy 不 emit、不 compact。
- full `restore_ai_session` 必须持 owner覆盖 history、agent mode，以及该 bridge 自己的
  `SessionCaptureBackend` end/find/resume/start；不得操作 application-global sidecar。
- frontend clear 必须 backend 成功后再 clear local timeline；只在明确 command unavailable
  时调用 legacy command，busy/error 不二次 clear。

## 5. 验证合同

- busy replacement 保留 old bridge；A done 后 C 得到新 generation；
- shutdown 后 late clone 永久拒绝，新 generation 不会让旧 generation 复活；
- concurrent init fail-fast，不同 session 可同时 owner；
- init-vs-shutdown 最终 shutdown 胜出；
- foreign lease 拒绝；
- isolated success/error/Stop、真实 task abort、async panic 都恢复原 history；
- retry/full restore busy 零副作用；frontend busy 保留 timeline 且不走 legacy fallback。
- listener handoff 对 publish 前 queued 与 overlap completion 都不漏且只处理一次；old/new
  generation 共用 pending-note queue；spawn 本身失败也广播一个 terminal completion；
- active old owner/late clone 存活时 tombstone 不 prune且 same-id init busy，全部 drop 后才 GC；
- Stop 落在 owner CAS 与 cancel reset 间仍保持 cancelled。
