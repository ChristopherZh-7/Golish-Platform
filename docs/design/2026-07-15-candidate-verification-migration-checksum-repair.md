# Candidate Verification / Stage Team migration checksum 修复设计

## 背景与故障

GUI 在 Target 页调用 `organization_list` 时得到 `Database failed to start`。真实根因不是 Target/Organization 数据，而是嵌入式 PostgreSQL 在 migration 阶段 fail closed：

- 数据库已成功记录 migration `20260714000002 candidate verification recovery`，SHA-384 为 `5228caa9af2eefb20f860407a7c39a97d38c71e83876ecf28da5e089a6c15a0f71dcf17f09780701be1249296b80b855`。
- 当前同版本 SQL 文件 SHA-384 为 `bedba07991ef570f99bc6a160233e50d61dbfb56c36edfdcbbfd58e6abcdd215868f96d3a23e48bb10105b2a79ab955b`。
- 修复第一条后，SQLx 继续暴露 `20260714000003 stage team scheduler` 的相同问题：数据库 checksum 为 `43af87b888669ddbd56d744acf1186bb97bc7cf7473fc940ba8ebe584abb7833a88489d625c363cd51efb3079e6f8e60`，当前文件为 `cc61505773700dde3aef7588257816c9a3437aab748284c3c2b8bd8c0672703717ddfdf4ac227c8f0c415336887f4b58`。
- 故障发生时 `golish-db` 的 checksum repair allowlist 为空，因此启动按设计拒绝修改过的已执行 migration。

用户已在 2026-07-15 明确要求修复本次 DB/migration 故障。

## 真实 schema 审计

在现有持久化库旁创建临时审计库，只按文件顺序应用到 `20260714000002`，然后比较该 migration 涉及的 9 张表、约束、索引、trigger 与 15 个函数：

- 表、列、约束、索引和 trigger 的 `pg_dump --schema-only` 结果一致；仅 `pg_dump` 随机 `restrict` token 不同。
- `attack_candidate_approvals.start_before` 数据后置条件满足：无 NULL、无 `start_before > expires_at`。
- `candidate_attempts.status` 数据后置条件满足：无约束外状态。
- 只有两个函数仍是已执行版本的旧定义：
  - `enforce_candidate_attempt_audit_transition()` 缺少 `queued -> abandoned`。
  - `enforce_candidate_attempt_authority()` 缺少 approval start-expired 后 exact submit-only continuation。

因此这不是可以只更新 checksum 的纯文本 drift；必须同时通过新版本 forward migration 把两个函数推进到当前定义。

对 `20260714000003` 做同样的真实库 / fresh audit catalog diff 后确认：既有表、约束与索引中没有隐藏漂移，但旧库缺少后来追加的 `stage_team_recovery_decisions`、`stage_team_unit_gaps`、`stage_team_repair_generations` 三张表及其约束/trigger；同时缺少 `enforce_stage_team_deliverable_submitter()`，并有 `enforce_stage_team_plan_contract()`、`enforce_stage_work_item_contract()`、`enforce_terminal_stage_worker_output()` 三个旧函数定义。三张表在旧库不存在，因此没有待回填数据；相关既有数据也不需要 UPDATE。

## 决策

采用精确 repair + forward migration：

1. 在 `CHECKSUM_REPAIR_ALLOWLIST` 增加两条 schema drift 的独立 old/new SHA-384 对，只允许上述 `20260714000002 candidate verification recovery` 与 `20260714000003 stage team scheduler` 的精确 version/description/old/new 四元组。
2. 新增 `20260715000001_candidate_verification_recovery_forward_fix.sql`，只用 `CREATE OR REPLACE FUNCTION` 重放两个存在语义 drift 的函数。
3. 新增 `20260715000002_stage_team_scheduler_forward_fix.sql`，只补 catalog diff 证明缺失的三张表、相关 authority trigger/function，并替换三个旧函数定义。表使用 `IF NOT EXISTS`，函数用 `CREATE OR REPLACE`，trigger 先 `DROP IF EXISTS` 再重建，使旧库补缺与新库已由当前 `00003` 建好对象两条路径都收敛到同一 catalog。
4. SQLx 第二次 migration run 必须成功应用所有 missing/forward migrations 后，DB 才能进入 ready；任一 forward migration 失败时仍 fail closed。
5. 新库集成 RED 暴露 forward 初稿会重复创建对象；真实库已在 RED 前执行过该初稿。因此再登记一条 `20260715000002` 初稿 checksum `3bb22741...d95022e` 到 idempotent 版本 `803ca624...7a36c3d` 的精确 pair。该变更只增加新装幂等保护，真实 catalog 前后已由 dump/function diff 证明相同，不需要额外 schema forward。
6. 新装数据库会执行当前 `00002/00003`，再由幂等 forward 校准函数/trigger；已有旧 checksum 数据库会先做 exact checksum CAS repair，再执行缺失 forward migration。

## 明确不做

- 不删除或重建 `pgdata`。
- 不手工无条件修改 `_sqlx_migrations`。
- 不把任意 checksum drift 泛化放行。
- 不修改既有 migration 文件、业务数据或与两次 catalog diff 无关的对象。
- 不回滚共享 dirty tree 中其他功能。

## 验证边界

- TDD 必须先证明当前 allowlist 拒绝真实 old checksum。
- focused `golish-db` test 必须证明只接受 exact version/description/old/new 四元组。
- 真实库克隆必须在单事务中成功应用 missing `00004` 与两个 forward migration，且 Stage Team table/function dump 与 fresh audit 库完全一致。
- 持久化库重启后必须出现 `Database migrations complete` / `db-ready`，并实际执行 `organization_list`。
- 按用户指令不再运行 `init.sh`；只跑 focused checks。全量 `just precommit` 若未运行，feature 保持 `in_progress`。
