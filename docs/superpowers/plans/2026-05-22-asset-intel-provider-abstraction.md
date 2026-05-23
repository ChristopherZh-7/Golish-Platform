# Asset Intel Provider Abstraction Phase 1 实现计划

> Superseded by `docs/superpowers/plans/2026-05-22-asset-intel-json-driven-providers.md`.

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划；实现代码前遵守 TDD，先写失败测试。

**目标：** 新增 `Asset Intel Service` 的 Phase 1 MVP，让 `Discover Assets` 能通过统一 IPC 列出 provider 并触发 hydrate，把规范化候选写入现有 `organizations.intel.engagement.candidates`。
**架构：** 在 `golish` 应用层新增 `asset_intel` 业务模块和 `commands_facade/asset_intel.rs`，复用现有 `OrganizationCandidate` 契约与候选 upsert 逻辑。第一版只提供 ENScan_GO provider skeleton 和测试用 mock provider，不调用真实 CLI。
**技术栈：** Rust Tauri command + serde IPC；React/TypeScript wrapper 通过 `frontend/lib/api/asset-intel.ts` 暴露。

## 文件结构

- `backend/crates/golish/src/tools/asset_intel.rs`：新增 Asset Intel 类型、provider registry、hydrate service、IPC 命令与单元测试。
- `backend/crates/golish/src/tools/organizations.rs`：把候选 upsert 抽成可复用 helper，供 `asset_intel` 调用。
- `backend/crates/golish/src/tools/mod.rs`：声明 `asset_intel` 模块。
- `backend/crates/golish/src/commands_facade/asset_intel.rs`：导出新 Tauri commands。
- `backend/crates/golish/src/commands_facade/mod.rs`：声明 facade 模块。
- `backend/crates/golish/src/commands_registry.rs`：注册 `asset_intel_list_providers` 与 `asset_intel_hydrate`。
- `frontend/lib/api/asset-intel.ts`：新增 typed wrapper。
- `frontend/lib/api/index.ts`：导出 `assetIntel` namespace。

## 任务 1：候选写入 helper

1. 在 `organizations.rs` 中新增测试：调用 helper 后能把 organization/target candidate 写入 `intel.engagement.candidates`，并继续复用去重逻辑。
2. 运行 `cargo test -p golish candidate_helper --lib`，确认红灯失败。
3. 提取 `upsert_organization_candidates(pool, id, candidates)` helper，原 `organization_candidates_upsert` command 改为调用 helper。
4. 运行同一测试确认绿灯。

## 任务 2：Asset Intel 纯服务契约

1. 在 `asset_intel.rs` 先写单元测试：
   - provider registry 暴露 `enscan-go` descriptor。
   - mock provider 输出能 normalize 成 organization 和 target candidates。
   - hydrate run 返回 `checked_empty` evidence 与 provider status。
2. 运行 `cargo test -p golish asset_intel --lib`，确认红灯失败。
3. 实现最小类型与纯函数：`AssetIntelProviderDescriptor`、`AssetIntelHydrateArgs`、`AssetIntelRun`、`ProviderStatus`、`AssetIntelProvider` trait、`default_registry()`。
4. 运行同一测试确认绿灯。

## 任务 3：Tauri IPC

1. 新增 `asset_intel_list_providers() -> Vec<AssetIntelProviderDescriptor>`。
2. 新增 `asset_intel_hydrate(state, args) -> AssetIntelRun`，读取 organization，选择 provider，调用 service，写入 candidates。
3. 按 `docs/development.md` 注册 facade 与 `commands_registry.rs`。
4. 运行 `cargo check -p golish`。

## 任务 4：前端 API Wrapper

1. 新增 `frontend/lib/api/asset-intel.ts`，只封装 IPC，不改 Target UI。
2. 在 `frontend/lib/api/index.ts` 导出 `assetIntel`。
3. 运行 `pnpm exec tsc --noEmit` 与 scoped Biome。

## 验证

- `cargo test -p golish asset_intel --lib`
- `cargo test -p golish candidate_helper --lib`
- `cargo check -p golish`
- `pnpm exec tsc --noEmit`
- `pnpm exec biome check frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts`

## Phase 2：ENScan_GO Adapter 接入

目标是在不改变 Target UI 契约的前提下，把 `enscan-go` 从 skeleton 升级为真实 provider adapter。第一版只执行只读企业情报命令，输出仍写入 `OrganizationCandidate`，不会直接创建 targets。

### 任务 5：命令构建与 JSON normalize

1. 在 `asset_intel.rs` 新增失败测试：
   - `build_enscan_command_plan()` 必须生成 `-n <company> -type aqc -field icp,app,wx_app,wechat -json -out-dir <dir>`，并按 discovery config 追加 `-invest` / `-deep` / `-branch`。
   - `parse_enscan_json_records()` 必须把 `invest` / `branch` 解析为 organization candidates，把 `icp` / `app` / `wx_app` / `wechat` 解析为 target candidates。
2. 运行 `cargo test -p golish asset_intel --lib`，确认红灯。
3. 实现最小命令构建与 JSON normalize 纯函数。
4. 再运行同一命令确认绿灯。

### 任务 6：真实 ENScan_GO 执行路径

1. `asset_intel_hydrate` 注入 `PentestState`，通过 `ConfigManager` 获取 `toolsconfig_dir` / `tools_dir`。
2. 使用 `golish_pentest::scan_toolsconfig` + `resolve_tool_executable("enscan-go", ...)` 定位可执行文件。
3. 用 `tokio::process::Command` 执行只读命令，超时 `180s`，stdout/stderr 只保留 preview 到 evidence。
4. JSON 解析顺序：先解析 stdout，再递归读取 `out_dir` 下 `.json` artifacts。
5. provider status 映射：
   - spawn / config missing → `unavailable`
   - timeout / non-zero exit → `failed`
   - 成功但 0 candidates → `checked_empty`
   - 成功且有 candidates → `completed`

### 任务 7：候选写入与验证

1. 复用 Phase 1 的 `upsert_organization_candidates_for_org` 写入候选。
2. 运行：
   - `cargo test -p golish asset_intel --lib`
   - `cargo test -p golish candidate_upsert --lib`
   - `cargo check -p golish`
   - `pnpm exec tsc --noEmit`
   - `pnpm exec biome check frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts`

## Phase 3：Frontend Discover Assets 闭环

目标是让 `TargetGroupedView` 的 Discover Assets workspace 消费统一 `assetIntel` API，而不是认识 ENScan_GO。UI 必须呈现 hydrate run 三态、provider status、candidate review 和显式 promote。

### 任务 8：前端 helper 与测试

1. 在 `TargetGroupedView.actions.test.ts` 覆盖：
   - `buildHydrateConfigFromEngagement()` 把 `min_ownership_percent` / `depth` / `include_branches` / `create_candidates` 映射为 hydrate config。
   - `getCandidateItems()` 返回 organization / target candidate buckets。
   - `getProviderStatusClass()` 区分 completed / checked_empty / failed / unavailable。
2. 运行 `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts`。

### 任务 9：Hydrate 与 provider status UI

1. `TargetGroupedView` 引入 `assetIntel.hydrate()`。
2. `Hydrate intel` action 调用统一 hydrate IPC，期间显示 loading。
3. Activity tab 显示 last run status、provider status、错误态和 checked-empty 文案。
4. hydrate 成功后刷新 organizations 并切到 Candidates tab。

### 任务 10：Candidate review / promote

1. Candidates tab 展示 organization / target candidate 列表，不只显示计数。
2. 支持 approve / reject：复用 `organization_candidates_upsert` 更新 candidate status。
3. 支持显式 promote：
   - organization candidate → 创建 child organization。
   - target candidate → 复用现有 `onBatchAdd` 创建 target。
4. Promote 仍由用户点击触发；hydrate 本身不扩大 active scan scope。

### 任务 11：Phase 3 验证

- `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts`
- `pnpm exec tsc --noEmit`
- `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts`
- `ReadLints` scoped check

## Phase 4：多 Provider / Auto Mode

目标是在 Target UI 不变的前提下，让 `Asset Intel Service` 能同时编排多个 provider。第一批新增 `0.zone`，auto mode 同时尝试 ENScan_GO 与 0.zone；未配置 0.zone key 时仅返回 `unavailable` provider status，不阻塞 ENScan_GO。

### 任务 12：Provider registry 扩展

1. `provider_descriptors()` 新增 `0.zone` descriptor。
2. descriptor 声明 `requiresIntegration: { toolId: "0.zone", groupIds: ["default"] }`。
3. capabilities 至少包含 `domains` / `apps` / `contacts`。
4. 新增 focused test 确认 descriptor 存在。

### 任务 13：0.zone adapter

1. 复用 `golish_intel_providers::zone::ZoneProvider`。
2. 从 `vault_entries` 读取 provider id `0.zone` 的 `api_key`，复用既有 obfuscation 解码。
3. 查询 `Domain` / `Site` / `Apk` 三类，只输出 candidates，不写 targets。
4. 结果映射：
   - `domain` / `url` / `ip` → target candidate。
   - `app_name` / `app_url` → target candidate。
   - `organization_name` → organization candidate。
5. 未配置 key → provider status `unavailable`。

### 任务 14：Auto mode 合并去重

1. `provider_ids` 为空时默认选择 `enscan-go` + `0.zone`。
2. 显式 `provider_ids` 只跑用户指定 provider。
3. 多 provider candidates 按 `kind + value(lowercase)` 去重，保留先返回的候选及其 evidence。
4. 新增 focused tests 覆盖 0.zone normalize 与跨 provider 去重。

### 任务 15：Phase 4 UI 与验证

1. Activity tab 显示 `asset_intel_list_providers` 返回的可用 provider chips。
2. 运行：
   - `cargo test -p golish asset_intel --lib`
   - `cargo check -p golish`
   - `pnpm vitest run frontend/components/TargetPanel/TargetGroupedView.actions.test.ts`
   - `pnpm exec tsc --noEmit`
   - `pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.actions.test.ts frontend/lib/api/asset-intel.ts frontend/lib/api/index.ts`
