# golish-agent-kit / db_traits

> **一句话职责**：DB 操作的 trait 抽象 + 本地模型类型——把 agent 层与 `golish-db`/`sqlx` 完全解耦：`DbRepoProvider`（CRUD）/ `DbTrackingBackend`（记录+记忆）/ `DbReadinessGate`（PG 就绪门）/ `TextEmbedder`（语义记忆嵌入），由 application 层注入实现。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/db_traits/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agent 层与 DB 的边界 trait（repo/tracking/readiness/embedder）或本地 DTO 时
- 排查 agent 层为何不直接依赖 golish-db/sqlx 时

## 职责

`investigation_asset_queue` 定义新 Investigation 的 SQL-free 公司→资产队列边界；运行时只能消费 host 冻结的顺序并用 exact CAS 推进，不能从 live targets 或模型输出自行重建队列。

该port的`load_current_evolution_authority`只接收operation/stage/scope/org/asset lane与expected evolution epoch；pending authority id由server从唯一open、same-lane source generation派生，caller/model没有nomination字段。genesis epoch不调用该seam，successor Analysis缺失/重复/foreign authority均fail closed。

`investigation_asset_verification` 定义 asset-bound 动态 Tool Manager 验证边界：DB 按 lane 选择下一条 canonical 未解决 hypothesis；每轮复用同一 Asset Primary durable chain，并允许 Primary 按需创建 0..N 个 asset-bound child，不冻结固定角色名单或角色数量；工具 inventory 与 0..N invocation 独立记账，业务终态只接受 Primary 基于已终态 Adviser review 提交的 `verified|refuted|invalid` resolution。mid-verification proposal 进入 typed pending-discovery backlog，消费前阻止资产 fixed-point。

定义 agent 层访问 DB 所需的 trait + 本地模型，使本 crate **不依赖 golish-db/sqlx**（依赖倒置）。application 层（golish-agent-app 的 db_bridge）提供具体实现。

## 公开接口

| 符号 | 说明 |
|---|---|
| `DbRepoProvider` | legacy/通用仓库操作与 deterministic harness truth seams（含 Candidate review/Verification truth/Wave consolidation/Reporting） |
| `RuntimeMemoryRepository` | typed runtime-memory 边界：project scope 注册/rename CAS、Task+operation/stage execution 原子创建与轮转、trusted submission/scope freeze、Unit/Worker seed/claim/prebound-chain/checkpoint/heartbeat/tool fence/terminal mutation，以及 Candidate Wave authority、Attempt terminalization、VerificationUnit close；错误保持 `RuntimeMemoryError`，不藏进 `anyhow` |
| `OperationStateView` / `StageForkCreate` | SQLx-free完整operation合同视图：runtime、Tool Truth、Investigation contract/mode三轴严格解析；stage fork可携带typed `OperationContractForkAdoption`，普通fork固定为`None`并由repo继承source pair |
| `HypothesisRegistryRepository` | Plan B唯一SQLx-free persistence port：snapshot/page/chunk、artifact、H1/H2 census/subreview/synthesis/reducer、Gate material与atomic apply；请求仅携带server-owned fence |
| `InvestigationAssetVerificationRepository` | 动态验证的 SQL-free authority port：server-derived canonical candidate、session authorization/budget、五角色 chain continuity、Tool Manager inventory、0..N invocation guard、terminal Adviser review + live Primary resolution，以及 pending discovery consume |
| `UnifiedInvestigationRepository::ensure_dynamic_asset_analysis_work` | server-owned Analysis cutover seam：同一事务验证从未执行的 fixed-roster v1 authority、追加 cutover receipt、封存旧 work 并创建 exact lane-bound dynamic work；stable request 漂移 fail closed |
| `DbTrackingBackend` | fire-and-forget 记录 + memory 存/搜 |
| `DbReadinessGate` | PG 启动就绪门 |
| `TextEmbedder` | 语义记忆文本嵌入 |
| `types` / `memory` / `repo` / `tracking`（本地 DTO） | trait + 本地模型；`StageAssetWaveView` 携带 durable wave 的对齐 `target_ids + asset_values`；`TechniqueOutcomeFact` 保留 asset/technique/outcome/evidence_id **及 source**，让 submit/final gate 能验证 trusted producer |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `DbReadinessGate` + re-export |
| `repo.rs` / `tracking.rs` / `memory.rs` / `types.rs` | 各 trait + 本地 DTO |

## 依赖

- `async-trait`、`uuid`；**无 golish-db/sqlx**（这是本模块的全部意义）

## 注意事项 / 坑

- **依赖倒置核心**：本 crate 定义 trait，golish-agent-app 注入实现——别在此引 golish-db/sqlx（会破坏 agent 栈的可测/解耦）。
- `RuntimeMemoryContract` 只有 `LegacyV1 → DualWriteLegacyRead → DualWriteV2Preferred → V2Only` 四个单调状态；operation 创建时从 DB singleton 冻结，request/model/CLI 参数不得自行选择。`OperationStateView.project_scope_id` 对新 runtime operation 必须为 `Some`，legacy V1 row 可为 `None`。
- `OperationStateView`还必须完整保留Tool Truth + Investigation contract/mode；bridge遇到未知值或七态外组合必须fail closed。fork adoption只接受server/trusted配置中的typed target与expected hashes，模型文本不能构造或改变。
- Worker runtime mutation 只能走 compound DTO/API；`RuntimeWorkerFence` 必须同时携带 operation/execution/unit/worker/token/epoch/checkpoint-version。`DualWriteV2Preferred` 的 `LoadedWorkerCheckpoint` 每次选择一条完整 V2 记录或一条完整 legacy fallback，禁止跨源拼字段。
- 本地 DTO 与 golish-db 的 row 类型是两套，由 bridge 转换。
- harness gate 相关读写也走 `DbRepoProvider` seam：`technique_outcome_facts` 必须返回保留 `source` 的 `TechniqueOutcomeFact`，不能退回丢 provenance 的四元组；Enumeration `blocked` 的 submit/final gate 据此只接受 preflight→四轴、route recovery→DIR、browser recovery→JS/JSAPI/PARAM 的 source/axis 组合，并要求匹配 current-target guarded evidence。audit `kind` 的精确校验由 app bridge 在投影前完成，不能指望 kit trait 自行查询 DB。`source_query_facts` 投影 `source_query_log` terminal rows，但只证明 source 尝试、不证明 found。legacy 名称 `mark_target_intel_dns_empty_outcomes` 是 target_intel DNS attempt 的 app-side hook：runtime 拿到真实 evidence id 后调用，trait 默认 no-op，生产实现分别写 `technique_outcomes(GOLISH-INTEL-DNS, empty|error)`；只有明确 no-record 才 empty，resolver/transport failure 必须是非终态 error。
- `query_target_data_for_operation_origin` 是 operation-bound endpoint planning seam：可选 `exact_origin` 只能选择 current work item 对应的 immutable manifest，不能扩大 scope 或授权网络。默认实现回落旧 operation-aware read 以兼容测试 double；production app bridge 必须保留 selector并在 wrapper send 前再次全等重验。
- Scoping 的 trusted target seam 是 `scoping_target_snapshot(org)`：app 实现只返回
  current-org `scope=in` 且 source 属于 manual/imported/stage-run-seed/seed/cli 的可支持类型。
  `parse_scope_review_tool_result` 必须解外层 ToolResult 及 `response` 内层 JSON array，skip/
  free text/畸形返回均不是批准。orchestrator 按 canonical value + type + scope 精确对齐，
  snapshot 读失败 fail closed；这条 seam 只读，绝不把 review proposal 写成 target。
- `scoping_actions_for_session(session, org, not_before)` 的 `org` 是 gate 已解析出的 trusted root，app/repo 投影不得降级成 session-wide 布尔值。`ScopingActionsSeen` 区分 parent-only exclusion、成功 proposal、成功且在 proposal 后完成的 unit review，以及 target review attempts；error/skip/另一 org/乱序都不能置为成功。
- EAS gate 的 ledger seam 是 `eas_evidence_facts_for_session_org_fresh(session, org, since)`：默认空且绝不 fallback 到 session-wide facts；app 实现负责 producer org、current target owner/project/scope、freshness 与 asset/technique raw witness 校验。
- EAS exact-origin seam 是 `eas_required_web_origins(org, since, current_wave_target_ids)`：返回本轮 fresh、current-owner、project 精确匹配且仍由 target 当前 URL/开放端口/明确 HTTP service 授权的 canonical origins；调用方明确传入空 wave membership 时必须保留 authoritative empty，读取失败必须让 preview/final gate fail closed。
- wave-aware stage 的 durable batch 也走 `DbRepoProvider` seam：`stage_asset_wave_current_or_create_initial` / `stage_asset_wave_create_next` / `stage_asset_wave_complete` 默认 no-op/None，app bridge 才接到 `golish-db::repo::stage_asset_waves`。最终 close 必须调用 `stage_asset_wave_create_next_or_seal_completion`，让“queue 下一波”与“原子发布 org completion 水位”成为互斥结果；不能用普通 create-next 后再单独写 completion。coverage snapshot seam 同时接 current wave ids/values，不能只传 value；present-invalid wave 与 `None` 必须保持可区分。
- completion 的 operation-bound 读取走 `org_stage_completions_get_with_run_id`；app bridge 必须保留 DB 行的 `stage_run_id`，stage_run/orchestrator 再与 current operation UUID 精确比较。默认 trait 把 legacy projection 映成 `stage_run_id=None`，因此 operation-bound caller 会 fail closed，不能只凭 fresh `passed_at` 接受 sibling operation 的 PASS。
- `stage_asset_coverage_for_operation` 是 operation-aware coverage seam：默认实现只为兼容测试 provider 回落旧 `stage_asset_coverage`，生产 app bridge必须把 trusted operation id 传给 snapshot。Enumeration 的 EAS transport handoff 只允许在这条 seam 下缩小 exact-origin denominator；tool executor、submit preview、task close 和 final org gate漏传 operation id 时必须 fail closed 为未排除，而不是读取全局/最新 marker。
- `RuntimeMemoryRepository::finalize_unit_pass` 是 post-Scoping 非 wave Unit 的唯一 PASS seam，接收 server-built `FinalizeUnitPass`（Candidate 时携带 hash-bound `candidate_acceptance`）并返回 Unit/Worker/immutable handoff 及 `replayed`；普通 `finish_worker_attempt` 只能提交非 PASS 终态。wave-aware V2 必须走 `close_wave_gate_pass`，把 exact wave completion 与“child wave + WaitingBackground”或 final seal 作为一个事务结果返回。`load_inherited_stage_handoffs` 按 exact operation/org/source-stage 读取 final-sealed handoff，供 `StageSpec.inherits_evidence_from` 下游注入，不允许 latest/global fallback。
- `park_stage_team_finalizer_after_failure` 只处理 durable submission 已存在、request epoch 已关闭、barrier exact ready 的 Company Controller finalizer。DTO 同时携带 exact lease/attempt/checkpoint fence 与 submission/manifest；实现应回排同一 WorkerRun 而非生成 sibling、修改 submission 或消耗 producer attempt fuel。默认实现 unavailable，测试 double 不会意外放宽生产恢复。
- `adopt_legacy_vuln_terminal_outcomes` 是已发生 no-purge replacement 的窄兼容 seam：输入只含 server-owned Controller fence，返回 adopted/no-op 与计数；production bridge必须在 DB 内验证 source/replacement Unit、submission、org/scope/manifest/evidence lineage并恢复统一 freshness epoch。默认测试实现为 no-op，runtime仍必须重新读取 coverage，不能信返回值直接 PASS。
- `DbRepoProvider::attack_v2_seed_candidate_manifest_for_unit` 只在 `attack_candidate` stage entry 使用：从 exact final-sealed `vuln_triage` authority 物化并冻结 manifest；`attack_v2_candidate_manifest_for_unit` 是 submit/final-seal 的 exact read seam。默认实现统一返回 `ATTACK_V2_REPO_UNAVAILABLE`，缺 production repo 时必须在 provider dispatch 前 fail closed，不能把 unavailable 当 empty。
- `RuntimeMemoryRepository::claim_candidate_attempt` / `heartbeat_candidate_attempt` 是 Candidate verifier 的 compound seam：claim 返回 opaque Attempt ref + exact WorkerRun/message chain，heartbeat 同事务续 WorkerRun+global lane；execution plan/canonical args/budget 不离开 DB authorizer。
- `RuntimeMemoryRepository::attack_v2_wave_authority_for_operation` 只返回 SQLx-free 的 `AttackV2WaveAuthorityView::{Initial,Current,Terminal}`；Unit 同时携带 typed entry（`VulnTriageHandoff|FactDeltaConsolidation`）与 state（`AwaitingManifest|FrozenManifest|TerminalNoInput`）。缺实现/identity 漂移必须 fail closed；runtime 不得用模型 org、静态 graph wave counter 或 latest row 猜 generation。
- `RuntimeMemoryRepository::seal_exhausted_vuln_residual_outcomes` 分别携带 Nuclei 与 anonymous-access 的 server maximum attempt；它只返回sealed/found/blocked计数，不把DB evidence body暴露给runtime。anonymous residual仍是`blocked`，绝不映射为checked-empty。
- `SeedStageRuntime.organization_ids` 是 server-owned frozen-scope 子集；Candidate follow-on 用它排除 `TerminalNoInput` org。空/重复/越界子集应由 production repo 拒绝，模型不可传该字段。
- `RuntimeMemoryRepository::terminalize_candidate_attempt` 返回 immutable lineage ids、terminal status、evidence/FactDelta counts 与 replay flag；`close_attack_v2_verification_unit` 只有在该 Unit 的 Candidate queue 排空且 DB terminal truth 成立后返回 `consolidation_status=ready`。两者都不把 plan、lease 或 exploit payload暴露给 trace/UI。
- `DbRepoProvider::attack_v2_review_barrier_for_operation` 返回 exact current Wave 的 durable counts/status/resume version；默认实现返回 `ATTACK_V2_REVIEW_REPO_UNAVAILABLE`。kit 只能据此 hold/allow stage routing，不能用 trace、前端状态或进程内 bool 替代 DB authority。
- `DbRepoProvider::attack_v2_consolidate_wave` 接收仅含 operation/snapshot/source-wave 的 server command；production bridge 必须在短事务提交后才返回 `AttackV2WaveConsolidationView`。返回值只含 immutable ids、`opened_next_wave|closed_no_delta|exhausted`、聚合 counts 与 replay flag；missing 实现统一 fail closed，绝不回退 process-local `chain_wave`。
- `DbRepoProvider::reporting_build_validated_revision` 是 Reporting stage-entry 的 server-owned build/reuse seam；`reporting_gate_truth` 是 close/submit preview 的 fresh read seam。两者默认返回 `REPORTING_TRUTH_REPO_UNAVAILABLE`，不能把 missing/error 当成空报告 PASS；类型只暴露 narrow `ReportingGateTruth`，不暴露 artifact/finalizer。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit db_traits
```

## Tool Truth Plan A DB seam（2026-07-30）

- DB traits只传 server-owned operation/org/stage authority、sealed denominator/receipt projection与 shadow assessment；模型/CLI request不能选择 rollout contract或提交 manifest count/hash。
- operation contract默认 `legacy_v1` 并在创建时冻结；`shadow_v1`只写审计 assessment，`receipt_v1`才允许 canonical receipt读取成为 coverage authority。
- denominator seal必须发生在 worker/provider dispatch前；缺 repo、ambiguous root、epoch drift或任何读取错误都 fail closed，不回退 latest session/global truth。

## Hypothesis Registry Plan B port（2026-07-30）

- `HypothesisRegistryRepository` 是 Candidate runtime 唯一 persistence port，覆盖 snapshot freeze/page、artifact、H1/H2 coverage census/reducer、final-submitter-only host compilation seal、Gate material与 atomic apply。
- request DTO 只携带 server-owned operation/scope/org/team/worker/attempt/version fence；没有 bundle root清单、fresh token、caller timestamp、feed signer或任意 canonical JSON write口。
- production首次Gate使用app-owned opaque snapshot source从DB `load_candidate_pre_gate_material_on`读取同一RR exact closure；pure Gate后才调用`apply_candidate_gate_pass`，后者在DB apply transaction内创建compiler seal并原子写canonical/outbox。`seal_candidate_compilation` / `load_candidate_gate_material`仍是typed compatibility/post-seal API，不代表production pre-seal顺序。
- unified Investigation 的 `investigation_analysis_host` 只让模型提交 typed hypothesis proposal与不含 URL/参数/credential/command 的 analysis advisory intent；canonical route/revision/claim component/contract/plan/generation/task identity 均不在 cognitive envelope。host 先绑定 exact Candidate snapshot/attempt，再执行 server compiler + atomic admission。冻结前置若来自 stage fork且已 stale，`StalePrerequisite`保留source operation、earliest required stage和exact retry command；same-operation stale则用`RevalidationRequired`保留obligation ids，不能混成fork建议。旧VerificationTask prepared subject、fixed-four capability strategy/action intent、Campaign advisory/finalizer DTO与trait methods已物理删除；dynamic asset Verification不复用这些ABI。
- semantic summary必须是raw closure的exact typed投影，不能只信hash：input/checklist/proposal/missed refs、blocker codes和bounded observations均由repository重验。canonical mutation transport显式携带generation transition/mutation hash；旧Campaign/Prepared Action repository ABI已删除，不能再作为Plan B或dynamic Verification的隐藏执行port。

## Historical Verification Campaign cutover（Plan C，2026-08-14）

- `db_traits/verification_campaign.rs`、`VerificationCampaignRepository`/shadow port及`DbRepoProvider` resolver已物理删除；历史Campaign schema/report reader保留，但没有runnable repository capability。
- fixed-four `DbRepoProvider::admit_candidate_generation_campaigns` compound ABI与返回DTO已物理删除；新 Investigation 不再通过该host seam从Candidate generation预建Campaign集合。
- 历史 `VerificationCampaignSchedulerView`、pending-evolution scheduler projection、consult DTO与三个 `drive/prepare/record` default port已物理删除；kit不再为已删除的Campaign scheduler保留一个永远返回unavailable的伪入口。asset business gate只认canonical current head resolution与unconsumed pending discovery。
- `InvestigationAnalysisHostRepository` 不再暴露 `prepare/apply/resume VerificationTask advisory` 或 `finalize_from_campaign_truth`；相关fixed VerificationTask subject/capability/advisory/finalizer DTO已删除。compiler admission view只暴露dynamic VerificationTask IDs，不再暴露Campaign reservation IDs或预分配objective count。
- `InvestigationGenerationAdmissionView.evolution_fixed_point` 区分“新 generation 已封存/准入”与“pending evolution 经 Analysis 后无新 semantic revision，source generation 已固定点关闭”；后一种 view 的 generation 字段只是 source authority且task必须为空，runtime不得把历史 admission 当作新验证工作。
- `RuntimeMemoryRepository::ensure_investigation_evolution_analysis_primary` 接受 exact pending-evolution authority、StageTeam identity 与由数据库统一函数派生的 subject fingerprint，返回 deterministic successor Analysis Primary、复制后的 durable chain 和 exact replay 标志；runtime cursor 以 `pending_evolution_authority_id` 区分普通 Analysis 与 Evolution Analysis，但不暴露 FactDelta 正文。
- authoritative scheduler不允许模型授权Prepared Action或提供URL、credentials、private manifest、budget、policy与Oracle verdict；这些authority始终保留在app/DB边界。
- `DbRepoProvider::wiki_get_page` 是 knowledge-base search/read 的 SQLx-free exact-page seam：provider 必须按同一 normalized logical path 返回索引中的 page；provider 缺失、读取失败或 exact row 不存在时，executor只允许尝试受 path-traversal guard 保护的 bundled-filesystem fallback，不能把 search 命中的逻辑路径改写成任意本地路径，也不能借此读取 evidence ledger。

## Target Intel semantic receipt seam（2026-08-02）

- `DbRepoProvider::{semantic_intel_receipt_append, semantic_intel_terminal_receipt}` 是 additive fixture seam；identity 必须同时绑定 exact operation/org/session/stable key。
- duplicate/frontier read contract中没有 `expansion_queue`；默认实现 fail closed/no terminal，production app bridge使用现有 `audit_log` 加 immutable redacted-artifact table。
- unified Investigation 的历史 PreparedAction assignment/send runnable port（list/materialize/claim/heartbeat/checkpoint/terminal/recover/execute/load）已物理删除；`DbRepoProvider` 与 `UnifiedInvestigationRepository` 不再暴露该ABI。历史assignment/Campaign表只供审计与report读取，新run必须使用lane-bound dynamic Verification session、Tool Manager invocation与Primary resolution authority。
- `RuntimeMemoryRepository` exposes production review freeze/read/verdict/finalizer calls with
  exact operation/org/unit/controller/reviewer/attempt/row-version/hash fences. Default
  implementations remain unavailable; the app bridge is the only SQL-backed composition.

## Investigation nested dispatch / task Primary rearm（2026-08-03）

- `InvestigationNestedDispatchRepository` 提供 SQL-free 的 nested cognitive begin/finish compound port；nested success 只能返回 `investigation_cognitive_output.v1` advisory object，不能携带事实、evidence、checked-empty、blocker 或 evidence watermark，nested failure 则必须是 typed blocked output。
- `RuntimeMemoryRepository::rearm_investigation_task_primary` 接受 previous Primary exact fence、plan/item row version、VerificationTask id 与 server-derived subject fingerprint；production 必须返回该 task 独立的 WorkItem/WorkerRun/message chain，并把 response-loss exact replay 与同 epoch 不同 task 冲突分开处理。缺实现默认 unavailable，不能回退进程内计划或复用旧 Primary identity。

## Unified Investigation retained-child read ports（2026-08-11）

- `RuntimeMemoryRepository::load_stage_work_item` 按 durable WorkItem id 读取并保留 operation、StageExecution、Unit、scope、organization 与 TeamPlan owner；默认实现 unavailable，调用方不得从 static seed 或 latest item 猜 dynamic child。
- unified Investigation repository还提供 exact logical-dispatch 与 latest sealed Refiner patch读取。DTO只暴露恢复所需identity/hash，不把SQLx row或可变模型上下文带入runtime。
- parked Analysis Primary只能在production repository已证明唯一pending child后作为read-only planning binding返回；它没有伪造lease，也不获得执行型工具authority。

## Zero-hypothesis analysis advisory（2026-08-12）

Analysis reducer允许`proposal_count=0`，但只在action intents同为0且exact typed residual为`no_bounded_hypothesis / sealed_input_did_not_support_a_proof_bound_hypothesis`时成立；非零proposal不得夹带该residual。该状态表示已检查的sealed输入没有形成proof-bound假设，不表示资产安全。

## Asset-bound Investigation scheduling（2026-08-13）

- `RuntimeMemoryRepository::ensure_investigation_asset_primary_schedule` 只返回一个持续 Asset Primary、其 WorkerRun 与跨round复用的message chain；不会预建role WorkItem或固定barrier。Primary后续只能经`request_stage_worker`请求0..N个server asset-bound child。
- migration-forward会把同lane/epoch旧`fixed_roster_v1` receipt保留为只读审计，并在旧epoch无live lease/tool时原子supersede/detach旧调度实体，建立fresh `primary_dynamic_v2` round；新runtime不得回退旧fixed roster。
- 当dynamic Primary的当前execution shell耗尽且exact blocked exhaustion output落库后，`ensure_investigation_asset_primary_schedule`沿同一root schedule追加receipt-backed continuation；view返回当前`execution_rearm_receipt_id`/`execution_ordinal`，每代fresh WorkItem/Worker复用同一asset message chain，Worker generation逐代递增。服务端最多允许32个continuation，超额返回typed fuel error；历史receipt、shell和exhaustion output均不可覆盖。
- unified Investigation Refiner新增dynamic v2 append/seal DTO：ordered active set可add/drop/retry/reorder，zero generator合法；exact replay不能新增ledger/patch/member/seal/event。v1历史行只读审计，不是新runtime fallback。
- unified Investigation Generator恢复提供三条SQL-free portable ABI：`load_pending_generator_recovery`按`created_at,tool_call_id`投影exact Primary lineage上的finished `submit_result`与现存subtask census；`materialize_generator`把chosen source、0..N subtasks、Generator event、Refiner ledger和source receipt同事务提交；`adopt_orphan_generator`只为已持久化但无ledger的exact census追加不可变adoption receipt。写API必须携带当前execution head的WorkItem/Worker/lease/attempt/checkpoint fence；模型不能提供provider call、canonical result或source hash。

## Investigation company / asset queue port（2026-08-13）

- `InvestigationAssetQueueRepository` 是 SQLx-free 的 strict company→asset queue port。freeze request 只含 unified stage identity、stable request 与 server policy fuel；公司 census 必须由 repository 从 sealed `operation_org_scope_units` 按 `depth,ordinal,organization_id` 派生，资产 census 必须从同 frozen project/company 的 in-scope Targets 按 `created_at,value,id` 派生，调用方不能提交成员。
- company/asset claim、asset transition、company complete 全部携带 exact owner tuple、expected member/lane version 与 queue head version；每个 `InvestigationCompanyQueueMemberView` / `InvestigationAssetLaneView` 同时返回所属 queue 的 `company_queue_head_version` / `asset_queue_head_version`，runtime无需从member/lane version或事件数猜CAS head。response-loss 只接受 stable request 的全字段 exact replay；foreign、out-of-order、stale CAS 都不得留下事件。
- `LoadInvestigationAssetBacklog` 返回 server 派生的单 lane审计 census及 `dynamically_resolved_root_count`。business gate只把具备 exact dynamic-v2 round/resolution/terminal-transition/state-event/current-terminal-head 链的 `verified|refuted|invalid` root视为已解决；旧 adjudication/server-validator 的closed head仍留在backlog。尚未消费的pending discovery独立阻断；Campaign/Wave/tool/actor/pending evolution均不参与gate。零hypothesis lane仍必须持有独立zero receipt。
- 最后一条asset progression返回server-authored resolution closure；每个typed member同时携带`organization_id`、`stage_run_unit_id`、`stage_team_plan_id`与统一`passed_at`，使外层coordinator能只依赖新queue authority恢复pass token，并可审计company→unit→plan的真实终态化。
- `seal_zero_hypothesis_fixed_point` 是独立 authority seam；caller只提供exact owner tuple与queue/lane CAS，DB在同一事务自行选择该lane最新zero-proposal compilation、zero-member generation seal与zero-revision Investigation canonical apply receipt，并封存server-derived empty-set hashes，内部authority ID/哈希不由AI或runtime传入。evolution fuel exhaustion返回保留exact `asset_lane_id` 的typed error。
- 最后一个asset的 `close_backlog_and_advance` 返回 `InvestigationResolutionClosurePublicationView`：publication与逐company `(organization_id,passed_at)` member完全由server生成；`exact_completion_authority(expected_operation_id)` 在 SQL-free 边界拒绝 foreign/empty/duplicate org、Unit 或TeamPlan投影，再按organization稳定排序供coordinator pass token使用。非最终推进不得返回closure；exact replay必须返回同一publication/member authority。
