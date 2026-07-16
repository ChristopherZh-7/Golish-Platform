# Stage Team 中断工具确定性恢复设计

- **日期**：2026-07-16
- **状态**：Approved；用户要求处理中途停机后的 same-chain 恢复，并明确要求使用 CLI 验证、不要运行 `init.sh`
- **范围**：Stage Team Worker 在前台工具执行中应用退出、工具结果未落库时的恢复与 Gate 解锁；首期覆盖 Enumeration crawler 与四个 EAS coverage wrapper
- **非目标**：降低 Gate、接受自然语言完成声明、直接重放旧参数、高风险/有写副作用工具的自动恢复、DB schema 变更

## 1. 问题与用户合同

真实 Enumeration Worker 曾在原 `WorkerRun/message_chain_id` 上启动
`enum_crawl_same_origin_urls`，Katana 子进程随后随应用退出消失；最新 EAS 现场则在
`eas_fingerprint_services` 已部分落下 SERVICE evidence 后退出。两者都让 `tool_calls` 保留
`running`，没有最终 ToolResult。
现有 startup reaper 正确地把 Worker/WorkItem 停在 `recovery_required`，但 operator resolution 只能把该
WorkItem 终结为 `exhausted + blocked`。Controller 的 sibling barrier 因此可以结束等待，Enumeration Gate 却会永久
保留未覆盖 cells，后续“继续”只能重复 BLOCK。

恢复合同是：

1. 保持原 operation、Unit、WorkItem、WorkerRun 与 `message_chain_id`；恢复只是同一 Agent Thread 的新 Turn。
2. 已落库的 evidence、technique outcome 与 worklist 是恢复位置的唯一事实源；模型必须先刷新它们，不能凭历史 prose
   猜测工具做到哪里。
3. 服务端只对白名单内的 bounded/read-only/idempotent capability 允许自动重新排队；旧调用本身先原子终态化，绝不复用
   旧 lease，也不在 DB transaction 内执行外部工具。
4. 新 Turn 不直接重放旧 args。它收到 host-owned recovery directive，先调用 `stage_worklist_status` 和
   `stage_worklist_next(prefer=["pending","error","partial"])`，再只处理返回的缺口。
5. 无法证明安全、预算已耗尽、identity/CAS 漂移或高风险工具继续 `recovery_required`，由 operator 明确处置。

## 2. 恢复分类

### 2.1 `resume_after_reconcile`

首期 closed allowlist 允许：

- `enum_crawl_same_origin_urls`；
- `eas_probe_http_liveness`；
- `eas_discover_ports`；
- `eas_fingerprint_services`；
- `eas_fingerprint_web_stack`。

这些 wrapper 共同满足：

- 后端重新验证 current workspace、organization 与 exact in-scope target/origin；
- 调用有界、前台、只读，且没有授权边界之外的副作用；
- crawler 输出只作为 seed；EAS wrapper 则把权威进度按 asset × technique cell 幂等落库；
- 新 Turn 先读取 current worklist，terminal cell 不重做，只为仍缺失的 exact cell 调用对应 capability。

因此旧 active tool 在 lease 过期后可被标成 `failed`，result 写入
`stage_team_interrupted_tool_reconciled`；同一 Worker/WorkItem 清除旧 lease 与 active tool 后回到 `queued`。claim
递增 `attempt_epoch`，继续同一 `message_chain_id`。

### 2.2 `manual_required`

除上述 closed allowlist 外全部保持人工恢复，包括通用数据写入、浏览器权威采集、route probe、漏洞探测、攻击、付费或
不可判定副作用工具。工具名未知、tool row 已终态但不符合既有 lease-fence 特例、attempt budget 耗尽时也不得自动恢复。

### 2.3 后续可扩展的 `adopt_terminal`

若未来某 capability 能以 exact tool-attempt generation 证明全部预期效果已经原子落库，可增加“采用已完成结果”分支。
当前 crawler 没有权威持久化结果，不能伪造 tool success，因此首期只做 reconcile 后 same-chain 补缺口。

## 3. 事务与状态机

`claim_stage_work_item` 在普通 claim 之前锁 plan/item/worker/tool，并执行：

```text
recovery_required|expired running Worker + exact allowlisted active tool
  ├─ policy != resume_after_reconcile       -> 保持 recovery_required
  ├─ attempts_used >= max_attempts           -> 保持 recovery_required
  └─ allowlisted bounded crawler/EAS wrapper
       1. tool_calls running|received -> failed(outcome_unknown_reconciled)
       2. Worker checkpoint += host recovery marker
       3. Worker -> queued，清 active tool/旧 lease，保留 WorkerRun + chain
       4. WorkItem -> queued，递增 row_version
       5. 同一 claim transaction 重新 claim，attempt_epoch + 1
```

所有更新使用 exact plan/work item/worker/tool identity、attempt epoch、checkpoint version、row version 与 lease token；
任何 CAS miss 都 fail closed。不会写 `stage_team_recovery_decisions`：该表保留 operator 的 immutable
`mark_blocked_outcome_unknown` 决策，本次是 server-owned scheduler reconciliation，不冒充人工选择。

## 4. Agent 行为与 Gate

Worker checkpoint 中写入 `stage_team_interrupted_tool_recovery.v1`。runtime 构造本次 child objective 时识别该 marker，
追加 host-owned 指令：

1. 先刷新 authoritative worklist/coverage；
2. terminal cells 不重做；
3. 只在当前页仍有对应 capability 的 exact gap 时才重调中断 wrapper；
4. 后续 producer 按当前 worklist 补齐 cells；`ready_to_submit=true` 后才提交。

这样 Gate 本身没有放宽；它仍只接受当前 operation 的 durable evidence/outcome。修复的是“WorkItem 被永久终结、没人再补
Gate 缺口”的 scheduler 状态机。

## 5. 验证

- DB integration：active `enum_crawl_same_origin_urls` / `eas_fingerprint_services` 经 startup park 后，下一次 claim 返回相同
  WorkItem/WorkerRun/message chain，tool 已 failed，attempt epoch +1，checkpoint 带恢复 marker。
- Negative integration：`query_target_data` 等非白名单 active tool 仍 `recovery_required` 且不可 claim。
- Runtime unit：恢复 marker 只生成 worklist-first directive；普通 checkpoint 不生成。
- CLI targeted nextest + clippy + fmt；按用户要求不运行 `init.sh`，也不依赖 GUI 或真实外部扫描。
