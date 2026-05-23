# Plan · Asset Intel Two-Phase Hydrate

> 关联设计：`docs/design/2026-05-22-asset-intel-two-phase-hydrate.md`
> 隶属 feature：`asset-intel-hydrate-disambiguation` Phase D
> 模式：单会话 in-process（[DISPATCH:off]），不分发

## 步骤

1. **后端 · 抽公共函数**（`backend/crates/golish/src/tools/asset_intel.rs`）
   - 把 `asset_intel_hydrate` 内部 provider 循环抽成
     `run_providers_for_org(pool, app_handle?, organization_id, company_name, providers, config, run_id) -> AssetIntelRun`，
     这样三个新命令复用同一段「跑 N 个 provider · merge candidates · 写 profile · upsert candidates · 汇总状态」逻辑。
   - 加 `select_subsidiary_providers(tools) -> Vec<&ToolConfig>` 与
     `select_enrichment_providers(tools) -> Vec<&ToolConfig>`，
     分别按 `capabilities.contains("subsidiaries")` 是否为真过滤；priority 排序复用现有 `select_asset_intel_providers` 的语义。

2. **后端 · 加三个 Tauri 命令**（同文件）
   - `asset_intel_hydrate_subsidiaries(args: AssetIntelHydrateSubsidiariesArgs) -> AssetIntelRun`
   - `asset_intel_enrich_organization(args: AssetIntelEnrichOrganizationArgs) -> AssetIntelRun`
   - `asset_intel_enrich_batch(args: AssetIntelEnrichBatchArgs) -> AssetIntelEnrichBatchResult`
   - IDOR：每个命令开头都 `organizations::get_one` 取 row + project_path，404 报 `GolishError::NotFound`。
   - enrich_batch 用 `organizations::list(project_path)` + filter `parent_id == args.parent_organization_id` 得到 children。

3. **后端 · facade + registry**
   - `commands_facade/asset_intel.rs` 加 `pub use crate::tools::asset_intel::*;`（已是 glob，可能无需改，但确认）
   - `commands_registry.rs` 在 `tauri::generate_handler!` 加上三个新命令名。

4. **后端 · 测试**
   - 单测：select_subsidiary_providers / select_enrichment_providers 各 1
   - 单测：enrich_batch 处理 0 children / N children / partial fail 各 1（mock providers 或 reuse `RecordingEmitter`）
   - `cargo nextest run -p golish --lib -E 'test(asset_intel)'` 全绿
   - `cargo nextest run -p golish-pentest` 全绿（不期望改 pentest）
   - `cargo check -p golish` 0 与本任务相关 warning

5. **前端 · API wrapper**（`frontend/lib/api/asset-intel.ts`）
   - 加 `AssetIntelHydrateSubsidiariesArgs / EnrichOrganizationArgs / EnrichBatchArgs` 类型
   - 加 `AssetIntelEnrichBatchResult { runs: AssetIntelRun[]; skipped: { organizationId: string; reason: string }[] }`
   - 加 `hydrateSubsidiaries / enrichOrganization / enrichBatch` 函数

6. **前端 · TargetGroupedView UI 拆分**
   - `OrgActionItem.primary.kind` 加 3 个新 kind：`hydrate_subsidiaries` / `enrich_organization` / `enrich_batch`
   - 删旧 `hydrate_intel` 渲染分支（或保留为 fallback 兼容）
   - 主公司行（`node.parentId == null`）渲染【查子公司】+【批量补字段】
   - 子公司行渲染【补字段】（单家 enrich）
   - 新增三个 handler：`handleHydrateSubsidiaries / handleEnrichOrganization / handleEnrichBatch`
   - workspace tab 切换：subsidiaries 完成切到 candidates；enrich 完成切到 overview（看 profile fields）
   - streaming 事件继续监听 `ASSET_INTEL_EVENT`，按 organizationId 隔离 activity（既有逻辑）

7. **前端 · 测试**（`TargetGroupedView.actions.test.ts` + 新 dialog 测）
   - 渲染：主公司行应有 2 个按钮（查子公司 + 批量补字段）
   - 渲染：子公司行应只有【补字段】
   - 点击「批量补字段」调 enrichBatch 并传 parentOrganizationId + includeSelf=true
   - `pnpm vitest run frontend/components/TargetPanel/` 全绿
   - `pnpm exec tsc --noEmit` exit 0
   - `pnpm exec biome check` 改动文件 No fixes applied

8. **质量门**
   - ReadLints 全部改动文件 0 errors
   - `just precommit` 本次改动范围内全绿（不解决既有 preexisting failure_kind PlanStep cherry-pick 遗留）

9. **收尾**
   - 更新 `agent-progress.md`：在本日栏目下追加新段落「Phase D 两阶段编排」
   - 更新 `feature_list.json` 该 feature 的 evidence：加 `D_two_phase_orchestration` + 扩展 `user_visible_behavior`
   - 不 commit；汇总改动文件 + 让用户授权

## 不在本 PR 范围

- 不修 enscan-go.json provider runtime / 不动 0.zone.json normalize / 不动 schema
- 不删旧 `asset_intel_hydrate` 命令（仅打 deprecated 注释 + 一行 tracing::warn）
- 不做 dry-run 调用次数预估（next PR）
- 不写主流程进度持久化（next PR，活动事件目前是 in-memory）

## 风险

- 单文件 `asset_intel.rs` 已 3753 行，再加 3 个命令 + 公共函数会更胖；本 PR 优先功能正确性，文件拆分留下个 PR。
- enrich_batch 是顺序跑（不并发），子公司多时耗时长；先不优化，让用户体感真实再决定。
