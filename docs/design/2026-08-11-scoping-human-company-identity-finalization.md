# Scoping 人工公司身份确认的服务端闭环

## 问题

`recon_lookup_company` 对低置信度唯一候选会先冻结 `needs_human` receipt。现有协议要求 Agent 在 `ask_human` 成功后再次调用同一工具并传入 `selected_candidate_id`，该第二次调用才冻结 `confirmed` receipt。实体 operation `9bc8fd1c-7f03-44ca-ba58-42ada29e5baa` 中，Human 已确认唯一候选、根组织已创建、子公司策略已选择，但模型漏掉第二次 resolver 调用；Scoping finalizer 因 `scoping_confirmed_company_identity_missing` 正确 fail closed。

## 决策

Scoping finalizer 增加一个窄的服务端恢复步骤，不再把协议正确性寄托在模型是否记得重复调用工具。该步骤只接受同一 operation、同一 Scoping execution 中的完整 durable witness：

1. 最新 receipt 是 `needs_human`，且没有既有 `confirmed` receipt；
2. receipt 内候选集合与后续 `ask_human` 的 `decision=company_identity` context 精确一致；
3. Human response 是原始 options 中的一个非 Other 选项，并且只匹配一个候选的 canonical legal name 与注册标识；
4. 后续成功的根 `manage_organizations(create)` 把该 canonical name 映射到 finalizer 声明的 exact root organization；
5. 组织属于 operation 的 canonical project 且不是子组织。

满足全部条件时，在 finalizer 的同一 operation-locked transaction 中追加 immutable `human_selected/confirmed` superseding receipt；候选/provider evidence、Human ToolCall 和 create ToolCall 都进入 receipt authority。歧义、Other、skip、失败结果、跨 operation/stage/org、名字或注册标识漂移均保持拒绝。

## pre-freeze exact resume

第一次 Gate BLOCK 后 operation 仍处于合法 pre-freeze shape：active Scoping execution 已存在，但 confirmed receipt、`engagement_org_id`、scope snapshot、Unit/Worker 尚未写入。旧 exact-resume selector 要求先有 `engagement_org_id` 才允许恢复，形成“必须恢复 finalizer 才能写入、必须已写入才可恢复”的循环。

该窄状态的 relational authority 只允许由完整 expected identity 解开：caller 必须显式传 exact `--expect-org`，DB 再只读重算同 operation/execution 的 needs-human candidate set、唯一 Human response、后续 root create 与 canonical project/root identity。只有全部一致时，resume 暂时携带该 root 进入同一 Scoping finalizer；成功后 operation 与 sealed scope 仍由 finalizer 正式绑定。缺 selector 或 durable witness 的普通 NULL organization 状态继续拒绝。

## 不做的事

- 不把普通组织行、模型自然语言 claim 或提交摘要当作 Company Identity authority。
- 不修改旧 `needs_human` receipt，不删除 ToolCall 或 evidence。
- 不触网重查 provider，不要求用户再次确认同一个候选。
- 不放宽 `finalize_scoping_scope` 对唯一 confirmed receipt 的既有检查。
- 不让 `--expect-org` 本身成为授权，也不改变非 Scoping 或 post-freeze resume 合同。

## 验收

- 嵌入式 PG 回归先证明当前完整 Human/Create witness 仍以 missing-confirmed 失败。
- 实现后同一用例自动追加唯一 confirmed receipt 并完成 Scoping finalization。
- 缺 create、Other/skip、候选/组织漂移仍 fail closed。
- 对 retained operation 只做正常 resume，让生产 finalizer消费既有 ToolCall；不手工改库。
- retained resume 必须从 pre-freeze shape 收敛到 Gate PASS、sealed root-only scope 与下一阶段 cursor。
