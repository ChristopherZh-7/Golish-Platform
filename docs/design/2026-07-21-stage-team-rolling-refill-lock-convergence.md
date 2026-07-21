# Stage Team Rolling Refill 锁收敛

## 问题

共享 `drain_rolling_stage_team_work` 在任一 child 完成后，会先同步等待 refill claim，再继续 poll `FuturesUnordered` 中的 sibling。生产 Vuln Triage run `pentest-chat-1784603212728-1` 证明这个顺序会自锁：一个 sibling 已在 `complete_stage_worker` 事务中持有 `operation_state ... FOR UPDATE`，而 refill `claim_stage_team_item` 等待同一行锁；持锁 sibling 只有再次被 poll 才能提交事务，scheduler 却正阻塞在 claim 上。

这与目标响应速度、Nuclei timeout、Gate 或前端渲染无关。高并发完成只会扩大触发窗口；一旦形成该等待图，继续等待不会恢复。

## 决策

把 claim 也建模为 rolling drain 内的持久 in-flight future，并保持最多一个 pending claim：

1. child futures 继续存放在 `FuturesUnordered`，数量不超过冻结的 company child cap。
2. pending claim 单独存放在第二个 `FuturesUnordered`，长度严格为 0 或 1；不得把临时 claim future 内联到 `select!`，避免另一分支获胜时 drop 一个可能已开始数据库事务的 claim。
3. 当 child 与 claim 同时存在时，用 `tokio::select! { biased; ... }` 同时 poll，优先消费 child completion。这样持有数据库行锁的 completion 始终有机会推进和提交，等待该锁的 claim 随后自然完成。
4. claim 返回 `Some(work)` 时才递增 `claim_sequence` 并启动 child；claim 保持串行，因此 durable worklist 顺序、lease identity 和 claim fencing 不变。
5. claim 返回 `None` 后暂停 refill，直到至少一个 child 完成再允许下一次 claim，防止 repository 暂无可领取项时空转；child completion 可能释放 active-worker额度或生成 retry WorkItem。
6. execution error仍记录第一项并继续排空可领取工作。claim/storage error或 cancellation停止创建新 claim，但已启动 child必须完成现有 landing；已经开始的 pending claim若返回 `Some`，该 worker也必须执行到 landing，不能遗留 durable running worker。

## 不变量与边界

- 不改变 Target Intel、EAS、Enumeration、Vuln Triage 的 company/provider/child并发上限。
- 不改变 scope、工具授权、扫描超时、evidence landing、Worker identity、attempt/retry或Gate语义。
- 同时最多一个 claim；不得 spawn、并行或超时重试 claim，不通过弱化 `FOR UPDATE` 掩盖 scheduler bug。
- 本轮不修改 schema/migration、IPC、前端或当前 production transaction。修复加载前，现有卡住的 run仍需经用户确认后重启/回滚并恢复。

## 验证

用 `Semaphore(1)` 模拟 operation row lock：child 1 持锁并等待 refill 开始；child 2 在确认锁已持有后完成并触发 refill；第三次 claim先唤醒 child 1，再等待同一锁。旧算法稳定 timeout；新算法继续 poll child 1、释放锁、完成 claim与全部三个 child。随后运行所有现有 rolling-drain错误、取消、cap与即时补位测试，以及 `golish-agent-runtime` scoped Clippy/rustfmt。
