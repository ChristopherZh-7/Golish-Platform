# golish-db

> **一句话职责**：Golish 的 PostgreSQL 持久化层——嵌入式 PG（pg_embed 自动下载+生命周期）+ pgvector 语义检索 + session→task→subtask→tool_call 层级 + pentest 数据 + token 用量分析。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-db/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 任何 DB 读写、加 repo CRUD、改 schema/migration、向量记忆检索时
- DB 启动失败、连接池、事务问题时
- ⚠️ 改 schema/migration 是 **AGENTS.md §2.7 高风险操作，必须先问用户**

## 职责

- C4 Post-Exploit canonical spine：`foothold_candidates`、`footholds`、`internal_asset_observations`、`attack_paths/edges`、`post_exploit_actions/approvals`、`objective_attempts`；全部绑定 frozen operation/project/org-at-time 与 evidence。
- Foothold / ObjectiveOutcome terminal row 与 `PostExploitFactTerminal.v1` outbox deliveries 同事务提交；P6a 数据库 trigger 拒绝任何非 `none` side effect。
- 四个信息阶段（Target Intel / EAS / Enumeration / Vuln Triage）的 final seal 在同一 caller-owned compound transaction 内关闭 immutable `StageEpisode` 并写 `StageEpisodeClosed.v1` outbox/catalog deliveries；Memory 写失败会连同 Unit/Worker/Handoff/completion 一起回滚。
- Candidate V2 terminalizer 在同一 caller-owned transaction 内终结 Attempt/Candidate、创建 verified Finding/lineage、提出 FactDelta、释放 lane/Worker，并写 deterministic `CandidateAttemptTerminal.v1` + catalog deliveries；outbox 任一写失败会把全部 canonical terminal state 回滚，exact response-loss replay 不重复事件或 delivery。
- Candidate V2 fuel 由 generation-0 canonical policy 与全 operation 的 `E=effective_attempt_fuel`、`R=retryable_backlog` 共同约束：review 批次预留首个 Attempt，claim 只消费真实可用槽，release 在 `R >= max_attempts_total-E` 时必须 terminalize 为 evidence-backed blocked residual，避免新 retry 占掉既有 retry 的最后槽；fuel-terminal replay 会重算完整 residual/fence，live target 删除只允许 nullable pointer 归零。
- Candidate V2 的 operation-wide Wave authority 与 consolidation 都由 DB 持有：初始 authority 只来自 generation-0 final-sealed `vuln_triage` handoff；后续 authority 只来自 immutable FactDelta consolidation。Verification 全部 Unit 达到 exact terminal truth 后，单一短事务原子记录 `opened_next_wave|closed_no_delta|exhausted`，接受/拒绝 FactDelta、保留 residual risk，并关闭 source Wave 或创建完整 frozen-org 的下一代 Wave。
- V2 Verification close 在同一短事务关闭 logical primary Worker/StageRunUnit、CAS WaveUnit ready，并从 durable Candidate/Attempt/Finding/FactDelta/no-candidate/evidence truth 写 deterministic `verification_stage_handoffs`；DB deferred authority 会逐项重算 typed claims、顺序、hash、coverage、terminal receipt/outbox/delivery bundle，缺行或 raw 漂移都不能成为 consolidation 输入。
- C7 `knowledge_context` 只暴露 scope-first read model：先锁定 operation/frozen snapshot/org/classification/validity，再排序或做向量相似度；没有 project-null/org-null/legacy global fallback。
- C5/C8 Cleanup ledger：side-effect action 与 cleanup obligation 通过 deferred exact back-reference 原子绑定；terminal obligation 必须在 deferred commit boundary 具备 exact attempt/absence/waiver/blocked decision 与独立 evidence 关系，Gate 与组织删除 precheck 同时重算该 relational truth，不能只信 status 字符串。`CleanupObligationTerminal.v1` 读取的 obligation/attempt/absence/waiver/blocked-decision evidence membership 与 parent terminal transition 通过行锁串行化：同一事务可先写 child 再 terminalize，terminal commit 后五类 child 均禁止新增，既有 membership 继续禁止 UPDATE/DELETE。CleanupAttempt 只允许一次 live→`verified_absent|verification_failed|execution_failed`；OLD 已 terminal 后整行 UPDATE（含 no-op）/DELETE 均拒绝，失败重试必须创建下一 ordinal 的新 Attempt。
- side-effect action+obligation compound transaction 在 exact evidence 后从 persisted rows 生成 `PostExploitActionPrepared.v1` + mandatory deliveries；payload 不含 raw plan/secret。

提供嵌入式 Postgres 的启动与连接池，以及结构化数据访问。owns `graph_knowledge_base` 等 migration（golish-graphiti 的表也在此）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `GolishDb::start(DbConfig)` / `.pool()` / `.stop()` | DB 句柄（持有嵌入式 PG + 池） |
| `create_lazy_pool` / `PgPool`(re-export sqlx) | 连接池 |
| `repo::*`（sessions / tool_calls / memories / audit …） | 各表 CRUD（scoped） |
| Runtime Memory rollout repos | frozen four-rank contract、Worker admission、whole-record shadow sample、DB-generated promotion receipt 与 typed post-commit reconcile |
| Candidate V2 repos | operation-frozen rollout + admission cutoff/DB-generated promotion receipt、`Initial|Current|Terminal` Wave authority、manifest/plan-bound approval、Attempt+WorkerRun+lane compound lease、typed Verification handoff/terminal receipt、FactDelta consolidation、verified Finding lineage |
| Candidate/Post-Exploit hash bridge | CandidateAttempt authority 保留 canonical `sha256:<hex>`；进入既有 Post-Exploit Foothold seam 时只在 trusted repo/DB validator 边界比较同一 64-hex digest，不放宽 operation/org/target/Attempt 绑定 |
| Runtime Memory final seal | 四信息阶段的 Unit/Worker/Handoff/completion + `StageEpisode` + `StageEpisodeClosed.v1` deliveries 原子提交 |
| `repo::knowledge_context` | DB-owned ContextPack authorization snapshot 与 canonical/runtime/handoff/assertion/document/vector scoped queries |
| `DbConfig` / `DbError` / `models::*` | 配置/错误/数据模型 |
| `gatekeeper` / `embeddings` | 准入门 / 向量 |

## 依赖

- **内部**：`golish-core`、`golish-platform`

## 被谁依赖 / 改动影响面

`golish`、各 `*-app`、`golish-app-core`、`golish-graphiti`、`golish-integrations`、`golish-scan-runner`、`golish-pentest`、`golish-vuln-intel`。**改 schema 影响面极大**。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `repo/` | 各表 scoped CRUD helper | [→](golish-db/repo.md) |
| `models/` | DB 数据模型 | [→](golish-db/models.md) |
| `embedded/` | 嵌入式 PG 启动/生命周期 | [→](golish-db/embedded.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `config.rs` / `pool.rs` / `error.rs` | 配置 / 连接池与 fail-closed migration repair / 错误 |
| `gatekeeper.rs` | 准入门 |
| `embeddings.rs` | 向量嵌入 |
| `build.rs` | 让嵌入 migration 集变化触发 crate 重建，避免新 SQL 未被编入 binary |

## 注意事项 / 坑

- **C9 Reporting read model（2026-07-13）**：`20260712000011_reporting_read_model.sql` 新增 reports/revisions/sections/claims/citations/source-manifest/content-addressed blob refs。manifest/section/claim/citation 在 validation 进入 `validated|invalid` 后即对 INSERT/UPDATE/DELETE 全冻结，不等 publication；artifact child 可在 validated+unpublished 阶段 attach，但 final/superseded 后同样冻结。UPDATE 同时检查 OLD/NEW owner 防“搬到 draft”绕过；所有 child I/U/D 都对 parent 取 `FOR UPDATE`，与 draft→validated/finalize 的 parent row mutation 冲突，禁止验证后夹带 child。validated+unpublished revision 本体拒绝 attestation/source/revision metadata 的任何 UPDATE，只允许保持其余列 exact 的 publication transition；final→superseded 也只允许 `publication_status` 与 trigger-owned `row_version` 改变。`final` 不是裸 enum flip：deferred constraint 在 commit 时要求 exact current revision、active local principal、至少一个 content-addressed artifact ref，以及 row-version/operation/project/stream/payload exact 的 `ReportRevisionFinalized.v1` outbox；普通 SQL 只设 final 必须失败。被任一 revision 引用的 blob 拒绝所有 UPDATE；被 validated/invalid/final/superseded revision 作为 `stage_handoff` source 保留的 final seal也拒绝 invalidation/delete（`REPORT_SEALED_REF_RETAINED`）。终态 `candidate_attempts` / `cleanup_obligations` 是 deterministic outbox canonical source：只允许一次 nonterminal→terminal，之后 canonical at-time/event 字段、no-op UPDATE 与 DELETE 都以 `TERMINAL_CANONICAL_SOURCE_IMMUTABLE` 拒绝，保证 row-version/event replay identity 稳定。唯一例外是删除 live target 后由 FK 驱动的 `candidate_attempts.target_live_id non-NULL→NULL`；该字段不是 canonical snapshot 内容，其他整行必须 exact 不变且 `row_version` 不递增，目标仍存在时的直接清空仍被拒绝。finalize 必须完整重读 source set、exact compare、CAS，并与 artifact refs/outbox 同事务；RAG/KG 不参与 Gate。

- **不变量 I2**：所有 CRUD 验资源所有权（IDOR），含批量；repo 是 scoped CRUD。
- **不变量 I9**：事务内禁止外部 HTTP/MQ/长耗操作（连接池雪崩）。
- **不变量 I10**：改 schema 必须向后兼容（先扩字段→上新代码→清旧字段）。
- migration `20260710000001_technique_outcomes_org_scoped_unique.sql` 是明确的 downtime-only 例外：它把 `technique_outcomes` 唯一键改为 `(organization_id, run_id, asset, technique)`，并把 target-bound `directory_entries` 唯一键改为 `(target_id, url, tool)`。部署时必须先停止旧 writer，再将 migration 与新 binary 一起上线；迁移完成后旧 binary 与新约束不兼容，禁止继续运行或回切旧 binary。该迁移只重建约束/索引并保留业务 rows，不授权在线混跑新旧写入方。
- `20260712000001_runtime_memory_foundation.sql` 是已冻结的 additive runtime-memory expand migration：四态 rollout singleton 只能 `rank+1`，operation contract/project scope 不可漂移，scope snapshot 采用 `snapshot → units → sealed_at` 一次性冻结，tool/worker/submission/handoff 以 composite identity + row locks 防跨 operation 与 lease TOCTOU。其 SHA-384 由 integration test 固定；后续变化必须新建更高版本 migration，不能改写本文件。
- `20260712000003_trusted_operator_principal.sql` 提供 server-owned local operator UUID；Candidate approval 的 `decided_by` 只 FK 到该 principal，request/model 不能选择 actor。`20260712000004_attack_execution_v2.sql` 是 additive Candidate V2 ownership spine：attack rollout 只能 forward CAS，operation 创建时冻结 contract，final-pass handoff/完整 manifest、plan-bound approval、Attempt/WorkerRun/lane、relational evidence 和 Finding lineage 均由 DB composite FK/trigger 约束。
- `20260712000005_attack_execution_v2_cutover.sql` 只执行无需 sample 的 attack rank0→1；不再直接到 `v2_only`，且不改既有 `operation_state`。`20260712000013_attack_execution_shadow_reads.sql` 另建 Candidate whole-record legacy mirror/attestation 表：legacy snapshot 是严格 canonical shape/hash 的被审计输入；INSERT 必须 exact 绑定 terminal Unit、non-invalidated handoff/submission 与 frozen manifest，DB 独立重建 V2 并原子生成 comparison/source/selected hash/server chronology。所有会改变重建结果的 Candidate/no-candidate evidence membership 与 audit semantic source 随 final handoff 冻结，只有真实 target FK 清 live pointer。`20260712000016_attack_rollout_candidate_cohort.sql` 新增 gen0 WaveUnit admission sequence、promotion cutoff/receipt、terminal/late-sample guard 与 DB canonical Candidate rebuild：promotion 在 singleton row lock 内 left-join admission→Wave/Unit→shadow，raw adjacent UPDATE 也要求非空完整 cohort、零 mismatch/缺失，并由 trigger 生成后再重算 receipt；caller/持久化标签不能自报分母、hash 或 receipt。dual attack 同时禁止 runtime `legacy_v1`，attack `v2_only` 继续只允许 runtime `v2_only`。
- `20260712000017_runtime_memory_shadow_attestation.sql` 把 runtime rank 1/2 promotion 改成 DB-authoritative retained cohort：dual Worker 的 AFTER owner trigger只提交 Worker id，admission prepare连显式 sequence/owner tuple/contract/rank/version/time都从当前数据库重建；完整 legacy/V2 record、hash、comparison、selected source 同样由 trigger 从 relational truth重建。历史 mismatch/缺 sample/current drift 永久阻断对应 rank；public CAS 与 raw adjacent UPDATE 共用 gate，只有 transition 完成后的 AFTER owner trigger能请求 receipt，prepare再从 updated singleton+old cohort覆盖全部 cutoff/count/digest字段。runtime/attack singleton 与 operation frozen contract pair 使用同一 compatibility matrix。
- 同一 `20260712000017` 把 `V2Only` legacy checkpoint 禁令下沉到 DB：安装 preflight 拒绝既有污染，之后 BEFORE trigger 拒绝 `graph_flow`、flat HarnessResume、worker/handoff、`agent_run` 等 11 个 legacy namespace；EAS/server-owned sibling 仍可写，dev reset 可原子移除禁用 key。普通 blob writer 也带 contract predicate，不能绕过 repo helper 重新注入 legacy checkpoint。
- `20260712000012_attack_fact_delta_wave_entry.sql` 增加 typed FactDelta decision、Wave consolidation/member、residual risk 与 follow-on entry 约束：一个 WaveUnit entry 必须二选一为 initial `vuln_triage` handoff 或 follow-on consolidation；accepted delta 的 assertion + `FactDeltaAccepted.v1` + 四路 delivery 与 consolidation 同事务，零增量不伪造 memory event，fuel/depth/candidate/attempt cap 耗尽时写 retained residual risk。
- `20260712000015_attack_fuel_reservation.sql` 冻结 generation-0/follow-on Wave policy、Candidate→Approval→Attempt exact target/plan/budget/capability tuple与 operation-wide candidate/attempt hard cap。Candidate/Approval/Attempt/FindingLineage/decided WorkItem/action journal/support evidence 与四类 canonical fuel residual 均受 DB audit trigger 保护；除真实 `targets` 删除触发的 nullable live pointer 清空外，terminal/decision/provenance 行不能改写或删除。
- `20260712000014_verification_stage_handoffs.sql` 增加 server-authored typed Verification seal：handoff id 由 WaveUnit UUIDv5 确定，`gate_passed_at` 强制取 DB 时间，payload 与每种 typed claim 都是拒绝额外键的 closed schema，claims/evidence/coverage/hash 只从 exact DB truth 投影；immediate + deferred authority 双栅栏分别拒绝 direct/nested pre-seed 与同事务后续 owner 漂移，V2-only primary Worker PASS 与 WaveUnit ready 必须在同一 commit 拥有 exact handoff。seal 会重新核对每个已引用 evidence 的 live-target owner，并对 FactDelta evidence 重新核对 source Attempt 时间窗，随后冻结 evidence identity、no-candidate decision evidence 与 FactDelta evidence membership（未引用且仍处 consolidation 前窗口的 FactDelta 不受误伤）；真实 live-target FK 删除只清 nullable pointer。每个 terminal Attempt 还必须具备 canonical result/action journal、Finding/FactDelta projection、deterministic `CandidateAttemptTerminal.v1` 与四路 projector delivery 的完整 receipt bundle；fuel-blocked 分支额外重算 retained residual truth。
- `20260712000018_candidate_post_exploit_hash_bridge.sql` 是 additive 兼容桥：Candidate V2 继续持久化 tagged `sha256:<64hex>`，既有 Post-Exploit `footholds.source_plan_hash` 继续使用 bare 64-hex；Foothold repo 只对来自 exact CandidateAttempt 的 trusted plan hash 去标签，DB authority validator 同时接受 `bare` 或 `sha256:bare` 的等价比较。该桥不接受任意算法/长度，也不放宽 operation、organization、target、terminal Attempt 或 evidence lineage 校验。
- `20260712000009_cleanup_obligation_ledger.sql` 在不开放任何 Tool 的前提下安装 P7a kernel：非 `none` action 必须在同事务带 active server principal 与 exact obligation back-reference；三态 live Attempt partial unique、independent absence evidence、trusted waiver residual/CAS 与 retained history 均由 DB 纵深约束。
- `20260712000010_cleanup_closeout.sql` 安装 exact terminal-truth deferred constraint、Cleanup evidence/decision immutability 与两阶段组织删除。Deletion job 只冻结 active server-owned `project_scopes.canonical_project_path`，锁后重读 subtree 防并发扩张，active subtree 禁止外部 reparent；artifact/hard-delete 使用独立 durable backoff deadline，防失败 hot-loop 与队列饥饿。
- Candidate review barrier 是 durable wake authority：review close 写 `resume_pending`，resume command CAS `dispatching`，成功后写 `resumed`；DB startup reaper 会把超时 dispatch 重置为 pending，同时保留所有 plan-bound approval/rejection 行。
- `pool.rs::repair_migrations` 只允许按 **version + description + exact old/new SHA-384** 明确登记的 migration-specific checksum repair，并用旧 checksum 做 compare-and-swap；当前 allowlist 为空。仅版本/description 匹配不足以修复，任何未登记 drift 必须 fail closed，尤其不得把扩写过的 runtime foundation 误标成已完整执行。missing migration 留给下一次 SQLx migrator 真正执行 SQL，dirty/failed 与 phantom history 同样保持失败。
- `repo/organizations.rs::subtree` 返回 root organization + descendants 的完整行，并按 root first / child sort order 排序；runtime `stage_run` 用它作为 continuation fan-out scope truth，防止模型续跑少传子公司导致资产分母缺口。
- `repo/tool_calls.rs::scoping_actions_for_session` 是 red-team scoping anti-shortcut gate 的 DB 审计口径：必须能识别真实 `unit_review`，并把 `manage_organizations(create_batch)` 的 `created`/`existing` id 当作已记录组织；但 REUSE 模式不应强迫创建。
- 相关设计：`docs/database-and-tools.md`、`docs/superpowers/plans/2026-05-30-p1-1-golish-db-scoped-crud-helper.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-db
```
