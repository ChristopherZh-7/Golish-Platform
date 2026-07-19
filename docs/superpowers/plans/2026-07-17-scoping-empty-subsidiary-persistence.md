# Scoping 空子公司结果可信落库实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 systematic-debugging 先以日志/DB 证据定位根因，使用 test-driven-development 按 RED→GREEN 实施，使用 verification-before-completion 在声明完成前验证。

**目标：** 让 Scoping 中成功但没有控股子公司的 discovery 以 checked-empty 落库，并让当前 lifecycle 在真实 review 协议与重复选择后确定性收敛。

**架构：** DB repo 对 exact operation/stage/root/latest human choice 做只读授权，runtime 仅在 Scoping subsidiary discovery 的 frozen org 缺失路径调用该授权；scope derivation 使用 latest same-root choice，review parser 同时兼容当前与历史协议。

**技术栈：** Rust 2021、sqlx、async-trait、serde_json、cargo-nextest、embedded Postgres。

---

### Task 1：登记设计和 RED 行为

**文件：**

- 新建：`docs/design/2026-07-17-scoping-empty-subsidiary-persistence.md`
- 修改：`feature_list.json`
- 修改：`agent-progress.md`
- 测试：`backend/crates/golish-db/src/repo/tool_calls.rs`
- 测试：`backend/crates/golish-db/tests/runtime_scope_freeze.rs`
- 测试：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`

**步骤：**

1. 增加当前 `{rows: []}` unit review 测试，确认旧 parser 失败。
2. 增加 latest repeated choice 与 Scoping passive recon authorization 的 embedded-PG 测试，确认旧 derivation 返回 ambiguity/缺少 API。
3. 增加 runtime org resolution 测试，确认只有 exact DB authorization 能接受 Scoping requested root。
4. 先运行 focused tests 并在 `agent-progress.md` 记录 RED 证据。

### Task 2：实现 DB lifecycle 真值

**文件：**

- 修改：`backend/crates/golish-db/src/repo/tool_calls.rs`
- 修改：`backend/crates/golish-db/src/repo/operation_scope_decisions.rs`
- 修改：`backend/crates/golish-db/src/repo/mod.rs`（仅在需要导出时）

**步骤：**

1. `approved_unit_review` 接受 object.rows array 与 legacy top-level array。
2. exact derivation 选 ordered lifecycle 的 latest parseable same-root choice。
3. 新增 exact Scoping passive recon root authorization；验证 operation/stage/project/root identity，并只接受 latest included choice。
4. 运行 golish-db focused tests，确认 GREEN。

### Task 3：把可信授权接入 runtime persistence

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`

**步骤：**

1. 在 repo trait 增加 fail-closed 的 Scoping passive recon authorization 方法，并由生产 bridge 调用 DB repo。
2. runtime evidence organization resolver 优先使用 frozen `harness_org_id`；仅对 Scoping subsidiary discovery 解析 requested UUID 并要求 exact DB authorization。
3. main-agent/sub-agent persistence 调用传入 stage execution id。
4. 成功持久化时设置 `outcome_persisted=true`，失败保持 partial/false。
5. 运行 runtime/app focused tests，确认 GREEN。

### Task 4：模块文档和验证

**文件：**

- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/backend/golish-db/repo.md`
- 修改：`docs/modules/backend/golish-recon-app/agent_tools.md`（若工具契约说明受影响）
- 修改：`docs/modules/INDEX.md`
- 修改：`agent-progress.md`
- 修改：`feature_list.json`

**步骤：**

1. 更新模块卡的职责、可信授权和 checked-empty 契约。
2. 运行 `just space-guard` 后执行 focused nextest、Clippy、rustfmt、JSON/diff checks。
3. 运行 `just precommit`；按用户要求不运行 `init.sh`。
4. 把命令、退出码和关键输出写入进度/evidence；只有所有 DoD 满足后标记 passing。
5. 共享工作树已有其他未提交改动，本轮不自动创建 commit。
