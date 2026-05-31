# M5 —— 抽 `golish-platform-app`（platform 服务，crate-per-service 最后一域）

> 父计划：`docs/superpowers/plans/2026-05-30-crate-per-service-split.md` §M5。
> 范式同 M2/M3（层次 A 编译期依赖链）。**结论先行：platform 是全部里程碑中最干净的一域**——4 个命令文件全部只 take `State<DbState>`，跨服务读全走 `golish_db::repo::`（L2 仓储层，已在 ALLOWLIST），**无任何兄弟 app crate 依赖** → 纯叶子 app crate（L5.5）。

---

## 1. 实证调查（2026-05-31 · grep/read 证据）

### 1.1 范围（4 个单文件，共 792 行）
- `tools/vault.rs`(275) · `tools/audit.rs`(244) · `tools/notes.rs`(101) · `tools/recordings.rs`(172)
- `tools/project_io.rs` **不在 M5 范围**（项目导入导出，非 platform 服务域，留守 golish）。

### 1.2 State 用法（决定性：全干净）
- 4 文件**全部**只用 `tauri::State<'_, DbState>`，**零 AppState**。与 M2/M3 的干净模块同型 → 层次 A 直接搬。

### 1.3 跨服务读（全经 golish_db::repo，无需兄弟 crate 依赖）
- `audit.rs` 读 `golish_db::repo::{audit, passive_scans, agent_logs, terminal_logs, search_logs}`。其中 `passive_scans`=recon-owned、`agent_logs`/`search_logs`=agent-owned（跨服务，已在 ALLOWLIST）；`terminal_logs`=platform-owned（自有）；`audit`=SHARED。
- `notes.rs` 读 `golish_db::repo::notes`（自有）。
- `vault.rs` 读 `golish_db::repo::vault`（自有）+ `golish_core::vault::{obfuscate/deobfuscate/...}` + `golish_core::time`。
- **关键**：跨服务读经 L2 仓储层，不经 recon-app/agent-app crate → platform-app 无兄弟依赖。

### 1.4 导入重映射（仅 3 类）
- `crate::error::GolishError` → `golish_app_core::GolishError`（4 文件）
- `crate::state::DbState` → `golish_app_core::DbState`（4 文件）
- `crate::tools::scoping::*` → `golish_app_core::scoping::*`（vault.rs/notes.rs）
- ts-rs `export_to` 字符串**不动**（与 agent/vuln/recon 已迁移文件一致，按 crate 根解析）。

### 1.5 ports 已就位
- `ports/platform/*`（VaultReadPort/PgVaultAdapter，S1-2a）已在 M3 下沉 `golish-app-core`，M5 **无需搬 ports**。

---

## 2. 执行步骤（层次 A）

1. **建 crate**：`golish-platform-app/{Cargo.toml, src/lib.rs}`。依赖：golish-app-core(GolishError/DbState/scoping) + golish-core(vault/time) + golish-db(repo) + tauri/sqlx/serde/serde_json/uuid/chrono/ts-rs。层级 L5.5。
2. **接线 backend/Cargo.toml**：members + default-members + workspace.dependencies。`golish/Cargo.toml` 加 `golish-platform-app = { workspace = true }`。
3. **git mv** `tools/{vault,audit,notes,recordings}.rs` → `golish-platform-app/src/{vault,audit,notes,recordings}.rs`；lib.rs 声明 4 模块。
4. **重映射**导入（§1.4）。
5. **facade**：`commands_facade/vault.rs` → `pub use golish_platform_app::vault::*;`；`commands_facade/workspace.rs` 的 audit/notes/recordings 三行 → `golish_platform_app::{audit,notes,recordings}::*`。
6. **golish 清理**：`tools/mod.rs` 删 `pub mod {vault,audit,notes,recordings};`（project_io 保留）。
7. **守卫**：
   - `check_dag.py` LAYER_TABLE 加 `"golish-platform-app": 5.5`。
   - `check_repo_ownership.py`：SOURCE_ROOTS 加 `("golish-platform-app","platform")`；删 4 条死 DOMAIN_RULES（`tools/{vault,audit,notes,recordings}`）；ALLOWLIST 3 键 `tools/audit.rs`→`golish-platform-app/audit.rs`；RAW_SQL 键 `tools/audit.rs`+`tools/recordings.rs`→`golish-platform-app/` 前缀（`tools/project_io.rs` 保留）。

---

## 3. 验证（完成定义）

- `cargo check -p golish-platform-app` + `cargo check -p golish` exit 0
- `cargo nextest run -p golish-platform-app`（含 ts-rs export_bindings）
- `cargo clippy -p golish-platform-app --all-targets -- -D warnings` + `cargo clippy -p golish --lib -- -D warnings`
- `python3 scripts/check_dag.py`（**51 crates**）+ `python3 scripts/check_repo_ownership.py`（OK clean）
- ReadLints 无错误

## 4. 回滚
- 未跑通前 facade 旧路径保留；失败则 `git mv` 搬回 + 删新 crate + 还原守卫。

## 5. 不在本里程碑
- 层次 B（platform→agent `AgentLogReadPort` + recon `passive_scans` 端口切断 audit.rs 出向）留后续端口里程碑。
- `just precommit` 全量 / commit / push 按 AGENTS.md §2.7 等用户授权。
