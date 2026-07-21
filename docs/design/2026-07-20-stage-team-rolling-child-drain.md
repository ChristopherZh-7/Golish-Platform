# Stage Team Rolling Child Drain

> Rolling refill与child completion的数据库锁收敛由 `docs/design/2026-07-21-stage-team-rolling-refill-lock-convergence.md` 补充；本文其余并发、安全与错误合同继续有效。

## 问题

Target Intel、External Attack Surface、Enumeration 与 Vuln Triage 的 Company Controller 都通过 `drain_company_controller_children` 执行 durable child WorkItem。当前实现按 `child_cap = max_workers_active - 1` 先 claim 固定一批，再用 `join_all` 等整批全部结束，最后才 claim 下一批。

这不是并发上限不足，而是补位策略错误。真实 Vuln run `pentest-chat-1784481969848-1` 的 plan 允许两个 child 槽位；前七批都同时启动一个约 0.5–1.1 秒的快速 N-day 分片和一个约 170.9–227.5 秒的通用 Nuclei 分片。快速分片结束后，第二个槽位在慢分片结束前一直空闲。相同结构也会影响普通 LLM SubAgent：两个 child 中一个先完成时，Controller 不会领取第三个。

## 决策

把共享 child drain 改为有界 rolling refill，不为 Vuln建立专用旁路：

1. 继续从冻结的 `TeamPlan.max_workers_active` 计算 per-company `child_cap`，Controller 占一个槽位；不改变任何 stage spec 数值。
2. 用 `FuturesUnordered` 保存当前调用方拥有的 child future。初始只 claim 到 `child_cap`；任意 future完成后立即重新调用 durable claim，直到本地 in-flight再次达到上限或 repository返回当前无可领取 WorkItem。
3. repository返回 `None` 只代表当前快照没有可领取项。若仍有本地 in-flight，等待下一项完成后必须再次尝试 claim，因为该完成可能释放 active-worker额度或生成 retry WorkItem；只有本地 in-flight为空且 claim仍为 `None` 时才结束 drain。
4. 单个 child execution error继续只记录第一项错误，不取消 sibling，也不停止领取剩余 durable工作；队列排空后才向上层传播第一项 execution error，保留现有 barrier/recovery语义。
5. claim/storage error或用户 cancellation出现时停止领取新 child，但已经开始的本地 future必须继续到各自工具 lifecycle/evidence landing边界；全部落稳后再返回该终止错误。不得因 rolling refill提前 drop正在执行的工具。
6. 每个 child仍需取得既有 global provider semaphore permit；company级 `buffer_unordered(company_cap)`、DB claim fencing、Worker lease、scope authorization、tool timeout/rate、evidence与Gate规则保持不变。

## 安全边界

- 不增加 company、provider或child并发上限，不改变 Nuclei `-rl/-c`，不扩大目标、工具或 capability。
- 不改 schema/migration、IPC、Worker identity、attempt/retry预算或 operator-recovery所有权。
- rolling refill只利用 plan已经授权但被固定批次浪费的槽位；任意时刻 durable active Worker仍由 repository的 `max_workers_active`确定性拒绝越界。
- 本轮不缓存 Nuclei template-list预检。后续生产run证明所谓独立DB landing deadlock实际由共享drain在poll child前同步await refill触发；锁收敛修复见上述2026-07-21补充设计。共享 drain仍必须确保一个 child错误不会跳过其它可领取 sibling。

## 验证

新增确定性 async测试模拟两个槽位：child 1快速完成、child 2被通知量阻塞、child 3仍在 child 2释放前被 claim并启动。再覆盖最大 in-flight从不超过cap、execution error仍排空 queued sibling、claim/cancellation停止新领取但等待已启动 child落稳。

只运行 `golish-agent-runtime` focused nextest、该crate scoped Clippy与rustfmt；不运行未获授权的 init/precommit或全workspace门禁，不启动新的真实外部扫描。
