# Stage Team 重叠派工、begin fence 与 child drain 收敛

## 背景

实体 Vuln Triage session `pentest-chat-1784364775375-1` 在同一 Controller dispatch 中接受了两个
不同 `dedupe_key`、但 role/kind/objective/空 `subject_refs` 完全相同的 whole-company child 请求。
两个 Worker 并发消费同一 durable worklist，其中一个在 `begin_worker_tool` 遭遇 PostgreSQL `40P01`
deadlock。`finish_worker_tool` 已有 `40P01`/`40001` 有界事务重试，但 begin 路径没有；runtime 又把任意
begin error 无条件标为 lease lost，导致同一 Worker 的 failure landing 被本地 lease flag 拒绝。

同批另一个 Worker 在 40 iteration 上限后已把稳定 WorkItem 重新排队；但是 child drain 对
`join_all` 的每项结果使用 `result?`，第一项错误直接终止 claim loop。最终一个普通 child failure 被包装为
`COMPANY_CONTROLLER_FAILED`，`stage_run` 发出 `halt_current_request`，Task 进入 waiting，排队重试没有在同一
请求中被再次领取。

## 目标

1. 同一 Controller tool call 不得以不同 `dedupe_key` 重复提交同一个规范化 assignment。
2. `begin_worker_tool` 与 finish 一样，对 PostgreSQL `40P01`/`40001` 做有限次完整事务重试。
3. begin fence 的 storage/transient error 不得伪造 lease loss；只有 DB 明确返回 `LeaseLost` 才污染本地
   lease 状态。
4. 一个 child 的错误不能阻止同批其他已完成、已排队重试或后续可领取 WorkItem 收敛；drain 应先耗尽当前
   可领取工作，再把仍未收敛的首个错误返回上层。
5. 保留 genuine lease loss、outcome-unknown side-effect tool、Gate BLOCK 与 request-terminal halt 的现有
   fail-closed 语义。

## 设计

### 1. Batch assignment identity

runtime 在所有 `subject_refs` canonicalize 后、任何 durable request 写入前，构造 batch-local assignment
identity：

- trimmed `role`；
- trimmed `kind`；
- 折叠空白后的 `objective`；
- 按 canonical JSON 排序后的 `subject_refs`。

同一 batch 出现重复 identity 时，整个 tool call 以
`STAGE_TEAM_DISPATCH_ASSIGNMENT_OVERLAP` 拒绝，不写部分 WorkItem。`dedupe_key` 继续承担 durable replay
identity；它不能用来绕过 batch-local 语义重叠检查。不同 objective 或不同 canonical subjects 仍可合法派给
相同 specialist，whole-company 空 subjects 也继续允许单项使用。

### 2. Symmetric worker-tool transaction retry

`golish-db::runtime_memory_tx` 抽出同一有界 retry runner，begin/finish 都使用三次上限和短线性 backoff。
只有 SQLSTATE `40P01`/`40001` 可重试；lease/CAS、identity、conflict、missing、connection failure 等错误立即
返回。每次 retry 都重新 `pool.begin()` 并完整执行 operation → Unit → Worker lock/order transaction，绝不
复用 aborted transaction。

### 3. Begin error lease classification

`RuntimeWorkerToolLifecycle::begin` 失败时先结束尚未实际 dispatch 的 generic tool row。只有 typed
`RuntimeMemoryError::LeaseLost` 设置 `bound.lease_lost`；`Storage`、`Unavailable`、`Conflict` 等错误保留 bound
可落状态，使 child executor 能调用既有 `retry_stage_worker` 原子结束当前 attempt并重新排队稳定 WorkItem。
`golish-sub-agents` 的通用 begin wrapper不得再次把任意 lifecycle error折叠为 lease loss；typed 分类与 shared
bound flag由 concrete lifecycle拥有。

finish 路径保持原有 unknown-outcome 策略：工具可能已经执行，finish fence 失败仍停止本地 Worker并交给既有
recovery policy，不能自动假定 side effect 未发生。

### 4. Drain all claimable work before propagation

每轮 `join_all` 后逐项收集结果：terminal/retry-scheduled/exhausted 继续计数；错误记录首项并继续处理 sibling。
claim loop随后继续领取 retry-scheduled 或既有 queued WorkItem。只有当 repository 返回“当前无可领取 child”时，
才返回此前记录的错误；若没有错误则正常完成。这样不会吞掉真正错误，也不会让一个错误把同批可恢复任务留在
队列中。

## 不变量

- 不改 schema、migration、IPC 或 live DB。
- assignment identity 仅用于同一 tool call 的 exact duplicate 拒绝，不从 objective 推导 scope authority。
- canonical subjects 仍由 DB 对 frozen operation/org/project/scope 重验。
- begin retry 不重放外部工具：事务成功后工具才 dispatch；所有 retry 都发生在 dispatch 之前。
- genuine `LeaseLost` 仍禁止旧 fence failure landing。
- drain 不把错误改成成功；耗尽可领取工作后仍把未收敛错误返回现有 Controller gap/halt 路径。

## 验证

- runtime router：不同 dedupe key 的同 identity batch 在 repository 调用前整体拒绝；不同 objective 合法。
- DB retry runner：两次 retryable error 后成功；non-retryable error只执行一次；SQLSTATE closed policy不变。
- runtime lifecycle：Storage 不污染 begin lease，typed LeaseLost 会污染。
- drain result collector：错误与 RetryScheduled 同批时保留错误且继续计数可恢复结果。
- focused nextest、受影响 crate Clippy、rustfmt、JSON/diff checks；不运行未获授权的全 workspace 门禁。
