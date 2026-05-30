# P0-3b 全量作用域 SQL 下沉 golish-db repo（IDOR 残余 · 选项 A 全拆）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans/` 逐任务实现此计划；每个任务单独 commit（本批工作树已有大量未提交改动，commit 拆分阶段统一处理，执行期以「批量改完→统一编译→批量修错」为节奏）。
> Relates to: `docs/design/2026-05-29-architecture-optimization.md` §3.1 B-D2 / §5 P0-3；`docs/superpowers/plans/2026-05-29-p0-scoped-sql-to-repo.md`（P0-3 母计划，已覆盖 vault/findings/methodology/pipeline-commands）；AGENTS.md 不变量 I2（IDOR）。
> 日期：2026-05-30 ｜ 状态：In progress ｜ 负责会话：bajie-mcp-agent-5

**目标：** 把 P0-3 母计划范围之外、仍散落在 `golish/src/tools/` 的项目作用域裸 SQL 全部下沉到 `golish-db` repo 层，命令层只调 repo（+ 需要 IDOR 收口处配 `scoping::ensure_scoped_*`）。

**架构：** 沿用既有三件套——① 泛型 `repo/scoped.rs`（`get_scoped`/`delete_scoped`/`list_by_project` 等，谓词 `IS NOT DISTINCT FROM`）；② 各表 repo 模块（薄封装 + 自定义 SQL fn）；③ 命令层 `tools/scoping.rs` 守卫。**关键约束：行为零漂移**——尤其 `targets` 域用的是 legacy 可见性谓词 `($n IS NULL OR project_path = $n OR project_path = '')`（含历史全局行 `project_path = ''`），**不能**换成通用 `IS NOT DISTINCT FROM`，必须在 `repo/targets.rs` 新增 targets 专用 scoped fn 原样保留该谓词。

**技术栈：** Rust、`sqlx`（Postgres）、`cargo nextest`、`golish-db`、`golish` Tauri 命令层。

---

## 现状（审计事实，带证据）

母计划 P0-3 已清零 `tools/{vault,findings/crud,methodology,pipeline/commands}.rs`。本计划处理**其余残余**，按"是否已有对应 repo"分三层：

### Tier 1 · 真 id-keyed scoped 改写（repo 已存在，legacy 谓词）
- `tools/targets/cmds.rs`：
  - `target_update` 所有权守卫：`SELECT id FROM targets WHERE id=$1 AND ($2 IS NULL OR project_path=$2 OR project_path='')`（L198-201）
  - `target_delete`：`DELETE FROM targets WHERE id=$1 AND ($2 IS NULL OR project_path=$2 OR project_path='')`（L319-322）
  - `target_update_status`：`UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2 AND ($3 IS NULL OR project_path=$3 OR project_path='')`（L355-356）

### Tier 2 · project 作用域 list/lookup（repo 已存在）
- `tools/targets/cmds.rs`：`target_list`（L20-28，legacy 谓词）、value-list（L136 `SELECT value FROM targets WHERE project_path=$1`）、`target_clear_all`（L337 `DELETE FROM targets WHERE project_path=$1`）
- `tools/targets/db.rs`：find-by-value（L46，legacy 谓词）、list（L93 `WHERE project_path=$1`）
- `tools/vuln_intel/commands/matching.rs`：`SELECT name, tags FROM targets WHERE ($1 IS NULL OR project_path=$1 OR project_path='')`（L17-19）
- `tools/pentest_bridge/{record_finding,js_extract_apis,js_collect}.rs`：target 反查（value→id，legacy NULL/'' 谓词）
- `tools/intel_providers.rs`：`SELECT id FROM organizations WHERE project_path=$1 AND name=$2 AND parent_id IS NULL LIMIT 1`（L315-320）→ `repo/organizations.rs`
- `tools/pentest_bridge/vault_ops.rs`：vault list（L168）+ get-by-name（L203）→ `repo/vault.rs`
- `tools/audit.rs`（6 处）→ `repo/audit.rs`（已存在）

### Tier 3 · project 作用域 ops（❗无 repo，新建模块）
- `scan_queue`（`tools/scan_queue.rs`：list L46 / clear-all L116 / delete-by-url L155 / clear-completed L169）→ **新建 `repo/scan_queue.rs`**
- `sensitive_scan_results` + `sensitive_scan_history`（`tools/sensitive_scan.rs`：load_sitemap L58 / list L224,233,296 / clear L249,253 / stats）→ **新建 `repo/sensitive_scan.rs`**
- `conversations` + `workspace_preferences`（`tools/conversation_store/{mod,batch}.rs`：list L136 / prefs L480 / batch 删 L62,72 动态 SQL）→ **新建 `repo/conversation_store.rs`**
- `directory_entries`（`tools/targets/directory.rs` L64；`tools/pipeline/storage.rs` L186 EXISTS）→ **新建 `repo/directory_entries.rs`**
- `sitemap_store`（`tools/pipeline/storage.rs` L278 读 / L350 删；`tools/sensitive_scan.rs` L58 读）→ **新建 `repo/sitemap_store.rs`**
- `custom_rules`（`tools/custom_rules.rs` 2 处）→ **新建 `repo/custom_rules.rs`**
- `targets` EXISTS 检查（`tools/pipeline/storage.rs` L43 `SELECT EXISTS(... targets WHERE value=$1 AND project_path=$2)`）→ `repo/targets.rs`（exact 谓词，新增 `exists_by_value_exact`）

> **执行前每个 Tier 3 文件都要先 `Read` 全文**确认 row 类型、签名、上下文，再动手（审计只取了行号证据）。

---

## 文件结构（创建 / 修改 + 职责）

| 文件 | 动作 | 职责 |
|---|---|---|
| `backend/crates/golish-db/src/repo/targets.rs` | 修改 | 加 legacy-scoped fn：`get_id_scoped_legacy` / `delete_scoped_legacy` / `update_status_scoped_legacy` / `list_legacy` / `list_values` / `clear_project` / `find_by_value_legacy` / `exists_by_value_exact` / `match_rows_legacy`（均原样保留 `($n IS NULL OR project_path=$n OR project_path='')` 谓词） |
| `backend/crates/golish-db/src/repo/organizations.rs` | 修改 | 加 `find_root_id_by_name(project_path, name)` |
| `backend/crates/golish-db/src/repo/vault.rs` | 修改 | 加 `list_meta_by_project` / `get_secret_by_name_scoped` |
| `backend/crates/golish-db/src/repo/audit.rs` | 修改 | 把 tools/audit.rs 的 6 处下沉为 repo fn |
| `backend/crates/golish-db/src/repo/scan_queue.rs` | **新建** | scan_queue 的 list/clear/delete-by-url/clear-completed |
| `backend/crates/golish-db/src/repo/sensitive_scan.rs` | **新建** | sensitive_scan_results/history 的 list/clear/stats + sitemap dirs 读 |
| `backend/crates/golish-db/src/repo/conversation_store.rs` | **新建** | conversations list/batch-delete + workspace_preferences 读写 |
| `backend/crates/golish-db/src/repo/directory_entries.rs` | **新建** | directory_entries list/exists |
| `backend/crates/golish-db/src/repo/sitemap_store.rs` | **新建** | sitemap_store 读/删（by name+project） |
| `backend/crates/golish-db/src/repo/custom_rules.rs` | **新建** | custom_rules 的作用域 CRUD |
| `backend/crates/golish-db/src/repo/mod.rs` | 修改 | `pub mod` 注册 6 个新模块 |
| `backend/crates/golish/src/tools/targets/{cmds,db,directory}.rs` | 修改 | 删裸 SQL → 调 repo + `ensure_scoped_*` |
| `backend/crates/golish/src/tools/{scan_queue,sensitive_scan,custom_rules,intel_providers}.rs` | 修改 | 同上 |
| `backend/crates/golish/src/tools/conversation_store/{mod,batch}.rs` | 修改 | 同上 |
| `backend/crates/golish/src/tools/pipeline/storage.rs` | 修改 | 同上 |
| `backend/crates/golish/src/tools/pentest_bridge/{record_finding,js_extract_apis,js_collect,vault_ops}.rs` | 修改 | 同上 |
| `backend/crates/golish/src/tools/vuln_intel/commands/matching.rs` | 修改 | 同上 |

> **DRY / YAGNI**：行类型（`ScanQueueRow` / `SensitiveScanRow` / `DirEntryRow` 等）当前定义在 `tools/`。优先用 `golish-db` 的**泛型** repo fn（`T: FromRow` 由命令层提供，无需把 row 类型搬进 golish-db）；仅当某 fn 必须返回结构化行且泛型不便时，才把 row 类型迁到 `golish-db/src/models`。

---

## 任务分解（小步骤，TDD；行为零漂移）

> **零漂移测试范式**：与 `repo/scoped.rs` 一致——给每个**自定义 SQL** 的 repo fn 抽一个纯 `build_*_sql()` 函数，加 `#[cfg(test)]` 断言其字符串 == 迁移前原始 SQL（无需活 DB）。复用 `scoped::*` 的 fn 无需新增测试（SQL 已在 scoped.rs 测过）。

### 任务 T1 · targets legacy-scoped repo fn（Tier 1，repo 侧先行）
- **文件：** `backend/crates/golish-db/src/repo/targets.rs`
- **步骤：** 新增（保留 legacy 谓词，原样字符串）：
```rust
const TARGET_SCOPE: &str = "($2 IS NULL OR project_path = $2 OR project_path = '')";

/// id 所有权守卫（legacy 可见性）。None == 不存在或跨项目。
pub async fn get_id_scoped_legacy(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<Option<Uuid>> {
    let row = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM targets WHERE id = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '')",
    ).bind(id).bind(project_path).fetch_optional(pool).await?;
    Ok(row)
}

/// 删除（legacy 可见性），返回 rows_affected。
pub async fn delete_scoped_legacy(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(
        "DELETE FROM targets WHERE id = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '')",
    ).bind(id).bind(project_path).execute(pool).await?;
    Ok(res.rows_affected())
}

/// 改状态（legacy 可见性），返回 rows_affected。
pub async fn update_status_scoped_legacy(pool: &PgPool, id: Uuid, status: &str, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE targets SET status = $1::target_status, updated_at = NOW() WHERE id = $2 AND ($3 IS NULL OR project_path = $3 OR project_path = '')",
    ).bind(status).bind(id).bind(project_path).execute(pool).await?;
    Ok(res.rows_affected())
}
```
- **验证：** `cd backend && cargo build -p golish-db`（编译过）。
- **提交：** `feat(db): add targets legacy-scoped repo methods (P0-3b T1)`

### 任务 T2 · 命令层 targets 改调（Tier 1）
- **文件：** `backend/crates/golish/src/tools/targets/cmds.rs`
- **步骤：**
  - `target_update` 守卫：`let owned = scoping::ensure_scoped_found(golish_db::repo::targets::get_id_scoped_legacy(pool, uid, project_path.as_deref()).await?)?;`（删 L198-203 裸 SELECT）
  - `target_delete`：`scoping::ensure_scoped_mutation(golish_db::repo::targets::delete_scoped_legacy(pool, uid, project_path.as_deref()).await?)?;`（删 L319-324）。⚠️ 原 `target_delete` 是否对零行报错需对照原行为——若原本静默成功，则**不**加 `ensure_scoped_mutation`（保行为）；若计划收口为 NotFound 需在提交说明里写明这是有意的行为强化。
  - `target_update_status`：同 delete，配 `ensure_scoped_mutation`（对照原 `res.rows_affected()` 处理逻辑保持一致）。
- **验证：** `cargo build -p golish`；`rg "FROM targets WHERE id" backend/crates/golish/src/tools/targets/cmds.rs` 归零。
- **提交：** `refactor(targets): route scoped id mutations through golish-db repo (P0-3b T2)`

### 任务 T3 · targets 其余 list/lookup（Tier 2）
- **文件：** `repo/targets.rs` + `tools/targets/{cmds,db}.rs` + `tools/vuln_intel/commands/matching.rs`
- **步骤：** repo 加 `list_legacy`（`SELECT <cols> ... WHERE ($1 IS NULL OR project_path=$1 OR project_path='') ORDER BY created_at`，返回 `Vec<Target>`）、`list_values(project_path)`、`clear_project(project_path)`、`find_by_value_legacy(value, project_path)`、`exists_by_value_exact(value, project_path)`、`match_rows_legacy(project_path)→Vec<(String, Value)>`；命令层逐个改调。
- **验证：** `cargo build -p golish`；`rg "FROM targets" backend/crates/golish/src/tools/targets/cmds.rs backend/crates/golish/src/tools/targets/db.rs backend/crates/golish/src/tools/vuln_intel/commands/matching.rs` 仅剩 repo 调用、无裸 SQL。
- **提交：** `refactor(targets): sink list/value/clear/lookup SQL into repo (P0-3b T3)`

### 任务 T4 · organizations + vault + audit（Tier 2，repo 已存在）
- **文件：** `repo/{organizations,vault,audit}.rs` + `tools/{intel_providers, pentest_bridge/vault_ops, audit}.rs` + `tools/pentest_bridge/{record_finding,js_extract_apis,js_collect}.rs`（target 反查用 T3 的 `find_by_value_legacy`）
- **步骤：** 各 repo 加对应 fn（`organizations::find_root_id_by_name`、`vault::list_meta_by_project` / `get_secret_by_name_scoped`、`audit::*`），命令层改调。每个自定义 SQL fn 配 `build_*_sql` 零漂移单测。
- **验证：** `cargo build -p golish`；相关文件 `rg "project_path" ` 仅剩 repo 调用。
- **提交：** 每文件域一个 `refactor(<area>): sink scoped SQL into golish-db repo (P0-3b T4.x)`

### 任务 T5 · 新建 6 个 Tier 3 repo 模块 + 命令层改调
- **文件：** 新建 `repo/{scan_queue,sensitive_scan,conversation_store,directory_entries,sitemap_store,custom_rules}.rs` + `repo/mod.rs` 注册 + 对应 `tools/` 命令层改调。
- **步骤（每个表一个子任务，统一范式）：**
  1. `Read` 对应 `tools/` 文件全文，抄出原始 SQL 与 row 类型 / 绑定顺序。
  2. 新建 `repo/<table>.rs`：为每条 SQL 写一个 fn（泛型 `T: FromRow` 优先；返回 `Vec<T>` / `Option<T>` / `u64`）；自定义 SQL 抽 `build_*_sql` + 零漂移单测。
  3. `repo/mod.rs` 加 `pub mod <table>;`。
  4. 命令层删裸 SQL → 调 repo（保持原 row 类型在命令层定义）。
- **验证：** `cargo build -p golish-db && cargo build -p golish`；逐文件 `rg "project_path" ` 仅剩 repo 调用。
- **提交：** 每表一个 `feat(db)+refactor: sink <table> SQL into repo (P0-3b T5.x)`

### 任务 T6 · 全局兜底 + 验证证据
- **步骤：**
  1. `rg -n "project_path (IS NOT DISTINCT FROM|= \$|IS NULL OR project_path)" backend/crates/golish/src/tools` → 仅剩注释/无命中（命令层裸作用域 SQL 清零）。
  2. `cd backend && cargo nextest run -p golish-db`（含新 `build_*_sql` 零漂移单测）→ 全绿。
  3. `cargo nextest run -p golish --lib`（受影响命令模块）→ 无回归。
  4. 把命令、退出码、关键输出记入 `agent-progress.md`「已记录证据」。
- **提交：** `chore(idor): residual scoped SQL fully sunk into repo (P0-3b T6)`（如有 fmt 改动）

---

## 影响面
- **golish-db**：`repo/{targets,organizations,vault,audit}.rs` 增 fn；新建 6 个 repo 模块；`repo/mod.rs` +6 行。
- **golish 命令层**：~16 个 `tools/` 文件删裸 SQL、改调 repo。
- **不影响**：命令名/签名/前端、DB schema、业务语义（含 targets legacy 可见性原样保留）。
- **安全收益**：IDOR / 作用域守卫从命令层各自裸 SQL 收敛到 repo 唯一边界。

## 验证
| 命令 | 预期 |
|---|---|
| `cargo nextest run -p golish-db` | 新 `build_*_sql` 零漂移单测 + 既有测试全绿 |
| `cargo build -p golish` | 命令层改调后编译通过 |
| `rg -n "project_path (IS NOT DISTINCT FROM\|= \$\|IS NULL OR project_path)" backend/crates/golish/src/tools` | 仅注释/无命中 |
| `just precommit` | 合并前门禁（注：按用户口径，clippy/sandbox baseline 不绿当 bug 收尾再处理） |

**实施约定（工程效率，设计文档 §7.1）：** 一个 Tier（或一个表）的 repo fn + 命令层改动**全部改完**后，**只**统一跑一次 `cargo build`，集中批量修错，再统一编译验证；不要每改一处就编译。

## 回滚
- repo 新增 fn / 新模块均为**纯增量**；命令层逐文件/逐表切换，未切换的保持原裸 SQL（行为不变）。
- 每个 Tier / 表一个 commit，可独立 revert。

## 风险
| 风险 | 缓解 |
|---|---|
| targets legacy 谓词被误换成 `IS NOT DISTINCT FROM` 改变可见性 | targets 专用 fn 原样保留 `($n IS NULL OR project_path=$n OR project_path='')`；T1 字符串与原 SQL 逐字对照 |
| `target_delete`/`update_status` 是否收口为 NotFound 改变行为 | 先读原命令对零行的处理；保持原行为，除非有意强化并在 commit 说明 |
| Tier 3 row 类型在命令层、repo 泛型推断失败 | 优先泛型 `T: FromRow` 由命令层提供；不行才迁 row 到 golish-db/models |
| 动态 SQL 批删（conversation_store/batch）难泛化 | 该 fn 接收 `surviving_ids: &[Uuid]`，在 repo 内构造占位符（与原逻辑一致），加 `build_*_sql` 对 0 个/ N 个 id 两形状单测 |
| 与工作树其它未提交改动冲突 | 本计划只碰 repo + 上述 tools 文件；小步提交 |

## 自检
- **规格覆盖**：审计三层逐文件映射到 T1-T5；全局兜底 T6。✓
- **占位符扫描**：Tier 3 各表标注「执行前 Read 全文抄原始 SQL」，非 TODO（SQL 原文在源文件、行号已给）；范式统一给出。✓
- **类型一致性**：repo fn 命名 `*_scoped_legacy`（legacy 谓词）/ `*_scoped`（IS NOT DISTINCT FROM）区分；命令层统一配 `ensure_scoped_mutation`(u64) / `ensure_scoped_found`(Option)。✓
