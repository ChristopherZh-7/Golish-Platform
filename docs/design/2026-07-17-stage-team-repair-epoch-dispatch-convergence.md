# Stage Team repair epoch 派工与失败状态收敛

## 背景

实体会话 `pentest-chat-1784267739419-1` 的 External Attack Surface Company Controller 在首次
`submit_stage_deliverable` 后被 Gate 要求重试 5 个 exact Web Origin。首次提交关闭了 dispatch epoch 0；
Gate BLOCK 的 durable repair transaction 随后把 TeamPlan 打开到 epoch 1，并让同一个
`leader:primary` WorkItem、WorkerRun 和 message chain 继续。

Controller 在 repair turn 调用 `stage_team_dispatch_workers` 时，应用层已确认 TeamPlan epoch 1 处于开放
状态，但 `stage_worker_requests` 的父 WorkItem 复合外键仍要求
`request.dispatch_epoch = parent_work_item.dispatch_epoch`。稳定 Controller WorkItem 按历史身份合同仍冻结在
epoch 0，因此插入命中
`stage_worker_requests_parent_work_item_id_team_plan_id_..._fkey`，没有创建 Request、WorkItem 或 WorkerRun。
前端又从 tool args 预画 assignment，在没有逐项 durable decision 时把该终态 persistence failure 显示为
永久 `queued`。

Controller 随后直接执行 `eas_fingerprint_web_stack`，第三次 transport probe 把 5 个 origin 以
`producer_outcome=blocked`、`independently_confirmed=true`、
`wrapped_transport_terminalized=true` 落库，Gate 因 DB truth 完整而 PASS。这证明 producer/Gate 合同有效，
故障仅存在于 repair epoch 的 child dispatch authority 与失败 UI 收敛。

## 目标

1. Gate repair 打开新 epoch 后，同一个稳定 Company Controller 可以合法创建当前 epoch 的 child Request 与
   WorkItem。
2. 跨 epoch 父身份只对 exact `company_controller + leader:primary + durable repair/resume authority` 开放；
   普通 Team、foreign owner、关闭 epoch、stale lease/checkpoint 继续 fail closed。
3. Request 与 accepted child WorkItem 仍绑定当前开放 epoch；只解除“稳定父 Controller 必须出生于当前
   epoch”的错误假设。
4. 任何没有 accepted Request 的终态 dispatch persistence failure 都显示为 error，不再显示 queued。
5. 不改历史行、不重写 evidence、不放宽 operation/org/project/scope authorization。

## 设计

### 1. 父 WorkItem 外键拆分 epoch 语义

新增 additive forward migration：

- 删除 `stage_worker_requests` 当前包含 `dispatch_epoch` 的 parent WorkItem 复合外键；
- 改为引用 `stage_work_items` 已有的无 epoch owner unique tuple：
  `id + team_plan_id + operation_id + stage_execution_id + stage_run_unit_id + scope_snapshot_id + organization_id`；
- 保留 Request 自身 `dispatch_epoch`，以及 accepted child WorkItem 与 Request 的当前 epoch 复合外键；
- 不更新任何历史数据。

父 WorkItem 的 epoch 表示它最初被创建的 generation；Request 的 epoch 表示本次派工所属 generation。对稳定
Controller 而言两者可以不同，但 owner identity 必须完全相同。

### 2. DB-authoritative 跨 epoch 授权

替换 `enforce_stage_worker_request_contract()`，在 parent epoch 与 request epoch 不同时要求：

- TeamPlan `coordination_mode=company_controller`；
- parent 是该计划唯一 `stable_key=leader:primary`、role 等于 leader/aggregator role、非 barrier WorkItem；
- Request 仍等于当前 `plan.dispatch_epoch`，且 plan open；
- 当前 epoch 存在 `building|sealed` 的 `stage_team_repair_generations`（active repair 使用 `building`），其 manifest exact 绑定该 leader WorkItem 和当前
  Controller WorkerRun；或存在已 applied 的 successor-Turn resume authority exact 绑定同一 leader/current
  epoch；
- parent WorkerRun owner tuple 仍由现有 FK、lease、attempt、checkpoint 和 runtime transaction 重验。

非 Controller 请求继续要求 `parent_work_item.dispatch_epoch = request.dispatch_epoch`。即使绕过 Rust repo
直接写 SQL，DB trigger 也保持同样边界。

### 3. Runtime typed failure

`request_stage_worker` 在调用 DB INSERT 前执行同一语义的显式检查，给不合法跨 epoch 返回稳定 conflict code，
而不是把原始 FK 名泄漏给模型。合法 repair Controller 继续走现有 request hash、dedupe、scope reauthorization
和 child WorkItem 原子创建。

### 4. UI 收敛

`SubAgentDetailView` 继续优先消费 `requests[].decision`。当 dispatch tool 已终态 error、没有任何 accepted
Request/child，且 args 中存在 workers 时，把 args-derived assignment 标为 dispatch error，并展示 tool result
中的稳定 code/error。该状态只描述“派工调用失败”，不伪造 durable rejected decision；accepted/partial success
仍以逐请求结果为准。

## 安全不变量

- 新 Request 与 child 永远只能写入当前开放 epoch。
- 跨 epoch 只允许 server-authored stable Controller；模型不能通过 role/objective/prose 自报 authority。
- operation、execution、unit、scope snapshot、organization、plan、WorkerRun fence 任一不一致都拒绝。
- 普通 Team parent 仍必须同 epoch；历史 repair/gap/evidence 行保持 immutable。
- UI error 不能被解释为 durable Request 已创建，也不能触发假 child drill-in。

## 验证

- DB RED/GREEN：epoch 0 Controller → close → Gate BLOCK reopen epoch 1 → resume same Controller → dispatch child，
  修复前复现 FK，修复后 Request/child 均在 epoch 1、parent 仍 epoch 0。
- DB negative：无 repair authority 继续 typed 拒绝；既有 contract 回归继续覆盖普通 Team、关闭 epoch、foreign owner 与 stale fence。
- Runtime：合法 repair dispatch 返回 `dispatch_accepted`；非法 parent epoch 返回 typed code，不出现 raw FK。
- Frontend：实体同形态 `STAGE_TEAM_DISPATCH_PERSIST_FAILED`/tool error 显示 error，queued 为零。
- 相关 nextest、Vitest、Clippy、rustfmt、TypeScript/Biome、migration/JSON/diff checks 与 `just precommit`。
- 按用户明确要求不运行 `init.sh`。
