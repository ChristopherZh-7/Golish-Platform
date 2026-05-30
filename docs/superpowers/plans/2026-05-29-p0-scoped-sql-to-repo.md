# P0-3 作用域 SQL 下沉 `golish-db` repo（I2）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans/` 逐任务实现此计划；先写测试（TDD），每个任务单独 commit。
> Relates to: `docs/design/2026-05-29-architecture-optimization.md` §3.1 B-D2 / §5 P0-3；AGENTS.md 不变量 I2（IDOR）。

**目标：** 把分散在 `golish` 命令层的「项目作用域裸 SQL」全部下沉到 `golish-db` repo 层，命令层只调 repo + `ensure_scoped_mutation`/`ensure_scoped_found`，禁止命令层再写带 `project_path` 的裸 SQL，使 IDOR 守卫收敛到唯一边界（落实 I2）。

**架构：** 以现有 `golish-db/src/repo/notes.rs` 为黄金样板——repo 的 `update`/`delete` 接收 `project_path: Option<&str>`、SQL 带 `WHERE id = $ AND project_path IS NOT DISTINCT FROM $`、返回 `rows_affected: u64`；敏感读用返回 `Option<T>` 的 scoped 查询。命令层调用后用 `ensure_scoped_mutation`/`ensure_scoped_found` 把零行/`None` 映射成 `NotFound`。

**技术栈：** Rust、`sqlx`（Postgres）、`cargo nextest`、`golish-db`、`golish` Tauri 命令层。

---

## 现状（事实，带证据）

- **黄金样板已存在**：`backend/crates/golish-db/src/repo/notes.rs`
  - `update(pool, id, content, color, project_path) -> Result<u64>`（`notes.rs:69-87`，SQL 含 `WHERE id = $3 AND project_path IS NOT DISTINCT FROM $4`，返回 `rows_affected`）
  - `delete(pool, id, project_path) -> Result<u64>`（`notes.rs:91-98`）
  - 命令层 `golish/src/tools/notes.rs:81-84`、`96-97` 调用 repo + `ensure_scoped_mutation`（**正确范式**）。
- **共享守卫已存在**：`backend/crates/golish/src/tools/scoping.rs`
  - `ensure_scoped_mutation(rows_affected: u64) -> Result<(), GolishError>`（`scoping.rs:19-25`）
  - `ensure_scoped_found<T>(row: Option<T>) -> Result<T, GolishError>`（`scoping.rs:31-33`）
- **命令层裸 SQL（违 I2 的分散点，待下沉）**：
  - `golish/src/tools/vault.rs`：`vault_get_value` 裸 `SELECT ... WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2`（`vault.rs:143-152`）；`vault_update` 先裸 `SELECT id ...`（`vault.rs:171-178`）再多条裸 `UPDATE`（`vault.rs:180-187+`）。共约 5 处。
  - `golish/src/tools/findings/crud.rs`：约 7 处裸作用域 SQL。
  - `golish/src/tools/methodology.rs`：约 3 处。
  - `golish/src/tools/pipeline/commands.rs`：约 2 处。
- **对应 repo 文件**：`golish-db/src/repo/{vault,findings,methodology,pipelines}.rs` 当前的 `get/delete/list` **不**按 `project_path` 作用域（见 design §3.1 B-D1），需补 scoped 方法。

---

## 文件结构（创建 / 修改 + 职责）

| 文件 | 动作 | 职责 |
|---|---|---|
| `backend/crates/golish-db/src/repo/vault.rs` | 修改 | 补 `get_value_scoped` / `exists_scoped` / scoped `update_*` / `delete`（带 `project_path`，返回 `u64`/`Option`） |
| `backend/crates/golish-db/src/repo/findings.rs` | 修改 | 补 findings 的 scoped CRUD |
| `backend/crates/golish-db/src/repo/methodology.rs` | 修改 | 补 methodology 的 scoped CRUD |
| `backend/crates/golish-db/src/repo/pipelines.rs` | 修改 | 补 pipelines 的 scoped CRUD |
| `backend/crates/golish-db/tests/scoped_repo.rs` | 新建 | repo 层 IDOR 单测（跨项目 id 应零行/None） |
| `backend/crates/golish/src/tools/vault.rs` | 修改 | 删裸 SQL，改调 repo + `ensure_scoped_*` |
| `backend/crates/golish/src/tools/findings/crud.rs` | 修改 | 同上 |
| `backend/crates/golish/src/tools/methodology.rs` | 修改 | 同上 |
| `backend/crates/golish/src/tools/pipeline/commands.rs` | 修改 | 同上 |

> **DRY / YAGNI**：本计划只下沉**已存在**的裸作用域 SQL（vault/findings/methodology/pipeline），不顺手做 design 中 P1-1 的「泛型 generic CRUD helper」——那是独立 feature。先把 IDOR 边界收敛对，再谈泛型抽象。

---

## 任务分解（小步骤，TDD）

### 任务 1：repo 层先写失败的 IDOR 测试（vault 试点）

- **文件：** `backend/crates/golish-db/tests/scoped_repo.rs`（新建）
- **步骤：** 写一个测试：项目 A 插入一条 vault entry，用项目 B 的 `project_path` 调 scoped `delete` 应返回 `0` 行；用 A 调应返回 `1`。

```rust
// 需要可用的测试 PgPool；沿用 golish-db 既有测试夹具（动手前 rg "async fn test_pool" backend/crates/golish-db 确认夹具名）。
#[tokio::test]
async fn vault_delete_is_project_scoped() {
    let pool = test_pool().await;
    let id = repo::vault::create(/* … project_path = Some("proj-A") … */).await.unwrap().id;

    // 跨项目删除：0 行
    let other = repo::vault::delete(&pool, id, Some("proj-B")).await.unwrap();
    assert_eq!(other, 0, "cross-project delete must affect 0 rows");

    // 同项目删除：1 行
    let mine = repo::vault::delete(&pool, id, Some("proj-A")).await.unwrap();
    assert_eq!(mine, 1);
}
```

- **验证：** `cd backend && cargo test -p golish-db vault_delete_is_project_scoped`；预期**失败/不编译**（scoped `delete` 签名尚不存在）。
- **提交：** `test(db): add failing vault scoped-delete IDOR test`

### 任务 2：vault repo 补 scoped 方法（让测试通过）

- **文件：** `backend/crates/golish-db/src/repo/vault.rs`
- **步骤：** 仿 `notes.rs:91-98` 增 scoped 删除/读取（保留旧 `delete(id)` 一个版本期，避免破坏其它调用方）：

```rust
/// Delete a vault entry scoped to project_path (AGENTS.md I2). Returns rows affected.
pub async fn delete_scoped(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(
        "DELETE FROM vault_entries WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Fetch the encrypted value scoped to project_path. None == missing or other project.
pub async fn get_value_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<String>> {
    let v = sqlx::query_scalar::<_, String>(
        "SELECT value FROM vault_entries WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(v)
}
```

- **验证：** `cd backend && cargo test -p golish-db vault_delete_is_project_scoped`；预期**通过**。
- **提交：** `feat(db): add project-scoped vault repo methods`

### 任务 3：命令层 vault 改调 repo（删裸 SQL）

- **文件：** `backend/crates/golish/src/tools/vault.rs`
- **步骤：** 把 `vault_get_value`（`vault.rs:143-152`）改为：

```rust
let pool = state.pool_ready().await?;
let uid: Uuid = id.parse()?;
let enc = crate::tools::scoping::ensure_scoped_found(
    golish_db::repo::vault::get_value_scoped(pool, uid, project_path.as_deref()).await?,
)?;
Ok(deobfuscate(&enc)?)
```

  并把 `vault_update` 的所有权前置检查（`vault.rs:171-178`）改为调用 `get_value_scoped`/新增 `exists_scoped`，删除命令内的裸 `SELECT id ...`。
- **验证：** `cd backend && cargo check -p golish`；`rg "project_path IS NOT DISTINCT FROM" backend/crates/golish/src/tools/vault.rs` 应**显著减少/归零**。
- **提交：** `refactor(vault): route scoped reads/deletes through golish-db repo`

### 任务 4：findings 重复任务 1-3（测试 → repo → 命令层）

- **文件：** `backend/crates/golish-db/src/repo/findings.rs`、`backend/crates/golish/src/tools/findings/crud.rs`、`backend/crates/golish-db/tests/scoped_repo.rs`
- **步骤：**
  1. 在 `tests/scoped_repo.rs` 增 `findings_update_is_project_scoped` 失败测试（跨项目 update 返回 0 行）。
  2. `repo/findings.rs` 增 `update_scoped` / `delete_scoped` / `get_scoped`（SQL 带 `id = $ AND project_path IS NOT DISTINCT FROM $`，返回 `u64`/`Option`，仿任务 2）。
  3. `findings/crud.rs` 的 7 处裸 SQL 改调 repo + `ensure_scoped_mutation`/`ensure_scoped_found`。
- **验证：** `cargo test -p golish-db findings_update_is_project_scoped` 通过；`cargo check -p golish`；`rg "project_path IS NOT DISTINCT FROM|project_path = \\$" backend/crates/golish/src/tools/findings/crud.rs` 归零。
- **提交：** `refactor(findings): sink scoped SQL into golish-db repo`

### 任务 5：methodology 重复任务 1-3

- **文件：** `backend/crates/golish-db/src/repo/methodology.rs`、`backend/crates/golish/src/tools/methodology.rs`、测试文件
- **步骤：** 同任务 4 的三步（失败测试 → repo scoped 方法 → 命令层改调），覆盖 methodology 的 3 处裸 SQL。
- **验证：** `cargo test -p golish-db methodology_*_is_project_scoped` 通过；`cargo check -p golish`；`rg` 该文件裸作用域 SQL 归零。
- **提交：** `refactor(methodology): sink scoped SQL into golish-db repo`

### 任务 6：pipeline 重复任务 1-3

- **文件：** `backend/crates/golish-db/src/repo/pipelines.rs`、`backend/crates/golish/src/tools/pipeline/commands.rs`、测试文件
- **步骤：** 同上，覆盖 pipeline 的 2 处裸 SQL。
- **验证：** `cargo test -p golish-db pipeline_*_is_project_scoped` 通过；`cargo check -p golish`；`rg` 归零。
- **提交：** `refactor(pipeline): sink scoped SQL into golish-db repo`

### 任务 7：全局兜底 grep + 清理旧未作用域方法

- **文件：** 全 `backend/crates/golish/src/tools/`
- **步骤：**
  1. 全局兜底：`rg -n "project_path (IS NOT DISTINCT FROM|= \\$)" backend/crates/golish/src/tools` 应**只**剩注释/无命中（命令层无裸作用域 SQL）。
  2. 若旧的非作用域 repo 方法（如 `vault::delete(id)`）已无调用方，删除之（先 `rg` 确认零引用）。
- **验证：** 上述 `rg` 命中为空；`just check-rust`（clippy 零 warning + 全后端单测）。
- **提交：** `chore(idor): remove residual bare scoped SQL & dead repo methods`

---

## 影响面

- **golish-db**：`repo/{vault,findings,methodology,pipelines}.rs` 增 scoped 方法；新增 `tests/scoped_repo.rs`。
- **golish 命令层**：`tools/{vault,findings/crud,methodology,pipeline/commands}.rs` 删裸 SQL、改调 repo。
- **不影响**：命令名/签名/前端（行为等价）、DB schema、其它域。
- **安全收益**：IDOR 守卫从「命令层各自裸 SQL」收敛到「repo 唯一边界 + scoping 守卫」，杜绝漏写。

## 验证

| 命令 | 预期 |
|---|---|
| `cargo test -p golish-db <scoped tests>` | 跨项目 id 操作返回 0 行 / `None`，同项目返回 1 行 / `Some` |
| `cargo check -p golish` | 命令层改调 repo 后编译通过 |
| `rg -n "project_path (IS NOT DISTINCT FROM\|= \\$)" backend/crates/golish/src/tools` | 无命中（裸作用域 SQL 清零） |
| `just test-rust` | 后端全单测通过 |
| `just precommit` | 合并前全绿门禁 |

**实施约定（工程效率，来源：用户要求 2026-05-29；见全局记忆 `golish:workflow:backend-build-policy`）：** 采用「**批量改完 → 统一编译 → 批量修错**」节奏——一个域（如 findings 的 7 处）的 repo 方法 + 命令层改动**全部改完**后，**只**统一跑一次 `cargo check`（或 `just check-rust`），集中查看全部错误后**批量**修复，再统一编译验证；**不要每改一处就编译一次**。仅在「全部改完」后才进入编译-修错循环。最终合并以 `just precommit` 全绿为准。

## 回滚

- repo 新增方法为**纯增量**；命令层**逐文件/逐域**切换，未切换的域仍走原裸 SQL（行为不变）。
- 每个域一个 commit，可独立 revert；旧 repo 方法保留到任务 7 确认无引用后再删，回滚安全。

## 风险

| 风险 | 缓解 |
|---|---|
| `project_path` 为 `NULL` 的旧数据语义 | 统一用 `IS NOT DISTINCT FROM`（NULL 安全相等），与现有 `notes.rs`/命令层一致 |
| 多条 UPDATE 的 vault_update 改造遗漏某字段的作用域 | 先用 `exists_scoped` 做一次前置守卫，确认所有权后再执行各 UPDATE（保持原逻辑结构） |
| 测试夹具/可用 PgPool 获取方式不明 | 任务 1 动手前 `rg "async fn test_pool\|#\\[sqlx::test\\]" backend/crates/golish-db` 确认现有夹具，沿用之 |
| 删旧 repo 方法误伤其它调用方 | 任务 7 先 `rg` 全仓库确认零引用再删 |
| 与活跃改动（findings/pipeline 正在迭代）冲突 | 按域小步提交、频繁 rebase；先做不活跃的 vault/methodology |

---

## 自检

- **规格覆盖**：①repo 补方法（任务 2/4/5/6）②命令层改调（任务 3/4/5/6）③删裸 SQL（任务 3-7）④作用域测试（任务 1/4/5/6）——全覆盖 vault/findings/methodology/pipeline。
- **占位符扫描**：测试夹具名、字段细节标注了「动手前 `rg` 确认」，非 TODO；scoped 方法均给出完整代码。
- **类型一致性**：scoped 方法统一签名 `(pool, id, …, project_path: Option<&str>) -> Result<u64 | Option<T>>`；命令层统一配 `ensure_scoped_mutation`（对 `u64`）/ `ensure_scoped_found`（对 `Option`）。
