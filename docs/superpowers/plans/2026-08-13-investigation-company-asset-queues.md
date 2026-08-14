# Investigation company and asset queues 实现计划

> Superseded for asset-internal orchestration by
> `2026-08-14-investigation-asset-primary-dynamic-team.md`. Do not implement or preserve the fixed
> four-role roster, fixed role order or exact-four barrier described below.

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 将 Investigation 改成持久的“公司串行 → 资产串行 → 固定多角色提出假设 → 当前资产真实验证 → 动态追加假设 → 固定点”闭环。
**架构：** 新增 server-owned company/asset queue authority 与 CAS cursor，把 Hypothesis/Generation/Verification authority 绑定到 exact asset lane；验证只使用现有 Tool Manager 的动态 inventory 与 per-call scope/JIT/budget/credential guard，业务 Gate 仅看 canonical hypothesis resolution，并用资产级 Primary 贯穿分析与验证。
**技术栈：** Rust 2021、sqlx、embedded PostgreSQL、rig-core sub-agents、Tauri stage_run CLI、nextest。

## 文件结构

- `backend/crates/golish-db/migrations/20260813000003_investigation_company_asset_queues.sql`：additive queue/lane/event/fixed-point schema 和跨表 guard。
- `backend/crates/golish-agent-kit/src/db_traits/investigation_asset_queue.rs`：portable commands/views/errors/repository contract。
- `backend/crates/golish-db/src/repo/investigation_asset_queue.rs`：transactional freeze/claim/transition/backlog/fixed-point。
- `backend/crates/golish-agent-app/src/ai/db_bridge/investigation_asset_queue.rs`：app repository bridge。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：ordered company/asset cursor 与 Asset Primary loop。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`：四角色 itinerary 与执行 objective。
- `backend/crates/golish-sub-agents/src/executor/{tool_setup,prompt_assembly}.rs`：host-injected exact execution tools。
- `backend/crates/golish-db/tests/investigation_asset_queue.rs`：migrated-PG state-machine/ownership/replay tests。
- `backend/crates/golish-agent-app/tests/investigation_asset_verification_loopback.rs`：动态 Tool Manager invocation、terminal resolution、same-lane discovery/admission integration。

## 任务 1：持久化公司和资产队列

1. 先写 migrated-PG RED：冻结两个有序公司、每公司两个资产，只允许 company 0 / asset 0 claim。
2. 运行 `just space-guard` 和 exact test，记录缺少 API/schema 的预期失败。
3. 新增 company queue seal/member/head/event 与 asset queue seal/lane/event/fixed-point 表；成员只能从 server-owned scope/target 查询冻结。
4. 实现幂等 freeze、exact claim、CAS transition、response-loss replay。
5. 复跑 GREEN，并补 foreign org、错误 ordinal、双 active、evolution fuel 测试。

## 任务 2：把 canonical hypothesis authority 绑定资产 lane

1. 先写 compiler RED：拒绝跨 lane subject、root、generation、VerificationTask 和 evolution successor。
2. 给 snapshot、attempt、work、hypothesis root、generation、wave、pending evolution、rearm/fixed receipt 加 lane 列与 mandatory new-runtime guard；历史 NULL 行仅可审计，不可恢复执行。
3. Analysis 只投影当前 lane 的 asset/web-origin/endpoint subject 与证据；proposal 必须解析回 lane `target_id`，revision 写 `target_live_id`。
4. generation predecessor/member 采用 lane-local ordinal；Task/Campaign/FactDelta/evolution owner 必须 exact 相等。
5. 复跑 compiler/migration focused tests 到 GREEN。

## 任务 3：一个 Asset Primary 与固定四角色讨论

1. 先写 runtime RED：同一 Asset Primary/message chain 贯穿 initial analysis、每个 hypothesis verification 和 evolution。
2. 冻结 `[browser,researcher,pentester,adviser]`；每个 analysis epoch 必须各派一个只读 WorkItem，其他角色可选。
3. Primary synthesis 前封 exact role census；缺失/失败不能由 Primary prose 替代。
4. 删除 one-Primary-per-VerificationTask 与独立 Execution Primary 的运行入口；所有 task result 回到 Asset Primary，不提供旧调度回退。
5. 复跑 runtime/sub-agent focused tests 到 GREEN。

## 任务 4：逐条清空当前资产 backlog 并使用真实工具

1. 先写 scheduler RED：stable hypothesis ordinal、当前资产未关闭时禁止 next asset、多 action、同 lane 追加新 hypothesis。
2. Campaign/list-assignable/driver 全部加 exact `asset_lane_id`，open counts 从 DB heads 派生。
3. 现有 Tool Manager 是唯一工具目录；不得新建 Investigation-only 固定 capability 枚举或少数工具白名单。按当前 installed/enabled/ready/config/policy 状态动态投影浏览器、HTTP、CLI、scanner、script、PoC 及未来工具。
4. 提出/反证假设时 reasoning workers 只读；进入 verification 后，资产团队可自主连续选择多个 Tool Manager 工具并依据结果换策略。Browser 使用真实 managed browser；curl/sqlmap/Nuclei/其他 CLI、临时脚本和 PoC 统一走既有 managed process/tool execution。
5. 每个具体调用都物化 exact asset assignment；runtime 与 Tool Manager 共同校验 schema、scope/JIT/lease/budget/credential/hash，不把角色名、模型 URL、raw shell 文本当权限。
6. 每次行动继续落 Tool Manager 自身审计/evidence；一个 hypothesis 可拥有 0..N 次工具行动，gate 不检查工具种类、调用次数、角色分配或 execution/receipt/Oracle 数量关系。
7. 新增独立 hypothesis resolution authority：仅 `supported|refuted|dismissed` 是已解决；`open|untested|inconclusive|blocked` 必须留在当前资产队列中继续验证。
8. 复跑 assignment/scheduler/Tool Manager adapter tests 到 GREEN。

## 任务 5：资产固定点、公司推进和实体 CLI

1. 先写 RED：zero-hypothesis fixed point、全部 canonical hypothesis resolved、验证中新增 hypothesis 回到同 asset、next asset/company ordering。
2. 实现 exact backlog projection 和 fixed-point receipt；它只按每条 current canonical hypothesis 的 resolved conclusion 以及不存在尚未入库的新 hypothesis proposal 判定，不以 Campaign/Wave/FactDelta、工具动作、receipt 或 Oracle 的状态/cardinality 判定；只允许 terminal receipt transaction 推进 cursor。
3. 所有公司/资产 lane terminal 后才能写 unified Investigation closure。
4. 构建 CLI，从已有 final Application Understanding seal 只 fork Investigation；不重跑前序阶段。
5. 逐个读取 `run.log`、run tree 和 DB。每发现一个断点，先写 RED、最小修复、重建、resume/fork，直到实体 closure。
6. 把命令、exit code、session/operation ids、动态工具 invocation、canonical resolution/discovery 和 queue counts 记录到 progress/feature evidence。

## 定向验证

```bash
cd backend
just space-guard
cargo nextest run -p golish-db --test investigation_asset_queue --status-level fail
cargo nextest run -p golish-agent-runtime -p golish-sub-agents -E 'test(investigation_asset_) | test(investigation_execution_)' --status-level fail
cargo nextest run -p golish-agent-app --test investigation_asset_verification_loopback --status-level fail
cargo clippy -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-pentest-app --lib --no-deps -- -D warnings
cargo fmt --all -- --check
```

依 `AGENTS.md` §0.1，未获用户明确要求时不运行 init/precommit/全 workspace 大型门禁。
