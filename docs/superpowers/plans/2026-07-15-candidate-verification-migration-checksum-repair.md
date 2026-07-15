# Candidate Verification / Stage Team migration checksum 修复实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。

**目标：** 在保留现有持久化数据的前提下修复 `20260714000002/00003` checksum drift，并保证旧库补齐 Candidate recovery 与 Stage Team 后来追加的 schema/function contract。

**架构：** `golish-db` 只接受分别审计过的精确 old/new SHA-384 repair 对；SQLx repair 后继续正常 migration 流程，由更高版本的 forward migrations 安装 catalog diff。Stage Team forward 同时支持 audited old schema 与 current clean-install schema。任一版本、描述、checksum 或 forward SQL 不匹配都继续 fail closed。

**技术栈：** Rust 2021、sqlx Migrator、PostgreSQL 17、cargo-nextest。

## 文件结构

- 修改 `backend/crates/golish-db/src/pool.rs`：真实 drift 的精确 allowlist 与回归测试。
- 新建 `backend/crates/golish-db/migrations/20260715000001_candidate_verification_recovery_forward_fix.sql`：两个 `CREATE OR REPLACE FUNCTION`。
- 新建 `backend/crates/golish-db/migrations/20260715000002_stage_team_scheduler_forward_fix.sql`：Stage Team 缺失表/trigger 与函数替换。
- 更新 `docs/modules/backend/golish-db.md` 与 `docs/modules/INDEX.md`：记录 migration repair/forward contract。
- 更新 `agent-progress.md`：记录 RED/GREEN、真实 DB 验收和未完成门禁。

## Task 1：锁定真实 checksum drift 的 RED

1. 在 `pool.rs` tests 中读取当前 migrator 的 `20260714000002`。
2. 构造成功 metadata row，old checksum 固定为 `5228caa9af2eefb20f860407a7c39a97d38c71e83876ecf28da5e089a6c15a0f71dcf17f09780701be1249296b80b855`。
3. 断言 `plan_checksum_repairs` 返回一条 repair，且 new checksum 等于当前 migrator checksum；当前空 allowlist 下测试必须以 `not explicitly allowlisted` 失败。
4. 运行：

   ```bash
   cd backend && cargo nextest run -p golish-db candidate_verification_recovery_known_checksum_drift_is_exactly_repairable --status-level fail
   ```

## Task 2：实现精确 allowlist

1. 给 `CHECKSUM_REPAIR_ALLOWLIST` 增加两条版本、描述、old checksum、new checksum 全匹配项；checksum 均用固定 48-byte byte string，不做前缀匹配。
2. 保留现有 dirty row、description mismatch、unknown drift fail-closed tests。
3. 重跑 Task 1 focused test，并运行全部 `pool::tests`。

## Task 3：新增 forward migration

1. 新建 `20260715000001_candidate_verification_recovery_forward_fix.sql`。
2. 用 `CREATE OR REPLACE FUNCTION enforce_candidate_attempt_audit_transition()` 固化 `queued -> running|abandoned`，其余既有 immutability/terminalization 规则保持当前 `00002` 定义。
3. 用 `CREATE OR REPLACE FUNCTION enforce_candidate_attempt_authority()` 固化：新 side effect 仍要求 current approved + `start_before` 未过期；仅已有 completed/failed action、无 started/outcome_unknown、无 terminal intent 的同 Attempt UPDATE 可在 approved/expired approval 下 submit-only continuation。
4. 增加测试读取 forward migration，断言两个函数均被重放，且不含 `ALTER TABLE`、`CREATE TABLE`、`UPDATE _sqlx_migrations`。

## Task 4：真实旧库路径验收

1. 在临时审计数据库应用 migrations，模拟 `00002` 为真实 old checksum 且 forward migration 尚未执行。
2. 运行 `golish-db` migration bootstrap，确认 exact checksum CAS repair 与 `20260715000001` 均成功。
3. 对比持久化库与 fresh audit 库的相关 table dump/function definitions。
4. 重启开发应用，检查最新 `backend.log` 出现 migration complete/db-ready，并在 Target 页触发 `organization_list` 成功。

## Task 4b：Stage Team 后续 drift 收口

1. 第一条 repair 后捕获真实启动 RED：`20260714000003 stage team scheduler is not explicitly allowlisted`。
2. 对真实库与应用当前 `00003` 的 table/function catalog 做 diff，锁定缺失的三张 recovery/gap/repair-generation 表、一个新提交者函数/trigger与三个旧函数定义。
3. 先写 checksum 与 forward migration 文件存在/对象覆盖的 RED tests，再增加第二条 exact allowlist 与 `20260715000002_stage_team_scheduler_forward_fix.sql`。
4. 从真实库创建临时 clone，按 SQLx 顺序单事务应用 missing `00004`、`15000001`、`15000002`；与 fresh audit 库比较 Stage Team table/function dump，要求两个 diff exit 0 后才允许触碰真实库。
5. 运行 clean-install integration；若 `00003` 已创建对象，`15000002` 必须通过 `IF NOT EXISTS` / `CREATE OR REPLACE` / trigger 重建安全收敛。由于 forward 初稿已在真实库执行，idempotence-only 文件变化也必须用第三条 exact old/new checksum pair 明确登记，禁止无条件 metadata update。

## Task 5：文档与收尾

1. 更新 `golish-db` 模块卡与主索引同步说明。
2. 把命令、exit code、关键输出写入 `agent-progress.md`。
3. 运行 focused fmt、`golish-db` tests、Clippy 与 `git diff --check`。按用户明确指令不运行 `init.sh`；未运行 full `just precommit` 时保持父 feature `in_progress`。
4. 不 commit、stage 或 push，除非用户另行要求。
