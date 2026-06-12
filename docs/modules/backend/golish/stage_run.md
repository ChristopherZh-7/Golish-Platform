# golish / stage_run

> **一句话职责**：headless 单/区间阶段实跑器（`golish --stage-run`，方案 2）——无 GUI 启真后端（嵌入式 PG + 真 pentest 工具 + 真 LLM），跑一个 harness stage 或 `--from..=--to` DAG 切片，自动确认 scoping HITL（`--auto-approve`），打印结构化报告（gate PASS/BLOCK + 原因/工具/evidence）并退出；transcript 照常落盘可 `--replay`。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/stage_run/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 headless 阶段实跑（boot/seed/run/report）、`--stage-run`/`--from`/`--to`/`--only`/`--org`/`--target` 行为时
- 调试逐阶段测试（替代 `just dev` 起 GUI 手动驱阶段）时

## 职责

无 GUI bootstrap（lazy pool + spawn_embedded_pg + 就绪门 → `AppState::new` → `extract_agent_state`）→ `cli::initialize_agent(CliRuntime)` → `configure_bridge(None)` → 跑 `TaskOrchestrator.run_stage`（DAG 切片）→ 自动 `respond_to_approval` 处理 ask_human → `format_report`。设计见 `docs/design/2026-06-06-headless-single-stage-runner.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| stage_run 入口（boot → orchestrate → report） | headless 跑 + 报告 |
| `--org`/`--target` seeding（`maybe_seed`/`seed_upstream`/`build_objective`） | 上游目标种入 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | boot + seed + 事件消费 + orchestrate + format_report |

## 依赖

- crate 内 app（bootstrap）、cli（initialize_agent）、agent 栈；`golish-agent-kit::harness`、嵌入式 PG

## 注意事项 / 坑

- 真 LLM + 真工具 + 真 evidence（无 GUI）；活体跑需 LLM key + 网络。
- gate 走确定性 evidence 门（I7/I8）；自动确认仅对 scoping HITL，不放松 gate。
- feature `headless-single-stage-runner-2026-06-06` 在 feature_list（in_progress）。
- **session 四身份必须同值**：`initialize_agent(.., &session_id)`（event/evidence 写入）、`set_session_id`（终端）、`set_chat_session_id`（gate/refiner 查账本）、transcript 目录都用同一个 `stage-run-{uuid}`。2026-06-12 前 event 侧残留 `"cli"`，导致 evidence 落账后 gate/refiner 查不到（账本 facts=0、submit-only 锁不可达）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish stage_run
# 活体：just stage <profile> <to> "<objective>"
```
