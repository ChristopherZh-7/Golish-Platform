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

提供与 GUI 共享服务的命令行入口。普通 CLI 在 DB-ready 后启动同一 Memory Supervisor + Investigation projection worker + Cleanup DB-global worker + Reporting orphan-artifact GC；shutdown 先停这些 process-owned worker，再停 runtime与 embedded PG，避免 worker 在 pool 停止后继续 claim、投影或扫文件。

## 公开接口

| 符号 | 说明 |
|---|---|
| `args`（clap 参数） | CLI 参数（含 fresh `--stage-run`、exact `--stage-run-resume` 与 shared-DB `--stage-run-fork`） |
| `runner` / `repl` | headless 执行 / REPL |
| `bootstrap::initialize_agent`（`pub(crate)`） | 用 CliRuntime 装配 agent；CLI 只解析 provider/model/API-key override 与必需身份字段，再调用 `golish-agent-app::ai::provider_bootstrap` 生成 GUI 同款 typed provider/shared config；调用方显式传 `event_session_id` |

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
- Provider route 必须先解析为 `AiProvider`/`ProviderConfig`，再走 app shared normalizer；未知/拼错 provider fail closed，禁止 fallback 成 OpenRouter。CLI flag 的 provider/model/API key 优先级保留，但 endpoint/reasoning/web-search/preferences/location/thoughts/Ollama base/model override/context config 不得在 CLI 另写一份 settings 规则。
- `--stage-run` 由 `stage_run` 模块承载；CLI 只 dispatch。
- CLI不调用Tauri `investigation_*` read commands；headless Candidate lifecycle继续使用显式验证后的bridge workspace与共享production runtime。它不提供rollout promotion或Plan C/D入口。
- hidden `--stage-run-test-database` 仅用于已有 shared-DB operation 的隔离验收，可与 exact resume 或 immutable-source fork 同用；值只接受小写 `golish_gatefix_*`、安全字符且不超过 63 字节。它不会自动创建/复制数据库，也不能选择默认 production 库。
- `--stage-run-fork <operation|session|chat-key>` 只 dispatch 到 `stage_run`。它必须配 `--only` 或完整 `--from/--to`，拒绝 Scoping、ephemeral DB 与 profile/org/target/subsidiary 覆盖；默认数据库因此与 GUI 相同。
- `--approve-phase-boundaries` 是兼容参数：当前内置 flow 的常规人工确认只在 Scoping，post-Scoping 不再产生 generic phase confirmation。该 flag 即使与 `--auto-approve` 同用也不授权 target scope、Candidate plan 或高风险 tool call。
- `--stage-run-resume <stage-run-key|session UUID|operation UUID>` 同样只 dispatch
  到 `stage_run`，不进入普通 headless chat。它与 fresh slice/seed/ephemeral 参数
  冲突；`--replay` 仍然只读，不能恢复 operation。CLI不再提供
  `--stage-run-campaign-authority`或任何PreparedAction人工授权注入参数。
- `running` 孤儿恢复必须显式传 `--allow-orphan-running`，首 stage 缺
  `graph_flow` 时还必须传 `--repair-missing-graph-flow`；两者都依赖
  `--expect-session/--expect-task/--expect-operation/--expect-org/--expect-stage`
  的 exact identity 校验，不能从 task 年龄猜测进程已死。
- 旧 startup reaper 若已把同一 flat-checkpoint orphan 标成固定
  `Abandoned: ...` failed，必须另外显式传 `--repair-reaped-task`；CLI 只在完整
  expected identity、固定 marker、operation advisory claim 和 CAS 全匹配时恢复，
  普通 failed task 永不复活。
- `runner::execute_once` 在启动 terminal-event receiver 前获取 bridge universal top-level lease；busy 请求不会发 `Completed/Error`，若先启动 receiver 再 acquire 会让 CLI 永久等待。执行结束仍持 lease 做 async request-state cleanup。
- `CliContext::shutdown` 顺序固定为 agent/MCP/sidecar → Investigation/Cleanup/Reporting workers → Memory Supervisor graceful join → runtime → owned embedded PG；不能先停 pool 再等 worker/projector。

## 测试入口

```bash
cd backend && cargo nextest run -p golish cli
# 参数层：cargo test -p golish cli::args::tests::test_args_stage_run_resume --lib
# 隔离库名护栏：cargo nextest run -p golish -E 'test(stage_run_db_accepts_only_explicit_gatefix_clone_names)'
```
