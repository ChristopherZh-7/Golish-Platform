# golish / cli

> **一句话职责**：Golish headless CLI——用与 GUI 同一套服务（经 `GolishRuntime`/`CliRuntime` 抽象）跑命令，支持 REPL，事件经 channel 给 output handler（print/JSON）；支撑自动化测试与脚本。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/cli/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 CLI 参数解析（args）、headless 跑命令（runner）、REPL、CLI 输出时
- 改 `initialize_agent`（CliRuntime 装配 agent）时

## 职责

提供与 GUI 共享服务的命令行入口。普通 CLI 在 DB-ready 后启动同一 Memory Supervisor + Cleanup DB-global worker + Reporting orphan-artifact GC；shutdown 顺序为 Cleanup/Reporting → Memory → runtime → embedded PG，避免 worker 在 pool 停止后继续 claim 或扫文件。

## 公开接口

| 符号 | 说明 |
|---|---|
| `args`（clap 参数） | CLI 参数（含 fresh `--stage-run` 与 exact `--stage-run-resume`） |
| `runner` / `repl` | headless 执行 / REPL |
| `bootstrap::initialize_agent`（`pub(crate)`） | 用 CliRuntime 装配 agent；调用方显式传 `event_session_id`（evidence 账本/后台任务/事件 envelope 的会话身份）——REPL 传 `"cli"`，stage-run 传与 `set_chat_session_id` 一致的 `stage-run-{uuid}`（gate/refiner 按该 id 查账本，写读必须同 id） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `args.rs` | clap 参数定义 |
| `runner.rs` | headless 命令执行 |
| `bootstrap/` / `repl/` | 初始化 / REPL |

## 依赖

- crate 内 app-core（`CliRuntime`）、agent 栈；`golish-cli-output`、`clap`、`atty`

## 注意事项 / 坑

- **与 GUI 共享逻辑**：经 `GolishRuntime` 抽象，别为 CLI 复制一套 agent 逻辑。
- `--stage-run` 由 `stage_run` 模块承载；CLI 只 dispatch。
- `--stage-run-resume <stage-run-key|session UUID|operation UUID>` 同样只 dispatch
  到 `stage_run`，不进入普通 headless chat。它与 fresh slice/seed/ephemeral 参数
  冲突；`--replay` 仍然只读，不能恢复 operation。
- `running` 孤儿恢复必须显式传 `--allow-orphan-running`，首 stage 缺
  `graph_flow` 时还必须传 `--repair-missing-graph-flow`；两者都依赖
  `--expect-session/--expect-task/--expect-operation/--expect-org/--expect-stage`
  的 exact identity 校验，不能从 task 年龄猜测进程已死。
- 旧 startup reaper 若已把同一 flat-checkpoint orphan 标成固定
  `Abandoned: ...` failed，必须另外显式传 `--repair-reaped-task`；CLI 只在完整
  expected identity、固定 marker、operation advisory claim 和 CAS 全匹配时恢复，
  普通 failed task 永不复活。
- `runner::execute_once` 在启动 terminal-event receiver 前获取 bridge universal top-level lease；busy 请求不会发 `Completed/Error`，若先启动 receiver 再 acquire 会让 CLI 永久等待。执行结束仍持 lease 做 async request-state cleanup。
- `CliContext::shutdown` 顺序固定为 agent/MCP/sidecar → Cleanup/Reporting workers → Memory Supervisor graceful join → runtime → owned embedded PG；不能先停 pool 再等 worker/projector。

## 测试入口

```bash
cd backend && cargo nextest run -p golish cli
# 参数层：cargo test -p golish cli::args::tests::test_args_stage_run_resume --lib
```
