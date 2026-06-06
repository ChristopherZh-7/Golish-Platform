# golish-cli-output / cli_json

> **一句话职责**：CLI `--json` 模式的标准化输出——`CliJsonEvent` wire 对象 + `convert_to_cli_json` 单一 dispatcher，把每个 `AiEvent` 变体按类别路由到 per-category helper；**不截断任何数据**（截断只在 terminal 模式做）。

- **类型**：目录模块（属于 crate [`golish-cli-output`](../golish-cli-output.md)）
- **路径**：`backend/crates/golish-cli-output/src/cli_json/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 新增 `AiEvent` 变体后要让 `--json` 模式输出它时（必须在 `convert_to_cli_json` 加分支）
- 改某类事件的 JSON 形状（字段重命名如 `args`→`input` / `result`→`output`）时
- eval 框架/脚本解析 CLI JSON 出问题时

## 职责

把 `AiEvent` 转成 eval 友好的标准化 JSON（`event` 字段替代 `type`、加 `timestamp`、tool 事件 `args`→`input`/`result`→`output`）。`convert_to_cli_json` 是单一 dispatcher，按 lifecycle/streaming/tools/sub_agent/context/loop_guard/workflow/hitl/task 路由到各 helper。**全程不截断**。

## 公开接口

| 符号 | 说明 |
|---|---|
| `CliJsonEvent` | 标准化 JSON 输出对象（event + timestamp + flatten data） |
| `convert_to_cli_json(&AiEvent) -> CliJsonEvent` | 单一转换 dispatcher |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `CliJsonEvent` + `convert_to_cli_json` 大 match |
| `lifecycle.rs` / `streaming.rs` / `tools.rs` / `sub_agent.rs` | 生命周期 / 流式 / 工具 / sub-agent |
| `context.rs` / `loop_guard.rs` / `workflow.rs` / `hitl.rs` / `task.rs` / `tail.rs` | 上下文 / 防循环 / 工作流 / HITL / 任务 |

## 依赖

- `golish_core::events::AiEvent`、`serde`/`serde_json`

## 注意事项 / 坑

- **绝不截断**：本模块全量透传 tool input/output/reasoning/text delta；截断只在 terminal 模式（可读性）做——别在这里加截断。
- `convert_to_cli_json` 是穷尽 match：新增 `AiEvent` 变体必须在此加分支（否则编译失败或漏输出）。
- 字段重命名（`args`→`input` 等）是对外契约，改名会破坏 eval 脚本。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-cli-output cli_json
```
