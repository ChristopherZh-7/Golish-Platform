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

提供与 GUI 共享服务的命令行入口（自动化/脚本）。`args` 解析（clap）、`runner` 执行、`repl` 交互、`bootstrap` 初始化（`initialize_agent` 用 `CliRuntime`）。事件经 `CliRuntime::emit` → channel → output handler（terminal/JSON/quiet，见 `golish-cli-output`）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `args`（clap 参数） | CLI 参数（含 `--stage-run` 等） |
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

## 测试入口

```bash
cd backend && cargo nextest run -p golish cli
```
