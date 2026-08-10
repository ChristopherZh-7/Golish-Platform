# Unified Stage Smoke Closure 设计

## 背景

Plan D 已让 operation-frozen topology 能在 `legacy_candidate_verification_v1` 与
`unified_investigation_v1` 间选择，但 fresh `--ephemeral-db` 仍从 migration 的 rank 0
默认值启动。因此旧的 `scripts/stage_smoke.py` 即使跑到 Reporting，也只会冻结
Candidate + Verification，不能证明 Application Understanding + Investigation 的真实实体闭环。

此外，旧 fixture 只有一个 HTML、一个 JS 和两个 fetch；`db_smoke_summary` 主要输出全库/项目/组织
累计计数。这两者都不足以证明以下命题：同一个 fresh operation 由 AI Main 自主规划，经过
Target Intel、Enumeration JS/API、Plan C、AU、Investigation 和 Reporting，并且只消费本次 run 的
typed evidence。

## 决策

### 1. 只在全新 ephemeral DB 选择 unified deployment default

新增隐藏参数 `--stage-run-test-joint-rank <5|6>`，并由 clap 强制同时存在
`--stage-run` 与 `--ephemeral-db`。它不是生产 rollout promotion，也不能连接默认用户数据库。

数据库 ready 后、任何 seed/session/task/operation 创建前，runner 执行一个事务：

1. 证明 runner 持有 `TempDir`，且 `operation_state` 行数为 0；
2. 锁住两个 rollout singleton，并证明它们仍是 migration 的 rank 0 / row_version 0；
3. 仅在这个事务中 disable 两个 direct-mutation fixture guard；
4. 以 exact WHERE/CAS 将 Tool Truth 选为 `receipt_v1`，Investigation 选为 rank 5
   `registry_authoritative_legacy_projection` 或 rank 6 `new_only`；
5. 恢复 guard，重新读取并通过 `operation_joint_contract_rank(...)` 验证目标 rank；
6. commit 后才允许 seed 和 operation 创建。

任一前提不满足就 rollback 并停掉本次 embedded PG。这个入口不能更新既有 operation、不能跨 rank
伪造生产 readiness receipt，也不能成为默认 app DB 的维护后门。

### 2. Controlled fixture 覆盖真实 JS/API 发现链

`stage_smoke.py --fixture-web` 继续只监听随机 `127.0.0.1` 端口，但 fixture 扩展为：

- SPA fallback 与多条可导航路径；
- module bootstrap、递归 chunk、重复 chunk 引用、source map；
- same-origin SDK、OpenAPI、GraphQL；
- query/header/body/form/GraphQL variable 参数形状；
- 只读 debug exposure 与 CORS wildcard，供 Vuln/AU/Investigation 做无破坏验证；
- POST endpoint 只在本地返回固定 fixture 响应，不产生外部副作用；
- `/logout` 等危险 route 用于证明 Browser guard 不发送 mutation。

wrapper 将 fixture 的随机端口写入 `GOLISH_STAGE_RUN_SEED_OPEN_PORTS`，使 EAS 从 trusted seed
得到 confirmed-open port，而不是让模型凭端口号猜服务。

### 3. 停库前输出 exact operation evidence

`--db-smoke-summary` 必须先通过
`sessions.chat_session_key -> tasks.id -> operation_state.operation_id` 解析唯一 operation。
0 个或多个 candidate 都输出 `stage_run_operation_resolution_not_exact`，禁止 fallback 到最新 session。

唯一 operation 会输出：

- frozen runtime / Tool Truth / Investigation / topology identity；
- operation-scoped stage/unit/plan/work/output/submission/tool/receipt/hypothesis/campaign/report 计数；
- stage runs、deliverables、capability receipts、Enumeration lane receipts、hypothesis revisions、
  campaigns、report revisions 的 canonical member array + member count + SHA-256。

exact set 只包含 UUID、状态、contract hash、source-set hash 等安全字段；不输出 tool body、token、
cookie、raw witness 或 report raw content。

### 4. 实体目标安全边界

本地 fixture 可验证 POST/JIT/Oracle/FactDelta 等受控行为。`moresec.cn` 的 fresh isolated run 只允许
被动与非破坏性探测；credential、写请求、exploit、race、OAST 等若无明确实体授权，必须成为 typed
residual，不能为追求 gate PASS 自动执行。实体证据必须来自其独立 ephemeral DB 与显式 session，
不得复用历史 Test1 pass 或 `run_tree.py` 的 latest/all-session fallback。

## Enumeration AI 所有权

这项 smoke 能力不改变 Enumeration 调度语义。Main AI 仍读取目标与缺口、自主 plan、调用 Browser / JS
analysis / Parameter / Resolution 工具并可根据结果修订 plan。五类 lane 和 Coverage 仅是结果提交时的
typed receipt DAG；runner 不创建固定 wave 代替 Main 做计划。

## 验证

- Python fixture/order：`python3 -m unittest scripts.tests.test_stage_smoke`
- CLI/summary unit：精确运行 `golish` 的 stage-run tests
- URL privacy/provenance：精确运行 Browser occurrence tests
- controlled entity：`python3 scripts/stage_smoke.py --fixture-web --unified-topology --profile red_team --to reporting ...`
- fresh real entity：相同 unified/ephemeral 入口，target 为本次明确授权的 `moresec.cn`，并核对输出的
  exact operation identity、transcript、run.log 与 DB exact sets。
