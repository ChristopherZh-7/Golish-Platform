# CLI/GUI 共享数据库阶段测试分叉设计

## 1. 目标

为 headless CLI 增加基于既有 GUI/CLI operation 的阶段测试分叉：测试者可以选择 `target_intel` 至 `attack_candidate` 的一个阶段或连续切片，创建新的测试 operation，采用源 operation 的 Scoping 组织范围和分叉创建时数据库中的 Target 快照，只运行所选阶段，不重跑严格前缀，同时保持 GUI 与 CLI 共用同一个 `TaskOperation`、`TaskOrchestrator`、Gate、Company Controller 和 Candidate Wave 内核。

源 operation 是只读 authority。分叉不能修改、reset、resume、伪造或复制源 Worker、tool call、submission、handoff、evidence；新阶段产生的 runtime、消息链和 evidence 全部属于新 operation。

## 2. 用户语义

新增入口：

```bash
golish /workspace \
  --stage-run-fork <pentest-chat-key|stage-run-key|session-uuid|operation-uuid> \
  --only vuln_triage \
  -e "重新测试漏洞阶段"

golish /workspace \
  --stage-run-fork <operation-uuid> \
  --from enumeration \
  --to attack_candidate
```

- `--only` 运行一个阶段；`--from` 与 `--to` 必须同时出现并运行连续 DAG 切片。
- 分叉不允许从 `scoping` 开始。需要重新执行 Scoping 时使用普通 `--stage-run`。
- profile、project scope、组织范围和 subsidiary 集合来自源 operation，不能用 CLI 参数改写。
- 默认使用 GUI 相同的应用数据库；`--ephemeral-db` 与 fork 冲突。
- chat/session selector 只有在恰好绑定一个 operation 时才可使用；有歧义必须改传 operation UUID。
- 源 operation 可以是 waiting、running、failed 或 finished，但必须有完整、未失效的严格前缀 authority。读取和创建事务会锁定并复验源 authority，避免与 reset/supersede 竞争。

## 3. 非目标

- 不把全局 `(organization_id, stage_kind)` completion ledger 当作前置完整性证明。
- 不自动采用数据库里“最新”的组织、operation 或 target。
- 不把源 operation 的 historical Target rows 冒充成 GUI 当时的快照；当前 schema 没有历史 Target snapshot。产品语义明确为“分叉创建时当前数据库快照”。
- 不自动批准 Candidate Review、Verification、主动工具授权或 Gate BLOCK。
- 不添加 CLI 专用 stage executor、Gate 或 Candidate 算法。

## 4. 不可变 Stage Fork lineage

新增 additive schema：

### 4.1 `operation_stage_forks`

一行绑定一个新 operation 和一个源 operation：

- `operation_id`：新测试 operation，主键。
- `source_operation_id`：源 GUI/CLI operation。
- source/target scope snapshot、project scope、profile、runtime/attack contract。
- `entry_stage`、`terminal_stage`、严格前缀 `adopted_stage_kinds`。
- fork manifest JSON 与 SHA-256。
- 全行 immutable，source 与 target operation 不得相同。

### 4.2 `operation_stage_fork_inputs`

每个 `(operation_id, source_stage_kind, organization_id)` 一行，冻结精确 source authority。Scoping 没有普通 Worker handoff，其 authority 是 sealed scope decision/snapshot + passed root Unit + workerless submission；其余阶段冻结 source execution、Unit、Worker、handoff、submission、scope/payload/evidence/coverage/Gate hash。DB trigger 按两种真实形状分别重验，不能为 Scoping 伪造一个普通 handoff。

Scoping 只要求 root organization 的 final seal；Target Intel、EAS、Enumeration、Vuln 对 source scope 中每个组织都要求一份 final seal。采用阶段必须构成所选入口的无洞严格前缀。

### 4.3 `operation_stage_fork_targets`

在创建事务内冻结当前数据库的 Target rows：组织、ordinal、live target id、type/value/scope/source、canonical identity hash。主动阶段至少需要一个 canonical `scope=in` Target；运行时授权精确读取这份 operation-bound snapshot，并复验 live target 没有跨 project/org/identity 漂移。

## 5. 原子创建

扩展 `CreateRuntimeOperation`，增加可选 `StageForkCreate`。同一短事务完成：

1. 锁定 source operation、source sealed scope 和 project scope。
2. 校验 canonical workspace 的 `project_scope_id` 与 source 精确相等。
3. 校验 source profile 和冻结 runtime/attack contract；新 operation 采用同 profile，部署合同必须兼容。
4. 创建新 session 下的 Task、operation、initial selected-stage execution。
5. 以 `reuse_reconfirmed` 决策克隆 source 组织成员关系到新 sealed scope；每个 approval source 记录 source snapshot provenance。
6. 冻结当前 Target snapshot。
7. 锁定、重验并写入严格前缀 fork inputs。
8. 计算 canonical manifest hash，插入 fork header。
9. commit 后才允许 provider/tool dispatch。

任一步失败都回滚新 Task、operation、execution、scope、fork 和 Target snapshot；事务内不执行 HTTP、LLM、扫描器或 MQ。

## 6. 前置 authority 解析

所有下游读取采用同一规则：

1. 当前 operation 已产生该 predecessor final seal 时优先当前 authority。
2. 否则只有 `operation_stage_fork_inputs` 明确列出的 adopted predecessor 才能回源。
3. 每次读取重验 source operation 未 supersede、handoff 未 invalidated、hash 与 fork manifest 一致。
4. 缺失、重复、scope 漂移、evidence 漂移或 source reset 均在 provider/tool dispatch 前 fail closed。

该 resolver 服务三条路径：

- 通用 `load_inherited_stage_handoffs` 上下文。
- Vuln 的 final-sealed Enumeration origins 与 `enumeration_surface_manifest`。
- Candidate initial Wave/manifest 的 Enumeration + Vuln final seals、TechniqueOutcomeSet 与 observation/support evidence。

## 7. Candidate 特殊合同

普通 Candidate 路径继续要求同-operation相邻 Enumeration→Vuln→Candidate。Fork-only Candidate 通过显式第三种 Wave entry `ForkedVulnHandoff`：

- `attack_wave_units.entry_stage_fork_input_id` 指向当前 target operation 的 Vuln fork input。
- direct Vuln handoff、FactDelta consolidation、fork input 三种 entry 严格 XOR。
- Candidate Wave/Unit、work items、decisions、Candidates、review 和 Attempts 全部属于新 operation 和新 scope snapshot。
- 初始 manifest 只从 fork manifest 指定的 source Enumeration/Vuln handoff与 source `TechniqueOutcomeSet` 重算；禁止按 org 查“最新”。
- source evidence 只允许作为 initial manifest 的 observation/support，以及由该 exact work item生成的 Candidate support/rationale。Candidate Attempt proof/refutation/blocker/fact_delta 必须来自新 operation。
- evidence-owner trigger 仅为 exact fork input 中冻结的 evidence id 打开窄例外；同 org 其它 source evidence 仍拒绝。
- Candidate Review 不因 fork 自动通过；`--only attack_candidate` 在 Candidate Gate 后正常结束，不跨入 Verification。

## 8. CLI/GUI 内核一致性

Fork adapter 只负责 selector、source preflight 和 typed `StageForkCreate`。实际执行保持：

```text
CLI fork adapter
  -> prepare_task_operation
  -> PreparedTaskOperation::run_stage_fork
  -> TaskOrchestrator::run_stage
  -> TaskOrchestrator::run_from_stage
  -> shared stage_run / Gate / Team / Candidate Wave
```

GUI full run、CLI fresh slice、CLI fork 的差异只在 operation 创建 authority和入口 cursor，不得分叉 stage logic。

## 9. 验证标准

- parser/selector：参数冲突、terminal source、歧义和 foreign workspace fail closed。
- DB：schema/FK/XOR/immutability、source tamper、Target drift、缺失组织 predecessor、事务回滚。
- Vuln：只使用 adopted Enumeration origin/manifest，source 不修改，前置阶段无新 tool call。
- Candidate：source GUI Vuln PASS 能创建 Candidate-only fork；缺 Enumeration/Vuln final seal、truncated watermark、outcome/evidence/hash漂移均在 provider 前失败。
- parity：fork 仍调用共享 `run_stage -> run_from_stage`；现有 fresh、exact resume、Candidate review和 evidence-owner测试保持通过。
- 集成：对本地 fixture DB 创建 GUI-shaped source，运行 fork 至选定 terminal，证明新 operation隔离、source row counts/hashes/status不变。
