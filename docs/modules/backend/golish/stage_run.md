# golish / stage_run

> **一句话职责**：headless 单/区间阶段实跑器（`golish --stage-run`，方案 2）——无 GUI 启真后端（嵌入式 PG + 真 pentest 工具 + 真 LLM），跑一个 harness stage 或 `--from..=--to` DAG 切片，自动确认 scoping HITL（`--auto-approve`），打印结构化报告（gate PASS/BLOCK + 原因/工具/evidence）并退出；测试模式可用临时嵌入式 DB 输出 `db_smoke_summary`；transcript 照常落盘可 `--replay`。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/stage_run/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 headless 阶段实跑（boot/seed/run/report）、`--stage-run`/`--from`/`--to`/`--only`/`--org`/`--target` 行为时
- 调试逐阶段测试（替代 `just dev` 起 GUI 手动驱阶段）时
- 改真实 smoke runner（`scripts/stage_smoke.py` / `just stage-smoke`）或临时 DB 测试模式时

## 职责

无 GUI bootstrap（lazy pool + spawn_embedded_pg + 就绪门 → `AppState::new` → `extract_agent_state`）→ `cli::initialize_agent(CliRuntime)` → `configure_bridge(None)` → 跑 `TaskOrchestrator.run_stage`（DAG 切片）→ 自动 `respond_to_approval` 处理 ask_human → `format_report`。`main.rs` 会把 `--stage-run` runtime 放到专用 32MiB 大栈线程里，避免 DeepSeek/agentic-loop 工具调用的大 async future 在主线程默认栈上溢出；普通 `--stage-run` 仍用默认 app DB。测试入口可显式传 `--ephemeral-db` 切到临时 pgdata + 随机本地端口，并在 embedded PG 停止前用 `--db-smoke-summary` 查询关键表，证明 rows 是否实际落库。**`--include-subsidiaries` 的子公司扇出（2026-06-14 方案 C / fleet Phase B）改走共享 `golish::engagement::scheduler::run_fleet_scheduler` + `OrgFleetExecutor`（每子 org 一个完整 run_stage + 独立 gate，串行；与 GUI `engagement_run_fleet` 命令共用同一调度内核，CLI 因此真测生产逻辑）。** 设计见 `docs/design/2026-06-06-headless-single-stage-runner.md` + `docs/superpowers/plans/2026-06-14-engagement-fleet-scheduler-convergence.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| stage_run 入口（boot → orchestrate → report） | headless 跑 + 报告 |
| `--org`/`--target` seeding（`maybe_seed`/`seed_upstream`/`build_objective`） | 上游目标种入 |
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
- `scripts/stage_smoke.py` 对 enumeration 默认给 `route_probe_paths` 设置小前台预算（env：`GOLISH_ROUTE_PROBE_DEFAULT_MAX_RUNTIME_MS=30000`、`GOLISH_ROUTE_PROBE_DEFAULT_MAX_REQUESTS=800`）并写进 objective，避免本地 fixture / DeepSeek smoke 等三分钟才收束；需要完整字典闭环时传 `--full-route-probe`。
- 临时 DB 跑完默认删除；需要事后连库人工排查时加 `--keep-ephemeral-db`。自动验证优先看 `--db-smoke-summary`，它是在 PG 仍存活时查询出来的。
- 被 Ctrl-C 或 panic 打断的 smoke 可能来不及停临时 embedded PG；收尾时只清理 `golish-stage-run-db-*` 临时 PG，勿杀默认 app DB（`~/Library/Application Support/golish-platform/pgdata`）。
- gate 走确定性 evidence 门（I7/I8）；自动确认仅对 scoping HITL，不放松 gate。
- feature `headless-single-stage-runner-2026-06-06` 在 feature_list（in_progress）。
- **子公司扇出收敛（2026-06-14 · 方案 C）**：旧 step 6.5 手写 Rust per-child 循环 → `run_fleet_scheduler`；`orchestrate` 改 `pub(crate)` 供 `engagement::fleet_run::OrgFleetExecutor` 复用（CLI `emit_progress=false`，无单卡）。`engagement` 域暂无独立模块卡，fleet 驱动文档见上述 plan（follow-up：补 engagement 卡）。
- **逐子进度 eprintln（2026-06-14 收敛后补回中途可见性）**：调度器（IO-free 内核）新增第 4 个注入 trait `FleetProgress`，CLI 传 `engagement::fleet_run::CliFleetProgress{label:"subsidiary"}` → 每个子公司进 executor 前后打 `[stage-run] ── subsidiary i/N: 名 → running/PASS/BLOCK/FAIL ──`（恢复 T1 把手写循环换成 `run_fleet_scheduler` 后丢的那条逐子可见性）。GUI 单卡路径传 `NoopProgress`（进度走 `StageRunOrgProgress` 事件）。续跑跳过的 org 只 `on_org_done`（SKIP(done)）、不 `on_org_start`。i/N 由调度器静态 org 序提供（checklist 串行下即真实顺序）。
- **session 四身份必须同值**：`initialize_agent(.., &session_id)`（event/evidence 写入）、`set_session_id`（终端）、`set_chat_session_id`（gate/refiner 查账本）、transcript 目录都用同一个 `stage-run-{uuid}`。2026-06-12 前 event 侧残留 `"cli"`，导致 evidence 落账后 gate/refiner 查不到（账本 facts=0、submit-only 锁不可达）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish stage_run
# 活体：just stage <profile> <to> "<objective>"
# 隔离 DB 活体 smoke：just stage-smoke <profile> <to> "<objective>"
# 更细控制：python3 scripts/stage_smoke.py --fixture-web --provider deepseek --model deepseek-v4-flash --profile assessment --to target_intel --objective "smoke target_intel"
```
