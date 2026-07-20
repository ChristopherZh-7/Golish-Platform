# Candidate、ContextPack、知识图谱与本地向量闭环

## 背景

Vuln 在独立克隆库 `golish_gatefix_20260720_d` 已有新鲜 Gate PASS，但后续 Candidate 无法建立 exact manifest。与此同时，Memory Fabric 的 Runtime Memory、Assertion、Document 和 Temporal Graph 主干并非整体停摆，却存在三个独立缺口：bound Worker 没有把可信 Unit 身份送进 ContextPack；Target Intel StageEpisode 没有可验证的 fact/evidence lineage；Embedding Projector 和 VectorPrior 没有本地 provider。

本设计把四条链分别闭环，但不降低 Gate、不伪造 evidence、不把 customer-confidential 内容发送到外部服务，也不修改 production 数据库。

## 决策

### 1. Candidate predecessor authority 跟随 exact provenance

Candidate manifest 不再把 `Enumeration.completed_at == Vuln.started_at` 当作唯一 lineage。普通未替换执行仍使用既有严格前驱关系；no-purge replacement 则只在以下身份全部一致时接受：

- fork input 指向当前 exact replacement Vuln execution、Unit、handoff；
- replacement operation state 中的 adoption marker 指向唯一 source Vuln execution；
- source Vuln execution 与 Enumeration 的既有 final-sealed predecessor lineage 匹配；
- organization、operation、scope snapshot、stage、final seal 和 handoff authority 全部一致。

缺失、多义、跨 org/scope、marker 与 fork input 冲突时继续 fail closed。不得按“最近一条”或时间窗口猜测。

### 2. ContextPack 使用 trusted bound-worker identity

ContextPack 的 Stage/Unit/Worker 身份由运行时绑定的 Worker lease/chain context提供，而不是由模型文本提供。顶层 task 即使没有 `stage_run_unit_id`，进入 specialist Worker 时仍能用 exact execution + Unit + Worker 读取 scoped context。没有完整绑定时保持无注入，不做宽松跨 Unit fallback。

### 3. Target Intel Episode 保存真实 lineage

StageEpisode 只引用本次 Target Intel 最终提交已验证的 durable fact/evidence refs。projector 不从 prose 推断、不制造占位 evidence；若 sealed handoff 没有可验证 refs，继续进入明确的 dependency failure。修复点放在 episode payload 构造/最终封存 seam，使 Assertion → Document → Graph 共用同一 lineage。

Target Intel 允许“确定性 Gate 已完整检查、但没有产生 canonical fact/evidence”的合法空结果。这个窄场景由 final-seal transaction 写一条 server-owned `runtime_memory_final_seal_attestation` evidence：内容只绑定 operation/org/execution/Unit/Worker/submission/scope/Gate hash/coverage hash，`target_id=NULL`，不声称发现任何目标事实。首次封存必须不存在同 identity attestation；response-loss replay 必须精确找到唯一一条，否则 fail closed。该 evidence 随 handoff → Episode → Assertion 传播，既不放宽长期知识的正 evidence 要求，也不把“未检查”伪装成“检查为空”。

### 4. 本地向量 provider

- pgvector 与 `vector(1536)` schema 保持不变，不新增 migration。
- Ollama 只允许 loopback endpoint，运行时客户端禁用系统代理和重定向。
- 使用 `qwen3-embedding:4b`，请求显式 `dimensions=1536`，返回维度或数值非法即 fail closed。
- 配置必须显式 opt-in；默认仍关闭。
- 同一个 provider 同时供 Embedding Projector 与 ContextPack VectorPrior 查询使用。
- customer-confidential 文档只允许本地 provider；外部 embedding 继续默认拒绝。
- 只重开因 `memory_embedding_provider_unconfigured` 被 terminal suppress、且上游 Document 已成功的 delivery；使用 CAS，保持幂等，不重开 invalidated/restricted/其他错误。

安装/模型下载可临时走 `127.0.0.1:6152`；服务运行和 Golish 查询只连接 `127.0.0.1:11434`。

### 5. Candidate 大清单使用服务端展开的分组提交

Candidate 仍要求每个冻结 `work_item_key` 恰好一个终态决定，但不再要求模型为几十个同类观察重复输出相同 rationale、evidence id 或完整 key。模型可用 `candidate_decision_groups`：异常/候选项用精确 `work_item_keys`，真正同质的整类观察可用 canonical manifest-kind `work_item_key_prefixes`（如 `surface_analysis:` / `scanner_observation:`）。服务端只在 exact Unit 的不可变 manifest 内展开前缀、为每个精确 key 补回冻结 evidence，再转成既有 `candidate_decisions` 并运行完全相同的完整性、证据和语义 Gate。非 canonical/空前缀、未知/重复 key、选择器重叠、显式 decisions 与 groups 混用均 fail closed。该紧凑输入不持久化为新业务 schema，也不降低一项一决策的最终契约。

## 验收

1. focused regression 证明 no-purge replacement + immutable fork 能建立 Candidate exact manifest，冲突 lineage 仍拒绝。
2. outer Task 无 Unit、bound Worker 有 exact Unit 时注入 ContextPack；解绑/错绑时不泄漏。
3. Target Intel episode 带真实 refs，Assertion/Document/Graph delivery 成功且无虚构 evidence。
4. 本地模型返回精确 1536 维，存量 active documents 获得 embeddings，VectorPrior 可查询；代理故意失效时 loopback仍可用。
5. 重建 CLI 后在克隆库重新 fork Candidate，至少跑到真实 review/approval barrier，再审计各记忆层。

## 2026-07-20 验收结果

- 克隆库 Candidate operation `24a76324-e286-40eb-915d-cfa76682dc98` 对 88 个 frozen work item 完整终态化并真实 Gate PASS：3 个 proposed Candidate、85 个 no-candidate，未创建 Attempt、未执行攻击。
- bound Worker 实际拿到 `canonical=1/runtime=2/vector=3` 的 ContextPack，且 sub-agent 收到 7199 字符 server briefing；Ollama 使用 `127.0.0.1:11434`、`qwen3-embedding:4b`、1536 维。
- source operation 的 EAS/Enumeration/Vuln 已形成 3 条 active Assertion、3 Document、3 Embedding；Temporal Graph 有 active organization entity。Candidate 查询词没有命中图实体时 `graph=0` 是相关性过滤，不是 projector 故障。
- 历史 Target Intel 空 evidence 事件仍以 immutable DLQ 形式保留，不回填或伪造历史；新 final seal 的 focused transaction test证明 server attestation、Episode/outbox 和 response-loss replay 原子且幂等。
- Review 是真实下一步。三份 plan 都是 live-target GET；未得到人类 approve 前不启动 Verification action。另有 no-attack 回归证明全 reject 后仍会产生 evidence-backed checked-empty Verification handoff。

## 非目标

- 不修改 production DB；
- 不删除现有测试库；
- 不放宽 Gate 或 final seal；
- 不新增真实主动攻击；
- 不在本任务加入 ANN 索引或 Graphiti 外部服务。
