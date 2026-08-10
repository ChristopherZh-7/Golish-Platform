# RAG-first 单阶段 Investigation 跨 Plan 增量实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把 Plan B 的 Hypothesis Analysis 与 Plan C 的 Verification Campaign 收进一个真实 `Investigation` 外层 stage；主 Agent先绑定exact snapshot，按organization建立隔离读取会话，再按application/trust-boundary shard组织该公司下的多个asset，读取RAG/KG、AU、指纹、Nuclei、Tool Truth与可追溯方法论corpus。多Agent自动生成绑定exact target member set的hypotheses，host自动为可验证hypotheses调度verification tasks。ChatPanel继续使用现有`Running specialist agents → tool-detail → 全屏 Agent 工作区`，点击hypothesis只查看其Agent运行过程。

**架构：** 新 operation 使用operation-frozen `unified_investigation_v1` topology、一个 `StageKind::Investigation`、一个 `stage_execution` 和一个 `stage_run request_id`；唯一前驱链是`vuln_triage → application_understanding → investigation`。认知执行复用Golish现有PentAGI substrate：Generator拆ordered subtasks，每个TaskWorker只有一个Primary，由它动态委派现有Pentester/Researcher/Browser/Coder/Installer/Enricher/Memorist/Adviser，worker可有界嵌套委派，Refiner每步调整剩余计划，Reflector纠偏。TaskOrchestrator是唯一dispatch writer。stage治理层保留automatic Verification Admission、durable HypothesisVerificationTask、sealed objective/delegation censuses、objective-local Campaign、Prepared Action/JIT、typed oracle、FactDelta reducer与frozen fuel。现有scope-first `ContextPackProvider`负责客户RAG/KG；新增content-addressed Methodology Corpus与out-of-transaction enrichment contract负责Golish/CyberStrike/security skills的可重放检索。Plan B/C/D的canonical authority、Reporting与rollout继续复用，不另造固定角色orchestrator。

**技术栈：** Rust 2021、Tokio、SQLx/PostgreSQL、rig-core、Tauri 2、ts-rs、React 19、TypeScript 6、Zustand、Vitest、Biome。

**设计依据：** [`docs/design/2026-08-02-rag-first-unified-investigation-stage.md`](../../design/2026-08-02-rag-first-unified-investigation-stage.md)、[`docs/design/2026-08-07-investigation-tool-catalog-and-admission.md`](../../design/2026-08-07-investigation-tool-catalog-and-admission.md)、[`docs/design/2026-07-29-tool-truth-hypothesis-verification-loop.md`](../../design/2026-07-29-tool-truth-hypothesis-verification-loop.md)、[`docs/design/2026-06-02-pentagi-engine-substrate-reference.md`](../../design/2026-06-02-pentagi-engine-substrate-reference.md)、[PentAGI flow execution at `879e87c`](https://github.com/vxcontrol/pentagi/blob/879e87c2c2688c4a95eac9c1aaf3cd6f6123ebe3/backend/docs/flow_execution.md)。

---

## 执行顺序与 Plan B/C/D 替代关系

本计划不是“先把旧B/C migration应用，再回头扩字段”。必须按下表weave，确保每个migration第一次应用前就包含最终contract：

| 顺序 | 保留的旧任务 | 本计划插入/替换 | 完成边界 |
|---|---|---|---|
| 0 | Plan A | 无 | Plan A以新鲜定向证据达到`passing` |
| 1 | Plan B Tasks 1–6 domain/Registry/plan基础 | Task 1–4 topology、corpus、baseline→final snapshot；先完成schema-neutral contract与DDL草案 | 暂不运行任何migration-backed test |
| 2 | Plan B Tasks 7–11 dynamic subtasks/census/seal/legacy projection/read | Task 5只冻结schema-neutral PentAGI ports、prompts、pure reducer与fake-ledger tests；跳过Task 12独立UI | 不注册production runtime，不宣称durable execution |
| 3 | Plan C Tasks 1–11 Campaign/JIT/oracle/FactDelta | Task 6把`00006`、`00007`、`20260802`三份DDL作为一个有序schema batch一次性冻结，并验证analysis/verification durable ledger；Task 7才注册production Investigation runtime并替换Task 12顶层transition | 只有同stage automatic scheduler与一个PentAGI dispatch writer |
| 4 | Plan D D1/D3/D4 | Task 8扩read/stop contract；Task 9取代D2 route/shell/双入口；Task 10扩Gate/report/rollout topology | 复用现有full-pane route |

本计划是跨B/C/D的delta，不建立第五套Registry、Campaign、projection、Agent transcript或Reporting authority。Plan B/C/D保留项可以由本feature的fresh evidence共同验收，但同一时间仍只允许本feature一个`in_progress`。

在Task 3与Task 4之间执行Task 3A：直接向Tool Manager新增10份Investigation缺口工具JSON，并冻结stage-owned admission catalog。工具JSON只增加inventory，不授予Agent执行权；Task 7接production Operator时才允许catalog中已满足typed adapter、Tool Truth、scope与JIT合同的member进入自动调度。

当前工作树审计结果：Plan B/C/D的计划文档和legacy Candidate/Verification runtime存在，但计划中命名的`hypothesis_registry`、`verification_campaign`、`InvestigationWorkspace`目标文件及`20260729000006`–`00008` migrations在本工作树尚不存在。另一方面，Golish现有task orchestrator、agentic loop、13类sub-agent、nested delegation、Refiner与Reflector已经构成PentAGI-style substrate，必须原位复用。实施时先运行Task 1 characterization/inventory；发现同名实现时必须复用并补测试，不得用计划文本覆盖现有代码。

## Gate 0：安全停下当前闭环并切换唯一 active feature

本Gate是实施前置，不计入后续12个实现Task（Task 1–11加Task 3A）。用户当前只要求先写计划，因此本轮不得执行停止、终止进程、切换feature、修改运行态或宣称旧闭环已经结束。等用户明确要求开始切换时，严格执行以下步骤。

### Step 0.1：先识别要停下的对象，不把四种状态混为一谈

执行时分别确认并记录：

1. **开发feature：** 当前`feature_list.json`唯一`in_progress`条目、对应设计/计划、已完成Task、未完成Task和fresh evidence；
2. **Codex开发任务：** task/thread identity、最后一个稳定消息边界、是否仍在工具调用或写文件；
3. **Golish运行闭环：** 如存在，则记录`operation_id/stage_execution_id/stage_run_unit_id/stage_run_request_id`、当前state/change_seq和stop projection；
4. **本地进程：** dev server、backend、test、migration runner或其它进程是否正在持有文件/DB/runtime lease。

只停止用户指明的对象。停止一个Codex开发任务不等于Golish operation已经drain；停止一个Golish stage也不等于开发feature完成；停止本地进程更不等于任何evidence gate通过。

### Step 0.2：在停止前写入可恢复 handoff

在`agent-progress.md`新增一条`PAUSED FOR UNIFIED INVESTIGATION`记录，至少冻结以下字段；值必须来自只读检查，不能凭对话推断：

```text
source_feature_id/status_before/design/plan
branch/head/staged_paths/unstaged_paths/untracked_paths
owned_modified_paths/shared_dirty_paths/conflict_paths
last_completed_task/first_unfinished_task/resume_entrypoint
fresh_verification_command/exit_code/key_output/evidence_ref
codex_task_or_thread_identity/last_stable_message
operation_id/stage_execution_id/stage_run_unit_id/stage_run_request_id/change_seq
transcript_path/run_log_path/run_tree_command
open_lease/open_action/open_migration/open_external_request
pause_requested_at/pause_observed_at/terminal_or_quiesced_state
remaining_risk/next_safe_command
```

若某一字段不适用，明确写`not_applicable`；如果没有检查，写`not_checked`，禁止用空值伪装“没有”。共享dirty tree按owned/shared/unknown分类，只记录，不回滚、不覆盖、不接管其它会话改动。

### Step 0.3：协作式停止并验证确实静止

- Codex开发任务只在稳定消息/工具边界请求暂停，等待任务返回可观察的paused/terminal状态；不得把“已发送停止请求”记成“已停止”。
- Golish运行闭环通过已有stage stop/control contract请求停止，然后读取exact-head projection；只有open work、lease、action、FactDelta和unknown-held denominator按其contract闭合后，才记`quiesced`。如果stop被Gate拒绝，保留真实BLOCK原因。
- 普通开发进程只在确认不承载migration、DB transaction、外部action或其它会话后停止。不得用强杀掩盖unknown outcome；需要强制终止时另行向用户说明风险并取得确认。
- 停止后运行`python3 scripts/run_tree.py --workspace <exact-workspace> --full`；涉及DB权威时追加`--db`。把命令、exit code、terminal state、首要open denominator和transcript/run.log路径写入handoff。

本Step不以“功能通过”为目标，只证明旧执行面已停止或明确记录了为什么不能停止。

### Step 0.4：按前置状态决定feature切换，不伪造`passing`

当前唯一active是`tool-truth-coverage-contract-2026-07-29`，它又是本计划的硬前置。切换矩阵如下：

| Plan A fresh状态 | 旧闭环处理 | `feature_list.json`动作 | 允许的新工作 |
|---|---|---|---|
| 全部focused verification与完成定义满足 | 将Plan A标为`passing`并填fresh evidence | 同一次小修改中把本feature设为唯一`in_progress` | 从Task 1开始 |
| 尚未passing，但剩余工作属于本计划必需的Tool Truth contract | 停止旧的B→C→D→Intel→JS大闭环，只保留Plan A最小前置 | Plan A继续作为唯一`in_progress`，本feature保持`not_started` | 只完成Plan A，不提前做本计划产品代码 |
| 尚未passing，且存在真实外部blocker | 保存handoff并写真实blocker | Plan A转`blocked`，本feature保持`not_started` | 只做不改变产品状态的计划/审计；等待输入 |
| 用户要求绕过或吸收Plan A | 不直接切换 | 先新增设计决策，重写dependency、verification与authority边界并再次评审 | 新决策落盘前不实现 |

不得因为用户要停下旧闭环，就把未验证的Plan A标成`passing`；也不得把“用户选择暂停”伪装成技术`blocked`。如果只是暂停而没有blocker，Plan A保留`in_progress`直至最小前置完成，避免半成品错误回退成`not_started`。

### Step 0.5：原子激活本feature并建立新会话恢复点

仅在矩阵第一行满足后执行：

1. 先读取并断言切换前恰好一个`in_progress`，且ID是Plan A；
2. 在同一次`feature_list.json`修改中把Plan A改为`passing`并写fresh evidence，再把`rag-first-unified-investigation-stage-2026-08-02`改为`in_progress`；
3. 在`agent-progress.md`新建实施会话，引用handoff、当前HEAD/dirty ownership、Task 1入口和本轮最小定向验证；
4. 再次断言切换后恰好一个`in_progress`，且ID是本feature；
5. 先执行Task 1 characterization，不先改schema、generated IPC、外部corpus或production profile。

```bash
jq -e '[.features[] | select(.status == "in_progress")] | length == 1' feature_list.json
jq -r '.features[] | select(.status == "in_progress") | .id' feature_list.json
jq -e '.features[] | select(.id == "tool-truth-coverage-contract-2026-07-29") | .status == "passing"' feature_list.json
jq -e '.features[] | select(.id == "rag-first-unified-investigation-stage-2026-08-02") | .status == "in_progress"' feature_list.json
git diff --check -- feature_list.json agent-progress.md
```

Expected：所有命令exit 0；旧闭环有可恢复handoff；Plan A有fresh passing evidence；本feature是唯一active。若任一断言失败，保持原状态并停止切换。

### Step 0.6：公司、资产与hypothesis的固定分层

实施不得采用“一次把所有公司和资产塞进同一个prompt”，也不要求机械地“每个asset一个Agent”。固定层级为：

```text
operation + Investigation stage_execution + stage_run_request_id
└── organization stage_run_unit
    ├── isolated MainOrganizationReadSession + snapshot + transcript partition
    └── application/trust-boundary shard
        ├── exact asset/origin/endpoint members
        └── hypothesis revision + exact affected-target-set hash
            └── verification task/campaign/action/evidence obligations
```

- **多个公司：** 每个organization拥有独立`stage_run_unit`、snapshot、read session、transcript partition、budget/fuel与action authority。顶层Main只消费typed/redacted receipt，不接收跨公司的raw asset、credential、evidence正文或resume chain。
- **同公司多个资产：** host按application、origin、trust boundary和已知关系形成可重放shard；Agent数量由工作量和边界动态决定，不与asset数量一一绑定。每个shard必须seal exact member set与hash。
- **跨资产hypothesis：** 只允许在同organization内、且存在typed relationship evidence时建立；revision必须列出全部受影响target members。一个asset上的positive/negative evidence不能自动关闭兄弟asset的obligation。
- **相似基础设施：** 即使两个organization出现相同hostname、IP、CDN或代码指纹，也保持不同identity root、snapshot、hypothesis和evidence ledger，不得自动合并。
- **跨公司协调：** 只允许聚合typed/redacted coverage与状态，用于全局进度和资源调度；禁止跨org传播raw检索正文、credentials、target action结果或未脱敏transcript。

## 实施前暂停点

1. **PAUSE A — schema/migration：** Task 2需要为已应用stage-rank函数新增唯一forward-only `20260802000004_unified_investigation_topology.sql`；Task 4与Task 6分别扩展Plan B/C预留的`20260729000006_hypothesis_registry.sql`与`20260729000007_verification_campaigns.sql`。开始任一schema工作前必须取得用户当前轮明确授权并重新确认timestamp未占用。三份DDL必须在任何一份migration文件被materialize、被runner应用、或任何migration-backed test运行前，按`00006 → 00007 → 20260802`一次性落齐、review并冻结；Task 2/4只写schema-neutral contract/tests与DDL checklist，Task 6才同时创建三份migration并执行schema batch。Tasks 2–5的focused tests必须使用pure/fake repo；任何会启动migration runner的test都延后到Task 6。不得修改任何已应用migration；若`00006`、`00007`或更高version已经进入任何目标数据库的`_sqlx_migrations`，立即停止，并按真实实施顺序重新分配全部新增forward-only timestamp，绝不能事后插入更低version。
2. **PAUSE B — generated IPC：** Task 8导出 ts-rs 并生成 `frontend/lib/generated/` 前必须取得明确授权；生成文件只由生成流程写，禁止手改。
3. **PAUSE C — external corpus/license：** Task 3只使用仓库内synthetic fixture。clone、下载、批量vendoring或再分发CyberStrike/第三方7,600+ skill files前必须取得明确授权并记录upstream revision、license与provenance。CyberStrike审计基线为commit `80ee899a4ccb2a152fb505e7ce9e1a7874b1f486`、license `AGPL-3.0-only`。
4. **PAUSE D — external/runtime action：** online enrichment只用fake transport完成contract测试；真实公开网页/provider fetch与任何真实HTTP/browser/CLI/credential/race/OAST动作都需当前轮独立授权，并继续受operation scope、egress policy、Prepared Action/JIT约束。本计划不授权执行。
5. 每次Cargo build/test/clippy前先运行`just space-guard`。未经明确授权，不运行`./init.sh`、`just check`、`just test`、`just precommit`、全workspace suite或真实实体闭环。
6. 同一时间只允许一个`feature_list.json`条目为`in_progress`。Plan A未完成时，本计划与B/C/D保持`not_started`。

## 目标文件结构

### Stage topology 与兼容

- 修改 `backend/crates/golish-agent-kit/src/harness/types.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/resources.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/operation_graph.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/operation_flow.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/operation_mermaid.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/phase.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/phase_flow.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/profile.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/stage_transition.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/stage_capability.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/handoff_catalog.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/stage_fork.rs`
- 修改 `backend/crates/golish-agent-bridge/src/agent_bridge/task_request.rs`
- 修改 `backend/crates/golish-events/src/transcript/summarizer.rs`
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`
- 修改 `backend/crates/golish-reporting-domain/src/report.rs`
- 修改 `backend/crates/golish-tools/src/definitions/security_tools.rs`
- 修改 `backend/crates/golish-db/src/repo/operation_stage_forks.rs`
- 新增 `backend/crates/golish-db/migrations/20260802000004_unified_investigation_topology.sql`
- 修改 `backend/crates/golish/src/cli/args.rs`
- 修改 `backend/crates/golish/src/stage_run/mod.rs`
- 新增 `resources/harness/stages/investigation/spec.json`
- 新增 `resources/harness/stages/investigation/methodology.md`
- 修改 `resources/harness/stages/application_understanding/spec.json`
- 保留 `resources/harness/graph/operation_graph.json` 与 `resources/harness/graph/operation_graph_application_model_v1.json` 的legacy语义
- 新增 `resources/harness/graph/operation_graph_unified_investigation_v1.json`
- 修改 `resources/harness/graph/phases.json`
- 修改 `resources/harness/profiles/pentest.json`
- 修改 `resources/harness/profiles/red_team.json`
- 修改 `resources/harness/profiles/smoke.json`
- 修改 `resources/harness/profiles/assessment.json`
- 修改 `resources/harness/profiles/bug_bounty.json`
- 修改 `resources/harness/profiles/cloud_assessment.json`

### Methodology Corpus 与 RAG snapshot

- 修改 `backend/crates/golish-core/src/lib.rs`
- 新增 `backend/crates/golish-core/src/methodology_context.rs`
- 新增 `backend/crates/golish-core/src/investigation_main_read_session.rs`
- 修改 `backend/crates/golish-skills/src/lib.rs`
- 新增 `backend/crates/golish-skills/src/methodology_catalog.rs`
- 新增 `backend/crates/golish-skills/tests/methodology_catalog.rs`
- 新增 `backend/crates/golish-skills/tests/fixtures/methodology-corpus/manifest.json`
- 新增 `backend/crates/golish-skills/tests/fixtures/methodology-corpus/skills/auth-testing/SKILL.md`
- 新增 `backend/crates/golish-skills/tests/fixtures/methodology-corpus/skills/config-exposure/SKILL.md`
- 修改 `backend/crates/golish-memory-domain/src/context.rs`
- 修改 `backend/crates/golish-memory-app/src/context_pack.rs`
- 修改 `backend/crates/golish-memory-app/src/retrieval.rs`
- 修改 `backend/crates/golish-memory-app/tests/scoped_rag_contract.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/knowledge_context.rs`
- 新增 `backend/crates/golish-agent-app/src/ai/db_bridge/investigation_context.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/context_snapshot.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/methodology_snapshot.rs`
- 新增 `backend/crates/golish-agent-app/src/ai/methodology_enrichment.rs`
- 新增 `backend/crates/golish-agent-app/src/ai/investigation_main_read_session.rs`
- 新增 `backend/crates/golish-agent-app/tests/methodology_enrichment.rs`

### Investigation Tool Manager JSON与admission catalog

- 新增 `resources/toolsconfig/arjun.json`
- 新增 `resources/toolsconfig/kiterunner.json`
- 新增 `resources/toolsconfig/schemathesis.json`
- 新增 `resources/toolsconfig/jwt-tool.json`
- 新增 `resources/toolsconfig/graphql-cop.json`
- 新增 `resources/toolsconfig/testssl-sh.json`
- 新增 `resources/toolsconfig/ssh-audit.json`
- 新增 `resources/toolsconfig/enum4linux-ng.json`
- 新增 `resources/toolsconfig/trivy.json`
- 新增 `resources/toolsconfig/interactsh-client.json`
- 新增 `resources/harness/stages/investigation/tool_catalog.json`
- 修改 `resources/harness/stages/investigation/spec.json`
- 修改 `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- 新增 `backend/crates/golish-pentest-domain/tests/investigation_tool_configs.rs`
- 新增 `backend/crates/golish-agent-kit/tests/investigation_tool_catalog.rs`

### Registry、automatic admission 与 Campaign

- 待Plan B落地后原位复用 `backend/crates/golish-core/src/investigation_contract.rs`
- 待Plan B落地后原位复用 `backend/crates/golish-core/src/hypothesis_verification.rs`
- 待Plan B落地后原位复用 `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/`
- 待Plan C落地后原位复用 `backend/crates/golish-agent-kit/src/harness/verification_campaign/`
- 新增 `backend/crates/golish-core/src/hypothesis_verification_task.rs`
- 新增 `backend/crates/golish-core/src/investigation_fuel.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/admission.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/task.rs`
- 修改 Plan B预留 `backend/crates/golish-db/migrations/20260729000006_hypothesis_registry.sql`
- 修改 Plan C预留 `backend/crates/golish-db/migrations/20260729000007_verification_campaigns.sql`
- 修改 Plan B预留 `backend/crates/golish-db/src/repo/hypothesis_registry.rs`
- 修改 Plan C预留 `backend/crates/golish-db/src/repo/verification_campaigns.rs`
- 新增 `backend/crates/golish-db/src/repo/hypothesis_verification_tasks.rs`
- 新增 `backend/crates/golish-db/src/repo/investigation_fuel.rs`
- 修改 `backend/crates/golish-db/src/repo/mod.rs`
- 修改 Plan B预留 `backend/crates/golish-db/tests/hypothesis_registry.rs`
- 修改 Plan C预留 `backend/crates/golish-db/tests/verification_campaigns.rs`

### Runtime、IPC 与 projection

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- 新增 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/investigation_analysis_agent_runner.rs`，取代Plan B planned `candidate_analysis_agent_runner.rs`，两者不得并存
- 待Plan C落地后原位复用 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/verification_campaign.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs`
- 修改Plan B planned `backend/crates/golish-agent-kit/src/task_orchestrator/hypothesis_analysis.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/task_operation.rs`
- 新增 `backend/crates/golish-agent-app/src/ai/investigation_runtime.rs`，取代Plan B planned `candidate_analysis_runtime.rs`，两者不得形成平行writer
- 待Plan B/D落地后原位扩展 `backend/crates/golish-agent-app/src/ai/commands/investigation/{mod.rs,dto.rs,cursor.rs}`
- 待Plan B/D落地后原位扩展 `backend/crates/golish-core/src/investigation_projection.rs` 与 `backend/crates/golish-db/src/repo/investigation_projection/`
- 修改 `backend/crates/golish-core/src/events/harness_trace.rs`
- 修改 `backend/crates/golish-core/src/events/event.rs`
- 新增 `backend/crates/golish-agent-app/src/ai/investigation_projection_event_bridge.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/mod.rs`
- 新增 `backend/crates/golish-agent-app/tests/investigation_projection_events.rs`
- 修改 `backend/crates/golish/src/state/mod.rs`
- 修改 `backend/crates/golish/src/app/bootstrap.rs`
- 修改 `backend/crates/golish/src/app/window_lifecycle.rs`
- 新增 `backend/crates/golish/tests/investigation_projection_event_wiring.rs`
- 由ts-rs生成更新 `frontend/lib/generated/GeneratedAiEvent.ts`，禁止手改
- 待Plan B落地后原位复用 `backend/crates/golish/src/commands_facade/investigation.rs`
- 修改 `backend/crates/golish/src/commands_registry.rs`
- 修改 `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- 修改Plan B planned `backend/crates/golish-sub-agents/src/defaults/prompts/hypothesis_analysis.rs`，不新增平行Investigation prompt
- 修改 `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- 修改 `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`
- 修改 `backend/crates/golish-sub-agents/src/executor/chain_persist.rs`

### 现有全屏 Workspace

- 待Plan B/D落地后原位扩展 `frontend/lib/api/investigation.ts`
- 新增 `frontend/components/Engagement/InvestigationWorkspaceView.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspaceView.test.tsx`
- 新增 `frontend/components/Engagement/AgentWorkspacePrimitives.tsx`
- 新增 `frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx`
- 修改 `frontend/components/Engagement/StageTeamWorkspaceView.tsx`
- 修改 `frontend/components/Engagement/StageRunDetailShell.tsx`
- 修改 `frontend/components/Engagement/CandidateVerificationWorkspaceView.tsx`（只保留legacy adapter职责）
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx`
- 修改 `frontend/components/AIChatPanel/StageMarker.tsx`
- 修改 `frontend/components/AIChatPanel/StageProgressBar.tsx`
- 修改 `frontend/components/AIChatPanel/AIChatPanel.tsx`
- 修改 `frontend/components/AIChatPanel/StageResetMenu.tsx`
- 修改 `frontend/components/AIChatPanel/StageResetMenu.test.ts`
- 待Plan C落地后原位复用 `frontend/components/Engagement/PendingPreparedActionPanel.tsx`
- 待Plan C落地后原位扩展 `frontend/components/Engagement/PendingPreparedActionPanel.test.tsx`
- 修改 `frontend/App/AppShell.tsx`
- 新增 `frontend/App/AppShell.detail-focus.test.tsx`
- 修改 `frontend/lib/stage-reset.ts`
- 修改 `frontend/store/types/session.ts`
- 修改 `frontend/store/types/index.ts`
- 修改 `frontend/store/store-types.ts`
- 修改 `frontend/store/slices/session.ts`
- 修改 `frontend/store/slices/session-core.ts`
- 新增 `frontend/store/investigation-workspace.test.ts`
- 修改 `frontend/services/ai-events/harness-handlers.ts`
- 修改 `frontend/services/ai-events/harness-handlers.test.ts`

---

## Task 1：用 characterization tests 冻结现行 B/C/D 与 full-screen 入口

**Files:**

- 修改 `frontend/components/AIChatPanel/ToolCallSummary.test.ts`
- 修改 `frontend/components/AIChatPanel/SubAgentInlineCard.test.tsx`
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx`
- 修改 `frontend/App/detailFocus.test.ts`
- 新增 `frontend/App/AppShell.detail-focus.test.tsx`
- 新增 `backend/crates/golish-agent-kit/tests/investigation_legacy_compatibility.rs`
- 新增 `backend/crates/golish-agent-runtime/tests/investigation_pentagi_substrate.rs`
- 修改 `agent-progress.md`

**Step 1：记录实现盘点**

运行：

```bash
rg --files backend frontend | rg 'hypothesis_registry|verification_campaign|investigation_projection|InvestigationWorkspace|HypothesisRegistry'
rg --files backend/crates/golish-db/migrations | rg '2026072900000[678]|hypothesis_registry|verification_campaign|investigation_projection'
rg -n 'StageKind::(AttackCandidate|Verification)|"attack_candidate"|"verification"' backend/crates frontend resources/harness
rg -n 'generate_subtasks|TaskWorker|stage_team_dispatch_workers|Refiner|Reflector|nested delegation' backend/crates/golish-agent-kit backend/crates/golish-agent-runtime backend/crates/golish-sub-agents
```

Expected：结果逐文件记录到本轮`agent-progress.md`，包括整个`backend/crates`、frontend与resources中的closed StageKind exhaustive census，以及现有PentAGI seam；尤其覆盖agent-bridge、events transcript summarizer、pentest bridge、reporting-domain、tools和sub-agents的stage-specific分支。不存在的计划文件记为“未落地”，已有文件按真实接口复用；不根据用户口头“差不多”或旧计划自然语言宣称实现完成。Task 2结束时新增catalog test必须证明census中每个match都有明确legacy/unified处理。

**Step 2：先写现有 UI 入口 characterization**

测试必须证明：

```ts
it("opens one stage_run in the existing full-pane tool-detail route", () => {
  // stage_run card writes [stageRunRequestId] and tool-detail
});

it("opens a specialist under its owning stage_run", () => {
  // specialist card writes [stageRunRequestId, agentRequestId]
});

it("keeps ChatPanel mounted and projecting events while the detail pane is visible", () => {
  // AppShell integration, not only the detailFocus pure helper
});
```

运行：

```bash
pnpm exec vitest run frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/AIChatPanel/SubAgentInlineCard.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/App/detailFocus.test.ts frontend/App/AppShell.detail-focus.test.tsx
```

Expected：现有入口测试全绿；任何失败先修 characterization 或记录真实差异，不开始新 route。

**Step 3：冻结 legacy stage contract**

新增测试：

```rust
#[test]
fn legacy_operations_keep_candidate_then_verification() {
    // legacy frozen profile still projects the two exact stages
}

#[test]
fn a_frozen_legacy_operation_is_never_reinterpreted_as_investigation() {
    // resume/fork reads operation-owned contract, not current global default
}
```

运行：

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-kit --test investigation_legacy_compatibility --status-level fail
```

Expected：RED只因 unified contract/fixture尚未定义；legacy assertions本身必须反映当前真实路径。

**Step 4：冻结PentAGI substrate复用边界**

新增characterization test证明Generator→Primary→dynamic specialist→Refiner/Reflector seam、nested delegation与trusted parent request identity均可复用；再加source catalog断言统一Investigation不得同时注册`candidate_analysis_agent_runner`与`investigation_analysis_agent_runner`，也不得出现第二套固定role orchestrator。

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-runtime --test investigation_pentagi_substrate --status-level fail
```

Expected：现有substrate行为全绿；只有尚未接入Investigation的断言为预期RED。

### Future Commit

```bash
git add frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/AIChatPanel/SubAgentInlineCard.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/App/detailFocus.test.ts frontend/App/AppShell.detail-focus.test.tsx backend/crates/golish-agent-kit/tests/investigation_legacy_compatibility.rs backend/crates/golish-agent-runtime/tests/investigation_pentagi_substrate.rs agent-progress.md
git commit -m "test(investigation): freeze legacy and full-pane behavior"
```

## Task 2：新增 `StageKind::Investigation` 与双 contract graph

**Files:** 使用“Stage topology 与兼容”列出的全部文件。

**Step 1：扩展 closed stage catalog**

先在 `harness/types.rs` 增加失败测试，要求：

```rust
assert_eq!(StageKind::try_parse("investigation"), Some(StageKind::Investigation));
assert_eq!(StageKind::Investigation.as_str(), "investigation");
assert!(StageKind::ALL.contains(&StageKind::Investigation));
```

然后添加enum variant、`ALL`、`as_str`、parser与serde round-trip。旧`AttackCandidate`和`Verification`不删除。

同时新增closed `StageTopologyContractV1`：`LegacyCandidateVerificationV1 | UnifiedInvestigationV1`，冻结到operation contract/fork adoption receipt。catalog test消费Task 1对整个`backend/crates`、frontend和resources的match census，逐个覆盖spec registry、profile/phase、tool list、submit tool、stage transition/capability、handoff、agent bridge、transcript summarizer、pentest bridge、dispatcher、sub-agent branches、fork/resume/rank、CLI/history/reset/report source与security tool definitions；任何新增closed match未登记即失败。

**Step 2：新增 stage spec/methodology**

`resources/harness/stages/investigation/spec.json`必须声明：

```json
{
  "id": "investigation",
  "kind": "investigation",
  "risk_level": "critical",
  "findings_allowed": false,
  "requires_stages": ["application_understanding"],
  "allowed_next_stages": ["access_validation", "reporting"],
  "specialist": "investigation"
}
```

`methodology.md`只描述RAG-first analysis、automatic admission、Prepared Action/JIT与FactDelta闭环；不得包含“点击 hypothesis 启动验证”或允许analysis child调用主动工具的文字。

测试还必须证明Main/Primary、Analysis worker、Verification worker与generic stage deliverable都不能提交Finding；只有host-owned revision adjudicator的专用writer可写Finding。

**Step 3：让新 profile 只有一个节点，legacy graph仍可解析**

保留现有`operation_graph.json`和`operation_graph_application_model_v1.json`逐字节legacy语义；从后者派生新`operation_graph_unified_investigation_v1.json`，唯一攻击链为：

```text
vuln_triage → application_understanding
application_understanding → investigation
investigation → access_validation
investigation → reporting
```

禁止`vuln_triage → investigation`直达边。两条outgoing edge是profile-frozen选择：短流程可直接Reporting，需要post-exploit链的profile才进入Access Validation；同一次stage transition不能同时选择两者。`application_understanding/spec.json`将`investigation`加入合法next，同时保留legacy next。profile resolver由operation-frozen topology选择graph/version；unified projection移除`attack_candidate/verification`，legacy frozen snapshot继续旧两节点。同一projection不得同时出现两种拓扑。

**Step 4：冻结DB stage-rank DDL checklist，但不创建migration文件**

定义`20260802000004_unified_investigation_topology.sql`必须只扩展已应用SQL中的closed stage rank/topology legal pair，不得修改创建旧rank函数的migration。本Task只落schema-neutral rank contract/test和DDL checklist，不创建该migration文件、不运行migration-backed test。Task 6按PAUSE A同时materialize三份DDL；随后repo与direct-SQL tests证明legacy rank不变、Investigation位于AU之后、foreign/unknown topology fail closed、fork/resume读取frozen contract。

**Step 5：定向验证**

```bash
jq empty resources/harness/graph/operation_graph.json resources/harness/graph/operation_graph_application_model_v1.json resources/harness/graph/operation_graph_unified_investigation_v1.json resources/harness/graph/phases.json resources/harness/profiles/pentest.json resources/harness/profiles/red_team.json resources/harness/profiles/smoke.json resources/harness/profiles/assessment.json resources/harness/profiles/bug_bounty.json resources/harness/profiles/cloud_assessment.json resources/harness/stages/application_understanding/spec.json resources/harness/stages/investigation/spec.json
cd backend && just space-guard && cargo nextest run -p golish-agent-kit -E 'test(investigation_stage_kind_roundtrip) | test(stage_topology_contract_) | test(unified_investigation_profile_has_one_stage) | test(legacy_operations_keep_candidate_then_verification) | test(investigation_stage_spec) | test(investigation_generic_submit_rejects_finding)' --status-level fail
```

Expected：schema-neutral选中测试通过且selector实际匹配；unified graph没有`vuln_triage → investigation`，Mermaid对new contract只出现一个Investigation节点，legacy仍显示Candidate→Verification；closed StageKind census无遗漏。DB rank tests明确延后到Task 6 schema batch，不得提前把高version migration应用到开发库。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/harness backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-app/src/ai/stage_fork.rs backend/crates/golish-agent-bridge/src/agent_bridge/task_request.rs backend/crates/golish-events/src/transcript/summarizer.rs backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs backend/crates/golish-reporting-domain/src/report.rs backend/crates/golish-tools/src/definitions/security_tools.rs backend/crates/golish/src/cli/args.rs backend/crates/golish/src/stage_run/mod.rs resources/harness backend/crates/golish-agent-kit/tests/investigation_legacy_compatibility.rs
git commit -m "feat(investigation): add unified stage topology"
```

## Task 3：实现 content-addressed Methodology Corpus

**Files:**

- 新增 `backend/crates/golish-core/src/methodology_context.rs`
- 修改 `backend/crates/golish-core/src/lib.rs`
- 新增 `backend/crates/golish-skills/src/methodology_catalog.rs`
- 修改 `backend/crates/golish-skills/src/lib.rs`
- 新增 `backend/crates/golish-skills/tests/methodology_catalog.rs`
- 新增三项 synthetic fixture文件

**Step 1：先写 manifest/parser/index RED tests**

```rust
#[test]
fn corpus_manifest_binds_revision_license_count_and_root_hash() {}

#[test]
fn index_matches_product_cwe_prerequisite_and_chain_tags_deterministically() {}

#[test]
fn skill_body_is_data_only_and_cannot_inject_tools_or_scope() {}

#[test]
fn same_content_produces_the_same_document_and_corpus_hashes() {}

#[test]
fn corpus_and_document_ids_are_deterministic_not_random() {}

#[test]
fn supersession_signature_revocation_and_license_policy_are_fail_closed() {}

#[test]
fn resolver_rejects_parent_symlink_root_escape_and_toctou_change() {}
```

synthetic fixture只包含通用auth testing与configuration exposure知识；不得复制CyberStrike原文，也不使用具体产品验收fixture。

**Step 2：定义纯 contract**

核心类型：

```rust
pub struct MethodologyCorpusManifestV1 {
    pub corpus_id: DeterministicCorpusId,
    pub source_kind: MethodologySourceKindV1,
    pub upstream_url: Option<String>,
    pub upstream_revision: String,
    pub license_spdx: String,
    pub license_text_sha256: String,
    pub signature_state: MethodologySignatureStateV1,
    pub trust_store_epoch: u64,
    pub document_count: u32,
    pub content_root_sha256: String,
    pub parser_contract_version: String,
    pub index_contract_version: String,
    pub ingested_at: DateTime<Utc>,
    pub superseded_at: Option<DateTime<Utc>>,
}

pub struct MethodologyHitV1 {
    pub corpus_id: DeterministicCorpusId,
    pub document_id: DeterministicDocumentId,
    pub relative_path: String,
    pub content_sha256: String,
    pub score_micros: i64,
    pub matched_tags: Vec<String>,
}
```

`instruction_authority=false`由private constructor/host envelope写死，不作为caller字段。validator强制SHA-256格式、deterministic ID derivation、relative path无穿越/absolute/symlink/root escape、resolve-read identity不变、license/revision非空、document exact count/root hash一致；signature未知/撤销、trust epoch stale、license policy拒绝或superseded manifest不能进入active query。

**Step 3：复用SKILL.md parser但隔离prompt injection**

`golish-skills`只复用frontmatter/body解析，不调用`DefaultSkillProvider`、不自动注入system prompt。catalog输出bounded metadata与safe excerpt ref；正文渲染继续走prompt-safe untrusted envelope。

**Step 4：实现 deterministic query**

query tags来自product/technology/CPE/CWE/WSTG/ATT&CK/auth/trust/gap/prerequisite/chain。stable sort固定为`score_micros DESC, corpus_id, document_id`，top-k与omission写入result。

**Step 5：验证**

```bash
cd backend && just space-guard && cargo nextest run -p golish-core -p golish-skills -E 'test(methodology_)' --status-level fail
cd backend && cargo fmt -p golish-core -p golish-skills -- --check
cd backend && just space-guard && cargo clippy -p golish-core -p golish-skills --all-targets -- -D warnings
```

Expected：synthetic corpus解析、deterministic identity/hash、检索、prompt-injection、signature/license/supersession与path/TOCTOU拒绝全绿；没有外部网络调用。

### Future Commit

```bash
git add backend/crates/golish-core/src/lib.rs backend/crates/golish-core/src/methodology_context.rs backend/crates/golish-skills/src backend/crates/golish-skills/tests
git commit -m "feat(investigation): add methodology corpus contract"
```

## Task 3A：直接补齐Investigation Tool Manager JSON与host-owned admission catalog

**Files:** 使用“Investigation Tool Manager JSON与admission catalog”列出的全部文件。

**Step 1：先写tool inventory与catalog exact-set RED tests**

```rust
#[test]
fn investigation_tool_config_exact_set_is_present_and_unique() {}

#[test]
fn every_new_json_roundtrips_through_production_tool_config_file() {}

#[test]
fn new_configs_use_only_valid_legacy_pentest_phase_tags() {}

#[test]
fn active_or_external_presets_never_become_default_auto_skills() {}

#[test]
fn no_new_config_directly_lands_a_finding_or_checked_empty() {}

#[test]
fn admission_catalog_references_existing_tool_config_ids_exactly_once() {}

#[test]
fn cognitive_roles_never_receive_operator_catalog_tools() {}

#[test]
fn external_oast_is_disabled_without_explicit_runtime_authority() {}
```

exact set固定为：

```rust
const INVESTIGATION_TOOL_CONFIG_IDS: [&str; 10] = [
    "arjun",
    "kiterunner",
    "schemathesis",
    "jwt-tool",
    "graphql-cop",
    "testssl-sh",
    "ssh-audit",
    "enum4linux-ng",
    "trivy",
    "interactsh-client",
];
```

先运行测试确认RED仅来自10份JSON、catalog和parser contract尚不存在，不得因为测试枚举路径错误而RED。

```bash
cd backend && just space-guard && cargo nextest run -p golish-pentest-domain --test investigation_tool_configs --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-agent-kit --test investigation_tool_catalog --status-level fail
```

**Step 2：新增10份production Tool Manager JSON**

每份文件必须使用现有`ToolConfigFile { tool: ToolConfig }`格式。字段矩阵固定如下：

| id | phases | category/subcategory | tier | install source | produces |
|---|---|---|---|---|---|
| `arjun` | `enum,vuln_id` | `web/param` | recommended | `s0md3v/Arjun`或官方`arjun`包 | `parameter,endpoint,info` |
| `kiterunner` | `enum,vuln_id` | `web/api` | recommended | `assetnote/kiterunner` | `endpoint,method,info` |
| `schemathesis` | `vuln_id` | `web/api` | optional | 官方`schemathesis`包 | `api_case,observation,info` |
| `jwt-tool` | `vuln_id,exploit` | `web/auth` | recommended | `ticarpi/jwt_tool` | `token_claim,observation,info` |
| `graphql-cop` | `vuln_id` | `web/graphql` | recommended | `dolevf/graphql-cop` | `observation,info` |
| `testssl-sh` | `vuln_id` | `network/tls` | recommended | `testssl/testssl.sh` | `tls_protocol,cipher,certificate,info` |
| `ssh-audit` | `vuln_id` | `network/ssh` | recommended | `jtesta/ssh-audit`或官方包 | `ssh_algorithm,banner,info` |
| `enum4linux-ng` | `enum,vuln_id` | `network/smb-ad` | recommended | `cddmp/enum4linux-ng` | `host,share,user,domain,info` |
| `trivy` | `vuln_id,post_exploit` | `cloud/container` | recommended | `aquasecurity/trivy` | `vulnerability,misconfiguration,secret,license,info` |
| `interactsh-client` | `vuln_id,exploit` | `network/oast` | optional | `projectdiscovery/interactsh` | `oast_interaction,info` |

所有JSON都要写explicit `runtime/launchMode/install/params/skills/output`。preset只允许version、本地decode/local artifact或bounded network-observe；Arjun/Kiterunner/GraphQL Cop active preset标记manual/JIT语义，Schemathesis state-changing preset与Interactsh default server preset不得加入`skills`。第一版所有`output`省略`db_action`，等待typed adapter evidence-first landing；不得直接复用现有`finding_add`。

**Step 3：冻结host-owned catalog parser**

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct InvestigationOperatorToolProfileV1 {
    pub tool_config_id: String,
    pub capability: String,
    pub execution_class: InvestigationToolExecutionClassV1,
    pub default_availability: InvestigationToolAvailabilityV1,
    pub target_kinds: Vec<String>,
    pub credential_mode: String,
    pub external_service: bool,
    pub terminal_truth: InvestigationToolTerminalTruthV1,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationToolExecutionClassV1 {
    NetworkObserve,
    ActiveBounded,
    LocalThenActive,
    StatefulFuzz,
    LocalOrRegistryRead,
    ExternalOast,
}
```

parser拒绝duplicate id/capability、unknown enum、空target kinds、`external_oast`却`external_service=false`、`stateful_fuzz/external_oast`却非disabled、active member声明terminal truth而无`typed_adapter_required`。`investigation/spec.json`只引用catalog contract/version/hash，不复制第二份member列表。

**Step 4：只做本地无目标smoke和配置验证**

```bash
jq empty resources/toolsconfig/arjun.json resources/toolsconfig/kiterunner.json resources/toolsconfig/schemathesis.json resources/toolsconfig/jwt-tool.json resources/toolsconfig/graphql-cop.json resources/toolsconfig/testssl-sh.json resources/toolsconfig/ssh-audit.json resources/toolsconfig/enum4linux-ng.json resources/toolsconfig/trivy.json resources/toolsconfig/interactsh-client.json resources/harness/stages/investigation/tool_catalog.json
cd backend && just space-guard && cargo run -p golish-pentest-domain --example validate_all_tools
cd backend && just space-guard && cargo nextest run -p golish-pentest-domain --test investigation_tool_configs --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-agent-kit --test investigation_tool_catalog --status-level fail
```

Expected：原47份与新增10份共57份config全部parse/normalize/validate；新增10个id与catalog exact-set一致；工具未安装时测试只证明typed not-ready，不下载、不联网。使用仓库内fake executable/`--version` fixture证明command builder正确；不访问真实target、registry、OAST或credential。

**Step 5：记录尚未满足的runtime/tool landing residual**

逐项把平台安装缺口、上游license/version/revision、executable路径、output format、typed adapter与Tool Truth状态写入`agent-progress.md`和feature evidence。JSON存在但binary smoke、typed landing或authorization contract未绿的member保持`contract_pending`或`disabled`，Task 7不得调度。

### Future Commit

```bash
git add resources/toolsconfig/arjun.json resources/toolsconfig/kiterunner.json resources/toolsconfig/schemathesis.json resources/toolsconfig/jwt-tool.json resources/toolsconfig/graphql-cop.json resources/toolsconfig/testssl-sh.json resources/toolsconfig/ssh-audit.json resources/toolsconfig/enum4linux-ng.json resources/toolsconfig/trivy.json resources/toolsconfig/interactsh-client.json resources/harness/stages/investigation backend/crates/golish-agent-kit/src/harness/stage_spec.rs backend/crates/golish-pentest-domain/tests/investigation_tool_configs.rs backend/crates/golish-agent-kit/tests/investigation_tool_catalog.rs
git commit -m "feat(investigation): add governed operator tool catalog"
```

## Task 4：把 ContextPack、Tool Truth、AU 与方法论封成 exact analysis snapshot

**Files:** 使用“Methodology Corpus 与 RAG snapshot”中除Task 3外的文件，并使用Plan B的snapshot repo/migration。

**Step 1：扩展scoped RAG coverage tests**

新增：

```rust
#[tokio::test]
async fn investigation_context_contains_exact_assets_fingerprints_nuclei_au_and_gaps() {}

#[tokio::test]
async fn foreign_org_or_operation_context_is_rejected_before_retrieval() {}

#[tokio::test]
async fn optional_rag_omission_is_sealed_as_residual_not_complete_analysis() {}

#[tokio::test]
async fn sealed_baseline_and_final_snapshots_reject_post_seal_query_append() {}

#[tokio::test]
async fn target_banner_route_nuclei_and_au_prose_are_untrusted_data() {}

#[tokio::test]
async fn fake_online_enrichment_freezes_a_successor_snapshot_without_private_target_data() {}

#[tokio::test]
async fn main_read_sessions_partition_raw_context_and_transcript_by_organization() {}

#[tokio::test]
async fn main_coordinator_receives_only_typed_redacted_read_receipts() {}

#[tokio::test]
async fn crash_resume_never_rebuilds_one_org_context_from_another_org_partition() {}

#[tokio::test]
async fn multiple_organizations_get_distinct_units_snapshots_and_transcript_partitions() {}

#[tokio::test]
async fn multiple_assets_in_one_org_form_application_shards_with_exact_members() {}

#[tokio::test]
async fn shared_hostname_ip_or_fingerprint_never_merges_organizations() {}

#[tokio::test]
async fn one_asset_evidence_never_closes_sibling_asset_obligations() {}
```

现有`canonical_current`只覆盖窄operation状态；扩展adapter查询exact operation/scope/org下的资产、指纹、Nuclei/technique outcomes、AU current revision/items、evidence与coverage gaps。每行都转成typed `ContextItem`，保留source_ref/content_hash/evidence_ids/validity。host先为每个organization建立独立`stage_run_unit`，再在unit内根据application/origin/trust-boundary关系生成ordered shard manifest；每个manifest保存exact target members与set hash。资产没有被分到shard时必须形成typed omission/residual，不能静默丢失。

**Step 2：定义snapshot header/members**

```rust
pub struct BaselineContextSnapshotV1 {
    pub baseline_snapshot_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub checked_tool_truth_bundle_id: Uuid,
    pub checked_tool_truth_root_count: u32,
    pub checked_tool_truth_root_set_sha256: String,
    pub temporal_cutoff: DateTime<Utc>,
    pub baseline_sha256: String,
}

pub struct InvestigationAnalysisSnapshotV1 {
    pub snapshot_id: Uuid,
    pub baseline_snapshot_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub checked_tool_truth_bundle_id: Uuid,
    pub checked_tool_truth_member_count: u32,
    pub checked_tool_truth_member_set_sha256: String,
    pub temporal_cutoff: DateTime<Utc>,
    pub context_item_count: u32,
    pub context_item_set_sha256: String,
    pub methodology_query_set_sha256: String,
    pub methodology_result_set_sha256: String,
    pub omission_set_sha256: String,
    pub relevant_evidence_snapshot_sha256: String,
    pub open_obligation_set_sha256: String,
    pub semantic_attempt_fingerprint: String,
    pub snapshot_sha256: String,
}

pub struct MainOrganizationReadSessionV1 {
    pub main_read_session_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
    pub session_contract_version: String,
}
```

该contract扩展Plan B完整Candidate Analysis authority bundle，不替代或降级它。members分别保存authority class；`CanonicalDb/Runtime/Handoff/Episode`与`Assertion/Document/TemporalGraph/Vector/Methodology`不得在同一proof role中混淆。Main coordinator先拿server-injected exact manifest/selector，再为每organization创建独立`MainOrganizationReadSessionV1`读取bounded snapshot正文；raw context/transcript只能写该session partition，coordinator只接收host-redacted typed receipt。任何actor都不能自报scope/org，多organization正文不能进入同一prompt、message chain或resume context。snapshot member另存`application_shard_id + target_member_set_sha256`；hypothesis semantic identity必须包含organization和exact affected-target set。跨asset结论只有在revision显式列出member且绑定relationship evidence时成立，不能把单一asset的proof提升为整个shard或organization的proof。

**Step 3：构造mandatory deterministic query与bounded analyst follow-up**

host先seal`BaselineContextSnapshotV1`。独立read-only query-planner wave读取S0，只提交typed local/enrichment query intent；host normalize/dedupe/policy/budget后执行本地检索，必要时创建durable`MethodologyEnrichmentRequestV1`。fake-transport worker在事务外fetch、冻结publisher/license/provenance/result或typed failure，再由host创建并seal新的`InvestigationAnalysisSnapshotV1`。后续dynamic analysis subtasks只读final snapshot，任何retry、enrichment completion、authority change或material FactDelta都创建successor attempt/snapshot，绝不扩写同一census。surface/application/auth/evidence与conflict只是host-sealed coverage/checklist axes和typed output classes；Generator可动态合并/拆分，不能把它们实现为固定Agent lanes。

query intent拒绝target URL、cookie、credential、payload、raw evidence、客户私有标识与scope字段。enrichment使用publisher allowlist、egress classification、redaction、lease/CAS/idempotency key；真实网络仍受PAUSE D。

**Step 4：冻结Plan B DDL/repo contract，但不创建migration文件**

在`20260729000006_hypothesis_registry.sql`的candidate analysis snapshot体系中加入：

- `candidate_analysis_context_pack_members`；
- `candidate_analysis_context_omissions`；
- `candidate_analysis_baseline_snapshots/members`；
- `candidate_analysis_snapshot_attempts`；
- `investigation_main_read_sessions/receipts`；
- `methodology_corpus_manifests`；
- `candidate_analysis_methodology_query_sets/members`；
- `candidate_analysis_methodology_result_sets/members`；
- `methodology_enrichment_requests/outbox/results`。

所有set采用open header→ordered members→host seal；post-seal append/update/delete失败。corpus大正文不写DB，只保存manifest/document identity/hash、安全摘要与content-addressed本地ref。本Task只冻结`00006`的DDL checklist、repo trait与pure/fake tests，不创建migration文件；Task 6同时materialize`00006`、`00007`和`20260802`后才首次应用schema batch。

**Step 5：验证**

```bash
cd backend && just space-guard && cargo nextest run -p golish-memory-app --test scoped_rag_contract --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-agent-kit --test hypothesis_registry_gate -E 'test(context_snapshot_) | test(methodology_signal_never_satisfies_proof)' --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-agent-app --test methodology_enrichment --status-level fail
```

Expected：schema-neutral scope-first、Plan A/B checked authority superset、baseline→query planner→immutable final、per-org Main raw-context/transcript partition、target/methodology prompt injection隔离、fake enrichment与knowledge-signal-only全部通过；没有真实网络调用。DB exact-set/append-only/read-session resume tests延后到Task 6 schema batch。

### Future Commit

```bash
git add backend/crates/golish-core/src/investigation_main_read_session.rs backend/crates/golish-memory-domain backend/crates/golish-memory-app backend/crates/golish-agent-app/src/ai/db_bridge backend/crates/golish-agent-app/src/ai/methodology_enrichment.rs backend/crates/golish-agent-app/src/ai/investigation_main_read_session.rs backend/crates/golish-agent-app/tests/methodology_enrichment.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry backend/crates/golish-agent-kit/tests/hypothesis_registry_gate.rs backend/crates/golish-db/src/repo/hypothesis_registry.rs backend/crates/golish-db/tests/hypothesis_registry.rs
git commit -m "feat(investigation): freeze rag and methodology snapshots"
```

## Task 5：冻结Plan B Analysis的PentAGI schema-neutral contract与ports

**Files:**

- 新增 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/investigation_analysis_agent_runner.rs`，取代Plan B planned Candidate runner
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/hypothesis_analysis.rs`
- 修改 `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- 修改Plan B planned `backend/crates/golish-sub-agents/src/defaults/prompts/hypothesis_analysis.rs`
- 修改 `backend/crates/golish-sub-agents/src/executor/{tool_setup.rs,response_parsing.rs,chain_persist.rs}`
- 修改Plan B的hypothesis analysis runtime/tests

**Step 1：写role/tool isolation RED tests**

```rust
#[test]
fn analysis_roles_receive_snapshot_refs_and_no_active_tools() {}

#[test]
fn proposals_keep_fact_au_gap_rag_and_methodology_refs_separate() {}

#[test]
fn controller_cannot_write_verified_or_refuted() {}

#[test]
fn generator_creates_two_to_eight_snapshot_bound_read_only_subtasks() {}

#[test]
fn primary_dynamically_delegates_each_runnable_subtask_to_an_independent_worker() {}

#[test]
fn main_reads_each_organization_in_an_isolated_snapshot_session_before_dispatch() {}

#[test]
fn main_and_nested_worker_ports_require_exact_transcript_and_dispatch_edge_identities() {}

#[test]
fn nested_delegation_refiner_and_reflector_keep_exact_parent_and_snapshot_identity() {}

#[test]
fn refiner_can_patch_strategy_order_but_not_sealed_input_or_coverage_denominator() {}

#[test]
fn investigation_registers_no_parallel_fixed_role_or_candidate_runner() {}

#[test]
fn analysis_dispatches_per_org_then_per_application_shard_without_one_agent_per_asset() {}

#[test]
fn cross_asset_hypothesis_requires_exact_members_and_relationship_evidence() {}

#[test]
fn identical_claims_in_different_orgs_keep_distinct_roots_tasks_and_evidence() {}
```

**Step 2：用fake ledger冻结PentAGI pipeline port，不注册production runtime**

operation Main coordinator拥有`owning_stage_run_request_id + transcript_request_id + worker_run_id`，但不读取raw正文。它先为每organization请求Task 4定义的`MainOrganizationReadSessionV1`，只消费typed/redacted receipt；pure test证明每个session使用不同context chain/transcript partition，crash/resume不能跨org恢复。每organization至少一个Analysis TaskWorker，并按host-sealed application shard拆分工作；每个TaskWorker只有一个Primary。Generator从host提供的coverage/checklist axes与exact shard member set动态合并或拆分2–8个ordered read-only subtasks；Primary按subtask动态选择现有Researcher/Enricher/Adviser/Memorist或受限analysis specialist，并可在host allowlist内嵌套委派。每个runnable subtask至少有一个不同actor identity的worker，不能Primary自问自答，也不能为每个subtask再造Primary。是否拆成多个TaskWorker取决于application/trust boundary和bounded context，而非简单按asset计数。

role definition复用现有registry，实际工具面取`role tools ∩ investigation-analysis envelope`：只含typed snapshot reads、bounded local methodology query intent和typed result barrier；不含`pentest_run`、HTTP、browser target access、MCP外部调用、knowledge mutation、Finding、scope/action/authorization工具。Refiner在每个subtask后只能调整尚未执行的strategy/order；Reflector只纠正无tool/非contract输出。port要求每个dispatch/result/patch/reflect event携带`transcript_request_id + parent_actor_transcript_request_id + parent_dispatch_tool_request_id + worker_run_id + snapshot hash`。

定义private-constructor `InvestigationPentagiExecutionLedgerPort`与tagged subject：`AnalysisAttempt { attempt_id, semantic_attempt_fingerprint } | VerificationTask { task_id, stable_task_key_sha256 }`。logical dispatch key固定为`subject + task_plan_hash/version + subtask_id + parent_dispatch_receipt_id + dispatch_ordinal`；retry只在同一dispatch receipt下递增fenced `attempt_epoch`，不能创建新logical dispatch。port覆盖begin plan、reserve dispatch、begin/finish attempt、append nested edge/pipeline event与seal census；只有TaskOrchestrator持有调用capability。Task 5只用in-memory fake证明唯一writer与identity，不声称`stage_team_*`或新表已持久化；DB实现和production wiring分别在Task 6/7完成。

**Step 3：提交typed proposals并由host reducer seal**

worker结果schema分别包含fact/evidence、AU、gap、RAG、methodology refs。TaskWorker只聚合typed outputs；pure reducer重算semantic key/root/revision/verification plan，并在fake ledger中验证H1/H2/input/checklist/chunk/subtask/delegation census seal。Generator/Primary/Refiner/worker都不能写canonical revision或verified/refuted；真实DB seal延后到Task 6，runtime注册延后到Task 7。

**Step 4：验证**

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents -E 'test(investigation_analysis_contract_) | test(candidate_analysis_reducer_) | test(pentagi_analysis_fake_ledger_) | test(methodology_refs_are_signal_only)' --status-level fail
cd backend && just space-guard && cargo clippy -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents --all-targets -- -D warnings
```

Expected：schema-neutral PentAGI Generator→Primary→dynamic/nested worker→Refiner/Reflector、2–8 subtasks、工具隔离、per-org Main read-session partition、exact identity port、ref分类、pure reducer与zero-hypothesis fake census全绿；source catalog证明只有一套planned analysis seam。没有migration runner、production module registration或durable-complete声明。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/task_orchestrator/hypothesis_analysis.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/investigation_analysis_agent_runner.rs backend/crates/golish-sub-agents/src/defaults backend/crates/golish-sub-agents/src/executor backend/crates/golish-agent-kit/tests backend/crates/golish-agent-runtime/tests backend/crates/golish-sub-agents/tests
git commit -m "feat(investigation): define rag-first analysis contracts"
```

## Task 6：实现automatic VerificationAdmissionSet与HypothesisVerificationTask

**Files:** 使用“Registry、automatic admission 与 Campaign”列出的Plan C相关文件。

**Step 1：写admission reducer RED tests**

```rust
#[test]
fn every_current_revision_gets_exactly_one_admission_disposition() {}

#[test]
fn every_ready_policy_allowed_revision_is_scheduled_automatically() {}

#[test]
fn missing_capability_schedules_the_task_and_becomes_an_objective_residual() {}

#[test]
fn no_known_capability_is_not_a_planning_readiness_value() {}

#[test]
fn deferred_readiness_gets_one_deferred_admission_member_and_reporting_residual() {}

#[test]
fn concurrency_limits_queue_tasks_but_never_drop_members() {}

#[test]
fn scheduler_replay_returns_the_same_task_and_campaign_ids() {}

#[test]
fn unchanged_revision_plan_evidence_and_obligations_reuse_task_across_generations() {}

#[test]
fn same_semantic_evidence_in_a_new_snapshot_reuses_task_and_stale_freshness_blocks() {}

#[test]
fn material_evidence_or_host_signed_rerun_receipt_creates_a_new_task_fingerprint() {}

#[test]
fn stable_task_key_is_unique_for_all_history_and_admissions_append_membership() {}

#[test]
fn every_plan_objective_has_exactly_one_campaign_or_typed_residual() {}

#[test]
fn zero_campaign_task_cannot_terminal_without_complete_explicit_residuals() {}

#[test]
fn campaign_header_is_reserved_before_assignment_seal_in_the_same_transaction() {}

#[test]
fn campaign_cancellation_appends_outcome_without_mutating_objective_assignment() {}

#[test]
fn outcome_campaign_set_equals_campaign_assignment_set_and_zero_campaign_seals_empty() {}

#[test]
fn already_satisfied_binds_current_adjudication_and_semantic_evidence_hash() {}

#[test]
fn task_state_events_use_cas_and_illegal_transition_or_post_terminal_append_fails() {}

#[test]
fn scheduler_bootstrap_and_reaper_claim_queued_work_without_ui_and_replay_after_crash() {}

#[test]
fn task_admission_never_authorizes_a_prepared_action() {}

#[test]
fn analysis_and_verification_subjects_share_one_durable_pentagi_ledger_contract() {}

#[test]
fn logical_dispatch_is_unique_and_retries_are_fenced_attempts_under_one_receipt() {}

#[test]
fn main_read_session_resume_is_durable_and_never_crosses_organization_partition() {}
```

**Step 2：定义contract**

```rust
pub enum VerificationAdmissionDispositionV1 {
    Scheduled,
    NeedsEnrichment,
    Deferred,
    OutOfScope,
    Unsafe,
    AlreadyTerminal,
    NoNewObligation,
}

pub struct HypothesisVerificationTaskHeaderV1 {
    pub task_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub hypothesis_revision_sha256: String,
    pub verification_plan_sha256: String,
    pub relevant_evidence_snapshot_id: Uuid,
    pub semantic_evidence_set_sha256: String,
    pub open_obligation_set_sha256: String,
    pub semantic_attempt_fingerprint: String,
    pub task_contract_version: String,
    pub first_admission_generation_id: Uuid,
    pub host_rerun_receipt_id: Option<Uuid>,
    pub host_rerun_receipt_sha256: Option<String>,
    pub rerun_contract_version: Option<u32>,
    pub stable_task_key_sha256: String,
}

pub enum HypothesisVerificationTaskStateV1 {
    Admitted,
    Queued,
    Planning,
    Running,
    AwaitingAuthorization,
    Consolidating,
    StopPending,
    Draining,
    Cancelled,
    Blocked,
    RecoveryRequired,
    Terminal,
}

pub enum TaskObjectiveAssignmentV1 {
    Campaign { campaign_id: Uuid },
    AlreadySatisfied {
        objective_adjudication_id: Uuid,
        adjudication_sha256: String,
        semantic_evidence_set_sha256: String,
    },
    Residual {
        kind: TaskObjectiveResidualKindV1,
        reason_code: String,
        owner: String,
        next_action: String,
        residual_receipt_id: Uuid,
    },
}

pub enum TaskObjectiveResidualKindV1 {
    NoKnownCapability,
    NeedsEnrichment,
    Deferred,
    OutOfScope,
    Unsafe,
    Blocked,
}

pub enum TaskObjectiveOutcomeV1 {
    CampaignTerminal { campaign_id: Uuid, terminal_receipt_id: Uuid },
    CancelledBeforeStart { campaign_id: Uuid, stop_receipt_id: Uuid },
    RecoveryRequired { campaign_id: Uuid, recovery_receipt_id: Uuid },
}
```

stable task key由host使用operation/stage execution/unit/org/scope/revision+hash/plan/semantic evidence exact set/open obligations/task contract version/semantic attempt fingerprint计算。semantic evidence hash排除`read_at`、epoch、timestamp与snapshot包装字段；freshness失效先block/revalidate。admission generation只通过append-only admission→task membership记录provenance，不参与identity。Agent与前端不传key或rerun reason。相同semantic attempt跨generation与same-semantic snapshot刷新复用；只有material input变化，或host签发带reason、authority receipt hash与monotonic contract version的rerun receipt，才能改变semantic attempt fingerprint并新建。

task header immutable。状态只通过append-only `hypothesis_verification_task_state_events`与CAS current-head projection推进；closed transition table拒绝非法跳转、post-terminal append与把unknown execution标cancelled。

**Step 3：schema PAUSE后扩展Plan C migration**

根据Task 2/4冻结的checklist，在同一review boundary同时materialize`20260729000006_hypothesis_registry.sql`、`20260729000007_verification_campaigns.sql`与`20260802000004_unified_investigation_topology.sql`，组成PAUSE A唯一schema batch。`00007`加入：

- `verification_admission_sets/members`；
- `verification_admission_task_memberships`；
- `hypothesis_verification_tasks`；
- `hypothesis_verification_rerun_receipts`；
- `hypothesis_verification_task_state_events/heads`；
- `hypothesis_verification_task_objective_assignment_sets/members`；
- `hypothesis_verification_task_objective_outcomes`；
- `hypothesis_verification_task_campaigns`；
- `investigation_pentagi_task_plans/subtasks/pipeline_events/delegation_census_seals`；
- `pentagi_task_run_requests/receipts`；
- `pentagi_logical_dispatch_receipts/attempts`；
- `investigation_fuel_budget_heads/reservations/events`与`investigation_semantic_cycle_receipts`；
- 全历史unique stable-task-key约束；
- immutable admission/task creation receipts；
- stage/unit/org/scope compound FKs与append-only triggers。

objective assignment set逐一覆盖Plan B sealed verification plan exact objectives；在同一事务先创建/预留Campaign immutable header，再写引用它的member并seal set。每个member恰好引用一个Campaign、带current adjudication/evidence的already-satisfied receipt，或带reason/owner/next-action/receipt的explicit residual。Campaign后续terminal/cancel/recovery只append outcome，不改assignment。零Campaign只在全集显式pre-admission residual时合法，并且不能写verified/refuted。它是Plan C `hypothesis_revision + objective + required_control` denominator的父级补充，不替代Campaign内部required-control set。task只聚合objective campaigns，不拥有verdict；单个task/campaign terminal不能直接终结revision。

`TaskObjectiveOutcomeV1` exact set只覆盖`Assignment::Campaign`成员，并在seal时强制`outcome.campaign_id set == assignment.campaign_id set`。AlreadySatisfied/Residual由immutable assignment receipt自身终结，禁止生成outcome；zero-Campaign task必须seal empty outcome set。它是Plan C `hypothesis_revision + objective + required_control` denominator的父级补充，不替代Campaign内部required-control set。task只聚合objective campaigns，不拥有verdict；单个task/campaign terminal不能直接终结revision。

PentAGI durable mapping固定为：existing `stage_team_plans`只保存一个stage-run-unit治理envelope；`investigation_pentagi_task_plans/subtasks`保存每个tagged subject（`analysis_attempt | verification_task`）的Generator输出与ordered set；`pentagi_task_run_requests`对tagged subject identity全历史唯一。每个logical dispatch receipt以`subject + task_plan_hash/version + subtask_id + parent_dispatch_receipt_id + dispatch_ordinal`唯一；retry只写同一receipt下带lease/fence的attempt epoch。实际Primary/worker/nested edge复用`stage_work_items + stage_worker_requests + stage_worker_runs`并绑定logical receipt；Refiner patch与Reflector attempt进入append-only pipeline events；census seal保存exact count/set hash。

DB repo实现Task 5的private `InvestigationPentagiExecutionLedgerPort`，仅TaskOrchestrator持有writer capability；begin plan、reserve logical dispatch及对应StageTeam work item/request、begin fenced attempt、finish result与seal census各自使用单一事务边界。Investigation runtime、Hypothesis scheduler和StageTeam scheduler都只能提交run request或读取projection，不能直接写dispatch ledger。`00006`还落地`investigation_main_read_sessions/receipts`，以compound FK锁定unit/org/snapshot/context/transcript partition；resume只读取同partition。

schema batch首次应用后，必须在任何后续Task前一次跑齐Task 2 stage-rank、Task 4 snapshot与本Task admission/task DB tests；任何一份DDL缺失都不得启动migration runner。

**Step 4：自动调度而非UI mutation**

generation seal outbox触发scheduler，但event不是唯一入口。cold-start bootstrap与periodic reaper扫描`sealed generation + missing admission receipt`、`queued + unclaimed`、expired lease/recovery；以stable request key、lease/CAS/fence幂等创建/claim task，capacity释放自动wake下一条。scheduler强读current revision/plan/readiness/policy/capability/evidence/obligations，seal admission，并在同一事务预留Campaign header、seal objective assignment、append admission membership；crash/response loss/duplicate wakeup返回同一receipt，不重复Campaign或Prepared Action。

Hypothesis task scheduler的派工权限止于claim task并写一个幂等`PentagiTaskRunRequested`。现有TaskOrchestrator是Generator/Primary/dynamic+nested worker/Refiner/Reflector的唯一dispatch writer；Investigation runtime和StageTeam adapter都不得第二次调用`stage_team_dispatch_workers`。

不存在`investigation_start_hypothesis_verification` command；任何同名旧草案/API必须为零引用。UI不在scheduler测试进程中也必须自动完成queued→planning→running或显式blocked/recovery。

**Step 5：验证**

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-kit -E 'test(verification_admission_) | test(hypothesis_verification_task_)' --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-db --test operation_stage_forks -E 'test(unified_investigation_) | test(stage_topology_)' --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(analysis_context_) | test(methodology_snapshot_) | test(main_read_session_)' --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-db --test verification_campaigns -E 'test(admission_) | test(hypothesis_task_) | test(pentagi_ledger_) | test(investigation_fuel_)' --status-level fail
rg -n 'investigation_start_hypothesis_verification|Start verification' backend frontend
```

Expected：三份migration按version一次性首次应用且全部DB tests全绿；所有plan objective有immutable assignment，只有Campaign assignments有exact-one outcome，zero-Campaign seal empty outcome；no-capability/deferred不丢member，全历史stable key与append-only admission membership阻止跨generation/same-semantic refresh重复攻击；analysis/verification tagged subject、logical dispatch receipt/fenced retries、Main per-org read-session resume及TaskOrchestrator-only DB port全绿；queue在零UI/零click下自动drain，crash/replay不重复；最后`rg`零命中。

### Future Commit

```bash
git add backend/crates/golish-core/src/hypothesis_verification_task.rs backend/crates/golish-core/src/investigation_fuel.rs backend/crates/golish-core/src/lib.rs backend/crates/golish-agent-kit/src/harness/verification_campaign backend/crates/golish-db/migrations/20260729000006_hypothesis_registry.sql backend/crates/golish-db/migrations/20260729000007_verification_campaigns.sql backend/crates/golish-db/migrations/20260802000004_unified_investigation_topology.sql backend/crates/golish-db/src/repo backend/crates/golish-db/tests/operation_stage_forks.rs backend/crates/golish-db/tests/hypothesis_registry.rs backend/crates/golish-db/tests/verification_campaigns.rs
git commit -m "feat(investigation): schedule hypothesis verification tasks"
```

## Task 7：在同一个stage_run内运行Campaign与FactDelta循环

**Files:** 使用“Runtime、IPC 与 projection”中的runtime/app/subagent文件。

**Step 1：写one-stage orchestration RED tests**

```rust
#[tokio::test]
async fn one_investigation_stage_runs_analysis_then_automatic_campaigns() {}

#[tokio::test]
async fn production_composition_registers_one_analysis_runner_after_schema_batch() {}

#[tokio::test]
async fn verification_task_reuses_pentagi_generator_primary_dynamic_workers_refiner_and_reflector() {}

#[tokio::test]
async fn every_runnable_subtask_has_an_independent_worker_and_durable_delegation_census() {}

#[tokio::test]
async fn nested_worker_delegation_keeps_exact_task_subtask_parent_and_stage_identity() {}

#[tokio::test]
async fn main_crash_resume_reopens_only_the_exact_organization_read_session_partition() {}

#[tokio::test]
async fn task_scheduler_writes_one_run_request_and_task_orchestrator_is_the_only_dispatch_writer() {}

#[tokio::test]
async fn cognitive_and_nested_workers_never_receive_raw_action_tools_even_after_jit() {}

#[tokio::test]
async fn nested_delegation_cannot_expand_stage_scope_credential_budget_or_authorization() {}

#[tokio::test]
async fn missing_or_wrong_investigation_harness_tag_fails_closed_before_worker_dispatch() {}

#[tokio::test]
async fn material_fact_delta_reenters_analysis_without_a_second_stage_run() {}

#[tokio::test]
async fn strategy_variants_stay_in_campaign_and_new_claims_enter_registry() {}

#[tokio::test]
async fn high_risk_action_waits_for_jit_inside_the_same_task() {}

#[tokio::test]
async fn frozen_fuel_and_semantic_cycle_guard_stop_unbounded_reanalysis_and_reexecution() {}

#[tokio::test]
async fn concurrent_work_creation_cannot_oversell_atomic_fuel_reservations() {}

#[tokio::test]
async fn durable_begin_consumes_fuel_and_unknown_execution_stays_held_without_replay() {}
```

**Step 2：把Campaign cognition映射到现有PentAGI Task/Subtask engine**

只有Task 6 schema batch与durable port integration全绿后，才在`direct/mod.rs`注册`investigation_analysis_agent_runner`、在`ai/mod.rs`注册`investigation_runtime`，并让`task_orchestrator/orchestrator.rs`持有DB-backed `InvestigationPentagiExecutionLedgerPort`。production composition catalog必须证明Candidate runner不存在、Investigation runner只有一个、Hypothesis scheduler和StageTeam scheduler都没有第二条dispatch调用路径。

使用稳定键：

```text
leader:primary
analysis:<attempt>:primary
analysis:<attempt>:subtask:<partition>:worker:<worker-run-id>
hypothesis:<revision-uuid>
task:<task-uuid>:primary
task:<task-uuid>:subtask:<subtask-uuid>:worker:<worker-run-id>
task:<task-uuid>:refiner:<patch-ordinal>
task:<task-uuid>:reflector:<attempt-ordinal>
```

deterministic action/oracle是artifact/tool event，不创建伪Agent。

`HypothesisVerificationTask`复用TaskWorker：Generator依据sealed objective assignment与Campaign required-control denominator创建ordered subtasks；整个task只有一个Primary，该Primary为每个subtask从host allowlisted现有Pentester/Researcher/Browser/Coder/Installer/Enricher/Memorist/Adviser中动态选择worker，并可有界嵌套委派；Refiner每个subtask后只能patch剩余strategy/order；Reflector只纠正无tool/非contract输出。不得为每个subtask另造Primary，也不得另建固定Strategist/Critic/Refiner agent registry或每次硬派同一角色。

每个runnable subtask至少有一个不同actor identity的worker，实际dispatch/result/nested edge/refiner patch/reflector attempt形成sealed delegation census。worker timeout/failure进入census与residual；Primary不能单Agentterminal。Task 6定义的durable mapping是唯一source：`investigation_pentagi_task_plans/subtasks`保存Generator与plan patch head，`stage_work_items/requests/worker_runs`保存delegation，pipeline events保存Refiner/Reflector，census seal保存count/set hash。Task scheduler只写一个`PentagiTaskRunRequested`，TaskOrchestrator唯一dispatch；StageTeam不得启动第二套loop。

Pentester/Browser/Coder/Installer等名称在此只是cognition profile。Primary及所有dynamic/nested specialist始终只能提交typed strategy、action intent与typed result；即使某Prepared Action已经JIT approved，也不获得raw HTTP、target browser、CLI/shell、credential、`pentest_run`、record_finding或action executor工具。host把intent编译成one-action command后，只由typed Operator执行真实I/O。每次generated/nested dispatch必须从trusted parent继承并由host重验exact `harness_stage=investigation`、operation/stage execution/unit/task/subtask、scope、organization、credential authority、fuel与authorization envelope；child只能收窄，缺tag或扩大任一轴都fail closed。

**Step 3：替换新contract的continuation**

`task_operation.rs`对legacy contract保留Candidate→Verification continuation；unified contract禁止该函数并始终在原Investigation execution内消费admission/task/campaign。resume使用exact operation/stage execution/unit/task cursor，不按“latest”猜测。

**Step 4：FactDelta回流**

Campaign terminal→objective outcome→FactDelta bundle→Registry reducer。same-claim evidence生成同root successor revision；独立可证伪claim经derive/split/merge。material delta先创建successor baseline/final snapshot与new semantic attempt fingerprint，再seal generation/admission；无material change写fixed-point/no-new-obligation receipt，不制造空generation或重复task。

**Step 5：冻结总fuel与cycle guard**

operation contract固定max analysis generations、verification tasks、Campaigns、subtasks、nested delegations、consult/tool calls、Prepared Actions、wall-clock、token/provider与risk budget。Task 6 schema中的`investigation_fuel_budget_heads/reservations/events`由DB repo唯一写入：operation/unit/task各轴持有immutable limit与CAS head；admission、Refiner patch、nested delegation和Prepared Action creation在创建work的同一事务/fence原子reserve。只有dispatch/durable begin前确定取消的reservation可refund；模型/provider request发出或Prepared Action durable begin时consume。response loss/unknown execution转`unknown_held`，不得refund或自动重放，直到typed recovery/manual settlement证明最终状态。`investigation_semantic_cycle_receipts`对revision/plan/semantic-evidence/open-obligation/remaining-work fingerprint做unique fence。reserve失败、unknown-held或cycle重复时写typed residual并停止相关work；不能先读remaining再异步扣减。UI click、refresh、Agent prose与duplicate event均不能补fuel。

**Step 6：验证**

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-runtime -p golish-agent-app -p golish-agent-kit -p golish-sub-agents -E 'test(unified_investigation_) | test(investigation_fact_delta_) | test(verification_campaign_)' --status-level fail
cd backend && just space-guard && cargo clippy -p golish-agent-runtime -p golish-agent-app -p golish-agent-kit -p golish-sub-agents --all-targets -- -D warnings
```

Expected：schema batch之后production只注册一个Investigation analysis/runtime seam；一个stage execution跨generation运行；每task一个Primary、唯一TaskOrchestrator dispatch writer、tagged subject/logical dispatch receipt/fenced retry、PentAGI dynamic/nested delegation、Refiner/Reflector、Main per-org resume partition、durable census与exact transcript identity全绿；cognitive worker即使JIT后也拿不到raw executor工具，nested child只能收窄authority；legacy continuation回归仍绿；原子fuel与unknown-held/cycle终止可证明；点击/前端完全不参与调度测试。

### Future Commit

```bash
git add backend/crates/golish-agent-runtime backend/crates/golish-agent-app backend/crates/golish-agent-kit backend/crates/golish-sub-agents
git commit -m "feat(investigation): run campaigns inside one stage"
```

## Task 8：扩展exact stage-run read model与独立stop control

**Files:**

- 待Plan B/D落地后原位扩展`golish-core/src/investigation_projection.rs`、`golish-db/src/repo/investigation_projection/`与`golish-agent-app/src/ai/commands/investigation/{mod.rs,dto.rs,cursor.rs}`
- 待Plan B落地后原位扩展commands facade/registry
- 待Plan B/D落地后原位扩展`frontend/lib/api/investigation.ts`
- 修改`golish-core/src/events/{harness_trace.rs,event.rs}`并新增`golish-agent-app/src/ai/investigation_projection_event_bridge.rs`
- 修改`golish-agent-app/src/ai/mod.rs`、`golish/src/state/mod.rs`、`golish/src/app/{bootstrap.rs,window_lifecycle.rs}`完成production lifecycle
- 新增`golish-agent-app/tests/investigation_projection_events.rs`
- 新增`golish/tests/investigation_projection_event_wiring.rs`
- 由ts-rs生成对应`frontend/lib/generated/`类型
- 修改Plan B/D的authorization/read-model tests

**Step 1：read model RED tests**

summary/detail必须返回：

```text
operation + investigation stage identity
analysis snapshot/source census
Main/Analysis Agent topology
hypothesis roots/revisions/relations/readiness
automatic admission disposition
verification task + objective Campaign topology
PentAGI Task/ordered Subtask/dynamic+nested Agent topology
Prepared Action/JIT safe projection
oracle/outcome/FactDelta/timeline
methodology citations without raw corpus body
change_seq + projection_schema_version + staleness
stage_topology_contract + investigation_run_state + investigation_run_state_head
stop_epoch + stop/reset/fork availability/reason + adoption/control policy version
```

每个actor node返回host-verified`organization_id/hypothesis_revision_id/task_id/subtask_id/worker_run_id/owning_stage_run_request_id/transcript_request_id/parent_actor_transcript_request_id/parent_dispatch_tool_request_id/status`。`transcript_request_id`必须精确等于live `ActiveSubAgent.parentRequestId`并作为现有transcript lookup key；`parent_dispatch_tool_request_id`绑定父actor的exact tool-call edge。Main必须是`stage_run`下真实durable child actor并拥有相同identity fields，不能用synthetic `__main__`根补位。测试覆盖live、nested、completed-restored、identity conflict、missing Main、cross-project/org IDOR、mixed operation/stage execution/stage-run request、stale change seq、untrusted/latest selector、raw credential/action/corpus leakage与伪造Agent identity；缺失或冲突一律返回unavailable/fail closed。

endpoint ownership固定为：summary返回stage identity、Main/Analysis topology、source census、counts与server-owned`InvestigationControlProjectionV1`。control projection包含`stage_topology_contract/investigation_run_state/investigation_run_state_head/stop_epoch/stop_allowed+reason/reset-fork availability+reason/adoption_contract_version/control_policy_version`；UI禁止从stage order/current stage或change seq猜authority。hypothesis page返回revision/admission/task摘要；selected hypothesis detail返回该task的objective/Campaign/Subtask/Agent topology；timeline独立分页。禁止summary无界塞入全部Campaign，也禁止per-row N+1 detail。

backend RED tests必须证明active/terminal/abandoned与legacy/unified组合投影出唯一control availability；stop request只接受summary返回的exact run-state head，stale head fail closed；reset/fork availability不读取“latest”或本地stage order。

**Step 2：保留六个只读commands**

```text
investigation_get_summary
investigation_list_hypotheses
investigation_get_hypothesis
investigation_list_campaigns
investigation_get_campaign
investigation_list_timeline
```

不新增start mutation。所有read始终绑定`operation_id + stage_execution_id + stage_run_request_id`。首次summary/page bootstrap允许typed `expected_change_seq=None`，并在同一REPEATABLE READ READ ONLY snapshot返回head/cutoff/epoch与control projection；此后detail、pagination、refresh必须带`Some(exact change_seq) + cutoff/epoch`，分页再带cursor，unit/org/entity detail再加exact identity。actor/project/org由trusted principal和DB ownership解析；`None`不能用于detail，`0`或“latest Investigation”不能替代bootstrap contract。

**Step 3：实现独立stage-level stop control**

新增`investigation_request_stop`，它与hypothesis row无关。request包含exact operation/stage execution/stage-run request、`expected_investigation_run_state_head`、`expected_change_seq`、idempotency key；前端不传work列表。后端在一个事务内锁定并CAS stage run head、递增stop epoch、写stop intent、关闭新admission，并从DB冻结`analysis/read sessions + query planner/enrichment request/outbox + verification task + PentAGI subtask/worker request + Campaign + Prepared Action + FactDelta/consolidation`全量open-work exact set/hash作为stop denominator。所有异步writer必须提交同一stop epoch/fence，之后只能写cancel/drain/recovery terminal event；只有每类冻结集合全部闭合后才写closure。重复request返回同一receipt。JIT继续使用Campaign/action-local CAS，不能复用stage head。UI later只允许明确命名的stage stop控件调用。

**Step 4：实现commit-after-projection refresh event producer**

新增AI event kind `investigation_projection_changed`，payload严格为`operation_id + stage_execution_id + stage_run_request_id + change_seq`。projection/outbox bridge只在projection batch commit后emit，`change_seq`取同一committed batch receipt/head；event重放幂等，不能先emit后commit。事件只是refresh hint，missed event/cold restore仍从DB bootstrap。`ai/mod.rs`注册并re-export supervisor；`AppState`持有lifecycle handle；`app/bootstrap.rs`在DB-ready后启动outbox cold replay/live claim；`app/window_lifecycle.rs`在进程退出前drain/shutdown。producer unit tests覆盖commit顺序、duplicate、out-of-order、missed-event recovery、cold restore与foreign identity；`golish/tests/investigation_projection_event_wiring.rs`必须通过production composition证明projection commit后frontend event channel恰收到一次exact committed seq。

**Step 5：generated IPC PAUSE后生成类型**

按`docs/development.md`完成函数→facade→registry→frontend wrapper→ts-rs生成，并让`GeneratedAiEvent`包含新variant。生成后禁止手改`frontend/lib/generated/`。

**Step 6：验证**

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-app --test investigation_ipc_authorization --test investigation_read_model --test investigation_stop_control --test investigation_projection_events --status-level fail
cd backend && just space-guard && cargo nextest run -p golish --test investigation_projection_event_wiring --status-level fail
pnpm exec vitest run frontend/lib/api/investigation.test.ts
pnpm typecheck
```

Expected：typed no-seq bootstrap后所有reads绑定exact head，bounded ownership、control projection、IDOR、redaction、live/nested/restored/Main Agent identity、production-wired commit-after-projection event、schema version、stage-level stop CAS/full open-work denominator/stop-epoch drain/replay与类型链全绿；无start command，stop mutation不能由hypothesis click触发。

### Future Commit

```bash
git add backend/crates/golish-core/src/investigation_projection.rs backend/crates/golish-core/src/events backend/crates/golish-db/src/repo/investigation_projection backend/crates/golish-agent-app/src/ai/commands/investigation backend/crates/golish-agent-app/src/ai/investigation_projection_event_bridge.rs backend/crates/golish-agent-app/src/ai/mod.rs backend/crates/golish-agent-app/tests/investigation_ipc_authorization.rs backend/crates/golish-agent-app/tests/investigation_read_model.rs backend/crates/golish-agent-app/tests/investigation_stop_control.rs backend/crates/golish-agent-app/tests/investigation_projection_events.rs backend/crates/golish/src/state/mod.rs backend/crates/golish/src/app/bootstrap.rs backend/crates/golish/src/app/window_lifecycle.rs backend/crates/golish/tests/investigation_projection_event_wiring.rs backend/crates/golish/src/commands_facade/investigation.rs backend/crates/golish/src/commands_facade/mod.rs backend/crates/golish/src/commands_registry.rs frontend/lib/api/investigation.ts frontend/lib/api/investigation.test.ts frontend/lib/generated
git commit -m "feat(investigation): expose unified read projection"
```

## Task 9：把Investigation投影嵌入现有全屏Agent工作区

**Files:** 使用“现有全屏 Workspace”列出的全部文件。

**Step 1：写UI RED tests**

```tsx
it("routes one investigation stage_run directly to the full-pane workspace", async () => {});
it("routes live-only and persisted investigation rows through the same exact workspace", async () => {});
it("fails closed when args rows operation stage or request identities disagree", async () => {});
it("shows Main, analysis tasks, hypotheses and the real PentAGI Primary to nested-worker tree", async () => {});
it("clicking a hypothesis only changes local focus and performs no mutation", async () => {});
it("deep-links an exact specialist transcript under the owning investigation stage_run", async () => {});
it("renders live nested and completed-restored transcript identities and never synthesizes Main", async () => {});
it("shows unavailable for missing Main or conflicting transcript identity", async () => {});
it("keeps an agent hypothesis or campaign selection stable across monotonic refresh", async () => {});
it("bootstraps summary with no expected seq then binds every detail refresh to returned head", async () => {});
it("renders methodology as signal citations and evidence separately", async () => {});
it("renders queued, starting, jit, blocked and terminal states without fake agents", async () => {});
it("uses buttons aria-current status live regions alerts and preserves keyboard focus", async () => {});
it("jit approve deny and stage stop are explicit controls unrelated to hypothesis selection", async () => {});
it("submits the exact server-projected investigation run head when stopping", async () => {});
it("derives reset fork and stop availability only from the typed control projection", async () => {});
it("passes exact operation and campaign to the existing prepared-action panel", async () => {});
it("uses server-projected policy to hide or disable jit", async () => {});
it("unified roadmap has no Candidate or Verification continuation CTA", async () => {});
it("legacy roadmap keeps Candidate to Verification continuation", async () => {});
it("investigation stop and StageResetMenu never call each other", async () => {});
```

用mock只断言“点击hypothesis前后mutation call count恒为0”；不能断言整个Workspace只有read commands，因为明确的JIT Approve/Deny与stage stop控件合法拥有各自mutation。测试必须证明这些按钮和hypothesis row不是同一click target。

**Step 2：实现direct route**

先抽generic exact stage resolver，不假设当前已有通用`stageKey`。`ToolCallDetailView`的两条真实分支都必须处理Investigation：1) args/result/live rows存在但stage execution尚未投影；2) execution已落地。identity一致时直接渲染`InvestigationWorkspaceView`；只有live actor时先显示真实Agent且不猜DB latest。任何operation/stage/request冲突fail closed。legacy Candidate/Verification继续走`CandidateVerificationWorkspaceView`。不修改`PaneLeaf`增加新mode，不创建`investigation-workspace` selector/store route。

**Step 3：复用现有Workspace primitives**

把`StageTeamWorkspaceView`内部私有Agent rail、conversation、plan、tool/artifact与一次性deep-link focus抽到`AgentWorkspacePrimitives.tsx`，或改造成可注入typed navigation tree的presentational primitive；不得嵌套第二层`StageRunDetailShell`。左栏顺序为真实Main coordinator→per-org bounded read sessions→Analysis Task Primary/ordered Subtasks/Workers→Hypotheses→Verification Task Primary→ordered Subtasks→dynamic/nested Workers；每个task只显示一个Primary。点击Agent/session以`transcript_request_id`显示exact partition transcript，点击hypothesis显示详情和其已经自动运行的团队。Main identity缺失显示unavailable，不生成`__main__`。shell side-rail aria label可配置。

**Step 4：三态与事件刷新**

组件本地保存discriminated selection：`{kind:"agent",agentRequestId} | {kind:"hypothesis",revisionId} | {kind:"campaign",campaignId}`。ChatPanel specialist deep-link只应用一次，后到Agent或refresh不抢焦点。

新增`InvestigationRefreshHint`与monotonic setter，并通过`frontend/store/types/index.ts`、`frontend/store/store-types.ts`公开；消费Task 8生成的`investigation_projection_changed`，payload为`operationId + stageExecutionId + stageRunRequestId + changeSeq`。duplicate/out-of-order忽略，gap触发bootstrap，foreign operation/request拒绝；event永远不是authority。ChatPanel保持mounted；AppShell integration test证明详情打开时event仍投影，missed event/cold restore从DB恢复。

selected hypothesis/Campaign detail把exact `operation_id + campaign_id`交给Plan C现有`PendingPreparedActionPanel`与既有Prepared Action read/decision API wrapper，不另造假按钮或第二套JIT mutation。JIT button与hypothesis row必须是不同DOM target；rollout/policy不允许JIT时，使用server-projected policy隐藏或禁用。

**Step 5：移除冲突入口**

- 不创建Plan B `HypothesisRegistryAudit.tsx`；
- 不创建Plan D `investigation-workspace` DetailViewMode/Pane route；
- unified topology roadmap只显示Investigation，不显示Candidate/Verification continuation CTA；legacy topology仍显示Candidate→Verification及原continuation；
- UI不显示“Start verification/开始验证”按钮；
- hypothesis click无Tauri mutation。

active unified Investigation下禁用`StageResetMenu`的developer reset/successor fork，正常终止只调用`investigation_request_stop`。仅terminal/abandoned run允许topology-aware successor fork，并要求adoption receipt；菜单文案不得硬编码Attack Candidate。legacy reset/fork行为不变；stop handler与reset/fork helper不得互相调用。

上述availability、frozen topology、run state、exact run-state head与adoption version全部读取Task 8的`InvestigationControlProjectionV1`；UI不得从`stageOrder/currentStage`、本地status或change seq推断。stop mutation必须原样提交projection给出的exact run-state head并让server CAS；stale返回typed error后bootstrap，不做latest重试。

所有hypothesis/Agent/control使用真实button；selection用`aria-current`/`aria-pressed`，变化用`role=status`/`aria-live=polite`，identity/read error用`role=alert`，refresh后保留DOM focus。

**Step 6：验证**

```bash
pnpm exec vitest run frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/AIChatPanel/SubAgentInlineCard.test.tsx frontend/components/AIChatPanel/AIChatPanel.stage-fork.test.tsx frontend/components/AIChatPanel/StageProgressBar.test.tsx frontend/components/AIChatPanel/StageMarker.test.ts frontend/components/AIChatPanel/StageResetMenu.test.ts frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx frontend/components/Engagement/StageRunDetailShell.test.tsx frontend/components/Engagement/CandidateVerificationWorkspaceView.test.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/services/ai-events/harness-handlers.test.ts frontend/store/investigation-workspace.test.ts frontend/lib/stage-reset.test.ts frontend/App/detailFocus.test.ts frontend/App/AppShell.detail-focus.test.tsx
pnpm exec biome check frontend/components/Engagement/AgentWorkspacePrimitives.tsx frontend/components/Engagement/InvestigationWorkspaceView.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageRunDetailShell.tsx frontend/components/Engagement/CandidateVerificationWorkspaceView.tsx frontend/components/Engagement/PendingPreparedActionPanel.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx frontend/components/AIChatPanel/StageMarker.tsx frontend/components/AIChatPanel/StageProgressBar.tsx frontend/components/AIChatPanel/StageResetMenu.tsx frontend/components/AIChatPanel/AIChatPanel.tsx frontend/App/AppShell.tsx frontend/lib/stage-reset.ts frontend/store/types/session.ts frontend/store/types/index.ts frontend/store/store-types.ts frontend/store/slices/session.ts frontend/store/slices/session-core.ts frontend/store/investigation-workspace.test.ts frontend/services/ai-events/harness-handlers.ts frontend/services/ai-events/harness-handlers.test.ts
pnpm typecheck
```

Expected：focused tests、Biome、typecheck全绿；只存在一个生产full-pane route。

### Future Commit

```bash
git add frontend/components/Engagement/AgentWorkspacePrimitives.tsx frontend/components/Engagement/InvestigationWorkspaceView.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageRunDetailShell.tsx frontend/components/Engagement/CandidateVerificationWorkspaceView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx frontend/components/AIChatPanel frontend/App/AppShell.tsx frontend/App/AppShell.detail-focus.test.tsx frontend/lib/stage-reset.ts frontend/store frontend/services/ai-events/harness-handlers.ts frontend/services/ai-events/harness-handlers.test.ts
git commit -m "feat(frontend): show unified investigation agents"
```

## Task 10：完成Gate、Reporting与operation-frozen rollout

**Files:** 使用Plan C revision adjudication、Plan D D1/D3/D4、stage gate/profile/fork/reporting相关文件。

**Step 1：写closure RED tests**

```rust
#[test]
fn investigation_cannot_close_with_active_action_unknown_execution_or_unconsumed_delta() {}

#[test]
fn every_current_hypothesis_has_a_sealed_admission_disposition() {}

#[test]
fn every_scheduled_task_has_complete_objective_and_delegation_censuses() {}

#[test]
fn objective_outcomes_close_campaign_assignments_without_mutating_the_denominator() {}

#[test]
fn zero_campaign_closes_with_a_sealed_empty_outcome_set() {}

#[test]
fn primary_only_or_missing_worker_task_cannot_close() {}

#[test]
fn exhausted_fuel_or_repeated_semantic_cycle_becomes_residual_and_stops_admission() {}

#[test]
fn stopped_or_unsupported_hypotheses_become_residual_not_refutation() {}

#[test]
fn zero_hypothesis_requires_complete_analysis_and_methodology_census() {}

#[test]
fn stop_intent_drains_unknown_execution_before_closure() {}

#[test]
fn stage_stop_head_and_server_frozen_active_set_prevent_concurrent_admission_crossing() {}

#[test]
fn stage_stop_denominator_drains_analysis_enrichment_workers_actions_and_fact_delta() {}

#[test]
fn frozen_topology_selects_exactly_one_legacy_or_unified_report_source() {}
```

**Step 2：实现RunClosure与Gate**

`InvestigationRunClosureV1`绑定current snapshot/generation、admission set、investigation run state head/stop epoch、task state-head exact set、objective assignment与Campaign-only outcome census、delegation census、Campaign terminal set、stop-frozen analysis/read-session/query/enrichment/task/subtask/worker/Campaign/action/FactDelta exact set、FactDelta watermark、fuel reservation/consume/refund/unknown-held与cycle census、fixed-point receipt及residual set。PASS/PASS_WITH_GAPS由deterministic Gate决定，模型不能声明完成。scheduled task漏assignment、Campaign assignment漏outcome或non-Campaign出现outcome、每task多Primary、Primary未委派独立worker、delegation census未seal、任一stop-frozen work仍在drain、unknown execution/fuel-held、可运行work未消费、dispatch重复或fuel账不闭合均BLOCK。zero-Campaign必须有完整terminal assignment receipts与sealed-empty outcome set。

**Step 3：Reporting读取新stage authority**

Reporting同时支持legacy Verification handoff与unified Investigation handoff，但按operation-frozen`stage_topology_contract`只能选一个。方法论/RAG refs只能进入Methodology/Limitations，不进入Finding proof source。PentAGI Agent prose/transcript只作audit narration，typed tool/evidence/oracle lineage才可成为authority。

**Step 4：rollout**

rollout legal pair逐项固定：

| rollout mode | new operation topology | execution |
|---|---|---|
| `legacy_only` | `legacy_candidate_verification_v1` | legacy authority |
| `shadow_registry` | `legacy_candidate_verification_v1` | legacy authority + shadow projection |
| `dual_read_compare` | `legacy_candidate_verification_v1` | legacy authority + complete-record compare |
| `registry_authoritative_legacy_projection` | `unified_investigation_v1` | unified authority + legacy compatibility projection |
| `new_only` | `unified_investigation_v1` | unified authority only；history仍可读 |

validator拒绝其它新组合；已经冻结的历史组合只按grandfathered receipt恢复，不能被当前默认改写。promotion receipt必须证明profile/graph/read model/report、PentAGI task identity、legacy replay与whole-record compatibility。fork只有带exact adoption receipt才可采用新pair，既有operation与history不原地切换。

**Step 5：验证**

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-kit -p golish-agent-app -p golish-reporting-domain -p golish-reporting-app -E 'test(investigation_gate_) | test(investigation_reporting_) | test(investigation_rollout_)' --status-level fail
cd backend && just space-guard && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-reporting-domain -p golish-reporting-app --all-targets -- -D warnings
```

Expected：legacy/new双路径、Gate、residual、report source与promotion tests全绿；没有真实target/provider请求。

### Future Commit

```bash
git add backend/crates/golish-agent-kit backend/crates/golish-agent-app backend/crates/golish-reporting-domain backend/crates/golish-reporting-app resources/harness
git commit -m "feat(investigation): gate and report unified runs"
```

## Task 11：更新模块卡、状态与定向完成证据

**Files:**

- 修改 `docs/modules/backend/golish-agent-kit/harness.md`
- 修改 `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- 修改 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改 `docs/modules/backend/golish-agent-app/ai.md`
- 修改 `docs/modules/backend/golish-agent-bridge.md`
- 修改 `docs/modules/backend/golish-agent-bridge/agent_bridge.md`
- 修改 `docs/modules/backend/golish-core.md`
- 修改 `docs/modules/backend/golish-db.md`
- 修改 `docs/modules/backend/golish-db/repo.md`
- 修改 `docs/modules/backend/golish-events.md`
- 修改 `docs/modules/backend/golish-events/transcript.md`
- 修改 `docs/modules/backend/golish-memory-app.md`
- 修改 `docs/modules/backend/golish-skills.md`
- 修改 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改 `docs/modules/backend/golish-reporting-domain.md`
- 修改 `docs/modules/backend/golish-sub-agents.md`
- 修改 `docs/modules/backend/golish-sub-agents/defaults.md`
- 修改 `docs/modules/backend/golish-sub-agents/executor.md`
- 修改 `docs/modules/backend/golish-tools/definitions.md`
- 修改 `docs/modules/backend/golish.md`
- 修改 `docs/modules/backend/golish/app.md`
- 修改 `docs/modules/backend/golish/state.md`
- 修改 `docs/modules/frontend/components.md`
- 修改 `docs/modules/frontend/lib.md`
- 修改 `docs/modules/frontend/services.md`
- 修改 `docs/modules/frontend/store.md`
- 修改 `docs/modules/INDEX.md`
- 修改 `feature_list.json`
- 修改 `agent-progress.md`

**Step 1：同步system-of-record**

模块卡只写真实落地职责/接口，不把计划中的未实现项提前标为current。索引状态与卡片一致。

**Step 2：执行最终focused验证**

```bash
jq empty feature_list.json resources/harness/graph/operation_graph.json resources/harness/graph/operation_graph_application_model_v1.json resources/harness/graph/operation_graph_unified_investigation_v1.json resources/harness/graph/phases.json resources/harness/profiles/pentest.json resources/harness/profiles/red_team.json resources/harness/profiles/smoke.json resources/harness/profiles/assessment.json resources/harness/profiles/bug_bounty.json resources/harness/profiles/cloud_assessment.json resources/harness/stages/application_understanding/spec.json resources/harness/stages/investigation/spec.json
cd backend && just space-guard && cargo nextest run -p golish-core -p golish-skills -p golish-memory-app -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app -p golish-reporting-domain -p golish-reporting-app -E 'test(methodology_) | test(investigation_) | test(pentagi_analysis_) | test(verification_admission_) | test(hypothesis_verification_task_)' --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-db --test operation_stage_forks --test hypothesis_registry --test verification_campaigns --status-level fail
cd backend && just space-guard && cargo nextest run -p golish-agent-app --test methodology_enrichment --test investigation_ipc_authorization --test investigation_read_model --test investigation_stop_control --test investigation_projection_events --status-level fail
cd backend && just space-guard && cargo nextest run -p golish --test investigation_projection_event_wiring --status-level fail
pnpm exec vitest run frontend/components/AIChatPanel/ToolCallSummary.test.ts frontend/components/AIChatPanel/SubAgentInlineCard.test.tsx frontend/components/AIChatPanel/AIChatPanel.stage-fork.test.tsx frontend/components/AIChatPanel/StageProgressBar.test.tsx frontend/components/AIChatPanel/StageMarker.test.ts frontend/components/AIChatPanel/StageResetMenu.test.ts frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx frontend/components/Engagement/StageRunDetailShell.test.tsx frontend/components/Engagement/CandidateVerificationWorkspaceView.test.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/services/ai-events/harness-handlers.test.ts frontend/store/investigation-workspace.test.ts frontend/lib/stage-reset.test.ts frontend/App/detailFocus.test.ts frontend/App/AppShell.detail-focus.test.tsx frontend/lib/api/investigation.test.ts
pnpm exec biome check frontend/components/Engagement/AgentWorkspacePrimitives.tsx frontend/components/Engagement/InvestigationWorkspaceView.tsx frontend/components/Engagement/InvestigationWorkspaceView.test.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageRunDetailShell.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx frontend/components/AIChatPanel frontend/App/AppShell.tsx frontend/lib/api/investigation.ts frontend/lib/stage-reset.ts frontend/store frontend/services/ai-events/harness-handlers.ts frontend/services/ai-events/harness-handlers.test.ts
pnpm typecheck
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json
```

Expected：全部exit 0；每个nextest selector实际匹配测试。未获授权仍不运行init/precommit/full workspace门禁。

**Step 3：记录evidence并更新feature状态**

只有在Plan A、B、C及本delta所需行为均有新鲜evidence、schema/IPC授权已履行、legacy/new路径测试全绿后，才能把相应feature改为`passing`。Plan D的D3/D4未完成时不得因UI完成而把Plan D标为passing。

**Step 4：精确diff检查**

```bash
git diff --check -- backend/crates/golish-core backend/crates/golish-skills backend/crates/golish-memory-domain backend/crates/golish-memory-app backend/crates/golish-agent-kit backend/crates/golish-agent-runtime backend/crates/golish-sub-agents backend/crates/golish-agent-app backend/crates/golish-agent-bridge backend/crates/golish-events backend/crates/golish-pentest-app backend/crates/golish-db backend/crates/golish-reporting-domain backend/crates/golish-reporting-app backend/crates/golish-tools backend/crates/golish resources/harness frontend/components/AIChatPanel frontend/components/Engagement frontend/components/ToolCallDetailView frontend/App frontend/lib frontend/store frontend/services/ai-events docs/modules docs/design/2026-08-02-rag-first-unified-investigation-stage.md docs/superpowers/plans/2026-08-02-rag-first-unified-investigation-stage.md feature_list.json agent-progress.md
```

Expected：exit 0。共享dirty tree的无关改动不纳入完成声明。

### Future Commit

```bash
git add docs/modules docs/design/2026-08-02-rag-first-unified-investigation-stage.md docs/superpowers/plans/2026-08-02-rag-first-unified-investigation-stage.md feature_list.json agent-progress.md
git commit -m "docs(investigation): record unified stage evidence"
```

---

## 完成判定

以下条件必须同时成立：

1. frozen unified topology只走`vuln_triage → application_understanding → investigation`，新operation只有一个Investigation stage/run identity，legacy仍按原两阶段恢复；
2. baseline→query planner→immutable final snapshot可重放，exact-scope canonical facts、RAG/KG、AU、checked Tool Truth、methodology与enrichment provenance均有hash/census；多个organization各有独立stage_run_unit/snapshot/read session/transcript/budget/action authority，crash/resume后仍隔离；同org多个asset按application/trust-boundary shard组织并seal exact members，未分片资产形成typed residual；
3. methodology/RAG只能产生signal/strategy，target/corpus prompt injection不能改role/tool/scope；generic Investigation Agent不能写Finding；
4. Hypothesis Analysis复用现有PentAGI Generator→每task唯一Primary→dynamic/nested worker→Refiner/Reflector，并自动seal canonical hypotheses；TaskOrchestrator是唯一dispatch writer；hypothesis identity绑定organization与exact affected-target set，单asset evidence不会关闭兄弟asset obligation，相同基础设施不会合并跨org roots；
5. sealed generation在零UI/零click下自动创建和drain幂等verification tasks；全历史stable key、semantic evidence hash、rerun receipt、tagged task-run subject与logical dispatch receipt/fenced retry阻止跨generation/crash/replay重复攻击；
6. 每个task完整覆盖immutable objective assignment exact set；outcome exact set只覆盖Campaign assignments并集合相等，zero-Campaign seal empty，每个runnable subtask有独立worker与durable delegation census；Primary不能单Agentterminal，Plan C required-control denominator不被替代；
7. hypothesis点击只改变`agent|hypothesis|campaign`查看焦点，零mutation；复用的JIT panel、stage stop与topology-aware reset/fork是彼此独立控件；
8. Campaign、Prepared Action/JIT、one-action Operator、oracle、FactDelta、successor snapshot/generation在同一stage内闭环；cognitive worker永远拿不到raw executor工具，原子fuel ledger、durable-begin consume、unknown-held no-replay与semantic-cycle guard阻止超卖/重复副作用/无限执行；
9. stage-level stop denominator中的analysis/read session/query/enrichment/task/subtask/worker/Campaign/action/FactDelta任一未drain、active/unknown action、未消费delta、objective/delegation/admission不闭合、重复dispatch、fuel账不闭合或可运行work遗漏会阻止Gate；
10. typed no-seq bootstrap→exact-head reads、server-owned control projection、exact Main/live/nested/restored transcript identity、production lifecycle-wired commit-after-projection event、full-screen live/persisted route、loading/error/empty/stale/recovery/accessibility、unified/legacy roadmap与adapter测试全绿；
11. focused验证命令、退出码与关键输出已写入`agent-progress.md`和feature evidence。
12. Tool Manager从47份扩为57份可解析配置，新增10项与Investigation admission catalog exact-set一致；工具存在不自动授权，active/stateful/OAST成员按JIT或disabled治理，缺typed adapter/Tool Truth时不能产生Finding或terminal coverage。

任一条件缺失时保持`in_progress`或转`blocked`，不得用“Agent已经想过/跑过”代替evidence。
