# Company Controller finalizer 重启恢复设计

## 背景

`vuln_triage` 的 Company Controller 已持久化 deliverable，确定性 Gate 也已 PASS，但 final seal 失败时，当前实现直接返回错误，仍把 final submitter 留在 `running` 且持有租约。进程重启后，通用 Stage Team reaper 会把这个 Controller 当成普通 producer 计算 attempt fuel；当历史 `attempt_epoch` 已超过 WorkItem 的 `max_attempts` 时，它会把 Worker 标为 `failed`、WorkItem 标为 `exhausted`，并写入不可变的 `stage_team_attempts_exhausted` output。closed TeamPlan 仍指向该 Worker，下一次 claim 因而报 `stage_team_final_submitter_not_replaceable`。

本设计不删除、覆盖或伪造这条终态历史，也不修改 migration/schema。

## 不变量

1. 已写入的 `stage_worker_outputs` 保持不可变；历史 `failed/exhausted` 组合不得原地复活。
2. 只有 closed Company Controller plan 的 exact final submitter、exact leader item、exact durable submission 和 server-authored lease-expiry output 全部匹配时，才允许自动恢复。
3. final seal 失败不消耗 producer attempt fuel；它是确定性 closeout 重试，不是新的扫描尝试。
4. 运行态替换保留 operation-scoped evidence、technique outcomes 和上游 handoff，只替换当前阶段的 execution/unit/team 壳。
5. 自动替换发生后必须结束当前 top-level request，因为内存中的 `stage_execution_id` 已失效；下一次独立 continuation 才能使用新 execution。
6. 无 exact durable submission、存在活动 tool、identity/row version 漂移或 barrier 不完整时 fail closed，不做自动替换。

## 新失败的正常路径

final submission 持久化且 Gate PASS 后，如果 materialization/final seal 报错：

- 在同一 DB 事务中校验 exact operation/execution/unit/plan/item/worker/submission fence；
- 把 leader WorkItem 从 `running` 经 `retry_pending` 回到 `queued`；
- 把 final submitter Worker 从 `running` 回到 `queued`，写入 server-authored finalization retry checkpoint，递增 checkpoint version，并清除租约；
- TeamPlan 保持 closed，`final_submitter_worker_run_id` 保持不变；
- 返回明确的 `company_controller_finalization_failed` halt，要求下一独立请求恢复同一 Worker/message chain。

启动 reaper 同时增加相同语义：如果过期 clean Worker 是 closed plan 的 exact final submitter，且存在与当前 worker/attempt/lease 一致的 durable submission，则无视 producer `max_attempts`，只做 exact requeue，不写 attempts-exhausted output。

## 已被错误终态化的历史路径

claim closed plan 时若发现 `failed` final submitter + `exhausted` leader item：

1. 先验证 immutable output 的 kind/failure code 确为 `stage_team_attempts_exhausted` / `stage_team_worker_lease_expired`；
2. 验证 exact durable deliverable submission 与 worker/attempt/lease identity；
3. 以 witness-bound CAS 调用现有 stage checkpoint replacement 事务：旧 execution/units/workers 变成历史，建立同 stage 的新 execution 和 frozen-scope units；
4. `fact_purge=None`，所以已落库扫描事实和 evidence 不删除；只失效当前阶段自己尚未完成的 handoff；
5. claim 返回 `stage_team_final_submitter_runtime_replaced`，runtime 转成 `company_controller_runtime_recovered` halt。用户下一次发送“继续”时进入新 execution。

这个 rollover 是追加式运行态恢复，不把不可变失败 output 改写成成功。

## 错误与 UI 合同

- `COMPANY_CONTROLLER_FINAL_SEAL_FAILED`：finalizer 已安全排队，下个独立请求重试 closeout。
- `COMPANY_CONTROLLER_RUNTIME_RECOVERED`：历史坏运行态已自动替换，当前请求必须停止，下个独立请求使用新 execution。
- `STAGE_TEAM_OPERATOR_RECOVERY_REQUIRED`：存在 outcome-unknown tool 或 identity 不完整，仍需人工恢复。

`tool_dispatch` 必须识别前两种 halt，不能让同一 assistant turn 继续 submit 或把它二次泛化为业务 Gate BLOCK。

## 定向验证

- DB：超过 producer attempt 上限的 exact final submitter 仍由 startup reaper requeue，普通 producer 仍 terminalize。
- DB：final seal failure parking 保持 plan closed/pointer 不变，清租约并可由 aggregator exact reclaim。
- DB：历史 `failed/exhausted` + immutable lease-expiry output + exact submission 会 no-purge rollover；缺任一 witness 不替换。
- Runtime：runtime-recovered/finalization-failed 都生成明确 halt，tool dispatch 终止当前 request。
