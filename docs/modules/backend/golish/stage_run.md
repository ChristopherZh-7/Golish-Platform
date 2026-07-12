# golish / stage_run

> **一句话职责**：headless 单/区间阶段实跑与 exact recovery（fresh `golish --stage-run` / persisted `--stage-run-resume`）——无 GUI 启真后端（嵌入式 PG + 真 pentest 工具 + 真 LLM），fresh 路径跑一个 stage/DAG 切片；resume 路径复用旧 session/task/operation/freshness/worker chain 并只重驱当前中断 stage；两者均打印 gate/evidence 报告，transcript 可 `--replay`。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/stage_run/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 headless 阶段实跑（boot/seed/run/report）、`--stage-run`/`--from`/`--to`/`--only`/`--org`/`--target` 行为时
- 调试逐阶段测试（替代 `just dev` 起 GUI 手动驱阶段）时
- 改真实 smoke runner（`scripts/stage_smoke.py` / `just stage-smoke`）或临时 DB 测试模式时

## 职责

无 GUI bootstrap（lazy pool + spawn_embedded_pg + 就绪门 → `AppState::new` → `extract_agent_state`）→ `cli::initialize_agent(CliRuntime)` → `configure_bridge(None)`。fresh 路径调用 `TaskOrchestrator.run_stage`；exact resume 路径先做 selector/identity/chain scope 校验，取得 operation advisory claim，必要时 CAS 补首 stage 缺失的 `graph_flow`，再调用同 task 的 `TaskOrchestrator.resume`（绝不 `run_stage`）。`main.rs` 把两条路径都放到专用 32MiB 大栈线程。测试入口可显式传 `--ephemeral-db`，但 resume 与 ephemeral/fresh slice/seed 参数互斥。**`--include-subsidiaries` 的 fresh 子公司扇出（2026-06-14 方案 C / fleet Phase B）改走共享 scheduler；resume 只重驱选中的一个 operation，不重开 fleet。** 设计见 `docs/design/2026-06-06-headless-single-stage-runner.md`、`docs/design/2026-07-11-stage-run-cli-exact-resume.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| stage_run 入口（boot → orchestrate → report） | headless 跑 + 报告 |
| `--stage-run-resume <selector>` | 恢复旧 stage-run chat key / DB session UUID / operation UUID；复用旧 DB session UUID、task/operation、org、profile、stage freshness、transcript 和 worker chain |
| `--allow-orphan-running` + exact expected ids | 仅在操作者确认旧进程已死时接受残留 `running`；默认只接受 `waiting` |
| `--repair-missing-graph-flow` | 在 exact identity + flat checkpoint 校验后，用 guarded `jsonb_set` 只补 `graph_flow`，保留 `stage_run_workers`/producer checkpoints/未知 sibling |
| `--repair-reaped-task` | 仅将带固定 startup-reaper abandoned marker 的 exact failed orphan 以 session/profile/stage/org/state/update-time CAS 恢复为 `waiting`；普通失败不可恢复 |
| `--org`/`--target` seeding（`maybe_seed`/`seed_upstream`/`build_objective`） | 上游目标种入 |
| `GOLISH_STAGE_RUN_SEED_OPEN_PORTS` | smoke 专用：给临时 DB 的 seeded targets 写入 confirmed-open `targets.ports[]`，格式 `host=80,443;host2=9001` |
| `--ephemeral-db` / `--keep-ephemeral-db` | stage-run 测试专用临时嵌入式 PG；默认清理，可显式保留 pgdata |
| `--db-smoke-summary` | 停 PG 前打印 session/project/org 关键表计数，验证真实落库 |
| `scripts/stage_smoke.py` / `just stage-smoke` | 包装真实 `golish --stage-run`，默认临时 DB，可选本地 HTTP fixture；脚本可显式传 `--provider` / `--model`，枚举 smoke 可用 `--route-probe-max-runtime-ms` / `--route-probe-max-requests` 控制 route_probe 前台预算 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | boot + seed + 事件消费 + orchestrate + format_report |
| `scripts/stage_smoke.py` | 真 stage smoke runner（临时 workspace/DB，本地 fixture，可接 `run_tree.py`） |
| `justfile` (`stage-smoke`) | 脚本化入口：`just stage-smoke <profile> <to-stage> "<objective>"` |

## 依赖

- crate 内 app（bootstrap）、cli（initialize_agent）、agent 栈、`engagement::{fleet_run（OrgFleetExecutor）, scheduler（run_fleet_scheduler）}`（子公司扇出共享调度）；`golish-agent-kit::harness`、嵌入式 PG

## 注意事项 / 坑

- 真 LLM + 真工具 + 真 evidence（无 GUI）；活体跑需 LLM key + 网络。
- `--ephemeral-db` 只隔离数据库；LLM provider / 外部情报源 / 主机工具仍是真调用，跑活体 smoke 前要确认授权与 API 成本。
- `GOLISH_STAGE_RUN_SEED_OPEN_PORTS` 只用于 isolated smoke；它会在 seed target 后写 `targets.ports[]` 和 EAS collected timestamps，方便复现“DB 已确认 open port，但 SERVICE-FINGERPRINT 没收口”的 retry 类问题。
- `scripts/stage_smoke.py` 对 enumeration 默认给 `route_probe_paths` 设置小前台预算（env：`GOLISH_ROUTE_PROBE_DEFAULT_MAX_RUNTIME_MS=30000`、`GOLISH_ROUTE_PROBE_DEFAULT_MAX_REQUESTS=800`）并写进 objective，避免本地 fixture / DeepSeek smoke 等三分钟才收束；需要完整字典闭环时传 `--full-route-probe`。
- 临时 DB 跑完默认删除；需要事后连库人工排查时加 `--keep-ephemeral-db`。自动验证优先看 `--db-smoke-summary`，它是在 PG 仍存活时查询出来的。
- 普通 `--stage-run` 启动前会探测配置端口：若 PostgreSQL 已在监听，说明本次复用了用户现有 DB，收尾只关闭本进程的 pool 并保留现有 PG；只有本次真正启动的 embedded PG 才调用 `stop()`。不能把“端口已占用、复用现有 PG”误当成本次拥有其生命周期。
- 被 Ctrl-C 或 panic 打断的 smoke 可能来不及停临时 embedded PG；收尾时只清理 `golish-stage-run-db-*` 临时 PG，勿杀默认 app DB（`~/Library/Application Support/golish-platform/pgdata`）。
- gate 走确定性 evidence 门（I7/I8）；自动确认仅对 scoping HITL，不放松 gate。
- fresh CLI 的 `--target` 是 trusted pre-stage intake：必须在 Scoping 前以
  `source='stage-run-seed'` 落精确 domain/IP/CIDR/URL/wildcard target。Headless
  `scope_review` auto-response 只从这些 `--target` 构造 exact table payload，不从 objective/
  LLM context 推断新 target；type/scope/value 必须与 DB trusted snapshot 一致。
- Scoping 未落 trusted seed 时必须阻塞，不得依靠 Target Intel `manage_targets`
  补种。`organizations.domains/app_domains/ip_ranges` 及 provider 数据都不能替代 CLI seed。
- 每次 parent/child `orchestrate` 先获取该 bridge 的 universal top-level request token，再用 `BridgeAgentExecutor::from_request` 升级 Task；`run_stage` 返回后仍持 lease 清 harness sidechannels。fleet 继续串行，因此同 bridge child runs 逐个取得 fresh request-scoped retry budget。
- **子公司扇出收敛（2026-06-14 · 方案 C）**：旧 step 6.5 手写 Rust per-child 循环 → `run_fleet_scheduler`；`orchestrate` 改 `pub(crate)` 供 `engagement::fleet_run::OrgFleetExecutor` 复用（CLI `emit_progress=false`，无单卡）。`engagement` 域暂无独立模块卡，fleet 驱动文档见上述 plan（follow-up：补 engagement 卡）。
- **逐子进度 eprintln（2026-06-14 收敛后补回中途可见性）**：调度器（IO-free 内核）新增第 4 个注入 trait `FleetProgress`，CLI 传 `engagement::fleet_run::CliFleetProgress{label:"subsidiary"}` → 每个子公司进 executor 前后打 `[stage-run] ── subsidiary i/N: 名 → running/PASS/BLOCK/FAIL ──`（恢复 T1 把手写循环换成 `run_fleet_scheduler` 后丢的那条逐子可见性）。GUI 单卡路径传 `NoopProgress`（进度走 `StageRunOrgProgress` 事件）。续跑跳过的 org 只 `on_org_done`（SKIP(done)）、不 `on_org_start`。i/N 由调度器静态 org 序提供（checklist 串行下即真实顺序）。
- **session 四身份必须同值**：`initialize_agent(.., &session_id)`（event/evidence 写入）、`set_session_id`（终端）、`set_chat_session_id`（gate/refiner 查账本）、transcript 目录都用同一个 `stage-run-{uuid}`。2026-06-12 前 event 侧残留 `"cli"`，导致 evidence 落账后 gate/refiner 查不到（账本 facts=0、submit-only 锁不可达）。
- **exact resume 不得重跑原命令**：fresh `--stage-run` 每次创建新 chat key/DB
  session/task/operation，旧 `technique_outcomes.run_id` 与 chain scope 都不可见。
  Resume 必须把旧 chat key 同时用于 event/evidence/run_id，把旧 `sessions.id`
  设回 tracker persistence session，并调用旧 task 的 `resume()`；同 stage 入口不会刷新
  `stage_started_at`。
- **fail-closed orphan/claim**：默认只收 `waiting`。残留 `running` 必须显式 flag
  加 expected DB session/task/operation/org/stage；进程内 bridge lease 无法跨进程证明
  owner 已死，因此不按时间猜 orphan。CLI 使用 operation UUID 派生的非阻塞 PG
  advisory lock 原子 claim，锁后重新读取全部身份；dedicated detached connection
  覆盖整个 resume，崩溃/断连自动释放，不持 DB transaction 跨 LLM/network。
- **首 stage 无 `graph_flow`**：graph executor 只在 node 返回后写嵌套 checkpoint；
  Ctrl-C 落在首 worker 内时，flat HarnessResumeState + `stage_run_workers` 仍有效但
  普通 `resume()` 不可加载。显式 repair 在 advisory claim 下要求 flat
  blob 可完整反序列化、profile/current_stage/current_stage_run_id 与
  operation/expected ids 全匹配且 `completed_count=0`，CAS `jsonb_set` 新增
  `{state: default, next_node: current_stage}`，并验证所有 sibling 原样保留后再
  resume；已有 `graph_flow.state` 也必须先完整反序列化，不能只凭 JSON object 外形。
- **startup reaper 与 flat checkpoint**：reaper 的 recoverable predicate 同时接受
  完整 `graph_flow` 或严格 flat first-stage checkpoint（profile/stage/run UUID、
  `completed_count=0`、非空 stage worker map）；后者会被 pause 为 `waiting` 而非
  fail。历史版本若已经写入固定 abandoned failed marker，exact resume 还必须显式
  `--repair-reaped-task`，在 advisory lock 下先 CAS 回 `waiting` 再补 graph；任何
  其它 failed result 都 fail-closed。
- **legacy chain task_id**：exact chain 必须匹配 chain id + DB session + specialist 且
  body 非空；`task_id=Some` 时还必须等于 operation。旧 stage-run chain 可能是
  `task_id=NULL`，由 guarded `operation_state.stage_run_workers` map 绑定 operation，
  可兼容但绝不手工回填；非空错 task 一律拒绝。

## 测试入口

```bash
cd backend && cargo nextest run -p golish stage_run
# exact resume 纯测试：cargo test -p golish stage_run::tests::resume_candidate --lib
# 活体：just stage <profile> <to> "<objective>"
# 隔离 DB 活体 smoke：just stage-smoke <profile> <to> "<objective>"
# 更细控制：python3 scripts/stage_smoke.py --fixture-web --provider deepseek --model deepseek-v4-flash --profile assessment --to target_intel --objective "smoke target_intel"
```
