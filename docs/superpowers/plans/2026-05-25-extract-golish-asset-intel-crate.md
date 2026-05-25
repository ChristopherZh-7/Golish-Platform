# Extract `golish-asset-intel` crate 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把 `backend/crates/golish/src/tools/asset_intel.rs`（5,578 行 / 70 fn / L6 application crate）抽出成 L1 DTO 模块 + L2 业务 crate + L6 薄 wrapper 三段，**byte-equal 行为零回归**。

**架构：** L1 `golish-pentest-domain::asset_intel`（纯 DTO，~600 行）+ L2 新建 `golish-asset-intel`（normalize / provider / runtime / hydrate / lookup，~4500 行）+ L6 `tools/asset_intel_commands.rs`（6 个 thin Tauri command + 4 个 adapter，~250 行）。L2 通过 `CandidateSink` / `ProfilePatchSink` / `EventSink` / `ToolExecResolver` 4 个 trait 端口由 L6 注入数据库与 Tauri 桥。

**技术栈：** Rust workspace · cargo nextest · async_trait · serde · ts-rs · 既有 sqlx PgPool / Tauri State。

**设计文档：** `docs/design/2026-05-25-extract-golish-asset-intel-crate.md`

**前置条件（启动前 MUST CHECK）：**

1. `feature_list.json` 中 `asset-intel-hydrate-disambiguation` 状态必须不是 `in_progress`（否则两者会在同一文件上并行改动，git 冲突极痛）。
2. `git status` 干净（除本计划外没有未提交的 `tools/asset_intel.rs` / `tools/organizations.rs` 改动）。
3. 当前 `cargo nextest run -p golish --lib -E 'test(asset_intel)'` exit 0 / 35+ passed —— 作为基线。
4. 当前 `cargo nextest run -p golish-pentest` exit 0 / 62 passed —— 作为基线。

---

## 文件结构（一次完成）

| 路径 | 角色 | 改动类型 |
|---|---|---|
| `backend/Cargo.toml` | workspace 加入新成员 `golish-asset-intel` | 扩展 |
| `backend/crates/golish-asset-intel/Cargo.toml` | 新 crate manifest | 新建 |
| `backend/crates/golish-asset-intel/src/lib.rs` | crate root + layer 注释 + re-export | 新建 |
| `backend/crates/golish-asset-intel/src/ports.rs` | 4 个 trait 端口 | 新建 |
| `backend/crates/golish-asset-intel/src/error.rs` | `AssetIntelError` enum | 新建 |
| `backend/crates/golish-asset-intel/src/events.rs` | `ASSET_INTEL_EVENT` 常量 + 事件 enum | 新建 |
| `backend/crates/golish-asset-intel/src/normalize/{mod,json_path,filters,records,profile_fields,lookup}.rs` | normalize 业务 | 新建（迁入） |
| `backend/crates/golish-asset-intel/src/provider/{mod,expand,descriptor,select}.rs` | provider fan-out | 新建（迁入） |
| `backend/crates/golish-asset-intel/src/runtime/{mod,cli_json,http_json,stream}.rs` | 运行时 | 新建（迁入） |
| `backend/crates/golish-asset-intel/src/hydrate/{mod,run,enrich}.rs` | hydrate 业务 | 新建（迁入） |
| `backend/crates/golish-asset-intel/src/lookup.rs` | lookup 业务 | 新建（迁入） |
| `backend/crates/golish-pentest-domain/src/lib.rs` | 加 `pub mod asset_intel;` | 扩展 |
| `backend/crates/golish-pentest-domain/src/asset_intel.rs` | 纯 DTO（迁入 17 个 type） | 新建 |
| `backend/crates/golish/Cargo.toml` | 新增 dep `golish-asset-intel = { path = "../golish-asset-intel" }` | 扩展 |
| `backend/crates/golish/src/tools/asset_intel.rs` | **最终删空**（迁移期保留 `pub use` 透传） | 重构 → 删除 |
| `backend/crates/golish/src/tools/asset_intel_commands.rs` | 6 个 thin Tauri command + 4 个 adapter | 新建 |
| `backend/crates/golish/src/tools/mod.rs` | `pub mod asset_intel_commands;`（删 `pub mod asset_intel;`） | 扩展 |
| `backend/crates/golish/src/commands_facade/asset_intel.rs` | 1 行 `pub use` 路径 | 扩展 |
| `backend/crates/golish/src/tools/organizations.rs` | `OrganizationCandidate*` 三 type 改 re-export from L1 | 扩展 |
| `scripts/check_dag.py` 或 `.github/workflows/arch-check.yml` | 把 `golish-asset-intel` 登记进 L2 节点 | 扩展 |
| `docs/architecture.md` | L2.assets 子集群里加一行 | 扩展 |
| `agent-progress.md` | 加一条本轮记录 | 扩展 |
| `feature_list.json` | 加 `extract-golish-asset-intel-crate` 一条 in_progress → 收尾 passing | 扩展 |

---

## 任务

### Task 1 · 新建空 crate `golish-asset-intel` 并加入 workspace

**文件：**
- 新建：`backend/crates/golish-asset-intel/Cargo.toml`
- 新建：`backend/crates/golish-asset-intel/src/lib.rs`
- 修改：`backend/Cargo.toml`（workspace members 列表）

**步骤：**

1. 创建 `backend/crates/golish-asset-intel/Cargo.toml`：

```toml
[package]
name = "golish-asset-intel"
version = "0.1.0"
edition = "2021"
license = "MIT"
publish = false

[dependencies]
golish-core = { path = "../golish-core" }
golish-pentest-domain = { path = "../golish-pentest-domain" }
golish-pentest = { path = "../golish-pentest" }

anyhow = { workspace = true }
async-trait = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["io-util", "process", "sync", "macros", "rt-multi-thread"] }
tracing = { workspace = true }
uuid = { workspace = true, features = ["v4", "serde"] }

[dev-dependencies]
pretty_assertions = { workspace = true }
```

> 备注：每个 dep 必须在 `backend/Cargo.toml` 的 `[workspace.dependencies]` 里都已声明，否则 `workspace = true` 会 fail。运行第 4 步前用 `rg '^async-trait' backend/Cargo.toml` 确认。如果某个 dep 未在 workspace 声明，要么改用具体版本号，要么先在 workspace 加。

2. 创建 `backend/crates/golish-asset-intel/src/lib.rs`：

```rust
//! Asset Intel domain service (L2 · `golish-platform/backend`).
//!
//! # Layer contract
//!
//! - **Depends on**:
//!   - `golish-core` (L1) — 事件 emitter trait + 共享类型
//!   - `golish-pentest-domain` (L1) — Asset Intel DTO
//!   - `golish-pentest` (L2 同层) — `ToolConfig`
//! - **Consumed by**:
//!   - `golish` (L6 application) — 通过 trait 端口注入 sqlx + Tauri 桥
//! - **Layer**: L2 simple infrastructure · assets sub-cluster
//!
//! 该 crate 不依赖 `sqlx` / `tauri` / `axum` 等具体 IO 实现，业务由 4 个端口
//! trait（`ports::CandidateSink` / `ProfilePatchSink` / `EventSink` /
//! `ToolExecResolver`）由 L6 注入。

pub mod error;
pub mod events;
pub mod ports;

pub use error::AssetIntelError;
pub use events::{AssetIntelStreamEvent, ASSET_INTEL_EVENT};
```

3. 修改 `backend/Cargo.toml`，在 `[workspace] members = [...]` 列表里按字母序追加：

```toml
    "crates/golish-asset-intel",
```

4. 验证：

```bash
cargo metadata --format-version=1 --no-deps 2>&1 | rg golish-asset-intel
cargo check -p golish-asset-intel
```

**预期输出：** 第一条命令打印含 `"name":"golish-asset-intel"` 的 JSON；第二条 `Finished` exit 0。

5. 创建占位 `ports.rs` / `error.rs` / `events.rs`：

`backend/crates/golish-asset-intel/src/error.rs`：

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssetIntelError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("provider {provider_id}: {message}")]
    Provider { provider_id: String, message: String },
    #[error("sink: {0}")]
    Sink(String),
    #[error("config: {0}")]
    Config(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

`backend/crates/golish-asset-intel/src/ports.rs`：

```rust
use crate::AssetIntelError;
use golish_pentest::models::ToolConfig;
use golish_pentest_domain::asset_intel::{OrganizationCandidate, OrganizationCandidates, ProfilePatch};
use std::path::PathBuf;
use uuid::Uuid;

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
    fn emit(&self, payload: crate::events::AssetIntelStreamEvent);
}

#[async_trait::async_trait]
pub trait ToolExecResolver: Send + Sync {
    async fn resolve_executable(&self, tool: &ToolConfig) -> Result<PathBuf, AssetIntelError>;
}
```

`backend/crates/golish-asset-intel/src/events.rs`（占位，Task 3 真正迁入完整 enum）：

```rust
pub const ASSET_INTEL_EVENT: &str = "asset-intel:event";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssetIntelStreamEvent {
    Placeholder,
}
```

6. 验证：

```bash
cargo check -p golish-asset-intel
```

**预期输出：** `Finished` exit 0。如果 `ports.rs` 报 `cannot find ProfilePatch`，**先注释掉 import 与 `ProfilePatchSink` 整段**，留 `OrganizationCandidate` 占位，Task 2/3 引入 DTO 后再恢复。

**Commit：**

```bash
git add backend/crates/golish-asset-intel/ backend/Cargo.toml
git commit -m "T1: new crate golish-asset-intel skeleton with port traits"
```

---

### Task 2 · 把 `OrganizationCandidate*` 三 type 迁到 `golish-pentest-domain`

**文件：**
- 新建：`backend/crates/golish-pentest-domain/src/asset_intel.rs`
- 修改：`backend/crates/golish-pentest-domain/src/lib.rs`
- 修改：`backend/crates/golish/src/tools/organizations.rs`
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`（仅 import 路径）

**步骤：**

1. 创建 `backend/crates/golish-pentest-domain/src/asset_intel.rs`（**整段从** `backend/crates/golish/src/tools/organizations.rs:63-97` **拷贝**，**保留所有原 derive 与字段**）：

```rust
//! Asset Intel DTOs shared between L2 `golish-asset-intel` 与 L6 `golish` IPC layer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationCandidateKind {
    Organization,
    Target,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationCandidate {
    #[serde(default)]
    pub id: String,
    pub kind: OrganizationCandidateKind,
    pub label: String,
    pub value: String,
    // …此处把 organizations.rs:74-92 范围内 OrganizationCandidate 的全部字段
    // 与默认值原样拷过来。不允许重命名/删字段/改 default。
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrganizationCandidates {
    pub organizations: Vec<OrganizationCandidate>,
    pub targets: Vec<OrganizationCandidate>,
}

/// 与 `golish_db::repo::organizations::ProfilePatch` 字段同名但放在 L1：
/// L6 adapter 负责 `From` 转换。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfilePatch {
    // …把 backend/crates/golish-db/src/repo/organizations.rs 内 ProfilePatch 的
    // 全部 pub 字段拷一份；L2 与 L6 共享同一个 schema。
}
```

> 拷贝指引：在执行此 task 前先 `Read backend/crates/golish/src/tools/organizations.rs` 看清楚字段，然后再 `Read backend/crates/golish-db/src/repo/organizations.rs` 找 `ProfilePatch` 的定义。**不允许**靠想象写字段。

2. 修改 `backend/crates/golish-pentest-domain/src/lib.rs`，在末尾追加：

```rust
pub mod asset_intel;
```

3. 修改 `backend/crates/golish/src/tools/organizations.rs`：把第 67-97 行 `OrganizationCandidateKind` / `OrganizationCandidate` / `OrganizationCandidates` 三个 def **整段删除**，在文件顶部 import 区追加：

```rust
pub use golish_pentest_domain::asset_intel::{
    OrganizationCandidate, OrganizationCandidateKind, OrganizationCandidates,
};
```

> `pub use` 确保现有外部使用方（`asset_intel.rs` 以及 4 个测试模块）零修改。

4. 修改 `backend/crates/golish-asset-intel/src/ports.rs`，把上一步的注释取消，确保用 `golish_pentest_domain::asset_intel::OrganizationCandidate` 等类型。

5. 验证：

```bash
cargo check -p golish-pentest-domain -p golish-asset-intel -p golish
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
cargo nextest run -p golish --lib -E 'test(organizations)' --status-level fail
```

**预期输出：** 三条全 exit 0；asset_intel 测试数应与 baseline 一致（>= 35 passed）；organizations 测试不回归。

**Commit：**

```bash
git add backend/crates/golish-pentest-domain/ backend/crates/golish/src/tools/organizations.rs backend/crates/golish-asset-intel/src/ports.rs
git commit -m "T2: move OrganizationCandidate{,Kind,s} to golish-pentest-domain"
```

---

### Task 3 · 把 17 个 `AssetIntel*` enum/struct DTO 迁到 `golish-pentest-domain::asset_intel`

**文件：**
- 修改：`backend/crates/golish-pentest-domain/src/asset_intel.rs`（追加 17 个 type）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`（删除迁出的 def，加 `pub use` 透传）
- 修改：`backend/crates/golish-asset-intel/src/events.rs`（把 `AssetIntelStreamEvent` 真正定义放在这里，但 `pub use` 自 L1）

**步骤：**

1. 在 `backend/crates/golish/src/tools/asset_intel.rs` 中**逐一定位**以下 17 个 type 的定义位置（用 `Grep 'pub (enum|struct) AssetIntel' backend/crates/golish/src/tools/asset_intel.rs`）：

| Type | 行号（参考，迁移前请 Grep 实际位置） |
|---|---|
| `AssetIntelStreamSource` | 43 |
| `AssetIntelBatchSource` | 52 |
| `AssetIntelProviderRuntimeKind` | 61 |
| `AssetIntelStreamEvent` | 77 |
| `AssetIntelCapability` | 114 |
| `AssetIntelProviderStatus` | 126 |
| `AssetIntelIntegrationRequirement` | 134 |
| `AssetIntelProviderDescriptor` | 141 |
| `AssetIntelProviderRecord` | 151 |
| `AssetIntelHydrateConfig` | 162 |
| `AssetIntelHydrateArgs` | 175 |
| `AssetIntelRunStatus` | 204 |
| `AssetIntelProviderRunState` | 212 |
| `AssetIntelProviderRunStatus` | 221 |
| `AssetIntelRun` | 229 |
| `LookupCompanyMatch` | 246 |
| `AssetIntelLookupRequest` | 266 |
| `AssetIntelLookupResult` | 281 |
| `ProfileFieldEntry` | 805 |
| `AssetIntelEnrichOrganizationArgs` | 3025 |
| `AssetIntelEnrichBatchArgs` | 3096 |
| `AssetIntelEnrichBatchSkip` | 3112 |
| `AssetIntelEnrichBatchResult` | 3125 |

2. 把这 23 个 type 的**完整 def**（包括 derive / serde attr / 字段注释）**整段移动**到 `backend/crates/golish-pentest-domain/src/asset_intel.rs`。

3. 在 `backend/crates/golish/src/tools/asset_intel.rs` 文件顶部（在原 `use` 区下方）加：

```rust
pub use golish_pentest_domain::asset_intel::{
    AssetIntelBatchSource, AssetIntelCapability, AssetIntelEnrichBatchArgs,
    AssetIntelEnrichBatchResult, AssetIntelEnrichBatchSkip,
    AssetIntelEnrichOrganizationArgs, AssetIntelHydrateArgs, AssetIntelHydrateConfig,
    AssetIntelIntegrationRequirement, AssetIntelLookupRequest, AssetIntelLookupResult,
    AssetIntelProviderDescriptor, AssetIntelProviderRecord, AssetIntelProviderRunState,
    AssetIntelProviderRunStatus, AssetIntelProviderRuntimeKind, AssetIntelProviderStatus,
    AssetIntelRun, AssetIntelRunStatus, AssetIntelStreamEvent, AssetIntelStreamSource,
    LookupCompanyMatch, ProfileFieldEntry,
};
```

4. 修改 `backend/crates/golish-asset-intel/src/events.rs`，把整个文件改成 re-export L1 真定义：

```rust
pub const ASSET_INTEL_EVENT: &str = "asset-intel:event";

pub use golish_pentest_domain::asset_intel::AssetIntelStreamEvent;
```

5. **关键**：原 `asset_intel.rs` 里这些 type 上的 ts-rs `#[ts(...)]` derive 要一并迁到 L1。如果当前文件没有 `ts-rs` 直接导出（grep 确认）就跳过这一步。

6. 验证：

```bash
cargo check -p golish-pentest-domain -p golish-asset-intel -p golish
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
pnpm exec tsc --noEmit
```

**预期输出：** 全 exit 0，asset_intel 测试数 ≥ 35 passed。如果 `tsc` 报缺类型，**立即停下**：说明 ts-rs 导出位置变了，需要在 L1 加上 ts-rs derive 重新生成 `frontend/lib/generated/`。

**Commit：**

```bash
git add backend/crates/golish-pentest-domain/src/asset_intel.rs \
        backend/crates/golish/src/tools/asset_intel.rs \
        backend/crates/golish-asset-intel/src/events.rs
git commit -m "T3: move 17 AssetIntel* DTOs to golish-pentest-domain"
```

---

### Task 4 · 迁 `normalize_provider_records` 与配套 JSON path / filter 辅助到 L2

**文件：**
- 新建：`backend/crates/golish-asset-intel/src/normalize/mod.rs`
- 新建：`backend/crates/golish-asset-intel/src/normalize/json_path.rs`
- 新建：`backend/crates/golish-asset-intel/src/normalize/filters.rs`
- 新建：`backend/crates/golish-asset-intel/src/normalize/records.rs`
- 修改：`backend/crates/golish-asset-intel/src/lib.rs`（`pub mod normalize;`）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`（删除迁出 fn，加 `pub use` 透传）

**步骤：**

1. 在 `backend/crates/golish/src/tools/asset_intel.rs` 中用 Grep 定位：

```
fn resolve_json_path
fn filter_passes
fn apply_filter_op
pub fn normalize_provider_records
fn extract_intel_array
fn extract_scalar
```

2. 创建 `backend/crates/golish-asset-intel/src/normalize/mod.rs`：

```rust
pub mod filters;
pub mod json_path;
pub mod records;

pub use records::normalize_provider_records;
```

3. `json_path.rs` 接收 `resolve_json_path` + 配套 helper（**整段移动**，包括 `#[cfg(test)] mod tests`）；签名必须保持公开为：

```rust
pub(crate) fn resolve_json_path(value: &serde_json::Value, path: &str) -> Option<serde_json::Value>;
```

> 如果原 fn 是私有（`fn` 没有 `pub`），新位置改 `pub(crate)`；导出范围最小化避免泄露。

4. `filters.rs` 接收 `filter_passes` / `apply_filter_op`，保持 `pub(crate)` 可见性。

5. `records.rs` 接收 `normalize_provider_records` + 一切只在 normalize 路径使用的辅助。**关键签名**应保持：

```rust
use golish_pentest::models::AssetIntelToolConfig;
use golish_pentest_domain::asset_intel::AssetIntelProviderRecord;

pub fn normalize_provider_records(
    raw: &serde_json::Value,
    config: &AssetIntelToolConfig,
) -> Vec<AssetIntelProviderRecord>;
```

> 如果原签名带 `tool_id: &str` 等额外参数，**严格保留**，不要趁机重构。

6. 把 `asset_intel.rs` 顶部加：

```rust
pub use golish_asset_intel::normalize::normalize_provider_records;
```

7. 验证：

```bash
cargo check -p golish-asset-intel -p golish
cargo nextest run -p golish-asset-intel --status-level fail
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
```

**预期输出：** L2 crate 新增 ≥ 8 个 test（normalize / json_path / filter 系列）全绿；L6 既有 asset_intel 测试 ≥ 35 passed 不回归。

**Commit：**

```bash
git add backend/crates/golish-asset-intel/src/normalize/ \
        backend/crates/golish-asset-intel/src/lib.rs \
        backend/crates/golish/src/tools/asset_intel.rs
git commit -m "T4: move normalize_provider_records + json_path + filters to L2"
```

---

### Task 5 · 迁 `extract_profile_field_entries` / `build_profile_patch_from_entries` 到 L2

**文件：**
- 新建：`backend/crates/golish-asset-intel/src/normalize/profile_fields.rs`
- 修改：`backend/crates/golish-asset-intel/src/normalize/mod.rs`（`pub mod profile_fields;`）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`（删迁出，加 `pub use`）

**步骤：**

1. Grep `asset_intel.rs`：

```
pub fn extract_profile_field_entries
fn build_profile_patch_from_entries
fn is_intel_array_profile_field
fn contact_dedupe_key
```

2. 创建 `profile_fields.rs`，**整段移动**这些 fn 及其单测。签名：

```rust
use golish_pentest::models::AssetIntelToolConfig;
use golish_pentest_domain::asset_intel::{ProfileFieldEntry, ProfilePatch};

pub fn extract_profile_field_entries(
    raw: &serde_json::Value,
    config: &AssetIntelToolConfig,
) -> Vec<ProfileFieldEntry>;

pub fn build_profile_patch_from_entries(
    existing: &serde_json::Value,
    entries: &[ProfileFieldEntry],
) -> Option<ProfilePatch>;
```

3. 在 `normalize/mod.rs` 加：

```rust
pub mod profile_fields;
pub use profile_fields::{build_profile_patch_from_entries, extract_profile_field_entries};
```

4. 在 `asset_intel.rs` 顶部加：

```rust
pub use golish_asset_intel::normalize::{build_profile_patch_from_entries, extract_profile_field_entries};
```

5. 验证：

```bash
cargo check -p golish-asset-intel -p golish
cargo nextest run -p golish-asset-intel --status-level fail
cargo nextest run -p golish --lib -E 'test(profile_fields)' --status-level fail
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
```

**预期输出：** profile_fields 单测（3 个）全绿；asset_intel 单测 ≥ 35 passed 不回归。

**Commit：**

```bash
git add backend/crates/golish-asset-intel/src/normalize/profile_fields.rs \
        backend/crates/golish-asset-intel/src/normalize/mod.rs \
        backend/crates/golish/src/tools/asset_intel.rs
git commit -m "T5: move extract_profile_field_entries + build_profile_patch_from_entries to L2"
```

---

### Task 6 · 迁 `expand_provider_tools` + `provider_descriptors_from_tools` + 3 个 `select_*` 到 L2

**文件：**
- 新建：`backend/crates/golish-asset-intel/src/provider/mod.rs`
- 新建：`backend/crates/golish-asset-intel/src/provider/expand.rs`
- 新建：`backend/crates/golish-asset-intel/src/provider/descriptor.rs`
- 新建：`backend/crates/golish-asset-intel/src/provider/select.rs`
- 修改：`backend/crates/golish-asset-intel/src/lib.rs`（`pub mod provider;`）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`

**步骤：**

1. Grep 定位：

```
fn expand_provider_tools
pub fn provider_descriptors_from_tools
fn select_asset_intel_providers
fn select_subsidiary_providers
fn select_enrichment_providers
```

2. 创建 `expand.rs`（**整段移动**，含 3 个测试）；签名：

```rust
use golish_pentest::models::ToolConfig;

pub fn expand_provider_tools(tools: &[ToolConfig]) -> Vec<ToolConfig>;
```

3. `descriptor.rs`：

```rust
use golish_pentest::models::ToolConfig;
use golish_pentest_domain::asset_intel::AssetIntelProviderDescriptor;

pub fn provider_descriptors_from_tools(tools: &[ToolConfig]) -> Vec<AssetIntelProviderDescriptor>;
```

4. `select.rs`（3 个 select fn 整段移动，**保留** owned `Vec<ToolConfig>` 返回签名 —— 这是 2026-05-23 的 design 决定）：

```rust
use golish_pentest::models::ToolConfig;

pub fn select_asset_intel_providers(
    tools: &[ToolConfig],
    requested: Option<&[String]>,
) -> Vec<ToolConfig>;

pub fn select_subsidiary_providers(tools: &[ToolConfig]) -> Vec<ToolConfig>;

pub fn select_enrichment_providers(tools: &[ToolConfig]) -> Vec<ToolConfig>;
```

> 如果实际签名与上面不同（例如还有 `enabled_only: bool` 参数），**保留实际签名**，不要修改。

5. `provider/mod.rs`：

```rust
pub mod descriptor;
pub mod expand;
pub mod select;

pub use descriptor::provider_descriptors_from_tools;
pub use expand::expand_provider_tools;
pub use select::{select_asset_intel_providers, select_enrichment_providers, select_subsidiary_providers};
```

6. `asset_intel.rs` 顶部加：

```rust
pub use golish_asset_intel::provider::{
    expand_provider_tools, provider_descriptors_from_tools, select_asset_intel_providers,
    select_enrichment_providers, select_subsidiary_providers,
};
```

7. 验证：

```bash
cargo check -p golish-asset-intel -p golish
cargo nextest run -p golish-asset-intel --status-level fail
cargo nextest run -p golish --lib -E 'test(expand_provider_tools)+test(provider_descriptors)+test(select_)' --status-level fail
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
```

**预期输出：** 全 exit 0；既有 select_* / provider_descriptors_from_tools 测试 ≥ 7 passed 不回归。

**Commit：**

```bash
git add backend/crates/golish-asset-intel/src/provider/ \
        backend/crates/golish-asset-intel/src/lib.rs \
        backend/crates/golish/src/tools/asset_intel.rs
git commit -m "T6: move expand_provider_tools + provider_descriptors_from_tools + select_* to L2"
```

---

### Task 7 · 迁 lookup（`extract_lookup_matches` / `dedupe_lookup_matches` / `run_lookup_cli_provider`）到 L2

**文件：**
- 新建：`backend/crates/golish-asset-intel/src/lookup.rs`
- 修改：`backend/crates/golish-asset-intel/src/lib.rs`（`pub mod lookup;`）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`

**步骤：**

1. Grep 定位：

```
pub fn extract_lookup_matches
fn dedupe_lookup_matches
async fn run_lookup_cli_provider
async fn asset_intel_lookup_company   ← 这个 Tauri command 留在 L6
```

2. 把 `extract_lookup_matches` / `dedupe_lookup_matches` / `run_lookup_cli_provider` **整段移动**到 `lookup.rs`。`run_lookup_cli_provider` 是 async 且需要 spawn process，所以签名需要改造接收 `&dyn ToolExecResolver`：

```rust
use crate::ports::ToolExecResolver;
use golish_pentest::models::AssetIntelToolConfig;
use golish_pentest_domain::asset_intel::{
    AssetIntelLookupRequest, AssetIntelLookupResult, LookupCompanyMatch,
};

pub async fn run_lookup_cli_provider(
    resolver: &dyn ToolExecResolver,
    config: &AssetIntelToolConfig,
    request: &AssetIntelLookupRequest,
) -> Result<AssetIntelLookupResult, crate::AssetIntelError>;

pub fn extract_lookup_matches(
    raw: &serde_json::Value,
    config: &AssetIntelToolConfig,
) -> Vec<LookupCompanyMatch>;

pub fn dedupe_lookup_matches(matches: Vec<LookupCompanyMatch>) -> Vec<LookupCompanyMatch>;
```

> 原 fn 里如果用了 `resolve_tool_executable`（synchronous helper from `golish-pentest`），把这一行 `let exec = resolve_tool_executable(...)?;` 改成 `let exec = resolver.resolve_executable(tool).await?;`，其余照搬。

3. `lib.rs`：

```rust
pub mod lookup;

pub use lookup::{dedupe_lookup_matches, extract_lookup_matches, run_lookup_cli_provider};
```

4. `asset_intel.rs` 内 `pub use` 透传 + `asset_intel_lookup_company` Tauri command 保留**当前**位置（Task 10 才迁），但**改用** L2 的 `run_lookup_cli_provider`。临时实现：

```rust
let resolver = crate::tools::asset_intel_commands::ToolExecResolverAdapter::new(/* … */);
let result = golish_asset_intel::run_lookup_cli_provider(&resolver, &config, &request).await?;
```

> `ToolExecResolverAdapter` 在 Task 10 才真正定义；这里写个临时 inline closure 实现 trait 也可（用 `struct Adhoc<F>(F);` + 手写 impl）。**优先**让 cargo check 过，不强求“一步到位”。

5. 验证：

```bash
cargo check -p golish-asset-intel -p golish
cargo nextest run -p golish-asset-intel --status-level fail
cargo nextest run -p golish --lib -E 'test(lookup)' --status-level fail
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
```

**预期输出：** L2 lookup 单测（≥ 2 个）全绿；L6 既有 lookup 测试 ≥ 2 passed 不回归。

**Commit：**

```bash
git add backend/crates/golish-asset-intel/src/lookup.rs \
        backend/crates/golish-asset-intel/src/lib.rs \
        backend/crates/golish/src/tools/asset_intel.rs
git commit -m "T7: move extract_lookup_matches + dedupe + run_lookup_cli_provider to L2"
```

---

### Task 8 · 迁运行时（`run_cli_json_provider` / `run_http_json_provider` / `CliJsonStreamShared`）到 L2，generic 化

**文件：**
- 新建：`backend/crates/golish-asset-intel/src/runtime/mod.rs`
- 新建：`backend/crates/golish-asset-intel/src/runtime/cli_json.rs`
- 新建：`backend/crates/golish-asset-intel/src/runtime/http_json.rs`
- 新建：`backend/crates/golish-asset-intel/src/runtime/stream.rs`
- 修改：`backend/crates/golish-asset-intel/src/lib.rs`（`pub mod runtime;`）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`

**步骤：**

1. Grep 定位：

```
struct CliJsonStreamShared
async fn run_cli_json_provider
async fn run_http_json_provider
```

2. 把 `CliJsonStreamShared` **整段移动**到 `stream.rs`，去掉 `EventEmitterHandle` 直接依赖；改成接收 `&dyn EventSink`：

```rust
use crate::events::AssetIntelStreamEvent;
use crate::ports::EventSink;
use golish_pentest_domain::asset_intel::ProfileFieldEntry;

pub(crate) struct CliJsonStreamShared<'a> {
    pub events: &'a dyn EventSink,
    pub run_id: uuid::Uuid,
    pub profile_entries: Vec<ProfileFieldEntry>,
    // …其他字段从原 struct 拷贝
}

impl<'a> CliJsonStreamShared<'a> {
    pub(crate) fn emit(&self, payload: AssetIntelStreamEvent) {
        self.events.emit(payload);
    }
    // …其他 method 整段拷贝；emit_opt 等 helper 替换为 self.events.emit(...)
}
```

3. `cli_json.rs` / `http_json.rs` 接收 `run_cli_json_provider` / `run_http_json_provider`，签名改造：

```rust
use crate::ports::{EventSink, ToolExecResolver};
use golish_pentest::models::AssetIntelToolConfig;
use golish_pentest_domain::asset_intel::AssetIntelProviderRecord;

pub(crate) async fn run_cli_json_provider(
    resolver: &dyn ToolExecResolver,
    events: &dyn EventSink,
    config: &AssetIntelToolConfig,
    run_id: uuid::Uuid,
    /* …其他参数照搬原签名 */
) -> Result<(Vec<AssetIntelProviderRecord>, Vec<ProfileFieldEntry>), crate::AssetIntelError>;
```

> **关键**：原 fn 内部所有 `emit_opt(emitter, &payload)` 改为 `events.emit(payload)`；所有 `resolve_tool_executable(tool, ...)` 改为 `resolver.resolve_executable(tool).await?`。

4. `runtime/mod.rs`：

```rust
pub mod cli_json;
pub mod http_json;
pub mod stream;

pub(crate) use cli_json::run_cli_json_provider;
pub(crate) use http_json::run_http_json_provider;
```

5. `asset_intel.rs` 不再 export 这两个 fn（它们已不直接被 IPC 使用）；保留 `pub(crate) use` 让 hydrate / enrich 还能调到。

6. 验证：

```bash
cargo check -p golish-asset-intel -p golish
cargo nextest run -p golish-asset-intel --status-level fail
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
```

**预期输出：** 全 exit 0，既有 asset_intel 测试 ≥ 35 passed 不回归。

**Commit：**

```bash
git add backend/crates/golish-asset-intel/src/runtime/ \
        backend/crates/golish-asset-intel/src/lib.rs \
        backend/crates/golish/src/tools/asset_intel.rs
git commit -m "T8: move run_cli/http_json_provider to L2 with EventSink + ToolExecResolver injection"
```

---

### Task 9 · 迁 hydrate / enrich 业务（`run_providers_for_org` / `hydrate*` / `enrich*`）到 L2

**文件：**
- 新建：`backend/crates/golish-asset-intel/src/hydrate/mod.rs`
- 新建：`backend/crates/golish-asset-intel/src/hydrate/run.rs`
- 新建：`backend/crates/golish-asset-intel/src/hydrate/enrich.rs`
- 修改：`backend/crates/golish-asset-intel/src/lib.rs`（`pub mod hydrate;`）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`

**步骤：**

1. Grep 定位：

```
async fn run_providers_for_org
fn dedupe_candidates_for_org
async fn hydrate
async fn hydrate_subsidiaries
async fn enrich_organization
async fn enrich_batch_inner
```

2. `run.rs` 接 `run_providers_for_org` + `hydrate` + `hydrate_subsidiaries`；签名改造（**所有 6 个 fn 都接收 ports trait 而非 `&PgPool` / `EventEmitterHandle`**）：

```rust
use crate::ports::{CandidateSink, EventSink, ProfilePatchSink, ToolExecResolver};
use golish_pentest::models::ToolConfig;
use golish_pentest_domain::asset_intel::{
    AssetIntelHydrateArgs, AssetIntelRun, OrganizationCandidates,
};
use uuid::Uuid;

pub async fn hydrate(
    resolver: &dyn ToolExecResolver,
    events: &dyn EventSink,
    candidates: &dyn CandidateSink,
    profiles: &dyn ProfilePatchSink,
    tools: &[ToolConfig],
    args: AssetIntelHydrateArgs,
) -> Result<AssetIntelRun, crate::AssetIntelError>;

// …hydrate_subsidiaries / run_providers_for_org 同样改造
```

3. `enrich.rs` 接 `enrich_organization` / `enrich_batch`；签名改造同上。

4. `hydrate/mod.rs`：

```rust
pub mod enrich;
pub mod run;

pub use enrich::{enrich_batch, enrich_organization};
pub use run::{hydrate, hydrate_subsidiaries, run_providers_for_org};
```

5. `asset_intel.rs` 不再直接 export 这些 fn（Tauri command 在 Task 10 调用 L2）；保留 `pub use` 仅为现有测试代码不破。

6. **在 L2 加 mock-driven 集成测试**：

`backend/crates/golish-asset-intel/tests/hydrate_mock_test.rs`：

```rust
use golish_asset_intel::ports::*;
// …手写 MockCandidateSink / MockEventSink / MockProfilePatchSink / MockToolExecResolver

#[tokio::test]
async fn hydrate_mock_smoke() {
    // 验收：给最简 fixture，hydrate 跑完后 mock sink 收到的 candidates 数 = fixture 预期
}
```

> 这一步只要 1-2 个 smoke test 即可，**不要**重写原 35 个测试。

7. 验证：

```bash
cargo check -p golish-asset-intel -p golish
cargo nextest run -p golish-asset-intel --status-level fail
cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail
```

**预期输出：** L2 新增的 hydrate smoke test 绿；L6 既有 asset_intel 测试 ≥ 35 passed 不回归。

**Commit：**

```bash
git add backend/crates/golish-asset-intel/src/hydrate/ \
        backend/crates/golish-asset-intel/src/lib.rs \
        backend/crates/golish-asset-intel/tests/hydrate_mock_test.rs \
        backend/crates/golish/src/tools/asset_intel.rs
git commit -m "T9: move hydrate/enrich business to L2 with port injection + add mock smoke test"
```

---

### Task 10 · 新建 L6 `tools/asset_intel_commands.rs`，6 个 thin Tauri command + 4 个 adapter

**文件：**
- 新建：`backend/crates/golish/src/tools/asset_intel_commands.rs`
- 修改：`backend/crates/golish/src/tools/mod.rs`（`pub mod asset_intel_commands;`）
- 修改：`backend/crates/golish/src/commands_facade/asset_intel.rs`（改 `pub use` 路径）
- 修改：`backend/crates/golish/src/tools/asset_intel.rs`（**仅保留** `pub use` 给 deprecated alias）
- 修改：`backend/crates/golish/Cargo.toml`（加 dep）

**步骤：**

1. 修改 `backend/crates/golish/Cargo.toml`，在 `[dependencies]` 加：

```toml
golish-asset-intel = { path = "../golish-asset-intel" }
```

2. 创建 `backend/crates/golish/src/tools/asset_intel_commands.rs`：

```rust
//! L6 thin wrapper for Asset Intel IPC commands.
//!
//! 6 个 #[tauri::command] + 4 个 adapter（CandidateSink / ProfilePatchSink /
//! EventSink / ToolExecResolver）。业务全部委托给 `golish-asset-intel` (L2)。

use std::path::PathBuf;
use std::sync::Arc;

use sqlx::PgPool;
use tauri::{AppHandle, State};
use uuid::Uuid;

use golish_asset_intel::ports::{CandidateSink, EventSink, ProfilePatchSink, ToolExecResolver};
use golish_asset_intel::AssetIntelError;
use golish_asset_intel::events::AssetIntelStreamEvent;
use golish_pentest::models::ToolConfig;
use golish_pentest_domain::asset_intel::{
    AssetIntelEnrichBatchArgs, AssetIntelEnrichBatchResult, AssetIntelEnrichOrganizationArgs,
    AssetIntelHydrateArgs, AssetIntelLookupRequest, AssetIntelLookupResult,
    AssetIntelProviderDescriptor, AssetIntelRun, OrganizationCandidate, OrganizationCandidates,
    ProfilePatch,
};

use crate::error::GolishError;
use crate::event_emitter::TauriEventEmitter;
use crate::state::DbState;
use crate::tools::pentest::PentestState;

// ----------------------- Adapters -----------------------

pub struct DbCandidateSink<'a>(pub &'a PgPool);

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
        .map_err(|e| AssetIntelError::Sink(e.to_string()))
    }
}

pub struct DbProfilePatchSink<'a>(pub &'a PgPool);

#[async_trait::async_trait]
impl<'a> ProfilePatchSink for DbProfilePatchSink<'a> {
    async fn apply_profile_patch(
        &self,
        org_id: Uuid,
        patch: ProfilePatch,
    ) -> Result<(), AssetIntelError> {
        // 转成 golish_db::repo::organizations::ProfilePatch
        let db_patch = golish_db::repo::organizations::ProfilePatch::from(patch);
        golish_db::repo::organizations::update_profile(self.0, org_id, db_patch)
            .await
            .map_err(|e| AssetIntelError::Sink(e.to_string()))?;
        Ok(())
    }
}

pub struct TauriEventSink {
    pub emitter: TauriEventEmitter,
}

impl EventSink for TauriEventSink {
    fn emit(&self, payload: AssetIntelStreamEvent) {
        let _ = self.emitter.emit(golish_asset_intel::events::ASSET_INTEL_EVENT, &payload);
    }
}

pub struct PentestStateExecResolver<'a> {
    pub pentest: &'a PentestState,
}

#[async_trait::async_trait]
impl<'a> ToolExecResolver for PentestStateExecResolver<'a> {
    async fn resolve_executable(&self, tool: &ToolConfig) -> Result<PathBuf, AssetIntelError> {
        let snapshot = self.pentest.tools_snapshot().await;
        let dirs = self.pentest.dirs_snapshot().await;
        golish_pentest::resolve_tool_executable(&snapshot, &dirs, &tool.id)
            .map_err(|e| AssetIntelError::Config(e.to_string()))
    }
}

// ----------------------- Tauri Commands -----------------------

#[tauri::command]
pub async fn asset_intel_list_providers(
    pentest: State<'_, PentestState>,
) -> Result<Vec<AssetIntelProviderDescriptor>, GolishError> {
    let snapshot = pentest.tools_snapshot().await;
    Ok(golish_asset_intel::provider::provider_descriptors_from_tools(&snapshot))
}

#[tauri::command]
pub async fn asset_intel_lookup_company(
    state: State<'_, DbState>,
    pentest: State<'_, PentestState>,
    request: AssetIntelLookupRequest,
) -> Result<AssetIntelLookupResult, GolishError> {
    let resolver = PentestStateExecResolver { pentest: &pentest };
    let snapshot = pentest.tools_snapshot().await;
    // …其余逻辑：遍历 lookup-capable provider，调 run_lookup_cli_provider，
    //   dedupe & sort —— 整段从旧 asset_intel_lookup_company 拷贝过来，
    //   把 run_lookup_cli_provider 改用 L2 版本（接 resolver）。
    todo!("详细实现见原 asset_intel.rs 内同名 fn")
}

#[tauri::command]
pub async fn asset_intel_hydrate(
    app: AppHandle,
    state: State<'_, DbState>,
    pentest: State<'_, PentestState>,
    args: AssetIntelHydrateArgs,
) -> Result<AssetIntelRun, GolishError> {
    let candidates = DbCandidateSink(state.pool());
    let profiles = DbProfilePatchSink(state.pool());
    let events = TauriEventSink { emitter: TauriEventEmitter::new(app.clone()) };
    let resolver = PentestStateExecResolver { pentest: &pentest };
    let tools = pentest.tools_snapshot().await;
    golish_asset_intel::hydrate::hydrate(
        &resolver, &events, &candidates, &profiles, &tools, args,
    )
    .await
    .map_err(GolishError::from)
}

#[tauri::command]
pub async fn asset_intel_hydrate_subsidiaries(/* 同上结构 */) -> Result<AssetIntelRun, GolishError> {
    todo!("结构同 asset_intel_hydrate，业务调 golish_asset_intel::hydrate::hydrate_subsidiaries")
}

#[tauri::command]
pub async fn asset_intel_enrich_organization(/* 同上 */) -> Result<AssetIntelRun, GolishError> {
    todo!("调 golish_asset_intel::hydrate::enrich_organization")
}

#[tauri::command]
pub async fn asset_intel_enrich_batch(/* 同上 */) -> Result<AssetIntelEnrichBatchResult, GolishError> {
    todo!("调 golish_asset_intel::hydrate::enrich_batch")
}
```

> **重要**：上面 4 个 `todo!()` **必须**在本 task 内被替换为实际从原 `asset_intel.rs` 拷贝并改造的代码 —— 不要把 `todo!` 留到 commit。它们之所以以 `todo!` 形式出现在计划里，是因为完整实现 200+ 行直接 inline 进 plan 会让 plan 超 4000 行；执行 task 时按原 fn 镜像写出来。

3. 修改 `backend/crates/golish/src/tools/mod.rs`：

```rust
pub mod asset_intel;            // ← 旧文件，本 task 仅清空 + 留 `pub use` deprecation
pub mod asset_intel_commands;   // ← 新增
```

4. 修改 `backend/crates/golish/src/commands_facade/asset_intel.rs`，把：

```rust
pub use crate::tools::asset_intel::{
    asset_intel_enrich_batch, asset_intel_enrich_organization, asset_intel_hydrate,
    asset_intel_hydrate_subsidiaries, asset_intel_list_providers, asset_intel_lookup_company,
};
```

改为：

```rust
pub use crate::tools::asset_intel_commands::{
    asset_intel_enrich_batch, asset_intel_enrich_organization, asset_intel_hydrate,
    asset_intel_hydrate_subsidiaries, asset_intel_list_providers, asset_intel_lookup_company,
};
```

5. 把旧 `backend/crates/golish/src/tools/asset_intel.rs` 改成 deprecation alias（最终 Task 11 删）：

```rust
//! Deprecated: business moved to `golish-asset-intel` (L2);
//! Tauri commands moved to `crate::tools::asset_intel_commands`.
//! This file will be removed in Task 11.

#![allow(unused_imports)]

pub use golish_asset_intel::events::ASSET_INTEL_EVENT;
pub use golish_pentest_domain::asset_intel::*;
```

6. 验证：

```bash
cargo check -p golish
cargo nextest run -p golish --lib --status-level fail
just check                                # 跑全部静态检查
pnpm exec tsc --noEmit
```

**预期输出：** `cargo check` exit 0；`cargo nextest -p golish` 全绿（含 asset_intel 35+ passed）；`just check` exit 0；`tsc` exit 0。**如果任一不绿，停下 debug，不准把 broken commit 推上去。**

**Commit：**

```bash
git add backend/crates/golish/src/tools/asset_intel_commands.rs \
        backend/crates/golish/src/tools/mod.rs \
        backend/crates/golish/src/commands_facade/asset_intel.rs \
        backend/crates/golish/src/tools/asset_intel.rs \
        backend/crates/golish/Cargo.toml
git commit -m "T10: add asset_intel_commands.rs thin wrappers + 4 adapters; route IPC via L2"
```

---

### Task 11 · 删空旧 `tools/asset_intel.rs`，跑全套 precommit，落 progress / feature_list / arch docs

**文件：**
- 删除：`backend/crates/golish/src/tools/asset_intel.rs`
- 修改：`backend/crates/golish/src/tools/mod.rs`（删 `pub mod asset_intel;`）
- 修改：`scripts/check_dag.py`（或 `.github/workflows/arch-check.yml`） — 加 `golish-asset-intel` 进 L2 节点
- 修改：`docs/architecture.md`
- 修改：`agent-progress.md`
- 修改：`feature_list.json`

**步骤：**

1. 用 Grep 确认没有其它文件还在 `use crate::tools::asset_intel::`：

```bash
rg 'crate::tools::asset_intel(::|;)' backend/crates/golish/src/ | rg -v asset_intel_commands
```

**预期输出：** 应为空。如果有命中，**先**改成对应的新路径（`crate::tools::asset_intel_commands::...` 或 `golish_asset_intel::...` 或 `golish_pentest_domain::asset_intel::...`），再继续。

2. 删除文件：

```bash
rm backend/crates/golish/src/tools/asset_intel.rs
```

3. 修改 `backend/crates/golish/src/tools/mod.rs`，删除 `pub mod asset_intel;` 那一行。

4. 修改 DAG guard。先查清楚：

```bash
rg golish-pentest backend/crates/golish-pentest/  -l --type yaml
rg -l L2 scripts/
```

找到声明 L2 节点的位置（设计文档里写过位于 `.github/workflows/arch-check.yml` 或 `scripts/check_dag.py`，以实际为准），追加 `golish-asset-intel` 到 L2 名单。

5. 修改 `docs/architecture.md`：在 L2.assets 表格里加一行：

```markdown
| `golish-asset-intel` | core, pentest-domain, pentest | Asset Intel domain：provider fan-out / normalize / hydrate / enrich / lookup |
```

总 crate 数从 48 改为 49（搜索全文中 `48 crates` 改为 `49 crates`）。

6. 修改 `agent-progress.md`：在「会话记录」段顶部加一条新记录，包含：
   - 本轮目标
   - 跑过的验证命令 + 退出码
   - 已记录证据（11 个 commit hash）
   - 风险 / 未提交的半成品
   - 下一步建议

7. 修改 `feature_list.json`：把 `extract-golish-asset-intel-crate` 条目状态切到 `passing`，填 `evidence` 字段（含设计文档路径、实施计划路径、各 task 验证输出）。

8. **全套 precommit**：

```bash
just precommit
```

**预期输出：** exit 0。如果有任何 fail，立即排查；不允许带 broken 提交。

9. **DAG check 显式跑一次**：

```bash
python3 scripts/check_dag.py    # 或对应路径
```

**预期输出：** exit 0；输出里能看到 `golish-asset-intel` 被识别为 L2。

10. **ts-rs byte-equal 检查**（手工）：

```bash
git status frontend/lib/generated/
```

**预期输出：** 应**空**或仅有 trivial 顺序差异。**如果有实质 diff（字段、类型、签名变化），立即停下回到 Task 3 检查 ts-rs derive 是否漏迁。**

**Commit：**

```bash
git add backend/crates/golish/src/tools/asset_intel.rs \
        backend/crates/golish/src/tools/mod.rs \
        scripts/check_dag.py \
        docs/architecture.md \
        agent-progress.md \
        feature_list.json
git commit -m "T11: delete old tools/asset_intel.rs; bump crate count to 49; update docs/progress/feature_list"
```

---

## 中途断点（每完成 3 个 task 检查一次）

| 断点 | 完成判定 | 不达标怎么办 |
|---|---|---|
| **断点 A（T3 后）** | DTO 全在 L1；35 个 asset_intel 测试全绿；前端 tsc 全绿 | 退回 T2/T3 检查 ts-rs derive 是否漏迁 |
| **断点 B（T6 后）** | normalize / profile / provider 三大块在 L2；L6 仅作 `pub use` 透传 | 退回 T4/T5/T6 检查 fn 签名是否被无意改动 |
| **断点 C（T9 后）** | 全部业务 fn 在 L2；L2 mock smoke test 绿；L6 还能编译 | 退回 T8 检查 EventSink / ToolExecResolver 是否漏适配 |
| **断点 D（T10 后）** | 6 个 IPC 通过 L6 thin wrapper 调 L2；just check 全绿 | 退回 T10 检查 4 个 adapter 是否漏写或类型不匹配 |

---

## 计划自检

### 规格覆盖度

| 设计章节 | 实现任务 |
|---|---|
| 4.1 L1 DTO | T2 + T3 |
| 4.2 L2 normalize | T4 + T5 |
| 4.3 L2 provider | T6 |
| 4.4 L2 runtime | T8 |
| 4.5 L2 hydrate / enrich | T9 |
| 4.6 L2 lookup | T7 |
| 5. Trait Port | T1（声明） + T8/T9（消费） + T10（adapter 实现） |
| 6. 迁移路径 | T1–T11 |
| 9.1 单元层 | T1–T11 每 task 都有 nextest |
| 9.2 静态 | T11 just precommit |
| 9.3 前端 IPC | T3 / T11 tsc 检查 |
| 9.4 byte-equal | T11 ts-rs diff 检查 |
| 9.5 端到端 | 计划外（用户做） |

### 占位符扫描

- `todo!()` 出现在 Task 10 的 4 个 Tauri command stub，**明确标注必须替换**；不允许 commit 时仍是 `todo!`。
- 计划中没有 "待定" / "TODO" / "类似上面" 等无效话术。

### 类型一致性

- `OrganizationCandidate` / `OrganizationCandidateKind` / `OrganizationCandidates` 在 T2 移到 L1 后，T3-T11 全部使用 `golish_pentest_domain::asset_intel::*` 路径，一致。
- `ProfilePatch` 在 L1 与 L6 同名但是不同 type；T10 的 `DbProfilePatchSink` 用 `From` 转换；明确标注。
- `AssetIntelStreamEvent` 真定义在 L1，L2 `events.rs` 仅 `pub use`，L6 用 L2 路径访问。
- `run_cli_json_provider` / `run_http_json_provider` 在 T8 改造签名后，T9 调用方使用新签名，所有调用点同步。

---

## 启动检查表（pre-flight checklist · 2026-05-25 体检补齐）

> 本节由 2026-05-25 一次专项体检追加，用来回答"现在能不能动手"。每项都是**事实判定**，
> 不是任务说明；勾选完之前不要进入 Task 1。重新启动会话时也应先扫一遍。

### 0. 启动前需要存在的资产

- [x] **设计文档**：`docs/design/2026-05-25-extract-golish-asset-intel-crate.md`（346 行，已 untracked，需在动手前先 `git add` 入库）
- [x] **实施计划**：本文件（1,289 行 + 本节追加 · 11 个 TDD task + 4 个中途断点 + 占位符扫描）
- [x] **feature_list 条目**：`extract-golish-asset-intel-crate`（priority=4 · status=not_started）

### 1. 阻塞项实际状态（2026-05-25 体检结论）

| # | 启动前提（原始） | 实际状态 | 说明 |
|---|---|---|---|
| 1 | `asset-intel-hydrate-disambiguation` 不是 `in_progress` | ⚠ **历史游离 in_progress** | 2026-05-22 一轮该任务的 A/B/C 三件套**代码已全部 commit 入库**（commit `59ce79b` two-phase hydrate + `900e5dc` 0.zone expand）；77 个 Rust + 27 vitest 全过；后续 5-23 / 5-24 几轮基于此继续扩展到 35 → 39 asset_intel 测试 + 62 pentest 测试全过。**唯一未完成项**：用户手动 E2E（验证 Look up 弹候选 / SQL 查 credit_code 写入主档案）。这和 `chat-model-selector-tests` / `provider-form-tests` 一样，是 feature_list 状态字段未及时切的历史游离 in_progress |
| 2 | `git status` 干净（不含本计划外未提交的代码改动） | ✅ 干净 | `git diff --stat HEAD -- backend/ frontend/ resources/` 为空（0 行代码改动）；当前未提交改动全在 `docs/` + `agent-progress.md` + `feature_list.json` + `AGENTS.md`，**不会**与本 refactor 冲突 |
| 3 | baseline `cargo nextest run -p golish --lib -E 'test(asset_intel)'` exit 0 / 35+ passed | 🟡 进行中 | 体检会话已发起一次 baseline 测试，结果由本节末"baseline 证据"小节补完 |
| 4 | baseline `cargo nextest run -p golish-pentest` exit 0 / 62 passed | ✅ **62 passed / 7 skipped exit 0** | 体检会话 2026-05-25 实跑确认 |

### 2. 启动前必须做的 3 个动作

- [ ] **D1 · 决定 hydrate-disambiguation 的最终状态**（任选其一）：
  - **D1a** 用户跑过手动 E2E 且接受效果 → 把 `feature_list.json` 该条目 `status` 改为 `passing`，填 `completion: { completed_at: "2026-05-25", outcome: "passing" }`
  - **D1b** 用户尚未跑 E2E 但代码已入库 → 把 `status` 改为 `not_started` + 在 `notes` 追加"代码层面已完成（commit 59ce79b + 900e5dc · 77+27 测试全过），仅待用户手动 E2E；E2E pending 不阻塞 extract-asset-intel-crate refactor"
  - **D1c** 创建新状态 `awaiting_user_e2e` → 需要先扩 `feature_list.json` `status_values`（影响 schema，慎选）
  - **推荐 D1b**，最小改动，不影响后续 refactor 启动
- [ ] **D2 · 把两份本计划文档（设计 + 实施）`git add` 入库** + commit message `docs(asset-intel): plan to extract golish-asset-intel L2 crate + checklist`（独立 commit，不混入 D1）
- [ ] **D3 · 在 `feature_list.json` 把 `extract-golish-asset-intel-crate` 状态从 `not_started` 切到 `in_progress`**（开工标识；动手 Task 1 前的最后一步）

### 3. 风险复检（启动前 1 小时再扫一次）

- ⚠️ **Cargo 编译时间**：`golish` 是 L6 应用 crate（52,743 行），cold-cache `cargo nextest run -p golish` 在 macOS M-series 上约 **7-9 分钟**完成 compile，跑测仅 < 1 秒。Task 1-11 每个都涉及 `cargo check -p golish`，预估每个 task 约 1-3 分钟 incremental check；全套 11 task 估 1-2 工作日。
- ⚠️ **后台任务并行**：本会话若同时跑 `cargo nextest -p golish` 与 `-p golish-pentest`，会互相抢 cargo target lock + sysroot lock，编译时长翻倍。**建议串行跑**，跑前用 `pkill cargo` 确认无残留进程。
- ⚠️ **`just precommit` 整仓 fail**：仓库本来就有 preexisting biome 警告（pty.ts 排序 / useTaskPlanState 等）+ M2 cherry-pick 遗留的 8 个 `failure_kind` PlanStep struct 字段缺失编译错。Task 11 第 8 步要求 `just precommit exit 0`，但**这两类 preexisting fail 与本 refactor 无关**——遇到时先用 `git stash` 隔离对照，如果不是新增的，记入 progress 但不阻塞本任务收尾。

### 4. baseline 证据（2026-05-25 体检会话补齐）

| 命令 | 期望 | 实测 |
|---|---|---|
| `cargo nextest run -p golish-pentest --status-level fail` | 62 passed | ✅ **62 passed / 7 skipped exit 0**（编译 8m 06s + 跑测 0.789s） |
| `cargo nextest run -p golish --lib -E 'test(asset_intel)' --status-level fail` | 35-39 passed | ⏳ 体检会话发起后正在编译中（cold-cache 预估 ~9 min），结果将由后续会话补填到本表 |
| `git diff --stat HEAD -- backend/ frontend/ resources/` | 0 行 | ✅ 输出为空，证明现有未提交改动全是 docs 类 |
| `git log --oneline -5` 含 `feat(asset-intel)` commit | ≥ 1 条 | ✅ `900e5dc feat(asset-intel): 0.zone expand` + `59ce79b feat(asset-intel): scaffold provider abstraction + flat schema + two-phase hydrate (WIP)` |

### 5. 启动指令模板（D1-D3 完成后即可执行）

```bash
git add docs/design/2026-05-25-extract-golish-asset-intel-crate.md \
        docs/superpowers/plans/2026-05-25-extract-golish-asset-intel-crate.md
git commit -m "docs(asset-intel): plan to extract golish-asset-intel L2 crate + checklist"

# D1b 推荐路径：
# 编辑 feature_list.json:
#   - hydrate-disambiguation: status "in_progress" -> "not_started"
#     notes 追加 "代码层面已完成 (commit 59ce79b + 900e5dc · 77+27 测试全过), 仅待用户手动 E2E"
#   - extract-golish-asset-intel-crate: status "not_started" -> "in_progress"
python3 -m json.tool feature_list.json >/dev/null

git add feature_list.json agent-progress.md
git commit -m "chore(harness): rebalance feature_list - hydrate-disambig code-done, start extract-asset-intel"

# 然后按 Task 1 步骤执行
```
