# S1-2 端口化横向耦合 实现计划

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans/` 逐任务实现本计划，每个任务独立验证 + 独立 commit。先读设计 `docs/design/2026-05-30-s1-2-port-horizontal-coupling.md`。

**目标：** 把命令层对别的服务 `golish-db` repo 的直读，改为走**提供方服务端口**（`*Port` trait + in-proc 适配器），逐条拔掉 S1-1 守卫的 `ALLOWLIST`，为 DB-per-service 抽服务铺平路。本计划**完整实现走路骨架 S1-2a（`VaultReadPort`）**，其余切片（S1-2b–g）在 §路线图列出，各自另写计划。
**架构：** 新增 `golish/src/ports/<service>/`：trait 定义本服务读/写方法（remote-ready，禁 `PgPool`/闭包），in-proc 适配器是唯一 `use golish_db::repo::<本服务>` 处；消费方持 `Arc<dyn Port>` 注入。守卫加 `DOMAIN_RULES ("ports/<service>","<service>")` 使适配器 owner==domain 合法，同时删消费方 allowlist 条目 → ratchet 净前进。零业务语义变更。
**技术栈：** Rust 2021（`async_trait`、`sqlx::PgPool`、`uuid`、`serde_json`）、`cargo nextest`、Python 守卫 `scripts/check_repo_ownership.py`、`just`。

---

## 背景与依据（证据）

- 设计：`docs/design/2026-05-30-s1-2-port-horizontal-coupling.md`（§2 两层端口、§3 清单、§4 模式、§5 骨架）。
- 前置：S1-1 守卫 `scripts/check_repo_ownership.py`（`ALLOWLIST` 30 条，124-157 行；其中 vault 两条 145、138 行 + raw-sql `vault_ops.rs` 一条 180 行）。
- 已读真实代码：
  - 消费方 `tools/pentest_bridge/vault_ops.rs`（`VaultTool`，`pool:Arc<PgPool>`；`list`→`repo::vault::list_name_meta_by_project:167`、`get`→`get_secret_by_name_project:201`、`store`→裸 INSERT `:122`）。
  - 消费方 `tools/pentest_bridge/auth_probe.rs`（`AuthProbeTool::resolve_token`→`repo::vault::get_value_by_name_project:253`）。
  - 提供方 `golish-db/src/repo/vault.rs`（`list_name_meta_by_project:314`、`get_secret_by_name_project:330`、`get_value_by_name_project:347`、`insert_full:120`）。
  - 构造点 `tools/pentest_bridge/mod.rs:34-53`（`create_pentest_bridge_tools(pool,…)` 构造 `VaultTool::new(pool.clone()):42` 与 `AuthProbeTool::new(pool.clone()):45`）。
  - 加密 helper `super::super::vault::{obfuscate_value,deobfuscate_value}`（留消费方，端口不碰）。
  - crate 根 `golish/src/lib.rs:43`（`pub mod tools;`）——在此加 `pub mod ports;`。

### 范围内（S1-2a）
1. 新增 `golish/src/ports/`（mod 根 + `platform/vault.rs`）：`VaultReadPort` trait + `PgVaultAdapter`。
2. 迁移 `vault_ops.rs`（list/get/store）与 `auth_probe.rs`（resolve_token）走端口。
3. 构造点注入 `PgVaultAdapter`。
4. 守卫：加 `("ports/platform","platform")`，删 vault 两条 `ALLOWLIST` + 一条 `RAW_SQL_ALLOWLIST`；`just arch` 绿。
5. 端口契约单测 + `cargo nextest -p golish` + 文档/feature_list/progress 收尾。

### 范围外（本计划不做）
- S1-2b–g（recon/vuln/agent-log/pentest-plan 端口、scan_queue 清理、asset_intel 模块解耦）——见路线图，各自写计划。
- 不改加密语义、不改 SQL、不动 `DbRepoProvider`（消费方端口已存在，设计 §2.1）。
- 不下沉 trait 到独立 crate（阶段 4，设计 §4.4）。

---

## 文件结构（S1-2a）

| 文件 | 动作 | 职责 |
|---|---|---|
| `backend/crates/golish/src/ports/mod.rs` | 新建 | `pub mod platform;` + 模块级 doc（端口层契约） |
| `backend/crates/golish/src/ports/platform/mod.rs` | 新建 | `pub mod vault;` + `pub use vault::{VaultReadPort, PgVaultAdapter};` |
| `backend/crates/golish/src/ports/platform/vault.rs` | 新建 | `VaultReadPort` trait + `PgVaultAdapter` 实现 + 契约测试 |
| `backend/crates/golish/src/lib.rs` | 修改 | `pub mod tools;` 后加 `pub mod ports;` |
| `backend/crates/golish/src/tools/pentest_bridge/vault_ops.rs` | 修改 | `VaultTool` 持 `Arc<dyn VaultReadPort>`；list/get/store 走端口；删 `use golish_db::repo::vault` |
| `backend/crates/golish/src/tools/pentest_bridge/auth_probe.rs` | 修改 | `AuthProbeTool` 持 `Arc<dyn VaultReadPort>`；`resolve_token` 走端口 |
| `backend/crates/golish/src/tools/pentest_bridge/mod.rs` | 修改 | `create_pentest_bridge_tools` 构造并注入 `PgVaultAdapter` |
| `scripts/check_repo_ownership.py` | 修改 | `DOMAIN_RULES` 加 `("ports/platform","platform")`；删 2 条 `ALLOWLIST` + 1 条 `RAW_SQL_ALLOWLIST` |
| `docs/architecture.md` | 修改 | data-ownership 节补「端口层」一句 + S1-2 进度 |
| `agent-progress.md` / `feature_list.json` | 修改 | 收尾记录 + 状态 |

---

## Task 1 — 新建端口层：`VaultReadPort` trait + `PgVaultAdapter`（TDD）

**文件：** 新建 `backend/crates/golish/src/ports/mod.rs`、`ports/platform/mod.rs`、`ports/platform/vault.rs`；改 `golish/src/lib.rs`。

**步骤 1.1：** `ports/mod.rs`：

```rust
//! Provider-side service ports (servitization S1-2).
//!
//! Each submodule holds one service's outbound port: a `*Port` trait (the
//! remote-ready contract — only serializable params, no `PgPool`/closures) and
//! an in-proc adapter (`Pg*Adapter`) that is the ONLY place allowed to call
//! `golish_db::repo::<that service>`. Consumers hold `Arc<dyn *Port>` and never
//! touch another service's repo directly. See
//! docs/design/2026-05-30-s1-2-port-horizontal-coupling.md.

pub mod platform;
```

**步骤 1.2：** `ports/platform/mod.rs`：

```rust
//! Platform service ports (vault / notes / terminal logs).

pub mod vault;

pub use vault::{PgVaultAdapter, VaultReadPort};
```

**步骤 1.3：** `ports/platform/vault.rs`（trait + 适配器；方法逐字镜像 `golish-db/src/repo/vault.rs`）：

```rust
//! `VaultReadPort` — platform's credential vault as a service port.
//!
//! In-proc adapter mirrors `golish_db::repo::vault` exactly (same SQL,
//! same project-scope/IDOR semantics). Encryption stays in the caller; the
//! port only moves already-obfuscated values, identical to today.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// Outbound port for the platform credential vault. Remote-ready: only
/// serializable params/returns, no pool/closures leak across the boundary.
#[async_trait]
pub trait VaultReadPort: Send + Sync {
    /// `(name, entry_type, username, notes)` for a project, alphabetical.
    async fn list_name_meta_by_project(
        &self,
        project_path: &str,
    ) -> anyhow::Result<Vec<(String, String, String, String)>>;

    /// `(enc_value, username, entry_type)` for the first entry matching `name`.
    async fn get_secret_by_name_project(
        &self,
        name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<(String, String, String)>>;

    /// Encrypted `value` for the first entry matching `name`.
    async fn get_value_by_name_project(
        &self,
        name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<String>>;

    /// Insert a vault entry with an already-encrypted value.
    #[allow(clippy::too_many_arguments)]
    async fn store_entry(
        &self,
        id: Uuid,
        name: &str,
        entry_type: &str,
        enc_value: &str,
        username: &str,
        notes: &str,
        project: &str,
        project_path: &str,
    ) -> anyhow::Result<()>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgVaultAdapter {
    pool: Arc<PgPool>,
}

impl PgVaultAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl VaultReadPort for PgVaultAdapter {
    async fn list_name_meta_by_project(
        &self,
        project_path: &str,
    ) -> anyhow::Result<Vec<(String, String, String, String)>> {
        Ok(golish_db::repo::vault::list_name_meta_by_project(self.pool.as_ref(), project_path).await?)
    }

    async fn get_secret_by_name_project(
        &self,
        name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        Ok(golish_db::repo::vault::get_secret_by_name_project(self.pool.as_ref(), name, project_path).await?)
    }

    async fn get_value_by_name_project(
        &self,
        name: &str,
        project_path: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(golish_db::repo::vault::get_value_by_name_project(self.pool.as_ref(), name, project_path).await?)
    }

    async fn store_entry(
        &self,
        id: Uuid,
        name: &str,
        entry_type: &str,
        enc_value: &str,
        username: &str,
        notes: &str,
        project: &str,
        project_path: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::vault::insert_full(
            self.pool.as_ref(),
            id,
            name,
            entry_type,
            enc_value,
            username,
            notes,
            project,
            &serde_json::json!([]),
            "",
            Some(project_path),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time guarantee the port is object-safe (consumers store
    // `Arc<dyn VaultReadPort>`).
    #[test]
    fn vault_read_port_is_object_safe() {
        fn _assert(_: &dyn VaultReadPort) {}
    }
}
```

> 注意 `insert_full` 第 9 参 `tags: &serde_json::Value` 与第 10 参 `source_url: &str`：原裸 INSERT（`vault_ops.rs:122`）未写 tags/source_url，故端口传 `json!([])` + `""` 保持等价（DB 列默认行为一致；执行时若 schema 默认值不同，用 `repo::vault::insert_full` 的默认空值即可，已与 `create` 路径一致）。

**步骤 1.4：** `golish/src/lib.rs` 在 `pub mod tools;`（43 行）后加：

```rust
pub mod ports;
```

**验证：**

```bash
cd backend && cargo check -p golish 2>&1 | tail -20
cargo nextest run -p golish ports::platform::vault 2>&1 | tail -20
```

预期：`cargo check` 通过；`vault_read_port_is_object_safe` 测试 PASS。

**提交：**

```bash
git add backend/crates/golish/src/ports backend/crates/golish/src/lib.rs
git commit -m "feat(arch): add VaultReadPort + PgVaultAdapter (S1-2a ports skeleton)"
```

---

## Task 2 — 迁移 `VaultTool` 走端口（list/get/store）

**文件：** 修改 `backend/crates/golish/src/tools/pentest_bridge/vault_ops.rs`。

**步骤 2.1：** 改 `use` 区与结构体。删 `use golish_db::repo::audit::PentestAudit;` 之外的 vault 直查依赖不需要——保留 `PentestAudit`（audit 是 SHARED_REPOS，合法）。把字段从 `pool` 换/补为端口与审计用 pool：

```rust
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use golish_core::Tool;
use golish_db::repo::audit::PentestAudit;

use super::super::vault;
use crate::ports::platform::VaultReadPort;

pub struct VaultTool {
    pool: Arc<PgPool>,
    vault: Arc<dyn VaultReadPort>,
}

impl VaultTool {
    pub fn new(pool: Arc<PgPool>, vault: Arc<dyn VaultReadPort>) -> Self {
        Self { pool, vault }
    }
}
```

> `pool` 保留：`PentestAudit::started(self.pool.as_ref(), …)` 仍用它（audit 是 shared）。仅**跨服务的 vault 读写**改走端口。

**步骤 2.2：** `store` 动作（原 `:122-137` 裸 INSERT）改端口：

```rust
                let (value_sha256, value_length) = credential_fingerprint(value);
                let enc_value = vault::obfuscate_value(value);
                let id = Uuid::new_v4();

                self.vault
                    .store_entry(
                        id,
                        name,
                        cred_type,
                        &enc_value,
                        username,
                        notes,
                        &project_path,
                        &project_path,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to store credential: {}", e))?;
```

**步骤 2.3：** `list` 动作（原 `:167`）改：

```rust
                let rows = self
                    .vault
                    .list_name_meta_by_project(&project_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to list credentials: {}", e))?;
```

**步骤 2.4：** `get` 动作（原 `:201`）改：

```rust
                let row = self
                    .vault
                    .get_secret_by_name_project(name, &project_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to get credential: {}", e))?;
```

**步骤 2.5：** 确认文件内已无 `golish_db::repo::vault` 字样（audit 的 `golish_db::repo::audit` 保留合法）：

```bash
grep -n "golish_db::repo::vault" backend/crates/golish/src/tools/pentest_bridge/vault_ops.rs; echo "should_be_empty"
grep -n "sqlx::query" backend/crates/golish/src/tools/pentest_bridge/vault_ops.rs; echo "should_be_empty"
```

预期：两个 grep 均无输出（裸 SQL 与 vault 直查都已消除）。

**验证（编译，注意构造点未改会暂时 RED，下一任务修）：** 本任务编译会因 `VaultTool::new` 多了一个参数而在 `mod.rs:42` 报错——这是预期，Task 4 修复。本任务不单独 `cargo check`，与 Task 3/4 连续提交后统一验证。

**提交：**

```bash
git add backend/crates/golish/src/tools/pentest_bridge/vault_ops.rs
git commit -m "refactor(arch): route VaultTool through VaultReadPort (S1-2a)"
```

---

## Task 3 — 迁移 `AuthProbeTool::resolve_token` 走端口

**文件：** 修改 `backend/crates/golish/src/tools/pentest_bridge/auth_probe.rs`。

**步骤 3.1：** 在结构体定义处补字段（结构体定义在文件上半部，grep 定位 `struct AuthProbeTool` 与其 `impl AuthProbeTool { pub fn new`）。加 import 与字段：

```rust
use crate::ports::platform::VaultReadPort;
```

把 `AuthProbeTool` 增 `vault: Arc<dyn VaultReadPort>` 字段，`new` 增同名参数（保留原 `pool`，其它方法如 `load_endpoints`、audit 仍用 pool）。

**步骤 3.2：** `resolve_token`（`:253`）改：

```rust
        let row = self
            .vault
            .get_value_by_name_project(name, project_path)
            .await
            .map_err(|e| anyhow::anyhow!("vault lookup failed: {}", e))?;
```

**步骤 3.3：** 确认无 vault 直查：

```bash
grep -n "golish_db::repo::vault" backend/crates/golish/src/tools/pentest_bridge/auth_probe.rs; echo "should_be_empty"
```

预期：无输出。

**提交：**

```bash
git add backend/crates/golish/src/tools/pentest_bridge/auth_probe.rs
git commit -m "refactor(arch): route AuthProbeTool through VaultReadPort (S1-2a)"
```

---

## Task 4 — 构造点注入 `PgVaultAdapter`

**文件：** 修改 `backend/crates/golish/src/tools/pentest_bridge/mod.rs`。

**步骤 4.1：** 在 `create_pentest_bridge_tools`（`:34-53`）开头构造一个共享适配器，注入两个工具：

```rust
pub fn create_pentest_bridge_tools(
    pool: Arc<PgPool>,
    config_manager: Arc<golish_pentest::ConfigManager>,
    app_handle: Option<tauri::AppHandle>,
) -> Vec<Arc<dyn Tool>> {
    let vault_port: Arc<dyn crate::ports::platform::VaultReadPort> =
        Arc::new(crate::ports::platform::PgVaultAdapter::new(pool.clone()));
    vec![
        Arc::new(ManageTargetsTool::new(pool.clone())),
        Arc::new(RecordFindingTool::new(pool.clone())),
        Arc::new(VaultTool::new(pool.clone(), vault_port.clone())),
        Arc::new(JsCollectTool::new(pool.clone())),
        Arc::new(JsExtractApisTool::new(pool.clone())),
        Arc::new(AuthProbeTool::new(pool.clone(), vault_port.clone())),
        Arc::new(RunPipelineTool::new(
            pool.clone(),
            config_manager.clone(),
            app_handle.clone(),
        )),
        Arc::new(FlowComposeTool::new(pool, config_manager, app_handle)),
    ]
}
```

**验证（此时三处改动闭合，应编译）：**

```bash
cd backend && cargo check -p golish 2>&1 | tail -20
cargo clippy -p golish --all-targets 2>&1 | grep -E "warning|error" | head -20; echo "clippy_done"
cargo nextest run -p golish 2>&1 | tail -15
```

预期：`cargo check` 通过；clippy 无新增 warning；golish 测试不回归。

**提交：**

```bash
git add backend/crates/golish/src/tools/pentest_bridge/mod.rs
git commit -m "refactor(arch): inject PgVaultAdapter into vault/auth tools (S1-2a)"
```

---

## Task 5 — 拔 ratchet：守卫 DOMAIN_RULES + 删 allowlist 条目

**文件：** 修改 `scripts/check_repo_ownership.py`。

**步骤 5.1：** 在 `DOMAIN_RULES`（92-119 行）**列表顶部**（first-match-wins，必须在 `("ai/","agent")` 等之前）加端口适配器域映射：

```python
DOMAIN_RULES: list[tuple[str, str]] = [
    # provider-side port adapters belong to the service they expose (S1-2).
    ("ports/platform", "platform"),
    ("tools/asset_intel", "recon"),
    # … 原有规则保持不变 …
]
```

**步骤 5.2：** 从 `ALLOWLIST`（124-157 行）**删除**这两行：

```python
        ("tools/pentest_bridge/auth_probe.rs", "vault"),
        ("tools/pentest_bridge/vault_ops.rs", "vault"),
```

**步骤 5.3：** 从 `RAW_SQL_ALLOWLIST`（162-195 行）**删除**这一行：

```python
        "tools/pentest_bridge/vault_ops.rs",
```

> `auth_probe.rs` 仍在 `RAW_SQL_ALLOWLIST`（它别处可能仍有裸 SQL，属 P0-3，本切片不动）——执行时先 `grep -n "sqlx::query" auth_probe.rs` 确认：若已无裸 SQL 也一并删除该行，否则保留。

**步骤 5.4：** 跑守卫，预期净前进且全绿：

```bash
python3 scripts/check_repo_ownership.py; echo "exit=$?"
just arch; echo "arch_exit=$?"
```

预期：`[repo-ownership] OK clean` / `exit=0`；`just arch` 两守卫绿 `arch_exit=0`。
> 若报 `tools/pentest_bridge/vault_ops.rs: ... has no domain` 或 `ports/platform/vault.rs: ... -> repo::vault (owned by platform)` 之类——说明 DOMAIN_RULES 顺序或前缀写错，回 5.1 修正。

**提交：**

```bash
git add scripts/check_repo_ownership.py
git commit -m "chore(arch): pull ratchet — vault coupling now via VaultReadPort (S1-2a)"
```

---

## Task 6 — 验证、文档与收尾

**文件：** 修改 `docs/architecture.md`、`agent-progress.md`、`feature_list.json`。

**步骤 6.1：** `docs/architecture.md` 的「Backend: data ownership」节（285-300 行附近）末尾补一句进度：

```markdown
> **S1-2 (in progress)**: cross-service reads migrate to provider-side
> service ports under `golish/src/ports/<service>/` (trait + in-proc adapter);
> each migrated call-site drops an allow-list entry. First port shipped:
> `VaultReadPort` (platform). See
> `docs/design/2026-05-30-s1-2-port-horizontal-coupling.md`.
```

**步骤 6.2：** 全套完成定义（AGENTS.md §3）：

```bash
just precommit; echo "precommit_exit=$?"
```

预期：全绿 `precommit_exit=0`（本切片改了 Rust，**必须**跑全套，不能像 S1-1 那样略过）。把命令与退出码、关键输出片段复制到 `agent-progress.md` 的「已记录证据」。

**步骤 6.3：** `feature_list.json` 把 `arch-s1-2-port-horizontal-coupling`（登记见下「feature_list 登记」）改 `passing` 并回填 `evidence`（端口文件、迁移文件、allowlist 30→28、precommit 证据）。

**步骤 6.4：** `agent-progress.md` 顶部加本轮会话记录（目标 / 已完成 / 验证 / 证据 / 下一步 = S1-2b ReconReadPort）。

**提交：**

```bash
git add docs/architecture.md agent-progress.md feature_list.json
git commit -m "docs(arch): record S1-2a VaultReadPort + feature_list/progress"
```

---

## feature_list 登记（S1-2 父条目，初始 not_started）

> 本计划由主控 MCP-1 起草时已写入 `feature_list.json`；执行 S1-2a 前把 `status` 改 `in_progress`（同时确保 `target-surface-workbench` 已让出唯一 in_progress 名额，AGENTS.md §2.1）。

```json
{
  "id": "arch-s1-2-port-horizontal-coupling",
  "priority": 1,
  "area": "backend/crates/golish/src/ports + tools/pentest_bridge + scripts/check_repo_ownership.py",
  "title": "S1-2 portify cross-service horizontal coupling (provider-side service ports)",
  "user_visible_behavior": "无用户可见行为变化。命令层不再直查别的服务的 golish-db repo，改走 *Port（trait + in-proc 适配器）；逐条拔掉 S1-1 守卫 ALLOWLIST（30→目标 1）。为 DB-per-service 抽服务铺路。S1-2a 先落 VaultReadPort（platform）。",
  "status": "not_started",
  "verification": [
    "S1-2a: just arch → exit 0 且 ALLOWLIST 减 2 (vault) + RAW_SQL_ALLOWLIST 减 1",
    "S1-2a: cargo nextest -p golish → 不回归；端口 round-trip 契约测试通过",
    "vault_ops.rs / auth_probe.rs 内 grep 'golish_db::repo::vault' 为空",
    "just precommit → exit 0（本切片含 Rust 改动，必须全套验证）"
  ],
  "evidence": {},
  "notes": "设计 docs/design/2026-05-30-s1-2-port-horizontal-coupling.md；计划 docs/superpowers/plans/2026-05-30-s1-2-portification.md。本计划详列 S1-2a 走路骨架；S1-2b(Recon,22条) / c(Vuln) / d(PentestPlan) / e(AgentLog) / f(scan_queue 映射清理) / g(asset_intel 模块解耦) 各自另写计划。scan_queue 第 24 条 allowlist 是领域映射伪阳性，需用户确认归属。"
}
```

---

## 路线图（S1-2b–g，各自另写计划，复用 S1-2a 模式）

| 切片 | 端口 / 动作 | allowlist 净减 | 关键文件 | 备注 |
|---|---|---|---|---|
| **S1-2b** | `ReconReadPort`（recon） | 22 | `ports/recon/*` + 消费方 `ai/db_bridge/recon.rs`、`tools/security_analysis.rs`、`tools/pentest_bridge/{auth_probe,js_collect/*,js_extract_apis,record_finding}.rs`、`tools/pipeline/storage.rs`、`tools/audit.rs`、`tools/vuln_intel/commands/matching.rs` | 最大；**按消费方子切**（每子片独立 `just arch` 绿）；recon repo：targets/target_assets/api_endpoints/js_analysis/fingerprints/passive_scans/sitemap_store/directory_entries |
| **S1-2c** | `VulnReadPort`(+wiki)（vuln） | 2 | `ports/vuln/*` + `ai/db_bridge/recon.rs`(vuln_intel)、`ai/db_bridge/wiki.rs`(wiki_kb) | 注意是给 agent 适配器内部用 |
| **S1-2d** | `PentestPlanReadPort`（pentest） | 1 | `ports/pentest/*` + `ai/db_bridge/orchestration.rs`(execution_plans) | 单条，小 |
| **S1-2e** | `AgentLogReadPort`（agent） | 2 | `ports/agent/*` + `tools/audit.rs`(agent_logs/search_logs) | audit 聚合读 |
| **S1-2f** | scan_queue 映射清理（非端口） | 1 | `scripts/check_repo_ownership.py`（改 REPO_OWNER 或 DOMAIN_RULES 一行）+ 删第 24 条 | 需用户确认 scan_queue 属 recon 还是 vuln（设计 §3.3） |
| **S1-2g** | `asset_intel` 模块 import 解耦（另一条轴） | 0 | `tools/asset_intel/mod.rs:27,30` | 编译期 `use crate::tools::{organizations,pentest}`，守卫抓不到；可并入 S3 god-crate 拆分 |

**S1-2 完成判据**：`ALLOWLIST` 30 → ≤1（scan_queue 处理后清零），`just arch` 常绿，全程零业务语义变更，每片有 `just precommit` 证据。

---

## 自检（writing-plans 收尾）

**1. 规格覆盖度（对照设计 §3-§6）：** §3 清单 29 真耦合 + 1 伪阳性 → 5 端口 + 1 清理 + 1 另轴，全部在路线图有切片；S1-2a 四个端口方法逐条对应设计 §5.1 表；守卫机制对应设计 §4.3。覆盖。

**2. 占位符扫描：** 无「TODO/待定/后续实现」占位。S1-2a 全部给真实代码（trait/adapter/迁移/注入/守卫 diff/验证命令）。S1-2b–g 是**显式延后**的独立切片（writing-plans「范围检查」要求多子系统拆分），非占位符——每片到时另写 code-complete 计划。

**3. 类型/标识一致性：** `VaultReadPort`/`PgVaultAdapter`/`vault`(字段) 在 Task 1 定义，Task 2/3 用 `self.vault.<method>`、Task 4 用 `crate::ports::platform::{VaultReadPort,PgVaultAdapter}`，方法名（`list_name_meta_by_project`/`get_secret_by_name_project`/`get_value_by_name_project`/`store_entry`）全程一致；端口方法签名与 `golish-db/src/repo/vault.rs` 对应函数入参/出参一致（`insert_full` 的 tags/source_url 默认值在 1.3 注明）。

**4. 风险/回滚：** 端口=新增文件 + 删 allowlist + 改少量消费方；Task 2/3 中间态会编译 RED（构造点未改），Task 4 闭合——已在 Task 2 验证段标注「连续提交后统一验证」。单 PR revert 即恢复直查。

**验证命令汇总：**
```bash
cd backend && cargo check -p golish            # Task 1/4 编译
cargo nextest run -p golish                     # 端口契约 + 不回归
grep -n "golish_db::repo::vault" backend/crates/golish/src/tools/pentest_bridge/{vault_ops,auth_probe}.rs  # 应空
python3 scripts/check_repo_ownership.py         # 守卫 OK clean
just arch                                        # 双守卫绿
just precommit                                   # 完成定义全套门禁
```
