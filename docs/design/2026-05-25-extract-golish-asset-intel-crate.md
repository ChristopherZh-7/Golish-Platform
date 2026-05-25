# Extract `golish-asset-intel` crate

> 日期：2026-05-25
> 状态：Draft
> Relates to: `docs/design/2026-05-22-asset-intel-json-driven-providers.md` ·
> `docs/design/2026-05-23-asset-intel-providers-flat.md`（保留其全部契约，本设计只动**代码所在的 crate**，不动行为）

## 1. 问题

`backend/crates/golish/src/tools/asset_intel.rs` 当前 **5,578 行 / 70 fn**，
是仓库 Rust 业务文件第一大户，且位于 L6 application crate（`golish`）下。

证据：

| 指标 | 值 |
|---|---|
| 单文件 LOC | 5,578（预算 500，超 11×） |
| 顶层 fn | 70 |
| `pub fn` | 9 |
| `pub struct/enum` | ~17（DTO + Capability + Status） |
| `#[tauri::command]` | 6（`asset_intel_list_providers` / `_lookup_company` / `_hydrate` / `_hydrate_subsidiaries` / `_enrich_organization` / `_enrich_batch`） |
| 外部使用方 | 仅 `commands_facade/asset_intel.rs` 1 处 |

它沉积在 L6 带来的具体伤害：

| # | 现象 | 后果 |
|---|---|---|
| H1 | L6 `golish` crate 总 52,743 行（占整 backend 53%），其中 `tools/` 占 55% | L6 应只是 Tauri 入口 + 命令桥，业务越多越难单独测，clean check 重启越慢 |
| H2 | `asset_intel.rs` 抓着 `crate::state::DbState` / `crate::tools::pentest::PentestState` / `crate::tools::organizations::upsert_…` 等 L6 内部符号 | 任何一次「单独跑 asset intel 业务测试」都得拖整个 L6 crate 编译 |
| H3 | 业务里同时混着「Tauri command」、「pure normalize 逻辑」、「subprocess spawn」、「JSON path 取值」、「Profile 字段映射」 | 单文件已成迷你 monolith，新增 provider/字段 PR diff 都打不开 |
| H4 | `OrganizationCandidate`、`AssetIntelProviderDescriptor` 等纯 DTO 也在 L6 | 想给 evaluator / harness / external benchmark 复用得拉整个 L6 |
| H5 | `golish-pentest-domain`（L1）已为 pentest types 准备好了 home，asset-intel types 没沿用 | 已有的分层约定被冷落 |

## 2. 目标

把 `tools/asset_intel.rs` 拆成 **L1 纯 types + L2 业务 crate + L6 薄 wrapper**
三段，**完全不改任何外部行为**：

- Tauri command 名 / 签名 / 事件 payload / JSON schema **保持 byte-equal**
- ts-rs 生成的 `frontend/lib/generated/` 内容**不变**
- 现有 35 个 `cargo nextest -p golish --lib -E 'test(asset_intel)'` + 62 个
  `golish-pentest` 单测**全部沿用、全部继续绿**

不做：

- 不改 normalize 规则、profile_fields 语义、provider fan-out 协议
- 不改 hydrate / enrich / lookup / discovery 业务流
- 不改 `resources/toolsconfig/*.json` schema
- 不动 `frontend/lib/api/asset-intel.ts` 任何一行

## 3. 当前对话面（事实清单）

### 3.1 `asset_intel.rs` 的依赖（向上 / 同层 / 向下）

```text
asset_intel.rs (L6)
├─ golish_core::{emit_opt, EventEmitterHandle}           (L1, OK 向下)
├─ golish_pentest::models::ToolConfig                    (L2, OK 向下)
├─ crate::error::GolishError                             (L6 自己)
├─ crate::event_emitter::TauriEventEmitter               (L6 自己)
├─ crate::state::DbState                                 (L6 自己, Tauri State)
├─ crate::tools::organizations::{                        (L6 自己, 重要)
│       upsert_organization_candidates_for_org,
│       OrganizationCandidate,
│       OrganizationCandidateKind,
│       OrganizationCandidates,
│  }
├─ crate::tools::pentest::PentestState                   (L6 自己, Tauri State)
└─ tauri::{State, AppHandle}                             (L6 only, command 表层)
```

### 3.2 `asset_intel.rs` 的公共表面

| 类别 | 数量 | 示例 |
|---|---|---|
| `pub enum` | 7 | `AssetIntelStreamSource` / `AssetIntelBatchSource` / `AssetIntelProviderRuntimeKind` / `AssetIntelStreamEvent` / `AssetIntelCapability` / `AssetIntelProviderStatus` / `AssetIntelRunStatus` / `AssetIntelProviderRunState` |
| `pub struct` (DTO) | 10 | `AssetIntelIntegrationRequirement` / `AssetIntelProviderDescriptor` / `AssetIntelProviderRecord` / `AssetIntelHydrateConfig` / `AssetIntelHydrateArgs` / `AssetIntelProviderRunStatus` / `AssetIntelRun` / `LookupCompanyMatch` / `AssetIntelLookupRequest` / `AssetIntelLookupResult` / `ProfileFieldEntry` / `AssetIntelEnrichOrganizationArgs` / `AssetIntelEnrichBatchArgs` / `AssetIntelEnrichBatchSkip` / `AssetIntelEnrichBatchResult` |
| `pub const` | 1 | `ASSET_INTEL_EVENT: &str = "asset-intel:event"` |
| `pub fn`（纯业务）| 3 | `normalize_provider_records` / `extract_lookup_matches` / `extract_profile_field_entries` |
| `#[tauri::command]` | 6 | `asset_intel_list_providers` / `_lookup_company` / `_hydrate` / `_hydrate_subsidiaries` / `_enrich_organization` / `_enrich_batch` |

### 3.3 `organizations::upsert_organization_candidates_for_org` 的签名

```rust
pub(crate) async fn upsert_organization_candidates_for_org(
    pool: &sqlx::PgPool,
    id: Uuid,
    candidates: Vec<OrganizationCandidate>,
) -> Result<OrganizationCandidates, GolishError>
```

属 L6 自身的 thin wrapper（内部转调 `golish_db::repo::organizations::*`）。

### 3.4 现有测试分布

- `cargo nextest -p golish --lib -E 'test(asset_intel)'` → 39 passed（最新）
- `cargo nextest -p golish-pentest --status-level fail` → 62 passed
- 业务测试主要测 normalize / fan-out / profile_fields / lookup，**全是纯 fn 测试**，不依赖 Tauri runtime / sqlx pool

## 4. 抽出分层设计

```text
L6 backend/crates/golish/                                    (原地 thin wrapper)
└─ src/tools/asset_intel_commands.rs (NEW · ~250 行)
   ├─ 6 个 #[tauri::command]
   ├─ DbAdapter / EventAdapter / PentestStateAdapter 三个 impl
   └─ pub use 转发，commands_facade/asset_intel.rs 一行不动

L2 backend/crates/golish-asset-intel/ (NEW · ~4500 行)
├─ Cargo.toml
├─ src/lib.rs                          (re-export + layer 注释)
├─ src/normalize/
│   ├─ mod.rs
│   ├─ json_path.rs                    (resolve_json_path 类辅助)
│   ├─ filters.rs                      (when filter 9 op)
│   ├─ records.rs                      (normalize_provider_records)
│   ├─ profile_fields.rs               (extract_profile_field_entries / ProfileFieldEntry / build_profile_patch_from_entries)
│   └─ lookup.rs                       (extract_lookup_matches / dedupe_lookup_matches)
├─ src/provider/
│   ├─ mod.rs
│   ├─ expand.rs                       (expand_provider_tools fan-out)
│   ├─ descriptor.rs                   (provider_descriptors_from_tools)
│   └─ select.rs                       (select_asset_intel / _subsidiary / _enrichment)
├─ src/runtime/
│   ├─ mod.rs
│   ├─ cli_json.rs                     (run_cli_json_provider)
│   ├─ http_json.rs                    (run_http_json_provider)
│   └─ stream.rs                       (CliJsonStreamShared / event helpers)
├─ src/hydrate/
│   ├─ mod.rs
│   ├─ run.rs                          (run_providers_for_org / hydrate / hydrate_subsidiaries)
│   └─ enrich.rs                       (enrich_organization / enrich_batch)
├─ src/lookup.rs                       (run_lookup_cli_provider + lookup_company)
├─ src/ports.rs                        (CandidateSink / EventSink / DbPool trait — Port-Adapter 边界)
└─ src/events.rs                       (ASSET_INTEL_EVENT 常量 + AssetIntelStreamEvent enum)

L1 backend/crates/golish-pentest-domain/   (已存在, 加 types 模块)
└─ src/asset_intel.rs (NEW · ~600 行)
   纯 DTO：OrganizationCandidate / OrganizationCandidateKind / OrganizationCandidates
            AssetIntelProviderDescriptor / AssetIntelProviderRecord
            AssetIntelHydrateConfig / AssetIntelHydrateArgs / AssetIntelRun / ...
            AssetIntelCapability / AssetIntelProviderStatus / AssetIntelRunStatus / ...
            LookupCompanyMatch / AssetIntelLookupRequest / AssetIntelLookupResult
            AssetIntelEnrich* / ProfileFieldEntry
```

**层规则核验**：

| Crate | 层 | 依赖 |
|---|---|---|
| `golish-pentest-domain` | L1 | （不变，仅加 types 模块） |
| `golish-asset-intel` | L2 | `golish-core` (L1) · `golish-pentest-domain` (L1) · `golish-pentest` (L2 同层 OK) |
| `golish` | L6 | + `golish-asset-intel` 作为新 dep |

无 back-edge。`arch-check.yml` 即应通过。

## 5. Trait / Port 设计（关键解耦点）

L2 `golish-asset-intel` **不能**看到 `tauri::*` / `sqlx::PgPool` / `crate::state::DbState`。
所有需要外部协作的能力走 trait 注入：

```rust
// crates/golish-asset-intel/src/ports.rs

#[async_trait::async_trait]
pub trait CandidateSink: Send + Sync {
    async fn upsert_for_org(
        &self,
        org_id: Uuid,
        candidates: Vec<OrganizationCandidate>,
    ) -> Result<OrganizationCandidates, AssetIntelError>;
}

#[async_trait::async_trait]
pub trait ProfilePatchSink: Send + Sync {
    async fn apply_profile_patch(
        &self,
        org_id: Uuid,
        patch: ProfilePatch,
    ) -> Result<(), AssetIntelError>;
}

pub trait EventSink: Send + Sync {
    fn emit(&self, payload: AssetIntelStreamEvent);
}

#[async_trait::async_trait]
pub trait ToolExecResolver: Send + Sync {
    async fn resolve_executable(
        &self,
        tool: &ToolConfig,
    ) -> Result<PathBuf, AssetIntelError>;
}
```

L6 在 `tools/asset_intel_commands.rs` 写 3 个 thin adapter：

```rust
pub struct DbCandidateSink<'a>(pub &'a sqlx::PgPool);

#[async_trait::async_trait]
impl<'a> CandidateSink for DbCandidateSink<'a> {
    async fn upsert_for_org(
        &self,
        org_id: Uuid,
        candidates: Vec<OrganizationCandidate>,
    ) -> Result<OrganizationCandidates, AssetIntelError> {
        crate::tools::organizations::upsert_organization_candidates_for_org(
            self.0, org_id, candidates,
        )
        .await
        .map_err(Into::into)
    }
}

pub struct TauriEventSink(pub TauriEventEmitter);
impl EventSink for TauriEventSink { /* emit via tauri::AppHandle */ }
```

**好处**：L2 业务可以用 `MockCandidateSink` / `MockEventSink` 单测，
零 Tauri runtime / 零 sqlx pool 依赖。

## 6. 迁移路径（TDD 小步，绿→改→再绿）

按 11 个 task 走，每个 task 都是一个 commit，每个 commit 在 `just check` 全绿之前**不合**：

| # | Task | 验收 |
|---|---|---|
| T1 | 新建 crate `golish-asset-intel` 框架（Cargo.toml + lib.rs + ports.rs + lib.rs `pub use` 占位）；workspace 注册；空 crate `cargo check -p golish-asset-intel` 绿 | `cargo metadata` 见新 crate / `cargo check` exit 0 |
| T2 | 把 `OrganizationCandidate*` 三个 type **移**到 `golish-pentest-domain::asset_intel`；L6 改 `pub use golish_pentest_domain::asset_intel::*;` 透传 | 35 个 asset_intel 单测全绿 / `tools::organizations` 编译 |
| T3 | 把 `AssetIntel*` 17 个 enum/struct DTO **移**到 `golish-pentest-domain::asset_intel`；L6 同样透传 | 同上 |
| T4 | 把 `normalize_provider_records` + 配套 helpers **移**到 `golish-asset-intel::normalize`；保留 L6 `pub use` 别名 | 单测全绿（这部分本来就是纯 fn） |
| T5 | 把 `extract_profile_field_entries` / `build_profile_patch_from_entries` / `ProfileFieldEntry` **移**到 L2 `normalize::profile_fields` | 同上 |
| T6 | 把 `expand_provider_tools` + `provider_descriptors_from_tools` + 三个 `select_*` **移**到 L2 `provider::*` | `select_*` 测全绿 |
| T7 | 把 `extract_lookup_matches` + `dedupe_lookup_matches` + `run_lookup_cli_provider` **移**到 L2 `lookup` | lookup 单测全绿 |
| T8 | 引入 `CandidateSink` / `ProfilePatchSink` / `EventSink` / `ToolExecResolver` trait；把 `run_cli_json_provider` / `run_http_json_provider` 改为 generic over trait | 单测改用 mock sink；nextest 全绿 |
| T9 | 把 `hydrate` / `hydrate_subsidiaries` / `enrich_organization` / `enrich_batch` 业务 fn **移**到 L2 `hydrate::*`；通过 trait 写库 / 发事件 | 单测继续走 mock sink，全绿 |
| T10 | L6 `tools/asset_intel.rs` 仅留 6 个 `#[tauri::command]` thin wrapper（重命名到 `tools/asset_intel_commands.rs`），构造 3 个 adapter，调用 L2 业务；`commands_facade/asset_intel.rs` 改 `pub use` 路径 | 6 个 IPC 名 / 签名 / 行为不变；ts-rs 输出 byte-equal |
| T11 | 删空 `tools/asset_intel.rs` 旧文件；`just precommit` 全绿 | `wc -l` 旧文件应 0 / 新文件 ≤ 200；`just precommit` exit 0 |

**测试保留策略**：

- 单元层（normalize / extract / fan-out）：测试代码**整段平移**到 L2 crate 的 `#[cfg(test)] mod tests`，不重写
- 集成层（hydrate / enrich，要 mock state）：在 L2 用 `MockCandidateSink` / `MockEventSink` 重写一遍（不超过 5 个 test）
- L6 IPC 层：保留 1 个 smoke test 确认 thin wrapper 编译过 + 透传正确

## 7. 向后兼容

| 维度 | 兼容性 |
|---|---|
| Tauri command 名 | 6 个全部不变（`asset_intel_*`） |
| Tauri command 参数 / 返回 | byte-equal（DTO 仍同名同字段，只是 import 路径变） |
| 事件 payload (`asset-intel:event`) | byte-equal |
| ts-rs 生成（`frontend/lib/generated/`） | **不应有 diff**；CI 增加一步 `cargo test --test ts_export -- --nocapture` 比对 |
| JSON schema (`resources/toolsconfig/*.json`) | 不动 |
| `frontend/lib/api/asset-intel.ts` | 不动 |
| 数据库 schema | 不动 |
| 现有 in_progress 任务 `asset-intel-hydrate-disambiguation` | **暂缓本 refactor 启动**，先等它进 passing；否则 git 冲突会非常痛 |

## 8. 影响面

| 影响域 | 是否动 | 备注 |
|---|---|---|
| 后端 crate 数 | ✅ | +1（48 → 49） |
| 后端 6 层 DAG | ✅ | L2 多 1 crate，无 back-edge |
| `arch-check.yml` | ⚠ | 需要把 `golish-asset-intel` 加进白名单（L2 节点） |
| `golish-pentest-domain` | ✅ | +1 模块 `asset_intel.rs` |
| `golish-pentest` | ❌ | `ToolConfig` 不动 |
| `golish` | ✅ | 减 ~4500 行 / 加 ~250 行 |
| `commands_facade/asset_intel.rs` | ✅ | 1 行 `pub use` 路径变更 |
| `commands_registry.rs` | ❌ | 6 个 command 名不变 |
| 前端 | ❌ | 一行不动 |
| 数据库 | ❌ | 不动 |
| 文档 | ✅ | `docs/architecture.md` 加 1 crate；`agent-progress.md` 记录；`feature_list.json` 加条目 |

## 9. 验证

### 9.1 单元层

```bash
cargo nextest run -p golish-asset-intel --status-level fail        # 新 crate 内 ~30 个 test 全绿
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail   # 35 既有继续绿
cargo nextest run -p golish-pentest --status-level fail            # 62 既有继续绿
cargo nextest run -p golish-pentest-domain --status-level fail     # 新加 ~5 个 round-trip test 全绿
```

### 9.2 静态检查

```bash
cargo fmt --check
cargo check -p golish-asset-intel -p golish-pentest-domain -p golish
just lint-rust              # clippy 零 warning
python3 scripts/check_dag.py        # DAG 守护
```

### 9.3 前端 / IPC

```bash
pnpm exec tsc --noEmit       # 应 exit 0（ts-rs 输出未变 → frontend/lib/generated 不变 → 类型不变）
pnpm exec biome check frontend/lib/api/asset-intel.ts   # 0 fix
pnpm vitest run frontend/components/TargetPanel/        # 全绿（前端零改动）
```

### 9.4 行为等价（关键）

新增一组 **byte-equal 断言**：

```rust
// crates/golish-asset-intel/tests/byte_equal.rs
#[test]
fn provider_descriptors_byte_equal() {
    // 同一 fixture 在 L6 旧路径 vs L2 新路径产生的 JSON 序列化必须完全一致
}
```

### 9.5 端到端

- `just dev` 启动后，Discover Assets → Look up → Hydrate Subsidiaries → Enrich
  各步骤 UI 行为与 refactor 前一致（用户做）
- `asset-intel:event` 事件 payload 与 refactor 前 byte-equal（用 Network DevTools 抓帧对比，用户做）

## 10. 后续清理（计划文件外）

- `docs/architecture.md` L2.assets 子集群里加 `golish-asset-intel`
- `agent-progress.md` 加一段记录
- `feature_list.json` 加 `extract-golish-asset-intel-crate` 条目（`not_started`）
- 同步在 `AGENTS.md` 的 quick-reference 表里没有具体 crate 名字，无需动
- 计划文件 `docs/superpowers/plans/2026-05-25-extract-golish-asset-intel-crate.md` 按 11 task 落地

## 11. 风险与缓解

| 风险 | 概率 | 缓解 |
|---|---|---|
| `OrganizationCandidate` 在 L1 暴露，但仍有 `golish-db` 写入需要 L2 | 低 | 用 `From<OrganizationCandidate> for golish_db::Candidate` 在 L6 适配 |
| `async_trait` 引入二进制膨胀 | 极低 | 已被 `golish-mcp` 等 crate 使用 |
| 与 in_progress `asset-intel-hydrate-disambiguation` 冲突 | **高** | 启动本任务前必须等 hydrate-disambiguation 切 passing 或显式 paused |
| ts-rs 生成路径变化导致 frontend/lib/generated 漂移 | 中 | T2/T3 完成后立刻跑 ts-rs export 比对，发现 diff 立即把字段位置 / serde rename 校齐 |
| L6 thin wrapper 漏掉错误转换 | 低 | T10 加 1 个集成 test 显式触发每个 command 的错误路径 |

## 12. 不在本设计的范围（避免 scope 蔓延）

- 不动 `tools/integrations/` / `tools/pentest_bridge/` / `tools/pentest/`（虽然它们也该下沉，留作后续 P1 task）
- 不动 `commands_facade/workspace.rs` 拆分（独立 P0-2 设计文档）
- 不动 `AppState → sub-state` 迁移（独立 P1-4 设计文档）
- 不引入 `golish-asset-intel` 的 v2 schema / 新 provider；只搬不动
- 不重写 normalize 规则 DSL；保留现状逐字平移
