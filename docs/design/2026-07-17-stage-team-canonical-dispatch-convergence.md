# Stage Team canonical 派发与拒绝状态收敛

## 背景

实体会话 `pentest-chat-1784256664463-1` 的 Enumeration Company Controller 把 21 个 Web Root
分成两个 `content_enumeration` 请求。Controller 传入的 `subject_refs` 为
`{"target_id":"…","target_url":"…"}`；durable repo 只接受带 `kind` 判别字段的
`CanonicalFactKey`，因此两个请求都以 `stage_team_request_subject_not_canonical` 拒绝，没有创建
child WorkItem 或 WorkerRun。EAS 的派发没有传 `subject_refs`，命中了
`organization_scope_implicit=true` 的合法公司级授权，因此没有出现同一问题。

全部拒绝时，runtime 只向模型返回通用 `STAGE_TEAM_DISPATCH_NONE_ACCEPTED`，丢弃已经持久化的逐请求
decision。前端随后从 tool args 预画两张 assignment 卡；因为 result 没有 decision，失败的工具被显示为
永久 `queued`。Controller 又拥有本阶段 producer tools，于是模型把合同错误误判为 worker/budget 问题并
直接执行 Enumeration。

## 目标

1. Company Controller 看见明确、provider-compatible 的 target subject ref schema。
2. 对实体会话已出现的 `{target_id,target_url}` selector 做安全、确定性的 canonicalization：只保留
   server 可重验的 target UUID，URL 继续只作为 objective/tool 参数，不成为授权。
3. 同一 target 的多个 Web Origin 只产生一个 canonical Target ref，避免重复 key 被 repo 拒绝。
4. 全部拒绝仍返回每个 durable request 的 decision/decision_code，模型和 UI 都能看见真实原因。
5. 历史通用错误结果也显示为 rejected/error，不能永久显示 queued。
6. DB 的 frozen operation/org/project/scope 校验保持不变；不新增 schema/migration，不把 URL 或模型文本
   提升为授权。

## 设计

### 1. 模型可见合同

`stage_team_dispatch_workers.workers[].subject_refs[]` 公开为 target-only canonical selector：

```json
{"kind":"target","target_id":"<uuid>"}
```

整公司派发可以省略 `subject_refs`，由现有 `organization_scope_implicit` policy 授权。`target_url` 不属于
canonical ref；需要把多个 exact Web Origin 交给 child 时，它们留在 bounded objective，child 的 producer
tool仍按 `target_id + target_url` 做 operation/org/scope 重验。

### 2. runtime canonicalization

host 在计算 request hash 和写 durable request 前 canonicalize refs：

- 已是合法 `CanonicalFactKey`：序列化回 canonical JSON，去掉多余字段；
- 只有 `target_id` 和可选 `target_url` 的已知 selector：解析 UUID，转换为
  `CanonicalFactKey::Target`；
- 其它结构：返回 `STAGE_TEAM_DISPATCH_WORKER_INVALID`，不猜测 kind；
- canonical JSON 按首次出现顺序去重。

这只是输入规范化，不是 authorization。规范化后的 key 仍进入 `request_stage_worker`，由 golish-db 在同一
事务校验 exact frozen operation、organization、project path 和 in-scope target。

### 3. durable rejection 回传

当 `accepted_count == 0`，runtime 返回 `success=false`，但 payload 保留：

- `status=dispatch_rejected`
- `accepted_count=0`
- `rejected_count/request_count`
- 完整 `requests[]`（request id、dedupe key、decision、decision_code、created_work_item_id）
- `next_action`，要求修正 canonical refs 或在确属整公司 scope 时省略 refs 后重试。

只有 `dispatch_accepted` 才能把 Controller park 并让 scheduler drain children；拒绝结果不能伪装成 barrier。

### 4. UI 收敛

新结果直接消费 `requests[].decision=rejected`。对旧 transcript 的
`STAGE_TEAM_DISPATCH_NONE_ACCEPTED`（无 requests）使用 exact code fallback，把原始 workers 卡显示为 error；
其它未知失败不凭 prose 推导 decision。

## 安全不变量

- target URL、objective、模型 prose 均不是 scope authority。
- target shorthand 只可收敛为 Target UUID；foreign/out-of-scope UUID 仍由 DB 拒绝。
- 去重只合并同一个 canonical key，不合并不同 target，不改变 objective 中的 exact origins。
- rejected request 永不创建 WorkItem/WorkerRun，UI 不得显示为 queued/running。
- Company Controller 仍可合法选择零个 child；本修复只消除“尝试派发但合同错误后伪装排队”的路径。

## 验证

- golish-sub-agents schema test：target ref 必须包含 `kind + target_id`，不允许 `target_url`。
- golish-agent-runtime RED/GREEN：实体 selector 规范化、重复 target 去重、全部拒绝逐请求回传。
- frontend RED/GREEN：历史通用 rejection 不再渲染 queued。
- 相关 crate nextest、Vitest、Clippy、rustfmt、TypeScript/Biome、JSON/diff checks。
- 按用户要求不运行 `init.sh`；完整 `just precommit` 仅在用户未禁止时作为最终 DoD。
