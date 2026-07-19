# Scoping 空子公司结果的可信落库与重试收敛

## 背景

Scoping 在用户明确选择“包含持股比例大于等于 51% 的子公司”后，会调用
`recon_discover_subsidiaries`。当 ENScan 成功但没有符合阈值的子公司时，正确语义是
`checked_empty`：provider 已执行、结果为空、该空结果应写入 evidence ledger 与 source outcome。

现有运行时只从已冻结的 `harness_org_id` 取得 evidence organization。Scoping 尚未 finalization，
因此这个字段为空，即使工具参数中的根组织已经由本轮 `ask_human` 明确授权，落库仍会失败并把
成功的空结果改写为 `completion_state=partial`。同一修复循环还暴露了两处生命周期契约漂移：

- Scoping gate 只识别旧的数组式 `unit_review` response，不识别前端当前的 `{ "rows": [] }`。
- exact scope derivation 要求历史中只能有一次可解析选择；repair 再问一次后，即使最新选择明确，
  仍返回 `scope_decision_choice_ambiguous`。

## 目标

1. 成功但零条符合条件的子公司结果必须持久化为 evidence-backed checked-empty。
2. Scoping 阶段不能直接信任模型提供的 organization UUID；只允许本 operation、exact stage
   execution、同一 root organization 且最新用户选择为 included 的请求落库。
3. `unit_review` 同时接受当前对象协议和历史数组协议。
4. repair 产生多个同 root 选择时，以最新可解析的人类选择为 authoritative；旧选择保留审计历史。
5. 不改 schema/migration，不扩大主动扫描权限。

## 设计

### 可信的 Scoping evidence organization

`golish-db::operation_scope_decisions` 新增只读授权查询。它复用 exact Scoping lifecycle 的身份约束：

- operation 未 supersede、当前 stage 为 `scoping`；
- stage execution 属于该 operation、kind 为 `scoping` 且仍为 `started`；
- organization 是 operation project scope 下的根组织；
- exact lifecycle 中最新一条可解析、同 root 的 `subsidiary_scope_choice` 为 `included`。

运行时在已有 `harness_org_id` 时继续使用原路径。只有
`StageKind::Scoping + recon_discover_subsidiaries` 且缺少 frozen org 时，才解析工具参数中的
organization UUID，并调用上述 DB 授权。任何身份不完整、选择缺失、latest choice 为 root-only 或
DB 错误都 fail closed。授权只用于 passive discovery 的 evidence/outcome persistence，不授权 EAS、
Enumeration、Vuln 或其他 recon tool。

### checked-empty 与返回真值

ENScan 的 `normalized N record(s)` 是 provider 原始归一化数量，不等于符合 `>=51%` 控股阈值的
子公司数量。`promoted_children=0` 且 provider completed 时，现有 structured fact mapper 生成
`GOLISH-INTEL-SUBSIDIARY empty`。落库成功后 tool result 明确写
`outcome_persisted=true`；只有证据或 source outcome 写入失败时才为 false/partial。

### Scoping lifecycle 收敛

`approved_unit_review` 从成功的人类响应中接受：

- 当前协议：JSON object 且包含数组字段 `rows`；
- 历史协议：顶层 JSON array。

scope derivation 从 exact ordered lifecycle 中选择最后一条可解析、同 root 的 human choice。若最新
choice 为 included，proposal/review 只能从该 choice 之后查找，禁止复用旧轮次的 stale review；若
最新 choice 为 root-only，直接派生 root-only unit set。没有任何可解析 choice 时仍 fail closed。

## 安全不变量

- 模型参数不是授权源；DB 中 exact operation/stage/human lifecycle 才是授权源。
- 空结果与未执行严格区分，checked-empty 必须带 evidence/source outcome。
- 历史选择不删除、不覆盖；latest choice 只决定本轮最终派生语义。
- 不修改 scope snapshot schema，不绕过 Scoping finalization，不授权外部主动请求。

## 验证

- 单元测试覆盖 `{rows: []}` 与 legacy array review。
- embedded Postgres 测试覆盖 included choice 的 passive recon 授权、latest root-only 撤回授权、
  multiple-choice derivation 收敛。
- runtime 测试覆盖仅 Scoping subsidiary discovery 能在 DB 授权后解析 requested root，其他缺 org
  路径继续拒绝。
- focused nextest、Clippy、rustfmt、JSON/diff checks，最终 `just precommit`。
