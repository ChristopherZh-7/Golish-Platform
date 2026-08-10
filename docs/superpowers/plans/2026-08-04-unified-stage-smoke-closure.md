# Unified Stage Smoke Closure 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 fresh isolated stage-run 能冻结 unified AU/Investigation topology，并以一个足够丰富的本地实体和 operation-scoped exact evidence 证明完整闭环。

**架构：** hidden CLI hook 只在 owned ephemeral DB 的 pristine rank 0 上选择 rank 5/6，随后复用生产 operation freeze/orchestrator/gate 路径。Python wrapper 提供 localhost controlled fixture；Rust 在 embedded PG 停止前按 exact session 解析唯一 operation 并输出安全 canonical sets。

**技术栈：** Rust 2021、clap、sqlx/Postgres、Python 3 `http.server`、Node/Playwright、cargo-nextest。

## 文件结构

- 修改 `backend/crates/golish/src/cli/args.rs`：声明只限 ephemeral stage-run 的 joint-rank 参数。
- 修改 `backend/crates/golish/src/stage_run/mod.rs`：pristine rollout bootstrap、唯一 operation 解析、exact-set summary 与 unit tests。
- 修改 `scripts/stage_smoke.py`：unified topology wrapper、controlled fixture、confirmed-open port seed。
- 修改 `scripts/tests/test_stage_smoke.py`：fixture 和 topology-aware route budget 测试。
- 修改 `scripts/run_tree.py`、`scripts/tests/test_run_tree_runtime_memory.py`：显式 session evidence 永不回退全库历史行。
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`：Rust 消费端再次做 value-free URL template sanitization。
- 修改 `docs/modules/backend/golish/stage_run.md`、`docs/modules/backend/golish-pentest-app/pentest_bridge.md`、`docs/modules/INDEX.md`：同步接口和安全边界。
- 修改 `feature_list.json`、`agent-progress.md`：记录可重放命令、run id、实体证据与最终状态。

## 任务 1：锁死 ephemeral unified bootstrap

**文件：**

- 修改：`backend/crates/golish/src/cli/args.rs`
- 修改：`backend/crates/golish/src/stage_run/mod.rs`

**步骤 1：写 parser 失败测试。**

测试应证明合法形状只接受：

```text
golish --stage-run --ephemeral-db --stage-run-test-joint-rank 6 --to reporting
```

去掉 `--ephemeral-db` 或传 rank 4 必须由 clap 拒绝。

**步骤 2：实现参数与 bootstrap。**

实现签名固定为：

```rust
async fn bootstrap_ephemeral_joint_rollout(
    pool: &sqlx::PgPool,
    args: &Args,
    stage_db: &StageRunDbConfig,
) -> anyhow::Result<()>;
```

函数必须验证 `args.ephemeral_db && stage_db.temp_dir.is_some()`、`operation_state=0`、两个 singleton
是 rank 0/row_version 0；更新后重新读取 `operation_joint_contract_rank` 并与 5/6 精确相等。

**步骤 3：定向验证。**

```bash
just space-guard
cd backend
cargo nextest run -p golish -E 'test(unified_test_rollout_requires_ephemeral_stage_run_and_closed_rank)' --status-level fail
```

预期：1 test passed。

## 任务 2：让 Rust URL 消费端独立保证 value-free

**文件：**

- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`

**步骤 1：构造含真实 query value 的失败测试。**

输入同时包含 `email=person@example.test` 与 `token=secret-token`；序列化后的 occurrence 不得包含任一
原值，必须保留 `email=%7Bvalue%7D` 与 `token=%7Bvalue%7D`。

**步骤 2：实现双 URL 表示。**

```rust
fn sanitized_url_without_values(value: &str) -> Option<String>;
fn sanitized_url_template(value: &str) -> Option<String>;
```

template 保留有界 query names 并把值替换为 `{value}`；canonical 删除 query。page、document base、
initiator、script provenance 和 form 均走 template，DB canonical request 仍走 without-values。

**步骤 3：定向验证。**

```bash
just space-guard
cd backend
cargo nextest run -p golish-pentest-app -E 'test(capture_v3_redacted_shape_has_no_sensitive_values) | test(browser_occurrence_cross_origin_discovery_binds_source_a_and_resolved_b)' --status-level fail
```

预期：2 tests passed。

## 任务 3：建立 controlled fixture

**文件：**

- 修改：`scripts/stage_smoke.py`
- 修改：`scripts/tests/test_stage_smoke.py`

**步骤 1：写 fixture 失败测试。**

测试启动随机 localhost server，读取 `/dashboard`、`/api/debug/config`，POST `/api/orders`，并确认
递归 chunk 与 OpenAPI 文件存在；finally 必须停止 server。

**步骤 2：实现 fixture 与 topology order。**

保留 `LEGACY_STAGE_ORDER` 与 `UNIFIED_STAGE_ORDER`；`--unified-topology` 向 Rust 传
`--stage-run-test-joint-rank 6`。fixture 的 GET/POST/OPTIONS 只处理固定本地路由，POST body 最多读取
64 KiB 且不持久化。

**步骤 3：验证。**

```bash
python3 -m unittest scripts.tests.test_stage_smoke
python3 -m py_compile scripts/stage_smoke.py
```

预期：8 tests OK，py_compile 退出码 0。

## 任务 4：输出唯一 operation 的 exact DB truth

**文件：**

- 修改：`backend/crates/golish/src/stage_run/mod.rs`

**步骤 1：扩展 summary 类型。**

新增字段必须为：

```rust
operation_id: Option<String>,
operation_identity: serde_json::Value,
operation_scoped: BTreeMap<String, serde_json::Value>,
operation_exact_sets: BTreeMap<String, serde_json::Value>,
```

**步骤 2：实现 exact resolver 与安全集合。**

resolver 只能通过 `sessions.chat_session_key -> tasks -> operation_state`；candidate 数不等于 1 时输出
`stage_run_operation_resolution_not_exact`。exact set 仅输出 ID、状态和 canonical hash，不输出 raw payload。

**步骤 3：验证 formatter 与真实 SQL。**

```bash
just space-guard
cd backend
cargo nextest run -p golish -E 'test(format_db_smoke_summary_lists_sections)' --status-level fail
```

预期：unit test 通过；后续 controlled entity 的 `db_smoke_summary` 所有 exact-set query 均无 `error`。

## 任务 5：跑 controlled unified entity

**文件：**

- 产物：隔离 workspace 的 `.golish/transcripts/<session>/transcript.json`
- 产物：同目录 `run.log`

**步骤 1：确认工具。**

确认 Golish managed httpx/WhatWeb、nmap、naabu、nuclei templates、subfinder、Playwright 与 provider key；
WhatWeb 必须用声明的 Ruby 3.2 启动成功。

同时运行：

```bash
python3 -m unittest scripts.tests.test_run_tree_runtime_memory
```

预期：当 exact session 没有 evidence fact 时返回空集合且只执行一条带 `session_id=%s` 的查询，
不会查询 all-session facts。

**步骤 2：运行。**

```bash
python3 scripts/stage_smoke.py \
  --fixture-web --unified-topology \
  --profile red_team --provider deepseek --model deepseek-v4-flash \
  --to reporting --json
```

预期：frozen topology 是 `unified_investigation_v1`，阶段序列含 AU + Investigation 且不含新建的
Candidate + Verification；Enumeration exact set 非空；Reporting revision validated/final 或留下 typed
operator-only publication residual。

**步骤 3：停止/恢复。**

在独立 controlled run 的安全 checkpoint 中止后，只用输出的 exact session/task/operation 参数执行
`--stage-run-resume`。恢复必须复用同 operation，且 exact sets 不出现重复 authority member。

## 任务 6：跑 fresh moresec.cn 非破坏实体

**步骤 1：创建独立 workspace 与 ephemeral DB。**

显式传 `--target moresec.cn`、`--unified-topology`、`--profile red_team`，不从历史 Test1 选择 latest
operation。

**步骤 2：限制实体动作。**

允许 passive TI、DNS/HTTP liveness、有限端口/服务指纹、只读 Browser/JS、非侵入 template。写请求、
credential、exploit、race、OAST 均记录 typed residual。

**步骤 3：核对证据。**

确认 `operation_identity` 的 topology/rank、每个 stage run、Enumeration lane receipt、hypothesis/campaign、
report revision 都属于本次唯一 operation；显式 session 的 transcript/run.log 与 DB hash 对齐。

## 任务 7：文档、门禁与唯一 commit

**步骤 1：更新 module cards、feature 与 progress。**

记录每个命令、退出码、nextest run id、entity session/operation id、首要 residual；删除 verification 中
不存在的测试路径，换成实际可重放入口。

**步骤 2：执行用户已授权的最终门禁。**

```bash
just precommit
```

预期：退出码 0。若失败，修复并重跑，不能用局部测试冒充。

**步骤 3：唯一提交。**

用户要求整条线只提交一次，因此本计划覆盖的 WIP 与原 Plan C/D/Enumeration WIP 在所有实体证据通过后
统一 stage，检查 staged diff 后创建恰好一个 commit；不 push。
