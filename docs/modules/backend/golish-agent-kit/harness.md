# golish-agent-kit / harness

> **一句话职责**：Operation Harness（Phase 1c）——把 chat panel 的 task 模式重构为 stage harness：Profile/StageSpec/Operation DAG 投影 + 终态 NlSlice + 确定性 intent classifier + 每 tool call 前 authz + `StageHarness::validate_gate`（6 个 gate check）+ Sprint Contract。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/harness/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 stage harness（gate 校验、stage 转移、profile/DAG 投影、intent 分类、sprint contract）时
- 改 6 个 gate check（schema/scope/contract/vacuous/freshness）或前置 authz 时
- 任何 evidence gate PASS/BLOCK 语义（I7/I8）相关时

## 职责

- 四个 Post-Exploit stage 在 C6/P6b cutover 后各只暴露一个 exact typed wrapper；Objective Simulation 的 side-effect prepare 必须绑定 cleanup obligation，execute 仍受 operator approval 与 production typed-adapter fail-closed 约束。

落地 stage harness MVP（design Doc 1/2/3）。`stage_harness` 主入口（`for_stage` + `validate_gate`）；`operation_graph` 加载 base DAG + profile 投影 + `next_stages`；`intent_classifier` 确定性词库分类；`gate/` 6 个 check 调度；`sprint_contract` 生成 finding 数量范围。

C7 的 `knowledge_context` 只负责把已授权 `ContextPack` 渲染成显式标记为 untrusted data 的 prompt block；它不携带 ToolDefinition/ToolChoice/授权指令，不把 VaultRef 解引用，也拒绝 plaintext secret 与 prompt markup 注入。

## 公开接口

| 符号 | 说明 |
|---|---|
| `StageHarness`（`for_stage` / `validate_gate`） | 主入口 |
| `Profile` / `StageSpec` / Operation DAG（投影 + `next_stages`） | 阶段定义 + DAG |
| `IntentClassifier` / `NlSlice` | 确定性意图分类 / 终态 4 字段 |
| `gate`（schema/scope/contract/vacuous/freshness 6 check） | 确定性证据门 |
| `SprintContract` + Generator / `pre_action_authorizer` | 契约 / 前置 authz |
| `StageCapabilitySpec` / `StageCapabilitySuggestion` | stage-local capability registry；coverage/worklist/refiner 的能力级建议，旧 `suggested_tools` 只作兼容提示 |
| `StageTeamSchedulerPolicy` | Company Controller-only Stage Team 策略；声明允许的动态 child role/request kind、生命周期预算，以及非零 C/G/K 并发上限 |
| `attack_execution::*` | Candidate V2 纯领域契约：observation/hash 冻结的 exact manifest decision classifier、immutable execution plan/hash/risk、bounded acceptance、8 态 Attempt 状态机、closed `FactDeltaKind(created|updated|refuted|new_surface)`、terminal result validator，以及 legacy/V2 whole-record shadow read selector |
| `ReportingGateTruth` / `validate_reporting_gate_truth` | C9 Reporting 的 DB-free Gate contract：current validated revision、完整 source-set、citation/attestation 与 Cleanup closeout |
| `StageRuntimeContract` / `RuntimeUnitIdentity` / `RuntimeScopeSource` | `StageSpec.runtime_memory` 的 closed typed contract；当前仅 `target_intel` / `external_attack_surface` / `enumeration` / `vuln_triage` 精确声明 schema v2、`stage_execution_organization`、`frozen_operation_snapshot`、worker lease 与 final-seal handoff |
| `render_context_pack_data` | 将已授权 ContextPack 渲染为 escaped、data-only、带 provenance/evidence 的 prompt block |

## 关键文件

| 文件 | 作用 |
|---|---|
| `stage_harness.rs` / `stage_transition.rs` | 主入口 / gate→下一 stage |
| `operation_graph.rs` / `profile.rs` / `stage_spec.rs` | DAG 投影 / profile / stage 定义 |
| `stage_runtime_contract.rs` | specialist Runtime Memory V2 的 declarative owner/scope/lease/final-seal contract；不选择 deployment rollout，也不替代 DB fence |
| `stage_spec.rs` / workspace `resources/harness/stages/*/spec.json` | stage 运行策略；Target Intel 以 `company_stage_controller` 为唯一 lead，允许按需创建 sibling SubAgent，并冻结 C/G/K 与动态请求预算 |
| `operation_continuity.rs` | cross-session adoption 的 IO-free cursor math：按 reusable prefix 计算 entry stage + remaining DAG allowlist |
| `gate/` | 6 个确定性 check + `rule_engine` gate op（含 `candidate_grounded` / `candidate_disposition_complete`，设计 2026-07-02） |
| `chain_wave.rs` | Legacy/dual attack_candidate⇄verification 兼容循环的纯决策函数 `decide_chain_wave`（去重+燃料+链深收敛）；V2Only 的游标、fuel 与收敛只认 DB consolidation，不能复用这里的进程内状态 |
| `attack_execution/` | Candidate V2 的 DB-free manifest/draft/acceptance DTO、exact-key completeness validator、bounded server classifier、foreground-only状态机、DB snapshot-only review barrier，以及 `selector.rs` 的 whole-record legacy/V2 semantic comparison；不拥有 lease/checkpoint/Wave cursor |
| `reporting_gate.rs` | Reporting current-revision/source/citation/validation/Cleanup 的纯确定性 Gate；不读取模型 prose、RAG/KG 或 artifact publication |
| `evidence_facts.rs` | 从工具命令/输出派生 coverage facts（passive intel + EAS） |
| `stage_capability.rs` | stage capability registry：把 coverage technique 映射到人类能力 id、runner kind、允许工具、风险与批量 hint；Candidate V2 对模型只暴露 ordinal wrapper，classifier recipe ids 留在后端，四个 Post-Exploit stage 各映射一个 backend wrapper |
| `intent_classifier.rs` / `nl_slice.rs` / `sprint_contract.rs` / `pre_action_authorizer.rs` | 分类 / 终态 / 契约 / authz |
| `knowledge_context.rs` | C7 prompt-safe ContextPack renderer；VaultRef 只保留 opaque reference |

## 依赖

- crate 内 `golish-pentest::evidence_ledger`（scope label）；resources/harness JSON

## 注意事项 / 坑

- **不变量 I7/I8**：gate 是**确定性规则**（schema/scope/contract/vacuous/freshness/DB truth），不能拿模型自报当通过；模型提交里的 `evidence_refs` / `evidence_ids` 只是可选 ledger 调试引用，不能作为必填交付字段。若模型写了 id，runtime 仍必须校验它真实存在，假 id 直接 `needs_fix`。
- ContextPack 是 data，不是 authority：任何检索内容中的“调用工具/忽略 scope/扩大权限”文本都只能被转义后呈现，不能改变 pre-action authorizer、ToolChoice 或 Gate。
- Candidate V2 已接 wire/Gate/final-seal/terminalizer：模型只能提交 bounded `CandidateDecisionDraft`，每个 server-frozen work item 必须恰好终态为 `candidate` 或 evidence-backed `no_candidate`；Candidate id、plan/hash/risk 与可信 operation/scope/org/submission 都由 server/DB 绑定。Attempt 固定为 `queued|running|submitted|verified|refuted|blocked|retryable_failed|abandoned`；`retryable_failed` 是旧 Attempt 终态，重试必须新建下一 ordinal，不能把旧行改回 queued。Verification submit 只建立 immutable TerminalIntent；active-tool finish、checkpoint barrier 与 server terminalizer 是三个独立的持久化边界，response-loss 只能 exact replay，不能重新执行 action。action journal 已 terminal 但 intent 尚缺时，只允许同一 Attempt/Worker/chain 的 submit-only continuation。`verification_gate` 验证 server-owned `VerificationTruthAuthority(expected_units)` 与 exact snapshot 的双向全等，missing/extra/foreign/duplicate unit 都 fail closed；terminal Finding 只来自 proof-backed verified Attempt 的 compound terminalizer。lease/checkpoint 仍由 P1 WorkerRun 负责。
- FactDelta 的 kind 是闭集 `created|updated|refuted|new_surface`，自由措辞只能进入 `summary`；未知模型 prose 在 typed DTO 反序列化时直接拒绝，不能泄漏成后续 Candidate technique。follow-on route 必须把 `delta_kind`、`observation_kind`、`allowed_techniques`、`enrichment_required` 四个轴分开：强 typed verifier evidence 才可直接冻结 Candidate observation，`refuted` 只形成 no-attack member；recognized unsupported adapter 整个事务回滚，信息不足只形成 pending enrichment authority并保持 source Wave open。
- `selector.rs::select_attack_read` 按 operation-frozen attack contract 一次选择一条完整 semantic record：decision children 先按 work-item key 规范排序，candidate/no-candidate child 数必须与对应 authoritative counts 相等，且 closed-review count 不得超过 frozen WaveUnit count。dual mode 只产生 `Match|Mismatch|V2Missing` shadow outcome；`DualWriteReadV2Fallback` 只能整条 fallback，`V2Only` 对 missing/incomplete 直接 `ATTACK_V2_READ_REQUIRED`，禁止 legacy decision + V2 count 逐字段拼接。production runtime-memory adapter 已在 Candidate final-seal transaction 中从 durable sources 构造完整 records并调用该 selector；DB repo 重算 comparison/source/hash后持久化，不能把 hash 当内容授权。
- `review_barrier::decide_review_barrier` 只消费一次 exact DB snapshot；open/pending/dispatching/stale/resumed/terminal 分支均不读进程内 wake flag。TaskOrchestrator 必须先解析投影 DAG 的真实 successor：`--to attack_candidate` 没有 successor，直接结束且不得读取 review barrier；只有真实的 Candidate→Verification crossing 才进入下面的判断。operation-frozen runtime/attack contract **同时**为 `v2_only` 时才回读并阻塞于该 seam；dual-write 仍合成/镜像 V2 shadow，但绝不读取 barrier、hold 主流程或派发 `candidate_verifier`。exact V2Only 下只有 `resumed|terminal` 才能进入 verification，读失败或 snapshot 不一致一律 hold。
- Candidate 对 evidence outcome 必须按字面解释：`blocked` 只表示检查未完成，不等于阴性，也不能推断为 WAF、限流或目标抵抗；只有本轮可信 evidence 明确给出原因时才能写该原因。`not_applicable` 也只约束 exact technique/producer，不能外推为目标安全。共享 methodology 与 `list_recent_evidence` contract 同时承载这条语义，避免模型把工具/模板配置缺口改写成目标侧结论。
- `target_intel` 的 6 个 `GOLISH-INTEL-*` 覆盖列仍必核，但阶段不再暴露任何 scan-tool selector（`allowed_tool_types=[]`）：found 只能来自 `recon_map_assets` / `recon_lookup_whois` 等 registry/provider 工具落库后的 DB truth；缺 provider、无结果或不适用要走 `blocked` / `checked_empty` / `not_applicable` 终态，不能切 CLI fallback。
- `target_intel` / `external_attack_surface` / `enumeration` / `vuln_triage` 的 `team_scheduler` 统一为 Company Controller 模式：每个公司只 seed 一个 `leader:primary`，role 为 `company_stage_controller`，之后的 WorkItem 必须由 Controller 按需提出并由 server allowlist/fence 落成 durable sibling SubAgent；不存在静态 fixed-shard 初始化，也不存在独立的 final 汇总 Agent。四个阶段的冻结 specialist 分别为 `recon` / `prober` / `enumerator` / `vuln_scanner`，Controller 和子 Worker 均只能解析到本阶段冻结 specialist，跨阶段 role 复用 fail closed。`max_company_units_active`（C）限制并发公司 Unit，`global_provider_cap`（G）限制跨公司所有模型 provider 并发，`max_workers`（K）限制单公司 live WorkerRun 且包含 Controller；三者反序列化时均要求非零。`max_dynamic_requests` 另行限制单公司生命周期内可接受的动态 child 总量。Candidate/Verification 与 Post-Exploit/Reporting/Cleanup 仍使用各自 typed scheduler，不会因这个普通公司阶段合同而切换。
- 当前无 migration 的 Team schema 对稳定 Controller WorkerRun 只支持一条 durable Gate gap source，因此 runtime 冻结 `max_controller_gate_repairs=1`；这不是 Gate 放宽，而是在第二次 gap 写入冲突前 fail closed。如需多轮 Controller Gate repair，必须先做显式向前 migration。
- `StageTeamSchedulerPolicy.aggregator_kind` / `aggregator_role` 以及 runtime-memory 的 `is_aggregator`、`stage_unit_aggregate` 等名称是既有持久化/wire 兼容槽位；当前 `aggregator_role` 的值是 `company_stage_controller`，语义是“唯一 Controller 兼任最终提交者”，不能据字段名恢复成第二个 Agent 或旧静态分片流程。
- Scoping 不执行 DNS/WHOIS/HTTP/端口工作，也不暴露 `manage_targets`。可信 UI/CLI
  必须在 stage 前写入 exact seed；Red Team 可各做一次 `unit_review` / `scope_review`。
  后者从 DB 取 trusted-source snapshot，按 canonical value + target_type + scope 精确对齐；
  编辑只是 proposal，模型/阶段不能写 scope。accepted 后立即终止。Target Intel 只能在
  trusted root 下确定性落 strict-child 域名和被动关系，不能发明 root/IP scope。
- `target_intel → external_attack_surface` 前有独立 `ACTIVE_RECON_TRUSTED_TARGET_REQUIRED` barrier：operation 必须已绑定 engagement org，且 `scoping_target_snapshot` 至少含一个 canonical `scope=in` domain/IP/CIDR/URL/wildcard。generic phase approval 不能制造 target；snapshot read error 也 hold。direct EAS stage slice/恢复 cursor 在 stage request 入口、`sync_stage_execution_on_entry` 与 executor 之前复核同一条件，返回 blocked/Interrupt，避免 CLI 绕过并保留 resumable Waiting。
- `target_intel` 的 SUBDOMAIN 是 registrable-apex 维度；被动发现出的叶子子域名、`www.*` 主机、URL 形态资产都不再要求继续做 SUBDOMAIN 枚举，避免越发现越把 coverage 分母撑大。
- `source_coverage` 规则读取 `GateContext.source_queries`（来自 `source_query_log`）来证明 provider/source 已终态尝试；它不投影 found，found 仍由 DB/ledger truth 决定。
- `coverage_complete` 在 `derive_from_evidence=true` 时只消费**带精确 technique** 的可信非 found source 终态：`checked_empty` / `blocked` 能关闭对应 gap，`error` 只计 source attempt、保持 coverage 非终态，必须修复或显式形成带 note 的 `blocked`。provider-wide `map_assets` 行只能给 `source_coverage` 证明 survey 已尝试，不能替 DNS/ASN/CT/OSINT/SUBDOMAIN 任一维度代签 empty/blocked。自报 `found` 仍必须有 DB/ledger truth，不能靠 source row 过门。
- Target Intel 的非执行 organization context 统一由 `TargetIntelOrganizationContext` 生成 canonical `organization:<uuid>` identity、`organization` 类型与 DNS/CT/SUBDOMAIN deterministic N/A，submit preview 和 final org gate 禁止各写一套。精确可信 N/A 只抑制同一 cell 的 source-query error；精确 evidence `Error`、另一资产或真实 applicable cell 的 source error 继续 fail closed。
- `coverage_complete` BLOCK 时会在 `HarnessRecoveryActions.coverage_gap_actions` 输出结构化缺口清单（`asset` / `technique` / `reason` / `suggested_capabilities` / 兼容 `suggested_tools`），供 `submit_stage_deliverable` 和 sub-agent repair mode 直接告诉模型“只补这些目标/技术/能力”，不要只靠自然语言 reason 猜。
- `operation_graph.json` 的分支边顺序是运行时语义：`made_progress=true` 走第一条（主路），`false` 走最后一条（bail）。攻击能力 profile 下 `enumeration` 主路必须是 `vuln_triage`，`reporting` 只能作为无内容/无可测面时的 bail 边；否则 Red Team/Pentest 会在内容枚举 PASS 后直接跳报告。
- `external_attack_surface` 走 guarded DB-truth 瘦交付：三条 gate path 先保存 fresh current-org business Found，再移除 session-wide/raw ledger 与 business EAS terminal 权限。`found` 只有 `fresh business Found ∩ fresh current-owner technique_outcome(found) ∩ matching positive guarded audit(asset,technique,outcome,id)` 三者交集才能关闭；`empty` 要求 fresh outcome(empty)+matching guarded audit，不要求 business Found。producer org、current target org/project/scope、stage cutoff 或当前 asset/technique 授权任一缺失均 fail closed；orphan business landing、orphan audit row、legacy 无 org/target/freshness 都不能关格。SERVICE-FINGERPRINT 的 fresh business 边仍要求每个 confirmed-open port 有端口级服务面/nmap fingerprint。`coverage_denominator.authoritative=true` 不要求模型手抄 denominator。
- `GateContext.not_applicable_coverage` 是 DB/调用方注入的确定性终态集合，由 `GateContextBuilder` 归一后供 `coverage_complete` 消费；只有规则 terminal set 包含 `NotApplicable` 时才可关闭 cell，不能用它让模型自报的 found 通过。EAS SERVICE 不再用 DNS/53-only 自动 not_applicable：只要有 confirmed-open port，就要求端口级服务指纹或 worker 显式 terminal outcome；但 `GOLISH-EAS-PORT empty/not_applicable` 会在 `org_gate` 中确定性派生同资产 `GOLISH-EAS-SERVICE-FINGERPRINT not_applicable`，因为没有开放端口就没有服务指纹面。Enumeration 现在先把 denominator 严格收敛到 exact HTTP(S) origin，因此 DNS-only/bare IP 根本不进入四轴；read model、submit preview 与 org gate 已删除旧 `eas_service_not_applicable_assets` 注入，不得再制造 rootless `done_assets`。
- `external_attack_surface` 按 identity 分工：domain/url 只用 HTTP Host/SNI 做 LIVENESS，
  并对每个 confirmed exact origin 做 WEB；显式 IP 先 PORT，再对每个 confirmed-open
  port 做 SERVICE。CIDR 行只承担 range LIVENESS+PORT，guarded in-range child IP 进
  supplemental wave 后承担 SERVICE/WEB。wildcard pattern 在 EAS/Enumeration 全 N/A。
- EAS 的适用性分类统一走 `technique_resolver::classify_stage_asset`：显式 IP 进
  PORT/SERVICE，CIDR 仅进 LIVENESS/PORT，`domain` / `url`（包括 host 为 IP 的
  `http://IP:port`）保持 origin/vhost 作用域。read model/worklist/preview/final gate/
  guarded evidence 必须复用同一 seam。
- DNS host→IP、`targets.real_ip` 和 `organizations.domains/app_domains/ip_ranges` 都不授权
  主动 IP 扫描。EAS 的 IP/CIDR 分母只接受可信 intake 显式 seed、已有同 org/
  project authorized target，或 CIDR wrapper guarded 产生的 in-range child IP。
- Target Intel 对 wildcard pattern 只保留 `GOLISH-INTEL-SUBDOMAIN`一格；`found` 需真实
  promoted strict-child domain target，empty/error/blocked 仍可见。模式本身不做 DNS/WHOIS/
  EAS，也不授权 apex。
- EAS 的 WEB-FINGERPRINT 有独立 exact-origin barrier：由本轮 fresh `httpx` / `nmap` / EAS HTTP 探活观测产生的每个 canonical `scheme://host:port` 都必须有同一精确 origin 的 guarded WEB outcome/evidence。terminal 可为 `found` / `empty`，或仅限 `source=eas_fingerprint_web_stack` 的 producer-owned `blocked`；blocked 还必须与 current target-bound audit fact 的 exact origin、technique、outcome、正 evidence id 完整一致。模型自报、父资产 exception、LIVENESS evidence、wrong source/kind/id、同 host 其他 scheme/port 都不能关闭 barrier。target-level 兼容 WEB cell 可把 exact-origin fact 按同 hostname 投影回父 target，但 barrier 仍逐 origin 核完备性，所以一个 HTTPS outcome 不能替代同 host 的 HTTP/其他端口；不同 hostname（含 `www`/apex/兄弟域名）也绝不互补。每个 missing origin 还会产生 `missing_exact_origin` recovery action，保证所有父 cell 已 partial 时 repair 仍有精确入口；required-origin 查询或 freshness/current-wave 身份读取失败时，read model、submit preview 与 final org gate 三路都 fail closed。
- EAS WEB 的 transport error 不再单次 terminal：spec 对 DB `error` marker 明确 `error_is_terminal=false`，只有 backend 同 operation epoch/org/target/origin/failure-class 的第三次 guarded failure能封 WhatWeb producer。这个 producer-owned blocked 可关闭 EAS exact-origin WEB cell，但不能单独缩小 Enumeration；后者还要求独立 fixed transport probe 的 target-bound blocked evidence。operation-aware coverage 仅删除匹配 exact owner/origin，不删除 target/port、也不影响 sibling Host/SNI。
- `enumeration` 的 per-org gate 资产轴只接受 EAS-confirmed、可物化为 exact `scheme://host:port` 的 HTTP(S) origin；alive 裸 domain/IP、仅 `http_status`、unknown TCP service 与 rootless row 不属于内容枚举分母，也不得被记成四轴 `done/not_applicable`。自身 URL、confirmed-open `ports[].url` 或明确 HTTP(S) service metadata 可产生 origin；没有任何 exact origin（含全 dead）是权威零分母。Enumeration 覆盖轴固定为四类：`GOLISH-ENUM-JS`、`GOLISH-ENUM-DIR`、`GOLISH-ENUM-PARAM`、`GOLISH-ENUM-JSAPI`，并与 `ai_get_stage_asset_coverage` / `stage_worklist_*` / `check_stage_asset_coverage` 同源。
- **Enumeration gate contract（2026-07-10）**：实际 denominator 是 normalized `scheme://host:port × GOLISH-ENUM-{JS,DIR,PARAM,JSAPI}`，四轴永远齐全（即使 origin 的 owning target 是 IP/CIDR，也不得丢 PARAM）。`AssetClass::classify` 仍按 host 语义把 `http(s)://IP:port` 归为 `Ip`；`technique_applies_web_aware` 因而必须把 canonical exact Web Origin 自身视作 web-capable，不能拿展开后的 origin 字符串去查 raw-IP `web_capable_assets`，后者只约束裸 IP/CIDR。DNS/53-only raw-host `not_applicable` context 也不得关闭任何 exact-origin cell。`enumeration_axis_from_coverage_snapshot` 同时供 `org_gate` / Task-mode close / submit preview 使用，并再次要求 `exact_web_origin=true`、canonical HTTP(S) URL、排除 `next_wave_pending`，防止任一入口重引入裸 host。per-org gate 只有在 org 已绑定、run/session 非空、freshness cutoff 存在，且 coverage snapshot 的 stage/org/session 身份与当前 gate 完整一致并明确返回 `assets` array 时才消费该轴；读取错误、身份或 shape 畸形都直接 BLOCK，不能回退 raw domain/IP，只有明确的 `assets: []` 才是权威零分母。spec 显式 `authoritative_found=true` / `require_note_for_other=true` / `error_is_terminal=false`：`found` / `checked_empty` 只认当前 run 的 outcome；app DB 投影还要求 ref 能与 same-run audit evidence 的 `(canonical exact-origin, technique, outcome, id)` 四元组完整匹配，`org_gate` 再拒绝非正数 id，自报 found/empty 不能过门。当前 `error` / `partial` 投影为非终态 marker，并且优先级高于自报 found/empty/blocked/not_applicable。没有 freshness cutoff 时必须拒绝整个 Enumeration outcome 投影，不得 presence fallback 到同 session 旧 run；`stage_worklist_*` / `list_enumeration_web_roots` / `check_stage_asset_coverage` 还会在缺 active session 或当前 Enumeration `stage_started_at` 时直接报错；其它 stage 保持默认兼容合同。
- **Trusted blocked 补充合同（2026-07-10，扩展 2026-07-11）**：`EvidenceOutcome::Blocked` 与 Error 分离，并有独立 producer/axis matrix：preflight source 可关闭四轴，route source 只可关闭 DIR，browser source 只可关闭 JS/JSAPI/PARAM。app bridge 在投影前还必须验证对应 audit kind 分别为 `enumeration_transport_blocked`、`dir_probe_recovery_exhausted`、`enumeration_collection_recovery_exhausted`，以及 fresh positive-id current-target-bound evidence；`org_gate::apply_technique_outcome_rows` 再按 source/axis 和 `(origin,technique,evidence_id)` guarded fact 二次 fail closed，无需 deliverable mirror。`TechniqueOutcomeFact.source` 必须穿透 DbRepoProvider seam，UI / submit preview / final org gate 三路同验；found/empty 原 producer ownership 不变，模型自报 found/empty/blocked/not_applicable 全部不能关闭 Enumeration；`not_applicable` 只认 `GateContext.not_applicable_coverage`，最终提交 `coverage: []`。
- `enumeration.allowed_tool_types` 只允许显式 `enum_preflight_web_origins`、`recon/crawler`、`web/route-probe`。不要用宽 `recon/http` selector（会泄露 raw httpx/curl），也不要把 `web/dir-fuzzer` 或主动隐藏参数爆破加回。
- `external_attack_surface` 开启 `asset_wave_barrier=true`：当前 batch 的资产分母优先冻结在 durable `stage_asset_waves` 的 item 列表（没有 running wave 时才回退 `operation_state.stage_started_at` cutoff）；本阶段运行中新落的资产保留进 DB 并显示为 `next_wave_pending`，但不撑大当前 gate 分母。所有当前 org wave PASS 后，`stage_run` queue supplemental durable wave；下一次 `stage_run` 处理该 operation/org/stage 中所有尚未进过任何 wave 的 backlog（含首波 limit 截断的旧资产与运行中新资产），不能用 parent `started_at` 时间地板漏掉未分配资产，也不能被第一轮 pass ledger 误跳过。
- current-wave 身份始终是 durable item 的 `target_id`，不是下游 read model 改写后的显示值。Enumeration snapshot 把一个 target 展开成多个 canonical Web Origin 时，所有 origin 继承 owning target id；共享 origin 的 current-wave owner 必须覆盖旧 owner。worklist/check、`submit_stage_deliverable` preview 与 final per-org gate 都透传同一 running wave `started_at + target_ids + asset_values`。存在但空/空白/错位/缺 target 的 wave 三路 fail-closed，只有真正 NoWave 才允许 cutoff fallback。
- `coverage_complete` 要区分“模型没交 coverage 矩阵”和“调用方明确注入 `in_scope_assets=Some([])`”：前者仍按 I8 BLOCK；后者代表 DB 真值下该 org/stage 零资产，EAS 等阶段应 vacuous pass，并与 `check_stage_asset_coverage.ready_to_submit=true` 保持一致，避免零资产 org 被推入 repair 后开始猜测扫描。
- EAS 工具证据事实也在 `evidence_facts.rs` 派生：`httpx` / `nmap -sn` / `naabu` 会映射到对应 `GOLISH-EAS-*` technique；concrete IP/CIDR 的 LIVENESS gap 会优先建议 `eas_discover_ports`，端口扫描 completion 会同步写 PORT 与 LIVENESS outcome；`whatweb` 只派生 `GOLISH-EAS-WEB-FINGERPRINT`，不再派生 LIVENESS 或 SERVICE-FINGERPRINT。`GOLISH-EAS-LIVENESS` 对 URL endpoint 使用 gate-compatible join key（去 scheme/大小写但保留 port/path），不能把 `http://host:90` 的探活事实折叠为裸 `host`；`nmap` 的 DNS failure（stderr `Failed to resolve`）会成为 `error` terminal fact，避免同一 liveness gap 无限重试。
- `external_attack_surface` 的 `stage_run_pass_token` claim 视为 Surface closeout 信号：它只在 orchestrator 按 per-org completion ledger 重算通过后才有效，避免 fan-out 阶段主 agent 只交 pass token 时被 `surface_coverage` 误拦；泛化的 `discovery` claim 仍不映射到 Surface。
- fan-out 阶段的 pass-token closeout 必须按 scoping 绑定的 engagement root org subtree 核 `org_stage_completions`；只有没有 root 绑定时才允许 legacy 全库 org 口径。若 `operation_state.current_stage` 仍是该 fan-out stage，closeout 还必须要求 completion 晚于本次 `operation_state.stage_started_at`，防止旧 run 的 passed ledger 生成当前 stage 的 pass token。否则同一 embedded DB 里的 sibling/test org 或旧 completion 会把当前 operation 卡死。
- `handoff_catalog` 是 final PASS 的 closed server catalog：允许 Organization/Target/TargetAsset/DNS/API/Directory/JS/Fingerprint/TechniqueOutcome/Finding，以及只用于 Candidate frozen manifest 的 `AttackCandidateWorkItem` key。`GateContextBuilder::build_with_canonical_source_hints` 把 server-derived key hints 与纯 Gate truth 分开返回；`build_server_final_seal`（不实现 model JSON `Deserialize`）绑定 exact runtime fence、submission、scope、deterministic Gate details 和可选 server-derived `CandidateAcceptance` 并生成 hash。acceptance 的 manifest/decision/plan/evidence 任一漂移都会改变 `seal_material_sha256`；DB 仍会在 final-seal 事务中回查 owner/timestamp/content hash/evidence，hint 本身永远不是 canonical truth。
- `operation_continuity` 只做纯决策：输入 profile-projected DAG + `ContinuitySnapshot`，输出已 adopt 阶段、第一未满足 stage、remaining-stage allowlist。是否允许复用、是否询问用户、DB snapshot 怎么构建都在上层；不要把 DB 查询塞进 harness 纯模块。上层 continuity preflight 必须有 engagement root 才能把 scoping 标为 reusable。
- stage tool whitelist 只约束真实扫描调用；`check_job` / `kill_job` / `list_jobs` / `wait_for_background_jobs` 是后台 job 控制面，必须 exempt，否则 submit barrier 报“后台任务仍在跑”后 worker 无法等待输出或检查明显卡死的 job。
- `tool_taxonomy.rs` 把 Golish direct Enumeration tools（含 `enum_preflight_web_origins`）视为 scan taxonomy 成员；preflight 归 `recon/http` 但 stage 只用显式 tool-name selector，其他内容工具归 `recon/crawler` / `web/route-probe`。raw crawler CLI 不进 allow-list。
- `stage_capability.rs` 是“AI 选能力，后端控配方”的元数据层：`target_intel` 只给 provider/read-only 能力，不暴露 scan CLI；EAS 把 LIVE/PORT/SERVICE/WEB-FINGERPRINT 分成 `BackendWrapper` 能力并建议 `eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services` / `eas_fingerprint_web_stack`，而不是让模型拼 httpx/naabu/masscan/nmap/whatweb 参数；`eas_fingerprint_web_stack` 固定 WhatWeb 配方并落 `fingerprints`，只挂 WEB-FINGERPRINT gap suggestion，不挂通用 SERVICE-FINGERPRINT，避免把 WhatWeb 错用到 SSH/MySQL/SMTP 等非 HTTP 服务。Enumeration 的 crawler supplement 也走 `BackendWrapper` 并建议 `enum_crawl_same_origin_urls`，其他内容枚举能力仍是 browser/js_extract/route_probe。`vuln_triage` 只注册两个 foreground `BackendWrapper`：九个 WSTG cell 建议 `vuln_nuclei_general`，`GOLISH-NDAY` 只建议 `vuln_nuclei_fingerprint_targeted`。stage spec 仅 allow 这两个 exact tool name，raw `nuclei` / `pentest_run` / shell 继续 deny-by-default；targeted 的 template ids 由后端按 current-owner fingerprint 冻结，模型不传 CLI 参数或模板路径。新增能力必须同时满足 stage tool whitelist，并保留 `suggested_tools` 作为旧字段兼容。
- `wstg_mapping.rs` 是公式化 Vuln taxonomy 与 canonical Nuclei tag 的双向单一事实源：adapter 不再复制 10 个 technique 常量/映射；alias 只用于 output→technique 解析，command construction 始终发稳定的单一 canonical tag。
- **攻击段三阶段（Candidate V2）**：`vuln_triage` 只写 10 类公式化 `technique_outcomes` + evidence（found/empty/blocked/not_applicable），禁止直接写 Candidate/Finding；`attack_candidate` 无扫描工具，只对 exact frozen manifest 提交 decision drafts，final Gate PASS 同事务接受 Candidate/no-candidate；`verification` 才执行经审批的 immutable plan 并产出 verified/refuted/blocked。DAG 保持无环，跨 wave 回流由外层 wave transaction 控制。
- `vuln_triage` 的 10 个 cell 全部是 producer-owned authoritative truth：Found/Empty/Blocked 只认两条 foreground Nuclei wrapper 的当前 operation、当前 owner、exact-origin evidence；NotApplicable 只认确定性后端 context，Error/partial 始终未完成。模型提交 `coverage=[]`，不能用带 note 的 blocked/N/A 或 source-query/旧 shell 事实闭格。generic/manual subtask gate 被显式拒绝，只有 `stage_run` 的 `vuln_scanner` per-org gate 能封口。
- **Vuln producer source matrix（2026-07-15）**：worklist、submit preview 与 per-org final gate 必须一致区分三类权威 producer：`GOLISH-NDAY → vuln_nuclei_fingerprint_targeted`、`WSTG-ATHN-04 → vuln_probe_anonymous_access`、其余八个公式化 WSTG technique → `vuln_nuclei_general`。匿名访问的 fresh current-owner exact-origin positive evidence 可通过 server-owned `not_applicable_coverage` 终结 cell；不得被 Nuclei source 冒充，也不得因 final gate 的旧统一 source 映射被丢弃。
- V2 Wave entry 是 closed union：generation 0 只接受 final-sealed `vuln_triage` handoff；follow-on 只接受 immutable `FactDeltaConsolidation`。没有 accepted delta 的 org 仍保留在完整 frozen scope，但以 `TerminalNoInput` 明确封口且不创建 placeholder worker。Verification Gate PASS 后只有 DB consolidation 可写 `opened_next_wave|closed_no_delta|exhausted`；`opened_next_wave` 固定回到 AttackCandidate，`exhausted` 固定进入 Reporting，`closed_no_delta` 只有 exact truth 中存在 proof + Finding id + exact lineage 的 `verified` Attempt 才进入 AccessValidation，零 Candidate / 全 refuted / 全 blocked 都进入 Reporting。`StageFlowOutcome.durable_wave_cursor=true` 表示 graph-local fuel counter不得覆盖已提交的 DB 决定，Legacy/dual 才继续走 `chain_wave`。
- Candidate verifier pre-action 只允许 `verify_execute_candidate_action`、`list_recent_evidence`、`submit_candidate_attempt`；identity/background/raw runner/Finding writer 均在 DB side effect 前拒绝。唯一 wrapper 的模型参数只有非负 `action_ordinal`。
- classifier 对 `nuclei_match_v1` 固定生成 `verify.nuclei_template_replay`，对当前且 suspicious 的 `anonymous_access_v1` 固定生成 `verify.anonymous_request_replay`；canonical plan hash 覆盖完整 observation/hash/target/template/endpoint/query。`surface_analysis_v1` 才允许从冻结 registry allowlist 显式选择 generic technique；未知 schema、hash/target/technique 漂移全部 fail closed。
- Reporting 的 `report_revision_validated` 只消费应用层重读的 `ReportingGateTruth`：revision 必须 current+validated，publication 只允许 `unpublished|final`，完整 source hash、claim/citation、validation attestation 与 Cleanup closeout 任一漂移即 BLOCK。Gate PASS 不等于 final publication，artifact/finalize 不在 stage seam。
- Verification capability metadata 的 `writes` 是 server-owned terminal business effects（Attempt evidence/result、Finding lineage、FactDelta），不是模型 SQL grant；action journal 属于执行 wrapper，Finding/lineage 仍只由 compound terminalizer 写。
- 设计见 `docs/design/2026-05-26-*` 与 `docs/design/2026-07-02-attack-stage-formulaic-candidate-exploit.md`；内层 harness 当前 deferred（见 AGENTS.md §6）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit harness
```
