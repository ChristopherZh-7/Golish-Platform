# 工具真值、Hypothesis Registry 与协作式 Verification 双循环设计

> **状态**：Approved for implementation planning（用户于 2026-07-29 确认）
>
> **日期**：2026-07-29
>
> **范围**：TargetIntel / ExternalAttackSurface / Enumeration / VulnTriage 的工具落地契约，Attack Candidate 的假设分析模型，Verification 的多 Agent 验证循环，具体动作授权、Gate、Reporting 与 Investigation Workspace。
>
> **授权边界**：本文只定义目标架构、兼容迁移边界和验收标准；不授权修改数据库 schema/migration、生成 IPC 类型、执行真实扫描、调用外部服务或切换 rollout。

## 1. 决策摘要

Golish 的现有 scope、organization isolation、evidence ledger、lease/CAS、action journal、response-loss recovery、Finding lineage 和确定性 Gate 应继续保留。需要重构的不是这些安全骨架，而是工具结果、Candidate 和 Verification 之间的业务语义。

本设计作出以下决定：

1. 每次工具执行必须产生统一、可核对的 Tool Truth / Landing Receipt。进程成功、事实落库、覆盖完整和安全结论是四件不同的事。
2. 产品中的 Candidate 阶段改为 **Hypothesis Analysis**。一个公司隔离的主分析 AI 动态调用多个只读分析 subagent，完整分析前序事实、Application Model、覆盖缺口和 residual，并逐条建立、合并、拆分、反驳或补充 hypothesis。
3. `Hypothesis Registry` 成为假设的 canonical system of record。现有 `attack_candidates` 在迁移期降为可执行兼容投影，不再代表全部假设。
4. Hypothesis 不以现有 adapter 是否存在为生存条件。缺少 adapter、凭证或前置信息必须保留为 readiness/capability 状态，不能变成 `NoCandidate` 或“未发现问题”。
5. Verification 改为真正的多 Agent Campaign。团队围绕一个 hypothesis 提出多种可证伪策略，逐轮编译具体动作、授权、执行、判定、反思和重排计划。
6. Nuclei、HTTP、browser、sqlmap 等都是可选 typed capability adapter，不是 Verification 阶段本身，也不是认知角色。
7. Candidate/Hypothesis 与 Verification 形成外层演化循环；单个 Verification Campaign 内部还有策略循环。旧 revision 和 sealed Wave 不原地改写；material 新证据通过 typed evolution event 形成下一 generation，无语义变化只写 fixed-point receipt。
8. 后台可以继续使用队列控制并发和副作用，但前端不再以 FIFO queue 作为产品模型。Candidate 和 Verification 两个 roadmap 节点打开同一个 operation-level Investigation Workspace，默认进入不同视图。
9. hypothesis 本身不要求人工批准。所有主动动作先编译成可查看、可哈希的 Prepared Action；只读低风险动作可按 policy 自动批准，高风险、有凭证、有写入或多请求动作必须在具体请求包生成后 JIT 审批。
10. Gate 的控制流与覆盖等级分离。允许继续不等于完整覆盖；用户界面和报告必须明确区分 `PASS`、`PASS_WITH_GAPS`、`BLOCKED` 以及 residual risk。

### 1.1 非目标

本文不做以下事情：

- 不用 PentAGI 的通用 Flow 引擎替换 Golish 的 operation/stage/scope/evidence 内核；
- 不把 Candidate 和 Verification 合并成一个拥有推理、授权、执行和 Finding 写入权的超级 Agent；
- 不给模型 raw shell、任意 `pentest_run`、任意 browser 或绕过 adapter 的逃生口；
- 不把所有 coverage gap 强行变成漏洞 hypothesis；
- 不让 AU/Application Model、模型 confidence 或自然语言总结充当漏洞证据；
- 不回写或重新解释历史 sealed operation；
- 不在本设计任务中创建 migration、修改生成类型、调用 provider 或执行真实目标验证。

### 1.2 改动规模判断

这是一次**产品语义和数据权威层面的较大改造**，因为 Candidate 要从执行候选队列变成 Hypothesis Registry，Verification 要从单 Attempt/replay 变成 durable Campaign，工具结果也要补统一 landing/coverage contract。它不是“只改每层 Gate 最终状态”或“只重画 Candidate 页面”能解决的问题。

但实现不应 big-bang 重写。Golish 已有的 operation/stage DAG、scope、organization isolation、evidence、lease/CAS、action journal、recovery 和 Finding lineage继续复用；新能力用 additive entities、operation-frozen contract 和 legacy projection 分四个计划逐步切入。换句话说：**架构概念要认真改，运行内核不推倒，部署必须渐进。**

## 2. 为什么现状会让人感觉漏测

### 2.1 Candidate 实际是 adapter-gated replay admission

当前主链是：

```text
Vuln / Enumeration frozen work item
  -> Candidate | NoCandidate
  -> machine-policy approval
  -> CandidateAttempt
  -> usually one allowlisted replay recipe in current fresh V2
  -> verified | refuted | blocked
```

现有关系模型保留 action journal 和扩展空间，并非数据库永远只能容纳一个 action；问题是 fresh V2 classifier、private recipe 和默认 action budget 目前通常把一次 Attempt 收窄为一个 replay。

关键事实：

- `CandidateHypothesisAuthority` 是授权 envelope，不是持续演化的假设注册表，也不是 ordered strategy plan；见 `backend/crates/golish-agent-kit/src/harness/attack_execution/types.rs`。
- 模型提交协议只有 Candidate/NoCandidate 二态；Candidate Gate 要求 frozen manifest 每项恰好闭合一次；见 `backend/crates/golish-agent-kit/src/harness/attack_execution/decision.rs`。
- `surface_analysis_v1` / `surface_analysis_v2` 不能成为可执行 Candidate，最终会被强制投影为带固定原因的 NoCandidate；见 `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`。
- 当前真正闭环的 observation/executor 主要是 Nuclei match、anonymous access 和 directory entry 三类；名义 technique registry 远大于真实 adapter 能力面；见 `backend/crates/golish-agent-kit/src/harness/attack_execution/classifier.rs`。
- AU/Application Model 只作为只读 context，不会自行创建 hypothesis 或 work item。

因此当前系统把下面三件事混在了一起：

```text
没有 typed observation
没有现成 adapter
没有安全 hypothesis
```

它们在逻辑上并不等价。

### 2.2 Verification 是真实调用，但不是持续协作团队

现有九个认知角色会真实调用模型，但拓扑仍是 Lead 中心的临时串行咨询：

- cognitive child 没有持久 chain、共享 strategy state 或 specialist-to-specialist 协作；
- Evidence Analyst、Refiner 和 Reflector 是 host pipeline，其他角色只在 Lead 主动 consult 时运行；
- Lead 的 action proposal 只有 `capability_id + rationale`；
- classifier 通常只给一个 recipe，当前预算通常只有一个 action；
- specialist prose 不形成 durable typed strategy、critique、oracle 或 refinement artifacts。

因此目前更接近“多个 AI 评论一次固定 replay”，不是 PentAGI 式持续规划、执行和重排。PentAGI 值得复用的是其 Flow/Task/Subtask/Action 和执行后调整计划的动态性，而不是替换 Golish 的证据与授权边界。参考：

- <https://github.com/vxcontrol/pentagi/blob/main/backend/docs/flow_execution.md>
- <https://github.com/vxcontrol/pentagi>

#### 2.2.1 与 PentAGI 的准确差异

PentAGI 官方 Flow 文档描述的是 persistent Flow → Task → system-generated/refined Subtask → Action，并在每个 Subtask 完成后由 Refiner 修改后续计划；Primary Agent 还能委派 pentester、research、coder 等 specialist。这解释了它在“任务怎么继续探索”上的灵活性。以下比较只针对公开执行模型，不把未公开或未来能力当作现状：

| 维度 | 当前 Golish | PentAGI 公开模型 | 本设计取舍 |
|---|---|---|---|
| 顶层组织 | 固定 operation/stage/profile | persistent Flow/Task/Subtask | 保留 stage authority，但在 Candidate/Verification 内引入可演化 Registry/Campaign |
| 动态规划 | Candidate 二态、Verification 通常一次 recipe；重排空间小 | Generator 建 Subtask，Refiner 在执行后增删改后续 Subtask | 引入 immutable obligation + typed plan delta，不复制通用任务队列 |
| 多 Agent | 有真实角色调用，但多为 Lead 临时 consult | Primary 可委派多类 specialist | Candidate 用并行只读 analyst；Verification 用 durable consult/critique/refinement artifact |
| 工具执行 | scope、typed adapter、action journal、lease/CAS、recovery 边界强 | 通用 terminal/browser/agent tools + Docker 执行环境 | 不开放 raw fallback；所有目标动作先编译 Prepared Action 再授权 |
| 证据/Gate | evidence ledger、stage Gate 和 Finding lineage 较强，但 coverage/control 混杂 | 公开 Flow 文档主要描述任务完成与工具结果；这不等同于 Golish 的 domain Gate | 继续以 DB/evidence 为 authority，并补 Tool Truth、oracle、coverage receipt |
| 假设演化 | Candidate 与 Attempt 基本绑定，缺少 canonical Registry | Subtask 可以动态调整，但不是安全 claim 的 revision ledger | 新增 root/revision/evolution/FactDelta 双循环 |
| 主要优势 | 可审计、scope/organization 隔离、确定性闭环、安全恢复 | 探索灵活、计划可调整、通用能力广、agent hierarchy 清楚 | 组合“Golish 的安全骨架 + PentAGI 的适应性” |
| 主要风险 | 容易 adapter-gated 漏假设、binary Gate 误显完整、UI queue 化 | 若直接照搬，通用工具与 done/subtask 语义不足以承担 Golish 的授权、证据和 coverage authority | 只借鉴认知编排，不替换 Golish 的安全 system of record |

因此这不是把 PentAGI 整体塞进 Verification，也不是把 Candidate 改成另一个任务队列。Candidate 负责“完整建立和演化假设”，Verification 负责“围绕一个冻结假设反复规划和取证”；两者通过不可变 FactDelta 循环，而不是共享一段可随意改写的 agent memory。

### 2.3 当前审批没有审阅最终请求包

现有 machine approval 发生在具体 canonical action args 物化之前。审批绑定 hypothesis authority、scope、capability/action allowlist 和 budget；随后数据库从私有 recipe 生成 exact action authority。

这能防止模型篡改服务器 recipe，但不能证明决策者看过最终 HTTP/Nuclei/browser 请求。尤其是模板驱动工具还存在：

- template 内容摘要未完整绑定到 authorization receipt；
- 模板可能生成的 method/header/body/request sequence 不可见；
- rate limit 不等于 total request limit；
- 实际请求数量没有和授权预算逐项核销。

### 2.4 Gate 证明闭环，不证明覆盖充分

现有阶段允许 evidence-backed `blocked` / `not_applicable` 成为终态。这避免无限重试是合理的，但当前 binary PASS 容易把“流程收口”误读成“测试充分”。审计中确认的典型风险包括：

- TargetIntel 的旧 source terminal row 可能跨 attempt 被复用；
- EAS 工具 `exit 0 + empty stdout` 可能被视为 checked-empty；
- Enumeration 单次 preflight 失败可封闭 JS/DIR/PARAM/JSAPI 四轴；
- positive 结果可能掩盖同批次 partial、skipped 或 parser reject；
- Nuclei scanner no-match 被提升成整个 technique checked-empty；
- anonymous/directory 的 2xx oracle 过于粗糙。

因此当前 Gate 更准确的含义是“预期记录均有终态”，不是“红队覆盖充分”。

### 2.5 前端队列是后端模型的直接投影

当前前端并非只在样式上像队列：

- `AttackCandidateReview` 以 approve/reject、plan hash、actions 和 expiry 为中心；
- `CandidateVerificationProtocol` 的根对象是 exact Wave 下的 Attempt queue item；
- Verification 视图依赖历史 Candidate `stage_run` 工具详情和 session-global hint；
- 同一个 Attempt 会在 queue protocol 和 attempt rows 中重复出现；
- read model 没有 campaign round、strategy revision、role activity、request packet、oracle 或跨 Wave hypothesis lineage。

精确现状入口：

- `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx` 只在选中特定 Candidate stage tool 时挂载 review/attempt 视图；
- `frontend/components/Engagement/CandidateVerificationProtocol.tsx` 以 `queue.items` 和 `Queue N · Attempt M` 组织主内容；
- `frontend/components/Engagement/CandidateAttemptRows.tsx` 再次读取并渲染同批 Attempt；
- `frontend/store/types/session.ts` 的 hint 是 session-global 单 cursor；
- `frontend/services/ai-events/harness-handlers.ts` 的 terminal/consolidation event 只能刷新已存在 hint，不能代替 operation bootstrap。

`AttackCandidateReview.tsx` 仍包含 manual approve/reject 和客户端 `executionPlan: unknown` 解析。这是 legacy plan UI；fresh hypothesis-JIT machine policy 下通常退化为审计视图，而且不包含最终 request packet，不能继续用“展示了 actions/budget”暗示 informed action approval。

只重排 CSS 不会解决问题。必须先改变 canonical read model 和入口。

### 2.6 审计证据摘要

| 已确认状态 | 代码位置 | 对设计的影响 |
|---|---|---|
| surface analysis 无 executor contract并被强制 NoCandidate | `golish-agent-kit/.../classifier.rs`、`golish-agent-app/.../harness_submit_tool.rs` | canonical Hypothesis 不能依赖旧 Candidate 二态 |
| fresh V2 主要只有 Nuclei/anonymous/directory typed adapter | `golish-agent-kit/.../classifier.rs`、`golish-pentest-app/.../verification_capabilities.rs` | capability registry 必须显示真实闭环矩阵 |
| machine approval 早于 canonical action args 物化 | `golish-db/.../attack_candidate_approvals.rs`、`candidate_attempts.rs` | Prepared Action 必须先编译、后审批 |
| cognitive children 无 durable chain/team state | `golish-sub-agents/.../response_parsing.rs` | strategy/critique/refinement 必须成为持久 typed artifacts |
| EAS empty stdout、Enumeration broad blocked、positive masks partial | `golish-pentest-app/.../eas_capabilities.rs`、`enum_preflight_web_origins.rs`、`js_extract_apis.rs`、`anonymous_access.rs` | Tool Truth 必须有逐 input completion 与 coverage strength |
| Verification UI 是 single-Wave Attempt queue | `CandidateVerificationProtocol.tsx`、`CandidateAttemptRows.tsx`、`candidate_recovery.rs` | 新 UI 需要 operation-level read model，不能只改组件样式 |

## 3. 目标架构：三个事实平面、两个循环

### 3.1 三个事实平面

```text
Tool Truth Plane
raw execution
  -> raw witness
  -> normalized observation
  -> business fact
  -> evidence
  -> coverage receipt

Hypothesis Plane
facts + AU context + gaps + contradictions
  -> hypothesis proposal
  -> canonical hypothesis revision
  -> readiness / verification objectives

Verification Campaign Plane
hypothesis revision
  -> B-owned verification plan / proof paths
  -> objective-local campaigns and strategies
  -> prepared actions
  -> authorization
  -> execution
  -> oracle
  -> evidence assessment
  -> objective outcomes / FactDelta
  -> revision-level aggregate adjudication
  -> verified | refuted | nonterminal
```

三个平面必须通过 typed、versioned、可哈希的接口通信。任何一个平面都不能靠自然语言声明替代下一平面的 authority。

所有“全集/当前集合”authority统一使用open header → canonical ordered members → host/DB重算count/hash → seal；sealed header/member append-only，post-seal追加、漏member、caller自报count/hash或读取unsealed set一律拒绝。event/head、lease和budget CAS另用append-only event + expected version，不把mutable head冒充immutable exact set。

### 3.2 两个嵌套循环

外层是 hypothesis 演化循环：

```text
Hypothesis Registry H(g)
  -> Candidate analysis seal
  -> Verification Wave W(n)
  -> FactDelta bundle D(n)
  -> consolidation
  -> material: Hypothesis Registry H(g+1)
     no change: fixed-point receipt on H(g)
```

内层是 Verification 策略循环：

```text
Campaign round
  -> parallel read-only consults
  -> strategy selection
  -> prepared action
  -> authorization
  -> single active action
  -> typed oracle
  -> evidence assessment
  -> refine / continue / terminalize
```

外层保证新事实能够形成新猜想；内层保证验证不是一次固定工具调用。

## 4. Tool Truth / Landing Contract

### 4.1 状态必须正交

一个工具结果不能只用 `success` 或一个 coverage outcome 表达。统一 contract 至少包含：

| 轴 | 枚举 | 含义 |
|---|---|---|
| `attempt_state` | `not_started / running / succeeded / failed / outcome_unknown / exhausted / superseded` | attempt 生命周期；exhausted/superseded 不冒充 observation |
| `landing_state` | `not_attempted / partial / committed / failed` | 结果是否完整进入 canonical persistence |
| `observation_state` | `found / no_match / indeterminate / not_applicable` | 本次观察看到了什么 |
| `coverage_extent` | `none / complete / partial / sampled / template_only` | 本次方法相对冻结分母覆盖了多少 |
| `coverage_gap_reason` | `none / transport / tool_failure / parser_reject / budget_exhausted / unsupported / policy_blocked / source_unavailable` | coverage 不完整的原因 |
| `reconciliation_state` | `pending / consistent / orphaned / superseded` | raw、typed、business、evidence 与 coverage 是否一致 |
| `security_interpretation` | `not_assessed / signal / proof / refutation / inconclusive` | 对 exact claim 的安全解释 |

`found` 只证明至少发现一项，不自动证明 coverage complete；`no_match` 也不自动等于 refutation。

`checked_empty` 不是额外的模糊状态，而是以下组合的派生投影：

```text
attempt_state = succeeded
landing_state = committed
observation_state = no_match
coverage_extent = complete
coverage_gap_reason = none
reconciliation_state = consistent
all sealed denominator inputs = completed
```

即便满足 checked-empty，它也只是“这个 exact producer/method 完整运行且没有 observation”，不是 hypothesis refutation。普通 producer 最多写 `not_assessed | signal`；只有 versioned trusted oracle 可以写 `proof | refutation | inconclusive`。

非法组合必须 fail closed，例如：

- `outcome_unknown + landing committed`；
- `no_match + coverage partial + checked_empty`；
- `reconciliation orphaned + terminal coverage publish`；
- `not_applicable` 没有 server-owned applicability rule；
- `proof/refutation` 没有 oracle id/version/digest；
- `superseded` attempt 的结果进入 current denominator。

batch roll-up 是确定性函数：

- `observation=found` 只要至少一个 applicable input found，同时保留兄弟 input 的 partial/gap；
- `observation=no_match` 只在所有 applicable input 均 complete no-match 时成立；
- 其余混合结果为 indeterminate；
- `coverage_extent=complete` 只在 denominator 全项 complete；sampled/template-only 永不升级 complete；
- 任一 pending/outcome_unknown/orphan/authority drift 使聚合 incomplete；
- 所有 input not_applicable 只有在每项都有 server rule 时才能聚合 not_applicable。

### 4.2 Capability Execution Receipt

每个 producer 必须在同一 attempt authority 下生成 `capability_execution_receipt.v1`，包含：

1. **Authority**
   - operation、stage execution、unit、organization；
   - worker run、attempt epoch、scope snapshot；
   - exact target/origin/technique；
   - capability、adapter、recipe 和 parser version。
2. **Input manifest**
   - 规范化输入全集、数量和 hash；
   - 每项 `attempted / completed / skipped / failed`；
   - 批处理不能只以进程 exit code证明每项已处理。
3. **Raw witness**
   - tool/version、固定参数 recipe；
   - started/completed timestamp；
   - exit/signal/timeout；
   - stdout/stderr hash、truncated flag；
   - raw artifact ref/hash。

raw witness 的持久化只允许走 host-owned encrypted vault callback：vault 在完成 AEAD seal、decrypt/re-hash attestation 与 retention binding 后返回不可由 caller 构造、不可序列化的 `VerifiedVaultSnapshot<'guard>`；typed landing、reconciliation 与 authority-set seal 只能在该 guard lifetime 内消费同一份 plaintext snapshot。公开 DTO 只能传播 artifact id、content hash 和 opaque locator hash/ref token，不能传播可解析 locator、workspace path、caller 自报的 raw hash/size 或 caller 拼出的 snapshot member `Vec`。明文不得落 workspace 或数据库；authority root/member census 必须由 host/repo 从已封存 receipt 与 vault attestations 重建。
4. **Typed landing**
   - schema/parser version；
   - accepted/rejected observation 及拒绝原因；
   - source offset 与 target binding。
5. **Persistence receipt**
   - business row ids/count；
   - evidence ids；
   - technique/coverage outcome ids；
   - transaction/CAS identity。
6. **Actual budget**
   - requests、bytes、runtime、retries、browser steps、OAST tokens；
   - planned 与 actual 对账。

发布协议必须分段，不能把外部执行伪装成数据库原子事务：

```text
begin transaction
  -> freeze authority/input manifest/budget reservation
  -> commit begin receipt
external execution (no DB transaction held)
  -> durable raw-witness staging
closeout transaction
  -> typed observations/business facts/evidence membership/actual counters CAS
  -> finalization receipt
reconciliation
  -> terminal coverage publish
```

raw artifact 丢失、closeout commit 失败和 response loss 分别进入稳定恢复状态；任何情况都不能自动重放有副作用的外部动作。Verification 的 `action_execution_receipt.v1` 必须引用恰好一个 `capability_execution_receipt.v1`，而不是形成第二套重叠执行真值。

只有 raw witness、typed landing、business persistence、evidence 和 coverage outcome 一致时，才允许发布 terminal coverage。

### 4.3 Reconciliation 与 orphan 检测

Gate 前必须运行确定性 reconciliation：

- raw artifact 声称发现事实，但 accepted observation/business row 为零时，标记 `landing_orphan`；
- typed observation 已写但 evidence 缺失时，标记 `evidence_orphan`；
- evidence 已写但 coverage outcome 未引用时，标记 `coverage_orphan`；
- input manifest 有未解释 item 时，coverage 不能是 complete/checked-empty；
- late result 的 attempt epoch 与 active epoch 不一致时只能记录为 superseded witness，不能污染当前 Gate。

### 4.4 blocked 与阶段推进

`blocked` 继续是合法事实，但不再等于完整覆盖。Gate 输出拆成：

- `control_decision = allow | hold`；
- `coverage_grade = complete | degraded | incomplete`。

UI 映射：

| control | coverage | 用户可见状态 |
|---|---|---|
| allow | complete | `PASS` |
| allow | degraded | `PASS_WITH_GAPS` |
| hold | incomplete/degraded | `BLOCKED` |

stage/org/wave 开始时必须先 seal `coverage_denominator.v1`，绑定 exact inputs、required controls、expected producer/campaign ids、expected receipts 和 authority hash。调用方只提交 stable stage/wave request identity；repo 必须在同一事务锁定 server-owned wave/stage facts，自行导出 canonical asset × technique exact set、重算 count/hash 并封存，不能接受 caller 传入 items、count、manifest hash 或先读后写的 authority snapshot。聚合只按该冻结分母计算：

- `complete`：所有 denominator item 完整处理且 reconciliation consistent；
- `degraded`：所有 item 已终态，缺口可定位、有 stable reason/owner/residual，不含 outcome_unknown 或 authority corruption；
- `incomplete`：仍有 pending/outcome_unknown、scope/auth drift、orphan、evidence corruption 或未解释 input。

`allow + incomplete` 与 `hold + complete` 是非法组合。`allow` 只表示控制流可以推进，不形成任何“目标安全”结论。

只有满足以下条件的 blocked 才可 `allow + degraded`：

- bounded retry 已耗尽；
- blocker 有稳定 reason、owner 和 exact affected inputs；
- outcome_unknown、scope/auth drift、evidence corruption 已被排除；
- residual risk 和下一步建议已持久化；
- blocked gap 会传入 Hypothesis Registry 和 Reporting。

## 5. Candidate 阶段：Hypothesis Analysis Team

### 5.1 产品语义

产品名称可继续显示 Candidate，但阶段职责改为：

> 读取前序全量 canonical facts 和 coverage gaps，形成可追溯、可演化的 canonical hypothesis authority ledger。

这里的“完整”只表示相对 exact frozen input denominator 的 input-complete，不表示系统已经穷举未知漏洞空间；proposed/contested/inconclusive hypothesis 是权威记录，不是已经证明的事实。

Candidate 阶段不是审批队列，不执行主动工具，不写 Finding，也不要求每个 hypothesis 已有 executor adapter。

### 5.2 公司隔离的团队拓扑

每个 organization 创建一个 `candidate_hypothesis_controller`：

- 唯一 canonical decision owner；
- 唯一 final submitter；
- 动态使用 2–8 个只读 analysis subagent 并发 lane，按 bounded micro-batch 跑完整个 worklist，不把 8 当总 work-item 上限；
- 小输入允许只启动一个 analyst；
- operation-level coordinator 只聚合状态，不读取或合并跨公司内容。

建议的分析能力角色是动态 capability，而不是固定工位：

- surface correlator：关联 TI/EAS/Enum/Vuln；
- application/trust analyst：读取 AU entity/workflow/ownership/trust boundary；
- auth/business-logic analyst：形成身份、角色、对象所有权、状态机假设；
- evidence/coverage critic：检查矛盾、遗漏、partial、sampled、template-only 与 blocked gap。

所有 child 都是 read-only；不能扫描、联网、修改 AU、写 canonical fact/evidence、创建 approval 或执行 action。

### 5.3 冻结输入快照

Controller 开始前，服务端生成 `candidate_analysis_snapshot.v1`：

- operation/org/wave/scope snapshot；
- predecessor final seals/handoffs；
- TargetIntel/EAS/Enumeration/Vuln canonical facts；
- typed observations 与 evidence membership；
- 完整 technique outcomes；
- current Application Model revision/items；
- blocked/partial/sampled/template-only/unchecked gaps；
- residual risks；
- 所有 source revision/hash；
- previous hypothesis generation seal/hash/current+terminal revision membership；
- previous state events/relations 与 open obligations；
- expected/unconsumed/consumed FactDelta exact ids 和 consumption watermark；
- campaign terminal/oracle/residual refs；
- enrichment/capability/policy/credential authority revisions；
- late/superseded result refs。

快照不能接受caller挑出的“好看receipt”。host从snapshot consumer spec和operation事实推导全部relevant stage/root exact set，在同一DB transaction与guard lifetime构造Plan A `CheckedToolTruthAuthorityBundle`；它保留fresh、expired、orphan、mixed-epoch和skew-exceeded全部成员供Candidate记录。只有semantic-consistent且temporally fresh的成员可进入authoritative fact projection；所有被排除成员必须以typed stale/invalid census、residual和revalidation obligation出现。Candidate Gate要求required roots全fresh，不能把“已从假设输入排除stale事实”误当快照完整。

每个target-state observation绑定host-frozen `EvidenceTemporalValidityPolicyV1`、target epoch、DB-clock observation window与`effective_valid_until`；阴性/反证TTL严格短于正向TTL。TTL过期保留as-of审计，但不能继续作为current Candidate、Verification或Reporting authority。same-semantic revalidation创建新receipt/lineage，绝不改旧时间或复活旧结论。

genesis snapshot 明确标记没有 previous generation；非 genesis snapshot 缺少上述任一 expected set 都不能开始分析。

大输入通过 snapshot-pinned 分页读取。page/key completion receipt 由服务端 query cursor 和 tool lifecycle 生成，不相信 child 自报；禁止 prompt 截断冒充全量读取。每个`(input, checklist member, chunk partition)`都有exact-one subreview disposition，host再seal跨chunk、跨input synthesis census；sample、context truncation、漏chunk或只做zero-proposal复核都只能产生degraded/residual，不能让Candidate Gate通过。proposal数量为零只是合法业务结果，不是唯一需要coverage review的情况。

Candidate可消费host-managed、签名且versioned的CVE/CPE/KEV/advisory/rule feed snapshot作为只读相关性输入；feed match只是一条signal/hypothesis source，不能成为proof。feed stale、产品版本未知、CPE映射不确定都必须形成residual；Agent实时浏览结果不能绕过frozen feed snapshot成为authority。

所有来自目标的 banner、页面、响应、AU 描述和其他字符串都必须作为带 provenance、类型、长度边界且 `instruction_authority=false` 的 untrusted data envelope 传给模型；它们不得拼入 system/tool instruction、改变角色权限、注入工具参数或覆盖 frozen contract。任何 analyst/controller 输出仍需经过 typed schema、引用完整性和 authority Gate，不能因目标内容中的指令性文本获得额外权限。

AU refs 与 evidence refs 分字段保存：AU 可以提高业务相关性、限定 subject 和影响，但不能冒充漏洞 proof。

### 5.4 两波分析

第一波是并行 proposal：

1. 服务端生成互斥 primary ownership work item，但相关 facts/evidence 可以被多个 shard 安全引用；额外生成 relationship/trust-boundary work item 和 deterministic cross-index，避免跨角色、对象、workflow 的猜想被拆散；
2. 每个 analyst 提交 `hypothesis_proposal.v1`；
3. proposal 包含结构化 claim、preconditions、impact、support refs、contradiction refs、AU refs、gap refs 和建议 readiness；
4. 服务端冻结 proposal census H1，模型不能静默删除或补写 census 外 proposal。

第二波是交叉复核：

1. `merge_conflict_critic` 按 deterministic conflict component 分片读取 H1，服务端最后做 exact-set reducer；不能要求单个上下文读取无限 proposal；
2. 只提交 `hypothesis_relation.v1` 和 proposed resolution；
3. 专门检查跨 shard duplicate、互斥前提、时序冲突、identity mismatch 和被忽略 gap；
4. Controller 分页逐 canonical cluster 作最终 merge/keep-separate/readiness decision。

### 5.5 Hypothesis canonical model

每个 canonical hypothesis 具有稳定 root id 和不可变 revision。状态分为三个正交轴：

| 轴 | 枚举 |
|---|---|
| `epistemic_state` | `proposed / supported / contested / verified / refuted / inconclusive / invalid` |
| `lifecycle_state` | `current / superseded / closed` |
| `planning_readiness` | `ready_for_strategy / needs_enrichment / deferred / out_of_scope / unsafe` |

`ready_for_strategy` 只要求 claim、subject、scope 和验证目标足够明确，不要求已有 adapter。adapter 是否存在由 Verification strategy/action compiler 决定。

capability 不是 hypothesis 的单一标量。`verification_capability_assessment.v1` 按以下复合键零到多保存：

```text
hypothesis_revision_id
verification_objective_id
capability_contract_id
policy_snapshot_id
```

每项 assessment 可为 `unassessed / available / adapter_missing / policy_denied / prerequisite_missing`。聚合视图只能导出 `has_runnable_strategy / planning_only / no_known_capability`，不能反向删除、refute 或拒绝落库 hypothesis。`authorization_required` 只属于具体 Prepared Action。

每个可进入Verification的revision还必须由host seal唯一`HypothesisVerificationPlanV1`。plan从结构化claim派生required claim-component exact set（clause、impact、trust/identity qualifier），绑定一个或多个objective/`VerificationContract`和ordered proof paths；每条path的component union必须exact覆盖该path声称证明的claim范围，缺objective只能收窄successor claim，不能留下“以后再看”却仍允许原claim终态。每条path先归约为`proved / falsified / unresolved`；outer truth固定为：任一proof path全部component proof即可verified；否则只有每条path都有valid且被plan标记为`path_falsifier`的refutation才refuted；其他组合才nonterminal。winning path以外或已falsified path内的未决objective仍进入coverage/residual/report limitation，但不能把存在/全称量词偷换成“所有objective都终态”的隐含AND。Plan C只用compound FK消费此plan，不能让Campaign或Agent临时重定义“做到什么算整条假设成立”。

合法状态组合必须由 Gate 验证：`verified/refuted/invalid` revision 必须 `lifecycle=closed`；`superseded` revision 不能 `ready_for_strategy`；derived child 是新的 current revision，不继承 parent 的 verified/readiness；contested hypothesis 可以 current，但必须有已处置 contradiction 和 owner/residual。

canonical revision 至少保存：

- structured claim/predicate；
- subject、target、role、object、workflow、trust boundary；
- 零到多 taxonomy/technique tags；tag 不参与 hypothesis identity，也不强制绑定单一 adapter；
- supporting observations/evidence；
- conflicting facts/evidence；
- assumptions 与 missing facts；
- AU context provenance；
- verification objectives 与 stopping criteria；
- priority、risk/impact；
- parent/child/duplicate/supersession lineage。

#### 5.5.1 Semantic identity 与去重

每个 hypothesis revision 都保存 `hypothesis_semantic_key.v1`。key 只由以下 canonical 字段组成：

- organization；
- target/subject 的 at-time identity；
- predicate schema/version 和 normalized arguments；
- trust/auth boundary；
- claim polarity。

prose、confidence、priority、technique/tag、supporting evidence 和提议者都不进入 semantic key。exact-key proposal 由服务端自动聚成同一 canonical cluster；“语义相似但 key 不同”的 proposal 只能由 Controller 通过 typed merge decision 合并，且 Gate 必须重新验证 organization、target/subject、trust/auth boundary 和 claim polarity，不能靠 embedding 距离自动越界合并。

root identity 固定为：

- 初始 root：在 operation/organization namespace 下，对 semantic-key hash 生成确定性 UUIDv5；
- `support / contradict / refine / terminalize`：保留 root，创建不可变 successor revision；`refine` 可产生新的 revision semantic key；
- `split`：每个 child 使用 parent root + child semantic key 的确定性新 root，并保留 split lineage；parent latest revision 变为 superseded且退出 successor generation membership；
- `merge`：使用排序后的 parent root ids + successor semantic key 生成确定性新 root，所有 parent latest revisions 变为 superseded且退出 successor generation membership；
- `derive`：使用 source root/revision + derivation rule + child semantic key 生成确定性新 root。

同一 root 同时最多一个 current revision；semantic key 相同不允许生成第二个平行 current root。历史 revision 的 key 永不回写。

服务端 reducer 在任何 insert 前按固定顺序消歧：

1. 由 canonical fields 重算 semantic key/hash，不接受模型提供的 hash；
2. 若 operation+organization 下已有同 key 的 current revision，proposal 自动 attach 到该 root/revision，`split/derive` 只增加 typed incoming lineage，不能创建第二个 root；`merge` 必须把该 current root 纳入 parent set 后重新决策；
3. 若同 key 只存在于历史 revision，且其 root 没有 current，material support/contradiction 通过 deterministic `reopened` state event 在同 root 创建 successor；无 material relation 则 `no_semantic_change`；
4. 若历史 key 所属 root 已有另一个 key 的 current revision，不能走 initial-root 公式或静默回退旧 claim，必须由 Controller 提交 explicit `split / derive / merge / refine` transition；operator 公式决定 successor root；
5. operator 计算出的 root id 若已存在且 canonical identity ingredients 完全相同，按 idempotent replay 复用；ingredients 不同则 fail closed 为 `ROOT_ID_COLLISION`。

revision ordinal 必须是 predecessor ordinal + 1，且 predecessor、state event、semantic key 和 lineage 在同一原子 reducer write 中绑定。任何 provider 完成顺序都不能改变上述选择。

### 5.6 输入 disposition

每个 frozen input item 分成控制面闭合和语义关系两部分。

`input_processing_disposition` 必须恰有一个：

- `analyzed`；
- `informational`；
- `duplicate_input`；
- `not_security_relevant`；
- `gap`；
- `blocked`。

`input_hypothesis_relation[]` 可以为零到多：

- `creates_hypothesis`；
- `supports_existing`；
- `contradicts_existing`；
- `qualifies_existing`；

每条 relation 必须引用 exact hypothesis/revision、fact/evidence/gap 和 relation hash。同一 observation 因而可以创建 H1、支持 H2 并限定 H3，而控制面仍能 exact-one 收口。

没有 hypothesis 并不等于没有 row。NoCandidate 只保留为旧执行投影的兼容 disposition，不能继续作为 canonical analysis truth。

### 5.7 Candidate Analysis Gate

Gate 必须确定性验证：

- snapshot current、scope/org/target/revision/handoff 对齐；
- 每个 shard 完整读取且 child terminal；
- proposal census exact-set 完整；
- 每个 proposal 恰属于一个 canonical cluster；
- 跨 org/target/auth boundary/evidence epoch 不得 merge；
- 服务端重算每个 revision semantic key/hash，并验证 root UUID 公式、predecessor exact、revision ordinal 连续；
- 每个 root 最多一个 current revision，每个 operation+organization+current semantic key 最多一个 root；历史 key revival/collision 必须命中固定 reducer route；
- 每个 accepted evolution proposal 恰好映射到 state event + revision/root/lineage mutation；rejected proposal 不得产生 canonical mutation；
- generation transition 对每个 previous member exact-one 映射 unchanged/terminal/successor；split/merge superseded parent 不进入 successor membership，全部 outgoing edges 必须落到新 membership 且无 orphan/环；
- 每条 contradiction 被 resolve、qualify、refute 或显式 blocked；
- absence/gap 不得作为反证；
- AU refs 不得进入 proof evidence；
- 每个 input item 恰有一个 processing disposition，且零到多 relation exact-set/引用完整；
- 每个 current hypothesis 恰有一个 readiness；
- `snapshot.expected_delta_ids == analysis_result.consumed_delta_ids`，其中 analysis result 必须恰为 generation seal 或 fixed-point receipt；每个 expected FactDelta 恰有一个 consumption status，seal 时 `unconsumed_delta_ids = ∅`；
- `quarantined_invalid_authority` 必须有 authority-corruption obligation；未裁决时 Gate HOLD，`discard_as_untrusted` 时必须绑定 invalidation/correction lineage、deferred residual 和 degraded coverage；
- `snapshot.expected_evolution_proposal_ids == accepted_evolution_proposal_ids ∪ rejected_evolution_proposal_ids`，且 accepted/rejected 不相交；accepted/rejected 只能描述 evolution proposal，不能描述 FactDelta；
- 每个 expected application-fact refinement request 恰有一个 `accepted / rejected / deferred_with_residual` disposition；accepted 必须在同一 terminal receipt 中绑定 exactly-one sealed Application Model revision id/hash、source hash 和 current-CAS adoption，rejected/deferred 必须绑定继续使用的旧 revision与 residual；未处置或绑定不完整的 request 留在 open-obligation hash 并阻止 seal；
- capability assessment 不得作为 hypothesis 存在与否的 Gate；
- child proposal 不能直接成为 canonical hypothesis record；
- final submitter 必须是 exact Controller。

## 6. Hypothesis Registry 与现有 Candidate 的边界

`Hypothesis Registry` 是 canonical hypothesis authority ledger；Tool Truth/evidence 才是事实 authority。产品中的 Candidate stage 输出 Registry generation seal 或绑定现有 generation 的 fixed-point receipt。

迁移期保留现有 `attack_candidate_work_items`、`attack_candidates`、approval 和 Attempt：

- 新 hypothesis revision 可带 nullable link 到旧 work item/Candidate；
- 已有支持 adapter 的 hypothesis 可以投影为 `ExecutionCandidateProjection`；
- 没有 adapter 的 hypothesis 仍进入 Verification strategy planning，并可能终止为 `capability_unavailable` residual；
- 旧 operation 继续按旧 Candidate/NoCandidate/Attempt replay；
- 新 contract operation 只以 hypothesis revision + Prepared Action hash + authorization receipt 作为执行 authority；旧 Candidate/approval/Attempt 是 non-authoritative compatibility projection；
- compatibility projection 失败只能形成 projection residual 或 HOLD 旧执行 consumer，不能回滚/删除合法 Registry generation；
- 旧 executor 必须按 frozen contract version fail closed，不能旁路 Prepared Action/JIT；
- 完成 cutover 后，Verification Campaign 以 `hypothesis_revision_id` 为根 authority，旧 Candidate id 仅作兼容引用。

revision 切换规则：

- H(g+1) seal 后，尚未 begin 的旧 revision Prepared Action/Candidate projection 全部失效；
- 已经 durable begin 的旧 revision Action 可以按冻结 authority 完成并保留历史结果；
- 旧 Action 的结果只能经 consolidation 影响新的 Registry，不能直接改写 H(g+1)；
- response-loss replay 只返回旧 Action 的相同 receipt/result，不获得新 revision authority。

旧数据只读兼容规则（不是 Registry 回填）：

| 旧数据 | Legacy read-model / report authority |
|---|---|
| Candidate | `LegacyHypothesisView`/audit observation；readiness 缺项显示 `legacy_unavailable`，不创建 canonical revision |
| verified Attempt | typed `grandfathered_legacy_security_verdict=verified` + 既有 Finding lineage；不创建新 Registry terminal |
| refuted Attempt | typed `grandfathered_legacy_security_verdict=refuted` + exact legacy refutation lineage；不创建新 Registry terminal |
| blocked Attempt | legacy inconclusive/deferred + blocker/residual |
| NoCandidate `typed_observation_required` | legacy needs_enrichment，不能投影为 refuted |
| unsupported adapter | legacy adapter_missing + residual |

legacy adapter只能读取冻结旧operation并生成显式标记的只读UI/report projection；不得向`attack_hypothesis_revisions`、verification plan、Campaign、objective outcome或revision adjudication表写入任何“回填”行。新链的verified/refuted只能由Plan B sealed plan和Plan C revision-level adjudication产生；若希望基于旧证据进入新链，必须显式fork/adopt成新的非终态 hypothesis，重新经过Tool Truth freshness、Candidate plan与Verification，而不是把历史Attempt升级成新authority。

## 7. Verification：真正的多 Agent Campaign

### 7.1 Campaign identity

每个 `hypothesis_revision + verification_objective + verification_contract` 同时最多有一个 active Verification Campaign；新的campaign version原子supersede旧version。Campaign只负责一个B-owned plan objective及其claim-component subset，不能代表整条revision。Campaign冻结：

- hypothesis revision 和 predicate；
- exact subject/target/scope/org；
- supporting/conflicting evidence snapshot；
- risk/credential/side-effect policy；
- request/runtime/cost/action budgets；
- allowed capability families；
- campaign version 和 stopping criteria。

同一revision可以同时或分批拥有多个objective-local Campaign。单Campaign无论结果多强，都只能关闭自己的objective outcome、coverage和FactDelta；它不能创建revision Finding/refutation、不能把Hypothesis写成verified/refuted，也不能让Reporting生成security verdict。revision终态只能由消费Plan B sealed proof paths和latest objective outcome exact set的host aggregate adjudicator产生。

predicate、subject、target identity 或 trust-boundary 语义变化必须先走外层 evolution，生成 H(g+1) 的新 hypothesis revision，再创建 Campaign。campaign version 只能在同一 predicate 下调整 strategy ordering、budget 或 policy snapshot，且重新冻结 authority；scope 扩大必须取得新的 scope authority。Refiner 只能 refine strategy，不能在 Campaign 内偷换 hypothesis claim。

admission、action begin、action closeout和revision aggregate adjudication都必须在Plan A server-derived multi-root checked-bundle callback内执行；只有所有required roots semantic-consistent、same-epoch/max-skew合法且TTL fresh时才能private-convert为`AllFreshToolTruthAuthorityBundle<'guard>`。每个compound持久化bundle seal、root/member census、semantic/freshness/temporal hashes、target epochs、observation window和effective validity；caller不能传裸reconciliation state、root list、旧seal或自报fresh。expired只创建revalidation/HOLD，orphan/tamper进入quarantine，二者不可混成一个“stale”布尔值。

### 7.2 团队角色

| 类型 | 角色 | 权限 |
|---|---|---|
| Owner | Verification Lead | 唯一 strategy decision owner、唯一 final submitter；无 raw attack tool |
| Planning | Generator/Strategist | 并行提出 2–5 个可证伪策略；只读 |
| Specialist | Web/API/Auth/Business Logic/Injection/Service specialist | 按 hypothesis 动态 consult；只读 |
| Research | Researcher/PoC Designer | 只读知识检索和本地 sandbox 设计；无目标网络 authority |
| Review | Evidence Analyst | 解释 typed oracle，不可覆盖 oracle |
| Review | Independent Critic/Adjudicator | 检查误判、control、coverage 和 refutation 范围 |
| Refinement | Refiner | 每轮后增删、重排或收窄剩余策略 |
| Recovery | Adviser/Reflector | 只由确定性 stall/no-progress 条件触发 |

consult 可以有限并行。真实Action除每Campaign单lane外，还必须由server从Prepared Action派生canonical conflict-key exact set；每个`organization + target/rate bucket | credential/session | mutable resource | control fixture`都有独立CAS head和fencing token，按`key_kind + key_hash`全量排序原子获取。不能把整组只hash成一个domain：`{A,B}`与`{B,C}`必须因共享B冲突；unknown outcome/cleanup未完成把全部key转recovery hold，不能等lease超时自动复用。

race/TOCTOU/double-spend等目标使用一个`ConcurrentActionGroupV1` Prepared Action，而不是把两次普通Action时间戳拼成证据。group一次冻结2..N个subrequest、credential/session、barrier/start window、union conflict keys、aggregate budget和per-subaction oracle role；JIT一次展示并授权完整group，host barrier有限并发执行，每member exact-one subexecution receipt，deterministic concurrency oracle消费完整census。无instrumented adapter时必须`adapter_missing + residual + coverage_sufficiency=not_assessed`，不能退化为串行“验证”。

consult census 在 provider 调用前持久化，状态闭集为 `queued / running / completed / failed / timed_out / cancelled`。specialist 只接收安全 typed projection；目标网页/响应内容是 untrusted data，不能进入 system instruction 或改变角色权限。

### 7.3 Durable typed artifacts

每个 round 无条件持久化 exact-one `campaign_round_input.v1`、consult census、`strategy_decision.v1`、`strategy_obligation_manifest.v1` 和 round disposition。其余 artifact 按发生条件保存，不能为了满足 Gate 伪造空记录：

- strategy proposal/critique、evidence assessment 和 plan refinement 为零到多；
- Action Compiler 成功时才有 `prepared_action.v1`，`no_action_compilable` 直接形成 typed round disposition/residual；
- 进入 authorization 才有 `authorization_decision.v1`；
- durable begin 后才有 `action_execution_receipt.v1`；
- landed/reconciled execution 才有 `action_oracle_assessment.v1`；
- oracle membership 或 terminal/no-action intent 进入objective contract evaluation时才有`campaign_adjudication.v1`；零Action adjudication必须绑定typed no-action reason，不能伪造oracle；只有objective-local terminal outcome才有`campaign_terminal_decision.v1`和`hypothesis_objective_outcome_receipt.v1`；
- `hypothesis_evolution_proposal.v1` 为可选模型建议，不是每轮义务，也不能代替 canonical FactDelta。

自然语言可以作为解释字段，但不能作为 Gate、Finding 或授权 authority。

每个 verification objective、required control 和 strategy proposal 都进入 immutable obligation manifest，并获得 `accepted / rejected / deferred / superseded / blocked` disposition。Refiner 只能追加 typed plan delta，不能覆盖旧 plan 或静默删除未执行策略。

### 7.4 单轮算法

```text
1. DB 冻结 campaign_round_input
2. DB 持久化 consult census、Lead lease/fence 和 round budget
3. Lead 让 1–3 个 specialist 并行提出策略/批评
4. Lead 对 obligation/proposal 作 typed disposition并选择一个 falsifiable strategy
5. Action Compiler 编译 immutable Prepared Action revision；也可确定性返回 no_action_compilable 并跳到 adjudication/refinement
6. 仅 compiled 分支用短事务持久化 pending authorization 和 deterministic redacted display projection
7. transaction 外等待 policy/human decision；不持有数据库事务或执行 lease；denied/expired 跳到 adjudication/refinement
8. 仅 authorized 分支在 begin transaction 以 CAS 校验 receipt/expiry/manifest drift，获取 conflict-domain lease、预留 budget并写 durable begin
9. 仅 durable-begin 分支在 transaction 外执行 typed adapter，并先 durable stage raw witness
10. closeout transaction 以 CAS 落 observation/fact/evidence/actual counters/finalization receipt
11. 仅 landed/reconciled execution 运行 deterministic action oracle 计算 exact observation
12. campaign adjudicator按一个objective的verification contract聚合一个或多个action oracle，形成claim-component/objective outcome
13. Evidence Analyst 与 Critic 解释 scope 和局限
14. Refiner 追加 plan delta，Lead 继续下一 round、提交 evolution proposal 或 terminal intent
```

Lead 不能自己填写 target、credential secret、canonical args、FactDelta 事实或 oracle verdict。canonical FactDelta 的 observation/fact/evidence 由服务端依据 execution receipt + oracle 生成；Lead 只能提交有引用的 hypothesis evolution proposal。authorization denied、capability gap 或 no action compilable 都是合法 round outcome，不强迫生成 Action。

### 7.5 Nuclei 的正确定位

Nuclei 只在以下条件均满足时可被选为 typed adapter：

- exact template id；
- template content digest 和本地 path witness；
- exact target/origin；
- recipe/executor/oracle version；
- 明确 prerequisites/control policy；
- 可证明的 total request upper bound；
- scope、risk 和 authorization 允许。

Nuclei no-match 的默认语义是 `scanner_no_match / inconclusive`。只有 exact recipe 声明 deterministic negative oracle，且 prerequisites、control 和 completeness 均满足时，才能 refute exact template condition；不得外推为整个漏洞 family 安全。

### 7.6 Adapter 能力矩阵

公开 capability 必须来自可验证矩阵：

```text
verification objective / contract
  × observation contract
  × action compiler
  × executor adapter
  × oracle schema
  × authorization tier
  × recovery policy
```

任一列缺失时显示 `adapter_missing` 或 `contract_pending`，不能只因 technique registry 中存在名称就向团队宣称可执行。

raw shell、通用扫描入口和任意 browser 不能作为 adapter 缺失时的 fallback。每个触达目标的动作都必须拥有 input contract、Action Compiler、bounded execution manifest、typed oracle、budget accounting 和 recovery policy。

建议按以下顺序补充 adapter：

1. Nuclei exact replay 修正；
2. HTTP baseline/differential；
3. anonymous/authenticated pair；
4. role × owner/non-owner authorization matrix；
5. directory soft-404/content fingerprint；
6. same-origin browser workflow；
7. parameter-level SQLi/XSS/CMDi bounded confirmation；
8. TLS/service exact checks；
9. OAST tokenized callbacks。

这份设计先解决“系统不能把未测/弱证据说成已验证”的authority soundness，不等于首批实现立刻拥有完整红队能力面。Plan C首批authoritative adapter只承诺anonymous/auth differential与directory soft-404；Nuclei negative、browser/OAST/WebSocket、injection、multi-role矩阵和race在各自typed compiler/executor/oracle完成前必须明确显示`unassessed|adapter_missing + residual`。因此A–D全部落地后仍可能“诚实地覆盖很窄”，不能把架构门禁完成解释成漏测问题已全部解决；扩大能力面需要后续按ThreatCoverageProfile优先级逐类交付。

## 8. Prepared Action 与风险分层 JIT 授权

### 8.1 先编译，后审批

授权顺序固定为：

```text
strategy
  -> Action Compiler
  -> Prepared Action / Execution Manifest
  -> policy or human decision
  -> authorization receipt
  -> begin action
```

审批不能再发生在 canonical request 物化之前。

Prepared Action revision 使用不可变状态机：

```text
drafted
  -> compiled | compile_rejected
compiled
  -> pending_authorization
pending_authorization
  -> authorized | denied | expired
drafted | compiled | pending_authorization | authorized-not-started
  -> superseded
authorized
  -> started
  -> succeeded | failed | outcome_unknown
succeeded | failed
  -> landed
  -> reconciled
outcome_unknown
  -> recovery_pending
  -> recovered_succeeded | recovered_failed | manually_blocked
recovered_succeeded | recovered_failed
  -> landed
  -> reconciled
```

manifest 任一字段漂移都创建新 Prepared Action revision，并可 supersede 任意尚未 started 的旧 revision，不原地修改。outcome_unknown 保留原 unknown witness，只能通过 recovery/人工裁决收口，不能自动重放。每个 Prepared Action 恰有一个 terminal disposition；只有 `authorized + started` 才要求 execution receipt，只有 landed execution 才运行 action oracle。denied/expired/compile_rejected/manually_blocked 必须有 reason/residual，不能伪造 execution 或 oracle。

### 8.2 Prepared Action 内容

HTTP 类动作至少显示并绑定：

- method、scheme、origin、path、query；
- DNS/connection policy、SNI/TLS、proxy、cookie jar、redirect origin 和 cross-origin policy；
- header/query/body/URL 中的 secret value 都用 opaque credential handle、credential version、scope 和 injection contract 代替；不保存明文或明文 hash；
- body schema、body hash 和 redacted payload diff；
- redirect、timeout、retry、rate、最大请求数；
- expected control、oracle version；
- 副作用、cleanup obligation 和 data handling policy。

模板/工具驱动动作至少显示并绑定：

- tool binary/version/digest；
- template/script/config content digest；
- exact target/parameter；
- path/method/payload family 与 bounded examples；
- 确定性最大请求数；
- 实际请求计数方式；
- adapter/executor/oracle/recovery version。

无法给出有限 request upper bound、目标范围或副作用边界的动作不得自动批准。Nuclei/browser/OAST 等动态能力可以授权有限请求族与强制 ceiling，不要求预览每个运行时字节；network governor 必须在发送第 `N+1` 个请求前拒绝并记录审计，而不是超限后才发现。

request count 只能由 host-owned trusted egress/transport governor 观察和阻断，覆盖 redirect、DNS/connection retry、browser subrequest 和 OAST callback registration；或由 adapter contract 静态证明上界。不能经过可信 governor、又无法静态证明上界的外部 adapter 只能进入 T2/T3，且不得宣称 actual-count complete 或获得自动批准。CLI 自报的 `-rl/-c`、stdout 计数和模型估算都不是 budget authority。

cleanup obligation未完成时，Campaign不能形成objective proof/refutation，只能进入blocker/inconclusive + residual。credential rotation、template digest、renderer、oracle或policy变化都会使未开始授权失效。

### 8.3 默认风险 tiers

| Tier | 典型动作 | 默认决定 |
|---|---|---|
| T0 | 本地分析、知识查询、无目标网络 | policy auto |
| T1 | engagement policy 已明确开启主动只读验证时，exact scope 内单次确定性 GET/HEAD/handshake | policy auto；UI 可见 |
| T2 | 多请求、凭证、注入 payload、browser workflow、OAST | 具体请求/sequence 生成后 human JIT |
| T3 | 写状态、exploit、破坏性、持久化、第三方影响 | engagement policy 显式允许 + 每动作人工确认；默认禁用 |

authorization receipt 必须绑定 manifest hash、policy version、adapter/template/oracle version、budget 和 secret refs。任一字段漂移都要求新审批。

### 8.4 Workspace 中的人工 JIT

- T0/T1 显示 policy auto decision 和安全审计投影，不显示人工批准按钮；
- T2/T3 显示 pending exact packet/request-family/sequence；
- UI mutation 只能提交 `decision(approve|reject) + prepared_action_id + private_manifest_hash + display_projection_hash + renderer_version + expected_row_version + requested_expiry? + stable_request_id`；
- UI 不回传 target、args、credential、payload 或 risk tier；
- backend 从 local principal 和 current engagement/policy snapshot 派生 actor authority，验证 pending state，并 clamp/派生 actual expiry；客户端 expiry 不是 authority；
- authorization receipt 同时绑定 private canonical manifest hash 和 deterministic redacted display projection hash，证明“用户看到的包”对应“实际执行的包”；
- packet refresh、credential version、digest、budget 或 projection `change_seq` 漂移立即禁用按钮并要求重审；
- response-loss 使用同一个 request id exact replay；
- reject/expiry 写 durable disposition/residual，不把 hypothesis 解释为 refuted。

## 9. Typed Oracle

Oracle与终态分为三层。`action_oracle_assessment.v1`只解释一个Action的exact observation；`campaign_adjudication.v1`按一个objective的冻结verification contract聚合多个Action oracle，只能确定objective-local proof/refutation/inconclusive；`hypothesis_revision_adjudication.v1`最后按Plan B sealed proof paths和latest objective outcome exact set，才能确定整条revision的verified/refuted/nonterminal。

verification contract必须声明本objective的predicate components、required controls和组合逻辑，例如`all_of / any_of / paired_differential / ordered_sequence`。Lead只能提交objective terminal intent，deterministic campaign terminalizer执行局部组合规则；它无权决定Plan B outer proof path。

每个 action recipe 必须声明机器可判定的 oracle：

```text
oracle_rule_id/version/digest: ...
prepared_action_id: ...
execution_receipt_hash: ...
input_evidence_snapshot_hash: ...
control_action_ids: [...]
predicate_component_ids: [...]
preconditions: satisfied | failed | unknown
control: valid | invalid | not_required
observation: proof | refutation | inconclusive | blocker
completeness: complete | partial
evidence_ids: [...]
reason_code: ...
limitation_codes: [...]
```

Evidence Analyst 只能解释这个结果，不能把 2xx、tool exit 0 或 prose 改写为 proof/refutation。

合法组合固定为：

- required precondition 未满足或 unknown 时固定为 `inconclusive/PRECONDITION_UNSATISFIED`；只有该 precondition 本身是冻结 predicate component，且 oracle rule digest 明确声明求值规则时例外；
- `control = invalid` 固定为 `inconclusive/CONTROL_INVALID`，不能 proof/refutation；
- `completeness = partial` 固定为 `inconclusive/COVERAGE_PARTIAL`；
- `attempt_state = outcome_unknown` 不运行 oracle；
- cleanup 未完成固定为 `blocker/CLEANUP_INCOMPLETE`；
- action proof 只覆盖其 predicate component，不自动 verified 整个 Campaign。

`campaign_adjudication.v1` 至少绑定：

- verification contract/hash；
- exact action-oracle membership/census hash；
- strategy/control obligation disposition hash；
- decisive predicate component ids；
- `continue / objective_proof / objective_refutation / inconclusive / blocked`；
- unsatisfied controls、evidence membership、limitation codes；
- evaluator version/digest。

最低 oracle 要求：

- authz/IDOR：身份、角色、owner/non-owner、对象和响应语义差分；每个角色session必须有current session-valid control，exact同一resource，cache/redirect/content normalization与max-skew固定；
- anonymous access：anonymous/authenticated或public baseline对照，cookie/credential store物理隔离，authenticated control有效且exact同一resource；动态baseline只能inconclusive，不能把任意2xx当proof；
- directory：使用多个deterministically derived nonexistent path建立soft-404 baseline，并同时判断variance、content type、redirect chain、WAF/challenge和动态模板；keyword命中或单个404/200不能单独proof/refute；
- injection：paired controls、重复性和误差阈值；
- browser/business logic：before/after state、业务 invariant、cleanup receipt；
- Nuclei：exact template condition，不外推漏洞 family；
- OAST：campaign/action-bound callback token receipt，另有 `waiting_for_callback`、deadline 和一次性 token consumption；callback grace 未结束不能按超时形成阴性。

## 10. 双循环的状态演化

### 10.1 不可变 generation

Verification 不得原地修改已 sealed 的 H(g) 或当前 Candidate。结果形成 `verification_fact_delta_bundle.v1`；每个 consolidation batch 都写 immutable receipt，但只有 semantic Registry、open-obligation 或 relevant authority snapshot 发生 material change 时，Candidate reducer 才创建 H(g+1)。若所有输入均为 `no_semantic_change`，则在 H(g) 上写 fixed-point receipt，不制造空 generation。迟到结果只能进入后续 consolidation batch，不能回写已 sealed generation。

### 10.2 合法 evolution operators

| Operator | 规则 |
|---|---|
| `support` | predicate 不变，追加支持事实，产生新 revision |
| `contradict` | 保留原支持证据并追加反证；不能删除历史 |
| `refine` | 明确收窄/限定 predicate、subject 或 impact；记录 retained/dropped semantics |
| `split` | parent superseded，生成 2+ children；非穷尽 partition 必须保留 residual branch |
| `merge` | 仅语义兼容且同 authority boundary；继承所有 support/contradiction/gaps |
| `derive` | 由新事实产生 child；必须有 derivation rule 和 source refs；不能继承 parent verified/readiness |
| `terminalize` | 仅由host revision-level aggregate adjudication的verified/refuted结果，或Registry判定hypothesis revision自身contract非法的invalid规则触发；关闭current revision。单Campaign、单objective或单Action不能触发。它不等于source FactDelta的authority quarantine |

每个 previous current revision 必须恰好映射到：

- `unchanged`；
- `terminal`；
- 一个或多个 `successor`。

lineage 必须无环、无 orphan、无双 current revision。

同一 generation 采用 set-based consolidation：服务端先冻结 `expected_delta_ids` 和 `expected_evolution_proposal_ids`，再 union 所有 support/contradiction/gap。每个 parent 在一代中最多接受一个 structural operator（`refine / split / merge / terminalize`）；`terminalize` 与另外三个 structural operator 互斥，`derive` 只可作为有独立 identity 的 child edge 共存。冲突 structural proposals 必须由 Controller 显式 accept/reject；同批 support + contradiction 默认得到 contested revision，不能由完成顺序决定结果。

FactDelta 不参与 accept/reject。每个 expected FactDelta 必须恰好一次写入 `applied / no_semantic_change / quarantined_invalid_authority` consumption；即使 authority 无效，原 bundle 仍不可变保留。accepted/rejected exact sets 只属于 `hypothesis_evolution_proposal.v1`。consolidation receipt 及其唯一 result（generation seal 或 fixed-point receipt）分别保存 consumed FactDelta exact set、每项 consumption status、accepted/rejected evolution decision exact sets、application-fact refinement disposition set 和 semantic hash。

### 10.3 FactDelta 路由

canonical `verification_fact_delta_bundle.v1` 先表达 server-derived observation/fact/evidence、exact claim relation 和 source Campaign/Action/oracle authority，并无条件持久化。它是不可变输入，不能被 Controller “拒绝”或删除；authority 不合法时也只能 `quarantined_invalid_authority` 并保留审计事实。模型只能提出可接受或拒绝的 `hypothesis_evolution_proposal.v1`，不能铸造或改写 FactDelta 事实。

consolidator 对每个 expected FactDelta 恰好执行一次：

- `applied`：其 canonical relation/observation 改变 Registry 或产生受约束 follow-on；
- `no_semantic_change`：输入有效但不改变当前语义，仍记录消费和原因；
- `quarantined_invalid_authority`：source binding、scope、org、oracle 或 evidence membership 无效；不影响 Registry 结论，但 bundle 保留，并创建 typed `authority_corruption_obligation` + affected campaign/hypothesis residual。未裁决时 `control=hold` 且不能 final seal；只有 corrected replacement bundle，或显式 `discard_as_untrusted + deferred_with_residual` 才能收口，后者必须降 coverage 并进入报告。

objective proof/refutation Campaign terminalizer已验证并冻结source authority；它产生的bundle若随后semantic orphan/tamper，属于invariant violation，不是普通业务分支。quarantine closeout必须在同一事务追加leaf outcome/terminal/FactDelta及所有依赖revision adjudication/Finding/refutation/report source的invalidation、correction和Gate HOLD；active Reporting立即排除旧aggregate verdict。旧revision terminal immutable，修复只能经H(g+1) successor、B-owned新plan和新的revision aggregate adjudication产生新current verdict，不能原地改回有效或改走另一旧proof path。

纯TTL到期不是quarantine。每个objective outcome、revision adjudication、Finding/refutation和report source强一致派生`authoritative | temporally_stale | semantically_invalid`：到`effective_valid_until`后保留`observed_as_of`历史，但从current selector排除并创建revalidation + H(g+1) re-adjudication obligation。same-semantic replacement receipt也不能延长或复活旧terminal；只有新objective outcome exact set和新aggregate adjudication可成为current。

response-loss 或重跑只能返回同一 consumption receipt；不能对同一 delta 生成第二次 semantic application。

Registry consolidation 后再派生以下新路由：

- `registry_only`：只更新 support/contradict/qualify/refute/inconclusive，不产生执行投影；
- `execution_projection`：current revision 有合法 verification objective，可进入下一 Wave/Campaign planning；
- `enrichment_request`：生成 exact observation contract、owner、trigger 和 durable follow-on；
- `residual_only`：capability/policy/scope/budget 阻止继续，保留 limitation。

旧 `NoAttack / DirectWorkItem / PendingEnrichment` 只作为 legacy execution projection 映射，不参与 Registry acceptance Gate。unsupported legacy projection 不得回滚 canonical FactDelta 或 hypothesis revision。

若 delta 改变角色、workflow、ownership 或 trust boundary，它不能由 Verification 直接修改 current Application Model。它必须先进入受约束的 application-fact refinement 路由，生成 exact request、owner 和 `accepted / rejected / deferred_with_residual` disposition；accepted disposition、exactly-one sealed Application Model revision/authority、source hash、current-CAS adoption 和 terminal receipt 必须在同一事务写入。rejected/deferred 继续绑定旧 revision + residual。下一 Registry generation 只读取已接受且 receipt 完整的 current revision。未处置或半落地 refinement 是 open obligation，进入 `open_obligation_hash`、drain 和 Candidate/Verification Gate。技术事实和 hypothesis relation 不需要伪装成 Application Model 修改。

新版本按 hypothesis dependency component 设置 barrier。一个 branch 的 enrichment 不应永久 HOLD 其他无依赖 hypothesis；全局 seal 仍必须列出全部 pending branch。

late delta 继续绑定原 hypothesis revision并标记 `stale_relevant`，不得自动传播给 child/successor；它仍按 `applied / no_semantic_change / quarantined_invalid_authority` 恰好一次消费。只有由它派生的 evolution proposal 才能被 accept/reject；若应用到 current lineage，必须显式 qualify 或 derive。

### 10.4 幂等与防无限循环

四层 identity 分开定义：

```text
Campaign admission = hypothesis_revision_id + contract_hash
                   + relevant_evidence_snapshot_hash + campaign_version
Round = campaign_id + round_ordinal + round_input_hash
Prepared Action = round_id + strategy_decision_id + private_manifest_hash
Execution = prepared_action_id + authorization_receipt_id + execution_ordinal
```

response-loss exactly-once 使用 Execution key 和 exact target/credential handle/attempt epoch，返回同一 receipt/result。认知 no-progress 使用独立 `semantic_attempt_fingerprint`：exact predicate/objective、required controls、action/adapter/oracle versions 和相关 evidence membership；无关新 evidence 或 Lead 轻微改写 prose 不能绕过防重。

`execution_ordinal` 由服务端在首次 dispatch 前一次性冻结；response loss 必须复用原 ordinal。Action 已触达外部目标后，调用方不能递增 ordinal重放；再次执行必须创建新 Prepared Action、新授权，并满足明确 recovery policy。

new round、consolidation、new generation 和 new wave 使用四个不同条件：

- `new_round_admission`：存在未尝试且仍有效的 sealed strategy/control obligation，上一 round 已终态且其 Prepared Action/Action（如有）均已终态，没有 conflict-domain lane blocker，且 campaign budget 允许；
- `consolidation_batch_admission`：一个 Wave 的 Campaign census 已全 terminal，或出现 late delta、enrichment/Application Model disposition、相关 authority change；每个 admitted batch 都写 receipt；
- `new_generation_commit`：consolidation 后的 `semantic_registry_hash`、`open_obligation_hash` 或 relevant authority snapshot 相比 H(g) material change 时才创建并 seal H(g+1)。即使变化只 terminalize 最后一个 hypothesis也必须生成 H(g+1)；全为 `no_semantic_change` 时只在 H(g) 写 fixed-point receipt；
- `new_wave_admission`：只能针对 sealed H(k) 判断；至少一个 current revision 具有尚未尝试、可调度的 verification/strategy obligation，不存在 scope/policy/authority blocker，且 operation/wave budget 允许，才创建下一 Wave/Campaign census。

genesis frozen input 可以直接生成 H(0)；H(0) seal 后仍需满足 `new_wave_admission` 才能开启首个 Campaign，不要求 previous delta。没有 runnable obligation 的 material change 可以触发 generation，但绝不能为了“有新 generation”伪造空 Verification Wave；fixed-point receipt 本身也不能成为开 Wave 的理由。

停止与 seal 分为三个层次，不能互相等待形成循环。

**Campaign-local stop/drain**：`campaign_stop_scheduling` 可由预算、deadline、policy 或 no-progress 触发，表示不再发本 Campaign 的新 consult/action；随后只 drain/处置本 Campaign 的 running/queued consult/Action、pending approval、outcome_unknown/recovery、OAST callback、cleanup 和 raw landing/reconciliation。local drain 完成即可 terminalize Campaign，写 `exhausted_with_residuals`、FactDelta 和 residual；它不得等待 FactDelta consumption、evolution decision、Application Model refinement、Registry generation 或 fixed point。

**Wave/generation consolidation**：所有 Campaign terminal 后，barrier 再消费 FactDelta、决策 evolution proposal、处置 enrichment/Application Model refinement，并写 consolidation receipt；materiality reducer 决定 H(g+1) 或 H(g) fixed point。

**Stage final seal**：`generation_member_roots` 只包含该 generation 的 current root 或 closed-terminal root。因 split/merge 被 supersede 且无 current 的 parent root 不进入 successor generation 的 runnable denominator；generation transition seal 必须把每个 previous member exact-one 映射为 unchanged/terminal/successor，并保存全部 outgoing split/merge edges。Gate 递归验证 successor roots 都在新 membership 中、无 orphan/环/漏映射。

只有以下全局条件满足后才能 final seal：

- 无 running/queued Campaign、Action、approval、callback、cleanup 或 recovery；
- 无 unconsumed FactDelta；
- expected evolution proposals 已 exact-set accept/reject，且 accepted decision 已应用；
- expected application-fact refinement 已原子绑定 accepted revision，或 rejected/deferred_with_residual；
- authority-corruption/quarantine obligation 已 corrected，或 `discard_as_untrusted + deferred_with_residual` 收口；
- 所有 contradiction 已形成 contested/refuted/qualified 或带 owner 的 residual；
- 所有 required enrichment 已完成，或转成 `deferred_with_residual`；
- 每个 generation member root 的 head 满足且只满足一种：`唯一 closed terminal head + 无 current revision`，或`唯一 current revision + 所有 verification objective 均有 no-runnable/deferred/residual disposition`；
- 不存在 current runnable revision，也没有合法未尝试 strategy obligation；
- 排除 generation id、row id 和 timestamp 后的 `semantic_registry_hash + open_obligation_hash` 确定性稳定。

Campaign terminal只推进一个objective head，映射固定为：

| Campaign terminal | Objective outcome / Hypothesis evolution |
|---|---|
| `objective_proof` | 写该objective及claim-component proof；revision仍非终态，等待latest outcome exact set的aggregate adjudication |
| `objective_refutation` | 写该objective及claim-component refutation；只有Plan B将其标为某proof path的valid falsifier时才可能参与整条refuted |
| `inconclusive` | 写objective inconclusive + exact residual；epistemic默认保持原值，FactDelta可独立形成support/contradict关系 |
| `blocked` | 写objective blocker、capability/policy residual；不是阴性 |
| `exhausted_with_residuals` | 写objective deferred/exhausted residual；不是阴性 |

revision-level reducer随后按B-owned plan运行：任一path全部proof就在独立事务写`verified + Finding lineage + terminal decision + state event`；否则只有每条path都有valid designated falsifier才写`refuted + exact refutation lineage + terminal decision + state event`；否则只写nonterminal adjudication和unresolved exact set。全量latest objective outcome set仍必须封存，非决定性未决项仍进入coverage/residual/report limitation，但不否决已满足的outer truth。单Campaign、挑选旧objective head、漏component/impact qualifier或使用expired/quarantined authority全部fail closed。

Campaign 预算到限只能触发 `campaign_stop_scheduling`；local drain 后即可 terminalize 为 `exhausted_with_residuals`。Wave consolidation 和 Stage final seal随后独立进行。它不是“测试完成/没有漏洞”。

## 11. Objective terminal、Revision adjudication 与 Gate

Campaign terminal status只描述一个objective：

| 状态 | 要求 |
|---|---|
| `objective_proof` | 本objective全部required components有typed proof + exact evidence/control/coverage；不创建Finding |
| `objective_refutation` | 本objective的exact components有deterministic refutation，required controls/completeness全满足；不直接关闭revision |
| `inconclusive` | 已有观察但不足以支持或反驳；保留 open/deferred hypothesis |
| `blocked` | 外部/授权/凭证/基础设施 blocker + residual；不是阴性 |
| `exhausted_with_residuals` |预算/no-progress 到限且已 drain + residual；不是阴性 |

四个barrier严格分层，避免循环依赖：

1. **Campaign objective terminalizer**：验证本Campaign的obligations、Prepared Actions、execution receipts、action oracles和objective adjudication，在一个事务写terminal decision/receipt、claim-component outcome seal、objective outcome head、coverage、residual和恰好一个immutable FactDelta；不创建Finding、不写revision verified/refuted；
2. **Revision aggregate adjudicator**：按canonical order锁全部objective heads，自行seallatest eligible outcome exact set并重验Plan A all-fresh multi-root temporal bundle；只有outer proof-path规则verified/refuted时，才在独立原子事务创建Finding或refutation lineage、revision terminal decision与Plan B state event；
3. **Wave consolidation barrier**：等待本Wave所有Campaign terminal/unassigned objective收口，逐个恰好一次消费FactDelta，仅接受/拒绝evolution proposal，并写consolidation receipt；material change生成H(g+1)，否则绑定H(g) fixed-point receipt；
4. **Verification Stage Gate**：检查冻结denominator、revision adjudication/nonterminal residual、consolidation decision、open obligations和next-wave/final route。

Verification Stage Gate 只读取 DB truth：

- campaign denominator 和 round exact-set；
- consult/action/recovery/terminalization 是否全部收口；
- 每个 Prepared Action 恰有一个 terminal disposition；只有 authorized+started 要求 execution receipt，只有 landed/reconciled execution 要求 action oracle；
- denied/expired/compile_rejected/superseded/manually_blocked 有 reason/residual，且没有伪造 execution/oracle；`manually_blocked` 还必须保留原 outcome_unknown witness 和人工裁决 receipt；
- 每个objective的latest claim-component outcome、exact evidence/control/coverage和Campaign terminal compound完整；
- revision verified/refuted必须绑定Plan B plan、latest objective outcome exact set、host aggregate adjudication、all-fresh temporal bundle以及Finding/refutation lineage；
- blocked/exhausted_with_residuals/inconclusive 的 stable reason/residual；`capability_unavailable` 是 blocker reason code，不是游离的 Campaign status；
- FactDelta 是否全部拥有唯一 consumption receipt，evolution proposal 是否 exact-set 决策并应用；
- enrichment 和 application-fact refinement obligation 是否完成或带 residual 收口。
- quarantine/authority-corruption 是否已 correction 或显式 discard 收口；invalidated terminal/Finding 不得继续作为 active verdict/report source。

认知 Agent prose、模型 confidence、多数票或“我认为完成”都不能成为 Gate authority。

## 12. Investigation Workspace

### 12.1 入口

保留 roadmap 中 Candidate 和 Verification 两个节点，以表达不同 authority boundary；点击任一节点打开同一个 operation-level `Investigation Workspace`：

- Candidate 默认打开 Hypotheses；
- Verification 默认打开 Campaigns；
- 已完成会话可从 operation/stage history 直接打开；
- 不再依赖选中某个历史 Candidate tool call 或先收到某条 review hint。

### 12.2 信息架构

```text
Investigation Workspace
├── Hypotheses
│   ├── source/fact/gap tree
│   ├── hypothesis rail
│   ├── selected hypothesis detail
│   └── Candidate analysis team activity
├── Campaigns
│   ├── strategy/round timeline
│   ├── team topology
│   ├── authorization packet
│   ├── action + oracle timeline
│   └── evidence/Finding/FactDelta lineage
├── Waves
│   └── H(g) -> W(n) -> D(n) -> H(g+1) | fixed-point H(g)
└── Timeline
    └── proposal/support/contradict/refine/split/merge/derive/verdict
```

### 12.3 Hypotheses 视图

左侧：

- TI/EAS/Enum/Vuln/AU/gap/residual source tree；
- 每类输入的 analyzed/remaining/partial/blocked counts；
- 点击可定位到相关 hypothesis。

中间：

- 按 organization/target/workflow/trust boundary/family 分组；
- 显示 epistemic/readiness/capability 状态；
- 支持 proposed/supported/contested/ready/enrichment/adapter-missing/terminal 筛选；
- 不是 FIFO 编号。

右侧：

- structured claim；
- supporting/conflicting evidence；
- assumptions/missing facts；
- AU context；
- verification objectives；
- revision/parent/children/merge/split lineage；
- 当前 Agent decisions 和引用。

底部/可展开区域复用之前阶段的 Controller/SubAgent 视觉语言，展示真实 Worker、assignment、attempt、typed output 和 Gate；不虚构没有持久数据的角色。

### 12.4 Campaigns 视图

- Campaign header：scope、hypothesis、risk、planned/actual budget、terminal state；
- team topology：Lead、consults、Critic、Refiner、Reflector；
- round timeline：snapshot → strategy → critique → prepared action → authorization → adapter → oracle → refinement；
- Authorization Packet：redacted request/sequence、risk tier、policy/decision、digests、预算；
- Action/Oracle：expected control、observed result、verdict、limitations；
- Evidence lineage：Observation → Hypothesis → Action → Evidence → Finding/Refutation/Residual → FactDelta；
- Recovery 只在异常时显示；
- UUID/hash/row version 放入 Audit details，不作为主要阅读标题。

### 12.5 后台队列的定位

后台仍可按 organization/campaign 串行真实 Action，并公平调度多个 hypothesis。队列、lease、row version、receipt ids 只出现在 Debug/Audit drawer，不再作为 Candidate/Verification 的主产品信息架构。

### 12.6 UI truth 与状态管理

- operation-level DB read model 是唯一 truth；
- event 仅递增 refresh hint；
- store 只保存 operation id、selected hypothesis/campaign 和 refresh cursor；
- 不把 canonical registry/campaign state复制进 Zustand；
- loading/error/empty 对 Hypotheses、Campaigns、Waves 独立呈现；
- secret、raw credential、完整 exploit payload、lease/checkpoint 永不进入 UI DTO。

所有 operation/campaign/hypothesis read 和 mutation 必须在后端验证 local operator、project、scope snapshot 与 organization ownership；前端传入的 id 只能是选择器，不能成为 IDOR authority。

### 12.7 Read-model API contract

新 UI 使用六个 operation-scoped typed read：

- `investigation_get_summary(operation_id)`：all generation/wave summary、counts、control decision、coverage grade、open obligations；
- `investigation_list_hypotheses(operation_id, filters, cursor, expected_change_seq)`：Hypothesis rail 的稳定排序分页；
- `investigation_list_campaigns(operation_id, filters, cursor, expected_change_seq)`：Campaign rail 的稳定排序分页；
- `investigation_get_hypothesis(operation_id, hypothesis_revision_id)`：claim、relations、evidence/gaps、readiness、capability assessments、lineage；
- `investigation_get_campaign(operation_id, campaign_id)`：rounds、roles、strategy obligations、redacted Prepared Actions、oracles、objective-local outcome/adjudication audit、residual；它不返回或铸造 revision verdict；
- `investigation_list_timeline(operation_id, cursor)`：generation/wave/event 分页流。

opaque detail id 只能在同一 operation 内解析；project/scope/org authority 全由服务端推导。每个组合 read 在 `REPEATABLE READ READ ONLY` snapshot 内生成，并返回：

```text
projection_schema_version
change_seq
read_at
observed_as_of
effective_valid_until
authority_epoch_set_hash
source_time_status
tool_truth_contract
investigation_contract_version
investigation_rollout_mode
mode_policy
next_cursor
```

`projection_schema_version` 只表示 read-model schema，V1 中固定为 `1`，不能被当作数据快照序号；current 分页一律使用 cursor codec V2。cursor 必须绑定 `projection_schema_version + as_of_change_seq + as_of_temporal_cutoff + authority_epoch_set_hash + earliest_effective_valid_until + filter_digest + stable_sort_key`。真正的 stale 判定同时比较 monotonic change sequence、DB-clock temporal cutoff 和 authority epoch exact set；任一漂移都返回统一 stale/restart code，不得把两个 snapshot 拼接，也不得用浏览器本地时钟续命。cursor V1 只能读取冻结历史/legacy projection，不能继续 current 多页读取。summary/detail/list 的旧响应不能覆盖较新的 `(change_seq, authority_epoch_set_hash, observed_as_of)`。大集合必须 cursor pagination + detail lazy load；1k+ hypotheses/multi-wave 不得把完整 request/evidence/prose 一次塞入 DTO。

read model 是 authoritative UI projection，不是新的 Gate/write authority。event 只携 `operation_id + change_seq` refresh hint；mount、restore、missed event、乱序 event 都必须通过主动 DB bootstrap 恢复。

### 12.8 路由与组件边界

- operation-level `InvestigationWorkspace` 由 Pane/Engagement route 直接挂载；
- Candidate、Verification roadmap 节点以及 live/completed/restored history 都 deep-link 到该 route；
- `ToolCallDetailView` 只保留 “Open Investigation Workspace”，不再嵌主 read model；
- store 只保存 `{operationId, defaultTab, selectedHypothesisId?, selectedCampaignId?, refreshSeq}`；
- Hypotheses/Campaigns/Waves 共享同一个 operation selection truth；
- `legacy_only / shadow_registry / dual_read_compare` 都保留旧 authority 所需的 review/recovery/resume UI，并可显示 Shadow/Compare 徽标；不能因显示新只读投影而隐藏旧 mutation；
- 从 `registry_authoritative_legacy_projection` 起，旧 review mutation 隐藏，new contract 使用 Prepared Action JIT；policy-auto 动作只显示只读 `PolicyDecisionAudit`；
- 主 DOM 不出现 `Queue N` 或 FIFO position，同一 hypothesis 的多 Attempt 按 Campaign round/action 归档；
- scheduler order 不能被 UI 解释为 priority 或 coverage；
- team 只展示 durable typed assignment/status/artifact/public summary。hidden chain-of-thought、system prompt、raw specialist prose、credential 和 payload 不进入 DTO；无持久 artifact 时显示 `not recorded`。

## 13. Reporting

报告必须同时回答“发现了什么”和“没有验证完什么”。canonical report source 至少包含：

- Hypothesis root/revision/event/relations；
- Candidate analysis snapshot/input disposition；
- Plan B verification plan/proof paths/claim components；
- Campaign/round/strategy decision与objective outcome exact set；
- revision-level adjudication/terminal decision；
- Prepared Action/authorization/execution/oracle；
- Finding/lineage；
- Refutation contract；
- FactDelta/consolidation；
- enrichment/capability gap；
- coverage receipt；
- residual risk。

report source 分级：

- `security_verdict_authority`：Plan C deterministic revision-level adjudication + Plan B sealed verification plan/proof paths/claim components + latest eligible objective outcome exact set + Plan A all-fresh multi-root temporal bundle + revision terminal decision；verified还必须绑定Finding/lineage，refuted还必须绑定exact refutation lineage。单Campaign或缺任一项只能降为objective/method observation + limitation；
- `grandfathered_legacy_security_verdict`：只允许 frozen `legacy_only / shadow_registry / dual_read_compare` operation 使用；必须同时具备 terminal CandidateAttempt/Verification truth、action/evidence manifest、Finding lineage（verified）或 exact legacy refutation receipt（refuted）以及 immutable adapter hash。它是独立的只读历史 authority，不能伪造成 Campaign terminal receipt，也不能被新权威 operation 用来产生新结论；
- `coverage_authority`：frozen denominator、coverage receipt 和 exact gap/residual membership；它只证明测试范围与完整性，不能单独证明或反驳漏洞；
- `execution_observation_audit`：Prepared Action、execution receipt、raw witness ref、typed observation/evidence、单action oracle和objective-local Campaign outcome；它们可证明“做过/看见过什么”，但在revision aggregate adjudication采纳前不能单独产生hypothesis verified/refuted；
- `method_audit_only`：consult、strategy、critique、refinement，只能解释测试方法，不能支撑漏洞或安全结论；
- `authorization_audit`：Prepared Action 的 deterministic redacted projection 和 decision，不含 secret、完整 payload/response 或利用细节。

`raw witness`只作为canonical source的id/hash/provenance被报告引用。raw vault使用per-operation envelope encryption、opaque locator、独立viewer授权/访问审计和operation-frozen retention；secret/PII命中时raw隔离，普通分析只读typed/redacted derivative，到期crypto-erasure保留hash/provenance而销毁key material。report builder、export和UI DTO只能读取deterministic redacted projection；raw body、完整stdout/stderr、credential/token/cookie、PII、完整请求/响应和利用payload永不进入artifact或DOM，也不能借report path绕过viewer。

报告规则：

- scanner no-match 标为方法级 observation，不得写成漏洞 family 安全；
- blocked、partial、sampled、template-only、unsupported 和 inconclusive 必须进入 limitation；
- NoCandidate reason、pending enrichment 和 exhausted residual 不能从 canonical snapshot 消失；
- `PASS_WITH_GAPS` 必须列 exact affected targets/techniques/inputs；
- coverage denominator 必须给出 planned、tested-complete、tested-degraded、untested 和 blocked counts；
- 每个 gap 引用 residual id、exact affected input、reason、owner 和 next action；
- 只有 verified + typed proof + Finding lineage 可以生成漏洞 Finding；
- refuted 必须说明 exact predicate、control 和覆盖边界；
- 报告 findings=0 仍然必须运行并展示覆盖与残余风险；
- report revision 绑定 active generation seal、wave consolidation result（new generation 或 fixed point）和 report-input hash；late result 不能静默改变已发布报告；
- 仍有 next wave、pending consolidation、callback、cleanup 或 recovery 时只能生成 draft，不能 final publish。

`report_input_seal`是open header→ordered typed source members→host recompute count/hash→seal的immutable exact set；source member使用closed variant和operation/project/org/hash compound FK，不能用开放`kind + UUID`。它同时绑定Plan A server-derived relevant-root `AllFreshToolTruthAuthorityBundle`的bundle/root/member、semantic/freshness/temporal、target epoch、observation window与effective-validity hashes。finalize、current view/reuse/export必须用DB clock重新all-fresh；TTL到期后已发布artifact仍可按`observed_as_of`历史下载但状态为`temporally_stale`，不再是current或新报告source。semantic orphan/quarantine是`revoked`并强拒下载/current reuse。same-semantic refresh也必须经H(g+1)新adjudication和新report revision，不能复活旧seal。

Reporting必须把`declared_coverage`和`global_coverage_sufficiency`分开。Wave denominator只证明对已冻结plan的planned/tested/gap状态；在未来versioned `ThreatCoverageProfileV1`（asset class × trust boundary × attack class × role/identity × discovery source）能够建立全局分母前，`coverage_sufficiency=not_assessed`并强制negative-space residual，禁止任何“全覆盖”“无漏洞”或等价clean assurance文案。

renderer对所有target/org/title/description/limitation等untrusted字符串先Unicode normalize、长度限制并拒绝bidi/control字符，再按sink编码：React只text node；JSON只serde；Markdown escape且禁raw HTML/任意image/link；URL只允许版本化route/scheme；PDF/DOCX只用structured text；如有CSV则防公式注入。历史artifact读取使用root-relative no-follow regular-file stable snapshot，前后核对identity/size/mtime，把同一request bytes复制+hash到private sealed snapshot后再attest/render/download；active content默认attachment/octet-stream，preview必须sandbox+CSP+strict MIME。`<script>`、`javascript:`、Markdown image/link、bidi、CRLF和超长字符串都是固定安全fixture。

Verification Campaign 的 coverage authority 由 Plan C 拥有，不由 Plan A 或 Reporting 临时推导。每个 sealed Registry generation 在任何 Campaign/consult/action 调度前冻结 `verification_wave_denominator`，覆盖所有应验证的 `hypothesis_revision + verification_objective + required_control`；Wave consolidation 为每个 denominator item 写 exact-one disposition（`tested_complete / tested_degraded / untested / blocked`）并生成 immutable coverage receipt。Campaign terminal receipt、Prepared Action 或 scanner 数量都不能反推 denominator。Reporting 只消费该 receipt 和 exact residual membership。

历史 final report artifact 保持 immutable、继续通过 legacy artifact adapter 读取，不强制反序列化成新 typed claim。legacy operation 若创建新 report revision，先通过 `grandfathered_legacy_security_verdict` adapter 重建可证明的 source；无法证明的旧 JSON claim 降为 observation/limitation，不能令既有 final artifact 失效，也不能 fallback 成新 `security_verdict_authority`。

## 14. 概念数据模型与兼容迁移

实现预计需要 additive、forward-only schema；开始任何 migration 前必须再次取得用户明确授权。

建议的新 canonical entities：

- `capability_execution_receipts`；
- `attack_hypotheses`；
- `attack_hypothesis_revisions`；
- `attack_hypothesis_relations`；
- `attack_hypothesis_state_events`；
- `hypothesis_generations` / `hypothesis_generation_seals`；
- `candidate_analysis_snapshots`；
- `candidate_analysis_work_items` / `hypothesis_proposals`；
- `candidate_analysis_subreview_census` / `candidate_analysis_synthesis_census`；
- `input_processing_dispositions` / `input_hypothesis_relations`；
- `attack_hypothesis_claim_components` / `attack_hypothesis_verification_plans` / `..._plan_paths` / `..._plan_path_members`；
- `hypothesis_merge_decisions` / `hypothesis_evolution_decisions`；
- `fact_delta_consumptions` / `enrichment_obligations` / `application_fact_refinement_obligations`；
- `verification_capability_assessments`；
- `verification_campaigns`；
- `verification_campaign_rounds`；
- `verification_consults`；
- `verification_strategy_artifacts` / `verification_strategy_obligations`；
- `verification_prepared_actions`；
- `verification_prepared_action_group_members` / `verification_action_subexecutions`；
- `verification_prepared_action_authorizations`；
- `verification_action_conflict_sets` / `verification_action_conflict_set_members` / `verification_conflict_key_heads`；
- `verification_budget_ledger_entries`；
- `verification_oracle_assessments`；
- `verification_campaign_adjudications`；
- `verification_campaign_terminal_decisions`；
- `hypothesis_objective_claim_component_outcome_seals` / `..._members`；
- `hypothesis_objective_outcome_receipts` / `hypothesis_objective_outcome_heads`；
- `hypothesis_objective_outcome_set_seals` / `..._members`；
- `hypothesis_revision_adjudications` / `hypothesis_revision_terminal_decisions`；
- `verification_fact_delta_bundles`；
- `verification_wave_coverage_denominators` / `verification_wave_coverage_members`；
- `verification_campaign_coverage_denominators` / `verification_campaign_coverage_members`；
- `verification_campaign_coverage_results` / `verification_campaign_coverage_receipts` / `verification_wave_coverage_receipts`；
- `hypothesis_residual_risks`；
- `investigation_projection_changes`。

artifact ownership 固定如下，表名可在实现计划中按现有 repo 命名规范微调，但不能改变 authority：

| Artifact | Storage | Writer/owner | Identity | Consumer |
|---|---|---|---|---|
| input manifest / Tool Truth receipt | new capability receipt + existing evidence/business tables | typed adapter closeout | operation+attempt+capability+input hash | stage Gate、Hypothesis snapshot、Reporting |
| candidate snapshot/generation seal | dedicated snapshot/generation tables | server preflight/Controller terminalizer | org+generation+snapshot hash | Candidate Gate、workspace |
| proposal/relation/disposition | dedicated append-only analysis tables | analyst proposal；Controller decision | snapshot+work item+artifact id/hash | Candidate Gate、Registry reducer |
| hypothesis root/revision/event | dedicated Registry tables | deterministic Registry reducer | root id+revision ordinal+semantic hash | Verification admission、workspace、Reporting |
| capability assessment/enrichment | dedicated obligation tables | Action Compiler/policy/enrichment terminalizer | revision+objective+contract+policy | Campaign scheduler、residual |
| consult/strategy/plan delta | campaign round/artifact tables | host scheduler + Lead typed decision | campaign+round+artifact id/hash | obligation Gate、method audit |
| Prepared Action/authorization | new Prepared Action tables；link old action/receipt for compatibility | Action Compiler/Authorization Broker | round+strategy decision+private manifest hash | executor、JIT UI、Gate |
| execution receipt | existing action journal + exactly one capability receipt ref | typed executor closeout | Prepared Action+authorization+execution ordinal | oracle、budget、recovery |
| action oracle/objective adjudication | dedicated oracle/Campaign adjudication/objective outcome tables | deterministic oracle/objective terminalizer | execution receipt hash+rule digest / campaign+objective+contract hash | revision aggregate adjudicator、Gate |
| revision adjudication/verdict | B-owned verification plan + C-owned latest objective outcome exact set/adjudication/terminal tables | deterministic host aggregate adjudicator | revision+plan seal+outcome set+all-fresh temporal bundle | Finding/refutation、Gate、Reporting |
| FactDelta/consumption/evolution | immutable bundle + exactly-once consumption + proposal/decision tables | server terminalizer/consolidator/Controller decision | source campaign+delta id+semantic hash / proposal id | next generation、Reporting |
| Campaign coverage denominator/receipt | dedicated Wave + Campaign denominator/member/result/receipt tables | server admission + consolidation terminalizer | operation+generation seal+objective/control exact set | Verification Gate、Workspace、Reporting |
| projection change sequence | one append-only projection outbox/head/change ledger created in Plan B | same transaction as canonical write；Plan C/D only extend the frozen entity-kind vocabulary | operation+change seq | operation-level UI refresh |

迁移原则：

1. 不修改历史 migration；
2. 新列先 nullable/link-only；
3. old operation 按 frozen contract 使用旧 read/write path；
4. dual mode 只有一个 canonical writer，compatibility side 由同事务 outbox/rebuildable projection产生，不允许两个 best-effort writer；
5. comparison/read-model acceptance 通过后才切新 contract；
6. old Candidate/Attempt/Finding ids 和复合 FK 保持；
7.旧 hash 保留 replay，新 contract 使用包含 route/readiness/adapter/oracle/version 的 v2 hash；
8.任何 legacy/unsupported/partial 数据都只向显式legacy projection与residual映射，不能在迁移中回填canonical Registry，更不能变成新链refuted/checked-empty。

### 14.1 Operation-frozen rollout 状态机

每个 operation 创建时同时冻结 `tool_truth_contract`、`investigation_contract_version` 和 `investigation_rollout_mode`。三者是一个联合 execution authority，不能由不同 singleton/default 独立拼接：

```text
legacy_only
  -> shadow_registry
  -> dual_read_compare
  -> registry_authoritative_legacy_projection
  -> new_only
```

箭头表示“部署对新建 operation 的默认 mode 逐步推进”，不是已冻结 operation 自身迁移。

- `legacy_only`：旧 Candidate/approval/Attempt/Gate 是 authority；Workspace 显示只读 Legacy projection；
- `shadow_registry`：旧链仍是 authority，Registry 由 outbox/reducer生成 shadow；divergence 记录审计并阻止 rollout promotion，不阻断旧 operation；
- `dual_read_compare`：旧链仍驱动 runtime/Gate，Registry 只做 complete-record exact compare；divergence 记录审计、阻止 rollout promotion，不能逐字段 fallback或授权新路径；
- `registry_authoritative_legacy_projection`：Registry/Prepared Action/Campaign Gate 是 authority，旧 Candidate/Attempt 只由 canonical events生成 compatibility projection；projection divergence 不改变 canonical结论，但旧 consumer fail closed；
- `new_only`：只运行新 authority，仍可读取历史 legacy projection。

联合 pair 是七态闭集，其他组合一律非法：

| Joint rank | Tool Truth | Investigation contract | Investigation mode |
|---:|---|---|---|
| 0 | `legacy_v1` | `legacy_candidate_v1` | `legacy_only` |
| 1 | `shadow_v1` | `legacy_candidate_v1` | `legacy_only` |
| 2 | `shadow_v1` | `hypothesis_registry_v1` | `shadow_registry` |
| 3 | `shadow_v1` | `hypothesis_registry_v1` | `dual_read_compare` |
| 4 | `receipt_v1` | `hypothesis_registry_v1` | `dual_read_compare` |
| 5 | `receipt_v1` | `hypothesis_registry_v1` | `registry_authoritative_legacy_projection` |
| 6 | `receipt_v1` | `hypothesis_registry_v1` | `new_only` |

`shadow_registry / dual_read_compare` 只运行无副作用 shadow evaluation：冻结相同 generation/objective、编译 non-executable shadow signature，并消费旧链已经产生的 capability receipt/legacy action observation做 typed oracle/complete-record compare；shadow artifact 落独立 evaluation ledger，不创建 canonical Campaign/待审批动作，不发请求、不写 Finding/FactDelta、不改变旧 Gate。只有 rank 5/6 可进入真实 Campaign。

rollout policy 只能由 `InvestigationRolloutMode::policy()` 一个 exhaustive pure matrix 提供。Campaign admission 在该 policy 之上只增加 `tool_truth_contract=receipt_v1`、generation seal、capability/reconciliation 和 component-availability 检查，不能再定义第二套 mode 布尔矩阵；UI、legacy mutation guard、comparison、operation creation/fork 都消费同一 policy。

per-record comparison 也只有一套 append-only ledger：Plan B 创建并拥有 sample/canonical hash writer，Plan C/D 只扩展 record kind 与 canonical fields；Plan D 可以新增 cohort aggregate 和 promotion receipt，但不能创建第二套 sample 表或 comparator truth。

operation 创建后 mode 不可中途切换。same-operation resume/continuation 保留 exact mode；新 operation 的 immutable-source fork 必须显式选择一个能消费 source final seals 的新 contract，并写 adoption receipt，不能隐式升级 source operation。

deployment default 只能按联合 pair 前进，不能回退：

每次只允许 joint rank `+1`；关键边的额外条件为：

1. rank 0→1：只启用 Tool Truth shadow writer，要求 additive schema/reconciliation writer健康；
2. rank 1→2：启用 Registry/shadow evaluator，要求 reducer和无副作用 port-isolation fixture健康，不要求 compare cohort；
3. rank 2→3：要求 closed shadow cohort且 mismatch/missing/incomplete/corrupt 全为零；
4. rank 3→4：在旧 Candidate authority 下启用 receipt-v1 fail-safe producer projection，要求 Tool Truth shadow reconciliation全闭合；
5. rank 4→5：要求closed dual cohort exact-set全绿、Campaign shadow replay complete，以及通过显式fork/adoption创建且另获执行授权的authoritative canary在action/receipt/oracle/coverage/report dry-run上全绿；此外必须通过versioned adversarial acceptance corpus：known-vulnerable、known-safe/control-failure、soft-404/WAF/dynamic baseline、multi-role/IDOR与race adapter-missing等每个fixture都有独立expected verdict/residual exact set；
6. rank 5→6：要求 compatibility projection健康、旧 mutation/read fallback调用为零、旧 consumer清单为空或已迁移、历史 read adapter可用。

每条边使用不同typed promotion evidence；不能用一个`compared_operations > 0` blanket条件。whole-record新旧一致只证明compatibility safety，不证明检测正确性：两边共享同一漏检时mismatch仍为零，所以独立对抗corpus是4→5不可替代的ground truth。`change_seq`是operation-local，cohort为每个operation冻结独立`as_of_change_seq`和temporal cutoff/epoch set，再对按operation排序的member manifest整体hash；TTL在无change event时过期也必须令current pagination/cohort stale。

唯一生产调用面是local admin CLI的dry-run/apply路径，必须重验active local principal、expected row versions、cohort/corpus manifest，在同一transaction按固定顺序锁住Plan A revalidation dispatch head、Plan C safety-hold singleton、Tool Truth rollout和Investigation rollout，写immutable joint promotion receipt后再CAS defaults；promotion仍需单独用户授权。运行控制有三个相互独立的hold/generation：`tool_truth_revalidation_dispatch`、`campaign_dispatch`、`operation_admission`。operation创建时另冻结revalidation policy，默认`manual_only`；只有显式`auto_passive_t0_t1`且第一hold解除才允许active operation自动刷新T0/T1，T2/T3始终走Prepared Action/JIT。初始campaign与revalidation dispatch均held、operation admission未held；任一hold变化不推进其他generation，旧authorization/claim不能跨on→off generation复活。线上异常只允许设置对应hold或发布前向contract，禁止倒退singleton/default。

legacy operation 打开 Investigation Workspace 时：

- Candidate/Attempt 映射为只读 Hypothesis/Campaign；
- 无法重建的 request packet、oracle、consult 或 strategy 显示 `legacy_unavailable`，不能显示空、0 或“已检查”；
- 旧 queue 只进入 Audit drawer；
- `attack_review_candidates/resume` 在 `legacy_only / shadow_registry / dual_read_compare` 保留必要 mutation；从 `registry_authoritative_legacy_projection` 起隐藏并由 Prepared Action JIT 替代；
- rollout 默认变化不能改变既有 operation 的 UI/authority。

## 15. 错误、恢复与并发

- 每个真实 Action 同时只能有一个 active owner；consult 可在 bounded lane 并行；
- action `outcome_unknown` 只能恢复或人工裁决，不能自动重放；
- late tool result 只能写 superseded witness；
- response-loss replay 返回相同 Prepared Action/receipt/result，不再次调用外部工具；
- DB deadlock/serialization 属于基础设施 retry，不得消耗业务 hypothesis attempt；
- 一条 branch 的 adapter/enrichment blocker 不应阻塞无依赖 branch；
- 任何 scope/org/target/credential/template/oracle drift 都使旧 Prepared Action 失效；
- 外部 HTTP/MQ/长耗操作不在 DB transaction 中执行；
- Finding 和 canonical fact 仍只由 deterministic terminalizer/consolidation 写入。

所有新失败路径必须返回稳定 `code`，至少区分 contract invalid、authority stale、capability unavailable、policy denied、budget exhausted、outcome unknown、oracle inconclusive、landing partial 和 reconciliation orphan；前端按 error-code map 翻译，不能根据 HTTP status 或自由文本猜业务语义。

Investigation read IPC的公共错误集合冻结为`INVESTIGATION_FORBIDDEN`、`INVESTIGATION_INVALID_ID`、`INVESTIGATION_INVALID_ARGUMENT`、`INVESTIGATION_CURSOR_INVALID`、`INVESTIGATION_PROJECTION_STALE`、`INVESTIGATION_AUTHORITY_CORRUPT`、`INVESTIGATION_DATABASE`、`INVESTIGATION_LEGACY_PROJECTION_DIVERGED`。公开DTO中的ID使用string并由handler解析，避免UUID/serde错误绕过稳定code；unknown或互斥filter使用`INVESTIGATION_INVALID_ARGUMENT`，空filter数组才表示“不限制”，不能把非法filter解释成空结果。detail command先授权operation，再解析并核对selector membership，防止跨operation存在性泄漏。

## 16. 分阶段实施边界

本文规格应拆成四份实现计划，不能作为一个巨型改动一次落地：

### Plan A：[Tool Truth 与 Coverage Contract](../superpowers/plans/2026-07-29-tool-truth-coverage-contract.md)

- 统一 receipt/status ontology；
- input manifest、actual budget 和 reconciliation；
- 修复 EAS empty stdout、Enumeration one-failure-four-axis、positive-masks-partial 等直接假阴性；
- 对 scanner/Nuclei no-match 只做兼容 fail-safe：不再提升为广义 checked-empty/refutation；完整 recipe oracle 留给 Plan C；
- Gate control/coverage grade 先 shadow-write；若切 user-visible，Plan A 同时提供最小 API/UI/report 映射，不能让新状态被旧 UI 画成绿色完整覆盖；
- 不改变 frozen old operation。

### Plan B：[Hypothesis Registry 与 Candidate Analysis Team](../superpowers/plans/2026-07-29-hypothesis-registry-candidate-analysis.md)

- 先落 `investigation_contract_version / rollout_mode` 的 operation-frozen 基础和 mode-gated writer/dispatcher；默认仍为 legacy，shadow/dual 不得授权新执行路径；
- additive registry/event/lineage；
- frozen candidate analysis snapshot；
- Controller + read-only analyst/critic 两波流程；
- deterministic proposal census/merge/readiness Gate；
- 旧 Candidate/NoCandidate compatibility projection；
- Hypotheses compatibility read model，使 Plan D 前也可审计 Registry。

### Plan C：[Verification Campaign、Prepared Action 与 Oracle](../superpowers/plans/2026-07-29-verification-campaign-prepared-action-oracle.md)

- 持久 campaign round/consult/strategy/refinement；
- exact Prepared Action 和风险分层 JIT；
- 交付最小但完整的 Prepared Action read/review/mutation API 与安全 UI：T2/T3 packet、hash/drift/expiry、approve/reject、三态与审计；Plan D 只把它迁入并丰富 operation-level Workspace，Plan C 不得依赖尚未实现的 Plan D 才能安全授权；
- 在现有 Pane 提供 operation-scoped durable `Pending Prepared Action` 入口/通知；cold start、restore、missed event 或没有 selected tool 时都主动 DB bootstrap，Plan D2 只迁移布局；
- planned/actual budget accounting；
- typed oracle；
- 修正 Nuclei/anonymous/directory 语义；
- immutable FactDelta exactly-once consumption + evolution proposal decision → next generation；
- deterministic stall/fixpoint。

### Plan D：[Investigation Workspace、Reporting 与 Rollout](../superpowers/plans/2026-07-29-investigation-workspace-reporting-rollout.md)

- D1：operation-level read model、fixed projection schema version/change sequence、legacy projection；
- D2：Workspace route 与 Hypotheses/Campaigns/Waves/Timeline，移除 queue-centric primary UX；
- D3：Reporting canonical sources、denominator、limitations 和 report-input seal；
- D4：shadow/dual-read acceptance、comparison Gate、对“新建 operation 默认 mode”的 promotion 和 legacy replay；不在此时才首次实现 contract/mode 安全隔离。

每份计划都必须独立提供可工作的增量、定向测试和 rollback/cutover 证据。任何计划都不能靠未实现的下一计划才能保证旧 operation 安全。

## 17. 验收标准

### 17.1 Tool Truth

- 工具 exit 0 但没有处理全部 input 时不能得到 complete/checked-empty；
- raw positive + parser rejection 会产生 partial/orphan，不会普通 PASS；
- late previous-attempt negative row 不能关闭新 attempt；
- network governor 在发送第 N+1 个请求前 fail closed 并产生审计事件；
- blocked 能推进时显示 PASS_WITH_GAPS 并进入 Hypothesis/Reporting。

### 17.2 Candidate/Hypothesis

- 一个 observation 可以产生多个有 lineage 的 hypothesis；
- 多个 observation 可以支持一个 hypothesis；
- 同一 input 可同时创建、支持和限定不同 hypothesis，同时 processing disposition 仍 exact-one；
- Vuln blocked、partial、template-only、adapter-missing 均有 durable registry state；
- AU 能产生业务逻辑/authz hypothesis context，但不能成为 proof；
- 一个仅由 AU/context/coverage gap 驱动且没有 typed adapter 的 hypothesis 仍能 canonical seal 并进入 strategy planning；
- analyst 漏读、截断、跨 org 引用、proposal orphan、非法 merge/split 均 Gate BLOCK；
- 每个input/checklist/chunk subreview与跨chunk/input synthesis exact census闭合；不是只有zero-proposal才做coverage review；
- snapshot使用server-derived checked multi-root bundle，required root任一expired/orphan/mixed-epoch/skew都记录census/residual并BLOCK；feed snapshot stale/revoked或产品版本未知不能成为proof；
- B-owned verification plan的claim components、impact/identity qualifiers和每条proof path exact覆盖；缺objective必须收窄claim，不能seal一个无法证明原claim的plan；
- 同义 proposal 可确定性合并且历史不丢失；
- exact semantic key proposal 自动聚类；semantic key 不同的相似 proposal 只有 typed Controller decision 才能合并，跨 org/target/trust/polarity merge 必须 BLOCK；
- historical semantic key revival、current-key collision 和 root-id collision 按固定 reducer route 收口；provider 完成顺序不能改变 root/revision identity；
- split/merge superseded parent 通过 transition exact mapping 退出 successor membership，successor roots 可 final seal且 lineage 无 orphan；
- 每个 expected FactDelta 永不被 accept/reject，且仅有一个 `applied / no_semantic_change / quarantined_invalid_authority` consumption receipt；
- material delta 即使只关闭最后一个 hypothesis 也生成并 seal 新 generation，但没有 runnable obligation 时不创建空 Wave；
- 全部 `no_semantic_change` 的 batch 只生成 fixed-point receipt，不创建空 generation/Wave；
- application-fact refinement 未处置会进入 open-obligation hash 并阻止 seal；accepted disposition 没有同事务 sealed revision/adoption receipt 也必须 BLOCK；
- no-adapter/no-action 的 inconclusive Campaign 不得把 proposed hypothesis 自动升级为 supported；
- quarantined authority 未裁决必须 HOLD；verified/refuted bundle 被 quarantine 时 Finding/terminal receipt 会失效且不能进入 active report；
- 两个 Campaign 并发产生 support+contradict且同时预算耗尽时，系统先 drain/consolidate，再形成 contested residual 和稳定 semantic hash；
- Candidate 页面显示主 AI、真实 subagent、分析进度和 hypothesis 演化，不显示 FIFO queue 作为主视图。

### 17.3 Verification

- 一个 campaign 可持久执行多个 round 和多种策略；
- 同revision可有多个objective-local Campaign；单Campaign只写objective outcome/coverage/FactDelta，不能创建Finding或revision终态；
- consult可以并行但普通真实Action单lane；race/TOCTOU只通过sealed ConcurrentActionGroup的barrier、subreceipt exact census与concurrency oracle验证；
- no_action_compilable/denied round 不产生 fake execution/oracle；Campaign exhausted 只等待 local drain，不等待 Wave consolidation才 terminal；
- 每个 verification objective/strategy/control 都有 durable disposition，Refiner 不能静默删除；
- Lead 无法自填 raw target/credential/canonical args/oracle；
- 具体 request/sequence 在审批前可见且 hash-bound；
- denied/expired/superseded Prepared Action 不要求 fake execution/oracle，但必须有 reason/residual；
- manually_blocked 保留原 outcome_unknown witness、人工裁决 receipt 和 residual；
- 复合authz/browser/business claim先由objective Campaign聚合多action oracle，再由revision aggregate adjudicator消费Plan B proof paths；单action或单Campaign proof不自动verified；
- Nuclei no-match 默认 inconclusive，不会错误 refute 广义 hypothesis；
- authz/directory/injection/browser oracle 有明确 controls；
- 新 evidence 可 support/contradict/refine/split/merge/derive 下一 Registry generation；
- 预算耗尽先 stop scheduling并 drain callback/cleanup/recovery/delta，再形成 residual；
- execution idempotency与semantic no-progress fingerprint分别阻止response-loss重放和无实质变化重试；
- revision aggregate terminalizer在独立事务创建/复用Finding或refutation lineage、terminal decision与state event；Campaign closeout失败或aggregate失败都不能留下半个verified/refuted状态；
- admission/begin/closeout/revision adjudication四处都重验server-derived Plan A all-fresh multi-root bundle、same epoch/max-skew/TTL；terminal后TTL到期变temporally_stale并经H(g+1)重新裁决，same-semantic refresh不能复活旧Finding；
- conflict按per-key head解决partial overlap；unknown/cleanup未完进入recovery hold，不能因lease时间到自动复用；
- Wave denominator在首个Campaign admission前冻结全部objective；Campaign denominator在首个授权前冻结不重叠partition；terminal时每个member恰有tested outcome或residual，未生成action的objective也不能从分母消失；
- shadow/dual只运行isolated planner/matcher/receipt-oracle replay；panic executor、authorization token、lease、journal、budget reservation和新capability receipt计数全部为零，shadow artifact不能进入Gate/Finding/FactDelta/Reporting。

### 17.4 Frontend/Reporting

- Candidate/Verification roadmap 节点均可直接打开 operation-level workspace；
- Candidate 节点默认 Hypotheses、Verification 节点默认 Campaigns；live/completed/restored 且没有 selected tool/review trace 也能打开；
- refresh event 丢失/乱序不影响 mount DB bootstrap，旧 projection response 不覆盖新版本；
- Hypothesis、Campaign、Wave 跨 generation 可导航，gen0 → FactDelta → gen1 可点击并恢复选择；
- Authorization Packet 展示 redacted concrete action，而不是只有 receipt id；
- T0/T1 只有 audit、无人工按钮；T2/T3 packet/digest漂移立即禁用 approval；secret/raw payload 不进入 DOM；
- Plan C 独立交付时，cold start/missed event/no selected tool 仍可发现 pending T2/T3；approve/reject 使用 CAS，expiry/supersede/drift 禁用旧按钮，response-loss 复用 stable request id；
- protocol ids/hash 位于 Audit details；
- 主 DOM 无 `Queue N` 和重复 Attempt；team 只显示真实 durable activity，无记录显示 `not recorded`；
- legacy/new contract 均可打开；legacy 缺失字段显示 `legacy_unavailable`，rollout default变化不改变既有 operation；
- Hypotheses/Campaigns/Waves 分别有 loading/error/empty，refresh保留旧数据和 stale 提示；
- `PASS_WITH_GAPS` 可允许 roadmap继续，但不能渲染为绿色完整覆盖，exact residual/affected inputs可见；
- 1k+ hypotheses/multi-wave 使用分页/虚拟化，不加载全量 payload，并覆盖键盘/aria与中英文文案；
- report 包含 no-candidate disposition、enrichment、capability gap、blocked/inconclusive/exhausted residual；
- method/authorization/raw observation或单Campaign在没有revision aggregate adjudication时不能生成verified/refuted文案；含token、PII、cookie、raw response和payload的fixture不得出现在report DOM/export；
- findings=0 时仍能区分完整覆盖与未测试；
- ThreatCoverageProfile未实现前`coverage_sufficiency=not_assessed`，即使declared denominator全测也不得显示“全覆盖/无漏洞”；
- report current seal绑定Plan A all-fresh temporal bundle；TTL过期显示as-of/temporally_stale，semantic invalid显示revoked；历史download读取同一request sealed snapshot并经过sandbox/attachment policy；
- renderer对HTML/URL/Markdown/bidi/control/CRLF/huge string做per-sink安全编码和边界测试；raw vault明文不落盘、访问审计、retention后crypto-erasure；
- typed legacy verdict明确显示legacy authority与coverage unavailable；既有final legacy artifact bytes/hash保持不变，新revision只能从typed legacy adapter重建。

所有新增 operation/campaign/hypothesis read API 都必须有跨 project、跨 organization、stale scope 和 deleted live target 的 ownership/at-time identity 回归测试。

### 17.5 Compatibility/Rollout

- `shadow_registry` divergence 只写审计并阻止 rollout promotion，不改变旧 runtime/Gate；
- `dual_read_compare` divergence 不得逐字段 fallback，也不得授权 Registry/Campaign 新执行路径；
- `registry_authoritative_legacy_projection` 的 projection divergence 不回滚 canonical truth，但旧 consumer 必须 fail closed；
- same-operation resume/continuation 保持frozen Tool Truth + Investigation完整joint pair；immutable-source fork无receipt完整继承，有receipt也只能前进一阶；
- deployment default/mode promotion 不改变任何已冻结 operation；
- 五种mode的canonical writer、Gate authority、legacy mutation、Prepared Action JIT和compatibility projection允许集合必须做exact matrix contract test；
- 数据库只接受七个joint rank；promotion每次只允许`rank + 1`并根据具体edge重算证据，cohort使用per-operation change cutoff manifest；两侧CAS任一失败全部rollback；
- whole-record green只证明compatibility；4→5另需versioned known-vulnerable/safe/control-failure/soft404/WAF/dynamic/IDOR/race adversarial corpus及独立expected outcome exact set；
- rollback不倒退default或改旧operation，只通过三个独立append-only hold/generation分别停止revalidation dispatch、Campaign dispatch和operation admission，保留closeout/recovery与全部审计证据。

## 18. 定向验证策略

实现阶段默认只运行受影响模块的定向验证，不自动运行 `init.sh`、`just precommit`、全 workspace Rust/前端测试或真实外部扫描。各计划至少覆盖：

- DB migration/repository contract tests；
- pure Gate/oracle/hash/evolution unit tests；
- scheduler/lease/recovery focused tests；
- adapter fault injection；
- focused Vitest、Biome、typecheck；
- JSON/spec/diff checks；
- 一组无外部目标的 replay fixtures。

真实目标、真实 provider、schema migration 应用、rollout 切换和全量门禁均需单独明确授权。

## 19. 与既有设计的关系

本文已获用户确认，并成为以下主题的现行product/architecture authority：

- `2026-07-22-application-understanding-candidate-verification.md` 中 Candidate 仅从已有 work item 形成可执行 hypothesis 的边界；
- `2026-07-23-company-isolated-pentagi-verification-team.md` 中 queue-centric、ephemeral consult 和 capability-only action proposal 的 strategy portions；
- `2026-07-22-candidate-quality-and-verification-cutover.md` 中 Candidate/NoCandidate 二态作为完整 hypothesis truth 的部分；
- `2026-07-28-candidate-verification-continuation.md` 的 continuation 仍作为历史 operation compatibility path，不作为新 Investigation 产品模型。

既有action receipt、scope、organization isolation、lease/CAS、recovery、evidence、Finding lineage和deterministic reducer骨架保持有效。被取代主题已在旧文档头部标出partial supersession；Plan A–D四份implementation plan已创建并登记，但仍全部是`not_started`，不代表实现或migration授权。

## 20. 规格完成检查

本设计固定了：

- 三个事实平面的职责和接口；
- Candidate 与 Hypothesis 的概念边界；
- Candidate 分析团队的 owner/subagent 权限；
- 内外两层循环及 evolution operators；
- Verification Campaign、Prepared Action、授权和 oracle；
- Gate、blocked、coverage 和 Reporting 语义；
- Investigation Workspace 信息架构；
- additive compatibility/rollout 原则；
- 四份实施计划的拆分边界；
- 确定性停止条件和验收标准。

本文没有授权实现或schema变更。核心产品语义、authority、状态机、循环、UI truth与rollout决策已获用户确认，并作为Plan A–D的冻结边界；实现时可以按现有命名规范细化表/DTO/函数名称，但不得自行改变已确认边界。
