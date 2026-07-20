# Vuln 扫描预算耗尽终态设计

## 问题

Vuln DB-authoritative executor 会把 broad Nuclei 超时缩成 single-technique retry，并把自动尝试限制为三次。现有规则对 `scanner_runtime/scan_budget_exhausted` 只设置 `automatic_retry_allowed=false`，但仍把 coverage 写成 `partial`。Runtime 随后既不会创建第四个 shard，又要求所有 coverage cell 都是 `found/checked_empty/blocked/not_applicable` 才能提交，形成确定性的永久死路：例如真实 run 在 `179/180`、`0 exact shards` 时只能返回 `VULN_WORKLIST_EXECUTION_EXHAUSTED`。

不能把 timeout 改写为 `checked_empty`：扫描预算耗尽不证明目标没有漏洞。也不能把任意 exit 124、配置错误、模板错误、parser/authority/DB 错误都写成 `blocked`，否则平台自身故障会被静默洗成 coverage 完成。

## 决策

引入一个窄的 producer-owned `budget_blocked` 终态，只允许可信 foreground wrapper deadline 在有界、最大预算恢复后产生。

1. Nuclei failure attribution 只有同时满足以下 runner tuple 才分类为 `scanner_runtime/scan_budget_exhausted`：
   - `status=timeout`；
   - `error_kind=COMMAND_TIMEOUT`；
   - `launch_guard_revalidated=true`；
   - foreground runner 已 kill、wait、drain 后返回。
   裸 `exit_code=124`、缺 launch guard 或 shape drift 只能是 `runner_failure/wrapper_failure`。
2. 可信 deadline attribution 带版本化 `failure_witness=foreground_wrapper_deadline_v1`，进入 wrapper result 与 evidence raw payload。该 witness 只由后端 runner result产生，不接受模型参数。
3. 普通 primary 仍为 300 秒；single-technique narrowed retry 使用 wrapper允许的最大 600 秒。连续同 exact operation/target/origin/tool/technique 的同类预算失败达到第三次，且当前尝试是 600 秒可信 deadline、请求精确包含一个 technique、没有 positive match时，producer 把 report 转为 `BudgetBlocked`。多 technique 或已解析出阳性的 partial结果不能被预算终态覆盖。
4. 为兼容已经由旧代码停在第三次 `partial + automatic_retry_allowed=false + scan_budget_exhausted` 的 durable operation，runtime只创建一次新的 `budget_recovery` shard（attempt 4，600 秒，stable key包含新 shape）。旧 evidence 只授权这次新执行，不能被离线直接改写为 blocked；必须由升级后的 wrapper再次真实执行并产生新的可信 witness。
5. `BudgetBlocked` 仍按 exact target/org/operation authority先追加 `vuln.nuclei_observation` evidence，再通过 attempt-generation guarded CAS 把 `technique_outcomes` 写成 `blocked`。任一 evidence、authority、CAS 或 DB write失败都保持 partial，不得 terminalize。
6. `blocked` 表示“此 technique 在当前 operation 的有界最大扫描预算内未能完成”，不是“没有漏洞”。最终 Gate可以带该明确限制收口；Finding/Candidate不会由 blocked产生。

## 不变边界

- `target_transport` 的三次稳定同类 breaker保持原 `TransportBlocked` 语义。
- `operator_cancelled`、template snapshot/proof、parser、authority、DB、普通 runner/config failure仍非终态；它们不能使用本规则。
- 不改 Gate terminal集合、DB schema/migration、IPC或前端 generated类型。
- 不做无 durable cursor 的模板级增量拼接。现有 broad→single-technique narrowing加600秒最大预算是本次可审计的最小执行形状；若未来需要跨多个模板批次聚合，必须另行设计持久 batch cursor、阳性累计和 generation-safe final publish。

## 当前 run 的恢复语义

应用加载新 backend 后，对同一 Vuln operation发一个独立“继续”请求。Runtime应从旧的第三次 budget-exhausted evidence创建唯一 attempt-4 `budget_recovery` shard：若600秒内完成，落 `found/empty`；若可信 deadline再次耗尽，落 evidence-backed `blocked`。无需重跑 Scoping、EAS或Enumeration，也不直接修改 live DB。

## 定向验证

- Runtime：旧 attempt-3 scan-budget partial只生成一个 attempt-4 budget recovery；普通 runner/config exhaustion仍无 shard；narrowed/budget recovery预算均为600秒。
- Pentest wrapper：可信 timeout tuple与裸 exit124分离；第三次最大预算可信 timeout产生 BudgetBlocked；混合/legacy/duplicate generation与普通 runner failure不产生 blocked；landing映射 blocked并保留 witness/network-attempt证据。
- Gate回归：backend-owned Vuln blocked仍可terminal，partial/error仍不能关闭coverage。
