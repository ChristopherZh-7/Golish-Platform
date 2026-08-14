# RAG-first 单阶段 Investigation 设计补遗

> Superseded in part by `docs/design/2026-08-12-investigation-primary-led-verification-execution.md` for Primary ownership, Verification worker capabilities, continuous hypothesis refinement, and the host/Agent execution split. The unified stage, Registry, evidence, JIT, Oracle, FactDelta, Reporting and rollout boundaries remain authoritative.

> **状态**：Approved for implementation planning（用户于 2026-08-02 确认RAG-first、单一ChatPanel阶段入口、现有全屏Agent工作区、自动验证调度、点击hypothesis只查看运行过程，以及复用PentAGI式Generator→Primary→dynamic/nested specialists→Refiner/Reflector多Agent逻辑）
>
> **日期**：2026-08-02
>
> **部分取代**：[`2026-07-29-tool-truth-hypothesis-verification-loop.md`](2026-07-29-tool-truth-hypothesis-verification-loop.md) 中 Candidate/Verification 两个可见阶段、顶层 Candidate→Verification 阶段切换，以及独立 Investigation Pane route 的产品交互；原设计的 Tool Truth、Hypothesis Registry、自动 Campaign 调度、Prepared Action、JIT、Typed Oracle、FactDelta、Reporting authority 和 rollout 安全边界继续有效。
>
> **授权边界**：本文只批准设计与实现计划，不授权 schema/migration、生成 IPC、下载或再分发外部 corpus、访问真实目标、调用外部 provider、启动攻击或切换 rollout。

## 1. 一句话结论

新建 operation 只进入一个用户可见的 `Investigation` 阶段：主 Agent 先绑定服务端冻结的 exact snapshot，并为每个organization启动独立、durable的隔离读取session来读取bounded canonical facts、RAG/知识图谱、Application Understanding、指纹、Nuclei与方法论上下文；主协调context只接收这些session的typed/redacted receipt，随后协调只读specialist agents形成canonical hypotheses。generation seal后，调度器自动为可验证hypotheses创建验证任务并运行Verification specialists。用户点击hypothesis只是查看其Agent、计划、工具、证据和实时过程，不触发mutation。

这不是把所有权限交给一个超级Agent。用户看到一个Main和一个工作区；内部仍由host、per-org Analysis TaskWorker、per-hypothesis Verification TaskWorker、dynamic specialists、Prepared Action broker与deterministic oracle分权。

## 2. 用户实际看到的流程

```text
ChatPanel
└─ Investigation
   └─ Running specialist agents
      └─ 点击后进入现有全屏 Agent 工作区
         ├─ Main Agent
         ├─ Hypothesis analysis specialists
         ├─ Hypotheses
         │  ├─ H1  [验证中]
         │  │  └─ Verification Task Primary + dynamic/nested specialists
         │  ├─ H2  [排队中]
         │  └─ H3  [需要补充信息]
         └─ Evidence / Campaign / Timeline
```

用户不再看到 Candidate 完成后再进入 Verification 的第二张阶段卡，也不需要从一个详情页跳到另一个 Workspace。

## 3. 为什么必须是一个真实外层 stage

只在前端把两个旧 `stage_run` 画成一张卡会留下两个真实 owner、两个 request identity、两个 Gate 和一次阶段切换。这样会产生四个问题：

1. 自动验证调度仍需等待或伪造 Candidate→Verification continuation；
2. Verification 产生的新 hypothesis 必须退出当前详情再重开 Candidate；
3. 历史恢复时无法用一个 exact `stage_run request_id` 还原完整 Agent 树；
4. 前端会被迫合并来自两个 operation cursor 的状态，破坏 request-scoped truth。

因此新 contract 使用一个真实 `StageKind::Investigation` 和一个 exact `stage_execution/stage_run` identity。Hypothesis Analysis 与 Verification Campaign 是该 stage 内的两类持久 work item，不是两个顶层 stage。

统一 contract 的唯一前驱链是：

```text
vuln_triage → application_understanding → investigation
                                               ├─→ reporting
                                               └─→ access_validation → … → reporting
```

不存在 `vuln_triage → investigation` 直达边，也不允许把不含 Application Understanding 的 legacy base graph解释成 unified graph。实现新增 versioned unified graph，由当前 application-model graph派生；legacy graph保持原样。

operation 还必须冻结独立的 `stage_topology_contract`：

- `legacy_candidate_verification_v1`；
- `unified_investigation_v1`。

它与 Plan B 的 writer/rollout mode形成closed legal pair，并进入 operation receipt、fork adoption、resume、history、reporting 与 promotion comparison。新建operation的合法矩阵固定为：

| rollout mode | stage topology contract | runtime authority |
|---|---|---|
| `legacy_only` | `legacy_candidate_verification_v1` | legacy Candidate/Verification |
| `shadow_registry` | `legacy_candidate_verification_v1` | legacy runtime；Registry只做shadow |
| `dual_read_compare` | `legacy_candidate_verification_v1` | legacy runtime；Registry只做compare |
| `registry_authoritative_legacy_projection` | `unified_investigation_v1` | unified authority；legacy仅compatibility projection |
| `new_only` | `unified_investigation_v1` | unified authority |

已经冻结为旧组合的历史operation不被全局默认重解释；只有带exact adoption receipt的新fork可以采用新矩阵。validator拒绝其它新组合。

历史 operation 不原地改写：

- 已冻结为 legacy contract 的 operation 继续 `attack_candidate → verification`，沿用现有兼容 UI；
- 新建且冻结为 unified contract 的 operation 走 `application_understanding → investigation`，再按operation-frozen profile选择直接Reporting或进入Access Validation链；
- rollout、resume、fork 与 reporting 必须根据 operation-frozen contract 选择路径，不能按当前全局默认重新解释历史。

## 4. 内部职责分工

| 角色 | 可以做什么 | 不能做什么 |
|---|---|---|
| Investigation Main Agent | 在任何dispatch前为每个organization创建exact snapshot-bound read session；只消费各session的typed/redacted receipt，展示全局进度、协调分析与验证任务、解释typed artifacts | 把organization正文写入协调transcript、在同一模型context混入多个organization正文、直接写事实、绕过Gate、拼接攻击参数 |
| Main organization read session | 只在exact `stage_run_unit + organization + snapshot`分区内读取bounded正文并提交typed receipt | 把raw transcript/context带到另一organization或直接派发攻击 |
| Analysis TaskWorker / Primary | 为一个 organization 冻结分析任务，使用Generator拆出2–8条只读subtask并动态派工，提交proposal/conflict output census | 扫描、联网、执行exploit、写Finding |
| Analysis specialist | 作为PentAGI-style worker关联surface、AU、auth、business logic、coverage gap与方法论signal，并可在allowlist内嵌套委派 | 写canonical revision、调用主动工具 |
| Verification TaskWorker / Primary | 围绕exact hypothesis revision/plan生成和refine subtasks，动态委派Pentester/Researcher/Browser/Coder等worker | 改写claim、直接批准动作、直接判定漏洞成立 |
| Verification specialist | 根据动态subtask提出strategy、研究、工具/PoC准备、执行建议或反证 | 绕过Prepared Action直接执行、写oracle verdict |
| Generator / Refiner / Reflector | pipeline-only拆分subtask、每步后调整剩余计划、无有效tool call时纠偏 | 作为可派worker、扩大sealed objective、改变scope/authorization |
| Host/DB reducer | 重算 identity/hash、创建 task/campaign、检查 scope/CAS、写 revision/lineage | 依赖模型自然语言做 authority 决定 |
| Prepared Action broker | 编译 exact request、风险分级、JIT、lease/budget | 把自动 task admission 当成攻击批准 |
| Typed oracle/adjudicator | 根据 current evidence 判定 objective 与 revision | 把 RAG、方法论命中或单个 Campaign prose 当 proof |

一个 operation 只有一个可见 `stage_run request_id`，但每个 organization 拥有独立 `stage_run_unit_id`、Analysis TaskWorker、snapshot、hypothesis、task 与 Campaign。operation-level Main Agent 是 `stage_run` 下真实、durable的coordinator child actor。每个`MainOrganizationReadSessionV1`绑定独立`main_read_session_id + stage_run_unit_id + organization_id + snapshot_id/hash + context_chain_id + transcript partition + seal receipt`；原文只存在于该partition。Main coordinator transcript只保存typed/redacted receipt。crash/resume/replay只能从同一session partition恢复，禁止用Main总transcript重建另一个organization的context；单organization也遵守同一边界。

transcript identity不再使用含糊的`agent_request_id/parent_request_id`：

- `owning_stage_run_request_id`：拥有该树的exact `stage_run` tool request；
- `transcript_request_id`：必须精确等于live `ActiveSubAgent.parentRequestId`，也是现有transcript lookup key；
- `parent_actor_transcript_request_id`：可空，指向逻辑父actor的transcript identity；
- `parent_dispatch_tool_request_id`：可空，指向父actor创建当前child的exact tool-call edge；
- `worker_run_id`与`stage_run_unit_id`：绑定durable执行和organization隔离。

Main也必须拥有真实的`transcript_request_id/worker_run_id`；缺失、冲突或恢复后无法证明映射时UI显示unavailable。不得用前端synthetic `__main__`根、latest lookup或猜测identity伪造Main transcript。

每个进入执行的analysis attempt或verification task不是“一个Agent自问自答”：必须先seal动态`PentagiTaskPlanV1`（ordered subtask exact set、allowed role catalog、cognitive tool envelope与fuel）。每个task只有一个Primary；Primary为每个runnable subtask至少委派一个不同actor identity的specialist，worker可继续有界嵌套委派，但不得再为每个subtask创建第二层Primary。角色由Primary按任务缺口动态选择，不硬编码每次都出现同一组Strategist/Critic；实际dispatch、nested delegation、result barrier、Refiner patch和Reflector纠偏全部进入durable census。缺少独立worker只能`blocked_with_residual`，不能由Primary单Agent terminal。

派工只有一个writer：automatic admission scheduler只claim `HypothesisVerificationTask`并写一个幂等`PentagiTaskRunRequested`；现有`TaskOrchestrator`唯一拥有Generator、Primary、dynamic/nested worker dispatch、Refiner与Reflector。`StageTeam`只作为Task/Subtask/worker identity的持久化adapter和read projection，不能第二次调用独立dispatch loop。`stage_team_plans`保存unit治理envelope，`PentagiTaskPlanV1`保存analysis attempt或verification task的ordered subtask set；`stage_work_items/stage_worker_requests/stage_worker_runs`保存实际delegation edge与actor，append-only pipeline event保存Refiner/Reflector，最终seal count/set hash。一个subtask的同一cognitive dispatch receipt只能产生一次。

统一stage的通用deliverable保持`findings_allowed=false`。只有host-owned revision adjudicator在重验sealed plan、objective exact set与current evidence后，可以通过专用Finding writer落库；Main、Analysis/Verification TaskWorker Primary与任意specialist均不能提交Finding。

### 4.1 复用现有 PentAGI substrate

Golish现有task engine已经实现PentAGI式`Generator → TaskWorker/Primary → dynamic specialist delegation → Refiner → next subtask → Reporter`，并支持specialist嵌套委派、agentic tool loop与Reflector。统一Investigation必须复用这套substrate，不另建一套固定“评审席位”orchestrator。

参考基线：

- [PentAGI flow execution（commit 879e87c）](https://github.com/vxcontrol/pentagi/blob/879e87c2c2688c4a95eac9c1aaf3cd6f6123ebe3/backend/docs/flow_execution.md)
- Golish现状参考：[`2026-06-02-pentagi-engine-substrate-reference.md`](2026-06-02-pentagi-engine-substrate-reference.md)

映射固定为：

```text
Investigation stage_run                      = Flow治理边界
per-org Hypothesis Analysis                  = read-only TaskWorker
HypothesisVerificationTask                   = verification TaskWorker
verification plan objective / Campaign work  = host-bound Subtask/Campaign
Primary Agent                                = 动态派工，不直接执行
Pentester/Researcher/Browser/Coder/Installer/
Enricher/Memorist/Adviser                    = 按需worker，可有界嵌套委派
Generator/Refiner/Reflector                   = pipeline-only cognition roles
Prepared Action/JIT/Oracle/Gate               = Golish host治理外壳
```

Generator/Refiner可以重排“怎么做”，但不能增删sealed verification objective、改变target/scope/credential/risk、把RAG当proof或绕过action compiler。PentAGI的自由度存在于strategy/subtask/delegation层；truth与副作用authority仍属于Golish host。

## 5. Investigation 输入不是一段大 prompt

### 5.1 三类 authority

Investigation 的dynamic analysis subtasks开始前，服务端构造并封存 `InvestigationAnalysisSnapshotV1`。快照把输入分成三类，UI 和模型都必须保留分类：

| 类别 | 例子 | 能否证明漏洞 |
|---|---|---|
| canonical current truth | exact operation 的资产、指纹、Nuclei observation、technique outcome、AU revision、evidence、coverage gap | 只有符合 VerificationContract 的 current evidence 才可能进入 proof/refutation |
| RAG/KG prior | Assertion、Document、Temporal Graph、VectorPrior、历史 episode | 否；始终 `must_revalidate` |
| methodology signal | Golish playbook、CyberStrike skill、安全技能 corpus、冻结的在线 enrichment | 否；只能触发 hypothesis、strategy 或 enrichment |

### 5.2 两段冻结，而不是边分析边改快照

为同时支持“先完整读取当前bounded snapshot”和“分析 Agent可以补充检索方向”，快照生命周期固定为：

```text
host seal BaselineContextSnapshotV1 (S0)
  → bounded methodology query-planner wave reads S0
  → Agent submits typed MethodologyQueryIntentV1 only
  → host normalize/dedupe/policy/budget + local retrieval
  → optional policy-approved enrichment worker freezes public result
  → host seal InvestigationAnalysisSnapshotV1 (S1)
  → dynamic analysis subtasks read S1 only
```

sealed snapshot、query set与result census永不追加或改写。query planner失败、在线enrichment完成、material FactDelta、authority epoch变化或重试都创建新的attempt与successor snapshot identity；不能向S0/S1 post-seal append。Main Agent在任何child dispatch前绑定当前snapshot manifest/hash，并通过host-issued、single-organization read session读取bounded正文；分析child只拿同一organization的scoped projection。

### 5.3 复用现有 ContextPack

RAG/知识图谱不另建第二套检索引擎。新 stage 复用现有 `ContextPackProvider`：

```text
scope authorization
  → classification policy
  → canonical
  → runtime
  → handoff
  → episode
  → assertion
  → document
  → temporal graph
  → vector
```

服务端使用 exact `operation_id + stage_execution_id + stage_run_unit_id + organization_id + stage_kind` 创建 `ContextSubject`。Agent 不得自报 project、organization、scope 或 classification ceiling。

当前 `ContextPack` 是有界的 prompt-safe retrieval view，不等于 Candidate 输入全集。`InvestigationAnalysisSnapshotV1` 是 Plan B `CandidateAnalysisSnapshot` 的扩展/超集，不得把其 rich checked authority降级成一个hash。它还必须封存：

- Plan A checked `AllFreshToolTruthAuthorityBundle` 的exact bundle/root/member identity、count/set hash、temporal cutoff与guard-bound source refs；
- predecessor final seals 与 exact source sets；
- current Application Model revision/items；
- fingerprints、product/version candidates、routes、auth flows 与 Nuclei outcomes；
- partial/sampled/template-only/blocked/unchecked gaps；
- ContextPack item membership、content hashes、omission reasons 与 token budget；
- methodology query/result census；
- previous hypothesis generation、relevant evidence snapshot、FactDelta watermark、open obligation exact set与semantic attempt fingerprint；
- capability、policy、credential authority revision。

mandatory canonical/runtime 超出预算时 fail closed；optional RAG layer可以显式 degraded，但 omission 必须形成 census/residual，不能把截断当完整读取。资产banner、route、Nuclei output、AU prose与其它target-derived正文同样按untrusted data封装；它们不能改变role、tool、schema、scope、authorization或system instruction。

## 6. CyberStrike 与安全技能 corpus

### 6.1 借鉴什么

以审计时冻结的 CyberStrike commit `80ee899a4ccb2a152fb505e7ce9e1a7874b1f486` 为参考，Golish 借鉴以下方法，不直接复制其运行权限模型：

- security skill 按技术栈、CWE、测试类别、攻击链与 prerequisite 建索引；
- skill 按需召回，不把数千文件一次性塞入 prompt；
- baseline → candidate action → response diff 的确认思路；
- 方法论负责扩大假设与策略空间，最终 finding 仍需可重放 evidence。

参考资料：

- [CyberStrike Security Skills 概览](https://github.com/CyberStrikeus/CyberStrike/blob/80ee899a4ccb2a152fb505e7ce9e1a7874b1f486/README.md#security-skills)
- [CyberStrike skill index engine](https://github.com/CyberStrikeus/CyberStrike/blob/80ee899a4ccb2a152fb505e7ce9e1a7874b1f486/packages/cyberstrike/src/skill/index-engine.ts)
- [CyberStrike vulnerability confirmation prompt](https://github.com/CyberStrikeus/CyberStrike/blob/80ee899a4ccb2a152fb505e7ce9e1a7874b1f486/packages/cyberstrike/src/agent/prompt/vuln/common-prompt.txt)
- [CyberStrike AGPL-3.0-only license](https://github.com/CyberStrikeus/CyberStrike/blob/80ee899a4ccb2a152fb505e7ce9e1a7874b1f486/LICENSE)

### 6.2 Methodology Corpus contract

每个可检索来源先生成 `MethodologyCorpusManifestV1`：

```text
corpus_id
source_kind
upstream_url
upstream_revision
license_spdx
license_text_sha256
signature_state
trust_store_epoch
document_count
content_root_sha256
parser_contract_version
index_contract_version
ingested_at
superseded_at
```

`corpus_id` 与 `document_id` 由canonical source identity、upstream revision、relative path和content root/hash确定性派生，不能使用随机UUID造成同内容不可重放。manifest append-only；新revision只通过supersession link接管active head。signature/trust-store/key-revocation epoch、license policy与active/superseded状态均参与admission。content resolver拒绝absolute path、`..`、symlink/root escape与resolve-read间TOCTOU。

每个 skill/document 保存稳定 document identity、relative path、content hash、bounded metadata 与以下可选标签：

- product、technology、CPE/range；
- CWE、OWASP/WSTG、ATT&CK、CIS/NIST；
- auth、business logic、credential、configuration、exposure；
- prerequisites、controls、negative oracle、attack-chain relationships；
- required capability 与 risk class。

原文是 host常量 `instruction_authority=false` 的 untrusted data，而不是caller可写bool。frontmatter/body 中出现 system prompt、tool call、scope expansion、credential 或任意指令性文本都不能改变 Agent 角色、工具或授权。

### 6.3 检索流程

服务端从 `BaselineContextSnapshotV1` 构造 mandatory baseline `MethodologyQuerySetV1`，输入包括 product/version candidates、CPE、service/banner、routes、auth/trust boundary、Nuclei template/CWE、coverage gaps 与 current hypothesis components。dynamic query-planner subtasks只能针对S0提交有界补充 query intent；host处理并冻结全部结果后才seal最终`InvestigationAnalysisSnapshotV1`，后续analysis subtasks不能继续扩写query census。

补充 query 只能改变本地 corpus 的检索词，不能携带 target URL、cookie、credential、action payload 或 scope mutation。host 对 query normalize、dedupe、budget、ranker version 与 top-k 负责，并封存：

- query count/set hash；
- 每条 query 的 canonical tokens/tags；
- corpus manifest exact set；
- returned document exact set、rank 与 score micros；
- omitted/rejected query reason；
- result content hash 与 citation ref。

methodology hit 只允许进入 hypothesis source role `knowledge_signal` 或 campaign strategy source，不得进入 `proof_evidence_refs`、`refutation_evidence_refs`、Finding lineage 或 Gate fact。

### 6.4 本地缺失时的在线 enrichment

Analysis specialist 不能临时浏览并把网页 prose 直接交给 reducer。它只能提交 `MethodologyEnrichmentRequestV1`，包含产品/主题、需要回答的问题、允许的公开来源类别与理由，不包含目标 credential 或 raw evidence。

经 operation policy 允许后，独立 enrichment worker 从durable outbox claim请求，在事务外通过publisher allowlist与egress classification获取公开资料；target URL、cookie、credential、raw evidence与客户私有标识不能进入请求。结果必须先冻结 URL、fetch time、publisher、document version、content hash、license/provenance、classification 与 exact result set，再进入successor methodology/snapshot。网络失败、来源不明、页面变化或 license 不允许摄取时写 residual，不伪造空结果。worker使用lease/CAS/idempotency key恢复；DB事务内不fetch。

下载、批量 vendoring 或再分发 CyberStrike/第三方 corpus 前设置独立 license/provenance 暂停点。AGPL-3.0-only 的影响必须由项目 owner 明确决策；未获决策时只实现 provider/manifest/index contract 与许可的本地 fixture，不把外部仓库内容提交到 Golish。

## 7. Hypothesis Analysis

Analysis TaskWorker/Primary只读取sealed `InvestigationAnalysisSnapshotV1`。host封存的只是coverage/checklist axes与两类typed output contract，不是固定Agent席位：

1. proposal coverage axes：surface correlation、application/trust、auth/business logic、evidence/coverage；
2. conflict output class：duplicate、contradiction、identity mismatch、missing prerequisite 与 ignored gap。

Generator根据当前缺口把这些axes动态合并或拆成2–8个ordered subtasks；Primary再动态选择worker。`critic/conflict`是typed result/census语义，不要求固定Critic Agent、固定lane数量或一axis一worker。

每条 proposal 必须分别引用：

- canonical fact/evidence refs；
- AU refs；
- gap/residual refs；
- RAG/KG refs；
- methodology refs。

字段为空是合法状态，但不能把 RAG/methodology ref 填进 evidence ref。host 继续负责 semantic key、root/revision、verification plan、merge/split/derive、generation seal 和 Gate。

Analysis 阶段可写 `proposed / supported / contested / inconclusive`，不能写 `verified / refuted`。每个hypothesis分别记录两个正交轴：planning readiness沿用canonical闭集（包括`ready_for_strategy / needs_enrichment / deferred / unsafe`等既有值），capability assessment单独记录`known / partial / no_known_capability / stale`及adapter census。`no_known_capability`绝不是readiness值。

只要hypothesis达到`ready_for_strategy`且scope/policy允许，系统仍创建task；具体objective无adapter时由task objective assignment记录`no_known_capability` residual。`deferred`则在admission中得到独立typed disposition并进入closure/report residual，不会从exact set消失。

## 8. 自动验证调度与 hypothesis 查看

### 8.1 自动 admission

当前 analysis generation seal 后，host 构造 `VerificationAdmissionSetV1`。它从 current hypothesis exact set、sealed verification plans、readiness、scope、capability、policy、credential authority 与并发预算确定每条 revision 的唯一 disposition：

- `scheduled`；
- `needs_enrichment`；
- `deferred`；
- `out_of_scope`；
- `unsafe`；
- `already_terminal`；
- `no_new_obligation`。

所有 `ready_for_strategy` 且未被 scope/policy 阻断、尚有新的semantic attempt fingerprint的 revision 都必须自动进入 `scheduled`；adapter/capability缺失不能在此处丢掉task，而要在task objective denominator中显式记录。`no_new_obligation`只适用于同一revision/plan/relevant-evidence/open-obligation fingerprint已经terminal或exhausted且没有material delta的情况。并发上限只控制排队顺序，不能静默从admission set删除hypothesis。每条disposition保存reason、owner、next action、attempt fingerprint与exact input refs，header/member由host重算count/set hash后seal。

### 8.2 HypothesisVerificationTask

每个 `scheduled` member 由调度器自动创建或重放一个 `HypothesisVerificationTaskV1`：

- 绑定 exact operation、stage execution、stage run unit、organization、scope snapshot、current hypothesis revision/hash、sealed verification plan、relevant evidence snapshot、open obligation exact set与task contract version；
- 是 UI/调度 aggregate，不是 security verdict authority；
- immutable task header只记录`first_admission_generation_id`；每个后续generation通过append-only admission→task membership复用该task，generation不进入stable task identity；
- stable task key在全历史唯一，不只是“active唯一”。跨generation重放返回同一receipt；只有material input变化，或host签发包含reason、authority receipt hash与monotonic rerun contract version的rerun receipt，才能形成新的semantic attempt fingerprint与新stable key；
- relevant-evidence identity使用排除`read_at/epoch/timestamp`等非语义字段的semantic evidence-set hash；相同语义的新snapshot不得重复攻击，freshness失效则先block/revalidate而不是创建重复task；
- stale/superseded revision、change-seq drift、foreign operation/org、closed stage 或 unsafe readiness 均 fail closed。

每个task先在一个事务内创建或预留Campaign header，再seal immutable `TaskObjectiveAssignmentSetV1`。它必须逐一覆盖verification plan的objective exact set；每个member恰好是：

- `campaign(campaign_id)`；
- `already_satisfied`；
- `no_known_capability`；
- `needs_enrichment`；
- `out_of_scope`；
- `unsafe`；
- `blocked`。

`already_satisfied` assignment receipt绑定current objective adjudication与semantic evidence hash；其它non-Campaign residual assignment保存typed reason、owner、next action与residual receipt。这两类assignment receipt自身就是terminal，不创建伪outcome。只有`campaign` assignment在运行后写exact-one append-only `TaskObjectiveOutcomeV1`（completed/blocked/cancelled-before-start/recovery-required），且`outcome.campaign_id` exact set必须等于`assignment.campaign_id` exact set，不回写immutable assignment。scheduled task必须完整创建assignment denominator；零Campaign只有在每个objective都有显式pre-admission terminal assignment时才合法，并seal empty outcome set，而且这种task不能产生verified/refuted结论。漏项、重复项、未sealed assignment、Campaign缺outcome或出现non-Campaign outcome均阻止terminal/Gate。

这个task级assignment set是Plan C `hypothesis_revision + objective + required_control` verification-wave denominator的父级补充，不替代、合并或降低后者。一个Campaign assignment仍需在Campaign内部完整覆盖required controls。

task header immutable，状态由append-only `HypothesisVerificationTaskStateEventV1` 与CAS current head推进。closed state至少包括：

```text
admitted → queued → planning → running
                    ↘ awaiting_authorization
running/awaiting_authorization → consolidating → terminal
任何非terminal → stop_pending → draining → cancelled/blocked/recovery_required
```

durable generation-seal outbox、cold-start bootstrap/reaper与lease/fence scheduler共同保证：没有用户点击也会claim queued task；capacity释放自动claim下一条；crash、response loss或重复wakeup只重放同一task/Campaign receipt。event只负责刷新，不拥有mutation。

task admission自动启动planning与只读consults，但不等于批准任何HTTP、browser、CLI、credential、爆破、并发或写入动作。真实动作仍必须经过Prepared Action编译、risk policy、JIT、lease、budget、durable begin与typed oracle。

### 8.3 自动循环必须有总fuel

operation-frozen budget同时限制analysis generations、verification tasks、Campaigns、subtasks、nested delegations、consults、Prepared Actions、wall-clock、token/provider与高风险动作额度。budget不是内存计数：operation/unit/task各轴都有immutable budget head、append-only reservation state event与semantic-cycle receipt。admission、Refiner patch、nested delegation和Prepared Action创建必须在同一DB事务/fence中原子reserve对应fuel，失败即不创建work。只有在dispatch/durable begin之前被确定取消的reservation可refund；模型/provider request发出或Prepared Action durable begin时立即consume。response loss/unknown execution进入`unknown_held`，既不refund也不自动重放，直至typed recovery或人工settlement证明最终consume/refund。这样避免并发read-then-act超卖和未知副作用重试。host拒绝相同revision/plan/evidence/obligation/cycle fingerprint因新generation再次执行。任一fuel耗尽或unknown-held形成typed residual并停止相关admission，不能由点击、刷新或Agent prose补充fuel。

### 8.4 点击只改变查看焦点

点击hypothesis行只更新组件本地selection，并在同一个全屏工作区显示该hypothesis已经自动创建的TaskWorker/Primary、dynamic/nested specialists、subtasks、strategy、action、oracle与evidence timeline。点击不得调用Tauri mutation、创建task、改变调度优先级或批准action。

若 task 尚在 durable queue、Agent identity 尚未出现或正在等待 JIT，详情分别显示 queued、starting 或 authorization-required 的真实状态；前端不能为了立即有内容而伪造 Agent。

JIT Approve/Deny与“停止整个Investigation”是独立、明确命名且经服务端授权的控制，不属于hypothesis click。JIT只作用于exact Campaign/Prepared Action；它不会给Primary、dynamic或nested worker增加raw工具权限。所有cognitive worker始终只能提交typed strategy/action intent/result，真实HTTP/browser/CLI/credential/pentest I/O只由host编译后的one-action typed Operator执行。

stop intent只关闭新的admission并令queued work取消、running/unknown work进入drain/recovery；不得把durable-begin或unknown execution强写成cancelled。request绑定exact operation/stage execution/stage-run、`expected_investigation_run_state_head`、`expected_change_seq`与idempotency key；客户端不提交work列表。服务端在一个事务内CAS stage head、递增`stop_epoch`、关闭admission，并冻结open analysis/read sessions、query-planner、enrichment requests/outbox、verification tasks、PentAGI subtasks/worker requests、Campaigns、Prepared Actions、FactDelta/consolidation的全量exact set/hash。所有异步writer必须携带同一stop epoch/fence，stop后只能写cancel/drain/recovery terminal event，不能产生新work；各类集合闭合后才允许closure。

## 9. 验证期间出现的新想法如何显示

使用三条确定性规则：

| 验证中出现的内容 | 记录位置 | UI |
|---|---|---|
| 同一 claim 的新 payload、工具或顺序 | Campaign strategy/round | 当前 hypothesis 下的策略记录，不新建子假设 |
| 新 evidence 支持或反驳同一 claim | 同一 root 的 successor revision + successor snapshot | 原 hypothesis 显示新 revision |
| 新的、可独立证伪的安全 claim | `derive / split / independent root / merge` | 显示为关联或子 hypothesis，并在下一 sealed generation根据新attempt fingerprint自动进入admission |

Agent 只能提交 typed `new_claim_signal`。是否创建 child/root、沿用 root 或合并，由 Registry reducer 根据 semantic key 与 lineage 规则决定。material FactDelta必须先构造successor baseline/final snapshot；没有新evidence、plan或open obligation时只写fixed-point/no-new-obligation receipt，不打开空generation或重复task。UI 展示 public narration、artifact、evidence 与 typed decision，不展示模型私有 chain-of-thought。

## 10. 同一个全屏工作区

目标生产入口复用现有链路；当前已存在的部分截止 `ToolCallDetailView` / `StageRunDetailShell`，`InvestigationWorkspaceView` 是本计划待新增的view component：

```text
stage_run tool card
  → ToolCallSummary
  → detailViewMode = tool-detail
  → PaneLeaf
  → ToolCallDetailView
  → InvestigationWorkspaceView
```

不新增 `investigation-workspace` DetailViewMode，不新增第二个 Pane route，也不让 `ToolCallDetailView` 只显示跳转链接。

`InvestigationWorkspaceView` 只嵌在现有`tool-detail` route内，复用 `StageRunDetailShell`、Agent rail、transcript renderer、deep-link focus 与独立滚动。实现应把当前私有Actor rail/conversation primitives抽成共享presentational组件或支持typed navigation tree注入，不能嵌套第二层shell。建议结构：

```text
Left rail
├─ Main Agent
├─ Analysis specialists
└─ Hypotheses
   └─ Hypothesis
      └─ Verification task
         └─ Task Primary
            └─ dynamically delegated Pentester / Researcher / Browser /
               Coder / Installer / Enricher / Memorist / Adviser

Main surface
├─ selected Agent transcript
├─ selected hypothesis detail
├─ Campaign / action / oracle timeline
└─ evidence and methodology citations
```

deterministic tool/action/oracle 显示为 artifact/event，不能伪装成 Agent。read model返回host验证的`organization_id + hypothesis_revision_id + task_id + subtask_id + worker_run_id + owning_stage_run_request_id + transcript_request_id + parent_actor_transcript_request_id + parent_dispatch_tool_request_id + status`关联；Agent deep-link仍使用同一个owning `stage_run request_id`，transcript lookup精确使用`transcript_request_id`。

前端只持有discriminated查看selection（`agent | hypothesis | campaign`）与refresh sequence，不复制 canonical Registry/Campaign read model。ChatPanel specialist deep-link只应用一次，不得在refresh后抢走用户选择。所有read始终绑定`operation_id + stage_execution_id + stage_run_request_id`；首次summary/page bootstrap允许typed `expected_change_seq=None`，并在同一只读snapshot返回projection head/cutoff/epoch。此后detail、pagination与refresh必须带exact expected seq/cutoff/epoch，服务端拒绝任何mixed/zero/latest guess。

read model有界拆分：summary含stage identity、Main/Analysis topology、source census、counts及server-owned `InvestigationControlProjectionV1`；后者明确返回`stage_topology_contract + investigation_run_state + investigation_run_state_head + stop_epoch + stop_allowed/reason + reset/fork availability/reason + adoption_contract_version + control_policy_version`。hypothesis page含revision、admission/task摘要；selected hypothesis detail含其objective/Campaign/Agent topology；timeline独立分页。前端不得从`stageOrder/currentStage`、change seq或本地状态猜stop/reset/fork authority，也不得把全部hypothesis/Campaign塞入summary或做N+1 detail读取。

commit后的projection outbox bridge发出专用AI event `investigation_projection_changed`，只传 `operation_id + stage_execution_id + stage_run_request_id + change_seq` 作为refresh hint。`change_seq`必须取自同一已提交projection batch receipt/head，emitter不得在commit前猜值。Zustand保存monotonic `InvestigationRefreshHint`；duplicate/out-of-order忽略，gap触发bootstrap，foreign operation/request identity拒绝。missed event、cold restore或bridge重放都重新读取DB projection；event不是authority。

selected hypothesis/Campaign把exact `operation_id + campaign_id`交给复用的`PendingPreparedActionPanel`；rollout/policy不允许JIT时，由server projection隐藏或禁用，不由前端自判。hypothesis selection、JIT button与stage stop是三个不同DOM target和三条不同authority path。

unified Investigation运行中禁用developer reset/successor fork；正常终止只能调用`investigation_request_stop`。仅在run已经terminal/abandoned后，developer recovery菜单才可发起带topology adoption receipt的successor fork，文案按frozen topology生成，不能硬编码“运行到Attack Candidate”。legacy operation继续保留Candidate→Verification reset/fork行为；stop与reset/fork不得互相调用。

必须覆盖 loading、error、empty、stale、identity unavailable、zero hypothesis、analysis sealing、automatic admission pending、task queued、task accepted但Agent未出现、authorization required/denied/expired、blocked/recovery、verified/refuted/inconclusive 和 residual。空结果不得渲染成“安全”。

## 11. Investigation Gate 与人工停止点

新stage不因Analysis TaskWorker生成了hypothesis就自动PASS，也不因某个hypothesis没有runnable capability就把它标成refuted。

阶段状态包括：

- `analyzing`；
- `scheduling_verification_tasks`；
- `verifying`；
- `awaiting_authorization`；
- `consolidating_fact_delta`；
- `reanalyzing_material_delta`；
- `ready_to_close`；
- `blocked`。

调度器持续消费新的sealed generation，但只在semantic attempt fingerprint变化且frozen fuel已原子reserve时建立新admission set。用户可通过独立stage-level stop control提交幂等stop intent；系统以stage-level head CAS停止新admission、递增stop epoch并冻结全部stage-owned open-work exact set，再逐类cancel/drain/recover analysis、enrichment、task/worker、Campaign/action与FactDelta/consolidation，最后才写`InvestigationRunClosureV1`。closure对exact current hypothesis/task/objective/action set记录terminal、queued-cancelled、needs_enrichment、deferred、out_of_scope、unsafe、no-known-capability、fuel-exhausted或recovery-required。被停止或无法运行的hypothesis只能成为typed residual，不能成为checked-empty或refutation。

Gate PASS/PASS_WITH_GAPS 至少要求：

1. 当前 analysis generation 已 sealed，输入/方法论 query census 完整；
2. 没有active analysis/read session、query/enrichment、task/subtask/worker request、Campaign、Prepared Action、lease、unknown execution或未消费FactDelta/consolidation；stop时这些集合与stop epoch/fence完全闭合；
3. 每个已启动task已terminal、blocked-with-residual、drained cancellation或明确recovery-required，且delegation census证明每个runnable subtask都有独立worker；
4. current hypothesis exact set有sealed `VerificationAdmissionSetV1`，每个scheduled task都有完整sealed objective assignment set；outcome exact set只覆盖Campaign assignments且set hash完全相等，AlreadySatisfied/Residual由assignment receipt终结，zero-Campaign seal empty outcome set；
5. material FactDelta已生成successor snapshot/generation，或fixed-point/no-new-obligation receipt证明无语义变化；
6. task/Campaign/generation总fuel的reservation/consume/refund/unknown-held ledger和semantic-cycle census已封存，不能存在超卖、未settle unknown-held或可继续自动执行却被遗漏的work；
7. verified/refuted只来自revision-level deterministic adjudication；
8. gaps、deferred hypotheses、methodology/RAG omissions、unsupported capability与enrichment失败进入Reporting。

zero hypothesis 只有在完整 input/checklist/query census 与 generation seal存在时才可关闭；其语义是“本次有界分析没有形成可验证 hypothesis”，不是“目标无漏洞”。

## 12. 与 Plan B/C/D 的关系

### Plan B 保留

- Hypothesis Registry、semantic key、revision/lineage/generation；
- 2–8 条dynamic只读 analysis subtasks 与 proposal/conflict output census；
- verification plan/proof paths、Gate、rollout/outbox/read commands。

Plan B 改动：candidate snapshot 纳入 scoped ContextPack 与 MethodologyContextSnapshot；删除独立 `HypothesisRegistryAudit` 产品入口，改为同一全屏工作区中的 hypothesis projection。

更精确地说：Plan B Tasks 1–6 的Registry/identity/revision/generation/plan基础保留并扩展snapshot schema；Tasks 7–8 的Controller、2–8 dynamic subtasks与proposal/conflict census保留，但coverage axes不得绑定固定Agent lane，planned Candidate runner/prompt原位重命名并折入Investigation，禁止并存第二套runner；Task 9只保留generation seal/finalizer，顶层Candidate dispatcher/handoff被取代；Tasks 10–11 legacy projection、rollout与read commands保留并扩展admission/task/team projection；Task 12 UI完全由本文工作区替代。

### Plan C 保留

- objective-local Campaign；
- multi-agent strategy/conflict-output/refinement；
- Prepared Action/JIT、typed executor/oracle、FactDelta、revision adjudication。

Plan C 改动：删除 Candidate seal 后切换到第二个顶层 Verification stage 的语义；改为同一 Investigation stage 内由 sealed `VerificationAdmissionSetV1` 自动创建 `HypothesisVerificationTaskV1`。

Plan C Tasks 1–4及6–11的Campaign domain、objective membership、权限分类、compiler、JIT、executor/oracle与FactDelta内核继续有效；旧Task 5固定九角色与1–3 consult lane的dispatch步骤由本文PentAGI dynamic Task取代，只保留durable artifact/census原则。Task 12保留lease/drain/Gate/fixed-point机制，但删除独立Verification stage_run、Candidate→Verification continuation与回跳。automatic task admission只启动planning/consult，从不改变Prepared Action授权语义。

### Plan D 保留

- D1 versioned read model/API；
- D3 canonical Reporting；
- D4 rollout、comparison、promotion 与 historical policy。

Plan D 改动：D2 不创建独立 Pane route/store mode；其 Hypotheses/Campaigns/Waves/Timeline 子视图嵌入现有 `tool-detail` 全屏 stage workspace。unified topology把Candidate/Verification双roadmap入口改成一个Investigation入口；legacy topology仍保留原双入口。

D1扩展unified stage、admission/task与Agent topology；D2 Task 6/7 route/shell完全由现有full-pane入口取代，Task 8删除双roadmap与仅跳转链接，但保留legacy adapter、mode guard、Prepared Action controls、audit projection与monotonic refresh；D3/D4增加`stage_topology_contract`后继续实施。

## 13. 验收标准

1. unified graph只允许`vuln_triage → application_understanding → investigation`，新operation roadmap只有一个Investigation节点与一个exact `stage_run` identity；legacy graph不改写。
2. operation-frozen `stage_topology_contract`与五种rollout mode满足closed legal matrix，控制profile、fork、resume、history、reporting与rollout；既有operation不会被当前默认重解释。
3. 点击`Running specialist agents`进入现有全屏工作区，ChatPanel保持mounted；live/completed-restored/nested actor按exact transcript identity恢复，Main缺identity显示unavailable；commit后的`investigation_projection_changed`经production lifecycle bridge发出且只作monotonic refresh hint。
4. host先seal baseline snapshot，query-planner只提交typed intent，host再seal不可变final snapshot；Main Agent在任何analysis dispatch前创建per-org durable read session，raw transcript/context严格分区且协调层只见typed receipt，post-seal append与crash/resume跨organization混合失败。
5. RAG/KG使用现有scope-first ContextPack与Plan A/B checked authority；cross-operation/org、classification drift以及target/methodology prompt injection fail closed。
6. methodology corpus可按manifest/revision/hash/signature/trust epoch重放，skill不全量注入prompt；每个命中可追溯到exact document，path/symlink/TOCTOU逃逸被拒绝。
7. 本地缺失时只创建durable enrichment request；policy-approved worker在事务外抓取公开allowlisted来源，成功与失败都形成successor snapshot或residual。
8. 方法论/RAG命中只能创建`knowledge_signal`，不能单独形成Finding、proof、refutation或Gate PASS；stage-level generic Finding submit被拒绝。
9. Generator把host coverage axes动态合并/拆分为2–8条只读subtasks，形成、去重、冲突复核并seal canonical hypotheses；axes/critic不是固定Agent lane。
10. generation seal后，所有ready且policy允许且有新semantic obligation的hypotheses自动进入sealed admission set并创建幂等task；deferred、非ready、terminal与no-new-obligation项都有唯一typed disposition。
11. task identity绑定exact stage/unit/org/scope/revision/plan/semantic evidence/obligation/rerun receipt，不以generation制造新身份；全历史stable key唯一，same-semantic refresh、crash、response loss与duplicate wakeup重放同一receipt。
12. 每个task的sealed objective assignment set完整覆盖verification plan，Campaign header在同一事务预留；outcome exact set只覆盖Campaign assignments且集合相等，non-Campaign assignment receipt自身终态，zero-Campaign seal empty outcome set，Plan C required-control denominator仍完整。
13. scheduler在无UI/无点击时自动claim queue，capacity释放自动运行下一task；它只写一个task-run request，现有TaskOrchestrator是唯一cognitive dispatch writer，lease loss进入recovery且不重复Campaign/action/subtask。
14. verification team在同一hypothesis节点下显示真实PentAGI-style单一Task Primary→dynamic/nested specialist树并满足sealed delegation census；每个runnable subtask至少有一个独立worker，不能每subtask另造Primary或由Primary单Agent terminal。
15. task admission只启动planning/consult；所有cognitive worker只提交typed intent/result，任何真实动作仍由one-action typed Operator遵守Prepared Action/JIT/lease/budget；Approve/Deny复用exact Campaign panel且是独立明确控件。
16. 点击hypothesis只改变discriminated查看焦点，零Tauri mutation、零调度变化、零action approval；首次read以typed no-seq bootstrap取得head，后续绑定exact seq；stop/reset/fork只读server-owned control projection，Agent deep-link与refresh不抢焦点。
17. 新strategy留在Campaign；material evidence/new claim先形成successor snapshot与attempt fingerprint，再进入revision/generation/admission；无变化写fixed-point/no-new-obligation receipt。
18. operation-frozen generation/task/Campaign/subtask/delegation/action/time/token/provider fuel通过原子reservation state ledger与semantic-cycle guard阻止并发超卖和无限循环；durable begin后consume、unknown执行保持unknown-held且不自动重放，点击不能补fuel。
19. stage stop以Investigation run head CAS递增stop epoch并由服务端冻结analysis/read-session/query/enrichment/task/subtask/worker/Campaign/action/FactDelta全量open-work exact set；closure前各异步writer fence、task state、delegation census、objective assignment/outcome、FactDelta与residual全部闭合。
20. unified roadmap只有Investigation且active run的developer reset禁用；legacy Candidate→Verification roadmap、resume/history/evidence保持不变，stop与topology-aware successor fork不互调；zero hypothesis、unsupported adapter、online enrichment失败和RAG omission都显式呈现为bounded result或residual。

## 14. 非目标

- 本补遗不把 security skills 变成可执行脚本或自动授权来源。
- 不让 Agent 直接遍历 7,600+ 文件或把它们全部塞入上下文。
- 不以某个具体产品作为验收前提；产品特定假设来自 corpus 与 current facts 的通用检索合同。
- hypothesis 卡片的查看动作不绑定task创建、调度或真实网络副作用。
- 不修改、删除或重新解释历史 Candidate/Verification 数据。
- 不在数据库事务内执行网络、provider、embedding、browser、CLI 或 corpus 下载。
