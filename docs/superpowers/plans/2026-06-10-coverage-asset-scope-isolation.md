# Coverage 资产盘按 organization 隔离 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
>
> 设计文档：`docs/design/2026-06-09-coverage-asset-scope-isolation.md`

**目标：** coverage gate 的资产分母按当前 operation 绑定的 `organization_id` 过滤,跨 org/跨 run 的持久 PG 数据不再互相污染 coverage(分母爆炸 → target_intel 无限 BLOCK)。

**架构：** 自底向上加一条 `org_id: Option<Uuid>` 透传链:DB SQL(`$2 IS NULL OR organization_id = $2`)→ ReconTargetsPort → db_traits `in_scope_assets(org_id)` → TaskOrchestrator 新增 `harness_org_id` 字段(照抄 `profile_override` 模式)→ headless `stage_run` 把 `seed.org_id` 灌入。`None` 一律退回旧全局行为(向后兼容,GUI/chat 路径不回归)。

**技术栈：** Rust workspace · sqlx(运行时绑定,非宏)· async_trait

**§9 开放问题拍板记录(实读 2026-06-10)：**
- §9-1 org_id 链路:**无现成透传链路**。headless 的 `seed_upstream` 返回 `SeedResult.org_id`(`Option<Uuid>`),在 `run()` L229 调 `orchestrate()` 处可达;orchestrator 透传照抄 `set_profile_override` 模式。GUI/chat(`chat.rs` L167)暂无 org 来源 → 不 set → None → 全局(P1 再接)。
- §9-2 缺省策略:None→全局(设计推荐,向后兼容)。
- §9-3 历史污染清理:不入产品,跳过。

**改动文件清单：**

| # | 文件 | 职责 |
|---|---|---|
| 1 | `backend/crates/golish-db/src/repo/targets.rs` | SQL builder + `list_in_scope_values` +org_id;SQL 单测 |
| 2 | `backend/crates/golish-app-core/src/ports/recon/targets.rs` | `ReconTargetsPort::in_scope_values` +org_id + adapter 透传 |
| 3 | `backend/crates/golish-agent-kit/src/db_traits/repo.rs` | trait `in_scope_assets(org_id)` |
| 4 | `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs` | impl 透传 |
| 5 | `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs` | `in_scope_assets_impl(org_id)` |
| 6 | `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs` | `harness_org_id` 字段 + setter |
| 7 | `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 2 个调用点传 `self.harness_org_id` |
| 8 | `backend/crates/golish/src/stage_run/mod.rs` | `orchestrate` +org_id 参数;seed 灌入 |

非目标:不改 `coverage_complete` 引擎;不改 GateContext;不动 GUI;不加清库命令。

---

## Task 1 · DB 层:SQL + `list_in_scope_values` 加 org 过滤(TDD)

**文件:** `backend/crates/golish-db/src/repo/targets.rs`

**步骤 1.1 — 写失败的 SQL 单测**(加到文件尾部 `mod tests` 内,紧跟现有 `legacy_list_and_lookup_sql_preserve_projection_and_predicate`):

```rust
#[test]
fn list_in_scope_values_sql_filters_scope_project_and_org() {
    assert_eq!(
        build_list_in_scope_values_legacy_sql(),
        "SELECT DISTINCT value FROM targets \
           WHERE scope::text = 'in' \
             AND ($1 IS NULL OR project_path = $1 OR project_path = '') \
             AND ($2 IS NULL OR organization_id = $2) \
           ORDER BY value"
    );
}
```

**步骤 1.2 — 跑测试确认红:**

```bash
cd backend && cargo nextest run -p golish-db list_in_scope_values_sql_filters
```
预期:FAIL(SQL 还没有 `$2` 谓词)。

**步骤 1.3 — 改 SQL builder(L148)+ 函数签名(L304):**

```rust
fn build_list_in_scope_values_legacy_sql() -> String {
    "SELECT DISTINCT value FROM targets \
       WHERE scope::text = 'in' \
         AND ($1 IS NULL OR project_path = $1 OR project_path = '') \
         AND ($2 IS NULL OR organization_id = $2) \
       ORDER BY value"
        .to_string()
}
```

```rust
/// project_path = all visible targets (single-workspace default). `org_id`
/// narrows the set to one organization's in-scope targets (coverage asset-axis
/// isolation, design 2026-06-09); `None` keeps the legacy whole-DB behaviour.
pub async fn list_in_scope_values(
    pool: &PgPool,
    project_path: Option<&str>,
    org_id: Option<Uuid>,
) -> Result<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(&build_list_in_scope_values_legacy_sql())
        .bind(project_path)
        .bind(org_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}
```

**步骤 1.4 — 跑测试确认绿 + 该 crate 编译:**

```bash
cd backend && cargo nextest run -p golish-db list_in_scope_values_sql_filters && cargo check -p golish-db
```
预期:测试 PASS;`cargo check -p golish-db` Exit 0(调用方 golish-app-core 还没改,下个任务处理——workspace check 此时会红,属预期中间态)。

## Task 2 · 端口层:`ReconTargetsPort::in_scope_values` +org_id

**文件:** `backend/crates/golish-app-core/src/ports/recon/targets.rs`

**步骤 2.1 — trait 方法(L76)加参数:**

```rust
    /// `None` project_path = all visible targets (single-workspace default).
    /// `org_id` narrows to one organization (coverage asset-axis isolation);
    /// `None` = legacy whole-DB set.
    async fn in_scope_values(
        &self,
        project_path: Option<&str>,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>>;
```

**步骤 2.2 — `PgReconTargetsAdapter` impl(L295)透传:**

```rust
    async fn in_scope_values(
        &self,
        project_path: Option<&str>,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(golish_db::repo::targets::list_in_scope_values(
            self.pool.as_ref(),
            project_path,
            org_id,
        )
        .await?)
    }
```

(文件已 `use uuid::Uuid`——`target_add` 签名用到;若无则补 import。)

**步骤 2.3 — 验证:**

```bash
cd backend && cargo check -p golish-app-core && cargo nextest run -p golish-app-core ports::recon
```
预期:Exit 0;object-safety 测试全 PASS。

## Task 3 · trait + impl:`in_scope_assets(org_id)` 透传

**文件:** `backend/crates/golish-agent-kit/src/db_traits/repo.rs`(L151)、`backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`(L287)、`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`(L248)

**步骤 3.1 — db_traits trait 默认方法加参数:**

```rust
    async fn in_scope_assets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        let _ = org_id;
        Ok(Vec::new())
    }
```
(doc comment 保留并补一句:`org_id` narrows the axis to the operation's organization;`None` = legacy whole-DB set。文件已 `use uuid::Uuid`。)

**步骤 3.2 — db_bridge impl 透传(mod.rs):**

```rust
    async fn in_scope_assets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        self.in_scope_assets_impl(org_id).await
    }
```
(mod.rs 顶部确认 `use uuid::Uuid;` 已在——operation_state 等方法已用;无则补。)

**步骤 3.3 — recon.rs impl:**

```rust
    pub(super) async fn in_scope_assets_impl(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        // `None` project_path = legacy "all visible" set (chat sessions carry
        // project_path=None). `org_id` narrows the asset axis to the current
        // operation's organization (coverage isolation, design 2026-06-09).
        self.recon_targets.in_scope_values(None, org_id).await
    }
```

**步骤 3.4 — 验证(此时 agent-kit 的调用点还没改,会红——先只查这两个 crate 能否单独过):**

```bash
cd backend && cargo check -p golish-agent-app 2>&1 | head -30
```
预期:报错只来自 `execute.rs` 调用点参数不匹配(golish-agent-kit),`golish-agent-app` 自身 impl 签名与 trait 一致。Task 4 修掉。

## Task 4 · orchestrator:`harness_org_id` 字段 + 2 个调用点

**文件:** `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs`、`.../subtask_phases/execute.rs`

**步骤 4.1 — orchestrator.rs 加字段(紧跟 `profile_override` L48 后):**

```rust
    /// Current operation's organization id (coverage asset-axis isolation,
    /// design 2026-06-09). Injected into `in_scope_assets` lookups so the
    /// coverage gate's denominator only contains THIS org's in-scope targets.
    /// `None` = legacy whole-DB axis (GUI/chat path until org wiring lands).
    pub(super) harness_org_id: Option<uuid::Uuid>,
```

构造函数(L89 附近 `profile_override: None,` 处)加:

```rust
            harness_org_id: None,
```

setter(紧跟 `set_profile_override` L114 后):

```rust
    /// Bind the current operation's organization (coverage asset-axis
    /// isolation). `None` keeps the legacy whole-DB asset axis.
    pub fn set_harness_org_id(&mut self, org_id: Option<uuid::Uuid>) {
        self.harness_org_id = org_id;
    }
```

**步骤 4.2 — execute.rs 两个调用点传参:**

L126(prompt 渲染——保持资产视图与 gate 分母一致):

```rust
                        let in_scope_assets = self
                            .repo
                            .in_scope_assets(self.harness_org_id)
                            .await
                            .unwrap_or_default();
```

L1139(`fetch_in_scope_assets_for_gate` 内):

```rust
        match self.repo.in_scope_assets(self.harness_org_id).await {
```

**步骤 4.3 — 验证:**

```bash
cd backend && cargo check -p golish-agent-kit -p golish-agent-app && cargo nextest run -p golish-agent-kit --status-level fail
```
预期:Exit 0;agent-kit 全部测试 PASS(mock 都走 trait default,签名同步后无需逐个改)。

## Task 5 · headless 接线:stage_run 把 seed.org_id 灌入

**文件:** `backend/crates/golish/src/stage_run/mod.rs`

**步骤 5.1 — `orchestrate()` 签名(L296)+1 参数:**

```rust
async fn orchestrate(
    bridge: &Arc<AgentBridge>,
    db_pool: &Arc<sqlx::PgPool>,
    session_id: &str,
    profile_id: &str,
    entry_stage: StageKind,
    allowlist: HashSet<StageKind>,
    task_input: &str,
    org_id: Option<uuid::Uuid>,
) -> Result<String> {
```

函数体内(L333 `set_stage_allowlist` 后):

```rust
    orchestrator.set_harness_org_id(org_id);
```

**步骤 5.2 — 调用处(L230)传 seed 的 org_id:**

```rust
    let result = orchestrate(
        &bridge,
        &db_pool,
        &session_id,
        &profile_id,
        entry_stage,
        allowlist,
        &task_input,
        seed.as_ref().and_then(|s| s.org_id),
    )
    .await;
```

**步骤 5.3 — 同步更新 stage_run 注释**(L128 提到 `in_scope_assets(None) sees any in-scope target regardless of project_path` 的旧语义注释改为 org-scoped):

```rust
    // P1 · seed minimal upstream (org + in-scope targets) so an isolated
    // downstream stage (e.g. --only target_intel) has real data to work on.
    // Scoped to the workspace project_path the agent's manage_targets /
    // manage_organizations tools use; the seeded org id is then bound to the
    // orchestrator so the gate's in_scope_assets(org_id) only sees THIS org's
    // targets (coverage asset-axis isolation, design 2026-06-09).
```

**步骤 5.4 — 验证:**

```bash
cd backend && cargo check -p golish
```
预期:Exit 0。

## Task 6 · 全量收口

**步骤 6.1 — fmt + clippy + 全测:**

```bash
cd backend && cargo fmt && cargo clippy -p golish-db -p golish-app-core -p golish-agent-kit -p golish-agent-app -p golish --lib -- -D warnings && cargo nextest run --status-level fail
```
预期:全绿零 warning。

**步骤 6.2 — `just precommit`(含前端,确认无 ts-rs 影响——本改动纯后端内部查询参数,不触跨 IPC 类型):**

```bash
just precommit
```
预期:全绿。

**步骤 6.3 — 更新 `feature_list.json` evidence + `agent-progress.md` 会话记录。**

## 验收对照(设计 §7 DoD)

- [x] 单测:SQL builder 含 org 谓词(Task 1;repo 层无 live-PG 测试基建,SQL 字符串断言 = 现有测试模式)
- [x] 向后兼容:org_id=None 走 `$2 IS NULL` 短路 = 旧全局行为;GUI/chat 不 set = None(回归零)
- [x] 注入空集防御:`fetch_in_scope_assets_for_gate` 现有"非空才注入"守卫不动——org 下无资产 → 回退自报集
- [ ] ⏳ 活体验证(可选,需用户跑):clean DB 后 `--stage-run --org vulnweb --target testhtml5.vulnweb.com`,trace 确认 `asset_count` 只含本 org 资产
