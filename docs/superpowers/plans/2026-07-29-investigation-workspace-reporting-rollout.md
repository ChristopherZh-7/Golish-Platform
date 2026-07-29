# Investigation Workspace、Canonical Reporting 与 Rollout 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 为每个 operation 提供可恢复、可分页、版本一致的 Investigation Workspace；把报告严格建立在分级 canonical authority 与脱敏投影上；再用 operation-frozen 的五态 rollout、whole-record comparison 和数据库证明的 promotion 安全切换新建 operation 的默认 contract。

**架构：** Plan B 的 Hypothesis Registry 和 Plan C 的 Verification Campaign 继续拥有写 authority 与 Gate；本计划新增的 read model 只在 `REPEATABLE READ READ ONLY` snapshot 内组合它们，并通过固定为 `1` 的 `projection_schema_version` 与单调 `change_seq` 驱动 UI。单个 Campaign 只提供 objective-local outcome/audit；只有 Plan C host-sealed revision-level adjudication/terminal decision 在重验 Plan B verification plan、proof paths、claim components 与 exact latest objective outcome set 后，才可成为新链的 `SecurityVerdict` authority。Reporting 采用 typed authority class、deterministic redacted projection、declared coverage denominator、Plan A `AllFreshToolTruthAuthorityBundle` 和 open→members→seal 的 report-input contract，禁止 raw witness 借 DTO/export 泄漏。Rollout 只消费 Plan B 的唯一 policy，并由联合 Tool Truth + Investigation default、immutable operation receipt、whole-record compatibility compare、versioned adversarial acceptance corpus 与锁内 promotion receipt落库；既有 operation 永不原地切 mode。

**技术栈：** Rust 2021、Tokio、sqlx/PostgreSQL、Tauri 2、ts-rs、React 19、TypeScript 6、Zustand、TanStack Virtual、Vitest、Biome。

**设计依据：** `docs/design/2026-07-29-tool-truth-hypothesis-verification-loop.md` §12–§18，重点是 §12.6–§12.8、§13、§14.1、§17.4–§17.5。

**计划边界：** 本计划是设计中 Plan D。它由 D1 read model、D2 Workspace、D3 Reporting、D4 Rollout 四个相互依赖的增量组成，因此保留为一份计划，但每个 Task 都必须产出可单独验证、可停止部署的安全增量。它不能在此时才首次实现 Plan B/C 的 writer、Gate、Prepared Action 或 operation-frozen safety isolation。

## 执行前置与暂停点

1. Plan A 的 Tool Truth/Coverage receipt、server-derived `AllFreshToolTruthAuthorityBundle` 与 revalidation dispatch hold，Plan B 的 Hypothesis Registry/generation seal/verification plan/proof paths/claim components/operation-frozen contract，以及 Plan C 的 Campaign/Prepared Action/oracle/objective outcome/revision adjudication/terminal decision 必须已经 `passing`，并且有新鲜定向证据。
2. Plan B 必须已经通过 `20260729000006_hypothesis_registry.sql` 创建 `investigation_rollout`、operation-frozen `investigation_contract_version / investigation_rollout_mode`、fork adoption receipt、`investigation_projection_outbox`、`investigation_projection_heads` 与 `investigation_projection_changes`，并导出 `golish_core::investigation_contract::InvestigationRolloutMode`。缺少任一接口时停止 Plan D，不在 `00008` 或 Plan D Rust 代码内临时造第二套 contract/head/change ledger。
3. Plan C 必须已经提供 `verification_campaigns`、round/strategy/prepared-action/authorization/execution/oracle、objective-local outcome、revision-level adjudication/terminal decision，以及 operation-scoped pending Prepared Action API；Workspace 只迁移布局并增加 richer read projection，禁止把单个 Campaign terminal 重新升级为 revision verdict。
4. **Schema 授权暂停点：** 开始 Task 2 前必须再次取得用户对修改 `golish-db` schema/migration 的明确授权。唯一允许的新 migration 文件是 `backend/crates/golish-db/migrations/20260729000008_investigation_projection.sql`；不得另开 `00009`、修改历史 migration，或用旧表的无类型 JSON 绕过授权。
5. 本计划不授权真实扫描、外部 provider/API 请求、rollout promotion、远端 push 或最终报告发布。测试使用数据库 fixture 和纯本地 UI fixture。
6. 每次运行 Cargo build/test/clippy 前先执行 `just space-guard`。默认只跑下文列出的定向验证；不自动运行 `./init.sh`、`just check`、`just test`、`just precommit` 或全 workspace 门禁。
7. 每个 Task 末尾的 `Future Commit` 只是未来按 Task 执行时的提交边界与建议 message；撰写本计划这一轮不执行 `git add`、`git commit`、`git push`，也不把计划文件之外的现有 worktree 变化纳入提交。

## 不变量与失败语义

- Read model 是 authoritative UI projection，不是 Gate 或 writer authority。
- 六个 read API 的 operation/project/scope/org/operator authority 全由服务端查询；前端 id 与 cursor 仅是 selector。
- 每个组合 read 在一个 `REPEATABLE READ READ ONLY` snapshot 中完成，并复用 Plan B 已冻结的 common envelope，保留其`projection_schema_version / change_seq / read_at`字段不变；`read_at`来自DB clock。每个authority projection另携`observed_as_of(=read_at) / effective_valid_until / authority_epoch_hash / temporal_status`，UI不使用本地时钟推断新鲜度。
- `projection_schema_version` 是 codec/normalization schema 版本，V1 固定为 `1`；普通数据变化只推进 `change_seq`。新分页cursor除operation/filter/schema/as-of change sequence/stable key/codec外，还绑定server从本次完整filtered authority universe派生的`as_of_temporal_cutoff / authority_epoch_set_hash`。即使`change_seq`未变，后续页DB clock越过cutoff或epoch set变化也返回`INVESTIGATION_PROJECTION_STALE`并要求从第一页重启；签名或resource/filter不匹配返回`INVESTIGATION_CURSOR_INVALID`。
- event 仅提供 `operation_id + change_seq` refresh hint；mount、restore、漏 event、乱序 event 均以主动 DB bootstrap 收敛。
- Zustand 只保存 workspace selector 和 refresh sequence，不保存 registry/campaign/report read model。
- legacy 缺失字段使用 `legacy_unavailable`，不能伪装为空、0、false、checked-empty 或 refuted。
- `legacy_only / shadow_registry / dual_read_compare` 保留旧 review/recovery mutation，且不显示、不授权 Prepared Action JIT；只有 `registry_authoritative_legacy_projection / new_only` 隐藏并由后端拒绝旧 mutation，改由 Prepared Action JIT 接管。UI 隐藏和后端拒绝必须同时存在。
- queue、lease、row version、receipt/hash 只进入 Audit drawer，不能成为主产品信息架构或优先级解释。
- raw body、stdout/stderr、credential、token、cookie、PII、完整 request/response 与 exploit payload 永不进入 report DTO、DOM、Markdown、JSON artifact。
- `method_audit_only`、`authorization_audit`、单 action observation、Campaign adjudication 与 Campaign terminal 都不能产生 revision-level verified/refuted 或 Finding；只有 Plan C revision-level adjudication + terminal decision 在重验 Plan B verification-plan/proof-path/claim-component exact set 与 latest objective outcomes 后才是新链 `SecurityVerdict` authority。
- declared coverage closure 与 global detection sufficiency 是两条正交轴。`ThreatCoverageProfileV1` 未实现前，`coverage_sufficiency=not_assessed`；即使 declared denominator 全部 tested，也禁止显示“全覆盖”“无漏洞”“安全”或完整绿色 PASS。`PASS_WITH_GAPS` 必须列 exact residual/affected inputs。
- report-input semantic authority 必须来自 Plan A 同 request、server-derived、relevant-root scoped 的 `AllFreshToolTruthAuthorityBundle`。TTL/epoch/window 失效只形成 `temporally_stale/as_of`，不能复用为 current authority；semantic orphan 或 Plan C quarantine 才是 revoked。same-semantic refresh不得复活旧 terminal/report，必须进入 H(g+1) 新 revision-level adjudication与新 report revision。
- deployment default 是 Tool Truth + Investigation 的合法联合状态，只影响新 operation；same-operation resume 保留原组合；fork adoption 必须有 exact receipt。
- comparison 是 whole-record exact compare；禁止字段级 fallback，禁止 comparison side 授权 execution。
- whole-record positive comparison只证明 compatibility，不证明漏洞 detection correctness；rank 4→5 还必须通过独立 versioned adversarial acceptance corpus 的 exact expected verdict/residual set。
- Plan A revalidation dispatch、Plan C Campaign dispatch 与 operation admission 是三个独立 hold/generation；`manual_only|auto_passive_t0_t1` 是 operation-frozen revalidation policy，T2/T3 始终回到 Plan C Prepared Action/JIT，任何 hold 不得代替另一个 hold。
- 所有失败返回带稳定 `code` 的错误对象，前端不解析自由文本。

## 目标文件结构

### D4 纯 rollout contract 与 deployment persistence

- 修改 Plan B 已创建的 `backend/crates/golish-core/src/investigation_contract.rs`
- 修改 `backend/crates/golish-core/src/lib.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/investigation_rollout.rs`
- 修改 Plan A 已创建的 `backend/crates/golish-db/src/repo/tool_truth_rollout.rs`
- 修改 Plan A 已创建的 `backend/crates/golish-db/src/repo/capability_execution_receipts.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/operation_rollout.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/investigation_projection/comparison.rs`
- 新增 `backend/crates/golish-db/src/repo/operation_default_rollout.rs`
- 新增 `backend/crates/golish-db/src/repo/report_authority_invalidation.rs`
- 修改 Plan C 已创建的 `backend/crates/golish-db/src/repo/verification_campaigns.rs`
- 修改 `backend/crates/golish-db/src/repo/operation_state.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish-db/src/repo/mod.rs`
- 新增 `backend/crates/golish-db/tests/investigation_rollout_migrations.rs`
- 新增 `backend/crates/golish-agent-app/tests/investigation_legacy_replay.rs`
- 新增 `backend/crates/golish/src/cli/investigation_rollout.rs`
- 修改 `backend/crates/golish/src/cli/args.rs`
- 修改 `backend/crates/golish/src/cli/mod.rs`

### D1 projection、legacy adapter 与 Tauri API

- 新增 `backend/crates/golish-db/migrations/20260729000008_investigation_projection.sql`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/investigation_projection/mod.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/investigation_projection/types.rs`
- 新增 `backend/crates/golish-db/src/repo/investigation_projection/version.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/investigation_projection/summary.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/investigation_projection/hypotheses.rs`
- 新增 `backend/crates/golish-db/src/repo/investigation_projection/campaigns.rs`
- 新增 `backend/crates/golish-db/src/repo/investigation_projection/timeline.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-db/src/repo/investigation_projection/legacy.rs`
- 新增 `backend/crates/golish-db/tests/investigation_projection_read_model.rs`
- 新增 `backend/crates/golish-agent-app/src/ai/operation_authority.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-agent-app/src/ai/commands/investigation/mod.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-agent-app/src/ai/commands/investigation/dto.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-agent-app/src/ai/commands/investigation/cursor.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/mod.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/mod.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/reporting.rs`
- 修改 Plan B 已创建的 `backend/crates/golish/src/commands_facade/investigation.rs`
- 修改 `backend/crates/golish/src/commands_facade/mod.rs`
- 修改 `backend/crates/golish/src/commands_registry.rs`
- 修改 Plan B 已创建的 `backend/crates/golish-agent-app/tests/investigation_ipc_authorization.rs`
- 新增 `backend/crates/golish-agent-app/tests/investigation_read_model.rs`
- 修改 Plan B 已创建的 `frontend/lib/api/investigation.ts`
- 由 ts-rs 生成或更新以下精确文件，禁止手改生成文件：
  - `frontend/lib/generated/InvestigationProjectionEnvelope.ts`
  - `frontend/lib/generated/InvestigationModePolicyView.ts`
  - `frontend/lib/generated/InvestigationCommandError.ts`
  - `frontend/lib/generated/ProjectionEntityKind.ts`（仅校验与 Plan B 一致）
  - `frontend/lib/generated/ProjectionInvalidationReason.ts`（仅校验与 Plan B 一致）
  - `frontend/lib/generated/TimelineEventKind.ts`（仅校验与 Plan B 一致）
  - `frontend/lib/generated/ProjectionSourceTimeStatusV1.ts`（仅校验与 Plan B 一致）
  - `frontend/lib/generated/InvestigationAuthorityTimeViewV1.ts`
  - `frontend/lib/generated/InvestigationCampaignListRequest.ts`
  - `frontend/lib/generated/InvestigationCampaignPageResponse.ts`
  - `frontend/lib/generated/InvestigationCampaignListItemView.ts`
  - `frontend/lib/generated/InvestigationCampaignDetailRequest.ts`
  - `frontend/lib/generated/InvestigationCampaignDetailResponse.ts`
  - `frontend/lib/generated/InvestigationCampaignDetailView.ts`
  - `frontend/lib/generated/InvestigationTimelineListRequest.ts`
  - `frontend/lib/generated/InvestigationTimelinePageResponse.ts`
  - `frontend/lib/generated/InvestigationTimelineItemView.ts`

### D2 Workspace、route、store 与恢复

- 新增 `frontend/components/Engagement/InvestigationWorkspace/index.ts`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/useInvestigationProjection.ts`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/WavesTab.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/InvestigationTimelineTab.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/HypothesisDetail.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/InvestigationStaleBanner.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.test.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.test.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.test.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.test.tsx`
- 新增 `frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.test.tsx`
- 修改 `frontend/store/types/session.ts`
- 修改 `frontend/store/slices/session.ts`
- 修改 `frontend/store/slices/session-core.ts`
- 修改 `frontend/store/selectors/pane-leaf.ts`
- 新增 `frontend/store/investigation-workspace.test.ts`
- 修改 `frontend/components/PaneContainer/PaneLeaf.tsx`
- 修改 `frontend/components/PaneContainer/PaneLeaf.lazy.test.tsx`
- 修改 `frontend/components/PaneContainer/PaneLeaf.memo.test.tsx`
- 修改 `frontend/components/AIChatPanel/StageProgressBar.tsx`
- 修改 `frontend/components/AIChatPanel/StageRow.tsx`
- 修改 `frontend/components/AIChatPanel/StageProgressBar.test.tsx`
- 修改 `frontend/components/AIChatPanel/AIChatPanel.tsx`
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx`
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.reporting.test.tsx`
- 修改 `frontend/components/Engagement/AttackCandidateReview.tsx`
- 修改 `frontend/components/Engagement/CandidateAttemptRows.tsx`
- 修改 `frontend/components/Engagement/CandidateVerificationProtocol.tsx`
- 修改 Plan B 已创建的 `frontend/components/Engagement/HypothesisRegistryAudit.tsx`
- 修改 Plan B 已创建的 `frontend/components/Engagement/HypothesisRegistryAudit.test.tsx`
- 修改 Plan C 已创建的 `frontend/components/Engagement/PendingPreparedActionPanel.tsx`
- 修改 Plan C 已创建的 `frontend/components/Engagement/PendingPreparedActionPanel.test.tsx`
- 修改 `frontend/services/ai-events/harness-handlers.ts`
- 修改 `frontend/services/ai-events/harness-handlers.test.ts`
- 修改 `backend/crates/golish-agent-app/src/conversation_store/mod.rs`
- 修改 `backend/crates/golish-agent-app/src/conversation_store/batch.rs`
- 修改 `frontend/lib/api/conversation-db.ts`
- 修改 `frontend/lib/workspace-storage.ts`
- 修改 `frontend/lib/conversation-db-sync.ts`
- 修改 `frontend/lib/conversation-db-sync.test.ts`
- 修改 `frontend/lib/terminal-restore.ts`
- 修改 `frontend/lib/terminal-restore.test.ts`

### D3 Reporting 与安全 projection

- 修改 `backend/crates/golish-reporting-domain/src/report.rs`
- 修改 `backend/crates/golish-reporting-domain/src/section.rs`
- 修改 `backend/crates/golish-reporting-domain/src/validation.rs`
- 修改 `backend/crates/golish-reporting-app/src/redaction.rs`
- 修改 `backend/crates/golish-reporting-app/src/read_model.rs`
- 修改 `backend/crates/golish-reporting-app/src/ports.rs`
- 修改 `backend/crates/golish-reporting-app/src/renderer.rs`
- 修改 `backend/crates/golish-reporting-app/src/finalizer.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/reporting_gate.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/reporting.rs`
- 修改 `backend/crates/golish-db/src/repo/report_revisions.rs`
- 修改 `backend/crates/golish-db/src/repo/report_source_manifest.rs`
- 修改 `backend/crates/golish-db/src/repo/report_sections.rs`
- 修改 `backend/crates/golish-db/src/repo/report_claims.rs`
- 新增 `backend/crates/golish-db/src/repo/report_authority_invalidation.rs`
- 修改 Plan A 已创建的 `backend/crates/golish-db/src/repo/capability_execution_receipts.rs`
- 修改 Plan C 已创建的 `backend/crates/golish-db/src/repo/verification_campaigns.rs`
- 新增 `backend/crates/golish-db/src/repo/historical_report_artifacts.rs`
- 修改 `backend/crates/golish-agent-app/tests/reporting_authority.rs`
- 修改 `backend/crates/golish-agent-app/tests/reporting_ipc_authorization.rs`
- 修改 `backend/crates/golish-db/tests/reporting_read_model_migrations.rs`
- 修改 `frontend/lib/api/reporting.ts`
- 修改 `frontend/components/Engagement/ReportReadModelView.tsx`
- 修改 `frontend/components/Engagement/ReportReadModelView.test.tsx`

### 文档与 evidence closeout

- 修改 `docs/modules/backend/golish-core.md`
- 修改 `docs/modules/backend/golish-db/repo.md`
- 修改 `docs/modules/backend/golish-agent-app/ai.md`
- 修改 `docs/modules/backend/golish-agent-app/conversation_store.md`
- 修改 `docs/modules/backend/golish-reporting-domain.md`
- 修改 `docs/modules/backend/golish-reporting-app.md`
- 修改 `docs/modules/frontend/store.md`
- 修改 `docs/modules/frontend/components.md`
- 修改 `docs/modules/frontend/lib.md`
- 修改 `docs/modules/INDEX.md`
- 修改 `agent-progress.md`
- 修改 `feature_list.json`

## Task 1（D4 基础）：审计唯一 policy 并冻结联合七态/逐边晋级条件

**Files:**

- Modify: `backend/crates/golish-core/src/investigation_contract.rs`（只加consumer一致性测试，不新建policy类型）
- Modify: `backend/crates/golish-db/src/repo/operation_rollout.rs`
- Create: `backend/crates/golish-db/tests/investigation_rollout_migrations.rs`（Task 2/12继续扩展）

**Step 1：写单一 policy consumer 与 joint-rank RED**

Plan B 已经拥有最终唯一的`InvestigationModePolicy`与`InvestigationRolloutMode::policy()`；本 Task 不重新定义或改写矩阵。测试固定所有consumer都读取该policy，并冻结七个Tool Truth+Investigation合法pair：

```rust
#[test]
fn joint_contract_rank_is_closed_and_monotonic() {
    let cases = [
        (0, "legacy_v1", "legacy_candidate_v1", "legacy_only"),
        (1, "shadow_v1", "legacy_candidate_v1", "legacy_only"),
        (2, "shadow_v1", "hypothesis_registry_v1", "shadow_registry"),
        (3, "shadow_v1", "hypothesis_registry_v1", "dual_read_compare"),
        (4, "receipt_v1", "hypothesis_registry_v1", "dual_read_compare"),
        (5, "receipt_v1", "hypothesis_registry_v1", "registry_authoritative_legacy_projection"),
        (6, "receipt_v1", "hypothesis_registry_v1", "new_only"),
    ];
    for (rank, tool_truth, investigation_contract, investigation_mode) in cases {
        assert_eq!(
            joint_contract_rank(tool_truth, investigation_contract, investigation_mode),
            Ok(rank),
        );
    }
    assert!(joint_contract_rank(
        "shadow_v1", "legacy_candidate_v1", "shadow_registry"
    ).is_err());
}
```

再用source scan/compile tests证明backend不存在`PlanBInvestigationPolicy`、第二个policy struct或手写五态布尔match；Candidate dispatcher、Campaign admission、legacy mutation、Workspace policy view与comparison都调用`mode.policy()`。前端只消费server-derived`InvestigationModePolicyView`做展示，不能复制authority matrix。

逐边criteria冻结为typed enum：rank 0→1 writer readiness；1→2 shadow evaluator readiness；2→3 closed shadow cohort exact；3→4 Tool Truth all-fresh semantic/freshness authority exact；4→5 closed dual compatibility cohort + authoritative canary + versioned adversarial acceptance corpus exact；5→6 legacy-consumer retirement。只有2→3与4→5要求positive comparison cohort，不能统一使用`compared_operations > 0`。positive whole-record comparison只说明新旧 projection/codec 在声明字段上兼容，绝不是 detection correctness 证据；4→5 必须额外验证 known-vulnerable、known-safe、control-failure、soft-404、WAF/interstitial、dynamic-content、multi-role IDOR、race 与 adapter-missing fixture 的 independently-authored expected verdict/residual exact set。

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_rollout_)' --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_rollout_migrations -E 'test(joint_contract_) | test(promotion_edge_)' --status-level fail)
```

Expected: Plan B policy tests继续通过；新增joint rank/edge criteria/consumer audit因D-owned criteria尚未实现而失败。

**Step 3：只实现 joint edge criteria，不实现第三套 policy**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationPromotionCriteriaV1 {
    ToolTruthShadowWriterReady,
    ShadowEvaluatorReady,
    ClosedShadowCohortExact,
    ToolTruthReceiptReconciliationExact,
    DualAndAuthoritativeCanaryExact,
    LegacyConsumersRetired,
}
```

`operation_rollout.rs`把每个`from_rank + 1`映射到唯一criteria和所需evidence shape；非法跳级/倒退返回typed error。五态authority仍完全由Plan B policy提供。

**Step 4：运行 GREEN 与 scoped clippy**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_rollout_)' --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_rollout_migrations -E 'test(joint_contract_) | test(promotion_edge_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-core -p golish-db --all-targets -- -D warnings)
```

Expected: 唯一policy仍全绿；七态闭集、逐边criteria、非法pair/跳级/倒退和consumer一致性全绿；两个crate无warning。

### Future Commit

```bash
git add backend/crates/golish-core/src/investigation_contract.rs backend/crates/golish-db/src/repo/operation_rollout.rs backend/crates/golish-db/tests/investigation_rollout_migrations.rs
git commit -m "feat(rollout): freeze joint investigation criteria"
```

## Task 2（D1/D3/D4 schema）：新增唯一 forward-only projection migration

**授权暂停点：** 开始本 Task 前向用户展示本 Task 的表、列、约束和 rollback；收到明确 schema/migration 授权后才创建 migration。未授权时停止，保持 Plan D 文件之外零修改。

**Files:**

- Create: `backend/crates/golish-db/migrations/20260729000008_investigation_projection.sql`
- Create: `backend/crates/golish-db/tests/investigation_projection_read_model.rs`
- Modify: `backend/crates/golish-db/tests/investigation_rollout_migrations.rs`
- Modify: `backend/crates/golish-db/tests/reporting_read_model_migrations.rs`

**Step 1：先写 migration RED**

三个 focused test 必须先证明 Plan B contract 能被安全扩展，同时不存在第二套同名 authority：

- `20260729000006_hypothesis_registry.sql` 已经且只创建一套 rollout、outbox、projection head/change ledger 与 adoption receipt；`00008` 不重复这些对象；
- Plan C canonical write与Plan B完整typed outbox source batch在同一事务提交，rollback不留下source head advance/outbox；materialized change/head只由后续whole-batch projector写；
- projector中途rollback不留下entity version/change/compatibility receipt/head advance，canonical/outbox仍可重试；较晚source batch不能越过predecessor；
- Plan B `investigation_projection_compare_samples` relation恰好一张，`investigation_shadow_comparisons`不存在；sample以`(operation, as_of_change_seq, record kind, record key)` exact-one；
- Plan B `investigation_rollout` default CAS 不能改既有 operation；operation mode/contract UPDATE 继续由 Plan B trigger 拒绝；
- promotion receipt 不能由 caller 自报缺失 cohort 或 mismatch count；
- fork target 若 contract/mode 与 source 不同，继续复用 Plan B adoption receipt；缺 receipt 时 fail closed；
- terminal workspace selector 可为 null，保存/恢复 JSON 不触碰 canonical read model；
- report revision 缺 generation seal、consolidation result 或 report-input hash 时不能 final。
- pre-existing final revision必须逐条绑定exact historical artifact **metadata** receipt；零artifact、DB内artifact metadata/ownership不一致使migration整体abort。SQL migration不谎称自己读取过filesystem bytes；historical read adapter每次提供旧bytes前必须fresh read+rehash并写request-scoped attestation，missing/mismatch/unreadable一律typed unavailable。migration后任何新historical final由DB trigger拒绝；
- legacy report多claim必须由report-level authority seal/member exact set覆盖；任选单个Attempt receipt、跨operation/org lineage或rank 5/6 legacy writer均拒绝；
- campaign report的wave terminal authority必须是tagged consolidation XOR fixed-point，并绑定最终Wave coverage receipt；局部Campaign coverage receipt不能final report；
- report begin/finalize/supersede相同stable request/hash exact replay原mutation receipt与source batch，request id payload drift拒绝；
- final report source authority后续quarantine/orphan时，A/C source writer同canonical transaction经shared seam追加invalidation+typed whole-batch；canonical strong-read guard立即拒绝current/export/reuse，历史download仅按revoked-history+fresh stable snapshot policy；异步projection随后显示revoked，artifact bytes不变。TTL自然过期只显示temporally-stale/as-of，不写revoked。
- upgraded fixture中现有`report_revisions_guard`只在受约束migration transaction内允许一次final→historical metadata binding，且仅新authority列可变；永久guard安装后同类UPDATE再次失败。clean与upgraded DB都必须通过。

```rust
#[tokio::test]
async fn plan_d_migration_reuses_plan_b_projection_authority() {
    let database = TestDatabase::new().await;
    assert_eq!(database.relation_count("investigation_rollout").await, 1);
    assert_eq!(database.relation_count("investigation_projection_outbox").await, 1);
    assert_eq!(database.relation_count("investigation_projection_heads").await, 1);
    assert_eq!(database.relation_count("investigation_projection_changes").await, 1);
    assert_eq!(database.relation_count("investigation_projection_compare_samples").await, 1);
    assert_eq!(database.relation_count("investigation_shadow_comparisons").await, 0);
    assert!(database.column_exists("terminal_state", "investigation_workspace_json").await);
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_projection_read_model --test investigation_rollout_migrations --test reporting_read_model_migrations --status-level fail)
```

Expected: 新 selector、compare aggregate、promotion receipt 与 report seal assertions 因 `00008` 尚未存在而失败；Plan B 的 rollout/head/change/outbox/adoption assertions 已通过且旧 migration 保持只读。

**Step 3：只扩展 Plan B projection contract 并加入安全 UI selector**

`00008` 不得出现 `CREATE TABLE investigation_rollout`、`CREATE TABLE investigation_projection_outbox`、`CREATE TABLE investigation_projection_heads`、`CREATE TABLE investigation_projection_changes` 或第二个 adoption receipt。只加入 pagination/timeline 所需索引和 selector：

```sql
CREATE INDEX investigation_projection_changes_timeline_idx
    ON investigation_projection_changes(operation_id, change_seq, entity_kind, entity_id);

ALTER TABLE terminal_state
    ADD COLUMN investigation_workspace_json JSONB;
```

索引列名必须以 Plan B `00006` 的实际列为准；如果 Plan B 已提供同等索引，migration test 证明 query plan 命中后不重复创建。Plan C 的 canonical repo 使用 Plan B 已有 outbox helper 在同一 transaction enqueue Campaign/round/action/oracle/adjudication/terminal/FactDelta 变化；不能新增第二个 sequence function，也不能 best-effort post-commit bump。

**Step 4：只加入 cohort aggregate、joint promotion receipt 与 safety-hold event**

Plan B已经创建唯一whole-batch outbox/source head/materialized entity-version/change/head ledger、`investigation_projection_compare_samples`和canonical comparator；`00008`不得再次创建这些表、sample表或单event enqueue函数。只新增：

```sql
CREATE TABLE investigation_projection_compare_aggregates (
    cohort_id UUID PRIMARY KEY,
    from_joint_rank SMALLINT NOT NULL CHECK (from_joint_rank BETWEEN 0 AND 5),
    to_joint_rank SMALLINT NOT NULL CHECK (to_joint_rank = from_joint_rank + 1),
    criteria_version TEXT NOT NULL CHECK (BTRIM(criteria_version) <> ''),
    projection_schema_version INTEGER NOT NULL DEFAULT 1
        CHECK (projection_schema_version = 1),
    cutoff_manifest_hash TEXT NOT NULL CHECK (cutoff_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    admitted_operation_count BIGINT NOT NULL CHECK (admitted_operation_count >= 0),
    expected_record_count BIGINT NOT NULL CHECK (expected_record_count >= 0),
    sampled_record_count BIGINT NOT NULL CHECK (sampled_record_count >= 0),
    matched_record_count BIGINT NOT NULL CHECK (matched_record_count >= 0),
    mismatch_record_count BIGINT NOT NULL CHECK (mismatch_record_count >= 0),
    missing_record_count BIGINT NOT NULL CHECK (missing_record_count >= 0),
    incomplete_record_count BIGINT NOT NULL CHECK (incomplete_record_count >= 0),
    corrupt_record_count BIGINT NOT NULL CHECK (corrupt_record_count >= 0),
    comparison_set_hash TEXT CHECK (
        comparison_set_hash IS NULL OR comparison_set_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    admission_closed BOOLEAN NOT NULL,
    aggregated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (sampled_record_count = matched_record_count + mismatch_record_count
        + missing_record_count + incomplete_record_count + corrupt_record_count),
    CHECK (sampled_record_count <= expected_record_count)
);

CREATE TABLE investigation_projection_compare_cohort_members (
    cohort_id UUID NOT NULL REFERENCES investigation_projection_compare_aggregates(cohort_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    as_of_change_seq BIGINT NOT NULL CHECK (as_of_change_seq >= 0),
    expected_record_count BIGINT NOT NULL CHECK (expected_record_count >= 0),
    sample_set_hash TEXT NOT NULL CHECK (sample_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY (cohort_id, operation_id)
);

CREATE TABLE operation_default_promotion_receipts (
    receipt_id UUID PRIMARY KEY,
    from_joint_rank SMALLINT NOT NULL CHECK (from_joint_rank BETWEEN 0 AND 5),
    to_joint_rank SMALLINT NOT NULL CHECK (to_joint_rank = from_joint_rank + 1),
    criteria_version TEXT NOT NULL CHECK (BTRIM(criteria_version) <> ''),
    tool_truth_from TEXT NOT NULL,
    tool_truth_to TEXT NOT NULL,
    investigation_contract_from TEXT NOT NULL,
    investigation_mode_from TEXT NOT NULL,
    investigation_contract_to TEXT NOT NULL,
    investigation_mode_to TEXT NOT NULL,
    cohort_id UUID REFERENCES investigation_projection_compare_aggregates(cohort_id) ON DELETE RESTRICT,
    cohort_cutoff_manifest_hash TEXT CHECK (
        cohort_cutoff_manifest_hash IS NULL
        OR cohort_cutoff_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    evidence_manifest_hash TEXT NOT NULL CHECK (evidence_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_member_count BIGINT NOT NULL CHECK (evidence_member_count > 0),
    canary_manifest_hash TEXT CHECK (canary_manifest_hash IS NULL OR canary_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    adversarial_acceptance_receipt_id UUID,
    expected_tool_truth_row_version BIGINT NOT NULL CHECK (expected_tool_truth_row_version >= 0),
    expected_investigation_row_version BIGINT NOT NULL CHECK (expected_investigation_row_version >= 0),
    promoted_by_principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (BTRIM(reason) <> ''),
    promoted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (to_joint_rank, evidence_manifest_hash),
    CHECK ((cohort_id IS NULL) = (cohort_cutoff_manifest_hash IS NULL)),
    CHECK ((from_joint_rank=4 AND to_joint_rank=5)
        = (adversarial_acceptance_receipt_id IS NOT NULL)),
    CHECK (operation_joint_contract_rank(
        tool_truth_from, investigation_contract_from, investigation_mode_from
    ) = from_joint_rank),
    CHECK (operation_joint_contract_rank(
        tool_truth_to, investigation_contract_to, investigation_mode_to
    ) = to_joint_rank)
);

CREATE TABLE tool_truth_shadow_writer_readiness_receipts (
    receipt_id UUID PRIMARY KEY,
    criteria_version TEXT NOT NULL,
    deployment_digest TEXT NOT NULL CHECK (deployment_digest ~ '^sha256:[0-9a-f]{64}$'),
    observation_window_started_at TIMESTAMPTZ NOT NULL,
    observation_window_ended_at TIMESTAMPTZ NOT NULL,
    observed_operation_count BIGINT NOT NULL CHECK (observed_operation_count > 0),
    readiness_member_count BIGINT NOT NULL CHECK (readiness_member_count = observed_operation_count),
    readiness_membership_hash TEXT NOT NULL CHECK (readiness_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    assessment_set_hash TEXT NOT NULL CHECK (assessment_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    missing_assessment_count BIGINT NOT NULL CHECK (missing_assessment_count = 0),
    orphan_reconciliation_count BIGINT NOT NULL CHECK (orphan_reconciliation_count = 0),
    corrupt_artifact_count BIGINT NOT NULL CHECK (corrupt_artifact_count = 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (observation_window_ended_at > observation_window_started_at)
);

CREATE TABLE tool_truth_shadow_writer_readiness_members (
    receipt_id UUID NOT NULL
        REFERENCES tool_truth_shadow_writer_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    assessment_set_id UUID NOT NULL,
    assessment_set_hash TEXT NOT NULL CHECK (assessment_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    reconciliation_census_hash TEXT NOT NULL CHECK (reconciliation_census_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,operation_id)
);

CREATE TABLE registry_shadow_evaluator_readiness_receipts (
    receipt_id UUID PRIMARY KEY,
    criteria_version TEXT NOT NULL,
    evaluator_contract_version TEXT NOT NULL,
    evaluator_digest TEXT NOT NULL CHECK (evaluator_digest ~ '^sha256:[0-9a-f]{64}$'),
    fixture_manifest_hash TEXT NOT NULL CHECK (fixture_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    evaluation_count BIGINT NOT NULL CHECK (evaluation_count > 0),
    evaluation_member_count BIGINT NOT NULL CHECK (evaluation_member_count = evaluation_count),
    evaluation_membership_hash TEXT NOT NULL CHECK (evaluation_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    external_port_call_count BIGINT NOT NULL CHECK (external_port_call_count = 0),
    canonical_mutation_count BIGINT NOT NULL CHECK (canonical_mutation_count = 0),
    incomplete_or_corrupt_count BIGINT NOT NULL CHECK (incomplete_or_corrupt_count = 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE registry_shadow_evaluator_readiness_members (
    receipt_id UUID NOT NULL
        REFERENCES registry_shadow_evaluator_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    fixture_id TEXT NOT NULL CHECK (BTRIM(fixture_id) <> ''),
    fixture_hash TEXT NOT NULL CHECK (fixture_hash ~ '^sha256:[0-9a-f]{64}$'),
    evaluation_result_hash TEXT NOT NULL CHECK (evaluation_result_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,fixture_id)
);

CREATE TABLE historical_read_adapter_probe_receipts (
    receipt_id UUID PRIMARY KEY,
    adapter_version TEXT NOT NULL,
    adapter_digest TEXT NOT NULL CHECK (adapter_digest ~ '^sha256:[0-9a-f]{64}$'),
    expected_artifact_count BIGINT NOT NULL CHECK (expected_artifact_count >= 0),
    expected_artifact_manifest_hash TEXT NOT NULL
        CHECK (expected_artifact_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    probed_artifact_count BIGINT NOT NULL CHECK (probed_artifact_count >= 0),
    probe_member_count BIGINT NOT NULL CHECK (probe_member_count = probed_artifact_count),
    probe_membership_hash TEXT NOT NULL CHECK (probe_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    failed_probe_count BIGINT NOT NULL CHECK (failed_probe_count = 0),
    CHECK (probed_artifact_count = expected_artifact_count),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE historical_read_adapter_probe_members (
    receipt_id UUID NOT NULL
        REFERENCES historical_read_adapter_probe_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    report_revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    historical_artifact_receipt_id UUID NOT NULL,
    read_attestation_id UUID NOT NULL,
    attestation_hash TEXT NOT NULL CHECK (attestation_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,report_revision_id,historical_artifact_receipt_id)
);

CREATE TABLE compatibility_projection_health_receipts (
    receipt_id UUID PRIMARY KEY,
    cohort_manifest_hash TEXT NOT NULL CHECK (cohort_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_projection_count BIGINT NOT NULL CHECK (expected_projection_count >= 0),
    projection_member_count BIGINT NOT NULL CHECK (projection_member_count = expected_projection_count),
    projection_membership_hash TEXT NOT NULL CHECK (projection_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    pending_batch_count BIGINT NOT NULL CHECK (pending_batch_count = 0),
    projection_error_count BIGINT NOT NULL CHECK (projection_error_count = 0),
    divergence_count BIGINT NOT NULL CHECK (divergence_count = 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE compatibility_projection_health_members (
    receipt_id UUID NOT NULL
        REFERENCES compatibility_projection_health_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    as_of_change_seq BIGINT NOT NULL CHECK (as_of_change_seq >= 0),
    projection_head_hash TEXT NOT NULL CHECK (projection_head_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,operation_id)
);

CREATE TABLE legacy_consumer_retirement_receipts (
    receipt_id UUID PRIMARY KEY,
    criteria_version TEXT NOT NULL,
    observation_window_started_at TIMESTAMPTZ NOT NULL,
    observation_window_ended_at TIMESTAMPTZ NOT NULL,
    consumer_inventory_hash TEXT NOT NULL CHECK (consumer_inventory_hash ~ '^sha256:[0-9a-f]{64}$'),
    consumer_inventory_count BIGINT NOT NULL CHECK (consumer_inventory_count >= 0),
    consumer_member_count BIGINT NOT NULL CHECK (consumer_member_count = consumer_inventory_count),
    consumer_membership_hash TEXT NOT NULL CHECK (consumer_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    unmigrated_consumer_count BIGINT NOT NULL CHECK (unmigrated_consumer_count = 0),
    legacy_mutation_call_count BIGINT NOT NULL CHECK (legacy_mutation_call_count = 0),
    legacy_read_fallback_call_count BIGINT NOT NULL CHECK (legacy_read_fallback_call_count = 0),
    compatibility_projection_health_receipt_id UUID NOT NULL
        REFERENCES compatibility_projection_health_receipts(receipt_id) ON DELETE RESTRICT,
    historical_adapter_probe_receipt_id UUID NOT NULL
        REFERENCES historical_read_adapter_probe_receipts(receipt_id) ON DELETE RESTRICT,
    retirement_manifest_hash TEXT NOT NULL CHECK (retirement_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (observation_window_ended_at > observation_window_started_at)
);

CREATE TABLE legacy_consumer_retirement_members (
    receipt_id UUID NOT NULL
        REFERENCES legacy_consumer_retirement_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    consumer_key TEXT NOT NULL CHECK (BTRIM(consumer_key) <> ''),
    consumer_binary_digest TEXT NOT NULL CHECK (consumer_binary_digest ~ '^sha256:[0-9a-f]{64}$'),
    retirement_evidence_hash TEXT NOT NULL CHECK (retirement_evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,consumer_key)
);

CREATE TABLE authoritative_report_dry_run_receipts (
    receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    generation_seal_id UUID NOT NULL,
    wave_coverage_receipt_id UUID NOT NULL,
    report_input_hash TEXT NOT NULL CHECK (report_input_hash ~ '^sha256:[0-9a-f]{64}$'),
    renderer_contract_version TEXT NOT NULL CHECK (BTRIM(renderer_contract_version) <> ''),
    redaction_sentinel_passed BOOLEAN NOT NULL CHECK (redaction_sentinel_passed),
    external_export_count BIGINT NOT NULL CHECK (external_export_count = 0),
    receipt_hash TEXT NOT NULL CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(operation_id,generation_seal_id,wave_coverage_receipt_id,report_input_hash)
);

CREATE TABLE adversarial_acceptance_corpus_receipts (
    receipt_id UUID PRIMARY KEY,
    corpus_contract_version TEXT NOT NULL CHECK (BTRIM(corpus_contract_version) <> ''),
    corpus_digest TEXT NOT NULL CHECK (corpus_digest ~ '^sha256:[0-9a-f]{64}$'),
    evaluator_binary_digest TEXT NOT NULL CHECK (evaluator_binary_digest ~ '^sha256:[0-9a-f]{64}$'),
    fixture_member_count BIGINT NOT NULL CHECK (fixture_member_count > 0),
    fixture_membership_hash TEXT NOT NULL CHECK (fixture_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_outcome_membership_hash TEXT NOT NULL CHECK (expected_outcome_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    observed_outcome_membership_hash TEXT NOT NULL CHECK (observed_outcome_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    mismatch_count BIGINT NOT NULL CHECK (mismatch_count = 0),
    missing_count BIGINT NOT NULL CHECK (missing_count = 0),
    extra_count BIGINT NOT NULL CHECK (extra_count = 0),
    sealed_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE adversarial_acceptance_corpus_members (
    receipt_id UUID NOT NULL
        REFERENCES adversarial_acceptance_corpus_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    fixture_kind TEXT NOT NULL CHECK (fixture_kind IN (
        'known_vulnerable','known_safe','control_failure','soft_404','waf_interstitial',
        'dynamic_content','multi_role_idor','race','adapter_missing'
    )),
    fixture_id TEXT NOT NULL CHECK (BTRIM(fixture_id) <> ''),
    fixture_hash TEXT NOT NULL CHECK (fixture_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_verdict TEXT NOT NULL CHECK (expected_verdict IN (
        'verified','refuted','inconclusive','blocked','not_assessed'
    )),
    expected_residual_set_hash TEXT NOT NULL CHECK (expected_residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    observed_verdict TEXT NOT NULL CHECK (observed_verdict IN (
        'verified','refuted','inconclusive','blocked','not_assessed'
    )),
    observed_residual_set_hash TEXT NOT NULL CHECK (observed_residual_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    outcome_hash TEXT NOT NULL CHECK (outcome_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,fixture_kind,fixture_id),
    CHECK (expected_verdict = observed_verdict),
    CHECK (expected_residual_set_hash = observed_residual_set_hash)
);

ALTER TABLE operation_default_promotion_receipts
    ADD CONSTRAINT operation_default_promotion_acceptance_fk
    FOREIGN KEY(adversarial_acceptance_receipt_id)
    REFERENCES adversarial_acceptance_corpus_receipts(receipt_id) ON DELETE RESTRICT;

CREATE TABLE operation_default_promotion_evidence_members (
    receipt_id UUID NOT NULL REFERENCES operation_default_promotion_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    evidence_kind TEXT NOT NULL CHECK (evidence_kind IN (
        'tool_truth_shadow_writer_readiness_receipt',
        'registry_shadow_evaluator_readiness_receipt',
        'shadow_comparison_sample',
        'tool_truth_all_fresh_authority_bundle',
        'dual_comparison_sample',
        'authoritative_canary_action_receipt',
        'authoritative_canary_oracle_receipt',
        'authoritative_canary_coverage_receipt',
        'authoritative_canary_revision_adjudication',
        'authoritative_canary_report_dry_run_receipt',
        'adversarial_acceptance_corpus_receipt',
        'legacy_consumer_retirement_receipt'
    )),
    operation_id UUID REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    tool_truth_readiness_receipt_id UUID
        REFERENCES tool_truth_shadow_writer_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    registry_readiness_receipt_id UUID
        REFERENCES registry_shadow_evaluator_readiness_receipts(receipt_id) ON DELETE RESTRICT,
    comparison_id UUID
        REFERENCES investigation_projection_compare_samples(comparison_id) ON DELETE RESTRICT,
    tool_truth_authority_bundle_id UUID
        REFERENCES tool_truth_authority_bundle_seals(id) ON DELETE RESTRICT,
    canary_action_execution_id UUID
        REFERENCES verification_action_executions(id) ON DELETE RESTRICT,
    canary_oracle_assessment_id UUID
        REFERENCES verification_oracle_assessments(id) ON DELETE RESTRICT,
    canary_wave_coverage_receipt_id UUID
        REFERENCES verification_wave_coverage_receipts(id) ON DELETE RESTRICT,
    canary_revision_adjudication_id UUID
        REFERENCES hypothesis_revision_adjudications(id) ON DELETE RESTRICT,
    canary_report_dry_run_receipt_id UUID
        REFERENCES authoritative_report_dry_run_receipts(receipt_id) ON DELETE RESTRICT,
    adversarial_acceptance_receipt_id UUID
        REFERENCES adversarial_acceptance_corpus_receipts(receipt_id) ON DELETE RESTRICT,
    legacy_retirement_receipt_id UUID
        REFERENCES legacy_consumer_retirement_receipts(receipt_id) ON DELETE RESTRICT,
    source_ref_kind TEXT NOT NULL CHECK (BTRIM(source_ref_kind) <> ''),
    source_ref_id TEXT NOT NULL CHECK (BTRIM(source_ref_id) <> ''),
    source_ref_hash TEXT NOT NULL CHECK (source_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY (receipt_id, ordinal),
    UNIQUE (receipt_id, evidence_kind, source_ref_kind, source_ref_id),
    CHECK (num_nonnulls(
        tool_truth_readiness_receipt_id, registry_readiness_receipt_id, comparison_id,
        tool_truth_authority_bundle_id, canary_action_execution_id,
        canary_oracle_assessment_id, canary_wave_coverage_receipt_id,
        canary_revision_adjudication_id,
        canary_report_dry_run_receipt_id, adversarial_acceptance_receipt_id,
        legacy_retirement_receipt_id
    ) = 1),
    CHECK (
        CASE evidence_kind
            WHEN 'tool_truth_shadow_writer_readiness_receipt' THEN tool_truth_readiness_receipt_id IS NOT NULL
            WHEN 'registry_shadow_evaluator_readiness_receipt' THEN registry_readiness_receipt_id IS NOT NULL
            WHEN 'shadow_comparison_sample' THEN comparison_id IS NOT NULL
            WHEN 'tool_truth_all_fresh_authority_bundle' THEN tool_truth_authority_bundle_id IS NOT NULL
            WHEN 'dual_comparison_sample' THEN comparison_id IS NOT NULL
            WHEN 'authoritative_canary_action_receipt' THEN canary_action_execution_id IS NOT NULL
            WHEN 'authoritative_canary_oracle_receipt' THEN canary_oracle_assessment_id IS NOT NULL
            WHEN 'authoritative_canary_coverage_receipt' THEN canary_wave_coverage_receipt_id IS NOT NULL
            WHEN 'authoritative_canary_revision_adjudication' THEN canary_revision_adjudication_id IS NOT NULL
            WHEN 'authoritative_canary_report_dry_run_receipt' THEN canary_report_dry_run_receipt_id IS NOT NULL
            WHEN 'adversarial_acceptance_corpus_receipt' THEN adversarial_acceptance_receipt_id IS NOT NULL
            WHEN 'legacy_consumer_retirement_receipt' THEN legacy_retirement_receipt_id IS NOT NULL
            ELSE FALSE
        END
    )
);

CREATE TABLE operation_rollout_safety_hold_events (
    event_id UUID PRIMARY KEY,
    hold_scope TEXT NOT NULL CHECK (hold_scope IN ('campaign_dispatch','operation_admission')),
    previous_held BOOLEAN NOT NULL,
    next_held BOOLEAN NOT NULL,
    previous_scope_generation BIGINT NOT NULL CHECK (previous_scope_generation >= 0),
    next_scope_generation BIGINT NOT NULL CHECK (next_scope_generation = previous_scope_generation + 1),
    previous_row_version BIGINT NOT NULL CHECK (previous_row_version >= 0),
    next_row_version BIGINT NOT NULL CHECK (next_row_version = previous_row_version + 1),
    reason_code TEXT NOT NULL CHECK (BTRIM(reason_code) <> ''),
    evidence_manifest_hash TEXT NOT NULL CHECK (evidence_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    principal_id UUID NOT NULL REFERENCES operator_principals(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (previous_held IS DISTINCT FROM next_held)
);
```

`change_seq`是operation-local，禁止用一个全局数字截断整个cohort。每个cohort member冻结自己的`as_of_change_seq`和sample-set hash，aggregate的`cutoff_manifest_hash`是按operation id排序后的member manifest hash；`admitted_operation_count`必须等于cohort member数量，receipt中的manifest hash必须完全一致。aggregate只能在锁内从Plan B sample exact set重算。rank 0→1、1→2、3→4、5→6不强制positive comparison cohort；rank 2→3和4→5必须`cohort_id`非空、admission closed、positive sample，且mismatch/missing/incomplete/corrupt为0。该 positive 结果只证明 compatibility。rank 4→5 还必须有authoritative canary manifest与独立sealed `adversarial_acceptance_corpus_receipt`；corpus必须覆盖九类fixture，逐member exact匹配independently-authored expected verdict与residual set，少/多/错一项均拒绝。typed repo而不是caller counts决定每条边的额外条件。

每张promotion receipt必须同时写`operation_default_promotion_evidence_members` exact set；`evidence_member_count`等于实际row数，`evidence_manifest_hash`是按ordinal排序的typed member canonical hash，不能只有不可追溯的摘要hash。0→1必须exact-one引用`tool_truth_shadow_writer_readiness_receipt`，1→2必须exact-one引用`registry_shadow_evaluator_readiness_receipt`，5→6必须exact-one引用`legacy_consumer_retirement_receipt`；这三张summary receipt还必须分别重读自己的immutable ordered member exact set，重新计算count/hash/window cutoff，不能只相信header自报摘要。2→3/3→4/4→5的comparison/all-fresh authority bundle/canary全部使用上表的typed FK，`num_nonnulls=1 + evidence_kind CASE`使字符串ID不能伪装authority；4→5必须同时引用dual sample、relevant-root all-fresh bundle、canary action execution、oracle assessment、最终Wave coverage receipt、revision adjudication、typed report dry-run receipt与adversarial acceptance corpus receipt。实际DDL按FK依赖拓扑创建各表，并给所有引用补operation/project/org compound FK；上面的分组顺序只为阅读。任何edge缺/额外evidence kind、cross-operation引用或nullable FK形状不符都由DB CHECK + repo exact-set拒绝。`source_ref_*`只作可读审计镜像，必须与typed row重算值一致但绝不是authority，也不复制raw payload。

readiness/probe/health/retirement/adversarial-acceptance header与member采用“open header → ordered members → close seal”的typed repo，close在同一transaction重读成员、冻结count/hash与cutoff；sealed后header/member均拒绝UPDATE/DELETE/追加。零成员只有契约明确允许的inventory/probe场景才可用canonical empty manifest表达，不能用“没查”冒充空。promotion只接受sealed receipt，并重新核对其member exact set；window结束后出现的新source必须进入下一枚receipt，不能修改旧窗口。

Plan A 独立拥有per-operation `tool_truth_revalidation_dispatch_held=true`、monotonic generation与既有`tool_truth_revalidation_dispatch_events`；Plan C 的 singleton 独立拥有 `campaign_dispatch_held=true`、`operation_admission_held=false` 及各自 generation。`00008`只为Plan C两scope添加`operation_rollout_safety_hold_events`，不复制Plan A event；D-owned local-admin coordinator按scope路由到owner repo，在同一transaction CAS对应held/generation/row version并写owner event。改变任一scope绝不递增或授权另两个scope。operation冻结`revalidation_policy=manual_only|auto_passive_t0_t1`：前者只允许人工显式revalidate，后者仅允许Plan A T0/T1被动、无目标侧副作用的自动revalidation；T2/T3一律进入Plan C Prepared Action/JIT并受campaign dispatch hold，不得借revalidation policy绕过。初始部署不阻止rank 0 legacy operation创建；Tool Truth自动revalidation与Campaign external dispatch保持held。fork enforcement读取Plan B统一`operation_contract_adoptions`，不新建第二张adoption表。

`investigation_projection_compare_aggregates`、cohort members、所有readiness/health/probe/retirement/adversarial-acceptance receipt headers与members、promotion receipts/members、Plan A revalidation与Plan C safety-hold events、legacy attempt source members、legacy report authority seal/members、artifact metadata receipts/members、historical read attestations/members、report-input seal/members、report mutation receipts与authority invalidation events全部安装数据库级append-only/sealed trigger，拒绝UPDATE/DELETE并在seal后拒绝追加member；aggregate重算写新cohort row，不覆盖旧证据。migration test必须直接SQL尝试修改/删除/向sealed set追加并断言失败。

**Step 5：扩展 report seal 与 authority columns**

```sql
CREATE TABLE legacy_attempt_authority_receipts (
    receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    organization_id UUID,
    candidate_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    hypothesis_revision_id UUID NOT NULL,
    terminal_status TEXT NOT NULL CHECK (terminal_status IN ('verified','refuted')),
    source_record_hash TEXT NOT NULL CHECK (source_record_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_member_count BIGINT NOT NULL CHECK (source_member_count > 0),
    source_membership_hash TEXT NOT NULL CHECK (source_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence_membership_hash TEXT NOT NULL CHECK (evidence_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    finding_id UUID,
    refutation_receipt_id UUID,
    limitation_membership_hash TEXT NOT NULL CHECK (limitation_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    adapter_version TEXT NOT NULL,
    adapter_digest TEXT NOT NULL CHECK (adapter_digest ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, attempt_id, adapter_version),
    FOREIGN KEY (operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT,
    CHECK (
        (terminal_status='verified' AND finding_id IS NOT NULL AND refutation_receipt_id IS NULL)
        OR (terminal_status='refuted' AND finding_id IS NULL AND refutation_receipt_id IS NOT NULL)
    )
);

CREATE TABLE legacy_attempt_authority_source_members (
    receipt_id UUID NOT NULL REFERENCES legacy_attempt_authority_receipts(receipt_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    source_kind TEXT NOT NULL CHECK (source_kind IN (
        'candidate_snapshot','attempt_terminal','evidence','finding_lineage',
        'refutation_lineage','limitation'
    )),
    source_ref_id TEXT NOT NULL CHECK (BTRIM(source_ref_id) <> ''),
    source_ref_hash TEXT NOT NULL CHECK (source_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,source_kind,source_ref_id)
);

CREATE TABLE legacy_report_authority_seals (
    seal_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    organization_id UUID,
    authority_member_count BIGINT NOT NULL CHECK (authority_member_count > 0),
    authority_membership_hash TEXT NOT NULL CHECK (authority_membership_hash ~ '^sha256:[0-9a-f]{64}$'),
    final_scope_source_set_hash TEXT NOT NULL CHECK (final_scope_source_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE legacy_report_authority_members (
    seal_id UUID NOT NULL REFERENCES legacy_report_authority_seals(seal_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    legacy_attempt_authority_receipt_id UUID NOT NULL
        REFERENCES legacy_attempt_authority_receipts(receipt_id) ON DELETE RESTRICT,
    claim_semantic_key TEXT NOT NULL CHECK (BTRIM(claim_semantic_key) <> ''),
    claim_hash TEXT NOT NULL CHECK (claim_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(seal_id,ordinal),
    UNIQUE(seal_id,claim_semantic_key)
);

ALTER TABLE report_revision_artifacts
    ADD CONSTRAINT report_revision_artifacts_identity_content_unique
    UNIQUE(revision_id,artifact_kind,content_key);

CREATE TABLE legacy_report_artifact_receipts (
    receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    report_revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    artifact_member_count BIGINT NOT NULL CHECK (artifact_member_count > 0),
    artifact_manifest_hash TEXT NOT NULL CHECK (artifact_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    integrity_scope TEXT NOT NULL DEFAULT 'database_metadata_only'
        CHECK (integrity_scope='database_metadata_only'),
    adapter_version TEXT NOT NULL,
    adapter_digest TEXT NOT NULL CHECK (adapter_digest ~ '^sha256:[0-9a-f]{64}$'),
    migration_batch_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (report_revision_id),
    UNIQUE (receipt_id,report_revision_id),
    FOREIGN KEY (operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE legacy_report_artifact_receipt_members (
    receipt_id UUID NOT NULL REFERENCES legacy_report_artifact_receipts(receipt_id) ON DELETE RESTRICT,
    report_revision_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    artifact_kind TEXT NOT NULL CHECK (artifact_kind IN ('markdown','json','pdf','docx')),
    content_key TEXT NOT NULL CHECK (BTRIM(content_key) <> ''),
    content_key_hash TEXT NOT NULL CHECK (content_key_hash ~ '^sha256:[0-9a-f]{64}$'),
    artifact_sha256 TEXT NOT NULL CHECK (artifact_sha256 ~ '^[0-9a-f]{64}$'),
    artifact_byte_count BIGINT NOT NULL CHECK (artifact_byte_count >= 0),
    storage_locator_hash TEXT NOT NULL CHECK (storage_locator_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(receipt_id,ordinal),
    UNIQUE(receipt_id,artifact_kind),
    FOREIGN KEY(receipt_id,report_revision_id)
        REFERENCES legacy_report_artifact_receipts(receipt_id,report_revision_id) ON DELETE RESTRICT,
    FOREIGN KEY(report_revision_id,artifact_kind,content_key)
        REFERENCES report_revision_artifacts(revision_id,artifact_kind,content_key) ON DELETE RESTRICT,
    FOREIGN KEY(content_key)
        REFERENCES report_artifact_blobs(content_key) ON DELETE RESTRICT
);

CREATE TABLE historical_artifact_read_attestations (
    attestation_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    report_revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    historical_artifact_receipt_id UUID NOT NULL
        REFERENCES legacy_report_artifact_receipts(receipt_id) ON DELETE RESTRICT,
    stable_read_request_id UUID NOT NULL,
    expected_manifest_hash TEXT NOT NULL CHECK (expected_manifest_hash ~ '^sha256:[0-9a-f]{64}$'),
    expected_member_count BIGINT NOT NULL CHECK (expected_member_count > 0),
    observed_member_count BIGINT NOT NULL CHECK (observed_member_count >= 0),
    observed_manifest_hash TEXT CHECK (
        observed_manifest_hash IS NULL OR observed_manifest_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    root_locator_hash TEXT NOT NULL CHECK (root_locator_hash ~ '^sha256:[0-9a-f]{64}$'),
    pre_snapshot_identity_hash TEXT NOT NULL CHECK (pre_snapshot_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    post_snapshot_identity_hash TEXT NOT NULL CHECK (post_snapshot_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    request_private_snapshot_hash TEXT CHECK (
        request_private_snapshot_hash IS NULL OR request_private_snapshot_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    integrity_status TEXT NOT NULL CHECK (
        integrity_status IN ('consistent','missing','hash_mismatch','length_mismatch','unreadable')
    ),
    attestation_hash TEXT NOT NULL CHECK (attestation_hash ~ '^sha256:[0-9a-f]{64}$'),
    attested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(operation_id,stable_read_request_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TABLE historical_artifact_read_attestation_members (
    attestation_id UUID NOT NULL
        REFERENCES historical_artifact_read_attestations(attestation_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    artifact_kind TEXT NOT NULL CHECK (artifact_kind IN ('markdown','json','pdf','docx')),
    content_key_hash TEXT NOT NULL CHECK (content_key_hash ~ '^sha256:[0-9a-f]{64}$'),
    pre_file_identity_hash TEXT CHECK (pre_file_identity_hash IS NULL OR pre_file_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    post_file_identity_hash TEXT CHECK (post_file_identity_hash IS NULL OR post_file_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    pre_size BIGINT CHECK (pre_size IS NULL OR pre_size >= 0),
    post_size BIGINT CHECK (post_size IS NULL OR post_size >= 0),
    pre_mtime_ns BIGINT,
    post_mtime_ns BIGINT,
    observed_sha256 TEXT CHECK (observed_sha256 IS NULL OR observed_sha256 ~ '^[0-9a-f]{64}$'),
    observed_byte_count BIGINT CHECK (observed_byte_count IS NULL OR observed_byte_count >= 0),
    integrity_status TEXT NOT NULL CHECK (
        integrity_status IN ('consistent','missing','hash_mismatch','length_mismatch','unreadable')
    ),
    PRIMARY KEY(attestation_id,ordinal),
    UNIQUE(attestation_id,artifact_kind)
);

CREATE TABLE report_revision_mutation_receipts (
    receipt_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    stable_request_id UUID NOT NULL,
    mutation_kind TEXT NOT NULL CHECK (mutation_kind IN ('begin','finalize','supersede')),
    request_hash TEXT NOT NULL CHECK (request_hash ~ '^sha256:[0-9a-f]{64}$'),
    report_revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    source_batch_id UUID NOT NULL REFERENCES investigation_projection_outbox_batches(batch_id) ON DELETE RESTRICT,
    response_hash TEXT NOT NULL CHECK (response_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(operation_id,mutation_kind,stable_request_id),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT
);

CREATE TYPE report_authority_invalidation_origin_v1 AS ENUM (
    'tool_truth_semantic_orphan','verification_authority_quarantine'
);

CREATE TABLE report_authority_invalidation_events (
    event_id UUID PRIMARY KEY,
    report_revision_id UUID NOT NULL REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    report_input_seal_id UUID NOT NULL,
    report_input_dependency_ordinal INTEGER NOT NULL CHECK (report_input_dependency_ordinal >= 0),
    origin_kind report_authority_invalidation_origin_v1 NOT NULL,
    tool_truth_orphan_reconciliation_id UUID,
    verification_quarantine_event_id UUID,
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'semantic_authority_orphaned','verification_authority_quarantined'
    )),
    superseding_report_revision_id UUID REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    source_batch_id UUID NOT NULL REFERENCES investigation_projection_outbox_batches(batch_id) ON DELETE RESTRICT,
    invalidated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(report_revision_id,report_input_seal_id,report_input_dependency_ordinal,origin_kind),
    FOREIGN KEY(tool_truth_orphan_reconciliation_id)
        REFERENCES capability_execution_reconciliations(id) ON DELETE RESTRICT,
    FOREIGN KEY(verification_quarantine_event_id)
        REFERENCES verification_authority_quarantine_events(id) ON DELETE RESTRICT,
    CHECK (num_nonnulls(tool_truth_orphan_reconciliation_id,verification_quarantine_event_id)=1),
    CHECK (
        (origin_kind='tool_truth_semantic_orphan' AND tool_truth_orphan_reconciliation_id IS NOT NULL
            AND verification_quarantine_event_id IS NULL
            AND reason_code='semantic_authority_orphaned')
        OR (origin_kind='verification_authority_quarantine' AND verification_quarantine_event_id IS NOT NULL
            AND tool_truth_orphan_reconciliation_id IS NULL
            AND reason_code='verification_authority_quarantined')
    )
);

CREATE TABLE report_input_seals (
    seal_id UUID PRIMARY KEY,
    report_revision_id UUID NOT NULL UNIQUE
        REFERENCES report_revisions(revision_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    project_scope_id UUID NOT NULL,
    seal_state TEXT NOT NULL CHECK (seal_state IN ('open','sealed')),
    authority_contract TEXT NOT NULL CHECK (authority_contract IN ('revision_adjudication_v1','legacy_report_v1')),
    tool_truth_authority_bundle_id UUID NOT NULL,
    tool_truth_authority_bundle_hash TEXT NOT NULL CHECK (tool_truth_authority_bundle_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_relevant_root_count BIGINT NOT NULL CHECK (tool_truth_relevant_root_count > 0),
    tool_truth_relevant_root_set_hash TEXT NOT NULL CHECK (tool_truth_relevant_root_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_relevant_member_count BIGINT NOT NULL CHECK (tool_truth_relevant_member_count > 0),
    tool_truth_relevant_member_set_hash TEXT NOT NULL CHECK (tool_truth_relevant_member_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_semantic_authority_hash TEXT NOT NULL CHECK (tool_truth_semantic_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_freshness_authority_hash TEXT NOT NULL CHECK (tool_truth_freshness_authority_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_temporal_validity_hash TEXT NOT NULL CHECK (tool_truth_temporal_validity_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_epoch_hash TEXT NOT NULL CHECK (tool_truth_epoch_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_observation_window_hash TEXT NOT NULL CHECK (tool_truth_observation_window_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_effective_validity_hash TEXT NOT NULL CHECK (tool_truth_effective_validity_hash ~ '^sha256:[0-9a-f]{64}$'),
    tool_truth_effective_valid_until TIMESTAMPTZ NOT NULL,
    tool_truth_observed_at TIMESTAMPTZ NOT NULL,
    tool_truth_validity_status TEXT NOT NULL CHECK (tool_truth_validity_status='all_fresh'),
    generation_seal_id UUID,
    generation_seal_hash TEXT CHECK (
        generation_seal_hash IS NULL OR generation_seal_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    wave_terminal_kind TEXT CHECK (wave_terminal_kind IN ('consolidation','fixed_point')),
    consolidation_receipt_id UUID,
    consolidation_receipt_hash TEXT CHECK (
        consolidation_receipt_hash IS NULL OR consolidation_receipt_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    fixed_point_receipt_id UUID,
    fixed_point_receipt_hash TEXT CHECK (
        fixed_point_receipt_hash IS NULL OR fixed_point_receipt_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    final_wave_coverage_receipt_id UUID,
    final_wave_coverage_receipt_hash TEXT CHECK (
        final_wave_coverage_receipt_hash IS NULL OR final_wave_coverage_receipt_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    legacy_report_authority_seal_id UUID
        REFERENCES legacy_report_authority_seals(seal_id) ON DELETE RESTRICT,
    legacy_report_authority_seal_hash TEXT CHECK (
        legacy_report_authority_seal_hash IS NULL OR legacy_report_authority_seal_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    final_scope_source_set_hash TEXT CHECK (
        final_scope_source_set_hash IS NULL OR final_scope_source_set_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_member_count BIGINT CHECK (source_member_count IS NULL OR source_member_count > 0),
    source_set_hash TEXT CHECK (source_set_hash IS NULL OR source_set_hash ~ '^sha256:[0-9a-f]{64}$'),
    coverage_membership_hash TEXT CHECK (
        coverage_membership_hash IS NULL OR coverage_membership_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    residual_membership_hash TEXT CHECK (
        residual_membership_hash IS NULL OR residual_membership_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    limitation_membership_hash TEXT CHECK (
        limitation_membership_hash IS NULL OR limitation_membership_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    mandatory_limitation_code TEXT CHECK (
        mandatory_limitation_code IS NULL OR mandatory_limitation_code='legacy_coverage_unavailable'
    ),
    report_input_hash TEXT CHECK (
        report_input_hash IS NULL OR report_input_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sealed_at TIMESTAMPTZ,
    UNIQUE(seal_id,report_revision_id,authority_contract),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    CHECK (
        (seal_state='open' AND source_member_count IS NULL AND source_set_hash IS NULL
            AND report_input_hash IS NULL AND sealed_at IS NULL)
        OR (seal_state='sealed' AND source_member_count > 0 AND source_set_hash IS NOT NULL
            AND report_input_hash IS NOT NULL AND sealed_at IS NOT NULL)
    ),
    CHECK (
        (authority_contract='revision_adjudication_v1'
            AND generation_seal_id IS NOT NULL AND generation_seal_hash IS NOT NULL
            AND final_wave_coverage_receipt_id IS NOT NULL
            AND final_wave_coverage_receipt_hash IS NOT NULL
            AND ((wave_terminal_kind='consolidation' AND consolidation_receipt_id IS NOT NULL
                    AND consolidation_receipt_hash IS NOT NULL
                    AND fixed_point_receipt_id IS NULL AND fixed_point_receipt_hash IS NULL)
                OR (wave_terminal_kind='fixed_point' AND fixed_point_receipt_id IS NOT NULL
                    AND fixed_point_receipt_hash IS NOT NULL
                    AND consolidation_receipt_id IS NULL AND consolidation_receipt_hash IS NULL))
            AND ((seal_state='open' AND coverage_membership_hash IS NULL
                    AND residual_membership_hash IS NULL)
                OR (seal_state='sealed' AND coverage_membership_hash IS NOT NULL
                    AND residual_membership_hash IS NOT NULL))
            AND legacy_report_authority_seal_id IS NULL
            AND legacy_report_authority_seal_hash IS NULL
            AND final_scope_source_set_hash IS NULL
            AND limitation_membership_hash IS NULL
            AND mandatory_limitation_code IS NULL)
        OR (authority_contract='legacy_report_v1'
            AND generation_seal_id IS NULL AND generation_seal_hash IS NULL
            AND wave_terminal_kind IS NULL
            AND consolidation_receipt_id IS NULL AND consolidation_receipt_hash IS NULL
            AND fixed_point_receipt_id IS NULL AND fixed_point_receipt_hash IS NULL
            AND final_wave_coverage_receipt_id IS NULL
            AND final_wave_coverage_receipt_hash IS NULL
            AND coverage_membership_hash IS NULL
            AND residual_membership_hash IS NULL
            AND legacy_report_authority_seal_id IS NOT NULL
            AND legacy_report_authority_seal_hash IS NOT NULL
            AND ((seal_state='open' AND final_scope_source_set_hash IS NULL)
                OR (seal_state='sealed' AND final_scope_source_set_hash IS NOT NULL))
            AND ((seal_state='open' AND limitation_membership_hash IS NULL)
                OR (seal_state='sealed' AND limitation_membership_hash IS NOT NULL))
            AND mandatory_limitation_code='legacy_coverage_unavailable')
    )
);

CREATE TYPE report_input_dependency_kind_v1 AS ENUM (
    'verification_plan_seal','proof_path','claim_component',
    'revision_adjudication','revision_terminal_decision','objective_outcome',
    'finding_lineage','refutation_receipt',
    'campaign_objective_audit','prepared_action_execution','oracle_assessment',
    'final_wave_coverage','hypothesis_residual','legacy_report_authority'
);

CREATE TABLE report_input_seal_members (
    seal_id UUID NOT NULL REFERENCES report_input_seals(seal_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID,
    dependency_kind report_input_dependency_kind_v1 NOT NULL,
    verification_plan_seal_id UUID,
    proof_path_id UUID,
    claim_component_id UUID,
    revision_adjudication_id UUID,
    revision_terminal_decision_id UUID,
    objective_outcome_id UUID,
    finding_id UUID,
    refutation_receipt_id UUID,
    campaign_objective_audit_id UUID,
    prepared_action_execution_id UUID,
    oracle_assessment_id UUID,
    final_wave_coverage_receipt_id UUID,
    hypothesis_residual_id UUID,
    legacy_report_authority_seal_id UUID,
    source_version BIGINT NOT NULL CHECK (source_version >= 0),
    dependency_hash TEXT NOT NULL CHECK (dependency_hash ~ '^sha256:[0-9a-f]{64}$'),
    authority_class TEXT NOT NULL CHECK (authority_class IN (
        'security_verdict_authority','grandfathered_legacy_security_verdict','coverage_authority',
        'execution_observation_audit','method_audit_only','authorization_audit'
    )),
    tool_truth_bundle_member_id UUID NOT NULL,
    tool_truth_bundle_member_hash TEXT NOT NULL CHECK (tool_truth_bundle_member_hash ~ '^sha256:[0-9a-f]{64}$'),
    semantic_key_hash TEXT NOT NULL CHECK (semantic_key_hash ~ '^sha256:[0-9a-f]{64}$'),
    member_hash TEXT NOT NULL CHECK (member_hash ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY(seal_id,ordinal),
    UNIQUE(seal_id,dependency_kind,semantic_key_hash,source_version),
    UNIQUE(seal_id,semantic_key_hash),
    FOREIGN KEY(operation_id,project_scope_id)
        REFERENCES operation_state(operation_id,project_scope_id) ON DELETE RESTRICT,
    CHECK (num_nonnulls(
        verification_plan_seal_id,proof_path_id,claim_component_id,
        revision_adjudication_id,revision_terminal_decision_id,objective_outcome_id,
        finding_id,refutation_receipt_id,
        campaign_objective_audit_id,prepared_action_execution_id,oracle_assessment_id,
        final_wave_coverage_receipt_id,hypothesis_residual_id,legacy_report_authority_seal_id
    )=1),
    CHECK (
        CASE dependency_kind
            WHEN 'verification_plan_seal' THEN verification_plan_seal_id IS NOT NULL
            WHEN 'proof_path' THEN proof_path_id IS NOT NULL
            WHEN 'claim_component' THEN claim_component_id IS NOT NULL
            WHEN 'revision_adjudication' THEN revision_adjudication_id IS NOT NULL
            WHEN 'revision_terminal_decision' THEN revision_terminal_decision_id IS NOT NULL
            WHEN 'objective_outcome' THEN objective_outcome_id IS NOT NULL
            WHEN 'finding_lineage' THEN finding_id IS NOT NULL
            WHEN 'refutation_receipt' THEN refutation_receipt_id IS NOT NULL
            WHEN 'campaign_objective_audit' THEN campaign_objective_audit_id IS NOT NULL
            WHEN 'prepared_action_execution' THEN prepared_action_execution_id IS NOT NULL
            WHEN 'oracle_assessment' THEN oracle_assessment_id IS NOT NULL
            WHEN 'final_wave_coverage' THEN final_wave_coverage_receipt_id IS NOT NULL
            WHEN 'hypothesis_residual' THEN hypothesis_residual_id IS NOT NULL
            WHEN 'legacy_report_authority' THEN legacy_report_authority_seal_id IS NOT NULL
        END
    )
);

ALTER TABLE report_authority_invalidation_events
    ADD CONSTRAINT report_authority_invalidation_dependency_fk
    FOREIGN KEY(report_input_seal_id,report_input_dependency_ordinal)
    REFERENCES report_input_seal_members(seal_id,ordinal)
    DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE report_revisions
    ADD COLUMN report_authority_contract TEXT CHECK (
        report_authority_contract IN ('revision_adjudication_v1','legacy_report_v1','historical_artifact_v0')
    ),
    ADD COLUMN report_input_seal_id UUID,
    ADD COLUMN historical_artifact_receipt_id UUID
        REFERENCES legacy_report_artifact_receipts(receipt_id) ON DELETE RESTRICT,
    ADD CONSTRAINT report_revisions_input_seal_authority_fk
        FOREIGN KEY(report_input_seal_id,revision_id,report_authority_contract)
        REFERENCES report_input_seals(seal_id,report_revision_id,authority_contract) ON DELETE RESTRICT;

-- Before installing the permanent reject trigger, the migration creates exactly one
-- database-metadata receipt + exact member set for every pre-existing final revision,
-- verifies persisted count/hash/length/storage-locator metadata and ownership, then binds
-- the revision to that receipt. Filesystem bytes are deliberately not claimed by SQL.

ALTER TABLE report_revisions ADD CONSTRAINT report_revisions_final_input_seal_check CHECK (
    publication_status <> 'final'
    OR (
        (report_authority_contract='revision_adjudication_v1'
            AND report_input_seal_id IS NOT NULL
            AND historical_artifact_receipt_id IS NULL
        )
        OR (report_authority_contract='legacy_report_v1'
            AND report_input_seal_id IS NOT NULL
            AND historical_artifact_receipt_id IS NULL
        )
        OR (report_authority_contract='historical_artifact_v0'
            AND historical_artifact_receipt_id IS NOT NULL
            AND report_input_seal_id IS NULL)
    )
) NOT VALID;

ALTER TABLE report_source_manifest
    ADD COLUMN authority_class TEXT NOT NULL DEFAULT 'method_audit_only' CHECK (
        authority_class IN (
            'security_verdict_authority','grandfathered_legacy_security_verdict','coverage_authority',
            'execution_observation_audit','method_audit_only','authorization_audit',
            'historical_artifact_read_only'
        )
    );

ALTER TABLE report_claims
    ADD COLUMN authority_class TEXT NOT NULL DEFAULT 'method_audit_only' CHECK (
        authority_class IN (
            'security_verdict_authority','grandfathered_legacy_security_verdict','coverage_authority',
            'execution_observation_audit','method_audit_only','authorization_audit'
        )
    );

-- historical_artifact_read_only is intentionally valid only for source metadata;
-- report_claims omits it so historical bytes can never become a new typed claim.

ALTER TABLE report_sections DROP CONSTRAINT report_sections_section_kind_check;
ALTER TABLE report_sections ADD CONSTRAINT report_sections_section_kind_check CHECK (
    section_kind IN (
        'executive_summary','organization','findings','attack_paths',
        'coverage','residual_risk','cleanup_residuals','methodology','limitations'
    )
);
```

所有裸UUID在实际DDL中必须补operation/project/org compound unique/FK：legacy Candidate/Attempt/Hypothesis/Finding/refutation lineage、Plan B verification-plan/proof-path/claim-component、Plan C revision adjudication/terminal decision/latest objective outcomes、generation seal、consolidation/fixed-point receipt、最终Wave coverage receipt、Plan A `AllFreshToolTruthAuthorityBundle` header/relevant-root/member、report revision与artifact receipt都不能跨authority拼接。`report_input_seal_members`是closed tagged union，不允许`source_kind/source_id TEXT`或generic UUID：每个nullable typed ID都必须有`(id,operation,project,organization,dependency_hash)`compound FK到对应canonical table，`num_nonnulls + CASE`确保variant exact-one；`dependency_hash`只作同FK的一部分，不能独立充当authority。其`tool_truth_bundle_member_id/hash`还必须exact-one指向同一个bundle的relevant member。seal header的bundle/root/member/semantic/freshness/temporal/epoch/window/effective-validity hashes全部由Plan A opaque authority bundle复制并由FK/repo重算，caller不能传裸hash。`legacy_report_authority_seals`重读member exact count/hash，并验证每个security claim exact-one receipt且无额外receipt；operation必须是冻结rank 0–4 legacy/shadow/dual authority，rank 5/6拒绝legacy writer。`LegacyAttemptV1`读模型携带receipt ID/hash、source_record_hash与adapter version/digest。

`report_input_seals`与members必须走唯一typed lifecycle：在同一canonical transaction按固定锁序锁Plan A relevant-root authority heads → Plan C revision authority/quarantine heads → operation/source head → report revision，调用Plan A request-scoped guard得到server-derived `AllFreshToolTruthAuthorityBundle`，创建`open` header，按canonical source key写ordered members，重读DB内成员与latest objective outcome exact set，计算全部membership/report-input hash后原子close为`sealed`。open header不能finalize；sealed header/member拒绝UPDATE/DELETE/追加。stable request replay返回原seal；payload drift、并发source arrival、bundle member/epoch/window变化或少/多/重复member全部rollback并要求新draft。finalize/current/reuse在新transaction重走同一锁序和Plan A all-fresh guard，既不信旧`all_fresh`字符串，也不信caller时间。

migration必须在安装永久`historical_artifact_v0`写拒绝trigger前，对每个pre-existing final revision逐条构造artifact **database-metadata** member exact set；任一final artifact count=0、DB记录的hash/length/storage locator为空或ownership不一致时整个migration abort。纯SQL不能读取filesystem，因此这里严禁写“已重算bytes”的虚假attestation。

read-only adapter每次查看或下载旧bytes都必须以新的`stable_read_request_id`执行root-relative、no-follow、regular-file-only读取：先打开预注册artifact root目录handle，再逐path component拒绝绝对路径、`..`、symlink、junction/alias与非regular file；打开文件handle后记录pre identity/device+inode(or platform file id)/size/mtime，copy+hash到权限最小的request-private sealed snapshot，随后从原handle重读post identity/size/mtime。pre/post任一变化、member boundary/hash/length不符、snapshot seal失败都写typed unavailable attestation且零bytes返回。attestation、preview、render与download必须共同读取这份request-private snapshot的同一bytes，禁止attest原文件后重新按path打开。snapshot在request结束安全回收，只把hash/identity evidence落库。

历史artifact即使Tool Truth TTL已过仍可作为`temporally_stale/as_of`历史下载，但不能成为current/reuse authority；semantic orphan/quarantine才标`revoked`。可执行/主动内容默认`Content-Disposition: attachment`与`application/octet-stream`；允许preview的闭集格式必须使用独立sandbox origin/iframe、无script CSP、`nosniff`、strict MIME与下载文件名清洗，不能以内联HTML/SVG/Office active content运行。

已有`report_revisions_guard`会阻止final row普通UPDATE，所以`00008`必须先安装一个migration-only过渡guard：它只在当前migration transaction设置的不可伪造batch guard存在、NEW exact引用同batch创建的metadata receipt、且除新authority/seal列外所有OLD/NEW列`IS NOT DISTINCT FROM`时，允许既有final发生exactly-once metadata binding。backfill后在同一migration内立即替换为永久guard，永久拒绝新INSERT historical revision、draft→historical、二次绑定、字段漂移或historical进入Begin/finalize/export。测试必须覆盖clean DB、含既有final的upgraded DB、无guard普通UPDATE失败、过渡更新只改允许列以及永久guard恢复。existing artifact metadata与实际bytes均不由migration改写。

`report_revision_mutation_receipts`使begin/finalize/supersede exactly-once：同`operation + mutation_kind + stable_request_id`且request hash相同返回原revision/source batch/response；payload drift返回typed conflict。canonical report mutation与完整typed outbox batch及mutation receipt同事务，projector异步materialize；不再依赖随机event identity。

Plan D在`golish-db/src/repo/report_authority_invalidation.rs`提供唯一shared compound `invalidate_reports_for_source_authority_on(tx, source: ReportInvalidationSourceV1, stable_request)`，但不拥有A/C source transition。`ReportInvalidationSourceV1`是closed enum，仅含带typed compound identity/hash的`ToolTruthSemanticOrphan`与`VerificationAuthorityQuarantine`，不接受generic kind/UUID/reason string。Plan A capability-execution semantic orphan writer与Plan C authority-quarantine writer必须在各自原canonical transaction内调用该seam：按固定锁序锁source authority head → affected report revisions canonical order → operation/source outbox head，重读`report_input_seal_members`typed反向索引，append逐dependency invalidation event，并通过Plan B唯一helper写完整typed Report `Invalidate` whole-batch；event exact引用已验证的`(seal_id,dependency_ordinal)`与对应A/C origin typed FK，不重复自由source id/hash。任一步失败使A orphan/C quarantine与report invalidation/outbox一起rollback。相同stable request/hash exact replay原event/batch，payload drift拒绝。禁止post-commit listener、异步补偿或Plan D扫描器补写。

TTL/epoch/window自然到期不调用上述semantic invalidation seam，只令current/finalize/reuse强读返回`temporally_stale`并保留as-of历史下载；Plan A semantic orphan或Plan C quarantine才写revoked invalidation。same-semantic fresh bundle即使重新all-fresh也不得删除invalidation、复活旧revision或沿用旧revision adjudication；必须由H(g+1)产生新的revision-level adjudication/terminal decision和新report revision。

并发测试必须固定并证明锁序与结果：finalize和A orphan/C quarantine竞争时，只可能finalize先提交后被同一source transaction完整invalidate，或source transition先提交使finalize在seal前失败；绝不能出现orphan/quarantine已提交而report invalidation/outbox缺失，也不能死锁。注入event/member/outbox任一步失败验证全compound为零；response-loss replay不产生第二个event/batch。因为projection异步，`get current / export / reuse-as-source`必须强读canonical invalidation。历史artifact download在semantic revoke后仍只按明确的revoked-history policy与fresh file snapshot提供，绝不能作为current authority。

**Step 6：运行 GREEN 与 migration lint**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_projection_read_model --test investigation_rollout_migrations --test reporting_read_model_migrations --status-level fail)
just space-guard
(cd backend && cargo fmt -p golish-db -- --check)
```

Expected: migration 可从干净数据库应用；原子 rollback、immutable mode、nullable UI selector、report seal 与 compare uniqueness 测试全部通过。

### Future Commit

```bash
git add backend/crates/golish-db/migrations/20260729000008_investigation_projection.sql backend/crates/golish-db/tests/investigation_projection_read_model.rs backend/crates/golish-db/tests/investigation_rollout_migrations.rs backend/crates/golish-db/tests/reporting_read_model_migrations.rs
git commit -m "feat(db): add investigation projection schema"
```

## Task 3（D1）：实现 snapshot/version/change-sequence 基础与 legacy adapter

**Files:**

- Modify: `backend/crates/golish-db/src/repo/investigation_projection/mod.rs`
- Modify: `backend/crates/golish-db/src/repo/investigation_projection/types.rs`
- Create: `backend/crates/golish-db/src/repo/investigation_projection/version.rs`
- Modify: `backend/crates/golish-db/src/repo/investigation_projection/legacy.rs`
- Create: `backend/crates/golish-db/src/repo/legacy_security_verdict.rs`
- Modify: `backend/crates/golish-db/src/repo/mod.rs`
- Modify: `backend/crates/golish-db/tests/investigation_projection_read_model.rs`

**Step 1：写 snapshot 与 legacy RED**

新增测试固定：

1. query 开始后并发插入新 hypothesis，当前 page/as-of change sequence 不看见该行；
2. canonical write与完整typed outbox source batch在同一事务成功/rollback；source commit后、projector commit前，captured head仍只看到旧materialized versions；
3. valid cursor的expected change sequence落后返回typed stale，tampered cursor返回invalid；
4. legacy Candidate映射成只读Hypothesis；Attempt只形成`LegacyAttemptV1` observation/verdict adapter，不伪造Campaign、oracle或terminal receipt；
5. deleted live target 仍显示 at-time identity，但不可作为 mutation authority；
6. project/org/scope 不匹配 fail closed。
7. projector整batch写entity versions/change/timeline/compatibility receipt/head：任一步注入失败全部rollback但canonical+outbox仍在；重试后一次可见；
8. insert batch先commit、close batch后commit但close worker先claim时返回`PROJECTION_PREDECESSOR_PENDING`；随后按source_batch_seq重放并产生稳定change seq/event IDs；

```rust
#[tokio::test]
async fn legacy_attempt_never_synthesizes_campaign_or_oracle() {
    let fixture = Fixture::legacy_candidate_attempt().await;
    let view = investigation_projection::legacy::load_attempt_history(
        fixture.pool(),
        fixture.authority(),
        fixture.attempt_selector(),
    )
    .await
    .expect("legacy attempt history");

    assert_eq!(view.oracle, LegacyField::LegacyUnavailable);
    assert!(view.synthetic_campaign_id.is_none());
    assert!(view.campaign_terminal_receipt_id.is_none());
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_projection_read_model -E 'test(snapshot_|projection_change_|legacy_)' --status-level fail)
```

Expected: 因snapshot helper、typed legacy field、whole-batch projector/materialized-head isolation尚未实现而失败。

**Step 3：定义 projection envelope 与 snapshot helper**

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyField<T> {
    Available(T),
    LegacyUnavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionHead {
    pub projection_schema_version: u32,
    pub change_seq: i64,
    pub read_at: DateTime<Utc>,
    pub as_of_temporal_cutoff: Option<DateTime<Utc>>,
    pub authority_epoch_set_hash: [u8; 32],
    pub tool_truth_contract: ToolTruthContract,
    pub investigation_contract_version: InvestigationContractVersion,
    pub investigation_rollout_mode: InvestigationRolloutMode,
    pub mode_policy: InvestigationModePolicy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionPage<T, K> {
    pub head: ProjectionHead,
    pub items: Vec<ProjectionItem<T>>,
    pub next_sort_key: Option<K>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionItem<T> {
    pub head: ProjectionHead,
    pub authority_time: ProjectionAuthorityTimeV1,
    pub data: T,
}

pub struct ProjectionAuthorityTimeV1 {
    pub authority_ref_hash: [u8; 32],
    pub effective_valid_until: Option<DateTime<Utc>>,
    pub authority_epoch_hash: [u8; 32],
    pub observed_as_of: DateTime<Utc>,
    pub temporal_status: ProjectionTemporalStatusV1,
}

pub async fn begin_read_snapshot(
    pool: &PgPool,
    authority: &OperationReadAuthority,
    expected_change_seq: Option<i64>,
) -> Result<(Transaction<'_, Postgres>, ProjectionHead), InvestigationProjectionError>;
```

`begin_read_snapshot` 的第一条 transaction statement 必须是：

```sql
SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY;
```

随后在同一 snapshot join `operation_state`、scope snapshot、projection head与server-owned authority temporal index，读取 DB `transaction_timestamp()`作为既有`read_at`；每个authority time view把同一值暴露为`observed_as_of`。同时为完整filtered result authority universe计算minimum effective-valid-until cutoff与epoch-set hash。每个projection member携自身effective-valid-until/epoch/status。`projection_schema_version`固定为1；数据变化只推进`change_seq`，但时间到期不需要canonical write。若合法请求/cursor的expected change sequence、as-of temporal cutoff或epoch-set hash任一不匹配，或DB clock已越过cursor cutoff，都返回：

```rust
InvestigationProjectionError::Stale {
    expected_change_seq,
    current_change_seq,
    restart_required: true,
    reason: ProjectionStaleReason,
}
```

`ProjectionStaleReason`至少区分`ChangeSeqAdvanced / TemporalCutoffExpired / AuthorityEpochChanged`。只用DB clock，不接受client current time；旧page items仍可显示`observed_as_of + temporally_stale`，但不得继续拼接下一页。

**Step 4：复用Plan B typed whole-batch outbox/projector，不包装逐event helper**

Plan B已负责hypothesis/relation/generation/analysis的source batch，并冻结唯一`ProjectionEntityKind / ProjectionChangeKind / TimelineEventKind / ProjectionInvalidationReason`、`ProjectionEntityV1` payload、operation-local`source_batch_seq/predecessor_batch_id`与whole-batch helper。Plan C canonical writer已经通过同一helper写Campaign全量mutation manifest；Plan D Task 10同样为report lifecycle写完整batch。Plan D不得再定义`enqueue_change(entity_kind: &str, mutation_kind: &str)`或单event wrapper，也不得扩展第二套enum/catalog。

每个canonical transaction固定顺序为：锁operation/source head → 由server分配next source_batch_seq与predecessor → 根据B/C exhaustive mutation route生成typed、server-redacted、immutable payload member exact set → canonical-sort并冻结batch count/hash → canonical rows与batch header/members一起commit。canonical rollback时source head/outbox/canonical rows一起消失；canonical commit不写materialized entity version、change或projection head。

Plan D只扩展Plan B projector对Campaign/Report `ProjectionEntityV1` variant的typed materializer。projector只能claim某operation最小连续未完成batch；重读count/hash/predecessor与entity-version direct predecessor后，在一个短transaction写全部materialized entity versions、typed timeline changes、legacy compatibility versions、batch receipt，并一次CAS推进head。失败全部rollback且head不动；较晚batch不能越过较早batch。reader在`REPEATABLE READ READ ONLY` snapshot只读`change_seq <= captured head`的materialized versions，禁止join当前canonical table补N+1状态。

RED必须显式暂停projector：source N+1已commit/outbox pending时head N读不到N+1；projector commit后一次看到完整batch。另注入projector中途失败，断言entity version/change/compatibility receipt/head均不存在；按source_batch_seq重试/rebuild后manifest与deterministic event IDs byte-for-byte一致。

**Step 5：实现只读 legacy mapping**

`legacy.rs` 只查询旧 `attack_candidates`、approval、`candidate_attempts`、wave/fact-delta/finding-lineage rows，不写旧表。映射规则固定：

- Candidate -> read-only legacy Hypothesis view，保留source Candidate/hash；不插入canonical Registry row；
- verified Attempt 只有terminal状态、evidence exact-set和Finding lineage齐全时生成`SecurityVerdictAuthority::LegacyAttemptV1 { terminal_status: "verified", ... }`；refuted还必须精确绑定Candidate snapshot/revision并使用`terminal_status: "refuted"`；缺任一binding、blocked/pending/error均为Inconclusive/Observation+residual；
- Attempt history只显示legacy action observation与`legacy_unavailable`字段，不生成synthetic Campaign/round/oracle/terminal receipt；
- 所有legacy verdict自动带`legacy_coverage_unavailable`，永远不能满足Campaign coverage authority；legacy findings=0不能显示clean/complete；
- approval/recovery/resume -> legacy mutation availability；
- strategy、consult、concrete request packet、typed oracle 无可靠来源时为 `LegacyUnavailable`；
- queue position、lease、row version 只进入 audit payload；
- no-candidate/blocked/partial 保留原 reason/residual，不提升成 refuted。
- `new_only` 停止任何新的 legacy compatibility projection write 和 legacy mutation，但既有 compatibility/history row 继续以 `read_only=true` 受控读取；不能把“停止新写”误实现为“历史消失”。

`legacy_security_verdict.rs`生成append-only`legacy_attempt_authority_receipts + source members`；report builder按claim exact set生成`legacy_report_authority_seal + members`，而不是在revision上任选一个Attempt receipt。pre-existing finalized artifact的`legacy_report_artifact_receipts + members`只由Task 2 migration生成，runtime只能读取；保持原bytes/hash且不经新typed claim parser重解释。新report revision/export必须从typed legacy report authority seal重建。

synthetic id 使用 versioned namespace + canonical old row id 的 UUIDv5，保证 restore/replay 稳定；hash 不包含读取时间。

**Step 6：运行 GREEN 与 scoped clippy**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_projection_read_model -E 'test(snapshot_|projection_change_|legacy_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-db --test investigation_projection_read_model -- -D warnings)
```

Expected: snapshot、same-transaction change、legacy unavailable、ownership 与 deleted-target tests 全绿；无 warning。

### Future Commit

```bash
git add backend/crates/golish-db/src/repo/investigation_projection/mod.rs backend/crates/golish-db/src/repo/investigation_projection/types.rs backend/crates/golish-db/src/repo/investigation_projection/version.rs backend/crates/golish-db/src/repo/investigation_projection/legacy.rs backend/crates/golish-db/src/repo/legacy_security_verdict.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/tests/investigation_projection_read_model.rs
git commit -m "feat(investigation): build projection foundation"
```

## Task 4（D1）：扩展 Plan B summary/Hypotheses，并新增 Campaigns/Waves/Timeline 查询

**Files:**

- Modify: `backend/crates/golish-db/src/repo/investigation_projection/summary.rs`
- Modify: `backend/crates/golish-db/src/repo/investigation_projection/hypotheses.rs`
- Create: `backend/crates/golish-db/src/repo/investigation_projection/campaigns.rs`
- Create: `backend/crates/golish-db/src/repo/investigation_projection/timeline.rs`
- Modify: `backend/crates/golish-db/src/repo/investigation_projection/types.rs`
- Modify: `backend/crates/golish-db/src/repo/investigation_projection/mod.rs`
- Modify: `backend/crates/golish-db/tests/investigation_projection_read_model.rs`

**Step 1：写稳定分页与大集合 RED**

fixture 至少生成 1,025 hypotheses、3 generations、4 waves 和多个 Campaign round，覆盖：

- hypotheses 稳定 keyset pagination 无重复/遗漏；
- campaigns 按 wave/campaign ordinal 稳定排序；
- timeline 按 `(change_seq,event_id)`；
- filter digest 改变后旧 cursor 被拒绝；
- page 1 后有新 write 时 page 2 返回 stale/restart；
- page 1 后无任何write但DB clock跨过任一authority effective-valid-until时，page 2仍返回`INVESTIGATION_PROJECTION_STALE`；epoch变化同样stale；
- 每个rail/detail/timeline authority projection返回`observed_as_of / effective_valid_until / authority_epoch_hash / temporal_status`，temporally stale由server DB clock判定；
- detail lazy load 不把完整 evidence/raw request/prose 放在 rail DTO；
- summary 给出 generation/wave counts、control decision、coverage grade、open obligations；
- legacy 和 registry 使用同一 response shape。

```rust
#[tokio::test]
async fn page_two_rejects_projection_drift_instead_of_splicing_snapshots() {
    let fixture = Fixture::registry_with_hypotheses(120).await;
    let first = fixture.list_hypotheses(None, None, 50).await.expect("first page");
    fixture.insert_hypothesis("late-hypothesis").await;

    let error = fixture
        .list_hypotheses(
            first.next_sort_key,
            Some(first.head.change_seq),
            50,
        )
        .await
        .expect_err("stale page");

    assert_eq!(error.code(), "INVESTIGATION_PROJECTION_STALE");
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_projection_read_model -E 'test(summary_|hypotheses_|campaigns_|timeline_|page_)' --status-level fail)
```

Expected: 新 query modules 或稳定 pagination 尚未实现，focused tests 失败。

**Step 3：实现窄 rail/detail 类型和查询签名**

```rust
pub struct HypothesisListFilter {
    pub organization_ids: Vec<Uuid>,
    pub epistemic_states: Vec<String>,
    pub readiness_states: Vec<String>,
    pub capability_states: Vec<String>,
    pub source_kinds: Vec<String>,
}

pub struct HypothesisSortKey {
    pub organization_ordinal: i32,
    pub group_key: String,
    pub readiness_rank: i16,
    pub epistemic_rank: i16,
    pub root_id: Uuid,
    pub revision_ordinal: i32,
}

pub async fn list_hypotheses(
    pool: &PgPool,
    authority: &OperationReadAuthority,
    filter: &HypothesisListFilter,
    after: Option<&HypothesisSortKey>,
    expected_change_seq: Option<i64>,
    expected_temporal_cutoff: Option<DateTime<Utc>>,
    expected_authority_epoch_set_hash: Option<[u8; 32]>,
    limit: u16,
) -> Result<ProjectionPage<HypothesisRailItem, HypothesisSortKey>, InvestigationProjectionError>;

pub async fn get_hypothesis(
    pool: &PgPool,
    authority: &OperationReadAuthority,
    hypothesis_revision_id: Uuid,
) -> Result<ProjectionItem<HypothesisDetail>, InvestigationProjectionError>;
```

`expected_change_seq` 必须与 cursor 中的 `as_of_change_seq` 及 transaction 内读取的 head 完全一致；`expected_temporal_cutoff / expected_authority_epoch_set_hash`也必须与server在同一snapshot从完整filtered authority universe重算值一致，且DB `transaction_timestamp()`不得晚于cutoff。`projection_schema_version` 固定为 `1`，不承担 stale detection。`limit` 默认 50、最大 100；SQL 使用 tuple keyset 或等价的 fully ordered predicate，不使用 OFFSET。Campaign key 为 `(wave_ordinal,campaign_ordinal,campaign_id)`；timeline key 为 `(change_seq,event_id)`。

rail DTO 只包含 label/status/count/digest/short public summary；evidence、relations、round/action detail 由 detail API lazy load。Prepared Action 只返回 Plan C deterministic redacted projection，不返回 canonical args/credential/payload/checkpoint。

**Step 4：实现 summary/Waves 聚合**

summary 一次返回：

```rust
pub struct InvestigationSummary {
    pub observed_as_of: DateTime<Utc>,
    pub authority_time_members: Vec<ProjectionAuthorityTimeV1>,
    pub active_generation_id: Option<Uuid>,
    pub generations: Vec<GenerationSummary>,
    pub waves: Vec<WaveSummary>,
    pub hypothesis_counts: HypothesisCounts,
    pub campaign_counts: CampaignCounts,
    pub control_decision: String,
    pub coverage_grade: String,
    pub coverage_denominator: CoverageDenominator,
    pub coverage_sufficiency: CoverageSufficiencyProjection,
    pub open_obligations: Vec<OpenObligationSummary>,
}
```

`CoverageDenominator` 字段固定为 `planned / tested_complete / tested_degraded / untested / blocked`；总数不等式或 exact membership 不闭合时 read model 返回 authority-corrupt，而不是自行修正。

**Step 5：运行 GREEN、query regression 与 clippy**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_projection_read_model -E 'test(summary_|hypotheses_|campaigns_|timeline_|page_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-db --test investigation_projection_read_model -- -D warnings)
```

Expected: 1,025-item pagination、drift、filters、lazy detail、coverage exactness 与 legacy/registry shape tests 全绿；无 warning。

### Future Commit

```bash
git add backend/crates/golish-db/src/repo/investigation_projection/summary.rs backend/crates/golish-db/src/repo/investigation_projection/hypotheses.rs backend/crates/golish-db/src/repo/investigation_projection/campaigns.rs backend/crates/golish-db/src/repo/investigation_projection/timeline.rs backend/crates/golish-db/src/repo/investigation_projection/types.rs backend/crates/golish-db/src/repo/investigation_projection/mod.rs backend/crates/golish-db/tests/investigation_projection_read_model.rs
git commit -m "feat(investigation): add paged read models"
```

## Task 5（D1）：把 Plan B 的三个 read commands 扩展为完整六命令 contract

**Files:**

- Create: `backend/crates/golish-agent-app/src/ai/operation_authority.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/investigation/mod.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/investigation/dto.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/investigation/cursor.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/mod.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/mod.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/reporting.rs`
- Modify: `backend/crates/golish/src/commands_facade/investigation.rs`
- Modify: `backend/crates/golish/src/commands_facade/mod.rs`
- Modify: `backend/crates/golish/src/commands_registry.rs`
- Modify: `backend/crates/golish-agent-app/tests/investigation_ipc_authorization.rs`
- Modify: `backend/crates/golish-agent-app/tests/investigation_read_model.rs`
- Modify: `frontend/lib/api/investigation.ts`
- Verify unchanged from Plan B: `frontend/lib/generated/InvestigationProjectionEnvelope.ts`
- Verify unchanged from Plan B: `frontend/lib/generated/InvestigationModePolicyView.ts`
- Verify unchanged from Plan B: `frontend/lib/generated/InvestigationCommandError.ts`
- Verify unchanged from Plan B: `frontend/lib/generated/ProjectionEntityKind.ts`
- Verify unchanged from Plan B: `frontend/lib/generated/ProjectionInvalidationReason.ts`
- Verify unchanged from Plan B: `frontend/lib/generated/TimelineEventKind.ts`
- Verify unchanged from Plan B: `frontend/lib/generated/ProjectionSourceTimeStatusV1.ts`
- Generate: `frontend/lib/generated/InvestigationAuthorityTimeViewV1.ts`
- Generate: `frontend/lib/generated/InvestigationCampaignListRequest.ts`
- Generate: `frontend/lib/generated/InvestigationCampaignPageResponse.ts`
- Generate: `frontend/lib/generated/InvestigationCampaignListItemView.ts`
- Generate: `frontend/lib/generated/InvestigationCampaignDetailRequest.ts`
- Generate: `frontend/lib/generated/InvestigationCampaignDetailResponse.ts`
- Generate: `frontend/lib/generated/InvestigationCampaignDetailView.ts`
- Generate: `frontend/lib/generated/InvestigationTimelineListRequest.ts`
- Generate: `frontend/lib/generated/InvestigationTimelinePageResponse.ts`
- Generate: `frontend/lib/generated/InvestigationTimelineItemView.ts`

**生成类型授权暂停点：** 本 Task 会新增公开 ts-rs/IPC 类型。执行者必须先展示上述精确生成文件、Rust DTO 字段与兼容性 golden diff，取得用户明确授权后才修改 Rust DTO、导出 bindings 或 wrapper；未授权时停止在本 Task 前。

**Step 1：写授权、cursor 与 race RED**

保留 Plan B 对 `investigation_get_summary / investigation_list_hypotheses / investigation_get_hypothesis` 的测试，再为 Campaign/Timeline 扩展 local operator success、inactive/non-local operator、cross-project、cross-org、stale scope snapshot、opaque detail id 属于另一 operation、malformed operation/detail/filter ID、unknown或互斥filter、deleted live target mutation prohibition、cursor tamper/filter mismatch、bounded limit、stale version 和 typed error payload。另加“change_seq不变但TTL跨页到期”和“authority epoch变化”race：二者都必须由DB clock/epoch重算返回`INVESTIGATION_PROJECTION_STALE`，client clock无效。Timeline另外固定：按`change_seq,event_id`而非时钟排序；typed `event_kind`不由generic change猜测；新事件分别返回source/projected time；历史未知source time返回`null + historical_unknown`，绝不拿projected time补值；invalidation有typed reason。malformed ID必须稳定返回`INVESTIGATION_INVALID_ID`，unknown或互斥filter必须返回`INVESTIGATION_INVALID_ARGUMENT`，不能落成通用serde/Tauri错误或静默空页。

```rust
#[tokio::test]
async fn detail_id_from_another_operation_is_forbidden() {
    let fixture = IpcFixture::two_projects().await;
    let error = fixture
        .get_hypothesis(fixture.operation_a, fixture.operation_b_revision)
        .await
        .expect_err("cross-operation selector");

    assert_eq!(error.code, "INVESTIGATION_FORBIDDEN");
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_ipc_authorization --test investigation_read_model --status-level fail)
```

Expected: Plan B 的 summary/Hypothesis tests 继续通过；新增 Campaign/Timeline cases 因 DTO/handler/cursor branch 尚未存在而失败。

**Step 3：抽取 shared operation authority**

把 reporting 当前的 local operator/project/scope/org 检查移入 `ai/operation_authority.rs`，保持旧 reporting code 映射兼容：

```rust
pub struct OperationReadAuthority {
    pub principal_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scope_snapshot_hash: String,
    pub engagement_org_id: Uuid,
}

pub async fn authorize_operation_read(
    pool: &PgPool,
    principal: &OperatorPrincipal,
    operation_id: Uuid,
) -> Result<OperationReadAuthority, OperationAuthorityError>;
```

server 从 `operation_state` 与 frozen scope snapshot 推导全部 ownership。请求不能提交 project/org/scope 作为 authority。

**Step 4：保留 Plan B V1解码兼容，并为有时效authority的分页签发V2 cursor**

不得修改 `InvestigationCursorV1` bytes、签名算法、common envelope 或错误 enum。Plan B 已冻结的V1 payload包含 `resource_kind / operation_id / projection_schema_version / as_of_change_seq / tool_truth_contract / investigation_contract_version / investigation_rollout_mode / filter_digest / page_size / tagged stable_sort_key`。由于V1无法表达TTL/epoch，Plan D在同一versioned codec新增`InvestigationCursorV2`，在V1字段外加入server-derived `as_of_temporal_cutoff / authority_epoch_set_hash / observed_as_of`；所有含expiring authority的新第一页只签发V2。V1只允许继续读取明确`non_expiring_legacy`静态authority，否则返回stale并要求重启取得V2。本Task实现：

- Campaign key 与 `(wave_ordinal,campaign_ordinal,campaign_id)` 的双向转换；
- Timeline key 与 `(change_seq,event_id)` 的双向转换；
- resource/filter/page-size/operation 任一不匹配返回 `INVESTIGATION_CURSOR_INVALID`；
- 签名有效但current head的`change_seq`前进、authority epoch-set变化、或DB clock越过`as_of_temporal_cutoff`任一成立都返回`INVESTIGATION_PROJECTION_STALE`；
- 保留一枚 Plan B 生成、明确绑定`non_expiring_legacy` fixture的golden Hypothesis cursor，实施 Plan D 后必须原样解码并继续分页；同格式若指向expiring authority必须stale升级，不能因兼容性绕过时间门禁。

cursor 不是 bearer token；command 在 decode 前仍完成 operation authorization。签名比较继续使用 Plan B 的 constant-time 实现，D 不得降级成明文或无签名 cursor。

**Step 5：复用 Plan B envelope/error，并实现三个新增 command DTO**

以下 DTO 是本 Task 唯一新增的 IPC surface；每个 response 内嵌 Plan B `InvestigationProjectionEnvelope`，不复制或改名其字段：

```rust
#[derive(Clone, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvestigationCampaignListRequest {
    pub operation_id: String,
    pub wave_ids: Vec<String>,
    pub campaign_states: Vec<String>,
    pub cursor: Option<String>,
    pub expected_change_seq: Option<i64>,
    pub page_size: Option<u32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationCampaignListItemView {
    pub campaign_id: String,
    pub wave_ordinal: i32,
    pub campaign_ordinal: i32,
    pub label: String,
    pub state: String,
    pub coverage_status: String,
    pub authority_time: InvestigationAuthorityTimeViewV1,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationCampaignPageResponse {
    pub envelope: InvestigationProjectionEnvelope,
    pub campaigns: Vec<InvestigationCampaignListItemView>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvestigationCampaignDetailRequest {
    pub operation_id: String,
    pub campaign_id: String,
    pub expected_change_seq: Option<i64>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationCampaignDetailView {
    pub campaign_id: String,
    pub hypothesis_revision_id: String,
    pub wave_ordinal: i32,
    pub campaign_ordinal: i32,
    pub state: String,
    pub coverage_status: String,
    pub round_ids: Vec<String>,
    pub prepared_action_ids: Vec<String>,
    pub authorized_action_count: u64,
    pub blocked_action_count: u64,
    pub open_residual_ids: Vec<String>,
    pub redacted_round_summaries: Vec<String>,
    pub authority_time: InvestigationAuthorityTimeViewV1,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationCampaignDetailResponse {
    pub envelope: InvestigationProjectionEnvelope,
    pub campaign: InvestigationCampaignDetailView,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvestigationTimelineListRequest {
    pub operation_id: String,
    pub event_kinds: Vec<TimelineEventKind>,
    pub cursor: Option<String>,
    pub expected_change_seq: Option<i64>,
    pub page_size: Option<u32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationTimelineItemView {
    pub event_id: String,
    pub change_seq: i64,
    pub event_kind: TimelineEventKind,
    pub entity_kind: ProjectionEntityKind,
    pub entity_id: String,
    pub entity_version: u64,
    pub source_occurred_at: Option<String>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub projected_at: String,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
    pub authority_time: InvestigationAuthorityTimeViewV1,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationTimelinePageResponse {
    pub envelope: InvestigationProjectionEnvelope,
    pub events: Vec<InvestigationTimelineItemView>,
}
```

这些是公开IPC DTO，所以所有ID字段使用`String`/`Vec<String>`；handler内部才转换为UUID。closed enum字段必须直接复用Plan B导出的`TimelineEventKind / ProjectionEntityKind / ProjectionSourceTimeStatusV1 / ProjectionInvalidationReason`，不能退化为自由字符串。operation ID先解析并完成operation authority，detail selector随后解析并验证membership；filter ID逐项解析。空filter数组表示“不限制”，重复值去重排序，unknown enum或互斥组合返回`INVESTIGATION_INVALID_ARGUMENT`。该规则复用Plan B冻结的`InvestigationCommandError`，D不得让框架反序列化错误绕过稳定code。

Timeline唯一排序键是`(change_seq,event_id)`，`source_occurred_at`只用于说明authority事实何时发生，`projected_at`只用于诊断投影延迟，二者都不得参与排序、event identity或semantic hash。历史source没有可信时间时必须返回`source_occurred_at=null + source_time_status=historical_unknown`，禁止用migration/projector当前时间冒充事实发生时间。Timeline item不再暴露generic `change_kind`或自然语言summary：业务语义只来自Plan B typed `event_kind`，详情通过authorized entity API获取。

`InvestigationCampaignDetailView` 只包含 Task 4 冻结的 redacted campaign/round/action/coverage detail；两种 detail type 均不得包含 raw args、credential、payload、stdout/stderr 或任意 `serde_json::Value`。

Plan B 已注册前三项：

```rust
investigation_get_summary,
investigation_list_hypotheses,
investigation_get_hypothesis,
```

Plan D 只新增并注册以下三项，不重复注册前三项、不添加 glob：

```rust
investigation_list_campaigns,
investigation_get_campaign,
investigation_list_timeline,
```

list request 接收 typed filters、`cursor / expectedChangeSeq / pageSize`；V2 cursor自带`as_of_change_seq / as_of_temporal_cutoff / authority_epoch_set_hash / observed_as_of`。所有 response 复用 common envelope并为每个authority返回`InvestigationAuthorityTimeViewV1`；前端按 `(projectionSchemaVersion, changeSeq, authorityEpochHash, observedAsOf)`丢弃旧响应并展示`observed_as_of / temporally_stale`，但绝不以浏览器本地时钟自行过期或续命。`pageSize`与Plan B相同，server clamp为`1..=100`。

**Step 6：扩展 TS-RS 类型并实现三个新增 wrapper**

```ts
export async function investigationListCampaigns(
  request: InvestigationCampaignListRequest,
): Promise<InvestigationCampaignPageResponse> {
  return invoke("investigation_list_campaigns", { request });
}

export async function investigationGetCampaign(
  request: InvestigationCampaignDetailRequest,
): Promise<InvestigationCampaignDetailResponse> {
  return invoke("investigation_get_campaign", { request });
}

export async function investigationListTimeline(
  request: InvestigationTimelineListRequest,
): Promise<InvestigationTimelinePageResponse> {
  return invoke("investigation_list_timeline", { request });
}
```

Plan B 的三个 wrapper、`InvestigationProjectionEnvelope`、`InvestigationModePolicyView` 与 `InvestigationCommandError` 保持 golden 序列化无 breaking diff。全部 wrapper 使用 snake_case command 名，并在 `frontend/lib/api/investigation.ts` 内统一调用 `./client` 导出的 `invoke`；组件不得裸调 Tauri，IPC type 不得手写。

**Step 7：运行 GREEN、ts-rs drift 与 clippy**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_ipc_authorization --test investigation_read_model --status-level fail)
just space-guard
(cd backend && cargo test -p golish-agent-app export_bindings -- --nocapture)
just space-guard
(cd backend && cargo clippy -p golish-agent-app -p golish --all-targets -- -D warnings)
pnpm exec vitest run frontend/lib/api/client.test.ts frontend/lib/ai/models.generated.test.ts
pnpm typecheck
```

Expected: 总计六命令的授权/cursor/error tests 全绿；registry 仅新增三条且无重复；生成类型与 wrapper typecheck；Rust 无 warning。

### Future Commit

```bash
git add backend/crates/golish-agent-app/src/ai/operation_authority.rs backend/crates/golish-agent-app/src/ai/commands/investigation/mod.rs backend/crates/golish-agent-app/src/ai/commands/investigation/dto.rs backend/crates/golish-agent-app/src/ai/commands/investigation/cursor.rs backend/crates/golish-agent-app/src/ai/mod.rs backend/crates/golish-agent-app/src/ai/commands/mod.rs backend/crates/golish-agent-app/src/ai/commands/reporting.rs backend/crates/golish/src/commands_facade/investigation.rs backend/crates/golish/src/commands_facade/mod.rs backend/crates/golish/src/commands_registry.rs backend/crates/golish-agent-app/tests/investigation_ipc_authorization.rs backend/crates/golish-agent-app/tests/investigation_read_model.rs frontend/lib/api/investigation.ts frontend/lib/generated/InvestigationAuthorityTimeViewV1.ts frontend/lib/generated/InvestigationCampaignListRequest.ts frontend/lib/generated/InvestigationCampaignPageResponse.ts frontend/lib/generated/InvestigationCampaignListItemView.ts frontend/lib/generated/InvestigationCampaignDetailRequest.ts frontend/lib/generated/InvestigationCampaignDetailResponse.ts frontend/lib/generated/InvestigationCampaignDetailView.ts frontend/lib/generated/InvestigationTimelineListRequest.ts frontend/lib/generated/InvestigationTimelinePageResponse.ts frontend/lib/generated/InvestigationTimelineItemView.ts
git commit -m "feat(investigation): expose authorized read APIs"
```

## Task 6（D2）：建立最小 workspace selector、持久恢复与 Pane route

**Files:**

- Modify: `frontend/store/types/session.ts`
- Modify: `frontend/store/slices/session.ts`
- Modify: `frontend/store/slices/session-core.ts`
- Modify: `frontend/store/selectors/pane-leaf.ts`
- Create: `frontend/store/investigation-workspace.test.ts`
- Modify: `frontend/components/PaneContainer/PaneLeaf.tsx`
- Modify: `frontend/components/PaneContainer/PaneLeaf.lazy.test.tsx`
- Modify: `frontend/components/PaneContainer/PaneLeaf.memo.test.tsx`
- Modify: `backend/crates/golish-agent-app/src/conversation_store/mod.rs`
- Modify: `backend/crates/golish-agent-app/src/conversation_store/batch.rs`
- Modify: `frontend/lib/api/conversation-db.ts`
- Modify: `frontend/lib/workspace-storage.ts`
- Modify: `frontend/lib/conversation-db-sync.ts`
- Modify: `frontend/lib/conversation-db-sync.test.ts`
- Modify: `frontend/lib/terminal-restore.ts`
- Modify: `frontend/lib/terminal-restore.test.ts`

**Step 1：写 selector-only persistence RED**

测试必须证明：open workspace 只写 selector；refresh 不写 response data；save/load 恢复 operation/tab/selection；旧 terminal row 的 null selector 正常恢复；Pane route lazy load；关闭 workspace 不清除 canonical server data。

```ts
it("persists only workspace selection and refresh sequence", () => {
  store.getState().openInvestigationWorkspace("session-1", {
    operationId: "operation-1",
    defaultTab: "hypotheses",
  });
  store.getState().bumpInvestigationRefresh("session-1", 42);

  const selection = store.getState().sessions["session-1"].investigationWorkspace;
  expect(selection).toEqual({
    operationId: "operation-1",
    defaultTab: "hypotheses",
    refreshSeq: 42,
  });
  expect(selection).not.toHaveProperty("hypotheses");
  expect(selection).not.toHaveProperty("campaigns");
});
```

**Step 2：运行 RED**

```bash
pnpm exec vitest run frontend/store/investigation-workspace.test.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.test.ts frontend/components/PaneContainer/PaneLeaf.lazy.test.tsx frontend/components/PaneContainer/PaneLeaf.memo.test.tsx
```

Expected: detail mode、selector actions、terminal column mapping 或 lazy route 尚未实现而失败。

**Step 3：新增唯一 store shape 与 actions**

```ts
export type InvestigationWorkspaceTab =
  | "hypotheses"
  | "campaigns"
  | "waves"
  | "timeline";

export interface InvestigationWorkspaceSelection {
  operationId: string;
  defaultTab: InvestigationWorkspaceTab;
  selectedHypothesisId?: string;
  selectedCampaignId?: string;
  refreshSeq: number;
}

export type DetailViewMode =
  | "timeline"
  | "tool-detail"
  | "sub-agent-detail"
  | "investigation-workspace";
```

actions 精确为：

- `openInvestigationWorkspace(sessionId, selection)`；
- `selectInvestigationHypothesis(sessionId, revisionId)`；
- `selectInvestigationCampaign(sessionId, campaignId)`；
- `selectInvestigationTab(sessionId, tab)`；
- `bumpInvestigationRefresh(sessionId, observedChangeSeq)`，取 monotonic max；
- `closeInvestigationWorkspace(sessionId)`。

store 不定义 `setHypotheses`、`setCampaigns` 或 report DTO setter。

**Step 4：接通 terminal-state persistence**

Rust `TerminalStateRow`、single save、batch save 与 load SELECT 都加入 `investigation_workspace_json`。TypeScript `TerminalStateRow` 与 `PersistedTerminalData` 使用同一 nullable JSON shape；restore 只接受四个 tab、非空 operation id 和非负 refreshSeq，非法 JSON 返回 null 并保留 terminal。

```ts
export function restoreInvestigationWorkspace(
  value: unknown,
): InvestigationWorkspaceSelection | undefined {
  const parsed = investigationWorkspaceSchema.safeParse(value);
  return parsed.success ? parsed.data : undefined;
}
```

conversation fingerprint 必须包含 selector，确保 selection 变化会持久化；不得包含 API response。

**Step 5：挂载 lazy Pane route**

`PaneLeaf.tsx` 根据 `detailViewMode === "investigation-workspace"` lazy load Workspace，并传 `sessionId + selection`。与其他 detail route 一致隐藏 terminal input；route 未选择 operation 时显示 typed empty entry state，不猜最近一次 tool call。

**Step 6：运行 GREEN、Biome 与 typecheck**

```bash
pnpm exec vitest run frontend/store/investigation-workspace.test.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.test.ts frontend/components/PaneContainer/PaneLeaf.lazy.test.tsx frontend/components/PaneContainer/PaneLeaf.memo.test.tsx
pnpm exec biome check frontend/store/types/session.ts frontend/store/slices/session.ts frontend/store/slices/session-core.ts frontend/store/selectors/pane-leaf.ts frontend/store/investigation-workspace.test.ts frontend/components/PaneContainer/PaneLeaf.tsx frontend/lib/api/conversation-db.ts frontend/lib/workspace-storage.ts frontend/lib/conversation-db-sync.ts frontend/lib/terminal-restore.ts
pnpm typecheck
```

Expected: selector-only、restore、legacy null row 与 lazy route tests 全绿；Biome/typecheck 通过。

### Future Commit

```bash
git add frontend/store/types/session.ts frontend/store/slices/session.ts frontend/store/slices/session-core.ts frontend/store/selectors/pane-leaf.ts frontend/store/investigation-workspace.test.ts frontend/components/PaneContainer/PaneLeaf.tsx frontend/components/PaneContainer/PaneLeaf.lazy.test.tsx frontend/components/PaneContainer/PaneLeaf.memo.test.tsx backend/crates/golish-agent-app/src/conversation_store/mod.rs backend/crates/golish-agent-app/src/conversation_store/batch.rs frontend/lib/api/conversation-db.ts frontend/lib/workspace-storage.ts frontend/lib/conversation-db-sync.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.ts frontend/lib/terminal-restore.test.ts
git commit -m "feat(investigation): persist workspace routing"
```

## Task 7（D2）：实现 version-monotonic Workspace shell 与四个独立视图

**Files:**

- Create: `frontend/components/Engagement/InvestigationWorkspace/index.ts`
- Create: `frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/useInvestigationProjection.ts`
- Create: `frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/WavesTab.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/InvestigationTimelineTab.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/HypothesisDetail.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/InvestigationStaleBanner.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.test.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.test.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx`

**Step 1：写 bootstrap、race 与三态 RED**

测试必须独立覆盖 Hypotheses、Campaigns、Waves、Timeline 的 loading/error/empty，另外覆盖：

- mount 无 event 时仍调用 summary；
- restore 的 `refreshSeq` 为 0 时仍 bootstrap；
- change seq 12 的 response 先到、seq 11 后到，旧 response 不覆盖；
- stale error（change、temporal cutoff或epoch）保留旧 items、显示带server `observed_as_of / temporally_stale`的reload banner、清 cursor 后从第一页重启；浏览器本地时钟不参与authority判定；
- Hypothesis/Campaign detail 只在选择后 lazy load；
- 1,025 rail items 虚拟化，DOM 不同时挂载全部 items；
- 键盘上下移动、Enter 选择、tablist/tabpanel aria 关联；
- gen0 → FactDelta → gen1 与 Wave/fixed point link 可导航并恢复选择。

```tsx
it("keeps newer projection when an older request resolves last", async () => {
  const older = deferred<InvestigationHypothesisListView>();
  const newer = deferred<InvestigationHypothesisListView>();
  mockListHypotheses.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);

  render(<InvestigationWorkspace sessionId="session-1" selection={selection} />);
  fireEvent.click(screen.getByRole("button", { name: /refresh/i }));

  newer.resolve(hypothesisPage({ projectionSchemaVersion: 1, changeSeq: 12, label: "new" }));
  expect(await screen.findByText("new")).toBeVisible();

  older.resolve(hypothesisPage({ projectionSchemaVersion: 1, changeSeq: 11, label: "old" }));
  await waitFor(() => expect(screen.queryByText("old")).not.toBeInTheDocument());
});
```

**Step 2：运行 RED**

```bash
pnpm exec vitest run frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.test.tsx frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx
```

Expected: Workspace shell/hook/tabs 不存在，或 monotonic/stale/virtualization assertions 失败。

**Step 3：实现 monotonic projection coordinator**

```ts
export interface ProjectionStamp {
  projectionSchemaVersion: 1;
  changeSeq: number;
}

export interface ProjectionResource<T> {
  data?: T;
  stamp?: ProjectionStamp;
  status: "idle" | "loading" | "ready" | "error" | "stale";
  errorCode?: string;
  nextCursor?: string;
}

export function acceptsProjection(
  current: ProjectionStamp | undefined,
  incoming: ProjectionStamp,
): boolean {
  if (!current) return true;
  return incoming.projectionSchemaVersion === current.projectionSchemaVersion &&
    incoming.changeSeq >= current.changeSeq;
}
```

hook 为每个 resource 保留本地 component state；`projectionSchemaVersion !== 1` 返回 typed unsupported-schema error，不尝试比较大小。后端内部虽区分`ChangeSeqAdvanced|TemporalCutoffExpired|AuthorityEpochChanged`，IPC继续映射Plan B既有`INVESTIGATION_PROJECTION_STALE` shape，保证`InvestigationCommandError.ts`不变。UI收到该code时状态切为`stale`但不清data，立即以`cursor=undefined`从第一页重启；重启response按每authority返回server `observed_as_of / temporally_stale`并更新banner。不得携旧cutoff/epoch或用本地Date判断“又fresh”。request 使用 generation token/AbortController；unmount 后 response 不写 state。

event `refreshSeq` 变化只触发 summary/page refresh，不直接改变 stamp 或 canonical data。

**Step 4：实现四 tab 与 detail lazy load**

- Hypotheses：source/fact/gap tree、grouped rail、epistemic/readiness/capability filters、claim/evidence/gaps/lineage detail；
- Campaigns：header、team topology、round strategy timeline、redacted authorization packet、action/oracle、Finding/FactDelta/residual lineage；
- Waves：只消费 summary 中的 generation/wave/consolidation links，展示 `H(g) -> W(n) -> D(n) -> H(g+1)` 或 fixed point；
- Timeline：cursor page 展示 proposal/support/contradict/refine/split/merge/derive/verdict；
- 无 durable team artifact 时显示 `not recorded`，不能生成角色或 chain-of-thought；
- UUID/hash/row version 不作为标题，只在 Task 8 Audit drawer 出现。

Hypotheses/Campaigns rail 用现有 `@tanstack/react-virtual`：

```tsx
const rowVirtualizer = useVirtualizer({
  count: items.length,
  getScrollElement: () => railRef.current,
  estimateSize: () => 52,
  overscan: 8,
});
```

pagination 在接近尾部时请求 next cursor；同一 cursor 只允许一个 in-flight 请求，append 前验证 stamp 与 item id 去重。

**Step 5：运行 GREEN、accessibility、Biome 与 typecheck**

```bash
pnpm exec vitest run frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.test.tsx frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx
pnpm exec biome check frontend/components/Engagement/InvestigationWorkspace
pnpm typecheck
```

Expected: bootstrap、race、stale retain/restart、独立三态、virtualization、keyboard/aria 与 cross-generation navigation tests 全绿；Biome/typecheck 通过。

### Future Commit

```bash
git add frontend/components/Engagement/InvestigationWorkspace/index.ts frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.tsx frontend/components/Engagement/InvestigationWorkspace/useInvestigationProjection.ts frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.tsx frontend/components/Engagement/InvestigationWorkspace/WavesTab.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationTimelineTab.tsx frontend/components/Engagement/InvestigationWorkspace/HypothesisDetail.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationStaleBanner.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.test.tsx frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx
git commit -m "feat(frontend): build investigation workspace views"
```

## Task 8（D2/D4 UI policy）：接通 roadmap/history、legacy 三态与 Audit drawer

**Files:**

- Create: `frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.test.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.test.tsx`
- Modify: `frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.tsx`
- Create: `frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.test.tsx`
- Modify: `frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx`
- Modify: `frontend/components/Engagement/PendingPreparedActionPanel.tsx`
- Modify: `frontend/components/Engagement/PendingPreparedActionPanel.test.tsx`
- Modify: `frontend/components/AIChatPanel/StageProgressBar.tsx`
- Modify: `frontend/components/AIChatPanel/StageRow.tsx`
- Modify: `frontend/components/AIChatPanel/StageProgressBar.test.tsx`
- Modify: `frontend/components/AIChatPanel/AIChatPanel.tsx`
- Modify: `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- Modify: `frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx`
- Modify: `frontend/components/ToolCallDetailView/ToolCallDetailView.reporting.test.tsx`
- Modify: `frontend/components/Engagement/AttackCandidateReview.tsx`
- Modify: `frontend/components/Engagement/CandidateAttemptRows.tsx`
- Modify: `frontend/components/Engagement/CandidateVerificationProtocol.tsx`
- Modify: `frontend/services/ai-events/harness-handlers.ts`
- Modify: `frontend/services/ai-events/harness-handlers.test.ts`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/attack.rs`
- Modify: `backend/crates/golish/src/commands_facade/attack.rs`

**Step 1：写 deep-link、mode policy 与 event RED**

测试固定：

1. Candidate roadmap action 打开同一 route 的 Hypotheses；Verification 打开 Campaigns；
2. row expand 行为保持，workspace 是独立 button；
3. live/completed/restored stage-run 有 operation id 即可进入，不要求 selected tool、candidate hint 或 report hint；
4. ToolCallDetail 不再嵌入 Candidate/Report 主 read model，只保留 “Open Investigation Workspace”；
5. 前三种 mode 的 server-derived `modePolicy.allowLegacyMutation=true` 时显示旧 review/recovery/resume，后两种为 false 时不渲染且 backend command 拒绝；前端不重新编码五态矩阵；
6. `legacy_unavailable` 与 empty/zero/not-checked 文案有区别；
7. 主 DOM 不含 `Queue 7`/FIFO position；Audit drawer 才显示 queue/lease/row version/receipt/hash；
8. T0/T1 只显示 `PolicyDecisionAudit`，无人工 approve/reject；Plan C T2/T3 panel 迁入 Campaign detail 后仍 CAS/expiry/drift-safe；
9. cold start、terminal restore、漏 event、没有 selected tool 时，只要 operation/campaign 内存在 pending Prepared Action，Campaign detail 都能主动查询并显示 review panel；
10. duplicate/out-of-order/missed refresh event 只 monotonic bump refreshSeq；不同 operation event 不刷新当前 workspace。

```tsx
it.each([
  [legacyModePolicyFixture, true],
  [shadowModePolicyFixture, true],
  [dualModePolicyFixture, true],
  [registryAuthorityPolicyFixture, false],
  [newOnlyPolicyFixture, false],
])("renders the server policy without recreating the matrix", (modePolicy, visible) => {
  render(<LegacyInvestigationAdapter modePolicy={modePolicy} legacy={legacyFixture} />);
  const review = screen.queryByRole("button", { name: /review candidate/i });
  expect(Boolean(review)).toBe(visible);
});
```

**Step 2：运行 RED**

```bash
pnpm exec vitest run frontend/components/AIChatPanel/StageProgressBar.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.reporting.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.test.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.test.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/services/ai-events/harness-handlers.test.ts
```

Expected: direct route、legacy policy、Audit drawer 或 event monotonic behavior 尚未实现而失败。

**Step 3：接通 Candidate/Verification/history 入口**

StageRow 接收 server-persisted `operationId`，增加独立 route action：

```ts
const defaultWorkspaceTab =
  stage === "candidate" ? "hypotheses" : stage === "verification" ? "campaigns" : undefined;

if (defaultWorkspaceTab && operationId) {
  openInvestigationWorkspace(sessionId, {
    operationId,
    defaultTab: defaultWorkspaceTab,
  });
}
```

operation id 来自 durable stage-run/terminal selection，不从某条 ToolCall 或 review hint 猜测。AIChatPanel 的 completed/restored history 复用同一 action。ToolCallDetail 的 candidate/report branch 只渲染 route link；保留工具 evidence/analyst activity detail。

**Step 4：把 Plan C Prepared Action review 安全迁入 Campaign detail**

`CampaignDetail`把`operation_id + Some(campaign_id)`传给Plan C已冻结的pending Prepared Action API，并复用同一个`PendingPreparedActionPanel`、authorization decision API、CAS、expiry与projection-drift handling；不得修改generated request、复制review状态或decision wrapper。Plan C旧fallback继续传`campaign_id=None`。mount、restore和refresh hint都触发bounded bootstrap，event只是刷新提示，不是panel唯一来源。

迁移采用可证明的两步切换：先让 Campaign detail 与原 operation-scoped ToolCallDetail fallback 共用同一 API，并用 cold-start/restore/missed-event fixture 证明二者都能找到同一 pending action；再删除旧 ToolCallDetail mount，只留下 “Open Investigation Workspace” route。任何 fixture 无法从 Campaign detail 恢复 pending action 时，不得删除旧 mount。

T0/T1 继续只显示 policy audit；只有 Plan C 已判定为 T2/T3 且处于 `pending_human_authorization` 的 action 才出现 approve/reject。UI 不能从 label、tool 名或 payload 自行推断 tier。

**Step 5：实现 legacy adapter 与 backend mode guard**

adapter只读取common envelope内server-derived `modePolicy.allowLegacyMutation`、`modePolicy.canonicalWriter`与`modePolicy.comparePolicy`；不读取当前deployment default，也不在TypeScript中重新实现五态authority matrix。旧UI只在policy允许时挂载，Shadow/Compare badge由server policy的writer/compare字段渲染。缺失字段统一组件：

```tsx
interface LegacyFieldViewProps<T> {
  field:
    | { kind: "available"; value: T }
    | { kind: "legacy_unavailable" };
  render: (value: T) => React.ReactNode;
}

function LegacyFieldView<T>({ field, render }: LegacyFieldViewProps<T>) {
  return field.kind === "legacy_unavailable"
    ? <span data-state="legacy-unavailable">legacy_unavailable</span>
    : render(field.value);
}
```

`attack_review_candidates`、resume/recovery command 在 backend 再读取 operation frozen mode，并调用 Plan B `policy().allow_legacy_mutation`；false 时返回 `ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT`。前端隐藏不能替代该检查。

**Step 6：把 scheduler internals 收入 Audit drawer**

Hypothesis/Campaign 主卡不显示 queue number、lease/checkpoint、row version、receipt UUID/hash。Audit drawer 用 definition list 展示这些 typed audit fields；scheduler order 标注为 execution audit，不得标成 priority/coverage。drawer 默认关闭且仍不能显示 raw canonical args、credential 或 payload。

**Step 7：让 event 只更新 refresh hint**

```ts
if (
  event.operation_id &&
  session.investigationWorkspace?.operationId === event.operation_id
) {
  bumpInvestigationRefresh(
    sessionId,
    Math.max(session.investigationWorkspace.refreshSeq, event.change_seq ?? 0),
  );
}
```

handler 不创建 Hypothesis/Campaign/Report DTO。mount 总是主动 bootstrap，因此没有 event、漏 event 或 cold restore 仍可恢复。

**Step 8：运行 GREEN、backend guard、Biome 与 typecheck**

```bash
pnpm exec vitest run frontend/components/AIChatPanel/StageProgressBar.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.reporting.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.test.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.test.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/services/ai-events/harness-handlers.test.ts
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -E 'test(investigation_legacy_mutation_)' --status-level fail)
pnpm exec biome check frontend/components/AIChatPanel/StageProgressBar.tsx frontend/components/AIChatPanel/StageRow.tsx frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/Engagement/InvestigationWorkspace frontend/components/Engagement/PendingPreparedActionPanel.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/services/ai-events/harness-handlers.ts
pnpm typecheck
```

Expected: direct/restored entry、cold-start/missed-event Prepared Action 恢复、server-policy legacy mutation、后两态双层拒绝、legacy unavailable、Audit-only queue 与 refresh-hint tests 全绿。

### Future Commit

```bash
git add frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.tsx frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.test.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx frontend/components/Engagement/PendingPreparedActionPanel.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/AIChatPanel/StageProgressBar.tsx frontend/components/AIChatPanel/StageRow.tsx frontend/components/AIChatPanel/StageProgressBar.test.tsx frontend/components/AIChatPanel/AIChatPanel.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.reporting.test.tsx frontend/components/Engagement/AttackCandidateReview.tsx frontend/components/Engagement/CandidateAttemptRows.tsx frontend/components/Engagement/CandidateVerificationProtocol.tsx frontend/services/ai-events/harness-handlers.ts frontend/services/ai-events/harness-handlers.test.ts backend/crates/golish-agent-app/src/ai/commands/attack.rs backend/crates/golish/src/commands_facade/attack.rs
git commit -m "feat(investigation): route legacy and live workspace"
```

## Task 9（D3）：把 report source/claim 收紧成 typed authority 与 redacted projection

**Files:**

- Modify: `backend/crates/golish-reporting-domain/src/report.rs`
- Modify: `backend/crates/golish-reporting-domain/src/section.rs`
- Modify: `backend/crates/golish-reporting-domain/src/validation.rs`
- Modify: `backend/crates/golish-reporting-app/src/redaction.rs`
- Modify: `backend/crates/golish-reporting-app/src/read_model.rs`
- Modify: `backend/crates/golish-reporting-app/src/ports.rs`

**Step 1：写 authority lattice 与 raw sentinel RED**

纯领域测试至少覆盖：

- strategy/consult/method claim 不能产生 verified/refuted；
- authorization/execution/Campaign objective outcome 无论是否有 Campaign adjudication都只能 audit/limitation；
- 单个Campaign terminal不能创建revision Finding或SecurityVerdict；
- verified 缺Plan C revision-level adjudication/terminal decision、Plan B verification-plan/proof-path/claim-component exact set、latest objective outcome exact set或Finding lineage时拒绝constructor；
- refuted 缺 exact refutation contract 降级；
- legacy verified/refuted 只有在 typed `LegacyAttemptV1` receipt、exact evidence membership 与 lineage/revision binding 完整时才成为 grandfathered verdict，并始终携带 `legacy_coverage_unavailable`；
- blocked/pending/error/缺 evidence 的 legacy Attempt 只能生成 inconclusive observation + residual；
- coverage receipt 只证明declared denominator closure，不能证明global detection sufficiency；`ThreatCoverageProfileV1`缺失时强制`coverage_sufficiency=not_assessed`；
- recursive token/cookie/PII/raw response/payload sentinel 被拒绝；
- report-input canonical hash 与 input order 无关，对 authority/source/version/seal 变化敏感。

```rust
#[test]
fn method_and_single_action_observation_cannot_be_security_verdicts() {
    for authority in [
        ReportAuthorityClass::MethodAuditOnly,
        ReportAuthorityClass::AuthorizationAudit,
        ReportAuthorityClass::ExecutionObservationAudit,
    ] {
        let error = ReportClaim::new_security_verdict(authority, security_verdict_fixture())
            .expect_err("non-verdict authority");
        assert_eq!(error.code(), "report_security_verdict_authority_invalid");
    }
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-reporting-domain -p golish-reporting-app -E 'test(report_authority_|report_redaction_|report_input_)' --status-level fail)
```

Expected: authority enum、typed claim values 或 recursive forbidden-value checks 尚未实现而失败。

**Step 3：扩展 canonical source kinds 与 authority class**

`ReportSourceKind` 增加：

```rust
HypothesisRoot,
HypothesisRevision,
HypothesisEvent,
HypothesisRelation,
CandidateAnalysisSnapshot,
InputProcessingDisposition,
VerificationCampaign,
VerificationCampaignRound,
VerificationStrategyDecision,
PreparedAction,
PreparedActionAuthorization,
PreparedActionExecutionReceipt,
ActionOracleAssessment,
CampaignAdjudication,
CampaignTerminalReceipt,
CampaignObjectiveOutcome,
HypothesisVerificationPlanSeal,
HypothesisProofPathSet,
HypothesisClaimComponentSet,
HypothesisRevisionAdjudication,
HypothesisRevisionTerminalDecision,
RefutationContract,
FactDeltaConsumption,
HypothesisGenerationSeal,
EnrichmentObligation,
CapabilityAssessment,
OracleCensusReceipt,
FinalWaveCoverageReceipt,
LegacyAttemptAuthorityReceipt,
LegacyReportAuthoritySeal,
HistoricalArtifactReceipt,
AuthorityQuarantineEvent,
HypothesisResidual,
```

定义并序列化 exact authority：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportAuthorityClass {
    SecurityVerdictAuthority,
    GrandfatheredLegacySecurityVerdict,
    CoverageAuthority,
    ExecutionObservationAudit,
    MethodAuditOnly,
    AuthorizationAudit,
    HistoricalArtifactReadOnly,
}
```

`ReportSourceVersion` 和 `ReportClaim` 都携 `authority_class`，source-set/report-input hash 包含该字段。authority 由 server source mapper 设置，不能由 Agent 或前端传入。`CampaignAdjudication / CampaignTerminalReceipt / CampaignObjectiveOutcome`永远是objective-local audit source，不能获得`SecurityVerdictAuthority`。新链只有`HypothesisRevisionAdjudication + HypothesisRevisionTerminalDecision + HypothesisVerificationPlanSeal + exact proof paths/claim components/latest objective outcomes`完整compound能获得该authority。`GrandfatheredLegacySecurityVerdict` 仅允许 frozen legacy/shadow/dual operation 的 typed adapter 使用，不能满足 Campaign coverage authority，不能被 registry-authoritative/new-only 新写入。`FinalWaveCoverageReceipt`是唯一可生成declared Coverage section的source；局部Campaign coverage只能降为method/observation audit。`HistoricalArtifactReceipt`只允许`HistoricalArtifactReadOnly` metadata view，不能进入typed claim constructor。

**Step 4：用 typed claim projection 替代 arbitrary JSON**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityVerdictProjection {
    Verified,
    Refuted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageSufficiencyProjection {
    NotAssessed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "contract", rename_all = "snake_case")]
pub enum SecurityVerdictAuthority {
    RevisionAdjudicationV1 {
        verification_plan_seal_id: Uuid,
        verification_plan_seal_hash: String,
        proof_path_set_hash: String,
        claim_component_set_hash: String,
        revision_adjudication_id: Uuid,
        revision_adjudication_hash: String,
        revision_terminal_decision_id: Uuid,
        revision_terminal_decision_hash: String,
        latest_objective_outcome_member_count: u64,
        latest_objective_outcome_set_hash: String,
        finding_id: Option<Uuid>,
        refutation_receipt_id: Option<Uuid>,
    },
    LegacyAttemptV1 {
        candidate_id: Uuid,
        attempt_id: Uuid,
        legacy_attempt_authority_receipt_id: Uuid,
        legacy_attempt_authority_receipt_hash: String,
        legacy_report_authority_seal_id: Uuid,
        legacy_report_authority_seal_hash: String,
        legacy_contract_version: String,
        terminal_status: String,
        source_record_hash: String,
        evidence_membership_hash: String,
        adapter_version: String,
        adapter_digest: String,
        finding_id: Option<Uuid>,
        refutation_receipt_id: Option<Uuid>,
        limitation_codes: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportClaimValue {
    SecurityVerdict {
        verdict: SecurityVerdictProjection,
        hypothesis_revision_id: Uuid,
        authority: SecurityVerdictAuthority,
    },
    Coverage {
        final_wave_coverage_receipt_id: Uuid,
        final_wave_coverage_receipt_hash: String,
        denominator_id: Uuid,
        denominator_hash: String,
        planned: u64,
        tested_complete: u64,
        tested_degraded: u64,
        untested: u64,
        blocked: u64,
        residual_ids: Vec<Uuid>,
        coverage_sufficiency: CoverageSufficiencyProjection,
    },
    ObservationAudit {
        source_id: String,
        source_hash: String,
        provenance: String,
        outcome_code: String,
    },
    MethodAudit {
        method_code: String,
        disposition_code: String,
    },
    AuthorizationAudit {
        prepared_action_id: Uuid,
        risk_tier: String,
        decision_code: String,
        request_digest: String,
        policy_digest: String,
    },
    Limitation {
        reason_code: String,
        affected_input_ids: Vec<String>,
        residual_ids: Vec<Uuid>,
        owner_code: String,
        next_action_code: String,
    },
}
```

`CoverageSufficiencyProjection` V1闭集只有`NotAssessed`；未来只有另一个版本化设计实现并封存`ThreatCoverageProfileV1`的asset class × trust boundary × attack class × identity/role × discovery-source/negative-space exact matrix后才能扩展，不能在本计划里用tested count推断。禁止 `serde_json::Value` 作为 public `ReportClaim.value`；DB `object_value JSONB` 只存 `ReportClaimValue` 的 tagged serialization，并在读出时 fail-closed deserialize。raw witness 只通过 id/hash/provenance 引用。`SecurityVerdict`闭集只有`Verified|Refuted`：objective-local Campaign/legacy的`inconclusive|blocked|exhausted_with_residuals`只能映射为`ObservationAudit + Limitation/Residual`，绝不能进入Findings。新链validator强制重验Plan B verification plan/proof paths/claim components、Plan C exact latest objective outcome set、revision adjudication/terminal decision与同revision Finding/refutation lineage；verified时Finding exact-one且refutation为空，refuted时反之。任何单Campaign artifact或缺失member都拒绝constructor。`LegacyAttemptV1`仍必须强制`legacy_coverage_unavailable`并验证Candidate source hash、Attempt terminal、Hypothesis revision、evidence exact-set、report-level seal与Finding/refutation lineage。

**Step 5：把 redaction 改成 allowlist + recursive defense-in-depth**

source mapper 不选择 raw body/stdout/stderr/request/response/payload/credential 字段。`redaction.rs` 对 typed projection 再递归扫描，禁止以下 normalized key class：

```rust
const FORBIDDEN_REPORT_KEYS: &[&str] = &[
    "password", "secret", "token", "api_key", "private_key", "cookie",
    "authorization", "credential", "request_body", "response_body",
    "stdout", "stderr", "raw_request", "raw_response", "payload", "email",
    "phone", "session_id",
];
```

命中 forbidden key、known token fixture 或未经批准的 arbitrary string field 时返回 `report_projection_forbidden_value`；finalizer 不把错误值替换后继续发布。vault reference 也只允许 opaque reference hash，不显示可解析 secret path。

所有attacker-controlled string在进入typed claim后仍要经过统一sink-safe contract：先验证Unicode normalization form、逐字段/逐集合长度上限并拒绝bidi control、NUL、不可见方向覆盖与非法控制字符；保留原始canonical evidence只以id/hash引用，不把“清洗后文本”冒充原证据。HTML/DOM使用text node而非HTML injection；Markdown按block/inline/link context分别escape；JSON只经typed serializer；URL只允许closed scheme/host/route allowlist并重验percent-decoding后结果；CSV export逐cell防`= + - @`及leading tab/CR公式注入（安全前缀并附typed export policy version）。加入HTML/Markdown link breakout、Unicode homoglyph/bidi、超长字段、double-encoded URL、CSV formula与filename/header injection fixtures；任一sink校验失败就拒绝artifact，不做best-effort替换后发布。

**Step 6：运行 GREEN 与 scoped clippy**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-reporting-domain -p golish-reporting-app -E 'test(report_authority_|report_redaction_|report_input_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-reporting-domain -p golish-reporting-app --all-targets -- -D warnings)
```

Expected: authority lattice、typed projection、hash sensitivity 和 raw sentinel tests 全绿；无 warning。

### Future Commit

```bash
git add backend/crates/golish-reporting-domain/src/report.rs backend/crates/golish-reporting-domain/src/section.rs backend/crates/golish-reporting-domain/src/validation.rs backend/crates/golish-reporting-app/src/redaction.rs backend/crates/golish-reporting-app/src/read_model.rs backend/crates/golish-reporting-app/src/ports.rs
git commit -m "feat(reporting): type authority and redaction"
```

## Task 10（D3）：重建 canonical report source set、denominator 与 report-input seal

**Files:**

- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/reporting_gate.rs`
- Modify: `backend/crates/golish-reporting-app/src/renderer.rs`
- Modify: `backend/crates/golish-reporting-app/src/finalizer.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/reporting.rs`
- Modify: `backend/crates/golish-db/src/repo/report_revisions.rs`
- Modify: `backend/crates/golish-db/src/repo/report_source_manifest.rs`
- Modify: `backend/crates/golish-db/src/repo/report_sections.rs`
- Modify: `backend/crates/golish-db/src/repo/report_claims.rs`
- Create: `backend/crates/golish-db/src/repo/report_authority_invalidation.rs`
- Modify Plan A-owned: `backend/crates/golish-db/src/repo/capability_execution_receipts.rs`
- Modify Plan C-owned: `backend/crates/golish-db/src/repo/verification_campaigns.rs`
- Create: `backend/crates/golish-db/src/repo/historical_report_artifacts.rs`
- Modify: `backend/crates/golish/src/reporting_artifact_store.rs`
- Modify: `backend/crates/golish-agent-app/tests/reporting_authority.rs`
- Modify: `backend/crates/golish-agent-app/tests/reporting_ipc_authorization.rs`
- Modify: `backend/crates/golish-db/tests/reporting_read_model_migrations.rs`
- Modify Plan A-created: `backend/crates/golish-db/tests/capability_execution_receipts.rs`
- Modify Plan C-created: `backend/crates/golish-db/tests/verification_campaigns.rs`

**Step 1：写 source authority、seal 与 open-work RED**

DB/app tests 必须证明：

- no-candidate disposition、pending enrichment/capability gap、blocked/inconclusive/exhausted residual 都进入 manifest/limitations；
- scanner no-match 只成为 method observation；
- 单Campaign adjudication/terminal/objective outcome即使verified也只能产audit，不能产revision SecurityVerdict或Finding；
- revision-level security claim必须exact绑定Plan B verification plan/proof paths/claim components、Plan C latest objective outcome set、revision adjudication/terminal decision与Finding/refutation lineage；少/多/旧一项均拒绝；
- findings=0 仍建立 Coverage 与 Residual Risk section；
- denominator exact counts 与 gap membership 一致，但`ThreatCoverageProfileV1`缺失时`coverage_sufficiency=not_assessed`且所有“全覆盖/无漏洞/安全”文案fixture失败；
- `PASS_WITH_GAPS` 列 affected targets/techniques/inputs；
- active generation seal、wave consolidation/fixed-point result、report-input hash 全部绑定 revision；
- next wave、pending consolidation、callback、cleanup、recovery 任一 open 时只能 draft；
- report-input走open header→ordered exact members→seal；open、少/多/重复member、self-reported hash、并发late source或跨bundle member均不能final；
- seal/finalize/current/reuse必须绑定Plan A relevant-root `AllFreshToolTruthAuthorityBundle`的bundle/root/member/semantic/freshness/temporal/epoch/window/effective-validity hashes；caller不能传authority hash；
- TTL/epoch/window在无projection change时到期，current/reuse返回temporally stale并要求新H(g+1)/adjudication/report，历史artifact仍可as-of下载；semantic orphan/quarantine才revoked，same-semantic refresh不复活旧terminal/report；
- report revision create/finalize/supersede与`entity_kind=report`的projection outbox同事务提交；rollback不留outbox，response-loss replay不产生第二个event，projector只推进一次`change_seq`；
- begin/finalize/supersede的stable request同hash exact replay原mutation receipt/batch，payload drift拒绝；outbox member是typed frozen source snapshot，projector不回读live report row；
- existing final migration只封存DB artifact metadata，并通过受限过渡guard完成exactly-once authority binding；clean/upgraded fixture都通过，永久guard恢复后普通final UPDATE失败；
- historical read使用root-relative no-follow regular-file handle，pre/post identity/size/mtime稳定且copy+hash到request-private sealed snapshot后，attestation/render/download才读同一bytes；symlink/path traversal/swap race/MIME confusion/active-content inline均拒绝；
- Plan A orphan与Plan C quarantine的source canonical tx调用shared report invalidation seam，同tx写event+typed whole-batch outbox；rollback/replay/finalize竞态遵守固定锁序。projection未推进时current/export/reuse也立即strong-read拒绝；历史download按revoked-history policy和fresh snapshot处理；
- cross-project/org/stale scope/deleted live target read 继续 fail closed。

```rust
#[tokio::test]
async fn final_report_requires_closed_generation_and_exact_report_input() {
    let fixture = ReportingFixture::with_open_callback().await;
    let error = fixture.finalize_current_revision().await.expect_err("open callback");
    assert_eq!(error.code(), "report_finalization_open_work");

    fixture.close_callback_and_rebuild().await;
    let finalized = fixture.finalize_current_revision().await.expect("final report");
    assert_eq!(finalized.report_input_hash, fixture.current_report_input_hash().await);
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test reporting_authority --test reporting_ipc_authorization --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test reporting_read_model_migrations --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts --test verification_campaigns -E 'test(report_authority_invalidation_)' --status-level fail)
```

Expected: new source classes、denominator sections、seal columns 或 open-work Gate assertions 失败。

**Step 3：重建 source snapshot 与 claim seeds**

扩展 `current_reportable_source_snapshot_on`，从 Plan B/C canonical rows 生成完整 source exact-set。`report_claim_seeds_on` 使用 fixed mapping：

| Canonical source | Authority | 可生成内容 |
|---|---|---|
| Plan C revision adjudication + revision terminal decision + Plan B verification plan/proof paths/claim components + exact latest objective outcomes + Finding/refutation | `security_verdict_authority` | verified/refuted |
| Campaign adjudication/terminal/objective outcome + exact oracle/evidence | `method_audit_only` / `execution_observation_audit` | objective-local方法与观察审计，绝不终结revision |
| Frozen legacy Candidate + terminal Attempt + exact evidence/Finding or refutation lineage + per-Attempt receipt + report-level authority seal | `grandfathered_legacy_security_verdict` | typed legacy verified/refuted + mandatory coverage limitation |
| Final Wave denominator + final Wave coverage receipt + exact residual membership | `coverage_authority` | coverage/gaps |
| Prepared Action execution + typed observation + single-action oracle | `execution_observation_audit` | 做过/观察到的事实 |
| consult/strategy/critique/refinement | `method_audit_only` | 方法审计 |
| redacted Prepared Action + decision | `authorization_audit` | 授权审计 |

单Campaign从来不是security bundle，不能用“字段齐全”升级；它始终映射为objective-local audit/outcome。revision-level compound不完整时拒绝SecurityVerdict constructor并形成`ObservationAudit + Limitation`，不能回退Campaign verdict。旧 CandidateAttempt 只能经 Task 9 的 typed `LegacyAttemptV1` adapter：verified/refuted 必须有 exact authority receipt；blocked/pending/error/缺 evidence/lineage 一律降为 inconclusive observation + residual；所有 legacy claim 强制 `legacy_coverage_unavailable`，且 `findings=0` 不得渲染为 clean/complete coverage。

**Step 4：计算 deterministic report-input seal**

```rust
#[serde(tag = "authority_contract", rename_all = "snake_case")]
pub enum ReportInputSealV1 {
    RevisionAdjudication(RevisionAdjudicationReportInputSealV1),
    Legacy(LegacyReportInputSealV1),
}

#[serde(tag = "terminal_kind", rename_all = "snake_case")]
pub enum WaveTerminalReceiptRefV1 {
    Consolidation { receipt_id: Uuid, receipt_hash: [u8; 32] },
    FixedPoint { receipt_id: Uuid, receipt_hash: [u8; 32] },
}

pub struct AllFreshToolTruthAuthorityBundleRefV1 {
    pub bundle_id: Uuid,
    pub bundle_hash: [u8; 32],
    pub relevant_root_count: u64,
    pub relevant_root_set_hash: [u8; 32],
    pub relevant_member_count: u64,
    pub relevant_member_set_hash: [u8; 32],
    pub semantic_authority_hash: [u8; 32],
    pub freshness_authority_hash: [u8; 32],
    pub temporal_validity_hash: [u8; 32],
    pub epoch_hash: [u8; 32],
    pub observation_window_hash: [u8; 32],
    pub effective_validity_hash: [u8; 32],
    pub effective_valid_until: DateTime<Utc>,
}

pub struct RevisionAdjudicationReportInputSealV1 {
    pub tool_truth_authority: AllFreshToolTruthAuthorityBundleRefV1,
    pub generation_seal_id: Uuid,
    pub generation_seal_hash: [u8; 32],
    pub verification_plan_seal_id: Uuid,
    pub verification_plan_seal_hash: [u8; 32],
    pub proof_path_set_hash: [u8; 32],
    pub claim_component_set_hash: [u8; 32],
    pub revision_adjudication_id: Uuid,
    pub revision_adjudication_hash: [u8; 32],
    pub revision_terminal_decision_id: Uuid,
    pub revision_terminal_decision_hash: [u8; 32],
    pub latest_objective_outcome_member_count: u64,
    pub latest_objective_outcome_set_hash: [u8; 32],
    pub wave_terminal: WaveTerminalReceiptRefV1,
    pub final_wave_coverage_receipt_id: Uuid,
    pub final_wave_coverage_receipt_hash: [u8; 32],
    pub source_member_count: u64,
    pub source_set_hash: [u8; 32],
    pub coverage_membership_hash: [u8; 32],
    pub residual_membership_hash: [u8; 32],
    pub report_input_hash: [u8; 32],
}

pub struct LegacyReportInputSealV1 {
    pub tool_truth_authority: AllFreshToolTruthAuthorityBundleRefV1,
    pub legacy_report_authority_seal_id: Uuid,
    pub legacy_report_authority_seal_hash: [u8; 32],
    pub final_scope_source_set_hash: [u8; 32],
    pub source_member_count: u64,
    pub source_set_hash: [u8; 32],
    pub limitation_membership_hash: [u8; 32],
    pub mandatory_limitation_code: LegacyCoverageLimitationCode,
    pub report_input_hash: [u8; 32],
}

pub enum HistoricalAuthorityTimeStatusV0 {
    AsOfFresh,
    TemporallyStale,
    RevokedHistory,
}

pub struct HistoricalArtifactReadAuthorityV0 {
    pub historical_artifact_receipt_id: Uuid,
    pub metadata_manifest_hash: [u8; 32],
    pub current_read_attestation_id: Uuid,
    pub current_read_attestation_hash: [u8; 32],
    pub request_private_snapshot_hash: [u8; 32],
    pub authority_time_status: HistoricalAuthorityTimeStatusV0,
}
```

Revision-adjudication seal的security authority必须是revision-level adjudication/terminal decision，并与Plan B verification plan/proof paths/claim components及latest objective outcome exact set一致；任何Campaign terminal只能作为audit member。`wave_terminal`仍必须exact-one引用最终consolidation或fixed-point receipt，不能拿Campaign terminal或UI wave row代替；coverage必须是同generation最终`verification_wave_coverage_receipt`，局部Campaign receipt拒绝。Legacy seal必须绑定report-level exact claim authority set，不允许任取一个Attempt receipt代表整份报告。两种新report seal都必须绑定同request由Plan A guard产生的relevant-root `AllFreshToolTruthAuthorityBundleRefV1`与ordered`report_input_seal_members`。`historical_artifact_v0`不是`ReportInputSealV1`分支：它只能走read-only metadata/fresh snapshot attestation adapter，永远不能Begin/finalize/export成新revision。

所有hash使用canonical length-prefixed fields，排除rendered prose；语义hash与freshness/temporal/epoch/window/effective-validity hash分轴保存，不能把same-semantic refresh伪装成原authority仍current。`BeginReportRevision`在一个transaction创建open seal、写ordered exact members、重算后close；validate/finalize在新transaction重新加载current source exact-set、revision authority、final Wave/legacy authority和Plan A all-fresh bundle。late source、TTL/epoch/window变化或bundle member drift使旧draft temporally stale并要求H(g+1)新裁决/新revision，不能UPDATE final artifact或用same-semantic refresh复活。semantic orphan/Plan C quarantine调用shared invalidation compound写revoked；TTL自然过期不写revoked。

`BeginReportRevision`、finalize和显式supersede的request都必须携server-issued `stable_request_id`与typed payload；repo计算`request_hash`，相同ID+相同hash返回原`report_revision_mutation_receipt`，相同ID+不同hash返回typed conflict。每个canonical mutation在同一transaction调用Plan B唯一whole-batch helper，冻结完整`ProjectionSourceSnapshotV1`和Report `insert|close|supersede` member、分配operation-local source batch predecessor，并与mutation receipt一起提交；不得调用字符串`enqueue_change`、不得逐event post-commit写入、不得直接推进projection head。projector按Plan B规则异步整批materialize deterministic event。同一request replay复用原batch/event identity，rollback时canonical row/mutation receipt/outbox batch全为零。

**Step 5：实现 denominator、limitations 与 finalization Gate**

Coverage section 总计必须满足：

```rust
planned == tested_complete + tested_degraded + untested + blocked
```

每个 untested/degraded/blocked gap 含 residual id、exact affected input、reason、owner、next action。Coverage只能标`declared_coverage_complete|declared_coverage_with_gaps`；`ThreatCoverageProfileV1`未实现前强制`coverage_sufficiency=not_assessed`，`PASS_WITH_GAPS` 禁止转成 `PASS`，declared complete也不得文案化为全覆盖/无漏洞/安全。finalizer 在 transaction 内重新检查 next wave、pending consolidation、callback、cleanup/recovery、revision-level authority、Plan A all-fresh relevant-root bundle、authority quarantine 与report-input exactness；失败返回稳定 code 且不 attach artifact。

**Step 6：保留历史 final artifact，并让 renderer 只接受 typed redacted read model**

现有 `historical_artifact_v0` final revision 的 artifact bytes/hash 保持不可变，不用新 parser 重新解释旧 JSON。新 UI 只展示 typed artifact metadata 与受控旧查看入口；每次查看/下载按root-relative no-follow regular-file stable snapshot规则复制并hash到request-private sealed snapshot，attestation/render/download共用该bytes。active content默认attachment/octet-stream；preview只走sandbox+CSP+strict MIME闭集。任何新 revision/export 都从 `RevisionAdjudicationV1` 或 `LegacyAttemptV1` authority snapshot 重建，禁止复制任意历史 JSON 作为新 canonical claim。

Markdown/JSON renderer 删除 arbitrary JSON pretty-print branch，仅 match `ReportClaimValue`。security claim、coverage、limitation、audit 使用各自模板；raw evidence viewer link 只能携 evidence id/hash，不能内嵌值。所有attacker-controlled string执行normalization/length/bidi-control验证并按HTML/Markdown/JSON/URL/CSV sink分别escape；URL allowlist、double-decode、CSV formula、header/filename injection fixtures必须通过。

**Step 7：运行 GREEN、artifact sentinel 与 clippy**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test reporting_authority --test reporting_ipc_authorization --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test reporting_read_model_migrations --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test capability_execution_receipts --test verification_campaigns -E 'test(report_authority_invalidation_)' --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-reporting-domain -p golish-reporting-app -E 'test(report_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-agent-app -p golish-db -p golish-reporting-domain -p golish-reporting-app --all-targets -- -D warnings)
```

Expected: canonical source、authority downgrade、denominator、open-work、seal/revision staleness 与 Markdown/JSON sentinel tests 全绿；无 warning。

### Future Commit

```bash
git add backend/crates/golish-agent-app/src/ai/db_bridge/reporting.rs backend/crates/golish-agent-app/src/ai/db_bridge/reporting_gate.rs backend/crates/golish-reporting-app/src/renderer.rs backend/crates/golish-reporting-app/src/finalizer.rs backend/crates/golish-agent-app/src/ai/commands/reporting.rs backend/crates/golish-db/src/repo/report_revisions.rs backend/crates/golish-db/src/repo/report_source_manifest.rs backend/crates/golish-db/src/repo/report_sections.rs backend/crates/golish-db/src/repo/report_claims.rs backend/crates/golish-db/src/repo/report_authority_invalidation.rs backend/crates/golish-db/src/repo/capability_execution_receipts.rs backend/crates/golish-db/src/repo/verification_campaigns.rs backend/crates/golish-db/src/repo/historical_report_artifacts.rs backend/crates/golish-agent-app/tests/reporting_authority.rs backend/crates/golish-agent-app/tests/reporting_ipc_authorization.rs backend/crates/golish-db/tests/reporting_read_model_migrations.rs backend/crates/golish-db/tests/capability_execution_receipts.rs backend/crates/golish-db/tests/verification_campaigns.rs
git commit -m "feat(reporting): seal canonical report inputs"
```

## Task 11（D3 UI）：展示 coverage/residual truth 并证明 report DOM 无 raw data

**Files:**

- Modify: `frontend/lib/api/reporting.ts`
- Modify: `frontend/components/Engagement/ReportReadModelView.tsx`
- Modify: `frontend/components/Engagement/ReportReadModelView.test.tsx`
- Modify: `frontend/components/AIChatPanel/AIChatPanel.reporting.test.tsx`

**Step 1：写 DOM truth 与 sentinel RED**

测试 fixture 同时放入 `TOKEN_SENTINEL_7f4a`、cookie、email/phone、raw response、stdout/stderr、完整 payload 字符串，并断言它们不出现在 rendered DOM；再覆盖：

- findings=0 + declared coverage complete 与 findings=0 + untested 的视觉结果不同，但二者都显示`coverage_sufficiency=not_assessed`且不出现全覆盖/无漏洞/安全；
- `PASS_WITH_GAPS` 使用 warning/limited style，不使用完整 PASS green token；
- planned/tested-complete/tested-degraded/untested/blocked 都显示；
- exact residual affected inputs、owner、next action 可展开；
- method/authorization/observation与所有单Campaign outcome/terminal明确标 Audit，不能出现revision verified/refuted heading；
- `LegacyAttemptV1` 显示 `Legacy authority` 与 `Coverage unavailable` badge，不能伪装成 Campaign verified 或完整 coverage；
- `historical_artifact_v0` 只显示 immutable artifact metadata/受控旧查看入口，不把旧 arbitrary JSON 注入新 claim DOM；
- stale revision 保留旧 report 并提示 refresh；
- generic `JSON.stringify(claim.value)` 不存在。

```tsx
it("never renders report raw-data sentinels", () => {
  const { container } = render(<ReportReadModelView model={redactionSentinelFixture} />);
  const text = container.textContent ?? "";
  for (const sentinel of [
    "TOKEN_SENTINEL_7f4a",
    "session-cookie=secret",
    "person@example.test",
    "RAW_HTTP_RESPONSE_SENTINEL",
    "EXPLOIT_PAYLOAD_SENTINEL",
  ]) {
    expect(text).not.toContain(sentinel);
  }
});
```

**Step 2：运行 RED**

```bash
pnpm exec vitest run frontend/components/Engagement/ReportReadModelView.test.tsx frontend/components/AIChatPanel/AIChatPanel.reporting.test.tsx
```

Expected: coverage/residual layout、authority badge 或 sentinel protection assertions 失败。

**Step 3：实现 typed renderer**

`ReportReadModelView` 对生成的 tagged claim union 使用 exhaustive switch；Coverage/Residual Risk/Global Sufficiency独立展示。declared closure标签使用`Declared coverage complete|Declared coverage with gaps`，且`ThreatCoverageProfileV1`缺失时始终显示`Global sufficiency not assessed`；没有绿色“安全/无漏洞/全覆盖”状态。Audit claim（含所有单Campaign outcome/terminal）不进入 Findings list。`LegacyAttemptV1` 使用独立 legacy authority presentation 并强制 coverage-unavailable limitation；`historical_artifact_v0` 仅进入 immutable artifact metadata viewer。无法识别的 claim kind 显示 typed unsupported error，不回退 raw JSON。

```ts
function assertNever(value: never): never {
  throw new Error(`unsupported_report_claim_kind:${String(value)}`);
}
```

UI link 只向独立授权 evidence viewer 传 opaque evidence id；不把 source value 放进 title、aria-label、data attribute、hidden node 或 telemetry metadata。

**Step 4：运行 GREEN、DOM sentinel、Biome 与 typecheck**

```bash
pnpm exec vitest run frontend/components/Engagement/ReportReadModelView.test.tsx frontend/components/AIChatPanel/AIChatPanel.reporting.test.tsx
pnpm exec biome check frontend/lib/api/reporting.ts frontend/components/Engagement/ReportReadModelView.tsx frontend/components/Engagement/ReportReadModelView.test.tsx frontend/components/AIChatPanel/AIChatPanel.reporting.test.tsx
pnpm typecheck
```

Expected: findings=0 distinction、declared coverage/global sufficiency separation、denominator、revision-adjudication/Campaign-audit/Legacy/historical authority badge、temporal stale retain 与 raw DOM/output-sink sentinel tests 全绿；Biome/typecheck 通过。

### Future Commit

```bash
git add frontend/lib/api/reporting.ts frontend/components/Engagement/ReportReadModelView.tsx frontend/components/Engagement/ReportReadModelView.test.tsx frontend/components/AIChatPanel/AIChatPanel.reporting.test.tsx
git commit -m "feat(frontend): render coverage-safe reports"
```

## Task 12（D4）：实现 whole-record comparison、promotion/default advancement 与 fork enforcement

**Files:**

- Modify: `backend/crates/golish-db/src/repo/investigation_projection/comparison.rs`
- Create: `backend/crates/golish-db/src/repo/operation_default_rollout.rs`
- Modify: `backend/crates/golish-db/src/repo/tool_truth_rollout.rs`
- Modify Plan A-owned: `backend/crates/golish-db/src/repo/tool_truth_revalidation.rs`
- Modify: `backend/crates/golish-db/src/repo/investigation_rollout.rs`
- Modify: `backend/crates/golish-db/src/repo/operation_rollout.rs`
- Modify: `backend/crates/golish-db/src/repo/operation_state.rs`
- Modify: `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- Modify: `backend/crates/golish-db/src/repo/mod.rs`
- Modify: `backend/crates/golish-db/tests/investigation_rollout_migrations.rs`
- Create: `backend/crates/golish-agent-app/tests/investigation_legacy_replay.rs`
- Create: `backend/crates/golish/src/cli/investigation_rollout.rs`
- Modify: `backend/crates/golish/src/cli/args.rs`
- Modify: `backend/crates/golish/src/cli/mod.rs`

**Step 1：写 whole-record、promotion 与 replay RED**

覆盖：

- Plan B `comparison_record.v1` 的 field order/timestamp/surrogate id 不改变 hash；语义状态、membership、authority、residual/coverage 改变 hash；
- one field mismatch 使 whole record mismatch，reader 不从另一侧补字段；
- shadow divergence 只写审计并阻止 promotion，不改变 legacy runtime/Gate；
- dual divergence 不授权 Registry/Campaign execution；
- registry-authoritative divergence 不回滚 canonical verdict，但 compatibility consumer fail closed；
- cohort 未 closed、sample 少一条、missing/incomplete/corrupt/mismatch 非零、contract 混合均拒绝要求 cohort 的 promotion edge；
- 每条 edge 只接受 Task 1 对应 criteria；0→1、1→2、3→4、5→6 不被错误要求 positive comparison cohort，2→3 与 4→5 必须有 closed positive cohort；
- 0→1、1→2、5→6各自缺一个/多一个/篡改一个readiness member都拒绝；外层promotion只引用summary receipt但内部member exact set不闭合时仍拒绝；
- whole-record positive sample只能证明compatibility；仅有zero mismatch不能宣称detection correctness；
- 4→5 缺 explicit authoritative canary、Plan A all-fresh authority、Plan C action/receipt/oracle/coverage/revision adjudication/report dry-run或sealed adversarial acceptance corpus任一 exact evidence 时拒绝；
- adversarial corpus少/多/错一个known-vuln/safe/control-failure/soft404/WAF/dynamic/multi-role-IDOR/race/adapter-missing fixture，或expected verdict/residual不是独立预写，均拒绝4→5；
- caller 伪造 zero counts 无效，repo 在锁内重算；
- rank 6→7、6→6和任何future unknown contract均typed拒绝，事故hold不改变rank；
- deployment default advance 后既有 operation joint pair 不变，新 operation 同 transaction 冻结完整新 pair；
- concurrent create/promotion 只能读到完整旧 pair 或完整新 pair，不允许 torn contract；
- same-operation resume 精确保持 joint pair；
- fork 任一轴变化时必须验证 Plan B unified adoption receipt 的 source seal/adoption set hash，且最多前进一阶；
- Plan A `tool_truth_revalidation_dispatch`只停止该operation新的自动revalidation claim/send，Plan C `campaign_dispatch`只停止新的Campaign external dispatch，`operation_admission`只停止新operation；三者独立CAS/generation/event，任何一个on/off不改变另两个。默认revalidation与campaign为held、operation admission为off；
- operation冻结`manual_only|auto_passive_t0_t1` policy；即使auto，T2/T3也必须Plan C Prepared Action/JIT，Plan A hold release不能授权T2/T3；
- operation create与fork target和并发`operation_admission=on`遵守同一hold→rollout固定锁序：hold先提交则零新row，create先提交则row完整冻结；resume已有operation不受该hold阻断；
- maintenance CLI dry-run 只读；apply 必须用户再次明确授权，普通 Tauri command 不存在；
- 五态 legacy fixture replay 结果符合 Task 1 matrix；
- historical legacy fixtures 覆盖 verified、refuted、blocked、missing-evidence 与已有 final artifact：前两类只有完整 typed receipt才可 grandfather，blocked/missing降级，已有 artifact bytes/hash不变。

```rust
#[tokio::test]
async fn promotion_reloads_complete_cohort_instead_of_trusting_caller_counts() {
    let fixture = RolloutFixture::closed_cohort_with_one_mismatch().await;
    let error = fixture
        .promote_defaults(PromoteOperationDefaults {
            expected_tool_truth_row_version: fixture.tool_truth_row_version,
            expected_investigation_row_version: fixture.investigation_row_version,
            target_joint_rank: 3,
            principal_id: fixture.local_principal_id,
            reason: "closed shadow cohort".into(),
        })
        .await
        .expect_err("mismatch blocks promotion");

    assert_eq!(error.code(), "investigation_rollout_comparison_not_exact");
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_rollout_migrations --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_legacy_replay --status-level fail)
```

Expected: canonical comparator、aggregate/promotion repo、default advance 或 replay assertions 尚未实现而失败。

**Step 3：复用 Plan B canonical comparison record 与唯一 sample writer**

Plan B 已冻结 `golish_core::investigation_comparison::ComparisonRecordV1`、canonical serializer、golden hash 和 `investigation_projection::comparison::compare_and_record_v1`。本 Task 不新增 comparator type、hash version、sample table或writer；只把 Plan C 原先为 typed `not_available_plan_c` 的 Campaign/action/oracle/coverage字段从 canonical authority 填充为实际 typed state，再调用同一个 V1 writer。

每个 legacy/registry side 必须先生成完整 record或明确 `registry_missing / legacy_projection_missing / incomplete / authority_corrupt`，不能生成部分字段后 fallback。`diff_summary` 继续只保存 differing path、reason code与两侧 cardinality，不保存具体值。`registry_authoritative_legacy_projection` 可以记录 AuditOnly sample，但 compatibility read 发现 divergence 必须返回 `INVESTIGATION_LEGACY_PROJECTION_DIVERGED`。

**Step 4：从唯一 sample ledger 锁内重算 aggregate**

扩展 Plan B `comparison.rs` 对每个operation读取其各自`(operation_id, as_of_change_seq, record_kind, record_key)` exact set，先写immutable cohort member，再按operation id canonical sort member manifest并写Task 2 aggregate。它不得接受caller counts或一个全局change cutoff；duplicate、少一条、member cutoff/hash不一致、mixed joint rank/schema、missing/incomplete/corrupt/mismatch均显式进入aggregate state并阻止需要exact cohort的edge。golden test证明实施D后Plan B的V1 hash contract未变。

rank 2→3 aggregate来自无外部副作用的 closed shadow evaluation；rank 4→5 aggregate同时绑定 closed dual compatibility cohort、显式 authoritative canary manifest与sealed adversarial acceptance corpus。compatibility cohort不能替代该corpus；corpus的expected verdict/residual由独立fixture manifest在执行前冻结。shadow evaluator 不写 canonical Campaign/action/receipt，不能拿执行器、授权 broker、credential resolver、lease、journal 或 budget reservation。

**Step 5：实现唯一联合 default coordinator、逐边 criteria 与安全 hold**

```rust
pub async fn promote_operation_defaults(
    tx: &mut Transaction<'_, Postgres>,
    request: PromoteOperationDefaults,
) -> Result<OperationDefaultPromotionReceipt, OperationRolloutError>;
```

固定锁序与事务顺序：

1. 事务若涉及operation-specific revalidation hold/create/fork，先按operation UUID canonical order取得同一repo-owned transaction advisory coordination key，再锁已存在的Plan A `tool_truth_revalidation_dispatch_heads`；
2. 再锁Plan C `verification_campaign_safety_holds` singleton；
3. 再锁 `tool_truth_rollout` singleton；
4. 最后锁 `investigation_rollout` singleton；promotion可在任一hold下dry-run，但apply不得隐式改变任一hold/policy；
5. 验证 active local principal、expected revalidation/campaign/operation-admission/tool-truth/investigation row versions、current joint rank与 `target = current + 1`；
6. 根据Task 1的edge enum在锁内重算readiness、all-fresh authority、compatibility cohort、canary、adversarial acceptance与consumer-retirement typed evidence member exact set；
7. canonical sort后计算evidence/canary/acceptance manifest hash，并保留每个source receipt/sample的typed id/hash；
8. 写immutable`operation_default_promotion_receipts`与全部`operation_default_promotion_evidence_members`，重读验证count/hash；
9. 对Tool Truth/Investigation两个default singleton做CAS；任一失败则receipt和两侧default全部rollback；三个hold不由promotion修改；
10. commit 后才影响新 operation，绝不 UPDATE旧 `operation_state`。

逐边 minimum 固定为：0→1 exact sealed Tool Truth shadow-writer readiness receipt及其operation members；1→2 exact sealed Registry/shadow-evaluator readiness receipt及其fixture members；2→3 closed positive shadow compatibility cohort且 mismatch/missing/incomplete/corrupt 为零；3→4 每个admitted scope的server-derived relevant-root `AllFreshToolTruthAuthorityBundle` exact；4→5 closed positive dual compatibility cohort + independently authorized authoritative canary 的action execution、同request all-fresh bundle、oracle、最终Wave coverage、revision adjudication、typed report dry-run + versioned adversarial acceptance corpus exact；5→6完整consumer inventory内legacy mutation/read fallback为零、compatibility projection health exact、historical adapter对expected artifact exact set的fresh byte probes全通过，且所有legacy writer/consumer已退休。禁止 generic string evidence、direct SQL setter、跳级、caller counts、force flag或独立 promote 任一 singleton。

当前闭集最高rank为6，没有rank 7，也不存在6→7“相邻”捷径。若未来需要移除compatibility tables/历史adapter或引入新contract，必须新增版本化migration、policy rank与独立设计；事故响应只能开启hold并forward-fix，不能把不存在的rank当回滚/升级通道。

safety hold由D local-admin coordinator路由到owner repo：`tool_truth_revalidation_dispatch`调用Plan A `tool_truth_revalidation.rs`按operation CAS head/generation并写Plan A event；`campaign_dispatch|operation_admission`调用Plan C singleton repo CAS各自generation并写event。跨scope同事务严格遵守“Plan A operation heads→Plan C singleton→rollout singleton”的锁序，单scope只锁自己的owner head，绝不先持有rollout锁再等待hold。紧急止血可分别开启三scope；初始revalidation与campaign held，operation-admission off。已发生action仅允许安全closeout/recovery并把未完成工作落residual/inconclusive。解除任一scope都需要独立用户授权与evidence manifest，旧authorization不能跨任何on→off generation复活，promotion不能隐式解除。

**Step 6：提供受审计 maintenance CLI，不暴露 Tauri command**

CLI surface 固定为：

```text
golish investigation-rollout status
golish investigation-rollout promote --target-joint-rank <N> --expected-tool-truth-version <V> --expected-investigation-version <V> --reason <TEXT> [--apply]
golish investigation-rollout safety-hold --scope <tool-truth-revalidation-dispatch|campaign-dispatch|operation-admission> [--operation-id <UUID>] --set <on|off> --expected-generation <G> --expected-version <V> --reason <TEXT> [--apply]
```

不带 `--apply` 永远是 read-only dry-run，输出 server/repo 锁内重算的 criteria、cohort/acceptance counts和manifest hash；不能由文件或参数注入 counts。`tool-truth-revalidation-dispatch`强制要求`--operation-id`并经Plan A repo验证operation policy/head；另两scope拒绝operation id。每次CAS同时验证owner scope generation与row version，写各自append-only event；任一scope的replay/payload drift和并发test独立覆盖。`--apply` 前执行者必须在当前会话再次取得用户明确授权；CLI 记录 principal、reason、evidence hash和结果 receipt。不得注册普通 Tauri command或前端按钮。

**Step 7：强制 operation create/resume/fork 语义**

`runtime_memory_tx.rs`创建新operation与创建fork target时先按target operation UUID取得同一revalidation advisory coordination key，再锁Plan C `verification_campaign_safety_holds → tool_truth_rollout → investigation_rollout`，在INSERT前重验`operation_admission_held=false`；插入operation后同transaction创建Plan A `tool_truth_revalidation_dispatch_policies/heads`，冻结`manual_only|auto_passive_t0_t1`且head初始held/generation，不能留下无policy operation。local-admin也先拿相同coordination key再锁Plan A head，因此不会出现create先持有Plan C而admin先持有Plan A的反序；竞争只可能“operation+policy/head完整提交”或“hold/coordination先提交后create重验”。随后验证七态合法pair，一次写入Tool Truth contract与Investigation contract/mode三个既有字段；joint rank始终由Plan B三元函数派生，不新增或写入operation rank列。promotion并发时只能观察完整旧pair或完整新pair。resume只读frozen pair/revalidation policy；fork无adoption receipt时完整继承source pair与policy（新head仍独立held），有receipt时只能前进一阶并验证exact set；不匹配整体rollback。

**Step 8：运行 GREEN、五态 replay、CLI dry-run 与 clippy**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_rollout_migrations --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_legacy_replay --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_rollout_)' --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish -E 'test(investigation_rollout_cli_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-core -p golish-db -p golish-agent-app --all-targets -- -D warnings)
```

Expected: single-V1 whole-record normalization、all divergence modes、逐边 joint promotion、default-only-new-operation、resume/fork receipt、maintenance CLI dry-run 与 five-mode/legacy-authority replay 全绿；无 warning。

### Future Commit

```bash
git add backend/crates/golish-db/src/repo/investigation_projection/comparison.rs backend/crates/golish-db/src/repo/operation_default_rollout.rs backend/crates/golish-db/src/repo/tool_truth_rollout.rs backend/crates/golish-db/src/repo/tool_truth_revalidation.rs backend/crates/golish-db/src/repo/investigation_rollout.rs backend/crates/golish-db/src/repo/operation_rollout.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/tests/investigation_rollout_migrations.rs backend/crates/golish-agent-app/tests/investigation_legacy_replay.rs backend/crates/golish/src/cli/investigation_rollout.rs backend/crates/golish/src/cli/args.rs backend/crates/golish/src/cli/mod.rs
git commit -m "feat(rollout): gate defaults with exact evidence"
```

## Task 13：完成模块卡、定向门禁与 evidence closeout

**Files:**

- Modify: `docs/modules/backend/golish-core.md`
- Modify: `docs/modules/backend/golish-db/repo.md`
- Modify: `docs/modules/backend/golish-agent-app/ai.md`
- Modify: `docs/modules/backend/golish-agent-app/conversation_store.md`
- Modify: `docs/modules/backend/golish-reporting-domain.md`
- Modify: `docs/modules/backend/golish-reporting-app.md`
- Modify: `docs/modules/frontend/store.md`
- Modify: `docs/modules/frontend/components.md`
- Modify: `docs/modules/frontend/lib.md`
- Modify: `docs/modules/INDEX.md`
- Modify: `agent-progress.md`
- Modify: `feature_list.json`

**Step 1：运行唯一的 scoped verification bundle**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_rollout_)' --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test investigation_projection_read_model --test investigation_rollout_migrations --test reporting_read_model_migrations --test capability_execution_receipts --test verification_campaigns --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_ipc_authorization --test investigation_read_model --test investigation_legacy_replay --test reporting_authority --test reporting_ipc_authorization --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-reporting-domain -p golish-reporting-app -E 'test(report_)' --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish -E 'test(investigation_rollout_cli_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-core -p golish-db -p golish-agent-app -p golish -p golish-reporting-domain -p golish-reporting-app --all-targets -- -D warnings)
just space-guard
(cd backend && cargo fmt -p golish-core -p golish-db -p golish-agent-app -p golish -p golish-reporting-domain -p golish-reporting-app -- --check)
pnpm exec vitest run frontend/store/investigation-workspace.test.ts frontend/lib/conversation-db-sync.test.ts frontend/lib/terminal-restore.test.ts frontend/services/ai-events/harness-handlers.test.ts frontend/components/PaneContainer/PaneLeaf.lazy.test.tsx frontend/components/AIChatPanel/StageProgressBar.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.reporting.test.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationWorkspace.test.tsx frontend/components/Engagement/InvestigationWorkspace/HypothesesTab.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignsTab.test.tsx frontend/components/Engagement/InvestigationWorkspace/CampaignDetail.test.tsx frontend/components/Engagement/InvestigationWorkspace/LegacyInvestigationAdapter.test.tsx frontend/components/Engagement/InvestigationWorkspace/InvestigationAuditDrawer.test.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/Engagement/ReportReadModelView.test.tsx
pnpm exec biome check frontend/store/types/session.ts frontend/store/slices/session.ts frontend/store/slices/session-core.ts frontend/store/selectors/pane-leaf.ts frontend/store/investigation-workspace.test.ts frontend/lib/api/investigation.ts frontend/lib/api/reporting.ts frontend/lib/conversation-db-sync.ts frontend/lib/terminal-restore.ts frontend/services/ai-events/harness-handlers.ts frontend/components/PaneContainer/PaneLeaf.tsx frontend/components/AIChatPanel/StageProgressBar.tsx frontend/components/AIChatPanel/StageRow.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/Engagement/InvestigationWorkspace frontend/components/Engagement/ReportReadModelView.tsx
pnpm typecheck
find backend/crates/golish-db/src/repo/investigation_projection -maxdepth 1 -name '*.rs' -print0 | xargs -0 wc -l
wc -l backend/crates/golish-db/src/repo/investigation_projection/comparison.rs backend/crates/golish-db/src/repo/operation_default_rollout.rs backend/crates/golish-agent-app/src/ai/operation_authority.rs backend/crates/golish/src/cli/investigation_rollout.rs
find frontend/components/Engagement/InvestigationWorkspace -maxdepth 1 \( -name '*.ts' -o -name '*.tsx' \) -not -name '*.test.ts' -not -name '*.test.tsx' -print0 | xargs -0 wc -l
```

Expected: 所有列出的 focused tests 通过；受影响 Rust crates 零 warning；Biome/typecheck 通过；新增非测试 Rust 文件各不超过 500 行，新增非测试 TS/TSX 文件各不超过 800 行。不得因此改跑全仓测试。

**Step 2：执行四类安全人工检查**

- 在 browser test fixture 中打开 Candidate/Verification、live/completed/restored route；确认没有 selected tool/review hint 也能 bootstrap。
- 搜索 Workspace 主 DOM fixture，确认 `Queue N`、lease/checkpoint/raw payload/chain-of-thought 不出现；Audit drawer 仅含 typed metadata。
- 搜索 report DTO/DOM/Markdown/JSON fixture，确认 token/cookie/PII/raw response/stdout/stderr/payload sentinel 全部不存在。
- 对五种 frozen mode 逐项核对 canonical writer、Gate、legacy mutation、Prepared Action JIT、compatibility projection 与 comparison behavior。

记录 exact fixture、截图或测试输出位置；不能用“已查看正常”替代 evidence。

**Step 3：更新模块卡与索引状态**

模块卡写清：

- Plan A/B own Tool Truth、all-fresh bundle/revalidation policy+hold、sample comparator、rollout/head/change/outbox/adoption receipt；Plan D扩展temporal-aware read/pagination、report-input dependency seal/shared invalidation seam、acceptance aggregate与唯一联合promotion；
- 六命令中前三个由 Plan B 创建、后三个由 Plan D 新增；
- fixed schema-version/read snapshot/change-sequence + temporal-cutoff/epoch V2 cursor contract；
- selector-only store 与 restore path；
- revision-adjudication security authority、Campaign objective-local audit、typed legacy/historical stable-snapshot authority、sink-safe redaction与all-fresh exact-member seal；
- five-mode mutation/compatibility matrix、adversarial detection acceptance、three independent holds与seven-rank joint default；
- focused test entry points。

同步 `docs/modules/INDEX.md` 状态列，不新增孤儿模块卡。

**Step 4：更新 progress 与 feature evidence**

把每条实际运行命令、exit code、关键输出、未运行的大型门禁、migration 授权记录、rollout 未实际 promotion、剩余风险写入 `agent-progress.md`。逐条核对 `feature_list.json.verification`；只有新鲜定向证据覆盖行为与风险时才能设 `passing`，否则保留 `in_progress`。

### Future Commit

```bash
git add docs/modules/backend/golish-core.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-agent-app/conversation_store.md docs/modules/backend/golish-reporting-domain.md docs/modules/backend/golish-reporting-app.md docs/modules/frontend/store.md docs/modules/frontend/components.md docs/modules/frontend/lib.md docs/modules/INDEX.md agent-progress.md feature_list.json
git commit -m "docs(investigation): record rollout evidence"
```

## 最小 rollback 与停止部署策略

### Migration 与 read model

- `00008` 只做 additive/forward-only 变更，不提供 destructive down migration。
- `terminal_state.investigation_workspace_json` nullable；旧 app 忽略该列，新 UI 遇到 null 回到未选择 operation 的 empty state。
- pagination/index/compare/report seal 发生问题时停止部署新 binary；不删除 Plan B head/change/outbox，不回写历史 Candidate/Attempt。
- projection 可由 Plan B outbox/reducer 重建；comparison sample/promotion receipt 是审计记录，不能清空来伪造通过。

### UI

- 在 `legacy_only / shadow_registry / dual_read_compare` 可把 Pane 入口暂时切回旧 UI，但 operation frozen authority 与旧 mutation guard 不变。
- UI route rollback 不允许把 registry-authoritative operation 重新开放旧 mutation；后端 guard 始终 fail closed。
- selector persistence rollback 只忽略 nullable JSON，不删除 conversation/terminal data。

### Reporting

- unsafe report revision 保持 draft/invalid，不 attach 或 publish artifact；已 final revision 不原地重写，修复后生成新 revision并 supersede。
- raw sentinel failure 立即停止 report build/export，不用字符串替换掩盖 typed projection 漏洞。
- report-input/source seal temporally stale时走H(g+1)新裁决并重建新draft；semantic orphan/quarantine走shared revoked invalidation；都不修改已发布artifact内容/hash，也不因same-semantic refresh复活旧revision。

### Rollout

- 联合 singleton default 永不向后移动；已冻结 operation 永不改 joint pair。尚未 promotion 时只停止 advance，不把“等待”伪装成 rollback。
- default 已推进后若发现问题，分别通过Plan A revalidation dispatch、Plan C Campaign dispatch、operation admission三个append-only hold暂停对应入口；三scope generation/CAS/event独立，已发生action只做安全closeout/recovery，未完成工作落residual/inconclusive。
- 修复后以新的 forward contract/criteria version重新走证据约束的相邻 transition，不能把任一 singleton私自倒退、批量 UPDATE旧 operation或删除旧 receipt/mismatch/coverage证据。
- 旧 binary 只有能识别数据库中所有 frozen joint rank/mode时才允许部署回滚；否则停止服务而不是 unknown-mode fallback。
- `shadow_registry / dual_read_compare` divergence 只阻止 promotion并保留旧 authority；禁止删除 mismatch row后重试。
- registry-authoritative compatibility divergence 保留 canonical truth并让旧 consumer fail closed；不能回退到逐字段 legacy fallback。
- fork adoption receipt 或 source seal 不匹配时不创建 target operation；修复 receipt/input 后以新 transaction 重试。

## 完成判定

只有同时满足以下条件，Plan D 才可宣称完成：

1. `00008` 未重复 Plan B 的 rollout/head/change/outbox/adoption/comparison/API authority，并有 migration test 证明。
2. 六个 operation-scoped API 在 exact ownership 与 repeatable-read snapshot 下工作，固定 `projection_schema_version=1`；V2 cursor同时以`change_seq + DB-clock temporal cutoff + authority epoch set`检测drift，TTL无write到期也不拼页，UI显示server `observed_as_of/temporally_stale`。
3. Candidate/Verification live/completed/restored 均直接进入同一 Workspace；四 tab 独立三态，旧 response 不覆盖新版本。
4. legacy 三 mode 保留必要 mutation，后两 mode 前后端均拒绝旧 mutation；`legacy_unavailable` 不被伪装为空。
5. queue/lease/hash 只在 Audit drawer，主 UI 不解释 scheduler order 为 coverage/priority。
6. 单Campaign只能objective-local audit；新SecurityVerdict exact绑定Plan B verification plan/proof paths/claim components与Plan C latest objective outcomes/revision adjudication/terminal decision。declared coverage与global sufficiency不越级，`ThreatCoverageProfileV1`缺失时永远`not_assessed`。
7. token/cookie/PII/raw response/stdout/stderr/payload sentinel 不出现在 DTO/DOM/artifact；attacker string normalization/length/bidi与HTML/Markdown/JSON/URL/CSV sink fixtures全绿。
8. report revision绑定active generation、revision adjudication、final Wave、Plan A relevant-root all-fresh bundle及open→members→seal exact typed dependencies；TTL只temporally stale，semantic orphan/C quarantine同source tx写report invalidation+whole-batch。历史artifact只从no-follow stable request snapshot读取。
9. shadow evaluation 无外部副作用，Campaign coverage denominator exact且terminal同事务封口；Plan B single-V1 whole-record compare无字段fallback。
10. 联合promotion按逐边criteria锁内重算；whole-record只证明compatibility，4→5另有九类versioned adversarial acceptance exact set；default只影响新operation，三个独立hold forward-only，普通Tauri无promotion入口。
11. same-operation resume保持frozen joint pair，fork adoption receipt exact，五态matrix/七态rank/replay全绿。
12. 定向验证的命令、exit code、关键证据已写入 progress/feature；未获授权的大型门禁如实记录未运行。
