# Organization deletion retained Tool Truth implementation plan

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让两阶段组织删除物理移除 live organization/Target，同时保留不可变 Tool Truth 与 Investigation 历史。
**架构：** 用两个连续向前 migration 把 retained authority relations 的 live-parent FK 与不可变行的 `SET NULL` live-target alias改成写入时 key-share 验证 trigger；原 hard-delete transaction和 immutable graph均不重写。用 embedded PostgreSQL 行为测试证明删除、保留和新写 fail-closed。
**技术栈：** PostgreSQL PL/pgSQL、sqlx、Rust nextest、embedded PostgreSQL。

## 文件

- `backend/crates/golish-db/tests/cleanup_obligation_kernel.rs`：增加 bound-wave 两阶段删除行为回归。
- `backend/crates/golish-db/migrations/20260811000001_organization_deletion_retained_tool_truth.sql`：安装 live-reference admission trigger并替换 retained relations 的 parent FK。
- `backend/crates/golish-db/migrations/20260811000002_organization_deletion_retained_live_target_aliases.sql`：保留 bound audit 与其它 append-only/CAS 行的 target UUID，避免 `ON DELETE SET NULL` 改写历史。
- `docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-cleanup-app.md`、`docs/modules/INDEX.md`：同步 hard-delete/retention合同。
- `feature_list.json`、`agent-progress.md`：记录状态与新鲜证据。

## Task 1：RED bound-wave deletion

1. 在现有 `frozen_scope` fixture 中创建一个 Target、`stage_asset_waves` header/member、`tool_truth_stage_wave_execution_bindings` 与绑定该 Target audit row 的 evidence production authority。
2. 走 production `request → claim_next_artifact_cleanup → complete_artifact_cleanup → hard_delete`。
3. 断言旧 schema 返回 `tool_truth_bound_wave_source_immutable`；实现后断言 live organization/Target 不存在，而 wave/member/binding exact IDs仍存在。
4. 验证命令：

```bash
cd backend
just space-guard
cargo nextest run -p golish-db --test cleanup_obligation_kernel -E 'test(organization_deletion_retains_bound_tool_truth_identity)' --status-level fail
```

## Task 2：forward migration

1. 新增 `organization_deletion_require_live_organization_reference()`：从 trigger argument读取 UUID，锁定 live organization，并拒绝 active deletion unit。
2. 新增 `organization_deletion_require_live_target_reference()`：锁定 live Target，按可选 trigger arguments重验 organization/project，并拒绝 active deletion unit。
3. 只对设计文档列出的 retained relations `DROP CONSTRAINT`，随后对对应 reference columns安装 `BEFORE INSERT OR UPDATE OF ...` trigger。第二个迁移处理 audit 与其它不可变 live-target aliases；可直接清理的 mutable/raw relations保留原 FK。
4. 重新运行 Task 1，预期 1/1 PASS；再运行现有两阶段删除测试，预期无回归。

## Task 3：focused verification and retained convergence

1. 运行：

```bash
cd backend
just space-guard
cargo nextest run -p golish-db --test cleanup_obligation_kernel -E 'test(/deletion/)' --status-level fail --test-threads 1
just space-guard
cargo clippy -p golish-db --test cleanup_obligation_kernel -- -D warnings
```

2. 对 migration、测试与 repo执行 rustfmt/SQL静态检查和 `git diff --check`。
3. 重建/启动一次当前应用使 forward migration 生效；不新建删除请求。只读确认 retained job变为 `hard_delete_committed`、live org/Targets消失、bound wave/member/binding仍存在。
4. 更新模块卡、feature evidence与progress；提交并普通 push 当前分支。
