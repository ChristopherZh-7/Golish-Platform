# Scoping 人工公司身份确认闭环实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 Human 已确认的 company candidate 在 Scoping finalizer 中由 exact durable witness 自动冻结 confirmed receipt，避免因模型漏掉第二次 resolver 调用而永久 BLOCK。

**架构：** 在 `golish-db` Company Identity repo 中加入 caller-owned transaction helper，解析同 operation/execution 的 immutable receipt 与 terminal ToolCalls 并追加 superseding receipt；`runtime_memory_tx::finalize_scoping_scope` 在读取 confirmed authority 前调用它。另提供同 witness 的只读 validator，让 `golish` exact resume 只在完整 expected org 与 durable Human/Create authority 全等时接受 pre-freeze Scoping shape。所有选择与组织绑定都由 exact tuple 验证，不能从提交 prose 或 caller UUID 推导。

**技术栈：** Rust 2021、SQLx/PostgreSQL、serde_json、嵌入式 PG integration test。

## 文件

- 修改 `backend/crates/golish-db/tests/runtime_scope_freeze.rs`：增加 retained Human confirmation 的 RED/GREEN 集成回归。
- 修改 `backend/crates/golish-db/src/repo/scoping_company_identities.rs`：增加 exact witness promotion helper 与纯解析边界。
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：在已有 operation lock 内调用 promotion helper。
- 修改 `backend/crates/golish/src/stage_run/runtime_v2.rs` 与 `mod.rs`：接入 Scoping-only pre-freeze exact resume authority 和纯 validation 回归。
- 更新 `docs/modules/backend/golish-db.md`、`docs/modules/backend/golish-db/repo.md` 与 `docs/modules/INDEX.md`：记录服务端恢复合同。
- 更新 `feature_list.json` 与 `agent-progress.md`：记录状态和证据。

## Task 1：RED 回归

1. 在 `runtime_scope_freeze.rs` 创建独立 Scoping operation，落一条 `needs_human` receipt、exact company-identity AskHuman、成功 root create、root-only choice和 trusted submission。
2. 调用 `finalize_scoping_scope`，断言当前代码返回 `scoping_confirmed_company_identity_missing`。
3. 运行：

```bash
just space-guard
cargo nextest run -p golish-db --test runtime_scope_freeze -E 'test(human_selected_company_identity_is_frozen_before_scope_finalization)' --status-level fail
```

预期：测试因 confirmed receipt 未生成而失败，且失败原因正是 missing-confirmed。

## Task 2：最小生产实现

1. 在 `scoping_company_identities.rs` 加入 `promote_exact_human_selection_on`，输入 caller-owned `PgConnection`、operation、stage execution、root organization。
2. 先返回既有 confirmed receipt；否则锁定 latest `needs_human` receipt并解析候选集合。
3. 读取之后的 terminal `ask_human` 与 `manage_organizations(create)` rows；要求 exact operation/stage、candidate set、Human option/response、canonical name、registration identifier、root org与canonical project全部一致。
4. 使用 deterministic attempt/id/hash追加 `human_selected/confirmed` receipt，保留 provider evidence，并把 Human/Create ToolCall ids写入 source/disambiguation authority。
5. 在 `finalize_scoping_scope` operation lock之后、confirmed SELECT之前调用 helper。

## Task 3：GREEN 与拒绝路径

1. 重跑 Task 1 命令，预期 1 passed。
2. 增加/保留一个缺 create 或 Other response 的测试，断言 helper不追加 receipt且 finalizer继续 fail closed。
3. 运行受影响测试文件的 Scoping selector：

```bash
just space-guard
cargo nextest run -p golish-db --test runtime_scope_freeze -E 'test(/company_identity|finalize_scoping_scope/)' --status-level fail
```

## Task 4：静态门禁与实体续跑

1. 运行 `cargo clippy -p golish-db --lib --tests -- -D warnings`、受影响文件 rustfmt check 与 `git diff --check`。
2. 重启/复用新 binary，对 operation `9bc8fd1c-7f03-44ca-ba58-42ada29e5baa` 做 exact resume；不创建 fresh task、不手工 INSERT。
3. 用 `scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --db --full` 和只读 DB 查询确认 Scoping 已通过且唯一 confirmed receipt存在。
4. 将命令、exit code、receipt/operation evidence写入 `agent-progress.md`，再决定 feature 为 `passing` 或保留 `in_progress`。

## Task 5：pre-freeze resume 循环闭合

1. 增加只读 `exact_human_selection_root_is_ready`，复用同 operation/execution candidate、Human response、root create 与 canonical project/root 验证；无任何写入。
2. `runtime_v2::load_relational_resume_authority` 在 Scoping + `engagement_org_id=NULL` 时要求 exact expected org 且只读 validator 为 true；其他 stage 与 post-freeze shape 不变。
3. `validate_resume_candidate` 只对该 shape 接受 relational root 等于 `expectations.organization_id`；缺失/漂移仍拒绝。
4. 跑 `golish` pure focused test 与 retained exact resume；确认 finalizer PASS 并写入正式 `engagement_org_id`/sealed scope。
